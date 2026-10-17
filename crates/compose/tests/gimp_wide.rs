//! Independent coverage equations exercise the checked document consumer, not preview math.
use photocraft_cms::{Builtin, WideWork};
use photocraft_compose::{
    gimp_composite::{CompositionError, WorkingSpace},
    gimp_render::try_render,
};
use photocraft_doc::{
    BlendMode, ColorMode, Document, Layer, LayerMask, PixelFormat, Rect, SampleEncoding, SampleType, Size,
    import_compositing::{GimpOperator, GimpSpace},
};
use photocraft_raster::Surface;

fn doc(depth: SampleType) -> Document {
    let mut doc = Document::new("wide", Size::new(1, 1), ColorMode::Rgb, depth);
    doc.sample_encoding = Some(SampleEncoding::Linear);
    doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
    doc
}
fn layer(doc: &Document, pixel: [f64; 4]) -> Layer {
    let mut layer = Layer::raster("wide", doc.pixel_format());
    layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, &pixel).unwrap();
    layer
}
fn render(doc: &Document) -> [f64; 4] {
    try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap().px[0]
}

fn modified_text_document() -> Document {
    use photocraft_doc::{
        item_identity::{ItemRef, ResourceId},
        xcf_items::{EditableSource, SourceCache, SourceItem},
        xcf_resources::{ResourceKind, ResourceReference},
        xcf_state::{ItemLocks, ItemState, XcfState},
    };
    let mut d = doc(SampleType::F64);
    d.layers = vec![layer(&d, [0.25, 1.25, -0.5, 1.0])];
    let owner = ItemRef::Layer(d.layers[0].id);
    let mut state = XcfState::default();
    state.items.push(ItemState {
        item: owner,
        name: "modified text".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(d.bounds()),
        mask: None,
        composition: None,
    });
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::Font,
        name: "unresolved inert font".into(),
        collection: None,
        internal: false,
        locator: None,
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    state.source_items.push(SourceItem {
        owner,
        source: EditableSource::Text {
            content: "source retained".into(),
            markup: None,
            font: ResourceId(1),
            direction: 0,
            source_modified: true,
            configuration: vec![],
            current_origin: None,
        },
        transform: None,
        cache: Some(SourceCache {
            bytes: vec![0, 0, 0, 255].into(),
            blake3: "b1a60e92fef42c93a79fb8f02aa1ba30a292f101971059aaa66f75a08a1ee54b".into(),
            width: 1,
            height: 1,
            format: PixelFormat::RGBA8,
        }),
    });
    d.xcf_state = Some(state);
    d
}

#[test]
fn modified_source_uses_current_wide_raster_and_preserves_original_cache() {
    let mut d = modified_text_document();
    let before = d.clone();
    assert_eq!(render(&d), [0.25, 1.25, -0.5, 1.0]);
    assert_eq!(d, before);
    // An explicit active profile assignment does not restore the immutable U8 cache.
    d.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.75, 0.125, 0.5, 1.0]).unwrap();
    assert_eq!(render(&d), [0.75, 0.125, 0.5, 1.0]);
    assert_eq!(d.xcf_state.as_ref().unwrap().source_items, before.xcf_state.as_ref().unwrap().source_items);
}

#[test]
fn modified_vector_uses_current_raster_with_inert_path_metadata() {
    use photocraft_doc::{item_identity::ItemRef, xcf_items::EditableSource};
    let mut d = modified_text_document();
    let path = photocraft_doc::NamedPath::default();
    let id = path.id;
    d.paths.push(path);
    let state = d.xcf_state.as_mut().unwrap();
    let mut item = state.items[0].clone();
    item.item = ItemRef::Path(id);
    item.raster_bounds = None;
    state.items.push(item);
    state.source_items[0].source = EditableSource::Vector { path: id, source_modified: true, configuration: vec![] };
    let before = d.clone();
    assert_eq!(render(&d), [0.25, 1.25, -0.5, 1.0]);
    assert_eq!(d, before);
}

#[test]
fn unmodified_corrupt_missing_and_transformed_source_evidence_is_rejected() {
    use photocraft_doc::xcf_items::{EditableSource, TransformState};
    for case in 0..5 {
        let mut d = modified_text_document();
        let source = &mut d.xcf_state.as_mut().unwrap().source_items[0];
        match case {
            0 => {
                if let EditableSource::Text { source_modified, .. } = &mut source.source {
                    *source_modified = false;
                }
            }
            1 => source.cache = None,
            2 => std::sync::Arc::make_mut(&mut source.cache.as_mut().unwrap().bytes)[0] = 1,
            3 => source.transform = Some(TransformState { matrix_f64_bits: [0; 9], interpolation: 0, pre_offset: [0; 2] }),
            _ => {
                source.source = EditableSource::Linked {
                    resource: photocraft_doc::item_identity::ResourceId(1),
                    last_verified_hash: None,
                    configuration: vec![],
                    current: None,
                }
            }
        }
        let before = d.clone();
        assert!(try_render(&d, d.bounds(), &mut |_| Ok(())).is_err());
        assert_eq!(d, before);
    }
}

#[test]
fn modified_source_does_not_bypass_required_opaque_or_nonraster_owners() {
    let mut d = modified_text_document();
    let state = d.xcf_state.as_mut().unwrap();
    let cache = state.source_items[0].cache.as_ref().unwrap();
    state.opaque.push(photocraft_doc::xcf_provenance::OpaqueRecord {
        owner: Some(state.source_items[0].owner),
        namespace: "unqualified-rendering".into(),
        identity: "required".into(),
        source_version: 19,
        order: 0,
        flags: 0,
        bytes: cache.bytes.clone(),
        blake3: cache.blake3.clone(),
        rendering_required: true,
    });
    assert!(try_render(&d, d.bounds(), &mut |_| Ok(())).is_err());
    let mut d = modified_text_document();
    d.layers[0].content = Layer::group("changed owner", vec![]).content;
    assert!(try_render(&d, d.bounds(), &mut |_| Ok(())).is_err());
}

