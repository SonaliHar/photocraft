//! UNRUN actual CURRENT storage display drafts. No source, GIMP output or
//! visual acceptance is claimed by these independently paid synthetic models.
use super::*;
use crate::backed_stock_point_context::tests::{adopt, ctl, fixture};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{SampleEncoding, Size, item_identity::ItemRef};

#[test]
fn wide_display_keeps_floating_result_until_composition_and_canvas_abyss_is_zero() {
    let (mut doc, q, guard, _, profile, _) = fixture(2, ColorMode::Rgb, SampleType::F32, SampleEncoding::NonlinearProfile);
    doc.size = Size::new(10, 8);
    let canvas = doc.bounds();
    let state = doc.xcf_state.as_mut().unwrap();
    let mask = state.filter_masks[0].id;
    state.filter_masks[0].bounds = canvas;
    let item = state.items.iter_mut().find(|item| item.item == ItemRef::Mask(mask)).unwrap();
    item.raster_bounds = Some(canvas);
    item.mask.as_mut().unwrap().bounds = canvas;
    let owner = adopt(doc, q, guard);
    let context = PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    let native_bounds = context.descriptor().bounds();
    let wide = context.render_wide_band(native_bounds, &ctl()).unwrap();
    let display = PreparedBackedStockPointDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    assert_eq!(display.bounds(), canvas);
    let band = display.render_band(canvas, &ctl()).unwrap();
    for y in 0..8usize {
        for x in 0..10usize {
            let actual = band.pixels()[y * 10 + x];
            let expected = if x < 8 && y < 6 { normal_over_zero_backdrop(wide.pixels()[y * 8 + x], 1.0).unwrap() } else { [0.0; 4] };
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        }
    }
    assert!(
        wide.pixels().iter().any(|pixel| pixel[3] > 0.0 && pixel[..3].iter().any(|value| f64::from(*value as f32).to_bits() != value.to_bits())),
        "the fixture must expose premature Float32 intermediate rounding"
    );
}

#[test]
fn hidden_and_fractional_layer_coverage_use_same_current_recipe_and_failure_is_private() {
    for visible in [true, false] {
        let (mut doc, q, guard, _, profile, _) = fixture(8, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
        doc.layers[0].visible = visible;
        doc.layers[0].opacity = 0.375;
        let layer = doc.layers[0].id;
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().visible = visible;
        let owner = adopt(doc, q.clone(), guard);
        let display = PreparedBackedStockPointDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
        let context = PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
        let wide = context.render_wide_band(owner.document().bounds(), &ctl()).unwrap();
        let band = display.render_band(owner.document().bounds(), &ctl()).unwrap();
        for (actual, input) in band.pixels().iter().zip(wide.pixels()) {
            let expected = normal_over_zero_backdrop(*input, if visible { 0.375 } else { 0.0 }).unwrap();
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        }
        let before = q.usage().unwrap();
        let denied = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Err(photocraft_raster::backing::BackingError::Work) };
        assert!(matches!(
            display.render_band(owner.document().bounds(), &denied),
            Err(BackedNormalError::Backing(photocraft_raster::backing::BackingError::Work))
        ));
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(owner.document().layers[0].opacity.to_bits(), 0.375f32.to_bits());
    }
}
