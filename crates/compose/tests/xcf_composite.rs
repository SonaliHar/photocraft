//! Synthetic equations complement, but do not replace, independent GIMP corpus oracles.
use photocraft_doc::{ColorMode, Document, ImportCompositing, Layer, Rect, SampleEncoding, SampleType, Size};
use photocraft_raster::Surface;
fn layer(doc: &Document, value: f32, alpha: f32, working: SampleEncoding) -> Layer {
    let mut l = Layer::raster("layer", doc.pixel_format());
    if let Some(s) = l.surface_mut() {
        s.write_pixel(0, 0, &[value, value, value, alpha]);
    }
    l.import_compositing = Some(ImportCompositing { blend_encoding: working, composite_encoding: working, pass_through: false });
    l
}
#[test]
fn nonlinear_float_linear_source_over_preserves_source_and_alpha() {
    let mut d = Document::new("float nonlinear", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.layers = vec![layer(&d, 0.0, 1.0, SampleEncoding::Linear), layer(&d, 1.0, 0.5, SampleEncoding::Linear)];
    let before = d.clone();
    let b = photocraft_compose::render(&d, d.bounds()).unwrap();
    let expected = photocraft_color::convert::linear_to_srgb(0.5);
    assert!((b.px[0][0] - expected).abs() < 1e-5);
    assert_eq!(b.px[0][3], 1.0);
    assert_eq!(d, before);
}
#[test]
fn linear_integer_nonlinear_composite_differs_from_stored_space_mix() {
    let mut d = Document::new("linear integer", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
    d.sample_encoding = Some(SampleEncoding::Linear);
    d.layers = vec![layer(&d, 0.0, 1.0, SampleEncoding::Perceptual), layer(&d, 1.0, 0.5, SampleEncoding::Perceptual)];
    let b = photocraft_compose::render(&d, d.bounds()).unwrap();
    assert!((b.px[0][0] - photocraft_color::convert::srgb_to_linear(0.5)).abs() < 2.0 / 65535.0);
}
#[test]
fn groups_isolate_and_pass_through_children_and_masks() {
    for pass in [false, true] {
        let mut d = Document::new("groups", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        d.sample_encoding = Some(SampleEncoding::Linear);
        let bottom = layer(&d, 0.1, 1.0, SampleEncoding::Linear);
        let mut group = Layer::group("group", vec![layer(&d, 0.9, 0.5, SampleEncoding::Linear)]);
        group.import_compositing =
            Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: pass });
        group.blend = if pass { photocraft_doc::BlendMode::PassThrough } else { photocraft_doc::BlendMode::Normal };
        let mask = Surface::try_from_interleaved(
            photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false),
            Rect::new(0, 0, 1, 1),
            &0.5f32.to_ne_bytes(),
        )
        .unwrap();
        group.mask = Some(photocraft_doc::LayerMask { surface: std::sync::Arc::new(mask), enabled: true, linked: true, density: 1.0, feather: 0.0 });
        d.layers = vec![bottom, group];
        assert!((photocraft_compose::render(&d, d.bounds()).unwrap().px[0][0] - 0.3).abs() < 1e-5);
    }
}
#[test]
fn explicit_encoding_overrides_depth_for_exposure() {
    use photocraft_compose::{
        Buffer,
        adjust::{Transfer, apply_depth},
    };
    use photocraft_doc::Adjustment;
    for depth in SampleType::ALL {
        let mut d = Document::new("encoding", Size::new(1, 1), ColorMode::Rgb, depth);
        d.sample_encoding = Some(SampleEncoding::Linear);
        assert_eq!(Transfer::for_doc(&d), Transfer::Gamma(1.0));
        d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        assert_eq!(Transfer::for_doc(&d), Transfer::SrgbExact);
        let mut b = Buffer { rect: d.bounds(), px: vec![[0.5, 0.5, 0.5, 1.0]] };
        apply_depth(&Adjustment::Exposure { exposure: 1.0, offset: 0.0, gamma: 1.0 }, &mut b, Transfer::for_doc(&d), Some(depth));
        let expected = photocraft_color::convert::linear_to_srgb(photocraft_color::convert::srgb_to_linear(0.5) * 2.0);
        assert!((b.px[0][0] - expected).abs() < 0.0001, "{depth:?}: {} versus {expected}", b.px[0][0]);
    }
}

#[test]
fn linear_float_exposure_keeps_signed_hdr_samples() {
    use photocraft_compose::{
        Buffer,
        adjust::{Transfer, apply_depth},
    };
    use photocraft_doc::Adjustment;
    for explicit in [None, Some(SampleEncoding::Linear)] {
        let mut d = Document::new("linear HDR", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        d.sample_encoding = explicit;
        let mut b = Buffer { rect: d.bounds(), px: vec![[-0.25, 0.5, 2.0, 0.5]] };
        apply_depth(&Adjustment::Exposure { exposure: 1.0, offset: 0.0, gamma: 1.0 }, &mut b, Transfer::for_doc(&d), Some(d.depth));
        assert_eq!(b.px[0].map(f32::to_bits), [-0.5_f32, 1.0, 4.0, 0.5].map(f32::to_bits));
        let hidden = [-0.25, 0.5, 2.0, -0.0];
        b.px[0] = hidden;
        apply_depth(&Adjustment::Exposure { exposure: 1.0, offset: 0.0, gamma: 1.0 }, &mut b, Transfer::for_doc(&d), Some(d.depth));
        assert_eq!(b.px[0].map(f32::to_bits), hidden.map(f32::to_bits));
    }
}

#[test]
fn imported_indexed_normal_thresholds_opacity_and_mask_coverage() {
    // https://developer.gimp.org/core/algorithm/compositing/#layer-modes
    for (opacity, expected) in [(0.49, 0.0), (0.5, 0.0), (0.51, 1.0)] {
        let mut d = Document::new("indexed", Size::new(1, 1), ColorMode::Indexed, SampleType::U8);
        d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let bottom = layer(&d, 0.0, 1.0, SampleEncoding::Linear);
        let mut top = layer(&d, 1.0, 1.0, SampleEncoding::Linear);
        top.opacity = opacity;
        d.layers = vec![bottom, top];
        assert_eq!(photocraft_compose::render(&d, d.bounds()).unwrap().px[0], [expected, expected, expected, 1.0]);
    }
}
