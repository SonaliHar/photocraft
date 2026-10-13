use super::*;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::FilterId,
    xcf_filters::{FilterArgument, OperationIdentity},
};
#[test]
#[ignore = "local unpublished independently authored Alien Map HSL precision states"]
fn generic_replace_preserves_unresolved_raw_fields_for_witnessed_hsl() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alien-map-hsl-precision-evidence");
    let read = |name: &str| std::fs::read(root.join(name)).unwrap();
    let binding = read("batch-manifest.json");
    let hex = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hex(&binding), "e36c3c0f6790a8eccfac020032a5809897ae979a592a01083ab09efca7c540eb");
    let binding: serde_json::Value = serde_json::from_slice(&binding).unwrap();
    for a in binding["artifacts"].as_array().unwrap() {
        let data = read(a["path"].as_str().unwrap());
        assert_eq!(hex(&data), a["sha256"].as_str().unwrap());
    }
    let proof: serde_json::Value = serde_json::from_slice(&read("fresh-verification.json")).unwrap();
    let mut count = 0;
    for row in proof["checks"].as_array().unwrap() {
        let state = &row["fresh"];
        let layer = &state["layers"][0];
        let precision = state["precision"].as_str().unwrap();
        let width = if precision.contains("U16") {
            2
        } else if precision.contains("FLOAT") {
            4
        } else {
            8
        };
        let stored = if precision.ends_with("_NON_LINEAR") { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let cfg = &layer["filters"][0]["config"];
        let epoch = "xcf-22-stock-alien-map-hsl-precision-v1";
        let node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:alien-map".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
            arguments: cfg
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, v)| FilterArgument {
                    name: name.clone(),
                    value: if name == "color-model" {
                        FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 1 }
                    } else if name.ends_with("-keep") {
                        FilterValue::Bool(v.as_bool().unwrap())
                    } else {
                        FilterValue::Number(ExactNumber::Float32Bits((v.as_f64().unwrap() as f32).to_bits()))
                    },
                })
                .collect(),
            source_display_name: None,
            source_icon: None,
            enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: "xcf-22-stock-point-replace-fields-v1".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: i32::MIN,
                stored_composite_space: i32::MAX,
                stored_operator: 0x12345678,
                resolved: None,
                pass_through: false,
            }),
        };
        let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
        let source = read(layer["native"]["file"].as_str().unwrap());
        let expected = read(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap());
        let projection = read(state["projection"]["file"].as_str().unwrap());
        assert_eq!(source.len(), 48 * 4 * width);
        assert_eq!(expected.len(), source.len());
        let decode = |s: &[u8]| match width {
            2 => f64::from(u16::from_le_bytes(s.try_into().unwrap())) / 65535.,
            4 => f64::from(f32::from_le_bytes(s.try_into().unwrap())),
            _ => f64::from_le_bytes(s.try_into().unwrap()),
        };
        let display = WorkingSpace::new(profile, stored, GimpSpace::ProfileRgb).unwrap();
        let mut worst = 0.0_f64;
        let mut live_worst = 0.0_f64;
        for (index, (input, reference)) in source.chunks_exact(4 * width).zip(expected.chunks_exact(4 * width)).enumerate() {
            let input: [f64; 4] = std::array::from_fn(|i| decode(&input[i * width..i * width + width]));
            let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
            assert_eq!(actual[3].to_bits(), input[3].to_bits());
            let q = |v: f64| {
                if width == 2 {
                    (v.clamp(0., 1.) * 65535.).round() / 65535.
                } else if width == 4 {
                    f64::from(v as f32)
                } else {
                    v
                }
            };
            let encoded = display.convert([q(actual[0]), q(actual[1]), q(actual[2])], true).unwrap();
            for c in 0..3 {
                worst = worst.max((q(actual[c]) - decode(&reference[c * width..c * width + width])).abs());
                let at = (index * 4 + c) * 8;
                let value = f64::from_le_bytes(projection[at..at + 8].try_into().unwrap());
                live_worst = live_worst.max((encoded[c] - value).abs() * input[3]);
            }
        }
        println!("{} Alien Map native {}/255 live {}/255", row["id"], worst * 255., live_worst * 255.);
        assert!(worst <= 2. / 255.);
        assert!(live_worst <= 2. / 255.);
        let mut invalid = node.clone();
        invalid.operation.schema_epoch = "unknown".into();
        assert!(PointStack::from_nodes(std::slice::from_ref(&invalid), profile, stored, 22).is_err());
        let mut invalid = node.clone();
        invalid.arguments.iter_mut().find(|a| a.name == "cpn-1-keep").unwrap().value = FilterValue::Number(ExactNumber::Float32Bits(1.0_f32.to_bits()));
        assert!(PointStack::from_nodes(std::slice::from_ref(&invalid), profile, stored, 22).is_err());
        let mut invalid = node.clone();
        invalid.arguments.iter_mut().find(|a| a.name == "cpn-1-frequency").unwrap().value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()));
        assert!(PointStack::from_nodes(std::slice::from_ref(&invalid), profile, stored, 22).is_err());
        assert!(PointStack::from_nodes(std::slice::from_ref(&node), photocraft_cms::Builtin::SGray.profile(), stored, 22).is_err());
        count += 1;
    }
    assert_eq!(count, 24);
}

