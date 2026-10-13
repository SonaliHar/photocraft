//! Controlled resident adoption/view restoration, not source semantic qualification.
use photocraft_doc::{
    AlphaChannel, Color, ColorMode, Document, LayerMask, PixelFormat, SampleType, Size, XcfState,
    item_identity::{ItemRef, MaskId},
    xcf_state::{ItemState, MaskState},
};
use photocraft_engine::{
    DocState, DocumentSnapshot, EngineError,
    channel_cmds::{ChannelTarget, imported_editing_mask_with_interrupt, imported_view_with_interrupt},
};
use photocraft_raster::{Interrupt, Surface, backing::BackingError};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn item(reference: ItemRef, visible: bool) -> ItemState {
    ItemState {
        item: reference,
        name: "Own item".into(),
        custom_name: false,
        visible,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: None,
        mask: None,
        composition: None,
    }
}
fn fixture() -> Document {
    let mut doc = Document::with_background("Own restoration", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.layers[0].mask = Some(LayerMask::hide_all());
    let channel = AlphaChannel::new("Own alpha", Surface::new(PixelFormat::GRAY8));
    let cid = channel.id;
    doc.channels.push(channel);
    let layer = doc.layers[0].id;
    let mut state = XcfState::default();
    state.items.push(item(ItemRef::Layer(layer), true));
    state.items.push(item(ItemRef::Channel(cid), false));
    let mut mask = item(ItemRef::Mask(MaskId(1)), true);
    mask.mask = Some(MaskState {
        owner: ItemRef::Layer(layer),
        bounds: doc.bounds(),
        offset: [0, 0],
        outside_f64_bits: 0f64.to_bits(),
        enabled: true,
        shown: false,
        editing: true,
        linked: true,
    });
    state.items.push(mask);
    state.selection.selected = vec![ItemRef::Layer(layer)];
    state.selection.active = Some(ItemRef::Layer(layer));
    state.selection.mask_target = Some(ItemRef::Mask(MaskId(1)));
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn controlled_and_legacy_adoption_preserve_active_mask_selection_history_view() {
    let doc = fixture();
    let old = DocState::new(doc.clone(), Some("inert/path".into())).unwrap();
    let current = DocState::new_with_interrupt(doc, Some("inert/path".into()), &Interrupt::NONE).unwrap();
    assert_eq!(current.active_layer, old.active_layer);
    assert_eq!(current.selected_layers, old.selected_layers);
    assert_eq!(current.channel_view, old.channel_view);
    assert_eq!(current.layer_anchor, old.layer_anchor);
    assert_eq!(current.path, old.path);
    assert_eq!(current.doc, old.doc);
    assert_eq!(current.channel_view.editing_mask, current.active_layer);
}
#[test]
fn first_channel_position_visibility_or_and_duplicate_selection_precedence_are_preserved() {
    let mut doc = fixture();
    let channel = doc.channels[0].id;
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.push(item(ItemRef::Channel(channel), true));
    state.selection.active = Some(ItemRef::Channel(channel));
    state.selection.selected = vec![ItemRef::Channel(channel)];
    let view = imported_view_with_interrupt(&doc, &Interrupt::NONE).unwrap();
    assert_eq!(view.alpha_visible, vec![true]);
    assert_eq!(view.target, ChannelTarget::Alpha(0));
    assert_eq!(view.editing_mask, None);
    doc.xcf_state.as_mut().unwrap().selection.selected.push(ItemRef::Channel(channel));
    let view = imported_view_with_interrupt(&doc, &Interrupt::NONE).unwrap();
    assert_eq!(view.target, ChannelTarget::Composite);
    assert_eq!(view.editing_mask, Some(doc.layers[0].id));
    // Explicit non-layer-mask target prevents fallback to an otherwise editing mask.
    doc.xcf_state.as_mut().unwrap().selection.mask_target = Some(ItemRef::Channel(channel));
    assert_eq!(imported_editing_mask_with_interrupt(&doc, &Interrupt::NONE).unwrap(), None);
}
#[test]
fn cancelled_structural_view_adoption_preserves_caller_owned_document() {
    let doc = Arc::new(fixture());
    let original = doc.clone();
    let stop = || true;
    assert_eq!(DocumentSnapshot::from_resident_with_interrupt(doc.clone(), &Interrupt::cancel_only(&stop)).unwrap_err(), BackingError::Cancelled);
    assert!(matches!(DocState::new_with_interrupt(doc.as_ref().clone(), None, &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
    assert_eq!(*doc, *original);
    let polls = AtomicUsize::new(0);
    let check = || polls.fetch_add(1, Ordering::Relaxed) >= 4;
    assert!(matches!(DocState::new_with_interrupt(doc.as_ref().clone(), None, &Interrupt::cancel_only(&check)), Err(EngineError::Cancelled)));
    assert_eq!(*doc, *original);
}

#[test]
fn controlled_resident_constructor_rejects_backed_layer_without_native_read_or_detachment() {
    use photocraft_doc::{BackedLayer, Layer, LayerContent};
    use photocraft_geom::Rect;
    use photocraft_raster::backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    #[derive(Debug)]
    struct NoRead {
        descriptor: NativePlaneDescriptor,
        reads: Arc<AtomicUsize>,
    }
    impl NativePlaneStore for NoRead {
        fn descriptor(&self) -> NativePlaneDescriptor {
            self.descriptor
        }
        fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Err(BackingError::Io)
        }
    }
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0, 0, 0, 0]).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 0, index: 1 << 20 });
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let raster = BackedRaster::new(Arc::new(NoRead { descriptor, reads: reads.clone() }), quota, 1, &ctl).unwrap();
    let backed = BackedLayer::new(raster, descriptor, &ctl).unwrap();
    let mut doc = Document::new("Backed refusal", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("Backed", LayerContent::BackedRaster(backed)));
    let source = Arc::new(doc);
    assert_eq!(DocumentSnapshot::from_resident_with_interrupt(source.clone(), &Interrupt::NONE).unwrap_err(), BackingError::Unsupported);
    assert!(DocState::new_with_interrupt(source.as_ref().clone(), None, &Interrupt::NONE).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(matches!(source.layers[0].content, LayerContent::BackedRaster(_)));
}