#[test]
fn prepared_source_cache_hashing_checks_shared_interrupt_mid_payload() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    let mut d = modified_text_document();
    let cache = d.xcf_state.as_mut().unwrap().source_items[0].cache.as_mut().unwrap();
    cache.bytes = vec![0; 1 << 20].into();
    cache.width = 262144;
    cache.height = 1;
    cache.blake3 = "488de202f73bd976de4e7048f4e1f39a776d86d582b7348ff53bf432b987fca8".into();
    PreparedRenderer::new(&d).unwrap();
    let before = d.clone();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 7;
    let control = photocraft_raster::Interrupt::cancel_only(&cancel);
    assert!(matches!(PreparedRenderer::new_with_interrupt(&d, &control), Err(CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))));
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 8);
    assert_eq!(d, before);
}

#[test]
fn authoritative_hdr_and_u32_low_bits_survive_document_render() {
    let mut d = doc(SampleType::F64);
    let pixel = [-0.0, -1.25, 3.25 + 2.0_f64.powi(-40), 1.0];
    d.layers = vec![layer(&d, pixel)];
    let before = d.clone();
    assert_eq!(render(&d).map(f64::to_bits), pixel.map(f64::to_bits));
    assert_eq!(d, before);
    let mut d = doc(SampleType::U32);
    let codes = [0x80000000_u32, 0x80000001, 0x80000002, u32::MAX];
    let raw: Vec<_> = codes.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut l = Layer::raster("default", d.pixel_format());
    *l.surface_mut().unwrap() = Surface::try_with_default_bytes(d.pixel_format(), &raw).unwrap();
    d.layers = vec![l];
    assert_eq!(render(&d), codes.map(|v| f64::from(v) / f64::from(u32::MAX)));
}

#[test]
fn normal_operators_follow_independent_coverage_equations() {
    let working = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let b = [0.2, -2.0, 4.0, 0.25];
    let s = [0.8, 2.0, -1.0, 0.5];
    for (op, alpha, source_weight) in [
        (GimpOperator::Union, 0.4375, 0.25 / 0.4375),
        (GimpOperator::ClipToBackdrop, 0.25, 0.25),
        (GimpOperator::ClipToLayer, 0.25, 1.0),
        (GimpOperator::Intersection, 0.0625, 1.0),
    ] {
        let result = working.normal(b, s, 0.5, op).unwrap();
        for i in 0..3 {
            assert!((result[i] - (b[i] * (1.0 - source_weight) + s[i] * source_weight)).abs() < 1e-14);
        }
        assert_eq!(result[3], alpha);
    }
}

#[test]
fn hard_mix_threshold_keeps_low_bits_ties_and_checked_domains() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    let working = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let evaluate = |b, s| working.separable(&working, [b, b, b, 1.0], [s, s, s, 1.0], 1.0, GimpOperator::Union, GimpBlendMode::HardMix);
    assert_eq!(evaluate(0.5, 0.5 - 2.0_f64.powi(-40)).unwrap(), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(evaluate(0.5, 0.5).unwrap(), [1.0; 4]);
    assert_eq!(evaluate(0.5, 0.5 + 2.0_f64.powi(-40)).unwrap(), [1.0; 4]);
    assert_eq!(evaluate(0.0, 1.0).unwrap(), [1.0; 4]);
    assert_eq!(evaluate(1.0, 0.0).unwrap(), [1.0; 4]);
    assert!(evaluate(1.01, 0.1).is_err());
    assert!(evaluate(f64::NAN, 0.1).is_err());
}