#[test]
fn generic_replace_rejects_unqualified_envelopes_even_disabled() {
    let mut args = vec![FilterArgument { name: "color-model".into(), value: FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 1 } }];
    for c in 1..=3 {
        for (suffix, value) in [("frequency", 1.0_f32), ("phaseshift", 0.0_f32)] {
            args.push(FilterArgument { name: format!("cpn-{c}-{suffix}"), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) });
        }
        args.push(FilterArgument { name: format!("cpn-{c}-keep"), value: FilterValue::Bool(false) });
    }
    let valid = FilterNode {
        id: FilterId(1),
        operation: OperationIdentity { name: "gegl:alien-map".into(), stored_version: "0:0".into(), schema_epoch: ALIEN_MAP_HSL_PRECISION_EPOCH.into() },
        arguments: args,
        source_display_name: None,
        source_icon: None,
        enabled: false,
        opacity_f64_bits: 1.0_f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(0),
        mask: None,
        auxiliary: vec![],
        composition: Some(GimpComposition {
            compatibility_epoch: POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: i32::MIN,
            stored_composite_space: i32::MAX,
            stored_operator: 0x12345678,
            resolved: None,
            pass_through: false,
        }),
    };
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let encoding = SampleEncoding::NonlinearProfile;
    assert!(PointStack::from_nodes(std::slice::from_ref(&valid), profile, encoding, 22).is_ok());
    for mutation in 0..8 {
        let mut bad = valid.clone();
        match mutation {
            0 => bad.composition.as_mut().unwrap().source_mode = 28,
            1 => bad.opacity_f64_bits = 0.5_f64.to_bits(),
            2 => bad.source_clipping = None,
            3 => bad.source_region = Some(1),
            4 => bad.operation.schema_epoch = "unknown".into(),
            5 => bad.operation.stored_version = "unobserved".into(),
            6 => {
                bad.arguments.pop();
            }
            _ => bad.arguments.push(bad.arguments[0].clone()),
        };
        assert!(PointStack::from_nodes(std::slice::from_ref(&bad), profile, encoding, 22).is_err(), "mutation{mutation}");
        let staged = photocraft_doc::xcf_filters::FilterStack {
            owner: photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(10)),
            nodes: vec![bad],
            revision: 0,
        };
        assert!(
            crate::gimp_neighborhood::OrderedStack::new(&staged, profile, encoding, 22, photocraft_geom::Rect::from_xywh(-13, 11, 257, 129), [false; 3])
                .is_err(),
            "disabled ordered mutation{mutation}"
        );
    }
    let staged = photocraft_doc::xcf_filters::FilterStack {
        owner: photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(10)),
        nodes: vec![valid.clone()],
        revision: 0,
    };
    let ordered =
        crate::gimp_neighborhood::OrderedStack::new(&staged, profile, encoding, 22, photocraft_geom::Rect::from_xywh(-13, 11, 257, 129), [false; 3]).unwrap();
    let input = [-0.0f64, 0.25, 0.75, 0.5];
    let output = ordered.point(input, &mut |_| Ok(())).unwrap();
    for (a, b) in input.iter().zip(output) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
    assert!(PointStack::from_nodes(&[valid.clone(), valid], profile, encoding, 22).is_err());
}

