//! UNRUN structural imported-current tests. These are paid synthetic storage
//! fixtures, not original-XCF, profile or renderer evidence. Expected raw model
//! clones and byte buffers belong to the external test harness.
use super::*;
use crate::{ColorMode, Layer, PixelFormat, SampleType, Size, SourceColorContext, xcf_state::*};
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicU64, Ordering};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: Arc<AtomicU64>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).ok_or(BackingError::Bounds)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Length);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        for (i, b) in output.iter_mut().enumerate() {
            *b = ((offset + i as u64) % 251) as u8;
        }
        Ok(())
    }
}
fn fixture(format: PixelFormat) -> (AccountedDocument, Arc<AtomicU64>) {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 8 << 20, scratch: 0 });
    // Prospective original DTO/Arc/string capacity fees precede construction.
    let source_index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let source_payload = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let mut doc = Document::new("original", Size::new(300, 280), format.mode, format.sample);
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.original_locator = Some("inert/source.xcf".into());
    for (index, bounds) in [Rect::new(-3, -2, 260, 259), Rect::new(4, 5, 15, 17)].into_iter().enumerate() {
        let descriptor = NativePlaneDescriptor::new(format, bounds, &[0x80; 40][..format.bytes_per_pixel()]).unwrap();
        let raster = BackedRaster::new(Arc::new(Store { descriptor, reads: reads.clone() }), q.clone(), 2, &ctl()).unwrap();
        let layer = Layer::new(format!("native{index}"), LayerContent::BackedRaster(crate::BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
        state.items.insert(
            0,
            ItemState {
                item: ItemRef::Layer(layer.id),
                name: layer.name.clone(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: Some(index as u32 + 1),
                source_flags: 0,
                raster_bounds: Some(bounds),
                mask: None,
                composition: None,
            },
        );
        state.selection.selected.push(ItemRef::Layer(layer.id));
        doc.layers.push(layer);
    }
    state.selection.active = Some(ItemRef::Layer(doc.layers[0].id));
    doc.xcf_state = Some(state);
    doc.icc_profile = Some(Arc::new(vec![7; 132]));
    doc.source_color_context = Some(SourceColorContext { source_precision: 650, original_icc: Some(Arc::new(vec![9; 132])) });
    let owner = AccountedDocument::from_owned(doc, Arc::new((source_index, source_payload)), q, &ctl()).unwrap();
    (owner, reads)
}
fn leaf(owner: &AccountedDocument, index: usize) -> &crate::BackedLayer {
    let LayerContent::BackedRaster(v) = &owner.document().layers[index].content else { panic!("fixture") };
    v
}

#[test]
fn both_targets_and_native_depths_change_only_view_and_exact_canonical_bounds() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let format = PixelFormat { mode, sample, alpha: true };
            let (original, reads) = fixture(format);
            for index in 0..2 {
                let id = original.document().layers[index].id;
                original.validate_imported_layer_translation(id, -257, 255, &ctl()).unwrap();
                let moved = original.try_with_imported_layer_translation(id, -257, 255, &ctl()).unwrap();
                assert_eq!(reads.load(Ordering::Relaxed), 0); // No native data read during model preparation.
                let mut expected = original.document().clone();
                expected.layers[index].content = LayerContent::BackedRaster(leaf(&moved, index).clone());
                expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().raster_bounds =
                    Some(leaf(&moved, index).operative_bounds());
                assert_eq!(moved.document(), &expected);
                assert_eq!(leaf(&moved, index).operative_bounds().x0, leaf(&original, index).operative_bounds().x0 - 257);
                assert!(moved.quota().shares_ledger(original.quota()));
                assert!(Arc::ptr_eq(&moved.owner.retention, &original.owner.retention));
                assert!(Arc::ptr_eq(moved.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
                assert!(Arc::ptr_eq(
                    moved.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
                    original.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
                ));
                assert_eq!(leaf(&moved, index).default_bytes(), leaf(&original, index).default_bytes());
            }
        }
    }
}

#[test]
fn repeated_moves_title_continuation_and_native_cow_keep_flat_source_retention() {
    let (original, _) = fixture(PixelFormat::RGBA64F);
    let id = original.document().layers[0].id;
    let mut current = original.clone();
    for _ in 0..8 {
        current = current.try_with_imported_layer_translation(id, -1, 1, &ctl()).unwrap();
    }
    let named = current.try_with_document_name("continued.pcraft", &ctl()).unwrap();
    assert!(Arc::ptr_eq(&named.owner.retention, &original.owner.retention));
    let bytes = [(-0.0f64).to_ne_bytes(), 0.10000000000000002f64.to_ne_bytes(), 1.125f64.to_ne_bytes(), (-0.0f64).to_ne_bytes()].concat();
    let rect = Rect::new(-8, 8, -7, 9);
    let written = named.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &bytes }, &ctl()).unwrap();
    let returned = written.try_with_imported_layer_translation(id, 8, -8, &ctl()).unwrap();
    let mut out = [0; 32];
    leaf(&returned, 0).checked_view().read_pixel_native_into(0, 0, &mut out, &ctl()).unwrap();
    assert_eq!(&out[..], bytes);
    assert_ne!(leaf(&original, 0), leaf(&returned, 0));
    assert_eq!(leaf(&original, 1), leaf(&returned, 1));
    assert!(Arc::ptr_eq(&returned.owner.retention, &original.owner.retention));
}