#[test]
fn legacy_basic_overlap_is_distinct_and_rejects_unqualified_policies() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    let profile = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    let linear = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
    let b = [0.2, 0.4, 0.8, 0.25];
    let s = [0.8, 0.5, 0.1, 0.75];
    let overlap = 0.25 * 0.5;
    let weight = overlap / (0.25 + 0.75 * overlap);
    for (mode, product) in [
        (GimpBlendMode::LegacyMultiply, [0.16, 0.2, 0.08]),
        (GimpBlendMode::LegacyScreen, [0.84, 0.7, 0.82]),
        (GimpBlendMode::LegacyDifference, [0.6, 0.1, 0.7]),
        (GimpBlendMode::LegacySoftLight, [0.296, 0.4, 0.672]),
        (GimpBlendMode::LegacyAddition, [1.0, 0.9, 0.9]),
        (GimpBlendMode::LegacySubtract, [0.0, 0.0, 0.7]),
        (GimpBlendMode::LegacyDarkenOnly, [0.2, 0.4, 0.1]),
        (GimpBlendMode::LegacyLightenOnly, [0.8, 0.5, 0.8]),
    ] {
        let result = profile.separable(&profile, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).unwrap();
        for c in 0..3 {
            assert!((result[c] - (b[c] * (1.0 - weight) + product[c] * weight)).abs() < 1e-14);
        }
        assert_eq!(result[3], b[3]);
        assert!(profile.separable(&profile, b, s, 0.5, GimpOperator::Union, mode).is_err());
        assert!(linear.separable(&profile, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
        assert!(profile.separable(&linear, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
        assert!(profile.separable(&profile, [1.1, 0.4, 0.8, 0.25], s, 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
    }
    let low = 0.25 + 2.0_f64.powi(-40);
    for (mode, neutral) in [(GimpBlendMode::LegacyMultiply, 1.0), (GimpBlendMode::LegacyScreen, 0.0), (GimpBlendMode::LegacySoftLight, 0.5)] {
        let result = profile.separable(&profile, [low, low, low, 0.25], [neutral, neutral, neutral, 0.75], 0.5, GimpOperator::ClipToBackdrop, mode).unwrap();
        assert_eq!(result[0].to_bits(), low.to_bits());
    }
}

#[test]
#[ignore = "24 MP release Hard Mix checked kernel timing"]
fn bench_hard_mix_wide_24mp() {
    use std::{hint::black_box, time::Instant};
    let working = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let start = Instant::now();
    for _ in 0..24_000_000 {
        black_box(
            working
                .separable(
                    &working,
                    black_box([0.2, 0.3, 0.4, 0.75]),
                    black_box([0.8, 0.7, 0.6, 0.5]),
                    black_box(0.5),
                    GimpOperator::ClipToBackdrop,
                    photocraft_compose::gimp_blend::GimpBlendMode::HardMix,
                )
                .unwrap(),
        );
    }
    println!("24 MP checked HardMix: {:?}", start.elapsed());
}

#[test]
#[ignore = "24 MP release legacy Multiply/Screen checked kernel timing"]
fn bench_legacy_basic_wide_24mp() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    use std::{hint::black_box, time::Instant};
    let working = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    for mode in [GimpBlendMode::LegacyMultiply, GimpBlendMode::LegacyScreen] {
        let start = Instant::now();
        for _ in 0..24_000_000 {
            black_box(
                working
                    .separable(
                        &working,
                        black_box([0.2, 0.3, 0.4, 0.75]),
                        black_box([0.8, 0.7, 0.6, 0.5]),
                        black_box(0.5),
                        GimpOperator::ClipToBackdrop,
                        black_box(mode),
                    )
                    .unwrap(),
            );
        }
        println!("24 MP checked {mode:?}: {:?}", start.elapsed());
    }
}

#[test]
fn masks_and_groups_apply_coverage_without_narrowing_color() {
    for pass in [false, true] {
        let mut d = doc(SampleType::F64);
        let mut group = Layer::group("group", vec![layer(&d, [0.9; 4])]);
        group.blend = if pass { BlendMode::PassThrough } else { BlendMode::Normal };
        group.opacity = 0.5;
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        group.mask = Some(LayerMask {
            surface: std::sync::Arc::new(Surface::try_with_default_bytes(format, &0.5_f64.to_ne_bytes()).unwrap()),
            enabled: true,
            linked: true,
            density: 1.0,
            feather: 0.0,
        });
        d.layers = vec![layer(&d, [0.1, 0.1, 0.1, 1.0]), group];
        let expected = 0.1 * 0.775 + 0.9 * 0.225;
        assert!((render(&d)[0] - expected).abs() < 1e-14);
        assert_eq!(render(&d)[3], 1.0);
    }
}

#[test]
fn profile_aware_linear_and_perceptual_spaces_are_distinct() {
    let profile = Builtin::Srgb.profile();
    let b = [0.0, 0.0, 0.0, 1.0];
    let s = [1.0; 4];
    let linear = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
    let stored = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    let expected = profile.trc.as_ref().unwrap()[0].eval_inverse64(0.5);
    assert!((linear.normal(b, s, 0.5, GimpOperator::Union).unwrap()[0] - expected).abs() < 1e-12);
    assert_eq!(stored.normal(b, s, 0.5, GimpOperator::Union).unwrap()[0], 0.5);
    WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::Lab).unwrap().normal(b, s, 0.5, GimpOperator::Union).unwrap();
}

#[test]
fn gray_working_transfer_ignores_preserved_rgb_curve_tags() {
    let mut profile = Builtin::GrayGamma22.profile().clone();
    profile.gray_trc = Some(photocraft_cms::curve::Curve::Gamma(2.2));
    profile.trc = Some(std::array::from_fn(|_| photocraft_cms::curve::Curve::Gamma(1.8)));
    let profile = photocraft_cms::Profile::parse(&profile.with_encoded_bytes().to_bytes()).unwrap();
    let before = profile.clone();
    let working = WorkingSpace::new(&profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
    let curve = profile.gray_trc.as_ref().unwrap();
    let expected = curve.eval_inverse64(curve.eval64(0.19) * 0.625 + curve.eval64(0.81) * 0.375);
    let output = working.normal([0.19, 0.19, 0.19, 1.0], [0.81, 0.81, 0.81, 1.0], 0.375, GimpOperator::Union).unwrap();
    assert_eq!(output.map(f64::to_bits), [expected.to_bits(), expected.to_bits(), expected.to_bits(), 1.0_f64.to_bits()]);
    assert_eq!(profile, before);
}

#[test]
fn invalid_samples_modes_and_cancellation_fail_without_document_mutation() {
    let mut d = doc(SampleType::F64);
    d.layers = vec![layer(&d, [0.5, 0.5, 0.5, 1.0])];
    let before = d.clone();
    assert!(
        try_render(&d, Rect::new(0, 0, 1, 1), &mut |work| if matches!(work, WideWork::SampleUnits(128)) {
            Err(CompositionError::Invalid("cancelled"))
        } else {
            Ok(())
        })
        .is_err()
    );
    assert_eq!(d, before);
    d.layers[0].blend = BlendMode::Multiply;
    assert!(try_render(&d, d.bounds(), &mut |_| Ok(())).is_err());
    let working = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    assert!(working.normal([0.0; 4], [f64::NAN, 0.0, 0.0, 1.0], 1.0, GimpOperator::Union).is_err());
    assert!(working.normal([0.0; 4], [0.0, 0.0, 0.0, 2.0], 1.0, GimpOperator::Union).is_err());
}

#[test]
fn lut_profile_never_invents_a_separable_transfer() {
    use photocraft_cms::{
        Clut, Curve,
        pipeline::Stage,
        profile::{Lut, LutKind},
    };
    let mut profile = Builtin::Srgb.profile().clone();
    profile.matrix = None;
    profile.trc = None;
    profile.a2b[0] = Some(Lut {
        kind: LutKind::Ab,
        inputs: 3,
        outputs: 3,
        stages: vec![
            Stage::Curves(vec![Curve::Identity; 3]),
            Stage::Clut(Clut::try_new_f64(vec![2; 3], 3, vec![0.2; 24]).unwrap()),
            Stage::Curves(vec![Curve::Identity; 3]),
        ],
    });
    let profile = profile.with_encoded_bytes();
    let space = WorkingSpace::new(&profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    assert_eq!(space.normal([0.0, 0.0, 0.0, 1.0], [1.0; 4], 0.5, GimpOperator::Union).unwrap(), [0.5, 0.5, 0.5, 1.0]);
    assert!(WorkingSpace::new(&profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).is_err());
    assert!(WorkingSpace::new(&profile, SampleEncoding::Linear, GimpSpace::LinearRgb).is_err());
    assert!(WorkingSpace::new(&profile, SampleEncoding::NonlinearProfile, GimpSpace::PerceptualRgb).is_err());
}

#[test]
fn unequal_profile_curves_are_applied_per_channel() {
    use photocraft_cms::Curve;
    let profile = {
        let mut profile = Builtin::Srgb.profile().clone();
        profile.trc = Some([Curve::Gamma(1.0), Curve::Gamma(2.0), Curve::Gamma(4.0)]);
        profile.with_encoded_bytes()
    };
    let space = WorkingSpace::new(&profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
    let result = space.normal([0.0, 0.0, 0.0, 1.0], [1.0; 4], 0.5, GimpOperator::Union).unwrap();
    assert_eq!(result[0], 0.5);
    assert!((result[1] - 0.5_f64.sqrt()).abs() < 1e-14);
    assert!((result[2] - 0.5_f64.sqrt().sqrt()).abs() < 1e-14);
}

#[test]
fn mask_registry_outside_coverage_and_implicit_source_profile_are_used() {
    use photocraft_doc::{
        SourceColorContext,
        item_identity::{ItemRef, MaskId},
        xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
    };
    let mut d = doc(SampleType::F64);
    d.icc_profile = Some(
        photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Linear).unwrap().to_bytes(),
    );
    d.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(d.icc_profile.as_ref().unwrap());
    d.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 0 });
    let mut top = layer(&d, [0.9, 0.9, 0.9, 1.0]);
    top.import_compositing = Some(photocraft_doc::ImportCompositing {
        blend_encoding: SampleEncoding::NonlinearProfile,
        composite_encoding: SampleEncoding::NonlinearProfile,
        pass_through: false,
    });
    top.mask = Some(LayerMask {
        surface: std::sync::Arc::new(
            Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &1.0_f64.to_ne_bytes()).unwrap(),
        ),
        enabled: true,
        linked: true,
        density: 1.0,
        feather: 0.0,
    });
    let mut state = XcfState::default();
    state.items.push(ItemState {
        raster_bounds: None,
        item: ItemRef::Mask(MaskId(123)),
        name: "mask".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        composition: None,
        mask: Some(MaskState {
            owner: ItemRef::Layer(top.id),
            bounds: Rect::new(1, 1, 2, 2),
            offset: [0, 0],
            outside_f64_bits: 0.5_f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: true,
        }),
    });
    d.xcf_state = Some(state);
    d.layers = vec![layer(&d, [0.1, 0.1, 0.1, 1.0]), top];
    let curve = &Builtin::Srgb.profile().trc.as_ref().unwrap()[0];
    let expected = curve.eval64((curve.eval_inverse64(0.1) + curve.eval_inverse64(0.9)) * 0.5);
    assert!((render(&d)[0] - expected).abs() < 1e-12);
}

#[test]
fn nonlinear_samples_use_active_profile_after_conversion_not_source_provenance() {
    use photocraft_cms::Curve;
    use photocraft_doc::{ImportCompositing, SourceColorContext};
    let mut d = doc(SampleType::F64);
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.source_color_context = Some(SourceColorContext { original_icc: Some(Builtin::Srgb.profile().to_bytes()), source_precision: 0 });
    let mut active = Builtin::Srgb.profile().clone();
    active.trc = Some([Curve::Gamma(2.0), Curve::Gamma(2.0), Curve::Gamma(2.0)]);
    d.icc_profile = Some(active.with_encoded_bytes().to_bytes());
    let mut top = layer(&d, [1.0; 4]);
    top.opacity = 0.5;
    top.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    d.layers = vec![layer(&d, [0.0, 0.0, 0.0, 1.0]), top];
    assert!((render(&d)[0] - 0.5_f64.sqrt()).abs() < 1e-14);
}

#[test]
#[ignore = "24 MP release kernel timing; run explicitly with --release --ignored --nocapture"]
fn bench_normal_wide_24mp() {
    use std::{hint::black_box, time::Instant};
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let backdrop = [0.2_f64, 0.3, 0.4, 0.75];
    let source = [0.8_f64, 0.7, 0.6, 0.5];
    let start = Instant::now();
    for _ in 0..24_000_000 {
        black_box(photocraft_compose::psblend::composite(
            BlendMode::Normal,
            black_box(backdrop.map(|v| v as f32)),
            black_box(source.map(|v| v as f32)),
            black_box(0.5),
        ));
    }
    let old = start.elapsed();
    let start = Instant::now();
    for _ in 0..24_000_000 {
        black_box(space.normal(black_box(backdrop), black_box(source), black_box(0.5), GimpOperator::Union).unwrap());
    }
    let wide = start.elapsed();
    println!("24 MP streamed Normal kernel: legacy f32 {old:?}; checked f64 {wide:?}; ratio {:.3}", wide.as_secs_f64() / old.as_secs_f64());
}

#[test]
fn prepared_reduction_matches_f64_premultiplied_average_and_damage() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    let mut d = doc(SampleType::F64);
    d.size = Size::new(4, 1);
    let mut l = Layer::raster("wide", d.pixel_format());
    for (x, pixel) in [[-1.0, 2.0, 3.0, 0.25], [1.0, 4.0, 5.0, 0.75], [0.3, 0.4, 0.5, 1.0], [0.7, 0.8, 0.9, 1.0]].iter().enumerate() {
        l.surface_mut().unwrap().try_write_pixel_f64(x as i32, 0, pixel).unwrap();
    }
    d.layers = vec![l];
    let prepared = PreparedRenderer::new(&d).unwrap();
    let full = prepared.render_reduced(2, 1, None, &mut |_| Ok(())).unwrap();
    assert_eq!(full.px[0], [0.5, 3.5, 4.5, 0.5]);
    let damage = prepared.render_reduced(2, 1, Some(Rect::new(2, 0, 3, 1)), &mut |_| Ok(())).unwrap();
    assert_eq!(damage.rect, Rect::new(1, 0, 2, 1));
    assert_eq!(damage.px, [full.px[1]]);
    assert!(prepared.render_reduced(0, 1, None, &mut |_| Ok(())).is_err());
}
#[test]
fn modern_multiply_checked_coverage_hdr_and_precision() {
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let b = [2.0, -3.0, 1.0 + f64::EPSILON, 0.25];
    let s = [4.0, 2.0, 1.0, 0.5];
    for op in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
        let a = 0.25;
        let ab = 0.25;
        let ao = match op {
            GimpOperator::Union => a + ab * (1.0 - a),
            GimpOperator::ClipToBackdrop => ab,
            GimpOperator::ClipToLayer => a,
            GimpOperator::Intersection => a * ab,
        };
        let actual = space.multiply(&space, b, s, 0.5, op).unwrap();
        assert_eq!(actual[3], ao);
        for i in 0..3 {
            let product = b[i] * s[i];
            let expected = match op {
                GimpOperator::Union => (ab * (1.0 - a) * b[i] + a * (1.0 - ab) * s[i] + ab * a * product) / ao,
                GimpOperator::ClipToBackdrop => (1.0 - a) * b[i] + a * product,
                GimpOperator::ClipToLayer => (1.0 - ab) * s[i] + ab * product,
                GimpOperator::Intersection => product,
            };
            assert_eq!(actual[i], expected);
        }
    }
    assert_eq!(space.multiply(&space, b, s, 0.0, GimpOperator::ClipToBackdrop).unwrap(), b);
    assert_eq!(space.multiply(&space, [0.0; 4], s, 1.0, GimpOperator::Union).unwrap(), s);
    assert!(space.multiply(&space, [f64::MAX, 1.0, 1.0, 1.0], [2.0, 2.0, 2.0, 1.0], 1.0, GimpOperator::Union).is_err());
    assert!(space.multiply(&space, [f64::NAN, 0.0, 0.0, 0.0], s, 0.0, GimpOperator::Union).is_err());
    let low = f64::from(1_u32) / f64::from(u32::MAX);
    assert_eq!(space.multiply(&space, [low, low, low, 1.0], [1.0; 4], 1.0, GimpOperator::Intersection).unwrap()[0].to_bits(), low.to_bits());
}
#[test]
#[ignore = "24 MP release Multiply kernel timing; run explicitly with --release --ignored --nocapture"]
fn bench_multiply_wide_24mp() {
    use std::{hint::black_box, time::Instant};
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let b = [0.2, 0.3, 0.4, 0.75];
    let s = [0.8, 0.7, 0.6, 0.5];
    let start = Instant::now();
    for _ in 0..24_000_000 {
        black_box(photocraft_compose::psblend::composite(BlendMode::Multiply, black_box(b.map(|v| v as f32)), black_box(s.map(|v| v as f32)), black_box(0.5)));
    }
    let legacy = start.elapsed();
    let start = Instant::now();
    for _ in 0..24_000_000 {
        black_box(space.multiply(&space, black_box(b), black_box(s), black_box(0.5), GimpOperator::ClipToBackdrop).unwrap());
    }
    let wide = start.elapsed();
    println!("24 MP streamed Multiply kernel: legacy f32 {legacy:?}; checked f64 {wide:?}; ratio {:.3}", wide.as_secs_f64() / legacy.as_secs_f64());
}