#[test]
#[ignore = "local unpublished independent point Replace field controls"]
fn seven_point_families_preserve_raw_field_controls_and_match_native() {
    use sha2::Digest;
    use std::io::Read;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-field-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    let hex = |raw: &[u8]| sha2::Sha256::digest(raw).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hex(&manifest), "2fd3ef89fd856cf892be0917c2b8ab4b051f8a8dba3017d75edabc0189119d04");
    let bindings: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    for artifact in bindings["artifacts"].as_array().unwrap() {
        let raw = std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap();
        assert_eq!(hex(&raw), artifact["sha256"].as_str().unwrap());
        assert_eq!(raw.len() as u64, artifact["bytes"].as_u64().unwrap());
    }
    let declarations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("first/declared-inputs.json")).unwrap()).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("first/stock-reader-report.json")).unwrap()).unwrap();
    let read_blob = |record: &serde_json::Value| {
        let raw = std::fs::read(root.join("first").join(record["path"].as_str().unwrap())).unwrap();
        assert_eq!(hex(&raw), record["sha256"].as_str().unwrap());
        let size = record["decoded_bytes"].as_u64().unwrap();
        assert!(size <= 48 * 32);
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(raw.as_slice()).take(size + 1).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded.len() as u64, size);
        assert_eq!(hex(&decoded), record["decoded_sha256"].as_str().unwrap());
        decoded
    };
    let mut worst = 0.0_f64;
    let mut count = 0;
    for (row, declared) in proof["rows"].as_array().unwrap().iter().zip(declarations["rows"].as_array().unwrap()) {
        assert_eq!(row["id"], declared["id"]);
        let state = &row["state"];
        let filter = &state["filters"][0];
        let name = filter["operation"].as_str().unwrap();
        let config = filter["config"].as_object().unwrap();
        let epoch = match name {
            "gegl:brightness-contrast" => BRIGHTNESS_CONTRAST_PRECISION_EPOCH,
            "gegl:levels" => GEGL_LEVELS_PRECISION_EPOCH,
            "gegl:exposure" => EXPOSURE_BLACK_PRECISION_EPOCH,
            "gegl:color-temperature" => COLOR_TEMPERATURE_PRECISION_EPOCH,
            "gegl:sepia" => SEPIA_PRECISION_EPOCH,
            "gegl:alien-map" => {
                if config["color-model"] == "hsl" {
                    ALIEN_MAP_HSL_PRECISION_EPOCH
                } else {
                    ALIEN_MAP_PRECISION_EPOCH
                }
            }
            _ => panic!("unqualified proof operation"),
        };
        let arguments = config
            .iter()
            .map(|(name, value)| FilterArgument {
                name: name.clone(),
                value: if let Some(value) = value.as_bool() {
                    FilterValue::Bool(value)
                } else if name == "color-model" {
                    FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: if value == "hsl" { 1 } else { 0 } }
                } else {
                    FilterValue::Number(ExactNumber::Float32Bits((value.as_f64().unwrap() as f32).to_bits()))
                },
            })
            .collect();
        let field = |tag: u64| {
            let value = declared["mutations"].as_array().unwrap().iter().find(|v| v["tag"] == tag).unwrap()["after"].as_str().unwrap();
            i32::from_be_bytes(std::array::from_fn(|i| u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).unwrap()))
        };
        let node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: name.into(), stored_version: declared["operation"][3].as_str().unwrap().into(), schema_epoch: epoch.into() },
            arguments,
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: field(37),
                stored_composite_space: field(36),
                stored_operator: field(35),
                resolved: None,
                pass_through: false,
            }),
        };
        let before = node.clone();
        let gray = state["model"] == "gray";
        let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let precision = state["precision"].as_str().unwrap();
        let stored = if precision.ends_with("non-linear") { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
        let width = if precision.starts_with("u16") {
            2
        } else if precision.starts_with("float") {
            4
        } else {
            8
        };
        let channels = if gray { 2 } else { 4 };
        let decode = |bytes: &[u8]| match width {
            2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.,
            4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
            _ => f64::from_le_bytes(bytes.try_into().unwrap()),
        };
        let point = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
        let source = read_blob(&state["source_native"]);
        let reference = read_blob(&state["baked_native"]);
        assert_eq!(source.len(), 48 * channels * width);
        assert_eq!(reference.len(), source.len());
        for (input, expected) in source.chunks_exact(channels * width).zip(reference.chunks_exact(channels * width)) {
            let y = decode(&input[..width]);
            let rgba = if gray { [y, y, y, decode(&input[width..])] } else { std::array::from_fn(|i| decode(&input[i * width..i * width + width])) };
            let actual = point.apply(rgba, &mut |_| Ok(())).unwrap();
            assert_eq!(actual[3].to_bits(), rgba[3].to_bits());
            for channel in 0..channels {
                let mut value = actual[if gray && channel == 1 { 3 } else { channel }];
                if width == 2 {
                    value = (value.clamp(0., 1.) * 65535.).round() / 65535.;
                } else if width == 4 {
                    value = f64::from(value as f32);
                }
                let expected = decode(&expected[channel * width..channel * width + width]);
                worst = worst.max((value - expected).abs());
                assert!((value - expected).abs() <= 2. / 255., "{} channel{channel}: {value} != {expected}", row["id"]);
            }
        }
        assert_eq!(node, before);
        count += 1;
    }
    assert_eq!(count, 264);
    println!("Seven-family264 raw field controls native max {}/255", worst * 255.);
}

