#![cfg(feature = "corpus")]
//! Exact FREE256 source/edit witnesses through the checked ordered document renderer.
use photocraft_cms::{Builtin, WideWork};
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{
    ColorMode, Document, Layer, LayerMask, SampleEncoding, SampleType, Size, SourceColorContext,
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
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
fn original_free256_source_and_edit_pass_ordered_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("gimp_curves_hashes.json")).unwrap();
    for (name, version) in [("free256-rgb-u8", "0:0")] {
        let base = format!("curve-config-matrix/{name}");
        let raw = read(&root, &format!("{base}.source.0.native.bin"), &hashes);
        assert_eq!(raw.len(), 432);

        for (stage, suffix, xcf_suffix) in [("source", "ref.png", "xcf"), ("edit-source", "edit.ref.png", "edit.xcf")] {
            let xcf = read(&root, &format!("{base}.{xcf_suffix}"), &hashes);
            assert_eq!(&xcf[..14], b"gimp xcf v022\0");
            let state: Value = serde_json::from_slice(&read(&root, &format!("{base}.{stage}.state.json"), &hashes)).unwrap();
            assert_eq!(state["precision"], "GIMP_PRECISION_U8_NON_LINEAR");
            let filter = &state["layers"][0]["filters"][0];
            let stored =
                String::from_utf8(read(&root, &format!("{base}.{}.stored-config.txt", if stage == "source" { "source" } else { "edit" }), &hashes)).unwrap();
            assert!(stored.starts_with("(trc non-linear)"));
            assert_eq!(stored.matches("(curve-type free)").count(), 5);
            let tables: Vec<Vec<u64>> = stored
                .lines()
                .filter_map(|line| line.trim().strip_prefix("(samples 256 "))
                .map(|line| line.trim_end_matches(')').split_whitespace().map(|v| v.parse::<f64>().unwrap().to_bits()).collect())
                .collect();
            assert_eq!(tables.len(), 5);
            let args = vec![
                FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:curves::trc".into(), value: 1 } },
                FilterArgument {
                    name: "config".into(),
                    value: FilterValue::Config(
                        ["value", "red", "green", "blue", "alpha"]
                            .iter()
                            .zip(tables)
                            .map(|(name, samples)| FilterArgument {
                                name: (*name).into(),
                                value: FilterValue::Curve { free: true, points_f64_bits: vec![], point_types: vec![], samples_f64_bits: samples },
                            })
                            .collect(),
                    ),
                },
            ];
            let mut doc = Document::new(name, Size::new(12, 9), ColorMode::Rgb, SampleType::U8);
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
                    Surface::try_from_interleaved(photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), bounds, &[0; 108]).unwrap(),
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
                        name: "gimp:curves".into(),
                        stored_version: version.into(),
                        schema_epoch: "xcf-22-stock-curves-replace".into(),
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
                        stored_blend_space: -1080065455,
                        stored_composite_space: -1080065455,
                        stored_operator: -1080065455,
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
            // A real layer mask and a saved effect mask share the same layer owner.
            // Their independent bounds/defaults must select different registries.
            let mut masked = doc.clone();
            let mut layer_mask = LayerMask::hide_all();
            std::sync::Arc::make_mut(&mut layer_mask.surface).try_write_pixel_f64(0, 0, &[1.0]).unwrap();
            masked.layers[0].mask = Some(layer_mask);
            let mut metadata = masked.xcf_state.as_ref().unwrap().items[1].clone();
            metadata.item = ItemRef::Mask(MaskId(201));
            let m = metadata.mask.as_mut().unwrap();
            m.bounds = photocraft_doc::Rect::new(0, 0, 1, 1);
            m.offset = [0, 0];
            m.outside_f64_bits = 1.0f64.to_bits();
            masked.xcf_state.as_mut().unwrap().items.push(metadata);
            masked.validate_xcf_state().unwrap();
            let separate = try_render(&masked, masked.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(separate.px, actual.px);
            let mut ordered = doc.clone();
            let stack = &mut ordered.xcf_state.as_mut().unwrap().filter_stacks[0];
            let mut invert = stack.nodes[0].clone();
            invert.id = FilterId(99);
            invert.mask = None;
            invert.operation = OperationIdentity { name: "gegl:invert-gamma".into(), stored_version: "0:0".into(), schema_epoch: "observed-v022".into() };
            invert.arguments.clear();
            let policy = invert.composition.as_mut().unwrap();
            policy.stored_blend_space = -261651887;
            policy.stored_composite_space = -261651887;
            policy.stored_operator = -261651887;
            stack.nodes.push(invert);
            let appended = try_render(&ordered, ordered.bounds(), &mut |_| Ok(())).unwrap();
            for (a, b) in actual.px.iter().zip(&appended.px) {
                assert_eq!(a[3], b[3]);
                if a[3] > 0.0 {
                    for c in 0..3 {
                        assert!((b[c] - (1.0 - a[c])).abs() < 1e-14);
                    }
                }
            }
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
                    WideWork::SampleUnits(_) => Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancel during evaluation")),
                })
                .is_err()
            );
            assert_eq!(held, 0);
            assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancelled"))).is_err());
            assert_eq!(doc, before);
            let mut invalid = doc.clone();
            invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "unknown".into();
            assert!(try_render(&invalid, invalid.bounds(), &mut |_| Ok(())).is_err());
            let mut invalid = doc.clone();
            let FilterValue::Config(channels) = &mut invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1].value else { panic!() };
            let FilterValue::Curve { samples_f64_bits, .. } = &mut channels[0].value else { panic!() };
            samples_f64_bits[0] = f64::NAN.to_bits();
            assert!(try_render(&invalid, invalid.bounds(), &mut |_| Ok(())).is_err());
            let reference = photocraft_codecs::decode(&read(&root, &format!("{base}.{suffix}"), &hashes)).unwrap();
            let mut worst = 0.0_f64;
            for y in 0..9 {
                for x in 0..12 {
                    for c in 0..4 {
                        worst = worst.max((actual.px[(y * 12 + x) as usize][c].clamp(0.0, 1.0) - f64::from(reference.get(x, y, c))).abs());
                    }
                }
            }
            println!("{name}/{stage} checked consumer max {}/255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0, "{name}/{stage}: {worst}");
        }
    }
}
