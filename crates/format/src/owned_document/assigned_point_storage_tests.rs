//! UNRUN structural native handoff drafts. Opaque profile bytes deliberately
//! establish no ICC semantics, source qualification or rendering permission.
use super::*;
use photocraft_doc::{
    BackedLayer, ColorMode, Layer, LayerContent, PixelFormat, SampleEncoding, SampleType, SourceColorContext, WorkingProfileInterpretation, XcfState,
    item_identity::ItemRef, xcf_filters::FilterStack, xcf_state::ItemState,
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
    },
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 16 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
}
impl NativePlaneStore for Store {
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
        let default = self.descriptor.default_bytes();
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = default[(offset as usize + i) % default.len()];
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _fee: QuotaLease,
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
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn source(q: &BackingQuota) -> Arc<Memory> {
    // This retained fixture envelope precedes all small fixture allocations.
    let model_fee = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    let bounds = Rect::new(0, 0, 8, 6);
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let words = [(-0.0f32).to_ne_bytes(), 0.25f32.to_ne_bytes(), 0.75f32.to_ne_bytes(), 1.0f32.to_ne_bytes()].concat();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &words).unwrap();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, quota: q.clone() }), q.clone(), 1, &ctl()).unwrap();
    let layer = Layer::new("structural assigned owner", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
    let reference = ItemRef::Layer(layer.id);
    let mut doc = Document::new("assigned native handoff", Size::new(8, 6), ColorMode::Rgb, SampleType::F32);
    doc.layers.push(layer);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    doc.icc_profile = Some(Arc::new(b"current structural bytes; no ICC admission".to_vec()));
    doc.source_color_context =
        Some(SourceColorContext { original_icc: Some(Arc::new(b"original structural bytes; independent owner".to_vec())), source_precision: 650 });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.provenance.source_hash = Some("a".repeat(64));
    state.items.push(ItemState {
        item: reference,
        name: "structural assigned owner".into(),
        custom_name: false,
        visible: true,
        locks: Default::default(),
        parent: None,
        source_tattoo: Some(7),
        source_flags: 0,
        raster_bounds: Some(bounds),
        composition: None,
        mask: None,
    });
    // An empty structural stack is intentionally not an executable Box recipe.
    state.filter_stacks.push(FilterStack { owner: reference, revision: 1, nodes: Vec::new() });
    state.selection.selected.push(reference);
    state.selection.active = Some(reference);
    doc.xcf_state = Some(state);
    let bytes = save_current_document_archive(&doc, std::io::Cursor::new(Vec::new()), Default::default(), q, &ctl()).unwrap().into_inner();
    let fee = q.reserve(QuotaKind::Resident, bytes.capacity() as u64 + 128).unwrap();
    drop(doc);
    drop(model_fee);
    Arc::new(Memory { bytes, quota: q.clone(), _fee: fee })
}
fn load(source: Arc<Memory>, q: &BackingQuota) -> OwnedDocumentArchive {
    load_document_from_owned_archive(source, &LoadOptions::default(), Default::default(), q.clone(), &ctl()).unwrap()
}
#[test]
fn explicit_assigned_handoff_retains_archive_both_profile_roles_and_native_words() {
    let q = quota();
    let archive = source(&q);
    let weak = Arc::downgrade(&archive);
    // A serialized original ICC does not broaden the old implicit endpoint.
    assert!(load(archive.clone(), &q).into_accounted_readonly_point_storage(&ctl()).is_err());
    let native = load(archive.clone(), &q);
    let current = native.document().icc_profile.as_ref().unwrap().clone();
    let original = native.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().clone();
    assert!(!Arc::ptr_eq(&current, &original));
    let owner = native.into_accounted_readonly_assigned_point_storage(&ctl()).unwrap();
    assert!(owner.is_readonly_assigned_point_storage());
    assert!(!owner.is_readonly_point_storage());
    owner.validate_readonly_assigned_point_storage(&ctl()).unwrap();
    assert!(owner.validate_readonly_point_storage(&ctl()).is_err());
    assert!(owner.quota().shares_ledger(&q));
    assert!(Arc::ptr_eq(&current, owner.document().icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(&original, owner.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()));
    drop(current);
    drop(original);
    drop(archive);
    assert!(weak.upgrade().is_some());
    let LayerContent::BackedRaster(layer) = &owner.document().layers[0].content else { panic!() };
    let mut pixel = [0; 16];
    layer.checked_view().read_logical_pixel_native_into(3, 2, &mut pixel, &ctl()).unwrap();
    assert_eq!(&pixel[..4], &(-0.0f32).to_ne_bytes());
    assert_eq!(&pixel[12..], &1.0f32.to_ne_bytes());
    drop(owner);
    assert!(weak.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn assigned_handoff_failure_refunds_and_cannot_select_an_implicit_profile_policy() {
    let q = quota();
    let archive = source(&q);
    let baseline = q.usage().unwrap();
    let deny = |_: u64| Err(BackingError::Cancelled);
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(matches!(load(archive.clone(), &q).into_accounted_readonly_assigned_point_storage(&cancelled), Err(FormatError::Cancelled)));
    assert_eq!(q.usage().unwrap(), baseline);
    let native = load(archive.clone(), &q);
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - q.usage().unwrap().index).unwrap();
    assert!(native.into_accounted_readonly_assigned_point_storage(&ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), baseline);
    for missing_original in [true, false] {
        let mut native = load(archive.clone(), &q);
        let edit_fee = q.reserve(QuotaKind::Resident, 4096).unwrap();
        if missing_original {
            native.document.source_color_context.as_mut().unwrap().original_icc = None;
        } else {
            native.document.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(native.document.icc_profile.as_ref().unwrap());
        }
        assert!(native.into_accounted_readonly_assigned_point_storage(&ctl()).is_err());
        drop(edit_fee);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
