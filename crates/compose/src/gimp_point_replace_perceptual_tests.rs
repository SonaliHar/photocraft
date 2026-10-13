// Independent Perceptual baked-native controls; live projection is checked by IO.
use super::*;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::FilterId,
    xcf_filters::{FilterArgument, OperationIdentity},
};
#[test]
#[ignore = "local unpublished Perceptual independent point originals"]
fn seven_point_families_match_perceptual_native() {
    point_replace_perceptual_native_packet("point-replace-perceptual-evidence", "e6c2dcd48b3aff0ecddb2dacc134a897d83983c38eeacc79898d7a50818576c8", 198);
}
fn point_replace_perceptual_native_packet(packet: &str, manifest_hash: &str, expected_count: usize) {
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
        let precision = row["precision"].as_str().unwrap();
        let stored = SampleEncoding::Perceptual;
        let half = precision.contains("HALF");
        let width = if precision.starts_with("U8_") {
            1
        } else if half || precision.starts_with("U16_") {
            2
        } else if precision.starts_with("DOUBLE_") {
            8
        } else {
            4
        };
        let channels = if gray { 2 } else { 4 };
        let maximum = if precision.starts_with("U8_") {
            Some(255.0)
        } else if precision.starts_with("U16_") {
            Some(65535.0)
        } else if precision.starts_with("U32_") {
            Some(4294967295.0)
        } else {
            None
        };
        let decode = |bytes: &[u8]| {
            if half {
                f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(bytes.try_into().unwrap()))))
            } else if let Some(maximum) = maximum {
                let value = match width {
                    1 => u32::from(bytes[0]),
                    2 => u32::from(u16::from_le_bytes(bytes.try_into().unwrap())),
                    _ => u32::from_le_bytes(bytes.try_into().unwrap()),
                };
                f64::from(value) / maximum
            } else if width == 8 {
                f64::from_le_bytes(bytes.try_into().unwrap())
            } else {
                f64::from(f32::from_le_bytes(bytes.try_into().unwrap()))
            }
        };
        let mut legacy = node.clone();
        legacy.composition.as_mut().unwrap().compatibility_epoch = epoch.into();
        assert!(
            matches!(PointStack::from_nodes(std::slice::from_ref(&legacy), profile, stored, 22), Err(CompositionError::Unsupported(reason)) if reason.contains("precision"))
        );
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
            if precision.starts_with("U32_") {
                let current = point.apply_current(rgba, true, &mut |_| Ok(())).unwrap();
                assert_eq!(current.map(f64::to_bits), actual.map(f64::to_bits));
            }
            assert_eq!(actual[3].to_bits(), rgba[3].to_bits());
            for channel in 0..channels {
                let mut value = actual[if gray && channel == 1 { 3 } else { channel }];
                // This is only the independently baked source storage boundary.
                // The current PhotoCraft Half-import document has F32 storage.
                if half {
                    let bits = photocraft_color::try_round_f32_to_f16_bits(value as f32).unwrap();
                    value = f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(bits)));
                } else if let Some(maximum) = maximum {
                    value = (value.clamp(0., 1.) * maximum).round() / maximum;
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

#[test]
fn perceptual_requires_closed_singleton_replace_compatibility() {
    let epoch = EXPOSURE_BLACK_PRECISION_EPOCH;
    let mut node = FilterNode {
        id: FilterId(1),
        operation: OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: epoch.into() },
        arguments: vec![
            FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5_f32.to_bits())) },
            FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0_f32.to_bits())) },
        ],
        source_display_name: None,
        source_icon: None,
        enabled: true,
        opacity_f64_bits: 1_f64.to_bits(),
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
            stored_operator: 7,
            resolved: None,
            pass_through: false,
        }),
    };
    let profile = photocraft_cms::Builtin::Srgb.profile();
    assert!(PointStack::from_nodes(std::slice::from_ref(&node), profile, SampleEncoding::Perceptual, 22).is_ok());
    assert!(PointStack::from_nodes(&[node.clone(), node.clone()], profile, SampleEncoding::Perceptual, 22).is_err());
    node.composition.as_mut().unwrap().compatibility_epoch = epoch.into();
    assert!(PointStack::from_nodes(std::slice::from_ref(&node), profile, SampleEncoding::Perceptual, 22).is_err());
    node.composition.as_mut().unwrap().compatibility_epoch = POINT_REPLACE_COMPATIBILITY_EPOCH.into();
    node.source_region = Some(1);
    assert!(PointStack::from_nodes(std::slice::from_ref(&node), profile, SampleEncoding::Perceptual, 22).is_err());
}
