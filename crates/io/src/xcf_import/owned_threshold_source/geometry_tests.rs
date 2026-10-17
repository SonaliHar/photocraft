//! UNRUN geometry census only; no source, sample or renderer qualification.
use super::*;

fn document() -> Document {
    let mut doc = synthetic();
    doc.size = Size::new(257, 257);
    let bounds = doc.bounds();
    doc.layers[0].content = LayerContent::Raster(Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), &[0; 32]).unwrap());
    let state = doc.xcf_state.as_mut().unwrap();
    state.items[0].raster_bounds = Some(bounds);
    let surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0.0f64).to_ne_bytes()).unwrap();
    state.filter_masks.push(photocraft_doc::FilterMask { id: MaskId(2), owner: FilterId(1), surface, bounds });
    state.items.push(state.items[0].clone());
    doc
}

#[test]
fn sparse_mask_grid_is_priced_per_actual_entry_without_copying_or_normalizing_words() {
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let mut doc = document();
    let empty = census::measure_threshold(&doc, &budget, &Interrupt::NONE).unwrap();
    for (x, y) in [(0, 0), (256, 0), (0, 256), (256, 256)] {
        doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(x, y, &[0.5 + 2f64.powi(-40)]).unwrap();
    }
    let mask = &doc.xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let before = mask.clone();
    let work = budget.used(photocraft_xcf::BudgetKind::Work);
    let full = census::measure_threshold(&doc, &budget, &Interrupt::NONE).unwrap();
    assert!(full.model >= empty.model + 4 * 256);
    assert!(budget.used(photocraft_xcf::BudgetKind::Work) >= work + 4);
    assert_eq!(mask, &before);
    assert_eq!(mask.default_bytes(), &(-0.0f64).to_ne_bytes());
    for (coord, tile) in mask.tiles() {
        assert!(Arc::ptr_eq(tile, before.tile(*coord).unwrap()));
    }
    assert!(census::measure_plain_for_test(&doc, &budget, &Interrupt::NONE).is_err(), "plain path stays closed");
}

#[test]
fn geometry_census_rejects_outside_sparse_tiles_pixel_ceiling_and_mid_map_cancel() {
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let mut doc = document();
    doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(-1, 0, &[0.5]).unwrap();
    assert!(census::measure_threshold(&doc, &budget, &Interrupt::NONE).is_err());
    let mut doc = document();
    doc.size = Size::new(262144, 4097);
    assert!(census::measure_threshold(&doc, &budget, &Interrupt::NONE).is_err());
    let mut doc = document();
    for (x, y) in [(0, 0), (256, 0), (0, 256), (256, 256)] {
        doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(x, y, &[0.5]).unwrap();
    }
    let polls = AtomicU64::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 2;
    assert!(matches!(census::measure_threshold(&doc, &budget, &Interrupt::cancel_only(&cancelled)), Err(IoError::Cancelled)));
    assert!(polls.load(Ordering::Relaxed) >= 3);
}