#[test]
fn modern_separable_basic_modes_keep_hdr_low_bits_and_checked_domains() {
    use photocraft_compose::gimp_blend::{GimpBlendMode, mode};
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let b = [-2.0, 4.0, 0.25, 0.5];
    let s = [3.0, -1.0, 0.75, 0.25];
    for (mode, overlap) in [
        (GimpBlendMode::Screen, [7.0, 7.0, 0.8125]),
        (GimpBlendMode::Difference, [5.0, 5.0, 0.5]),
        (GimpBlendMode::Addition, [1.0, 3.0, 1.0]),
        (GimpBlendMode::Subtract, [-5.0, 5.0, -0.5]),
        (GimpBlendMode::Exclusion, [13.0, 11.0, 0.625]),
        (GimpBlendMode::Overlay, [-12.0, 13.0, 0.375]),
        (GimpBlendMode::GrainMerge, [0.5, 2.5, 0.5]),
        (GimpBlendMode::GrainExtract, [-4.5, 5.5, 0.0]),
        (GimpBlendMode::LinearLight, [3.0, 1.0, 0.75]),
        (GimpBlendMode::DarkenOnly, [-2.0, -1.0, 0.25]),
        (GimpBlendMode::LightenOnly, [3.0, 4.0, 0.75]),
    ] {
        let out = space.separable(&space, b, s, 0.5, GimpOperator::Intersection, mode).unwrap();
        assert_eq!(&out[..3], &overlap);
        assert_eq!(out[3], 0.0625);
        let out = space.separable(&space, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).unwrap();
        assert_eq!(&out[..3], &std::array::from_fn::<_, 3, _>(|i| b[i] * 0.875 + overlap[i] * 0.125));
        assert_eq!(out[3], 0.5);
        assert!(space.separable(&space, [f64::NAN, 0.0, 0.0, 0.0], s, 0.0, GimpOperator::Union, mode).is_err());
    }
    let low = 1.0 / f64::from(u32::MAX);
    for mode in [GimpBlendMode::Addition, GimpBlendMode::Subtract, GimpBlendMode::Difference, GimpBlendMode::Screen, GimpBlendMode::Exclusion] {
        let out = space.separable(&space, [low, low, low, 1.0], [0.0, 0.0, 0.0, 1.0], 1.0, GimpOperator::Intersection, mode).unwrap();
        assert_eq!(out[0].to_bits(), low.to_bits());
    }
    assert!(space.separable(&space, [0.0, 0.0, 0.0, f64::MAX], [1.0; 4], 1.0, GimpOperator::Union, GimpBlendMode::Addition).is_err());
    assert!(space.separable(&space, [f64::MAX, 1.0, 1.0, 1.0], [f64::MAX, 1.0, 1.0, 1.0], 1.0, GimpOperator::Union, GimpBlendMode::Addition).is_err());
    assert!(space.separable(&space, [0.0; 4], [0.0; 4], 0.0, GimpOperator::Union, GimpBlendMode::Normal).is_err());
    for source in [5, 22, -31, 62, i32::MAX] {
        assert!(mode(source).is_err());
    }
}

