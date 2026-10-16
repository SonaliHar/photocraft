//! Deferred structural contracts only. No operation/source/CMS qualification.
use super::*;
use crate::{
    Document, Layer, LayerContent, PixelFormat, SampleType, XcfState,
    item_identity::ItemRef,
    xcf_filters::{FilterNode, FilterStack, OperationIdentity},
    xcf_state::ItemState,
};
use photocraft_geom::Size;
use photocraft_raster::backing::{BackingError, ReadControl};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn mask(value: f64) -> FilterMask {
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
    surface.try_write_pixel_f64(0, 0, &[value]).unwrap();
    FilterMask { id: MaskId(2), owner: FilterId(3), surface, bounds: Rect::new(0, 0, 1, 1) }
}
fn doc(mask: FilterMask) -> Document {
    // External tiny fixture only; no AccountedDocument or source claim.
    let mut doc = Document::new("selected structural fixture", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    let layer = Layer::new("root", LayerContent::Raster(Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true))));
    let owner = ItemRef::Layer(layer.id);
    let mut state = XcfState::default();
    for (item, name) in [(owner, "root"), (ItemRef::Mask(mask.id), "mask")] {
        state.items.push(ItemState {
            item,
            name: name.into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(doc.bounds()),
            mask: None,
            composition: None,
        });
    }
    state.filter_stacks.push(FilterStack {
        owner,
        revision: 7,
        nodes: vec![FilterNode {
            id: mask.owner,
            operation: OperationIdentity { name: "structural:unqualified".into(), stored_version: "0:0".into(), schema_epoch: "no-source-authority".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: false,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask.id)),
            auxiliary: vec![],
            composition: None,
        }],
    });
    state.filter_masks.push(mask);
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    doc
}
#[test]
#[ignore = "UNRUN/uncompiled: raw rounding preimage, numeric nonempty and strict wrapper boundary"]
fn checked_float_is_not_raw_unit_or_byte_zero() {
    for (raw, nonempty, bytezero) in
        [(f64::from_bits(0x3ff0000009b25010), true, false), (f64::from_bits(1), true, false), (-0.0, false, false), (0.0, false, true)]
    {
        let m = mask(raw);
        let proof = m.validate_checked_f32_coverage_with(&ctl()).unwrap();
        assert_eq!(proof.raw_nonempty(), nonempty);
        assert_eq!(proof.logical_all_byte_zero(), bytezero);
        assert_eq!(m.validate_with(&Interrupt::NONE).is_ok(), raw <= 1.0);
        let mut pixel = [0.0];
        m.surface.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
        assert_eq!(pixel[0].to_bits(), raw.to_bits());
    }
    for raw in [f64::from_bits(0x8000000000000001), f64::NAN, f64::INFINITY, 1.0001, -0.1] {
        assert!(mask(raw).validate_checked_f32_coverage_with(&ctl()).is_err());
    }
    let mut m = mask(0.5);
    let bytes = m.surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap().bytes_mut();
    let length = bytes.len();
    bytes[length - 8..].copy_from_slice(&(-0f64).to_ne_bytes());
    assert!(m.validate_checked_f32_coverage_with(&ctl()).is_err()); // outside logical extent, even -0
    let m = FilterMask {
        surface: Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0f64).to_ne_bytes()).unwrap(),
        ..mask(0.0)
    };
    assert!(m.validate_checked_f32_coverage_with(&ctl()).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: proof is actual immutable target; all other canonical invariants stay operative"]
fn selected_doc_proof_never_validates_a_clone_or_repairs_other_fields() {
    let d = doc(mask(f64::from_bits(0x3ff0000009b25010)));
    assert!(d.validate_xcf_state().is_err());
    let proof = d.checked_f32_filter_mask_coverage_with(&ctl()).unwrap().unwrap();
    d.validate_xcf_state_with_checked_filter_mask(&proof, &Interrupt::NONE).unwrap();
    let clone = d.clone(); // external harness; its mask has another immutable address
    assert!(clone.validate_xcf_state_with_checked_filter_mask(&proof, &Interrupt::NONE).is_err());
    let mut broken = d.clone();
    broken.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None;
    assert!(broken.checked_f32_filter_mask_coverage_with(&ctl()).is_err());
    let mut broken = d.clone();
    broken.xcf_state.as_mut().unwrap().items[1].item = broken.xcf_state.as_ref().unwrap().items[0].item;
    let own = broken.checked_f32_filter_mask_coverage_with(&ctl()).unwrap().unwrap();
    assert!(broken.validate_xcf_state_with_checked_filter_mask(&own, &Interrupt::NONE).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: prospective scan work, cancellation, payload cap and absent-mask strict continuation"]
fn scan_fails_before_payload_walk_and_preserves_typed_controls() {
    // Same debit sequence with a genuine non-Sync local source counter. This is
    // adapter parity, not a fresh operational meter or a cached proof discount.
    let m = mask(f64::from_bits(0x3ff0000009b25010));
    let shared = std::sync::atomic::AtomicU64::new(0);
    let charge = |n| {
        shared.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    };
    let shared_control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let first = m.validate_checked_f32_coverage_with(&shared_control).unwrap();
    let local = std::cell::Cell::new(0u64);
    let local_work = |n| {
        local.set(local.get().checked_add(n).unwrap());
        Ok(())
    };
    let second = m.validate_checked_f32_coverage_with_local_work(&Interrupt::NONE, &local_work).unwrap();
    assert_eq!(first.raw_nonempty(), second.raw_nonempty());
    assert_eq!(first.logical_all_byte_zero(), second.logical_all_byte_zero());
    assert_eq!(local.get(), shared.load(std::sync::atomic::Ordering::Relaxed));
    let denied = |_: u64| {
        local.set(local.get() + 1);
        Err(BackingError::Work)
    };
    assert!(matches!(m.validate_checked_f32_coverage_with_local_work(&Interrupt::NONE, &denied), Err(BackingError::Work)));
    let d = doc(mask(0.5));
    let zero = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(d.checked_f32_filter_mask_coverage_with(&zero), Err(BackingError::Work)));
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(d.checked_f32_filter_mask_coverage_with(&c), Err(BackingError::Cancelled)));
    let mut empty = doc(mask(0.0));
    let state = empty.xcf_state.as_mut().unwrap();
    state.filter_stacks[0].nodes.clear();
    state.filter_masks.clear();
    state.items.truncate(1);
    assert!(empty.checked_f32_filter_mask_coverage_with(&ctl()).unwrap().is_none());
    empty.validate_xcf_state().unwrap();
    let mut m = mask(0.5);
    m.bounds = Rect::new(0, 0, 262144, 1);
    // Actual shared tiles count toward the prospective physical payload, no giant allocation.
    let tile = m.surface.tiles().next().unwrap().1.clone();
    for x in 1..33 {
        m.surface.put_tiles(std::iter::once((photocraft_geom::TileCoord::new(x, 0), tile.clone())));
    }
    assert!(matches!(m.validate_checked_f32_coverage_with(&ctl()), Err(BackingError::Length)));
}