#[test]
fn valid_zero_move_succeeds_at_full_quota_and_failure_never_changes_original() {
    let (original, reads) = fixture(PixelFormat::RGBA32F);
    let id = original.document().layers[0].id;
    let q = original.quota();
    let baseline = q.usage().unwrap();
    let full = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    let zero = original.try_with_imported_layer_translation(id, 0, 0, &ctl()).unwrap();
    assert!(zero.downgrade().ptr_eq(&original.downgrade()));
    assert!(matches!(original.try_with_imported_layer_translation(id, 1, 0, &ctl()), Err(BackingError::Quota)));
    drop(full);
    assert_eq!(q.usage().unwrap(), baseline);
    for failure in [BackingError::Work, BackingError::Cancelled] {
        let work = |_: u64| Err(failure);
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(original.try_with_imported_layer_translation(id, 1, 0, &c).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let late = |_: u64| if q.usage().map_err(|_| BackingError::Integrity)?.index > baseline.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(matches!(
        original.try_with_imported_layer_translation(id, 1, 0, &ReadControl { interrupt: &Interrupt::NONE, work: &late }),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(original.try_with_imported_layer_translation(id, i32::MIN, 0, &ctl()).is_err());
    assert!(original.try_with_imported_layer_translation(LayerId(0), 0, 0, &ctl()).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[test]
fn malformed_current_half_context_and_filtered_or_selected_models_refuse_even_zero() {
    let (original, _) = fixture(PixelFormat::RGBA32F);
    let id = original.document().layers[0].id;
    for case in 0..6 {
        let q = original.quota().clone();
        let index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
        let payload = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
        let mut doc = original.document().clone(); // Test-only synthetic malformed current.
        match case {
            0 => doc.source_color_context = None,
            1 => doc.xcf_state = None,
            2 => doc.xcf_state.as_mut().unwrap().items[0].name = "divergent".into(),
            3 => doc.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(Rect::new(0, 0, 1, 1)),
            4 => doc.xcf_state.as_mut().unwrap().filter_stacks.push(crate::xcf_filters::FilterStack {
                owner: ItemRef::Layer(id),
                nodes: Vec::new(),
                revision: 0,
            }),
            _ => doc.selection_channel_id = Some(crate::item_identity::ChannelId(1)),
        }
        let owner = AccountedDocument::from_owned(doc, Arc::new((original.clone(), index, payload)), q, &ctl()).unwrap();
        assert!(owner.try_with_imported_layer_translation(id, 0, 0, &ctl()).is_err());
    }
}
