#![cfg(feature = "corpus")]
//! Hash-bound isolated/pass-through group masks, with independent child extents.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{
    BlendMode, ColorMode, Document, Layer, LayerMask, PixelFormat, Rect, SampleEncoding, SampleType, Size, SourceColorContext,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{ItemRef, MaskId},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::Surface;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn read(root: &std::path::Path, name: &str, hashes: &Value) -> Vec<u8> {
    let raw = fs::read(root.join(name)).unwrap();
    let hash: String = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, hashes[name].as_str().unwrap());
    raw
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn authored_masked_group_normal_and_passthrough_match_checked_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("gimp_group_hashes.json")).unwrap();
    for group_mode in [28, 61] {
        for phase in ["source", "edit"] {
            let case = format!("group-{group_mode}-none-mask");
            let state: Value = serde_json::from_slice(&read(&root, &format!("{case}.fresh-{phase}_state.json"), &hashes)).unwrap();
            let xcf = read(&root, &format!("{case}.{}", if phase == "source" { "xcf" } else { "edit.xcf" }), &hashes);
            assert_eq!(&xcf[..14], b"gimp xcf v013\0");
            assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
            assert_eq!(state["size"], serde_json::json!([16, 12]));
            assert!(state["assigned_icc_sha256"].is_null());
            let reference = photocraft_codecs::decode(&read(&root, &format!("{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let mut doc = Document::new(&case, Size::new(16, 12), ColorMode::Rgb, SampleType::F32);
            doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
            doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
            doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
            doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
            let mut state_out = XcfState::default();
            state_out.provenance.format_version = 13;
            let items = state["layers"].as_array().unwrap();
            assert_eq!(items.len(), 4);
            let mut rasters = Vec::new();
            for (index, item) in items.iter().enumerate().skip(1) {
                assert!(item["mask"].is_null());
                assert!(item["filters"].as_array().unwrap().is_empty());
                let native = &item["native"];
                let raw = fs::read(root.join(native["file"].as_str().unwrap())).unwrap();
                let hash: String = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect();
                assert_eq!(hash, native["sha256"].as_str().unwrap());
                let bytes: Vec<u8> = raw.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect();
                let bounds = item_bounds(item);
                let mut layer = Layer::raster(item["name"].as_str().unwrap(), doc.pixel_format());
                *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), bounds, &bytes).unwrap();
                layer.opacity = (item["opacity"].as_f64().unwrap() / 100.0) as f32;
                layer.blend = if index == 2 { BlendMode::Multiply } else { BlendMode::Normal };
                assert_eq!(item["composition"]["mode"], if index == 2 { 30 } else { 28 });
                rasters.push(layer);
            }
            let backdrop = rasters.remove(2);
            let lower = rasters.remove(0);
            let foreground = rasters.remove(0);
            let mut group = Layer::group(items[0]["name"].as_str().unwrap(), vec![foreground, lower]);
            group.blend = if group_mode == 61 { BlendMode::PassThrough } else { BlendMode::Normal };
            let bounds = item_bounds(&items[0]);
            assert_eq!(bounds, if phase == "source" { Rect::from_xywh(0, -1, 12, 9) } else { Rect::from_xywh(-1, 0, 12, 11) });
            assert!(items[0]["filters"].as_array().unwrap().is_empty());
            assert_eq!(items[0]["opacity"], 100.0);
            let mask = &items[0]["mask"];
            assert_eq!(mask["size"], items[0]["size"]);
            assert_eq!(mask["apply"], true);
            assert_eq!(mask["show"], false);
            let raw = fs::read(root.join(mask["file"].as_str().unwrap())).unwrap();
            let hash: String = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hash, mask["sha256"].as_str().unwrap());
            let bytes: Vec<u8> = raw.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect();
            let mut layer_mask = LayerMask::reveal_all();
            layer_mask.surface = std::sync::Arc::new(Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), bounds, &bytes).unwrap());
            group.mask = Some(layer_mask);
            let parent = Some(group.id);
            if let photocraft_doc::LayerContent::Group(children) = &group.content {
                for child in &children.children {
                    let original = items.iter().find(|item| item["name"].as_str() == Some(child.name.as_str())).unwrap();
                    state_out.items.push(item_state(child, parent, if child.blend == BlendMode::Multiply { 30 } else { 28 }, Some(item_bounds(original))));
                }
            }
            state_out.items.push(item_state(&backdrop, None, 28, Some(item_bounds(&items[3]))));
            state_out.items.push(item_state(&group, None, group_mode, Some(bounds)));
            state_out.items.push(ItemState {
                item: ItemRef::Mask(MaskId(100)),
                name: "original group mask".into(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: None,
                composition: None,
                mask: Some(MaskState {
                    owner: ItemRef::Layer(group.id),
                    bounds,
                    offset: [0, 0],
                    outside_f64_bits: 0.0_f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: true,
                    linked: true,
                }),
            });
            doc.layers = vec![backdrop, group];
            doc.xcf_state = Some(state_out);
            doc.validate_xcf_state().unwrap();
            let before = doc.clone();
            let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(doc, before);
            assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancelled"))).is_err());
            assert_eq!(doc, before);
            if group_mode == 28 {
                let mut explicit = doc.clone();
                explicit.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
                let assigned = try_render(&explicit, explicit.bounds(), &mut |_| Ok(())).unwrap();
                // Explicit same-byte assignment changes ICC math, not group initialization.
                for (before, after) in actual.px.iter().zip(&assigned.px) {
                    assert_eq!(before[3].to_bits(), after[3].to_bits());
                    for c in 0..3 {
                        assert!((before[c] - after[c]).abs() < 1.0 / 255.0);
                    }
                }
                let mut generic = doc.clone();
                for item in &mut generic.xcf_state.as_mut().unwrap().items {
                    if item
                        .composition
                        .as_ref()
                        .is_some_and(|policy| policy.compatibility_epoch == photocraft_doc::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH)
                    {
                        item.composition.as_mut().unwrap().compatibility_epoch = "unqualified-group".into();
                    }
                }
                let wrong = try_render(&generic, generic.bounds(), &mut |_| Ok(())).unwrap();
                let delta = actual.px.iter().zip(&wrong.px).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0_f64, f64::max);
                assert!(delta > 0.1, "unqualified groups must retain their previous semantics");
            }
            if std::env::var_os("PHOTOCRAFT_XCF_GROUP_BENCH").is_some() && phase == "source" {
                let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap();
                let start = std::time::Instant::now();
                for _ in 0..125_000 {
                    std::hint::black_box(renderer.render(doc.bounds(), &mut |_| Ok(())).unwrap());
                }
                println!("24 MP streamed authored group{group_mode}: {:?}", start.elapsed());
            }
            let mut worst = 0.0_f64;
            for y in 0..12 {
                for x in 0..16 {
                    for c in 0..4 {
                        let delta = (actual.px[(y * 16 + x) as usize][c] - f64::from(reference.get(x, y, c))).abs();
                        worst = worst.max(delta);
                    }
                }
            }
            println!("group{group_mode}/{phase}: {}/255", worst * 255.0);
            assert!(worst <= 1.0 / 65535.0, "{worst}");
        }
    }
}
fn item_bounds(item: &Value) -> Rect {
    Rect::from_xywh(
        item["offset"][0].as_i64().unwrap() as i32,
        item["offset"][1].as_i64().unwrap() as i32,
        item["size"][0].as_u64().unwrap() as u32,
        item["size"][1].as_u64().unwrap() as u32,
    )
}
fn item_state(layer: &Layer, parent: Option<photocraft_doc::LayerId>, source_mode: i32, bounds: Option<Rect>) -> ItemState {
    ItemState {
        item: ItemRef::Layer(layer.id),
        name: layer.name.clone(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: parent.map(ItemRef::Layer),
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: bounds,
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: if matches!(layer.content, photocraft_doc::LayerContent::Group(_)) && source_mode == 28 {
                photocraft_doc::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH.into()
            } else {
                "authored-group-Auto".into()
            },
            source_mode,
            legacy: false,
            stored_blend_space: if source_mode == 30 { -1 } else { 0 },
            stored_composite_space: -1,
            stored_operator: if source_mode == 30 { -2 } else { -1 },
            pass_through: source_mode == 61,
            resolved: Some(ResolvedGimpComposition {
                blend_space: GimpSpace::LinearRgb,
                composite_space: GimpSpace::LinearRgb,
                operator: if source_mode == 30 { GimpOperator::ClipToBackdrop } else { GimpOperator::Union },
            }),
        }),
    }
}
