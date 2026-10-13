use super::*;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::FilterId,
    xcf_filters::{FilterArgument, OperationIdentity},
};
#[test]
#[ignore = "local unpublished independent RGB Yuv live and baked originals"]
fn independent_rgb_yuv_u16_f32_baked_and_live() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("saturation-precision-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(
        sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c"
    );
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap()).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            artifact["sha256"].as_str().unwrap()
        );
    }
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let live_root = root.parent().unwrap().join("saturation-yuv-live-evidence");
    let live_manifest = std::fs::read(live_root.join("batch-manifest.json")).unwrap();
    assert_eq!(
        sha2::Sha256::digest(&live_manifest).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "fda3e70af6b490509ea0b02569b56f7489ac8498b728ca9254393c59ef421960"
    );
    let live_manifest: serde_json::Value = serde_json::from_slice(&live_manifest).unwrap();
    for a in live_manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            sha2::Sha256::digest(std::fs::read(live_root.join(a["path"].as_str().unwrap())).unwrap()).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            a["sha256"]
        );
    }
    let live_proof: serde_json::Value = serde_json::from_slice(&std::fs::read(live_root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in proof["checks"].as_array().unwrap() {
        let state = &row["fresh"];
        let layer = &state["layers"][0];
        if layer["native_format"].as_str().unwrap().starts_with('Y')
            || state["precision"].as_str().unwrap().contains("DOUBLE")
            || layer["filters"][0]["config"]["colorspace"] != "CIE-Yuv"
        {
            continue;
        }
        let precision = state["precision"].as_str().unwrap();
        let width = if precision.contains("U16") {
            2
        } else if precision.contains("FLOAT") {
            4
        } else {
            8
        };
        let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
        let channels = if gray { 1 } else { 3 };
        let stored =
            if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
        let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let epoch = "xcf-22-stock-rgb-yuv-saturation-precision-v1";
        let node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:saturation".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
            arguments: vec![
                FilterArgument {
                    name: "scale".into(),
                    value: FilterValue::Number(ExactNumber::Float32Bits((layer["filters"][0]["config"]["scale"].as_f64().unwrap() as f32).to_bits())),
                },
                FilterArgument {
                    name: "colorspace".into(),
                    value: FilterValue::Enum {
                        domain: "GeglSaturationType".into(),
                        value: match layer["filters"][0]["config"]["colorspace"].as_str().unwrap() {
                            "Native" => 0,
                            "CIE-Lab" => 1,
                            "CIE-Yuv" => 2,
                            _ => panic!("unexpected observed choice"),
                        },
                    },
                },
            ],
            source_display_name: None,
            source_icon: None,
            enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: epoch.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -1581350319,
                stored_composite_space: -1581350319,
                stored_operator: -1581350319,
                resolved: None,
                pass_through: false,
            }),
        };
        let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
        let mut edited_existing = node.clone();
        edited_existing.operation.schema_epoch = SATURATION_PRECISION_EPOCH.into();
        edited_existing.composition.as_mut().unwrap().compatibility_epoch = SATURATION_PRECISION_EPOCH.into();
        let edited_stack = PointStack::from_nodes(std::slice::from_ref(&edited_existing), profile, stored, 22).unwrap();
        let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
        let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        let observation = live_proof["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
        let merged = std::fs::read(live_root.join(observation["merged"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(merged, reference, "fresh independent public merge reproduces original native");
        let live_bytes = std::fs::read(live_root.join(observation["live"]["projection"]["file"].as_str().unwrap())).unwrap();
        let merged_bytes = std::fs::read(live_root.join(observation["merged"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(live_bytes, merged_bytes, "live agrees with independent merge for admitted depths");
        let display = WorkingSpace::new(profile, stored, GimpSpace::ProfileRgb).unwrap();
        let mut display_worst = 0.0_f64;
        let decode = |bytes: &[u8]| match width {
            2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
            4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
            _ => f64::from_le_bytes(bytes.try_into().unwrap()),
        };
        assert_eq!(source.len(), 48 * (channels + 1) * width);
        assert_eq!(source.len(), reference.len());
        let mut worst = 0.0_f64;
        for (i, (pixel, expected)) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)).enumerate() {
            let color = decode(&pixel[..width]);
            let alpha = decode(&pixel[channels * width..]);
            let input =
                if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
            let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
            assert_eq!(actual, edited_stack.apply(input, &mut |_| Ok(())).unwrap());
            let quantize = |v: f64| if width == 2 { (v.clamp(0.0, 1.0) * 65535.0).round() / 65535.0 } else { f64::from(v as f32) };
            let displayed = display.convert([quantize(actual[0]), quantize(actual[1]), quantize(actual[2])], true).unwrap();
            for (c, v) in displayed.iter().enumerate() {
                let at = (i * 4 + c) * 8;
                let expected = f64::from_le_bytes(live_bytes[at..at + 8].try_into().unwrap());
                display_worst = display_worst.max((v - expected).abs() * alpha);
            }
            assert_eq!(actual[3].to_bits(), alpha.to_bits());
            assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
            for (channel, expected) in expected.chunks_exact(width).enumerate() {
                let value = if channel == channels { actual[3] } else { actual[channel] };
                let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                worst = worst.max((value - decode(expected)).abs());
            }
        }
        println!("{} Saturation private native {}/255", row["id"], worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
        println!("{} independent live {}/255", row["id"], display_worst * 255.0);
        assert!(display_worst <= 2.0 / 255.0);
        count += 1;
        let mut invalid = node.clone();
        invalid.enabled = true;
        invalid.operation.schema_epoch = "unqualified".into();
        assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        let mut wrong_choice = node.clone();
        wrong_choice.arguments[1].value = FilterValue::Enum { domain: "GeglSaturationType".into(), value: 3 };
        assert!(PointStack::from_nodes(&[wrong_choice], profile, stored, 22).is_err());
    }
    assert_eq!(count, 12);
}