#[test]
#[ignore = "24 MP checked basic blend timing; run --release --ignored --nocapture"]
fn bench_basic_blends_wide_24mp() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    use std::{hint::black_box, time::Instant};
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    for mode in [
        GimpBlendMode::Screen,
        GimpBlendMode::Difference,
        GimpBlendMode::Addition,
        GimpBlendMode::Subtract,
        GimpBlendMode::Exclusion,
        GimpBlendMode::Overlay,
        GimpBlendMode::GrainMerge,
        GimpBlendMode::LinearLight,
        GimpBlendMode::HardLight,
    ] {
        let start = Instant::now();
        for _ in 0..24_000_000 {
            black_box(
                space
                    .separable(
                        &space,
                        black_box([0.2, 0.3, 0.4, 0.75]),
                        black_box([0.8, 0.7, 0.6, 0.5]),
                        black_box(0.5),
                        GimpOperator::ClipToBackdrop,
                        black_box(mode),
                    )
                    .unwrap(),
            );
        }
        println!("24 MP {mode:?} linear f64 arithmetic {:?}", start.elapsed());
    }
}

#[test]
fn modern_hard_light_branches_and_hdr_scope_are_checked() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let out = space.separable(&space, [0.2, 0.4, 0.8, 1.0], [0.25, 0.5, 0.75, 1.0], 1.0, GimpOperator::Intersection, GimpBlendMode::HardLight).unwrap();
    for (actual, expected) in out.into_iter().zip([0.1, 0.4, 0.9, 1.0]) {
        assert!((actual - expected).abs() < 1e-15);
    }
    assert!(space.separable(&space, [-0.1, 0.4, 0.8, 1.0], [0.25, 0.5, 0.75, 1.0], 1.0, GimpOperator::Intersection, GimpBlendMode::HardLight).is_err());
    assert!(space.separable(&space, [0.2, 0.4, 0.8, 1.0], [1.25, 0.5, 0.75, 1.0], 1.0, GimpOperator::Intersection, GimpBlendMode::HardLight).is_err());
}

