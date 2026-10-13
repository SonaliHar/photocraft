#![cfg(feature = "corpus")]
//! Original opaque RGB/U8 Gaussian slice through the actual ordered document consumer.
use photocraft_cms::{Builtin, WideWork};
use photocraft_compose::gimp_render::PreparedRenderer;
use photocraft_doc::{
    ColorMode, Document, Layer, Rect, SampleEncoding, SampleType, Size, SourceColorContext,
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn read(root: &std::path::Path, name: &str, hashes: &Value) -> Vec<u8> {
    let raw = fs::read(root.join(name)).unwrap();
    let hash: String = Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(hash, hashes[name].as_str().unwrap());
    raw
}
fn node(x: f32) -> FilterNode {
    FilterNode {
        id: FilterId(1),
        operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: "observed-v022".into() },
        source_display_name: None,
        source_icon: None,
        enabled: true,
        opacity_f64_bits: 1.0_f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(0),
        mask: Some(ItemRef::Mask(MaskId(7))),
        auxiliary: vec![],
        arguments: vec![
            FilterArgument { name: "std-dev-x".into(), value: FilterValue::Number(ExactNumber::Float32Bits(x.to_bits())) },
            FilterArgument { name: "std-dev-y".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1.5_f32.to_bits())) },
            FilterArgument { name: "filter".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 0 } },
            FilterArgument { name: "abyss-policy".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 } },
            FilterArgument { name: "clip-extent".into(), value: FilterValue::Bool(true) },
        ],
        composition: Some(GimpComposition {
            compatibility_epoch: "observed-v022".into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: -856260015,
            stored_composite_space: -856260015,
            stored_operator: -856260015,
            resolved: None,
            pass_through: false,
        }),
    }
}
fn document(raw: &[u8], x: f32) -> Document {
    let mut doc = Document::new("Gaussian original", Size::new(96, 80), ColorMode::Rgb, SampleType::U8);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 150 });
    let mut layer = Layer::raster("Original base", doc.pixel_format());
    *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), doc.bounds(), raw).unwrap();
    let mut state = XcfState::default();
    state.provenance.format_version = 22;
    state.items.push(ItemState {
        item: ItemRef::Layer(layer.id),
        name: "Original base".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(2),
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        mask: None,
        composition: None,
    });
    let bounds = doc.bounds();
    state.filter_masks.push(
        photocraft_doc::FilterMask::try_new(
            MaskId(7),
            FilterId(1),
            Surface::try_from_interleaved(photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), bounds, &vec![0; 96 * 80]).unwrap(),
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
        source_tattoo: None,
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
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(layer.id), nodes: vec![node(x)], revision: 0 });
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    doc
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_gaussian_source_edit_full_and_banded_are_immutable_and_match() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("gimp_gaussian_hashes.json")).unwrap();
    let raw = read(&root, "filter-owner-layer.source.0.native.bin", &hashes);
    assert_eq!(raw.len(), 96 * 80 * 4);
    assert!(raw.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    for (x, phase, reference, xcf) in [
        (2.25, "reopened", "filter-owner-layer.ref.png", "filter-owner-layer.xcf"),
        (3.75, "edited", "filter-owner-layer.edit.ref.png", "filter-owner-layer.edit.xcf"),
    ] {
        assert_eq!(&read(&root, xcf, &hashes)[..14], b"gimp xcf v022\0");
        let state: Value = serde_json::from_slice(&read(&root, &format!("filter-owner-layer.{phase}_state.json"), &hashes)).unwrap();
        let c = &state["layers"][0]["filters"][0]["config"];
        assert_eq!(c["std-dev-x"].as_f64(), Some(f64::from(x)));
        assert_eq!(c["filter"], "auto");
        assert_eq!(c["abyss-policy"], "clamp");
        assert_eq!(c["clip-extent"], true);
        let doc = document(&raw, x);
        let before = doc.clone();
        let renderer = PreparedRenderer::new(&doc).unwrap();
        let mut held = 0_u64;
        let mut peak = 0;
        let mut poll = |event| {
            match event {
                WideWork::ReserveBytes(n) => {
                    held += n;
                    peak = peak.max(held)
                }
                WideWork::ReleaseBytes(n) => held -= n,
                _ => {}
            }
            Ok(())
        };
        let full = renderer.render(doc.bounds(), &mut poll).unwrap();
        assert_eq!(held, 96 * 80 * 32);
        assert!(peak <= 34 * 1024 * 1024);
        let expected = photocraft_codecs::decode(&read(&root, reference, &hashes)).unwrap();
        let mut worst = 0.0_f64;
        for y in 0..80 {
            for x in 0..96 {
                for c in 0..4 {
                    worst = worst.max((full.px[(y * 96 + x) as usize][c].clamp(0.0, 1.0) - f64::from(expected.get(x, y, c))).abs());
                }
            }
        }
        println!("{phase} actual ordered Gaussian worst {}/255", worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
        for (y0, y1) in [(0, 17), (17, 53), (53, 80)] {
            let band = renderer.render(Rect::new(0, y0, 96, y1), &mut |_| Ok(())).unwrap();
            for (i, p) in band.px.iter().enumerate() {
                let expected = full.px[y0 as usize * 96 + i];
                for c in 0..4 {
                    assert!((p[c] - expected[c]).abs() < 1e-9);
                }
            }
        }
        let reduced = renderer.render_reduced(48, 40, None, &mut |_| Ok(())).unwrap();
        assert_eq!(reduced.px.len(), 48 * 40);
        let cancelled = || true;
        assert!(renderer.render_with_interrupt(doc.bounds(), &mut |_| Ok(()), &Interrupt::cancel_only(&cancelled)).is_err());
        assert_eq!(doc, before);
        let started = std::sync::atomic::AtomicBool::new(false);
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || started.load(std::sync::atomic::Ordering::Relaxed) && polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 2;
        let mut held = 0_i64;
        let mut account = |event| {
            match event {
                WideWork::SampleUnits(n) if n > 10_000_000 => started.store(true, std::sync::atomic::Ordering::Relaxed),
                WideWork::ReserveBytes(n) => held += n as i64,
                WideWork::ReleaseBytes(n) => held -= n as i64,
                _ => {}
            }
            Ok(())
        };
        assert!(renderer.render_with_interrupt(doc.bounds(), &mut account, &Interrupt::cancel_only(&cancel)).is_err());
        assert!(started.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(held, 0);
        assert_eq!(doc, before);
        let mut bad = doc.clone();
        bad.xcf_state.as_mut().unwrap().items[0].raster_bounds = None;
        assert!(PreparedRenderer::new(&bad).is_err());
        let mut bad = doc.clone();
        bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = None;
        assert!(PreparedRenderer::new(&bad).is_err());
        let mut bad = doc.clone();
        bad.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[1.0]).unwrap();
        assert!(PreparedRenderer::new(&bad).is_err());
        let mut bad = doc.clone();
        bad.source_color_context.as_mut().unwrap().original_icc = Some(Builtin::Srgb.profile().to_bytes());
        assert!(PreparedRenderer::new(&bad).is_err());
        let mut bad = doc.clone();
        bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(2.5_f32.to_bits()));
        assert!(PreparedRenderer::new(&bad).is_err());
        let mut held = 0_i64;
        let mut deny = |event| {
            match event {
                WideWork::ReserveBytes(n) if n >= 32 * 1024 * 1024 => return Err(photocraft_compose::gimp_composite::CompositionError::Invalid("deny cache")),
                WideWork::ReserveBytes(n) => held += n as i64,
                WideWork::ReleaseBytes(n) => held -= n as i64,
                _ => {}
            }
            Ok(())
        };
        assert!(renderer.render(doc.bounds(), &mut deny).is_err());
        assert_eq!(held, 0);
    }
}

