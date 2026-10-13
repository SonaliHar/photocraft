#![cfg(feature = "corpus")]
//! Independent literal RGB/alpha/HDR/profile raster Median qualification.
use photocraft_algo::gimp::neighborhood::{GaussianLimits, MedianNeighborhood, MedianParams, try_median_blur, try_median_blur_quantized_encoded};
use photocraft_cms::{Builtin, Profile};
use photocraft_doc::{
    ColorMode, Document, Layer, SampleEncoding, SampleType, Size, SourceColorContext, XcfState,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use photocraft_doc::{PixelFormat, Rect};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn read(root: &Path, descriptor: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), descriptor["sha256"].as_str().unwrap());
    bytes
}
fn native_samples(root: &Path, drawable: &Value) -> Vec<f64> {
    let descriptor = drawable["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"].is_null()).unwrap();
    let _declared_bounds = &descriptor["rect"];
    let bytes = read(root, descriptor);
    if drawable["drawable_native_format"].as_str().unwrap().contains("double") {
        bytes.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect()
    } else {
        bytes.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect()
    }
}
fn visible_samples(root: &Path, drawable: &Value) -> Vec<f64> {
    let descriptor =
        drawable["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"] == "R'G'B'A double").unwrap();
    read(root, descriptor).as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect()
}
fn document(root: &Path, record: &Value) -> Document {
    let source = &record["reopened"];
    let config = &source["config"];
    let double = source["precision"] == "GIMP_PRECISION_DOUBLE_NON_LINEAR";
    let depth = if double { SampleType::F64 } else { SampleType::F32 };
    let mut doc = Document::new("Own Median color", Size::new(16, 12), ColorMode::Rgb, depth);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let assigned = &source["profiles"]["assigned"];
    let original = if assigned.is_null() { None } else { Some(std::sync::Arc::new(read(root, assigned))) };
    doc.icc_profile = Some(original.clone().unwrap_or_else(|| Builtin::Srgb.profile().to_bytes()));
    if original.is_none() {
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
    }
    doc.source_color_context = Some(SourceColorContext { original_icc: original, source_precision: if double { 750 } else { 650 } });
    let bounds = doc.bounds();
    let mut layer = Layer::raster("Own literal RGB alpha HDR", doc.pixel_format());
    let descriptor = source["layer"]["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"].is_null()).unwrap();
    let bytes = read(root, descriptor);
    let native: Vec<u8> = if double {
        bytes.as_chunks::<8>().0.iter().flat_map(|v| u64::from_le_bytes(*v).to_ne_bytes()).collect()
    } else {
        bytes.as_chunks::<4>().0.iter().flat_map(|v| u32::from_le_bytes(*v).to_ne_bytes()).collect()
    };
    *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), bounds, &native).unwrap();
    let mut state = XcfState::default();
    state.provenance.format_version = 22;
    state.items.push(ItemState {
        item: ItemRef::Layer(layer.id),
        name: layer.name.clone(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(2),
        source_flags: 0,
        raster_bounds: Some(bounds),
        mask: None,
        composition: Some(GimpComposition {
            source_mode: 28,
            stored_blend_space: 0,
            stored_composite_space: -1,
            stored_operator: -1,
            legacy: false,
            pass_through: false,
            compatibility_epoch: "qualified-Auto".into(),
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        }),
    });
    state.filter_masks.push(
        photocraft_doc::FilterMask::try_new(
            MaskId(7),
            FilterId(1),
            Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, depth, false), bounds, &vec![0; 16 * 12 * if double { 8 } else { 4 }])
                .unwrap(),
            bounds,
        )
        .unwrap(),
    );
    state.items.push(ItemState {
        item: ItemRef::Mask(MaskId(7)),
        name: "selection mask".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(3),
        source_flags: 0,
        raster_bounds: None,
        composition: None,
        mask: Some(MaskState {
            owner: ItemRef::Layer(layer.id),
            bounds,
            offset: [0, 0],
            outside_f64_bits: 0,
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        }),
    });
    let node = FilterNode {
        id: FilterId(1),
        operation: OperationIdentity { name: "gegl:median-blur".into(), stored_version: "0:0".into(), schema_epoch: "xcf22-rgb-alpha-median-v1".into() },
        source_display_name: None,
        source_icon: None,
        enabled: true,
        opacity_f64_bits: 1.0f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(0),
        mask: Some(ItemRef::Mask(MaskId(7))),
        auxiliary: vec![],
        arguments: vec![
            FilterArgument { name: "radius".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: config["radius"].as_i64().unwrap() }) },
            FilterArgument { name: "neighborhood".into(), value: FilterValue::Enum { domain: "gegl:median-blur::neighborhood".into(), value: 1 } },
            FilterArgument { name: "percentile".into(), value: FilterValue::Number(ExactNumber::Float32Bits(50f32.to_bits())) },
            FilterArgument { name: "alpha-percentile".into(), value: FilterValue::Number(ExactNumber::Float32Bits(50f32.to_bits())) },
            FilterArgument { name: "abyss-policy".into(), value: FilterValue::Enum { domain: "gegl:median-blur::abyss-policy".into(), value: 1 } },
            FilterArgument { name: "high-precision".into(), value: FilterValue::Bool(config["high-precision"].as_bool().unwrap()) },
        ],
        composition: Some(GimpComposition {
            source_mode: 62,
            stored_blend_space: -1474592175,
            stored_composite_space: -1474592175,
            stored_operator: -1474592175,
            legacy: false,
            pass_through: false,
            compatibility_epoch: "qualified-Replace".into(),
            resolved: None,
        }),
    };
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(layer.id), nodes: vec![node], revision: 0 });
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    doc
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn authored_raster_median_color_native_matches_checked_cms_kernel() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").map(PathBuf::from).unwrap().join("median-raster-color-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), "f5f9965d5f5ac36dfd0d015701f24efa64776a7fcc2183b1acbdb03a0412d1b1");
    let observations: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(observations["errors"].as_array().unwrap().is_empty());
    for record in observations["records"].as_array().unwrap() {
        read(&root, &record["source"]);
        let source = &record["reopened"];
        let visible_before = visible_samples(&root, &source["projection"]);
        let visible_after = visible_samples(&root, &record["merged_projection"]);
        assert_eq!(visible_before.len(), visible_after.len());
        let projection_delta = visible_before.iter().zip(&visible_after).map(|(a, b)| (a - b).abs()).fold(0.0f64, f64::max);
        assert!(projection_delta <= 2.0 / 255., "public merge changed appearance: {projection_delta}");
        let assigned = &source["profiles"]["assigned"];
        let profile = if assigned.is_null() { Builtin::Srgb.profile().clone() } else { Profile::parse(&read(&root, assigned)).unwrap() };
        let native = native_samples(&root, &source["layer"]);
        let bounds = Rect::new(0, 0, 16, 12);
        let output_bounds = bounds;
        let mut input = Surface::new(PixelFormat::RGBA64F);
        input.try_write_region_f64_with(bounds, &native, &Interrupt::NONE).unwrap();
        let before = input.clone();
        let config = &source["config"];
        let params = MedianParams {
            radius: config["radius"].as_i64().unwrap() as i32,
            neighborhood: MedianNeighborhood::Circle,
            percentile: 50.,
            alpha_percentile: 50.,
            high_precision: config["high-precision"].as_bool().unwrap(),
        };
        let limits = GaussianLimits { resident_bytes: 64 * 1024 * 1024, work_units: 10_000_000 };
        let output = if params.high_precision {
            try_median_blur(&input, bounds, bounds, params, limits, &Interrupt::NONE)
        } else {
            try_median_blur_quantized_encoded(&input, bounds, bounds, params, limits, &Interrupt::NONE)
        }
        .unwrap();
        assert_eq!(input, before);
        let expected = native_samples(&root, &record["merged_layer"]);
        let mut worst = 0.0f64;
        for (index, pixel) in expected.as_chunks::<4>().0.iter().enumerate() {
            let mut result = [0.; 4];
            output
                .try_read_pixel_f64(
                    output_bounds.x0 + (index % output_bounds.width() as usize) as i32,
                    output_bounds.y0 + (index / output_bounds.width() as usize) as i32,
                    &mut result,
                )
                .unwrap();
            let mut rgb = [0.; 3];
            rgb.copy_from_slice(&result[..3]);
            for channel in 0..3 {
                worst = worst.max((rgb[channel] * result[3] - pixel[channel] * pixel[3]).abs());
            }
            worst = worst.max((result[3] - pixel[3]).abs());
        }
        println!("{} native premultiplied: {}/255", record["id"].as_str().unwrap(), worst * 255.);
        assert!(worst <= 2.0 / 255., "{}: {worst}", record["id"]);
        let doc = document(&root, record);
        let original = doc.clone();
        let prepared = photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap();
        if record["id"] == "implicit-unit-f32.source"
            && let Some(iterations) = std::env::var("PHOTOCRAFT_XCF_RASTER_MEDIAN_BENCH").ok().and_then(|v| v.parse::<usize>().ok())
        {
            let started = std::time::Instant::now();
            for _ in 0..iterations.min(125_000) {
                std::hint::black_box(prepared.render(doc.bounds(), &mut |_| Ok(())).unwrap());
            }
            println!("{} isolated 192px finiteMedian renderer frames: {:?}", iterations.min(125_000), started.elapsed());
        }
        let rendered = prepared.render(doc.bounds(), &mut |_| Ok(())).unwrap();
        let mut render_error = 0.0f64;
        for (actual, reference) in rendered.px.iter().zip(expected.as_chunks::<4>().0.iter()) {
            for (a, b) in actual.iter().zip(reference) {
                render_error = render_error.max((a - b).abs());
            }
        }
        println!("{} actual renderer unclipped: {}/255", record["id"].as_str().unwrap(), render_error * 255.);
        assert!(render_error <= 2.0 / 255.);
        let display_transform = photocraft_cms::Transform::new(&profile, Builtin::Srgb.profile(), photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
        let mut visible_error = 0.0f64;
        for (actual, reference) in rendered.px.iter().zip(visible_before.as_chunks::<4>().0.iter()) {
            let mut rgb = [0.; 3];
            display_transform.try_eval_f64(&actual[..3], &mut rgb).unwrap();
            for (a, b) in [rgb[0], rgb[1], rgb[2], actual[3]].iter().zip(reference) {
                visible_error = visible_error.max((a - b).abs());
            }
        }
        println!("{} logical-canvas generic display: {}/255", record["id"].as_str().unwrap(), visible_error * 255.);
        assert!(visible_error <= 2.0 / 255.);
        assert_eq!(doc, original);
        let cancelled = || true;
        assert!(prepared.render_with_interrupt(doc.bounds(), &mut |_| Ok(()), &Interrupt::cancel_only(&cancelled)).is_err());
        let mut held = 0i64;
        let mut limited = |work| {
            match work {
                photocraft_cms::WideWork::ReserveBytes(n) => held += n as i64,
                photocraft_cms::WideWork::ReleaseBytes(n) => held -= n as i64,
                photocraft_cms::WideWork::SampleUnits(n) if n > 100_000 => {
                    return Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied work"));
                }
                _ => {}
            }
            Ok(())
        };
        assert!(prepared.render(doc.bounds(), &mut limited).is_err());
        assert_eq!(held, 0);
        assert_eq!(doc, original);
        for variant in 0..4 {
            let mut bad = doc.clone();
            let node = &mut bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            match variant {
                0 => node.operation.stored_version = "unknown".into(),
                1 => node.source_region = Some(1),
                2 => node.arguments[0].value = FilterValue::Number(ExactNumber::Signed { width: 32, value: 6 }),
                _ => node.composition.as_mut().unwrap().stored_blend_space = -643996976,
            }
            assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&bad).is_err());
        }
        if record["case"] == "implicit-hdr-f64" && record["phase"] == "edit" {
            let render_constant = |red: f64| {
                let mut changed = doc.clone();
                let raw: Vec<u8> = [red, 0.4, 1.75, 0.5].into_iter().flat_map(f64::to_ne_bytes).collect();
                *changed.layers[0].surface_mut().unwrap() = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &raw).unwrap();
                photocraft_compose::gimp_render::PreparedRenderer::new(&changed).unwrap().render(changed.bounds(), &mut |_| Ok(())).unwrap().px[0][0]
            };
            assert!((render_constant(0.5 + 2f64.powi(-40)) - render_constant(0.5)).abs() > 2f64.powi(-43));
        }
    }
}