#[test]
#[ignore = "24 MP checked min/max timing; run --release --ignored --nocapture"]
fn bench_darken_lighten_wide_24mp() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    use std::{hint::black_box, time::Instant};
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    for mode in [GimpBlendMode::DarkenOnly, GimpBlendMode::LightenOnly] {
        let start = Instant::now();
        for _ in 0..24_000_000 {
            black_box(
                space
                    .separable(
                        &space,
                        black_box([0.2, 0.3, 0.4, 0.75]),
                        black_box([0.8, 0.7, 0.6, 0.5]),
                        black_box(0.5),
                        GimpOperator::ClipToBackdrop,
                        black_box(mode),
                    )
                    .unwrap(),
            );
        }
        println!("24 MP {mode:?} linear f64 arithmetic {:?}", start.elapsed());
    }
}

#[test]
fn prepared_lab_space_keeps_analytical_implicit_profile_definition() {
    use photocraft_cms::{Intent, Transform};
    let profile = Builtin::Srgb.profile();
    let lab = Builtin::LabD50.profile();
    let forward = Transform::new(profile, lab, Intent::RelativeColorimetric, false).unwrap();
    let reverse = Transform::new(lab, profile, Intent::RelativeColorimetric, false).unwrap();
    let b = [0.02, 0.19, 0.33, 1.0];
    let s = [0.19, 0.02, 0.7, 1.0];
    let mut lb = [0.0; 3];
    let mut ls = [0.0; 3];
    forward.try_eval_f64(&b[..3], &mut lb).unwrap();
    forward.try_eval_f64(&s[..3], &mut ls).unwrap();
    let mixture: [f64; 3] = std::array::from_fn(|c| (lb[c] + ls[c]) * 0.5);
    let mut expected = [0.0; 3];
    reverse.try_eval_f64(&mixture, &mut expected).unwrap();
    let working = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::Lab).unwrap();
    let actual = working.normal(b, s, 0.5, GimpOperator::Union).unwrap();
    assert_eq!(&actual[..3], &expected);
    let quantized = photocraft_cms::Profile::parse(&profile.to_bytes()).unwrap();
    let other = WorkingSpace::new(&quantized, SampleEncoding::NonlinearProfile, GimpSpace::Lab).unwrap().normal(b, s, 0.5, GimpOperator::Union).unwrap();
    assert!(actual.iter().zip(other).any(|(a, b)| a.to_bits() != b.to_bits()));
}

