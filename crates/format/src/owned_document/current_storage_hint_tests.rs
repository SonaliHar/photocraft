//! UNRUN/uncompiled format-only CURRENT marker, migration and custody regressions.
use super::*;
use photocraft_doc::{
    BackedLayer, Color, ColorMode, Layer, LayerContent, PixelFormat, SampleEncoding, SampleType, SourceColorContext, WorkingProfileInterpretation, XcfState,
    item_identity::ItemRef, xcf_filters::FilterStack, xcf_state::ItemState,
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaKind, QuotaLease, QuotaLimits},
    },
};
use std::io::{Cursor, Seek, SeekFrom, Write};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
struct Writer {
    cursor: Cursor<Vec<u8>>,
    quota: BackingQuota,
    _fee: QuotaLease,
}
#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl Writer {
    fn new(q: &BackingQuota) -> Self {
        let max = 16usize << 20;
        let header = (std::mem::size_of::<Self>() + std::mem::size_of::<Memory>() + 256) as u64;
        let mut fee = q.reserve(QuotaKind::Resident, max as u64 + header).unwrap();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(max).unwrap();
        assert!(bytes.capacity() <= max);
        fee.reconcile_capacity(bytes.capacity() as u64 + header).unwrap();
        Self { cursor: Cursor::new(bytes), quota: q.clone(), _fee: fee }
    }
    fn finish(self) -> Arc<Memory> {
        Arc::new(Memory { bytes: self.cursor.into_inner(), quota: self.quota, _fee: self._fee })
    }
}
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.cursor.position().checked_add(b.len() as u64).is_none_or(|n| n > self.cursor.get_ref().capacity() as u64) {
            return Err(std::io::Error::other("paid output capacity"));
        }
        self.cursor.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for Writer {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(p)
    }
}
impl ImmutableCanonicalArchive for Memory {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let a = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}

fn manifest(memory: &Memory) -> crate::Manifest {
    let mut reader = crate::zip::stream::Reader::new(Cursor::new(memory.bytes.as_slice()), Default::default(), &Interrupt::NONE).unwrap();
    serde_json::from_slice(&reader.read_by_name("manifest.json", 1 << 20, &Interrupt::NONE).unwrap()).unwrap()
}
fn rewritten_header(memory: &Memory, q: &BackingQuota, edit: impl FnOnce(&mut serde_json::Value)) -> Arc<Memory> {
    // Source/output/manifest copies belong to this external harness; never claimed
    // as a production codec receipt. The returned writer has a distinct paid fee.
    let _harness = q.reserve(QuotaKind::Resident, 32 << 20).unwrap();
    let mut input = crate::zip::stream::Reader::new(Cursor::new(memory.bytes.as_slice()), Default::default(), &Interrupt::NONE).unwrap();
    let original = input.read_by_name("manifest.json", 1 << 20, &Interrupt::NONE).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    edit(&mut value);
    fn collect(value: &serde_json::Value, hashes: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::String(s) if s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()) => {
                hashes.insert(s.clone());
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    collect(value, hashes)
                }
            }
            serde_json::Value::Object(values) => {
                for value in values.values() {
                    collect(value, hashes)
                }
            }
            _ => {}
        }
    }
    let mut hashes = std::collections::BTreeSet::new();
    collect(&value, &mut hashes);
    let changed = serde_json::to_vec(&value).unwrap();
    let mut writer = crate::zip::stream::Writer::new_accounted(Writer::new(q), Default::default(), &ctl(), q).unwrap();
    writer.add_accounted("manifest.json", changed.len() as u64, &mut changed.as_slice(), &ctl()).unwrap();
    for hash in hashes {
        for prefix in ["blobs", "tiles"] {
            let path = format!("{prefix}/{hash}.zst");
            if input.contains(&path) {
                let bytes = input.read_by_name(&path, 16 << 20, &Interrupt::NONE).unwrap();
                writer.add_accounted(&path, bytes.len() as u64, &mut bytes.as_slice(), &ctl()).unwrap();
            }
        }
    }
    writer.finish_accounted(&ctl()).unwrap().finish()
}

