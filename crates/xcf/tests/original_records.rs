//! Scoped original authoring witnesses retained locally while full-corpus publication is pending.
use photocraft_xcf::*;
fn load(relative: &str) -> XcfFile {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let bytes = std::fs::read(root.join(relative)).expect("reviewed local full-XCF corpus missing");
    decode(&bytes, &Default::default()).unwrap_or_else(|e| panic!("{relative}: {e}"))
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_point_effect_states_and_item_identities() {
    for operation in ["exposure", "invert-linear", "invert-gamma", "brightness-contrast"] {
        for suffix in ["xcf", "edit.xcf"] {
            let file = load(&format!("operations/gegl-{operation}/operation-gegl-{operation}.{suffix}"));
            let effect = file.layers.iter().flat_map(|d| &d.effects).find(|e| e.operation == format!("gegl:{operation}")).unwrap();
            println!("{operation}.{suffix}: header={} version={:?} state={:?}", file.version, effect.version, effect.state);
            for property in &effect.properties {
                match property.tag {
                    8 => assert_eq!(effect.state.enabled, Some(u32::from_be_bytes(property.payload.as_slice().try_into().unwrap()) == 1)),
                    33 => assert_eq!(effect.state.opacity_bits, Some(u32::from_be_bytes(property.payload.as_slice().try_into().unwrap()))),
                    46 => assert_eq!(effect.state.clipping, Some(u32::from_be_bytes(property.payload.as_slice().try_into().unwrap()))),
                    _ => {}
                }
            }
        }
    }
    for name in ["active-layers-paths-locks", "active-channels-paths-locks"] {
        let file = load(&format!("item-state-probe/{name}.xcf"));
        let active = file.layers.iter().chain(&file.channels).filter(|d| d.active).count();
        assert!(active > 0);
        assert!(file.paths.iter().any(|p| p.selected));
        assert!(file.paths.iter().any(|p| !p.strokes.is_empty()));
        for d in file.layers.iter().chain(&file.channels) {
            for p in &d.raw_properties {
                if p.tag == 20 {
                    assert_eq!(d.tattoo, Some(u32::from_be_bytes(p.payload.as_slice().try_into().unwrap())));
                }
            }
        }
    }
    for method in [0u32, 1, 2, u32::MAX] {
        let file = load(&format!("item-sets-three-types/channel-{method}.xcf"));
        assert!(file.item_sets.iter().any(|s| s.item_type == 1 && s.method == method));
    }
    let file = load("floating-layer.xcf");
    let floating = file.layers.iter().find(|d| d.floating_attachment.is_some()).unwrap();
    let target = floating.floating_attachment.unwrap();
    assert!(file.layers.iter().any(|d| d.record_offset == target));
    assert!(floating.unsupported.iter().any(|s| s.contains("floating")));
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_custom_unit_and_channel_effect_records() {
    for name in ["own-shared-text-unit.xcf", "own-shared-text-unit.edit.xcf"] {
        let file = load(&format!("text-custom-unit/{name}"));
        assert_eq!(file.version, 19);
        assert_eq!(file.dpi, (96., 120.));
        let Unit::Custom(unit) = file.unit else { panic!("custom unit lost") };
        assert_eq!(unit.factor.to_bits(), 2.5f32.to_bits());
        assert_eq!(unit.digits, 2);
        assert_eq!(unit.plural.as_deref(), Some("Own shared text unit"));
    }
    for (name, stddev) in [("own-channel-gaussian.xcf", 2.25), ("own-channel-gaussian.edit.xcf", 3.75)] {
        let file = load(&format!("canonical-channel-effect/{name}"));
        assert_eq!(file.version, 26);
        let effect = file.channels.iter().flat_map(|c| &c.effects).find(|e| e.operation == "gegl:gaussian-blur").expect("channel effect lost");
        let arg = effect.arguments.iter().find(|a| a.name == "std-dev-x").expect("Gaussian argument lost");
        assert_eq!(arg.value, ArgumentValue::Float32Bits(f32::to_bits(stddev)));
    }
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_link_transform_interpolation_records() {
    for (name, id) in [("none", 0), ("linear", 1), ("cubic", 2), ("nohalo", 3), ("lohalo", 4)] {
        for suffix in ["xcf", "edit.xcf"] {
            let f = load(&format!("transform-interpolation/link-{name}.{suffix}"));
            let d = f.layers.iter().find(|d| d.link.is_some()).expect("linked source lost");
            assert_eq!(d.transform.as_ref().unwrap().interpolation, id);
            assert!(d.raw_properties.iter().any(|p| p.tag == 48 && p.payload.len() == p.declared_bytes as usize + 4));
        }
    }
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_modern_path_geometry_and_selection() {
    for name in ["vector-basic", "vector-transform", "vector-modified"] {
        for suffix in ["xcf", "edit.xcf"] {
            let f = load(&format!("{name}.{suffix}"));
            let source =
                std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../photocraft-corpus/xcf/full/{name}.{suffix}"))).unwrap();
            for p in &f.paths {
                if let Some(at) = p.record_offset {
                    assert_eq!(p.raw_record, source[at as usize..at as usize + p.raw_record.len()]);
                }
            }
            assert!(!f.paths.is_empty(), "{name}.{suffix}");
            assert!(f.paths.iter().any(|p| p.strokes.iter().any(|s| !s.points.is_empty())));
            let layer = f.layers.iter().find(|d| d.vector.is_some()).unwrap();
            let v = layer.vector.as_ref().unwrap();
            assert_eq!(v.path_tattoo, 1201);
            assert!(v.filled && v.stroked);
            assert_eq!(v.stroke.width_bits, 3f32.to_bits());
            assert_eq!(v.stroke.miter_limit_bits, 10f32.to_bits());
            assert_eq!(v.stroke.dashes_bits, [3f32.to_bits(), 1f32.to_bits(), 2f32.to_bits(), 1f32.to_bits()]);
            let property = layer.raw_properties.iter().find(|p| p.tag == 47).unwrap();
            assert_eq!(v.source_modified, u32::from_be_bytes(property.payload[..4].try_into().unwrap()) == 1);
            assert!(v.fill.color.as_ref().unwrap().encoding.contains("float"));
            assert_eq!(v.fill.pattern.as_ref().unwrap().name, "Pine");
            let state = if suffix == "xcf" { "reopened" } else { "edited" };
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
            let reopened = std::fs::read(root.join(format!("{name}.{state}.0.native.bin"))).unwrap();
            let stored = &layer.plane.as_ref().unwrap().bytes;
            println!("{name}.{suffix}: stored={} reopened={} equal={}", stored.len(), reopened.len(), stored == &reopened);
            if v.source_modified {
                assert!(stored == &reopened, "{name}.{suffix}: modified cache differs");
            }
            if name == "vector-basic" && suffix == "xcf" {
                assert_eq!((stored.len(), reopened.len()), (15768, 14280));
                assert!(stored != &reopened);
            }
            println!(
                "{name}.{suffix}: modified={} point-kinds={:?}",
                v.source_modified,
                f.paths[0].strokes[0].points.iter().map(|p| p.kind).collect::<Vec<_>>()
            );
            if name == "vector-basic" {
                // Independent GIMP reopened_state/edited_state public path getters.
                let expected: [f32; 18] = if suffix == "xcf" {
                    [12., 14., 12., 14., 12., 14., 74., 19., 74., 19., 88., 38., 65., 61., 29., 62., 29., 62.]
                } else {
                    [15., 16., 15., 16., 15., 16., 77., 21., 77., 21., 91., 40., 68., 63., 32., 64., 32., 64.]
                };
                let path = &f.paths[0];
                assert_eq!(path.tattoo, Some(1201));
                assert!(!path.visible);
                assert!(path.strokes[0].closed);
                let xy: Vec<u32> = path.strokes[0].points.iter().flat_map(|p| p.components.iter().take(2).copied()).collect();
                assert_eq!(xy, expected.map(f32::to_bits));
            }
            println!("{name}.{suffix}: header={} paths={}", f.version, f.paths.len());
        }
    }
    for method in [0u32, 1, 2, u32::MAX] {
        let f = load(&format!("item-sets-three-types/path-{method}.xcf"));
        assert!(!f.paths.is_empty());
        for p in &f.paths {
            assert!(p.properties.iter().filter(|p| p.tag == 43).all(|_| p.selected));
            assert!(p.strokes.iter().flat_map(|s| &s.points).all(|p| p.components.len() >= 2));
        }
    }
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_curve_levels_configuration_selector_matrix() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/curve-config-matrix");
    let mut count = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("xcf") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let f = decode(&bytes, &Default::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let effect = f.layers.iter().flat_map(|d| &d.effects).find(|e| matches!(e.operation.as_str(), "gimp:levels" | "gimp:curves")).unwrap();
        assert_eq!(effect.version.as_deref(), Some("0:0"));
        let cfg = effect.arguments.iter().find(|a| a.name == "config").unwrap().configuration.as_ref().unwrap();
        let channels: Vec<&str> = match cfg.typed.as_ref().unwrap() {
            TypedConfiguration::Levels(v) => v.channels.iter().map(|c| c.channel.as_str()).collect(),
            TypedConfiguration::Curves(v) => {
                assert!(v.channels.iter().all(|c| c.curve.samples_bits.len() == c.curve.n_samples as usize));
                v.channels.iter().map(|c| c.channel.as_str()).collect()
            }
            TypedConfiguration::ColorBalance(_) | TypedConfiguration::BrightnessContrast(_) | TypedConfiguration::HueSaturation(_) => {
                panic!("expected Levels or Curves source configuration")
            }
        };
        assert_eq!(channels, ["value", "red", "green", "blue", "alpha"], "{name}");
        count += 1;
    }
    assert_eq!(count, 48);
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn exceptional_flag_authored_graph_is_parsed_without_stock_render_claim() {
    for name in ["own-exceptional-graph.xcf", "own-exceptional-graph.edit.xcf"] {
        let f = load(&format!("graph-enrollment/{name}"));
        let d = f.layers.iter().find(|d| !d.effects.is_empty()).unwrap();
        let effect = &d.effects[0];
        assert_eq!(effect.operation, "gegl:gegl");
        let graph = effect.arguments.iter().find(|a| a.name == "string").unwrap().graph.as_ref().unwrap();
        assert_eq!(graph.nodes, 2);
        let operations: Vec<_> =
            graph.entries.iter().filter_map(|e| if let GraphEntry::Operation { identity, .. } = e { Some(identity.as_str()) } else { None }).collect();
        assert_eq!(operations, ["gaussian-blur", "exposure"]);
        assert!(d.unsupported.iter().any(|s| s.contains("effects")));
    }
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_point_and_gaussian_mask_metadata_audit() {
    let paths = [
        "filter-owner-layer",
        "operations/gegl-exposure/operation-gegl-exposure",
        "operations/gegl-invert-linear/operation-gegl-invert-linear",
        "operations/gegl-invert-gamma/operation-gegl-invert-gamma",
        "operations/gegl-brightness-contrast/operation-gegl-brightness-contrast",
        "operations/gegl-levels/operation-gegl-levels",
    ];
    for path in paths {
        for suffix in ["xcf", "edit.xcf"] {
            let f = load(&format!("{path}.{suffix}"));
            let e = f.layers.iter().flat_map(|d| &d.effects).next().unwrap();
            let m = e.mask.as_ref().unwrap();
            println!(
                "{path}.{suffix}: header={} envelope={:?} mask(name={:?},tattoo={:?},visible={},offset={:?},size={:?},active={},selection={},showmasked={},edit={},shown={},flags={:?})",
                f.version,
                e.state,
                m.name,
                m.tattoo,
                m.visible,
                m.offset,
                (m.width, m.height),
                m.active,
                m.selection,
                m.show_masked,
                m.mask_editing,
                m.mask_shown,
                m.raw_properties.iter().map(|p| (p.tag, p.payload.clone())).collect::<Vec<_>>()
            );
            assert!(m.plane.as_ref().unwrap().bytes.iter().all(|b| *b == 0));
            assert!(m.mask_enabled && m.visible && m.show_masked);
            assert!(!m.mask_editing && !m.mask_shown && !m.legacy_linked && !m.active && !m.selection);
            assert_eq!(m.offset, (0, 0));
            assert_eq!(m.tattoo, Some(if suffix == "xcf" { 3 } else { 4 }));
            assert_eq!(m.opacity.to_bits(), if suffix == "xcf" { 0x3f000000 } else { 0x3f008081 });
            assert_eq!(e.state.region, Some(0));
            assert_eq!(e.state.clipping, Some(0));
        }
    }
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_text_layout_matrix_preserves_raw_configs_and_raster() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-layout-matrix");
    let mut count = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("xcf") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let (case, state) = if let Some(case) = name.strip_suffix(".edit") { (case, "fresh-edit") } else { (name, "fresh-source") };
        let bytes = std::fs::read(&path).unwrap();
        let f = decode(&bytes, &Default::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let layer = f.layers.iter().find(|l| l.text.is_some()).unwrap();
        let text = layer.text.as_ref().unwrap();
        let raw = std::fs::read(root.join(format!("{case}.{state}.0.gimp-text-layer.bin"))).unwrap();
        assert_eq!(text.raw, raw, "{name}: original parasite bytes");
        assert!(text.unknown_fields.is_empty(), "{name}: {:?}", text.unknown_fields);
        assert!(layer.plane.is_some());
        count += 1;
    }
    assert_eq!(count, 112);
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned yet)"]
fn original_text_modified_cache_and_markup_are_preserved() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    for name in ["text-basic", "text-markup", "text-outline", "text-painted", "text-transform"] {
        for edit in [false, true] {
            let suffix = if edit { ".edit" } else { "" };
            let state = if matches!(name, "text-painted" | "text-transform") {
                if edit { "gui-edited-reopened" } else { "gui-source-reopened" }
            } else if edit {
                "edited"
            } else {
                "reopened"
            };
            let f = load(&format!("{name}{suffix}.xcf"));
            let layer = f.layers.iter().find(|d| d.text.is_some()).unwrap();
            let text = layer.text.as_ref().unwrap();
            assert_eq!(text.raw, std::fs::read(root.join(format!("{name}.{state}.0.gimp-text-layer.bin"))).unwrap(), "{name}{suffix}");
            assert_eq!(layer.plane.as_ref().unwrap().bytes, std::fs::read(root.join(format!("{name}.{state}.0.native.bin"))).unwrap(), "{name}{suffix}");
            if name == "text-painted" && !edit {
                assert!(layer.text_source_modified());
            }
            for run in &text.runs {
                assert!(text.content.is_char_boundary(run.start as usize));
                assert!(text.content.is_char_boundary(run.end as usize));
            }
        }
    }
}

#[test]
#[ignore = "requires reviewed original full-XCF corpus"]
fn original_free256_curve_envelope_audit() {
    for suffix in ["xcf", "edit.xcf"] {
        let f = load(&format!("curve-config-matrix/free256-rgb-u8.{suffix}"));
        let e = &f.layers[0].effects[0];
        assert_eq!(f.version, 22);
        assert_eq!(e.operation, "gimp:curves");
        assert_eq!(e.version.as_deref(), Some("0:0"));
        assert_eq!((e.state.composite_mode, e.state.composite_space, e.state.blend_space), (Some(-1080065455), Some(-1080065455), Some(-1080065455)));
        println!(
            "{suffix}: epoch{} operation{} version{:?} state{:?} tags{:?}",
            f.version,
            e.operation,
            e.version,
            e.state,
            e.properties.iter().map(|p| p.tag).collect::<Vec<_>>()
        );
        let mask = e.mask.as_ref().unwrap();
        assert_eq!((mask.width, mask.height, mask.offset), (12, 9, (0, 0)));
        assert!(mask.mask_enabled && mask.visible && !mask.mask_editing && !mask.mask_shown && !mask.legacy_linked);
        println!(
            "mask{}x{} tattoo{:?} visible{} offset{:?} enabled{} editing{} shown{} linked{} zero{}",
            mask.width,
            mask.height,
            mask.tattoo,
            mask.visible,
            mask.offset,
            mask.mask_enabled,
            mask.mask_editing,
            mask.mask_shown,
            mask.legacy_linked,
            mask.plane.as_ref().unwrap().bytes.iter().all(|b| *b == 0)
        );
        for a in &e.arguments {
            if let Some(EffectConfiguration { typed: Some(TypedConfiguration::Curves(c)), .. }) = &a.configuration {
                println!(
                    "trc{:?} channels{:?}",
                    c.trc,
                    c.channels
                        .iter()
                        .map(|v| (
                            &v.channel,
                            &v.curve.curve_type,
                            v.curve.n_samples,
                            v.curve.samples_bits.len(),
                            v.curve.points_bits.len(),
                            v.curve.point_types.len()
                        ))
                        .collect::<Vec<_>>()
                );
                assert_eq!(c.channels.len(), 5);
                assert_eq!(c.trc.as_deref(), Some("non-linear"));
                assert_eq!(c.channels.iter().map(|v| v.channel.as_str()).collect::<Vec<_>>(), ["value", "red", "green", "blue", "alpha"]);
                assert!(c.channels.iter().all(|v| v.curve.curve_type == "free" && v.curve.n_samples == 256 && v.curve.samples_bits.len() == 256));
            }
        }
    }
}

#[test]
#[ignore = "requires reviewed original full-XCF corpus"]
fn original_smooth_curve_envelope_audit() {
    for suffix in ["xcf", "edit.xcf"] {
        let f = load(&format!("curve-config-matrix/smooth-corners-rgb-u8.{suffix}"));
        let e = &f.layers[0].effects[0];
        println!("{suffix}: header{} op{} version{:?} envelope{:?}", f.version, e.operation, e.version, e.state);
        let mask = e.mask.as_ref().unwrap();
        println!(
            "mask{}x{} offset{:?} tattoo{:?} visible{} enabled{} shown{} editing{} linked{} zero{}",
            mask.width,
            mask.height,
            mask.offset,
            mask.tattoo,
            mask.visible,
            mask.mask_enabled,
            mask.mask_shown,
            mask.mask_editing,
            mask.legacy_linked,
            mask.plane.as_ref().unwrap().bytes.iter().all(|b| *b == 0)
        );
        let c = e.arguments.iter().find_map(|a| a.configuration.as_ref().and_then(|c| c.typed.as_ref())).unwrap();
        let TypedConfiguration::Curves(c) = c else { panic!("curves") };
        println!(
            "TRC{:?} channels{:?}",
            c.trc,
            c.channels
                .iter()
                .map(|v| (&v.channel, &v.curve.curve_type, v.curve.n_samples, v.curve.samples_bits.len(), v.curve.points_bits.len(), &v.curve.point_types))
                .collect::<Vec<_>>()
        );
        assert_eq!(c.channels.len(), 5);
        assert!(c.channels.iter().all(|v| v.curve.curve_type == "smooth" && v.curve.n_samples == 256 && v.curve.samples_bits.len() == 256));
    }
}

#[test]
#[ignore = "requires reviewed independently authored structured Levels matrix"]
fn original_structured_levels_envelopes() {
    for trc in [0, 1] {
        for clamps in ["00", "01", "10", "11"] {
            for suffix in ["xcf", "edit.xcf"] {
                let name = format!("curve-config-matrix/levels-trc{trc}-clamps{clamps}.{suffix}");
                let file = load(&name);
                let effect = &file.layers[0].effects[0];
                let mask = effect.mask.as_ref().unwrap();
                println!(
                    "{name}: header{} operation{} version{:?} envelope{:?} mask{}x{} offset{:?} tattoo{:?} enabled{} shown{} editing{} linked{} zero{}",
                    file.version,
                    effect.operation,
                    effect.version,
                    effect.state,
                    mask.width,
                    mask.height,
                    mask.offset,
                    mask.tattoo,
                    mask.mask_enabled,
                    mask.mask_shown,
                    mask.mask_editing,
                    mask.legacy_linked,
                    mask.plane.as_ref().unwrap().bytes.iter().all(|b| *b == 0)
                );
                assert_eq!(file.version, 22);
                assert_eq!(effect.operation, "gimp:levels");
                assert_eq!(effect.version.as_deref(), Some("0:0"));
                println!("precision{} sample{:?} mask plane{} bytes", file.precision, file.sample, mask.plane.as_ref().unwrap().bytes.len());
                assert_eq!(effect.arguments.iter().map(|argument| argument.name.as_str()).collect::<Vec<_>>(), ["trc", "config"]);
                assert!(matches!(effect.arguments[0].value, ArgumentValue::Enum(value) if value == trc));
                let Some(TypedConfiguration::Levels(config)) = effect.arguments[1].configuration.as_ref().and_then(|config| config.typed.as_ref()) else {
                    panic!("typed Levels configuration");
                };
                assert_eq!(config.time_bits, Some(0));
                assert_eq!(config.trc.as_deref(), Some(if trc == 0 { "linear" } else { "non-linear" }));
                assert_eq!(config.clamp_input, Some(clamps.starts_with('1')));
                assert_eq!(config.clamp_output, Some(clamps.ends_with('1')));
                assert_eq!(config.channels.iter().map(|channel| channel.channel.as_str()).collect::<Vec<_>>(), ["value", "red", "green", "blue", "alpha"]);
                assert_eq!(mask.plane.as_ref().unwrap().sample, Sample::F32);
                assert_eq!(mask.plane.as_ref().unwrap().channels, 1);
            }
        }
    }
}

#[test]
#[ignore = "requires reviewed original full-XCF composition corpus"]
fn original_gray_normal_multiply_mask_envelopes() {
    for mode in [28, 30] {
        for suffix in ["xcf", "edit.xcf"] {
            let file = load(&format!("composition-matrix/gray-mode-{mode}.{suffix}"));
            println!("gray-mode-{mode}.{suffix}: header{} precision{} sample{:?}", file.version, file.precision, file.sample);
            for layer in &file.layers {
                println!(
                    "layer {:?} mode{} bounds{}x{} offset{:?} policy{:?}/{:?}/{:?}",
                    layer.name, layer.mode, layer.width, layer.height, layer.offset, layer.composite_mode, layer.composite_space, layer.blend_space
                );
                if let Some(mask) = &layer.mask {
                    println!(
                        "mask {}x{} offset{:?} visible{} enabled{} shown{} editing{} linked{} sample{:?}",
                        mask.width,
                        mask.height,
                        mask.offset,
                        mask.visible,
                        mask.mask_enabled,
                        mask.mask_shown,
                        mask.mask_editing,
                        mask.legacy_linked,
                        mask.plane.as_ref().map(|plane| plane.sample)
                    );
                }
            }
            assert_eq!(file.sample, Sample::F32);
        }
    }
}

#[test]
#[ignore = "requires reviewed original full-XCF composition corpus"]
fn original_group_mask_envelopes() {
    for mode in [28, 61] {
        for suffix in ["xcf", "edit.xcf"] {
            let file = load(&format!("composition-matrix/group-{mode}-none-mask.{suffix}"));
            println!("group-{mode}.{suffix}: header{} precision{} sample{:?}", file.version, file.precision, file.sample);
            let mut masks = 0;
            for layer in &file.layers {
                println!(
                    "layer {:?} group{} mode{} opacity{} bounds{}x{} offset{:?} policy{:?}/{:?}/{:?}",
                    layer.name,
                    layer.group,
                    layer.mode,
                    layer.opacity,
                    layer.width,
                    layer.height,
                    layer.offset,
                    layer.composite_mode,
                    layer.composite_space,
                    layer.blend_space
                );
                if let Some(mask) = &layer.mask {
                    masks += 1;
                    println!(
                        "mask {}x{} offset{:?} visible{} enabled{} shown{} editing{} linked{} sample{:?}",
                        mask.width,
                        mask.height,
                        mask.offset,
                        mask.visible,
                        mask.mask_enabled,
                        mask.mask_shown,
                        mask.mask_editing,
                        mask.legacy_linked,
                        mask.plane.as_ref().map(|plane| plane.sample)
                    );
                    assert_eq!((mask.width, mask.height), if suffix == "xcf" { (12, 9) } else { (12, 11) });
                }
            }
            assert_eq!(masks, 1);
        }
    }
}

#[test]
#[ignore = "requires reviewed source-modified original corpus"]
fn original_modified_source_cache_envelopes() {
    for name in ["text-painted", "vector-modified"] {
        for suffix in ["xcf", "edit.xcf"] {
            let file = load(&format!("{name}.{suffix}"));
            println!(
                "{name}.{suffix}: header{} precision{} sample{:?} encoding{:?} model{:?} ICC{}",
                file.version,
                file.precision,
                file.sample,
                file.encoding,
                file.model,
                file.icc.len()
            );
            let layer = file.layers.iter().find(|d| d.text.is_some() || d.vector.is_some()).unwrap();
            println!(
                "source {:?} textflags{} vectorModified{:?} group{} effects{} mask{} transform{} link{} model{:?} alpha{} bounds{}x{} offset{:?} mode{} policy{}/{}/{} opacity{}",
                layer.name,
                layer.text_flags,
                layer.vector.as_ref().map(|v| v.source_modified),
                layer.group,
                layer.effects.len(),
                layer.mask.is_some(),
                layer.transform.is_some(),
                layer.link.is_some(),
                layer.model,
                layer.alpha,
                layer.width,
                layer.height,
                layer.offset,
                layer.mode,
                layer.composite_mode,
                layer.composite_space,
                layer.blend_space,
                layer.opacity
            );
            if name == "text-painted" {
                assert_eq!(layer.text_source_modified(), suffix == "xcf");
            }
        }
    }
}
#[test]
#[ignore = "requires reviewed composition/filter original corpus"]
fn original_requested_group_gaussian_and_explicit_spaces_envelopes() {
    for name in [
        "group-28-group-mask",
        "group-61-group-mask",
        "blend-space-rgb-non-linear",
        "blend-space-rgb-perceptual",
        "composite-space-rgb-non-linear",
        "composite-space-rgb-perceptual",
        "composite-space-lab",
    ] {
        for suffix in ["xcf", "edit.xcf"] {
            let file = load(&format!("composition-matrix/{name}.{suffix}"));
            println!("{name}.{suffix}: header{} precision{} sample{:?}", file.version, file.precision, file.sample);
            for layer in &file.layers {
                println!(
                    "OWNER mask flags enabled{} editing{} shown{} raw{:?}",
                    layer.mask_enabled,
                    layer.mask_editing,
                    layer.mask_shown,
                    layer.raw_properties.iter().filter(|p| matches!(p.tag, 11..=13)).map(|p| (p.tag, p.payload.clone())).collect::<Vec<_>>()
                );
                println!(
                    "layer {:?} group{} mode{} opacity{} bounds{}x{} offset{:?} policy{}/{}/{} masks{} effects{}",
                    layer.name,
                    layer.group,
                    layer.mode,
                    layer.opacity,
                    layer.width,
                    layer.height,
                    layer.offset,
                    layer.composite_mode,
                    layer.composite_space,
                    layer.blend_space,
                    layer.mask.is_some(),
                    layer.effects.len()
                );
                for effect in &layer.effects {
                    println!("effect {} version{:?} state{:?}", effect.operation, effect.version, effect.state);
                    for argument in &effect.arguments {
                        println!("arg{}={:?}", argument.name, argument.value);
                    }
                    if let Some(mask) = &effect.mask {
                        println!(
                            "effectmask{}x{} offset{:?} enabled{} shown{} edit{} visible{} linked{} zero{} sample{:?}",
                            mask.width,
                            mask.height,
                            mask.offset,
                            mask.mask_enabled,
                            mask.mask_shown,
                            mask.mask_editing,
                            mask.visible,
                            mask.legacy_linked,
                            mask.plane.as_ref().is_some_and(|p| p.bytes.iter().all(|b| *b == 0)),
                            mask.plane.as_ref().map(|p| p.sample)
                        );
                    }
                }
            }
        }
    }
}