#[test]
fn active_implicit_profile_is_bound_and_explicit_same_bytes_keep_icc_semantics() {
    use photocraft_doc::{ImportCompositing, SourceColorContext, WorkingProfileInterpretation};
    let mut d = doc(SampleType::F64);
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    d.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
    let b = [0.02, 0.19, 0.33, 1.0];
    let s = [0.19, 0.02, 0.7, 0.5];
    let mut top = layer(&d, s);
    top.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    d.layers = vec![layer(&d, b), top];
    d.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(d.icc_profile.as_ref().unwrap());
    let before = d.clone();
    let implicit = render(&d);
    let exact = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb)
        .unwrap()
        .normal(b, s, 1.0, GimpOperator::Union)
        .unwrap();
    assert_eq!(implicit, exact);
    assert_eq!(d, before);
    d.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let declared = render(&d);
    let parsed = photocraft_cms::Profile::parse(d.icc_profile.as_ref().unwrap()).unwrap();
    let expected = WorkingSpace::new(&parsed, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap().normal(b, s, 1.0, GimpOperator::Union).unwrap();
    assert_eq!(declared, expected);
    assert_ne!(declared[0].to_bits(), implicit[0].to_bits());
    let mut stale = before.clone();
    stale.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
    let untouched = stale.clone();
    assert!(try_render(&stale, stale.bounds(), &mut |_| Ok(())).is_err());
    assert_eq!(stale, untouched);
    // A self-consistent digest for an arbitrary ICC is not semantic admission.
    stale.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(stale.icc_profile.as_ref().unwrap());
    assert!(try_render(&stale, stale.bounds(), &mut |_| Ok(())).is_err());
    let mut mismatched = before;
    mismatched.working_profile_interpretation = WorkingProfileInterpretation::implicit_sgray(mismatched.icc_profile.as_ref().unwrap());
    assert!(try_render(&mismatched, mismatched.bounds(), &mut |_| Ok(())).is_err());
}

#[test]
fn modern_contrast_and_division_checked_branches() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let b = [0.2, 0.4, 0.8, 1.0];
    let source = [0.25, 0.5, 0.75, 1.0];
    for (mode, expected) in [
        (GimpBlendMode::SoftLight, [0.12, 0.4, 0.88]),
        (GimpBlendMode::PinLight, [0.2, 0.4, 0.8]),
        (GimpBlendMode::ColorDodge, [0.2 / 0.75, 0.8, 3.2]),
        (GimpBlendMode::ColorBurn, [-2.2, -0.2, 1.0 - 0.2 / 0.75]),
        (GimpBlendMode::VividLight, [0.0, 0.4, 1.0]),
        (GimpBlendMode::Divide, [0.8, 0.8, 0.8 / 0.75]),
        (GimpBlendMode::LinearBurn, [-0.55, -0.1, 0.55]),
    ] {
        let result = space.separable(&space, b, source, 1.0, GimpOperator::Intersection, mode).unwrap();
        for (actual, expected) in result[..3].iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-14, "{mode:?}: {actual} != {expected}");
        }
        assert_eq!(result[3], 1.0);
        assert!(space.separable(&space, [-0.1, 0.4, 0.8, 1.0], source, 1.0, GimpOperator::Intersection, mode).is_err());
        assert!(space.separable(&space, b, [f64::NAN, 0.5, 0.75, 1.0], 1.0, GimpOperator::Intersection, mode).is_err());
        assert!(space.separable(&space, b, source, f64::INFINITY, GimpOperator::Intersection, mode).is_err());
    }
    for (mode, endpoint) in [
        (GimpBlendMode::ColorDodge, 1.0),
        (GimpBlendMode::ColorBurn, 0.0),
        (GimpBlendMode::Divide, 0.0),
        (GimpBlendMode::VividLight, 0.0),
        (GimpBlendMode::VividLight, 1.0),
    ] {
        assert!(space.separable(&space, b, [endpoint, 0.5, 0.75, 1.0], 1.0, GimpOperator::Intersection, mode).is_err());
    }
    let low = 0.25 + 2.0f64.powi(-40);
    let result = space.separable(&space, [low, 0.4, 0.8, 1.0], [0.5, 0.5, 0.5, 1.0], 1.0, GimpOperator::Intersection, GimpBlendMode::SoftLight).unwrap();
    assert_eq!(result[0].to_bits(), low.to_bits());
    // A singular source is irrelevant when it has no coverage.
    let unchanged = space.separable(&space, b, [0.0; 4], 1.0, GimpOperator::ClipToBackdrop, GimpBlendMode::Divide).unwrap();
    assert_eq!(unchanged, b);
    // Division overflow is a checked error, not an infinity or output clamp.
    assert!(space.separable(&space, [1.0; 4], [f64::from_bits(1), 0.5, 0.5, 1.0], 1.0, GimpOperator::Intersection, GimpBlendMode::Divide).is_err());
}

#[test]
#[ignore = "24 MP checked contrast timing; run --release --ignored --nocapture"]
fn bench_contrast_division_wide_24mp() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    use std::{hint::black_box, time::Instant};
    let space = WorkingSpace::new(Builtin::LinearSrgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    for mode in [
        GimpBlendMode::SoftLight,
        GimpBlendMode::PinLight,
        GimpBlendMode::ColorDodge,
        GimpBlendMode::ColorBurn,
        GimpBlendMode::VividLight,
        GimpBlendMode::Divide,
        GimpBlendMode::LinearBurn,
    ] {
        let start = Instant::now();
        for _ in 0..24_000_000 {
            black_box(
                space
                    .separable(
                        &space,
                        black_box([0.2, 0.3, 0.4, 0.75]),
                        black_box([0.8, 0.7, 0.6, 0.5]),
                        black_box(0.5),
                        GimpOperator::ClipToBackdrop,
                        black_box(mode),
                    )
                    .unwrap(),
            );
        }
        println!("24 MP {mode:?} linear f64 arithmetic {:?}", start.elapsed());
    }
}

