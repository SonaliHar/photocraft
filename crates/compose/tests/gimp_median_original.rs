#![cfg(feature = "corpus")]
//! Hash-bound opaque RGB/U8 Median source/edit full RGBA references.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{
    ColorMode, Document, Layer, SampleEncoding, SampleType, Size, SourceColorContext,
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::Surface;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn read(root: &std::path::Path, file: &str, hashes: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(file)).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, hashes[file].as_str().unwrap(), "{file}");
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_median_source_and_edit_matches_full_rgba_reference() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("gimp_median_hashes.json")).unwrap();
    for (name, version) in [("gegl-median-blur", "0:0")] {
        let base = format!("operations/{name}/operation-{name}");
        let raw = read(&root, &format!("{base}.source.0.native.bin"), &hashes);
        assert_eq!(raw.len(), 4096);
        assert!(raw.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        for (stage, suffix, xcf_suffix) in [("reopened", "ref.png", "xcf"), ("edited", "edit.ref.png", "edit.xcf")] {
            let xcf = read(&root, &format!("{base}.{xcf_suffix}"), &hashes);
            assert_eq!(&xcf[..14], b"gimp xcf v022\0");
            let state: Value = serde_json::from_slice(&read(&root, &format!("{base}.{stage}_state.json"), &hashes)).unwrap();
            assert_eq!(state["precision"], "GIMP_PRECISION_U8_NON_LINEAR");
            let filter = &state["layers"][0]["filters"][0];
            let args = filter["config"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, value)| FilterArgument {
                    name: name.clone(),
                    value: match name.as_str() {
                        "radius" => FilterValue::Number(ExactNumber::Signed { width: 32, value: value.as_i64().unwrap() }),
                        "percentile" | "alpha-percentile" => FilterValue::Number(ExactNumber::Float32Bits((value.as_f64().unwrap() as f32).to_bits())),
                        "neighborhood" | "abyss-policy" => FilterValue::Enum { domain: format!("gegl:median-blur::{name}"), value: 1 },
                        "high-precision" => FilterValue::Bool(value.as_bool().unwrap()),
                        _ => panic!("unknown authored arg"),
                    },
                })
                .collect();
            let mut doc = Document::new(name, Size::new(32, 32), ColorMode::Rgb, SampleType::U8);
            doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
            doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
            doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
            doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 150 });
            let mut layer = Layer::raster("original", doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), doc.bounds(), &raw).unwrap();
            let mut imported = XcfState::default();
            imported.provenance.format_version = 22;
            let bounds = doc.bounds();
            imported.items.push(ItemState {
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
                composition: None,
            });
            imported.filter_masks.push(
                photocraft_doc::FilterMask::try_new(
                    MaskId(7),
                    FilterId(1),
                    Surface::try_from_interleaved(photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), bounds, &vec![0; 1024])
                        .unwrap(),
                    bounds,
                )
                .unwrap(),
            );
            imported.items.push(ItemState {
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
            imported.filter_stacks.push(FilterStack {
                owner: ItemRef::Layer(layer.id),
                revision: 0,
                nodes: vec![FilterNode {
                    id: FilterId(1),
                    operation: OperationIdentity {
                        name: name.replacen('-', ":", 1),
                        stored_version: version.into(),
                        schema_epoch: "xcf22-rgb-u8-quantized-median-v1".into(),
                    },
                    arguments: args,
                    source_display_name: None,
                    source_icon: None,
                    enabled: filter["visible"].as_bool().unwrap(),
                    opacity_f64_bits: 1.0_f64.to_bits(),
                    source_clipping: Some(0),
                    source_region: Some(0),
                    mask: Some(ItemRef::Mask(MaskId(7))),
                    auxiliary: vec![],
                    composition: Some(GimpComposition {
                        compatibility_epoch: "observed-v022".into(),
                        source_mode: 62,
                        legacy: false,
                        stored_blend_space: -261651887,
                        stored_composite_space: -261651887,
                        stored_operator: -261651887,
                        resolved: None,
                        pass_through: false,
                    }),
                }],
            });
            doc.layers.push(layer);
            doc.xcf_state = Some(imported);
            doc.validate_xcf_state().unwrap();
            let before = doc.clone();
            if stage == "reopened"
                && let Some(iterations) = std::env::var("PHOTOCRAFT_XCF_MEDIAN_BENCH").ok().and_then(|v| v.parse::<usize>().ok())
            {
                let prepared = photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap();
                let started = std::time::Instant::now();
                for _ in 0..iterations.min(10000) {
                    std::hint::black_box(prepared.render(doc.bounds(), &mut |_| Ok(())).unwrap());
                }
                println!("{} isolated1024px Median frames: {:?}", iterations.min(10000), started.elapsed());
            }
            let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(doc, before);
            let cancelled = || true;
            assert!(
                photocraft_compose::gimp_render::PreparedRenderer::new(&doc)
                    .unwrap()
                    .render_with_interrupt(doc.bounds(), &mut |_| Ok(()), &photocraft_raster::Interrupt::cancel_only(&cancelled))
                    .is_err()
            );
            for variant in 0..4 {
                let mut bad = doc.clone();
                let n = &mut bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                match variant {
                    0 => n.operation.stored_version = "unknown".into(),
                    1 => n.source_region = Some(1),
                    2 => n.arguments.iter_mut().find(|a| a.name == "high-precision").unwrap().value = FilterValue::Bool(true),
                    _ => n.composition.as_mut().unwrap().stored_operator = 1,
                }
                assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&bad).is_err());
            }
            let mut held = 0i64;
            let mut deny = |work| {
                match work {
                    photocraft_cms::WideWork::ReserveBytes(n) => held += n as i64,
                    photocraft_cms::WideWork::ReleaseBytes(n) => held -= n as i64,
                    photocraft_cms::WideWork::SampleUnits(n) if n > 100_000 => {
                        return Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied Median work"));
                    }
                    _ => {}
                }
                Ok(())
            };
            assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut deny).is_err());
            assert_eq!(held, 0);
            assert_eq!(doc, before);
            let reference = photocraft_codecs::decode(&read(&root, &format!("{base}.{suffix}"), &hashes)).unwrap();
            let mut worst = 0.0_f64;
            for y in 0..32 {
                for x in 0..32 {
                    for c in 0..4 {
                        worst = worst.max((actual.px[(y * 32 + x) as usize][c].clamp(0.0, 1.0) - f64::from(reference.get(x, y, c))).abs());
                        assert_eq!(
                            (actual.px[(y * 32 + x) as usize][c].clamp(0., 1.) * 255.).round() as u8,
                            (f64::from(reference.get(x, y, c)) * 255.).round() as u8
                        );
                    }
                }
            }
            println!("{name}/{stage} checked consumer max {}/255", worst * 255.0);
            assert!(worst <= 1.0 / 65535.0, "{name}/{stage}: {worst}");
        }
    }
}
