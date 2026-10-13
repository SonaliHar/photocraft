#![cfg(feature = "corpus")]
//! Exact FREE256 source/edit witnesses through the checked ordered document renderer.
use photocraft_cms::{Intent, Profile, Transform, WideWork};
use photocraft_compose::gimp_render::try_render;
const POLICY_CODE: i32 = -1080065455;
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
fn original_structured_levels_trc_clamps_match_checked_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("gimp_levels_hashes.json")).unwrap();
    for trc in [0, 1] {
        for clamps in ["00", "01", "10", "11"] {
            let name = format!("levels-trc{trc}-clamps{clamps}");
            let version = "0:0";
            let base = format!("curve-config-matrix/{name}");

            for (stage, suffix, xcf_suffix) in [("source", "ref.png", "xcf"), ("edit-source", "edit.ref.png", "edit.xcf")] {
                let xcf = read(&root, &format!("{base}.{xcf_suffix}"), &hashes);
                assert_eq!(&xcf[..14], b"gimp xcf v022\0");
                let state: Value = serde_json::from_slice(&read(&root, &format!("{base}.{stage}.state.json"), &hashes)).unwrap();
                assert_eq!(state["precision"], if trc == 0 { "GIMP_PRECISION_FLOAT_LINEAR" } else { "GIMP_PRECISION_FLOAT_NON_LINEAR" });
                assert_eq!(state["assigned_profile"]["present"], false);
                let raw = read(&root, &format!("{base}.{stage}.0.native.bin"), &hashes);
                assert_eq!(raw.len(), 1728);
                let raw: Vec<u8> = raw.as_chunks::<4>().0.iter().flat_map(|b| f32::from_le_bytes(*b).to_ne_bytes()).collect();
                let filter = &state["layers"][0]["filters"][0];
                let stored =
                    String::from_utf8(read(&root, &format!("{base}.{}.stored-config.txt", if stage == "source" { "source" } else { "edit" }), &hashes))
                        .unwrap();
                assert!(stored.starts_with(if trc == 0 { "(trc linear)" } else { "(trc non-linear)" }));
                let mut channels = Vec::new();
                for line in stored.lines().skip(3) {
                    let tokens: Vec<&str> = line.trim().trim_matches(['(', ')']).split_whitespace().collect();
                    if tokens[0] == "channel" {
                        channels.push(FilterArgument { name: tokens[1].into(), value: FilterValue::Config(vec![]) });
                    } else {
                        let FilterValue::Config(fields) = &mut channels.last_mut().unwrap().value else { panic!() };
                        fields.push(FilterArgument {
                            name: tokens[0].into(),
                            value: FilterValue::Number(ExactNumber::Float64Bits(tokens[1].parse::<f64>().unwrap().to_bits())),
                        });
                    }
                }
                let args = vec![
                    FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:levels::trc".into(), value: trc } },
                    FilterArgument { name: "clamp-input".into(), value: FilterValue::Bool(clamps.starts_with('1')) },
                    FilterArgument { name: "clamp-output".into(), value: FilterValue::Bool(clamps.ends_with('1')) },
                    FilterArgument { name: "config".into(), value: FilterValue::Config(channels) },
                ];
                let mut doc = Document::new(&name, Size::new(12, 9), ColorMode::Rgb, SampleType::F32);
                doc.sample_encoding = Some(if trc == 0 { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile });
                doc.icc_profile = Some(
                    photocraft_cms::xcf_color::effective_profile(
                        None,
                        photocraft_cms::ColorSpace::Rgb,
                        if trc == 0 { photocraft_cms::xcf_color::Encoding::Linear } else { photocraft_cms::xcf_color::Encoding::NonlinearProfile },
                    )
                    .unwrap()
                    .to_bytes(),
                );
                doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
                doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: if trc == 0 { 600 } else { 650 } });
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
                        Surface::try_from_interleaved(photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), bounds, &[0; 432])
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
                            name: "gimp:levels".into(),
                            stored_version: version.into(),
                            schema_epoch: "xcf-22-stock-levels-replace".into(),
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
                            stored_blend_space: POLICY_CODE,
                            stored_composite_space: POLICY_CODE,
                            stored_operator: POLICY_CODE,
                            resolved: None,
                            pass_through: false,
                        }),
                    }],
                });
                doc.layers.push(layer);
                doc.xcf_state = Some(imported);
                doc.validate_xcf_state().unwrap();
                let before = doc.clone();
                let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                assert_eq!(doc, before);
                assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancelled"))).is_err());
                assert_eq!(doc, before);
                let mut held = 0u64;
                assert!(
                    try_render(&doc, doc.bounds(), &mut |work| match work {
                        WideWork::ReserveBytes(bytes) => {
                            held += bytes;
                            Ok(())
                        }
                        WideWork::ReleaseBytes(bytes) => {
                            held -= bytes;
                            Ok(())
                        }
                        WideWork::SampleUnits(_) => Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancel during Levels")),
                    })
                    .is_err()
                );
                assert_eq!(held, 0);
                let mut invalid = doc.clone();
                invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                    FilterValue::Enum { domain: "gimp:levels::trc".into(), value: 2 };
                assert!(try_render(&invalid, invalid.bounds(), &mut |_| Ok(())).is_err());
                let reference = photocraft_codecs::decode(&read(&root, &format!("{base}.{suffix}"), &hashes)).unwrap();
                let source_profile = Profile::parse(&read(&root, &format!("{base}.{stage}.effective_profile.icc"), &hashes)).unwrap();
                let display =
                    Transform::new(&source_profile, &Profile::parse(reference.icc.as_ref().unwrap()).unwrap(), Intent::RelativeColorimetric, false).unwrap();
                let mut worst = 0.0_f64;
                for y in 0..9 {
                    for x in 0..12 {
                        let pixel = actual.px[(y * 12 + x) as usize];
                        let mut rgb = [0.0; 3];
                        display.try_eval_f64(&pixel[..3], &mut rgb).unwrap();
                        let evaluated = if pixel[3] <= 0.0 { [0.0; 4] } else { [rgb[0], rgb[1], rgb[2], pixel[3]] };
                        for (c, sample) in evaluated.iter().enumerate() {
                            worst = worst.max((sample.clamp(0.0, 1.0) - f64::from(reference.get(x, y, c))).abs());
                        }
                    }
                }
                println!("{name}/{stage} checked consumer max {}/255", worst * 255.0);
                assert!(worst <= 2.0 / 65535.0, "{name}/{stage}: {worst}");
            }
        }
    }
}