#[derive(Debug)]
struct FixtureHold {
    // The model/native/profile payloads precede this holder in their owners.
    _index: QuotaLease,
    _resident: QuotaLease,
}
#[derive(Debug)]
struct ZeroStore {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    _hold: Arc<FixtureHold>,
}
impl NativePlaneStore for ZeroStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        out.fill(0);
        Ok(())
    }
}
fn current_empty(q: &BackingQuota, sample: SampleType) -> photocraft_doc::accounted_document::AccountedDocument {
    // All fixture model/profile/native-owner allocations occur after this real prehold.
    let hold = Arc::new(FixtureHold {
        // Closed one-root/one-empty-stack/one-item fixture: at most four short
        // strings, one four-byte ICC, and one native descriptor/store. This
        // 1MiB Index envelope also includes model headers and bounded spare capacity.
        _index: q.reserve(QuotaKind::Index, 1 << 20).unwrap(),
        _resident: q.reserve(QuotaKind::Resident, 8 << 20).unwrap(),
    });
    let mut doc = Document::new("derived empty", Size::new(1, 1), ColorMode::Rgb, sample);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: if sample == SampleType::F32 { 650 } else { 750 } });
    let icc = vec![1, 2, 3, 4];
    doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: blake3::hash(&icc).to_hex().to_string() };
    doc.icc_profile = Some(Arc::new(icc)); // opaque structural fixture, no CMS claim
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let desc = NativePlaneDescriptor::new(format, Rect::new(0, 0, 1, 1), &vec![0; format.bytes_per_pixel()]).unwrap();
    let native = BackedRaster::new(Arc::new(ZeroStore { descriptor: desc, quota: q.clone(), _hold: hold.clone() }), q.clone(), 1, &ctl()).unwrap();
    let layer = Layer::new("root", LayerContent::BackedRaster(BackedLayer::new(native, desc, &ctl()).unwrap()));
    let owner = ItemRef::Layer(layer.id);
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.items.push(ItemState {
        item: owner,
        name: layer.name.clone(),
        custom_name: false,
        visible: layer.visible,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        composition: None,
        mask: None,
    });
    state.filter_stacks.push(FilterStack { owner, revision: 17, nodes: vec![] });
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    photocraft_doc::accounted_document::AccountedDocument::new_readonly_filtered_flat_point_current_storage(doc, hold, q.clone(), &ctl()).unwrap()
}
fn load(memory: Arc<Memory>, q: &BackingQuota) -> Result<OwnedDocumentArchive> {
    load_document_from_owned_archive(memory, &Default::default(), Default::default(), q.clone(), &ctl())
}
#[test]
#[ignore = "UNRUN/uncompiled: legacy1..9 archives traverse both missing migration steps without inventing marker or changing ordinary minimums"]
fn legacy_migrations_and_closed_hint_schema() {
    let q = quota();
    let _external = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let doc = Document::with_background("legacy", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let raw = save_current_document_archive(&doc, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    assert_eq!(manifest(&raw).minimum_reader_version, 2);
    for version in 1..=9 {
        let changed = rewritten_header(&raw, &q, |v| {
            v["format_version"] = version.into();
            v["minimum_reader_version"] = if version == 1 { 1 } else { 2 }.into();
            v.as_object_mut().unwrap().remove("current_storage_hint");
        });
        assert!(crate::load_from_bytes(&changed.bytes).is_ok());
        let owned = load(changed, &q).unwrap();
        assert_eq!(owned.current_storage_hint(), None);
        assert_eq!(owned.document().layers.len(), 1);
    }
    for version in [8, 9] {
        let mut v = serde_json::json!({"format_version":version,"minimum_reader_version":2,"document":{}});
        crate::migrate::migrate(&mut v).unwrap();
        assert_eq!(v["format_version"], 11);
        assert!(v.get("current_storage_hint").is_none());
        assert!(crate::migrate::migrate(&mut serde_json::json!({"format_version":version})).is_err());
    }
    assert_eq!(serde_json::to_string(&crate::NativeCurrentStorageHint::FilteredFlatPoint).unwrap(), "\"filtered_flat_point\"");
    assert!(serde_json::from_str::<crate::NativeCurrentStorageHint>("\"unknown\"").is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: both float depths actual CURRENT owner marker/save/reopen, immutable archive retention, strict wrong handoff and unmarked Source refusal"]
fn current_owner_roundtrip_and_explicit_handoff() {
    for sample in [SampleType::F32, SampleType::F64] {
        let q = quota();
        let owner = current_empty(&q, sample);
        let memory = save_accounted_current_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
        let header = manifest(&memory);
        assert_eq!((header.format_version, header.minimum_reader_version), (10, 10));
        assert_eq!(header.current_storage_hint, Some(crate::NativeCurrentStorageHint::FilteredFlatPoint));
        assert!(matches!(crate::load_from_bytes(&memory.bytes),Err(FormatError::Unsupported(s)) if s.contains("owned handoff")));
        assert!(load(memory.clone(), &q).unwrap().into_accounted_document(&ctl()).is_err());
        assert!(load(memory.clone(), &q).unwrap().into_accounted_readonly_point_storage(&ctl()).is_err());
        let loaded = load(memory.clone(), &q).unwrap();
        let saved_again = loaded.save_archive(Writer::new(&q), Default::default(), &ctl()).unwrap().finish();
        assert_eq!(manifest(&saved_again).current_storage_hint, header.current_storage_hint);
        let weak = Arc::downgrade(&saved_again);
        let current = load(saved_again, &q).unwrap().into_accounted_readonly_filtered_flat_storage(&ctl()).unwrap();
        assert!(weak.upgrade().is_some());
        assert!(current.is_readonly_filtered_flat_point_storage());
        assert_eq!(current.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 17);
        assert!(current.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        let unmarked = save_current_document_archive(owner.document(), Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
        assert_eq!(manifest(&unmarked).current_storage_hint, None);
        assert!(load(unmarked, &q).unwrap().into_accounted_readonly_filtered_flat_storage(&ctl()).is_err());
        drop(current);
        assert!(weak.upgrade().is_none());
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: underdeclared/unknown markers reject; marker never grants structural custody; shared work, cancellation and ledger identity remain effective"]
fn malformed_markers_and_controls() {
    let q = quota();
    let owner = current_empty(&q, SampleType::F32);
    let memory = save_accounted_current_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    for kind in 0..3 {
        let changed = rewritten_header(&memory, &q, |v| match kind {
            0 => v["minimum_reader_version"] = 9.into(),
            1 => v["format_version"] = 9.into(),
            _ => v["current_storage_hint"] = "unknown".into(),
        });
        assert!(load(changed, &q).is_err());
    }
    let other = quota();
    assert!(save_accounted_current_document_archive(&owner, Writer::new(&other), Default::default(), &other, &ctl()).is_err());
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(save_accounted_current_document_archive(&owner, Writer::new(&q), Default::default(), &q, &deny).is_err());
    let interrupted = Interrupt::new(&|| true, &|_| {});
    let cancel = ReadControl { interrupt: &interrupted, work: &|_| Ok(()) };
    assert!(matches!(
        load_document_from_owned_archive(memory.clone(), &Default::default(), Default::default(), q.clone(), &cancel),
        Err(FormatError::Cancelled)
    ));
    let _external = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let plain = Document::with_background("no capability", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let raw = save_current_document_archive(&plain, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    let forged = rewritten_header(&raw, &q, |v| {
        v["format_version"] = 10.into();
        v["minimum_reader_version"] = 10.into();
        v["current_storage_hint"] = "filtered_flat_point".into();
    });
    let loaded = load(forged, &q).unwrap();
    assert_eq!(loaded.current_storage_hint(), Some(crate::NativeCurrentStorageHint::FilteredFlatPoint));
    assert!(loaded.into_accounted_readonly_filtered_flat_storage(&ctl()).is_err());
}

#[path = "point_f32_coverage_tests.rs"]
mod point_f32_coverage_tests;
