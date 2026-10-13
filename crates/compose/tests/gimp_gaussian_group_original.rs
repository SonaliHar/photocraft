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
fn authored_gaussian_group_references_require_live_checked_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("gimp_gaussian_group_hashes.json")).unwrap();
    for group_mode in [28, 61] {
        for phase in ["source", "edit"] {
            let case = format!("group-{group_mode}-group-mask");
            let state: Value = serde_json::from_slice(&read(&root, &format!("{case}.fresh-{phase}_state.json"), &hashes)).unwrap();
            let xcf = read(&root, &format!("{case}.{}", if phase == "source" { "xcf" } else { "edit.xcf" }), &hashes);
            assert_eq!(&xcf[..14], b"gimp xcf v022\0");
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
            state_out.provenance.format_version = 22;
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
            assert_eq!(items[0]["filters"].as_array().unwrap().len(), 1);
            assert_eq!(items[0]["filters"][0]["operation"], "gegl:gaussian-blur");
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
            let node = gaussian_node(if phase == "source" { 0 } else { 1 });
            let effect_bounds = Rect::from_xywh(0, 0, 16, 12);
            state_out.filter_masks.push(
                photocraft_doc::FilterMask::try_new(
                    MaskId(101),
                    node.id,
                    Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), effect_bounds, &vec![0; 16 * 12 * 4])
                        .unwrap(),
                    effect_bounds,
                )
                .unwrap(),
            );
            state_out.items.push(ItemState {
                item: ItemRef::Mask(MaskId(101)),
                name: "saved empty effect selection".into(),
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
                    bounds: effect_bounds,
                    offset: [0, 0],
                    outside_f64_bits: 0,
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: false,
                }),
            });
            state_out.filter_stacks.push(photocraft_doc::xcf_filters::FilterStack { owner: ItemRef::Layer(group.id), nodes: vec![node], revision: 0 });
            doc.layers = vec![backdrop, group];
            doc.xcf_state = Some(state_out);
            doc.validate_xcf_state().unwrap();
            let before = doc.clone();
            if std::env::var_os("PHOTOCRAFT_XCF_GAUSSIAN_GROUP_PROBE").is_some() {
                if group_mode == 28 {
                    recursive_probe(&doc, bounds, &reference, phase);
                }
                continue;
            }
            if group_mode == 61 {
                assert!(try_render(&doc, doc.bounds(), &mut |_| Ok(())).is_err());
                continue;
            }
            let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(doc, before);
            let mut disabled = doc.clone();
            disabled.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
            let disabled_before = disabled.clone();
            let disabled_actual = try_render(&disabled, disabled.bounds(), &mut |_| Ok(())).unwrap();
            verify_disabled(&disabled, &disabled_actual.px, phase);
            assert_eq!(disabled, disabled_before);
            let node = &mut disabled.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            let policy = node.composition.as_mut().unwrap();
            policy.stored_blend_space = -1474592175;
            policy.stored_composite_space = -1474592175;
            policy.stored_operator = -1474592175;
            let saved_disabled = disabled.clone();
            let saved_actual = try_render(&disabled, disabled.bounds(), &mut |_| Ok(())).unwrap();
            verify_disabled(&disabled, &saved_actual.px, phase);
            assert_eq!(disabled, saved_disabled);
            let mut unknown_disabled = saved_disabled.clone();
            unknown_disabled.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "unknown".into();
            let unknown_before = unknown_disabled.clone();
            assert!(try_render(&unknown_disabled, unknown_disabled.bounds(), &mut |_| Ok(())).is_err());
            assert_eq!(unknown_disabled, unknown_before);
            disabled.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = true;
            assert!(try_render(&disabled, disabled.bounds(), &mut |_| Ok(())).is_err());
            disabled = disabled_before.clone();
            assert!(
                try_render(&disabled, disabled.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancelled"))).is_err()
            );
            assert_eq!(disabled, disabled_before);
            for variant in 0..3 {
                let mut bad = doc.clone();
                let node = &mut bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                match variant {
                    0 => node.source_region = Some(1 - node.source_region.unwrap()),
                    1 => node.operation.stored_version = "unknown".into(),
                    _ => node.arguments[4].value = photocraft_doc::xcf_filters::FilterValue::Bool(false),
                }
                let unchanged = bad.clone();
                assert!(try_render(&bad, bad.bounds(), &mut |_| Ok(())).is_err());
                assert_eq!(bad, unchanged);
            }
            let mut changed = doc.clone();
            if let photocraft_doc::LayerContent::Group(group) = &mut changed.layers[1].content {
                for child in &mut group.children {
                    let bytes: Vec<u8> = [0.2_f32, 0.8, 0.1, 1.0].into_iter().flat_map(f32::to_ne_bytes).collect();
                    *child.surface_mut().unwrap() = Surface::try_with_default_bytes(PixelFormat::RGBA32F, &bytes).unwrap();
                }
            }
            let current = try_render(&changed, changed.bounds(), &mut |_| Ok(())).unwrap();
            let delta = current.px.iter().zip(&actual.px).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0_f64, f64::max);
            assert!(delta > 0.01, "Gaussian must evaluate current children");
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
                assert!(try_render(&generic, generic.bounds(), &mut |_| Ok(())).is_err());
            }
            if std::env::var_os("PHOTOCRAFT_XCF_GROUP_BENCH").is_some() && phase == "source" {
                let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap();
                let start = std::time::Instant::now();
                let iterations = std::env::var("PHOTOCRAFT_XCF_GROUP_BENCH").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(125_000);
                for _ in 0..iterations {
                    std::hint::black_box(renderer.render(doc.bounds(), &mut |_| Ok(())).unwrap());
                }
                println!("{iterations} streamed 192px authored Gaussian group{group_mode}: {:?}", start.elapsed());
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
fn verify_disabled(doc: &Document, actual: &[[f64; 4]], phase: &str) {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").map(PathBuf::from).unwrap().join("group-disabled-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), "06c29caff71c0da5c64ed8fbc59f7ff5de4a30d7a59d992d344b062f59779bcc");
    let packet: Value = serde_json::from_slice(&bytes).unwrap();
    let reopened = &packet["records"].as_array().unwrap().iter().find(|r| r["phase"] == phase).unwrap()["reopened"];
    assert_eq!(reopened["filter_visible"], false);
    let read = |descriptor: &Value| {
        let bytes = fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), descriptor["sha256"].as_str().unwrap());
        bytes
    };
    if let photocraft_doc::LayerContent::Group(group) = &doc.layers[1].content {
        for child in &group.children {
            let captured = reopened["children"].as_array().unwrap().iter().find(|r| r["name"] == child.name).unwrap();
            let native = captured["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"].is_null()).unwrap();
            let bytes: Vec<u8> = read(native).as_chunks::<4>().0.iter().flat_map(|p| u32::from_le_bytes(*p).to_ne_bytes()).collect();
            let bounds = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(child.id)).unwrap().raster_bounds.unwrap();
            assert_eq!(child.surface().unwrap().try_read_interleaved(bounds).unwrap(), bytes);
        }
    }
    let projection = reopened["projection"]["reads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["region"] == "buffer-extent" && r["requested_format"] == "R'G'B'A double")
        .unwrap();
    let expected = read(projection);
    let mut worst = 0.0f64;
    for (pixel, raw) in actual.iter().zip(expected.as_chunks::<32>().0.iter()) {
        let alpha = f64::from_le_bytes(raw[24..32].try_into().unwrap());
        for (c, value) in pixel.iter().enumerate() {
            if c < 3 && alpha == 0.0 && pixel[3] == 0.0 {
                continue;
            }
            worst = worst.max((value - f64::from_le_bytes(raw[c * 8..c * 8 + 8].try_into().unwrap())).abs());
        }
    }
    println!("disabled group/{phase}: {worst}");
    assert!(worst < 1e-6);
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

fn gaussian_node(region: u32) -> photocraft_doc::xcf_filters::FilterNode {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity};
    photocraft_doc::xcf_filters::FilterNode {
        id: photocraft_doc::item_identity::FilterId(101),
        operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: "xcf22-isolated-small-auto-v1".into() },
        source_display_name: Some("Own original blur".into()),
        source_icon: None,
        enabled: true,
        opacity_f64_bits: 1.0_f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(region),
        mask: Some(ItemRef::Mask(MaskId(101))),
        auxiliary: vec![],
        composition: Some(GimpComposition {
            compatibility_epoch: "observed-group-v022".into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: -643996976,
            stored_composite_space: -643996976,
            stored_operator: -643996976,
            resolved: None,
            pass_through: false,
        }),
        arguments: vec![
            FilterArgument { name: "std-dev-x".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1061997773)) },
            FilterArgument { name: "std-dev-y".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1067869798)) },
            FilterArgument { name: "filter".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 0 } },
            FilterArgument { name: "abyss-policy".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 } },
            FilterArgument { name: "clip-extent".into(), value: FilterValue::Bool(true) },
        ],
    }
}

