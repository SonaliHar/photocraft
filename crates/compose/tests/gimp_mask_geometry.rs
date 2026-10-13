//! Canonical geometry semantics only; these synthetic tests do not qualify unequal GIMP files.
use photocraft_compose::{gimp_composite::CompositionError, gimp_render::try_render};
use photocraft_doc::{
    ColorMode, Document, Layer, LayerMask, PixelFormat, Rect, SampleEncoding, SampleType, Size,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{ItemRef, MaskId},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::Surface;

fn item(reference: ItemRef, bounds: Option<Rect>, composition: Option<GimpComposition>, mask: Option<MaskState>) -> ItemState {
    ItemState {
        item: reference,
        name: "synthetic geometry".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: bounds,
        mask,
        composition,
    }
}
fn policy(operator: GimpOperator) -> GimpComposition {
    GimpComposition {
        compatibility_epoch: "synthetic-independent-mask-geometry".into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: 1,
        stored_operator: match operator {
            GimpOperator::Union => 1,
            GimpOperator::ClipToBackdrop => 2,
            GimpOperator::ClipToLayer => 3,
            GimpOperator::Intersection => 4,
        },
        pass_through: false,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator }),
    }
}
fn fill(format: PixelFormat, rect: Rect, color: f64, alpha: f64) -> Surface {
    let mut surface = Surface::new(format);
    let channels = surface.channels();
    let mut pixel = vec![color; channels];
    if format.alpha {
        pixel[channels - 1] = alpha;
    }
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            surface.try_write_pixel_f64(x, y, &pixel).unwrap();
        }
    }
    surface
}
#[test]
fn declared_mask_bounds_and_outside_policy_are_independent_of_owner_and_tiles() {
    for model in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::F32, SampleType::F64] {
            for encoding in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
                for outside in [0.0_f64, 1.0] {
                    for operator in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
                        let mut doc = Document::new("synthetic geometry", Size::new(3, 3), model, depth);
                        doc.sample_encoding = Some(encoding);
                        let profile = photocraft_cms::xcf_color::effective_profile(
                            None,
                            if model == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
                            photocraft_cms::xcf_color::Encoding::Linear,
                        )
                        .unwrap();
                        doc.icc_profile = Some(profile.to_bytes());
                        let owner_bounds = Rect::from_xywh(-1, 0, 3, 3);
                        let mask_bounds = Rect::from_xywh(-1, 1, 2, 1);
                        let mut bottom = Layer::raster("backdrop", doc.pixel_format());
                        *bottom.surface_mut().unwrap() = fill(doc.pixel_format(), doc.bounds(), 0.25, 0.25);
                        let mut top = Layer::raster("owner", doc.pixel_format());
                        *top.surface_mut().unwrap() = fill(doc.pixel_format(), owner_bounds, 0.75 + 2.0_f64.powi(-40), 0.5);
                        top.opacity = 0.75;
                        let mask_format = PixelFormat::new(ColorMode::Grayscale, depth, false);
                        // A nonzero tile/default outside the declared mask must never become coverage.
                        let mut surface = if depth == SampleType::F64 {
                            Surface::try_with_default_bytes(mask_format, &0.9_f64.to_ne_bytes()).unwrap()
                        } else {
                            Surface::try_with_default_bytes(mask_format, &0.9_f32.to_ne_bytes()).unwrap()
                        };
                        for x in -1..1 {
                            surface.try_write_pixel_f64(x, 1, &[0.375 + 2.0_f64.powi(-40)]).unwrap();
                        }
                        let mut mask_sample = [0.0];
                        surface.try_read_pixel_f64(0, 1, &mut mask_sample).unwrap();
                        top.mask = Some(LayerMask { surface: std::sync::Arc::new(surface), enabled: true, linked: true, density: 1.0, feather: 0.0 });
                        let mut source = [0.0; 4];
                        let n = doc.pixel_format().channels();
                        top.surface().unwrap().try_read_pixel_f64(0, 0, &mut source[..n]).unwrap();
                        let source_color = source[0];
                        let mut state = XcfState::default();
                        state.items.push(item(ItemRef::Layer(bottom.id), Some(doc.bounds()), Some(policy(GimpOperator::Union)), None));
                        state.items.push(item(ItemRef::Layer(top.id), Some(owner_bounds), Some(policy(operator)), None));
                        state.items.push(item(
                            ItemRef::Mask(MaskId(1)),
                            None,
                            None,
                            Some(MaskState {
                                owner: ItemRef::Layer(top.id),
                                bounds: mask_bounds,
                                offset: [0, 0],
                                outside_f64_bits: outside.to_bits(),
                                enabled: true,
                                shown: false,
                                editing: false,
                                linked: true,
                            }),
                        ));
                        doc.layers = vec![bottom, top];
                        doc.xcf_state = Some(state);
                        let before = doc.clone();
                        let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                        for y in 0..3 {
                            for x in 0..3 {
                                let alpha =
                                    if owner_bounds.contains(x, y) { 0.375 * if mask_bounds.contains(x, y) { mask_sample[0] } else { outside } } else { 0.0 };
                                let (a, c) = match operator {
                                    GimpOperator::Union => {
                                        let a = alpha + 0.25 * (1.0 - alpha);
                                        (a, (0.25 * 0.25 * (1.0 - alpha) + source_color * alpha) / a)
                                    }
                                    GimpOperator::ClipToBackdrop => (0.25, 0.25 * (1.0 - alpha) + source_color * alpha),
                                    GimpOperator::ClipToLayer => (alpha, if alpha == 0.0 { 0.0 } else { source_color }),
                                    GimpOperator::Intersection => (alpha * 0.25, if alpha == 0.0 { 0.0 } else { source_color }),
                                };
                                let pixel = actual.px[(y * 3 + x) as usize];
                                assert!((pixel[0] - c).abs() < 1e-12);
                                assert!((pixel[3] - a).abs() < 1e-12);
                            }
                        }
                        assert!(try_render(&doc, doc.bounds(), &mut |_| Err(CompositionError::Invalid("denied"))).is_err());
                        let mut polls = 0;
                        assert!(
                            try_render(&doc, doc.bounds(), &mut |_| {
                                polls += 1;
                                if polls >= 4 { Err(CompositionError::Invalid("cancel during geometry render")) } else { Ok(()) }
                            })
                            .is_err()
                        );
                        assert!(polls >= 4, "cancellation happens after productive render work");
                        assert_eq!(doc, before);
                        let mut malformed = doc.clone();
                        malformed.xcf_state.as_mut().unwrap().items.last_mut().unwrap().mask.as_mut().unwrap().outside_f64_bits = f64::NAN.to_bits();
                        assert!(try_render(&malformed, malformed.bounds(), &mut |_| Ok(())).is_err());
                    }
                }
            }
        }
    }
}