#[test]
#[ignore = "local unpublished independently authored point-filter extent and offset controls"]
fn seven_point_families_match_independent_variable_extent_native() {
    point_replace_native_packet("point-replace-geometry-evidence", "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72", 129);
}
#[test]
#[ignore = "local unpublished independently authored U8 Linear and Nonlinear point-filter controls"]
fn seven_point_families_match_independent_u8_native() {
    point_replace_native_packet("point-replace-u8-evidence", "e472b64e19df16ce7fa5999785f9859c4a553c233bb341c47d6aa0c98d1816b5", 66);
}
fn point_replace_native_packet(packet: &str, manifest_hash: &str, expected_count: usize) {
    use sha2::Digest;
    use std::io::Read;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(packet);
    let read = |name: &str| std::fs::read(root.join(name)).unwrap();
    let hex = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hex(&read("batch-manifest.json")), manifest_hash);
    let manifest: serde_json::Value = serde_json::from_slice(&read("batch-manifest.json")).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hex(&read(artifact["path"].as_str().unwrap())), artifact["sha256"].as_str().unwrap());
    }
    let proof: serde_json::Value = serde_json::from_slice(&read("observations.json")).unwrap();
    let read_blob = |record: &serde_json::Value| {
        let encoded = read(record["file"].as_str().unwrap());
        assert_eq!(hex(&encoded), record["sha256"].as_str().unwrap());
        let length = record["decoded_bytes"].as_u64().unwrap();
        assert!(length <= 320 * 320 * 32);
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(length + 1).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded.len() as u64, length);
        assert_eq!(hex(&decoded), record["decoded_sha256"].as_str().unwrap());
        decoded
    };
    let mut worst = 0.0_f64;
    let mut count = 0;
    for row in proof["records"].as_array().unwrap() {
        let state = &row["reopened"];
        let filter = &state["layers"][0]["filters"][0];
        let name = filter["operation"].as_str().unwrap();
        let config = filter["config"].as_object().unwrap();
        let epoch = match name {
            "gegl:brightness-contrast" => BRIGHTNESS_CONTRAST_PRECISION_EPOCH,
            "gegl:levels" => GEGL_LEVELS_PRECISION_EPOCH,
            "gegl:exposure" => EXPOSURE_BLACK_PRECISION_EPOCH,
            "gegl:color-temperature" => COLOR_TEMPERATURE_PRECISION_EPOCH,
            "gegl:sepia" => SEPIA_PRECISION_EPOCH,
            "gegl:alien-map" => {
                if config["color-model"] == "hsl" {
                    ALIEN_MAP_HSL_PRECISION_EPOCH
                } else {
                    ALIEN_MAP_PRECISION_EPOCH
                }
            }
            _ => panic!("unqualified proof operation"),
        };
        let arguments = config
            .iter()
            .map(|(name, value)| FilterArgument {
                name: name.clone(),
                value: if let Some(value) = value.as_bool() {
                    FilterValue::Bool(value)
                } else if name == "color-model" {
                    FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: if value == "hsl" { 1 } else { 0 } }
                } else {
                    FilterValue::Number(ExactNumber::Float32Bits((value.as_f64().unwrap() as f32).to_bits()))
                },
            })
            .collect();
        let node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: name.into(),
                stored_version: if name == "gegl:exposure" { "1:0".into() } else { "0:0".into() },
                schema_epoch: epoch.into(),
            },
            arguments,
            source_display_name: None,
            source_icon: None,
            enabled: filter["visible"].as_bool().unwrap(),
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: i32::MIN,
                stored_composite_space: i32::MAX,
                stored_operator: 0x12345678,
                resolved: None,
                pass_through: false,
            }),
        };
        let before = node.clone();
        let gray = row["model"] == "GRAY";
        let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let precision = state["precision"].as_str().unwrap();
        let stored = if precision.ends_with("_NON_LINEAR") { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
        let width = if precision.contains("U8") {
            1
        } else if precision.contains("U16") {
            2
        } else if precision.contains("FLOAT") {
            4
        } else {
            8
        };
        let channels = if gray { 2 } else { 4 };
        let decode = |bytes: &[u8]| match width {
            1 => f64::from(bytes[0]) / 255.,
            2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.,
            4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
            _ => f64::from_le_bytes(bytes.try_into().unwrap()),
        };
        let point = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
        let source = read_blob(&state["layers"][0]["native"]);
        let reference = read_blob(&row["baked"]["layers"][0]["native"]);
        let pixels = row["drawable"][0].as_u64().unwrap() * row["drawable"][1].as_u64().unwrap();
        assert_eq!(source.len(), pixels as usize * channels * width);
        assert_eq!(reference.len(), source.len());
        for (input, expected) in source.chunks_exact(channels * width).zip(reference.chunks_exact(channels * width)) {
            let y = decode(&input[..width]);
            let rgba = if gray { [y, y, y, decode(&input[width..])] } else { std::array::from_fn(|i| decode(&input[i * width..i * width + width])) };
            let actual = point.apply(rgba, &mut |_| Ok(())).unwrap();
            assert_eq!(actual[3].to_bits(), rgba[3].to_bits());
            for channel in 0..channels {
                let mut value = actual[if gray && channel == 1 { 3 } else { channel }];
                if width == 1 {
                    value = (value.clamp(0., 1.) * 255.).round() / 255.;
                } else if width == 2 {
                    value = (value.clamp(0., 1.) * 65535.).round() / 65535.;
                } else if width == 4 {
                    value = f64::from(value as f32);
                }
                let expected = decode(&expected[channel * width..channel * width + width]);
                worst = worst.max((value - expected).abs());
                assert!((value - expected).abs() <= 2. / 255., "{} channel{channel}: {value} != {expected}", row["id"]);
            }
        }
        assert_eq!(node, before);
        count += 1;
    }
    assert_eq!(count, expected_count);
    println!("Seven-family{count} {packet} controls native max {}/255", worst * 255.);
}