fn recursive_probe(doc: &Document, extent: Rect, reference: &photocraft_codecs::Image, phase: &str) {
    use photocraft_algo::gimp::neighborhood::{
        GaussianAbyss, GaussianLimits, GaussianParams, try_gaussian_fir, try_gaussian_small_auto, try_gaussian_young_recursive,
    };
    use photocraft_compose::gimp_composite::WorkingSpace;
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let working = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
    let mut input_doc = doc.clone();
    let backdrop = input_doc.layers.remove(0);
    let group_id = input_doc.layers[0].id;
    input_doc.layers[0].mask = None;
    let state = input_doc.xcf_state.as_mut().unwrap();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.items.retain(|item| item.item != ItemRef::Layer(backdrop.id) && !matches!(item.item, ItemRef::Mask(_)));
    input_doc.validate_xcf_state().unwrap();
    let mut pixels = try_render(&input_doc, extent, &mut |_| Ok(())).unwrap();
    // Diagnostic-only independent public get_buffer evidence. Never an importer
    // fallback or an authoritative replacement for the live group evaluator.
    if let Some(full) = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS") {
        let evidence = PathBuf::from(full).join("group-buffer-evidence");
        let observation_bytes = fs::read(evidence.join("observations.json")).unwrap();
        let observation_hash: String = Sha256::digest(&observation_bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(observation_hash, "bbe3111958d216f036cedf2bda84a4a303386b0904a1eaeaa1108fb9cb2a77b2");
        let observations: Value = serde_json::from_slice(&observation_bytes).unwrap();
        let id = format!("group-28-{phase}-filter0-mask0");
        let record = observations["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap();
        let descriptor =
            record["group"]["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"].is_null()).unwrap();
        let bytes = fs::read(evidence.join(descriptor["file"].as_str().unwrap())).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash, descriptor["sha256"]);
        assert_eq!(bytes.len(), pixels.px.len() * 16);
        let captured: Vec<[f64; 4]> =
            bytes.as_chunks::<16>().0.iter().map(|p| std::array::from_fn(|c| f64::from(f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())))).collect();
        let mut worst = [0.0_f64; 4];
        let mut visible_worst = [0.0_f64; 4];
        for (live, independent) in pixels.px.iter().zip(&captured) {
            for c in 0..4 {
                worst[c] = worst[c].max((live[c] - independent[c]).abs());
                if independent[3] > 0.0 {
                    visible_worst[c] = visible_worst[c].max((live[c] - independent[c]).abs());
                }
            }
        }
        println!("group input {phase}: live/native max RGBA {worst:?}, visible {visible_worst:?}");
        let enabled_id = format!("group-28-{phase}-filter1-mask1");
        let enabled = observations["records"].as_array().unwrap().iter().find(|r| r["id"] == enabled_id).unwrap();
        let projection =
            enabled["projection"]["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"].is_null()).unwrap();
        let projection_bytes = fs::read(evidence.join(projection["file"].as_str().unwrap())).unwrap();
        let mut reference_delta = 0.0_f64;
        for (index, pixel) in projection_bytes.as_chunks::<16>().0.iter().enumerate() {
            let alpha = f32::from_le_bytes(pixel[12..16].try_into().unwrap());
            for c in 0..4 {
                let sample = if alpha == 0.0 { 0.0 } else { f64::from(f32::from_le_bytes(pixel[c * 4..c * 4 + 4].try_into().unwrap())).clamp(0.0, 1.0) };
                reference_delta = reference_delta.max((sample - f64::from(reference.get((index % 16) as u32, (index / 16) as u32, c))).abs());
            }
        }
        println!("fresh GIMP projection vs original PNG {phase}: {}/255", reference_delta * 255.0);
        if std::env::var_os("PHOTOCRAFT_XCF_GAUSSIAN_GROUP_CAPTURE_INPUT").is_some() {
            pixels.px = captured;
        }
    }
    let mask_first = std::env::var_os("PHOTOCRAFT_XCF_GAUSSIAN_GROUP_MASK_FIRST").is_some();
    let original_group = doc.layers.iter().find(|layer| layer.id == group_id).unwrap();
    let original_mask = &original_group.mask.as_ref().unwrap().surface;
    let mut linear = Vec::new();
    for (index, p) in pixels.px.iter().enumerate() {
        let rgb = std::array::from_fn::<_, 3, _>(|i| profile.trc.as_ref().unwrap()[i].eval64(p[i]));
        let alpha = if mask_first {
            let x = extent.x0 + (index % extent.width() as usize) as i32;
            let y = extent.y0 + (index / extent.width() as usize) as i32;
            let mut opacity = [0.0];
            original_mask.try_read_pixel_f64(x, y, &mut opacity).unwrap();
            p[3] * opacity[0]
        } else {
            p[3]
        };
        linear.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
    }
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_region_f64_with(extent, &linear, &photocraft_raster::Interrupt::default()).unwrap();
    let kernel = std::env::var("PHOTOCRAFT_XCF_GAUSSIAN_GROUP_KERNEL").unwrap_or_else(|_| "fir".into());
    let params = GaussianParams {
        std_dev_x: f64::from(f32::from_bits(1061997773)),
        std_dev_y: f64::from(f32::from_bits(1067869798)),
        abyss: if std::env::var_os("PHOTOCRAFT_XCF_GAUSSIAN_GROUP_ABYSS_NONE").is_some() { GaussianAbyss::None } else { GaussianAbyss::Clamp },
        clip_extent: true,
    };
    let limits = GaussianLimits { resident_bytes: 64 * 1024 * 1024, work_units: 20_000_000 };
    let interrupt = photocraft_raster::Interrupt::default();
    let input_extent =
        if phase == "source" && std::env::var_os("PHOTOCRAFT_XCF_GAUSSIAN_GROUP_CANVAS_REGION").is_some() { extent.intersect(&doc.bounds()) } else { extent };
    let output = match kernel.as_str() {
        "finite" => try_gaussian_small_auto(&source, extent, input_extent, params, limits, &interrupt).unwrap(),
        "iir" => try_gaussian_young_recursive(&source, extent, input_extent, params, limits, &interrupt).unwrap(),
        "fir-iir" | "iir-fir" => {
            let first = GaussianParams { std_dev_y: 0.0, ..params };
            let second = GaussianParams { std_dev_x: 0.0, ..params };
            let mid = if kernel == "fir-iir" {
                try_gaussian_fir(&source, extent, input_extent, first, limits, &interrupt).unwrap()
            } else {
                try_gaussian_young_recursive(&source, extent, input_extent, first, limits, &interrupt).unwrap()
            };
            if kernel == "fir-iir" {
                try_gaussian_young_recursive(&mid, extent, input_extent, second, limits, &interrupt).unwrap()
            } else {
                try_gaussian_fir(&mid, extent, input_extent, second, limits, &interrupt).unwrap()
            }
        }
        _ => try_gaussian_fir(&source, extent, input_extent, params, limits, &interrupt).unwrap(),
    };
    if let Some(full) = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS") {
        let evidence = PathBuf::from(full).join("group-filter-merge-evidence");
        let observation_bytes = fs::read(evidence.join("observations.json")).unwrap();
        let observation_hash: String = Sha256::digest(&observation_bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(observation_hash, "a680999b7b4a9ffc60145051ade30ecdc39d56621c850359a960491e964b519d");
        let observations: Value = serde_json::from_slice(&observation_bytes).unwrap();
        let id = format!("group-28-{phase}");
        let record = observations["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap();
        let descriptor = record["public_group_merge"]["raster_before"]["drawable"]["reads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["region"] == "buffer-extent" && r["requested_format"] == "RGBA double")
            .unwrap();
        let bytes = fs::read(evidence.join(descriptor["file"].as_str().unwrap())).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash, descriptor["sha256"]);
        let mut worst = [0.0_f64; 4];
        let mut location = [(0, 0, 0.0, 0.0); 4];
        for (index, pixel) in bytes.as_chunks::<32>().0.iter().enumerate() {
            let x = extent.x0 + (index % extent.width() as usize) as i32;
            let y = extent.y0 + (index / extent.width() as usize) as i32;
            let mut actual = [0.0; 4];
            output.try_read_pixel_f64(x, y, &mut actual).unwrap();
            for c in 0..4 {
                let expected = f64::from_le_bytes(pixel[c * 8..c * 8 + 8].try_into().unwrap());
                let delta = (actual[c] - expected).abs();
                if delta > worst[c] {
                    worst[c] = delta;
                    location[c] = (x, y, actual[c], expected);
                }
            }
        }
        println!("direct filtered linear output {phase}/{kernel}: RGBA {worst:?}, locations {location:?}");
    }
    let group = doc.layers.iter().find(|layer| layer.id == group_id).unwrap();
    let mask = &group.mask.as_ref().unwrap().surface;
    let mut worst = 0.0_f64;
    let mut worst_location = (0, 0, 0, 0.0, 0.0);
    for y in 0..12 {
        for x in 0..16 {
            let mut b = [0.0; 4];
            backdrop.surface().unwrap().try_read_pixel_f64(x, y, &mut b).unwrap();
            let mut out = working.normal(b, [0.0; 4], 0.0, GimpOperator::Union).unwrap();
            if extent.contains(x, y) {
                let mut p = [0.0; 4];
                output.try_read_pixel_f64(x, y, &mut p).unwrap();
                let rgb = std::array::from_fn::<_, 3, _>(|i| profile.trc.as_ref().unwrap()[i].eval_inverse64(p[i]));
                let mut opacity = [0.0];
                mask.try_read_pixel_f64(x, y, &mut opacity).unwrap();
                out = working.normal(b, [rgb[0], rgb[1], rgb[2], p[3]], if mask_first { 1.0 } else { opacity[0] }, GimpOperator::Union).unwrap();
            }
            if out[3] == 0.0 {
                out = [0.0; 4];
            }
            for (c, sample) in out.iter().enumerate() {
                let delta = (sample - f64::from(reference.get(x as u32, y as u32, c))).abs();
                if delta > worst {
                    worst_location = (x, y, c, *sample, f64::from(reference.get(x as u32, y as u32, c)));
                }
                worst = worst.max(delta);
            }
        }
    }
    println!("isolated {kernel} probe(mask_first={mask_first},abyss={:?})/{phase}: {}/255 at {worst_location:?}", params.abyss, worst * 255.0);
}
