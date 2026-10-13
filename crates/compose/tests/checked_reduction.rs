//! Checked display reduction is derived data; every native source remains exact.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_compose::{
    render_reduced, render_reduced_damage, render_reduced_damage_with_interrupt, render_reduced_rect, render_reduced_rect_with_interrupt,
    render_reduced_with_interrupt, thumbnail_buffer, thumbnail_buffer_with_interrupt,
};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scene(sample: SampleType) -> Document {
    let mut doc = Document::new("own derived display", Size::new(97, 65), ColorMode::Rgb, sample);
    let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
    let mut bottom = Surface::with_default(fmt, &[0.2, 0.6, 0.8, 0.5]);
    bottom.try_write_pixel_f64(-3, -2, &[0.9, 0.1, 0.3, 0.75]).unwrap();
    bottom.try_write_pixel_f64(96, 64, &[0.1, 0.9, 0.6, 0.25]).unwrap();
    doc.layers.push(Layer::new("own bottom", LayerContent::Raster(bottom)));
    let mut top = Layer::new("own top", LayerContent::Raster(Surface::with_default(fmt, &[0.8, 0.2, 0.4, 0.5])));
    top.opacity = 0.625;
    let mut mask = LayerMask::reveal_all();
    std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(12, 9, 44, 36), &[0.25]);
    top.mask = Some(mask);
    doc.layers.push(Layer::group("own group", vec![top]));
    doc
}

#[test]
fn checked_reduction_matches_resident_values_without_altering_any_native_depth() {
    // Imported half-float is explicitly widened to canonical F32; it is not a
    // distinct native Surface storage type.
    for sample in SampleType::ALL {
        let doc = scene(sample);
        let before = doc.clone();
        for (w, h) in [(1, 1), (23, 17), (97, 65)] {
            assert_eq!(render_reduced_with_interrupt(&doc, w, h, &Interrupt::NONE).unwrap(), render_reduced(&doc, w, h).unwrap());
        }
        let extent = Rect::new(-13, -7, 115, 84);
        assert_eq!(render_reduced_rect_with_interrupt(&doc, extent, 23, 17, &Interrupt::NONE).unwrap(), render_reduced_rect(&doc, extent, 23, 17).unwrap());
        assert_eq!(thumbnail_buffer_with_interrupt(&doc, 31, &Interrupt::NONE).unwrap(), thumbnail_buffer(&doc, 31).unwrap());
        assert_eq!(doc, before);
    }
}

#[test]
fn checked_damage_coordinates_and_values_match_whole_reduction_cells() {
    let doc = scene(SampleType::U16);
    let full = render_reduced_with_interrupt(&doc, 23, 17, &Interrupt::NONE).unwrap();
    for damage in [Rect::new(12, 9, 44, 36), Rect::new(-2, -2, 1, 1), Rect::new(96, 64, 120, 80), Rect::new(120, 90, 130, 100)] {
        let part = render_reduced_damage_with_interrupt(&doc, 23, 17, damage, &Interrupt::NONE).unwrap();
        assert_eq!(part, render_reduced_damage(&doc, 23, 17, damage).unwrap());
        for y in part.rect.y0..part.rect.y1 {
            for x in part.rect.x0..part.rect.x1 {
                assert_eq!(part.get(x, y), full.get(x, y));
            }
        }
    }
}

#[test]
fn invalid_bounds_and_output_counts_fail_before_empty_success_or_allocation() {
    let doc = scene(SampleType::F64);
    assert_eq!(render_reduced_with_interrupt(&doc, u32::MAX, 1, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
    for rect in [Rect::new(1, 0, 0, 1), Rect::new(i32::MIN, 0, i32::MAX, 1)] {
        assert_eq!(render_reduced_rect_with_interrupt(&doc, rect, 1, 1, &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
        assert_eq!(render_reduced_damage_with_interrupt(&doc, 1, 1, rect, &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
    }
    let mut imported = Document::new("own unsupported", Size::new(0, 0), ColorMode::Rgb, SampleType::U8);
    imported.xcf_state = Some(Default::default());
    assert_eq!(render_reduced_with_interrupt(&imported, 1, 1, &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion));
    assert_eq!(render_reduced_damage_with_interrupt(&imported, 1, 1, Rect::EMPTY, &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion));
}

#[test]
fn late_cancel_returns_no_reduced_buffer_and_preserves_source() {
    let doc = scene(SampleType::F64);
    let before = doc.clone();
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
    assert_eq!(
        render_reduced_rect_with_interrupt(&doc, Rect::new(-13, -7, 115, 84), 23, 17, &Interrupt::cancel_only(&cancel)),
        Err(RasterInputError::Cancelled)
    );
    assert!(polls.load(Ordering::Relaxed) > 100);
    assert_eq!(doc, before);
    assert!(render_reduced_with_interrupt(&doc, 23, 17, &Interrupt::NONE).is_ok());
}