fn floating_document() -> Document {
    use photocraft_doc::{
        import_compositing::{GimpComposition, ResolvedGimpComposition},
        item_identity::ItemRef,
        xcf_editing::FloatingAttachment,
        xcf_state::{ItemLocks, ItemState, XcfState},
    };
    let mut d = doc(SampleType::F64);
    d.size = Size::new(3, 1);
    let low = 0.25 + 2.0f64.powi(-40);
    d.layers = vec![layer(&d, [low, 0.5, 0.75, 1.0]), layer(&d, [0.75, 0.5, 0.25, 1.0])];
    let source = [0.75f64, 0.5, 0.25, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    *d.layers[1].surface_mut().unwrap() = Surface::try_with_default_bytes(d.pixel_format(), &source).unwrap();
    let mut state = XcfState::default();
    for (i, l) in d.layers.iter().enumerate() {
        state.items.push(ItemState {
            item: ItemRef::Layer(l.id),
            name: l.name.clone(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(if i == 0 { d.bounds() } else { Rect::new(1, 0, 2, 1) }),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: "qualified-floating-normal-profile".into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: 2,
                stored_operator: 1,
                pass_through: false,
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::ProfileRgb,
                    composite_space: GimpSpace::ProfileRgb,
                    operator: GimpOperator::Union,
                }),
            }),
        });
    }
    state.floating = Some(FloatingAttachment { layer: d.layers[1].id, target: ItemRef::Layer(d.layers[0].id) });
    d.xcf_state = Some(state);
    d.validate_xcf_state().unwrap();
    d
}

#[test]
fn floating_defaults_are_confined_to_declared_footprint_and_reject_unknown_policies() {
    let d = floating_document();
    let before = d.clone();
    let actual = try_render(&d, d.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(actual.px[0][0].to_bits(), (0.25 + 2.0f64.powi(-40)).to_bits());
    assert_eq!(actual.px[1], [0.75, 0.5, 0.25, 1.0]);
    assert_eq!(actual.px[2], [0.0; 4]);
    assert_eq!(d, before);
    for variant in 0..5 {
        let mut bad = d.clone();
        match variant {
            0 => bad.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().resolved.as_mut().unwrap().operator = GimpOperator::ClipToBackdrop,
            1 => bad.xcf_state.as_mut().unwrap().items[1].raster_bounds = Some(Rect::new(3, 0, 4, 1)),
            2 => bad.layers.swap(0, 1),
            3 => bad.layers[1].opacity = 0.5,
            _ => bad.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(Rect::new(0, 0, 2, 1)),
        }
        let before = bad.clone();
        assert!(try_render(&bad, bad.bounds(), &mut |_| Ok(())).is_err());
        assert_eq!(bad, before);
    }
    let cancelled = photocraft_raster::Interrupt::new(&|| true, &|_| {});
    assert!(photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&d, &cancelled).is_err());
    for value in [f64::NAN, f64::INFINITY] {
        let mut invalid = d.clone();
        let bytes = [value, 0.5, 0.25, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
        *invalid.layers[1].surface_mut().unwrap() = Surface::try_with_default_bytes(invalid.pixel_format(), &bytes).unwrap();
        let original_bytes = invalid.layers[1].surface().unwrap().default_bytes().to_vec();
        assert!(try_render(&invalid, invalid.bounds(), &mut |_| Ok(())).is_err());
        assert_eq!(invalid.layers[1].surface().unwrap().default_bytes(), original_bytes);
    }
}

#[test]
#[ignore = "24 MP release five legacy arithmetic/min-max kernels"]
fn bench_legacy_arithmetic_wide_24mp() {
    use photocraft_compose::gimp_blend::GimpBlendMode;
    use std::{hint::black_box, time::Instant};
    let working = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    for mode in [
        GimpBlendMode::LegacyDifference,
        GimpBlendMode::LegacyAddition,
        GimpBlendMode::LegacySubtract,
        GimpBlendMode::LegacyDarkenOnly,
        GimpBlendMode::LegacyLightenOnly,
    ] {
        let start = Instant::now();
        for _ in 0..24_000_000 {
            black_box(
                working
                    .separable(
                        &working,
                        black_box([0.2, 0.3, 0.4, 0.75]),
                        black_box([0.8, 0.7, 0.6, 0.5]),
                        black_box(0.5),
                        GimpOperator::ClipToBackdrop,
                        black_box(mode),
                    )
                    .unwrap(),
            );
        }
        println!("24 MP {mode:?}: {:?}", start.elapsed());
    }
}

#[test]
#[ignore = "24 MP release legacy SoftLight checked kernel"]
fn bench_legacy_softlight_wide_24mp() {
    use std::{hint::black_box, time::Instant};
    let working = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    let start = Instant::now();
    for _ in 0..24_000_000 {
        black_box(
            working
                .separable(
                    &working,
                    black_box([0.2, 0.3, 0.4, 0.75]),
                    black_box([0.8, 0.7, 0.6, 0.5]),
                    black_box(0.5),
                    GimpOperator::ClipToBackdrop,
                    photocraft_compose::gimp_blend::GimpBlendMode::LegacySoftLight,
                )
                .unwrap(),
        );
    }
    println!("24 MP LegacySoftLight {:?}", start.elapsed());
}
