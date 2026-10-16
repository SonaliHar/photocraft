//! Deferred codec-only synthetic CURRENT custody. No Source/CMS/render grant.
use super::*;
use photocraft_doc::{
    BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleEncoding, SampleType, SourceColorContext, Surface, WorkingProfileInterpretation,
    XcfState,
    accounted_document::{AccountedDocument, ProgrammeNativeStorageKind},
    item_identity::ItemRef,
    xcf_filters::FilterStack,
    xcf_state::ItemState,
};
use photocraft_geom::{Rect, Size, TileCoord};
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

#[derive(Debug)]
struct Hold {
    _index: QuotaLease,
    _resident: QuotaLease,
}
#[derive(Debug)]
struct Zero {
    descriptor: NativePlaneDescriptor,
    q: BackingQuota,
    _hold: Arc<Hold>,
}
impl NativePlaneStore for Zero {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        out.fill(0);
        Ok(())
    }
}
fn raw_fixture(q: &BackingQuota, sample: SampleType, gray8: bool) -> (Document, Arc<Hold>) {
    // This fixed one-root/one-item/one-stack, one selection tile model is <1MiB
    // before spare capacities; both real preholds precede all model/native data.
    let hold = Arc::new(Hold { _index: q.reserve(QuotaKind::Index, 1 << 20).unwrap(), _resident: q.reserve(QuotaKind::Resident, 8 << 20).unwrap() });
    let mut d = Document::new("selected codec CURRENT", Size::new(1, 1), ColorMode::Rgb, sample);
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: if sample == SampleType::F32 { 650 } else { 750 } });
    let icc = vec![1, 2, 3, 4];
    d.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: blake3::hash(&icc).to_hex().to_string() };
    d.icc_profile = Some(Arc::new(icc)); // structural hash only, no CMS qualification
    let f = PixelFormat::new(ColorMode::Rgb, sample, true);
    let descriptor = NativePlaneDescriptor::new(f, d.bounds(), &vec![0; f.bytes_per_pixel()]).unwrap();
    let native = BackedRaster::new(Arc::new(Zero { descriptor, q: q.clone(), _hold: hold.clone() }), q.clone(), 1, &ctl()).unwrap();
    let l = Layer::new("root", LayerContent::BackedRaster(BackedLayer::new(native, descriptor, &ctl()).unwrap()));
    let item = ItemRef::Layer(l.id);
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.items.push(ItemState {
        item,
        name: l.name.clone(),
        custom_name: false,
        visible: l.visible,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(d.bounds()),
        composition: None,
        mask: None,
    });
    state.filter_stacks.push(FilterStack { owner: item, revision: 17, nodes: vec![] });
    d.layers.push(l);
    d.xcf_state = Some(state);
    let sf = PixelFormat::new(ColorMode::Grayscale, if gray8 { SampleType::U8 } else { sample }, false);
    let default = if gray8 {
        vec![64]
    } else if sample == SampleType::F32 {
        0.25f32.to_ne_bytes().to_vec()
    } else {
        0.25000000000000006f64.to_ne_bytes().to_vec()
    };
    let pixel = if gray8 {
        vec![128]
    } else if sample == SampleType::F32 {
        0.50000006f32.to_ne_bytes().to_vec()
    } else {
        0.5000000000000001f64.to_ne_bytes().to_vec()
    };
    let mut surface = Surface::try_with_default_bytes(sf, &default).unwrap();
    for p in surface.try_tile_mut(TileCoord::new(0, 0)).unwrap().bytes_mut().chunks_exact_mut(sf.bytes_per_pixel()) {
        p.copy_from_slice(&pixel);
    }
    if !gray8 {
        let zero = if sample == SampleType::F32 { (-0f32).to_ne_bytes().to_vec() } else { (-0f64).to_ne_bytes().to_vec() };
        surface.try_tile_mut(TileCoord::new(0, 0)).unwrap().bytes_mut()[..zero.len()].copy_from_slice(&zero);
    }
    d.selection = Some(surface);
    (d, hold)
}
fn selected(q: &BackingQuota, sample: SampleType, gray8: bool) -> AccountedDocument {
    let (d, hold) = raw_fixture(q, sample, gray8);
    AccountedDocument::from_owned_programme_rehydrating_selection(ProgrammeNativeStorageKind::FilteredFlatPoint, d, hold, q.clone(), &ctl()).unwrap()
}
fn load(m: Arc<Memory>, q: &BackingQuota, c: &ReadControl<'_>) -> Result<OwnedDocumentArchive> {
    load_document_from_owned_archive(m, &Default::default(), Default::default(), q.clone(), c)
}
#[test]
#[ignore = "UNRUN/uncompiled: codec-only selected CURRENT exact native selection/default/padding, actual same-class guard rehydration, marker preservation and archive lifetime; not Source/CMS/render evidence"]
fn selected_current_native_storage_and_guard_roundtrip() {
    for sample in [SampleType::F32, SampleType::F64] {
        for gray8 in [false, true] {
            let q = quota();
            let owner = selected(&q, sample, gray8);
            assert!(owner.is_readonly_filtered_flat_point_storage());
            assert!(!owner.is_legacy_storage());
            {
                let proof = owner.checked_programme_selection(&ctl()).unwrap();
                assert!(proof.matches_owner(&owner));
                assert!(proof.matches_document(owner.document()));
                assert_eq!(proof.storage_kind(), ProgrammeNativeStorageKind::FilteredFlatPoint);
            }
            let memory = save_accounted_current_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
            assert!(load(memory.clone(), &q, &ctl()).unwrap().into_accounted_document(&ctl()).is_err());
            let archive = load(memory.clone(), &q, &ctl()).unwrap();
            assert_eq!(archive.current_storage_hint(), Some(crate::NativeCurrentStorageHint::FilteredFlatPoint));
            assert_eq!(archive.document().selection, owner.document().selection);
            let rewritten = archive.save_archive(Writer::new(&q), Default::default(), &ctl()).unwrap().finish();
            drop(archive);
            let reopened = load(rewritten.clone(), &q, &ctl()).unwrap().into_accounted_readonly_filtered_flat_storage(&ctl()).unwrap();
            assert!(reopened.is_readonly_filtered_flat_point_storage());
            assert!(!reopened.is_legacy_storage());
            assert_eq!(reopened.document().selection, owner.document().selection);
            assert_eq!(reopened.document().xcf_state, owner.document().xcf_state);
            assert_eq!(reopened.document().source_color_context, owner.document().source_color_context);
            assert_eq!(reopened.document().icc_profile, owner.document().icc_profile);
            {
                let p = reopened.checked_programme_selection(&ctl()).unwrap();
                assert!(p.matches_owner(&reopened));
                assert!(!p.matches_owner(&owner));
            }
            drop(owner);
            drop(memory);
            drop(rewritten);
            assert!(q.usage().unwrap().resident > 0);
            drop(reopened);
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().index, 0);
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: direct typed selected native handoff original Work/cancel/Index controls, unmarked Source refusal and generic Document refusal; no Legacy fallback or source authority"]
fn selected_handoff_controls_and_unmarked_source_fail_closed() {
    let q = quota();
    let owner = selected(&q, SampleType::F64, false);
    let memory = save_accounted_current_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(load(memory.clone(), &q, &ctl()).unwrap().into_accounted_readonly_filtered_flat_storage(&denied).is_err());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(load(memory.clone(), &q, &ctl()).unwrap().into_accounted_readonly_filtered_flat_storage(&cancelled), Err(FormatError::Cancelled)));
    let archive = load(memory.clone(), &q, &ctl()).unwrap();
    let usage = q.usage().unwrap();
    let index = q.reserve(QuotaKind::Index, (32 << 20) - usage.index).unwrap();
    assert!(archive.into_accounted_readonly_filtered_flat_storage(&ctl()).is_err());
    drop(index);
    let raw = save_current_document_archive(owner.document(), Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    assert_eq!(load(raw.clone(), &q, &ctl()).unwrap().current_storage_hint(), None);
    assert!(load(raw.clone(), &q, &ctl()).unwrap().into_accounted_readonly_filtered_flat_storage(&ctl()).is_err());
    assert!(load(raw, &q, &ctl()).unwrap().into_accounted_document(&ctl()).is_err());
    assert!(owner.checked_programme_selection(&ctl()).is_ok());
}