#[test]
fn ordered_point_before_and_after_neighborhood_matches_independent_stages() {
    let raw: Vec<u8> = (0..96 * 80).flat_map(|i| if i % 96 < 32 { [208, 96, 48, 255] } else { [48, 80, 128, 255] }).collect();
    let base = document(&raw, 2.25);
    let gaussian = PreparedRenderer::new(&base).unwrap().render(base.bounds(), &mut |_| Ok(())).unwrap();
    let mut point = node(2.25);
    point.id = FilterId(2);
    point.operation.name = "gegl:invert-gamma".into();
    point.arguments.clear();
    point.mask = None;
    let policy = point.composition.as_mut().unwrap();
    policy.stored_blend_space = -261651887;
    policy.stored_composite_space = -261651887;
    policy.stored_operator = -261651887;
    let mut after = base.clone();
    after.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.push(point.clone());
    after.validate_xcf_state().unwrap();
    let unchanged = after.clone();
    let actual = PreparedRenderer::new(&after).unwrap().render(after.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(after, unchanged);
    for (actual, input) in actual.px.iter().zip(&gaussian.px) {
        for c in 0..3 {
            assert!((actual[c] - (1.0 - input[c])).abs() < 1e-12);
        }
        assert_eq!(actual[3], 1.0);
    }
    let mut before = base.clone();
    before.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.insert(0, point);
    before.validate_xcf_state().unwrap();
    let mut inverted = raw.clone();
    for p in inverted.as_chunks_mut::<4>().0.iter_mut() {
        for v in p.iter_mut().take(3) {
            *v = 255 - *v;
        }
    }
    let independent = document(&inverted, 2.25);
    let expected = PreparedRenderer::new(&independent).unwrap().render(independent.bounds(), &mut |_| Ok(())).unwrap();
    let unchanged = before.clone();
    let actual = PreparedRenderer::new(&before).unwrap().render(before.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(before, unchanged);
    for (actual, expected) in actual.px.iter().zip(&expected.px) {
        for c in 0..4 {
            assert!((actual[c] - expected[c]).abs() < 1e-12);
        }
    }
}
