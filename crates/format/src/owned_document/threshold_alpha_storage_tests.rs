//! UNRUN actual archive handoff/storage-lifetime and closed-class refusal drafts.
use super::*;
use photocraft_doc::{
    BackedLayer, ColorMode, Layer, LayerContent, PixelFormat, SampleEncoding, SampleType, SourceColorContext, Surface, XcfState,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState},
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
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        control.check(output.len() as u64)?;
        let default = self.0.default_bytes();
        for (i, out) in output.iter_mut().enumerate() {
            *out = default[(offset as usize + i) % default.len()];
        }
        Ok(())
    }
}
fn fixture(q: &BackingQuota) -> (Document, QuotaLease) {
    // Real source fixture model/native metadata is paid before construction.
    let guard = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    let bounds = Rect::new(0, 0, 8, 6);
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let descriptor = NativePlaneDescriptor::new(format, bounds, &(-0.0f64).to_ne_bytes().repeat(4)).unwrap();
    let raster = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &ctl()).unwrap();
    let layer = Layer::new("exact current source", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
    let id = layer.id;
    let owner = ItemRef::Layer(id);
    let mut doc = Document::new("readonly native archive", Size::new(8, 6), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(layer);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 750 });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.provenance.source_hash = Some("a".repeat(64));
    let filter = FilterId(100);
    let mask = MaskId(101);
    state.filter_stacks.push(FilterStack {
        owner,
        revision: 7,
        nodes: vec![FilterNode {
            id: filter,
            operation: OperationIdentity {
                name: "gimp:threshold-alpha".into(),
                stored_version: "0:0".into(),
                schema_epoch: "xcf-22-stock-threshold-alpha-precision-v1".into(),
            },
            source_display_name: Some("Original threshold".into()),
            source_icon: Some("inert icon".into()),
            arguments: vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())) }],
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: Some(2),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: None,
        }],
    });
    let mut surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0.0f64).to_ne_bytes()).unwrap();
    surface.try_write_pixel_f64(0, 0, &[0.5 + 2f64.powi(-40)]).unwrap();
    state.filter_masks.push(photocraft_doc::FilterMask { id: mask, owner: filter, bounds, surface });
    state.items.push(ItemState {
        item: ItemRef::Mask(mask),
        name: "retained selection mask".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(17),
        source_flags: 0,
        raster_bounds: Some(bounds),
        composition: None,
        mask: Some(MaskState { owner, bounds, offset: [0, 0], outside_f64_bits: 0.0f64.to_bits(), enabled: true, shown: false, editing: false, linked: false }),
    });
    state.items.push(ItemState {
        item: owner,
        name: "exact current source".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(18),
        source_flags: 0,
        raster_bounds: Some(bounds),
        mask: None,
        composition: None,
    });
    state.selection.selected.push(owner);
    state.selection.active = Some(owner);
    doc.xcf_state = Some(state);
    (doc, guard)
}
#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Memory {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        control.check(output.len() as u64)?;
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(output.len()).ok_or(BackingError::Bounds)?;
        output.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
        Ok(())
    }
}
fn source(q: &BackingQuota) -> Arc<Memory> {
    let (doc, guard) = fixture(q);
    let bytes = save_current_document_archive(&doc, std::io::Cursor::new(Vec::new()), Default::default(), q, &ctl()).unwrap().into_inner();
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64 + 128).unwrap();
    drop(doc);
    drop(guard);
    Arc::new(Memory { bytes, quota: q.clone(), _lease: lease })
}
fn load(source: Arc<Memory>, q: &BackingQuota) -> OwnedDocumentArchive {
    load_document_from_owned_archive(source, &LoadOptions::default(), Default::default(), q.clone(), &ctl()).unwrap()
}

#[test]
fn explicit_handoff_keeps_actual_archive_native_mask_words_and_legacy_route_closed() {
    let q = quota();
    let original = source(&q);
    let weak = Arc::downgrade(&original);
    let native = load(original.clone(), &q);
    let state = native.document().xcf_state.as_ref().unwrap();
    let expected = state.filter_masks[0].surface.try_read_interleaved(Rect::new(0, 0, 8, 6)).unwrap();
    let original_tile = state.filter_masks[0].surface.tiles().next().unwrap().1;
    let tile_pointer = Arc::as_ptr(original_tile);
    assert_eq!(original_tile.bytes().get(..8), Some((0.5 + 2f64.powi(-40)).to_ne_bytes().as_slice()));
    for word in original_tile.bytes().chunks_exact(8).skip(1) {
        assert_eq!(word, (-0.0f64).to_ne_bytes());
    }
    assert_eq!(state.filter_masks[0].surface.default_bytes(), (-0.0f64).to_ne_bytes());
    assert_eq!(state.filter_stacks[0].nodes[0].arguments[0].value, FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())));
    let legacy = load(original.clone(), &q).into_accounted_document(&ctl()).unwrap();
    // Existing codec-paid adoption remains valid, but serialized filter fields
    // do not automatically select the new runtime-only storage class.
    assert!(legacy.validate_readonly_threshold_alpha_storage(&ctl()).is_err());
    drop(legacy);
    let owned = native.into_accounted_readonly_threshold_alpha_storage(&ctl()).unwrap();
    owned.validate_readonly_threshold_alpha_storage(&ctl()).unwrap();
    assert!(owned.quota().shares_ledger(&q));
    assert_eq!(Arc::as_ptr(owned.document().xcf_state.as_ref().unwrap().filter_masks[0].surface.tiles().next().unwrap().1), tile_pointer);
    assert_eq!(owned.document().xcf_state.as_ref().unwrap().filter_masks[0].surface.try_read_interleaved(Rect::new(0, 0, 8, 6)).unwrap(), expected);
    drop(original);
    assert!(weak.upgrade().is_some());
    let LayerContent::BackedRaster(layer) = &owned.document().layers[0].content else { panic!() };
    let mut pixel = [0; 32];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &ctl()).unwrap();
    assert_eq!(pixel.as_slice(), (-0.0f64).to_ne_bytes().repeat(4));
    drop(owned);
    assert!(weak.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn selection_and_unsupported_recipe_fail_without_restoring_runtime_authority() {
    let q = quota();
    let original = source(&q);
    let baseline = q.usage().unwrap();
    for selected in [true, false] {
        let mut native = load(original.clone(), &q);
        let edit_fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
        if selected {
            native.document.selection = Some(Surface::new(PixelFormat::GRAY8));
        } else {
            native.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.name = "unsupported:retained".into();
        }
        assert!(native.into_accounted_readonly_threshold_alpha_storage(&ctl()).is_err());
        drop(edit_fee);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}

#[test]
fn cancellation_and_owner_header_quota_failure_drop_archive_payload_before_fees() {
    let q = quota();
    let original = source(&q);
    let baseline = q.usage().unwrap();
    let native = load(original.clone(), &q);
    let deny = |_: u64| Err(BackingError::Cancelled);
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(matches!(native.into_accounted_readonly_threshold_alpha_storage(&cancelled), Err(FormatError::Cancelled)));
    assert_eq!(q.usage().unwrap(), baseline);
    let native = load(original.clone(), &q);
    let used = q.usage().unwrap().resident;
    let blocker = q.reserve(QuotaKind::Resident, (128 << 20) - used).unwrap();
    assert!(native.into_accounted_readonly_threshold_alpha_storage(&ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), baseline);
}
