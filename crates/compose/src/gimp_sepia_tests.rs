use super::*;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::FilterId,
    xcf_filters::{FilterArgument, OperationIdentity},
};
#[test]
#[ignore = "local unpublished independently authored Sepia RGB precision states"]
fn independent_rgb_sepia_native_and_live() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("sepia-precision-evidence");
    let read = |name: &str| std::fs::read(root.join(name)).unwrap();
    let binding = read("batch-manifest.json");
    let hex = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hex(&binding), "f12cc00111e3505e351e1a234916cad4b9ea38f577aa01127e3192792386d374");
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
        let epoch = "xcf-22-stock-sepia-precision-v1";
        let raw = i32::from_be_bytes([0xd1, 0x83, 0x82, 0x51]);
        let node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:sepia".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
            arguments: vec![
                FilterArgument {
                    name: "scale".into(),
                    value: FilterValue::Number(ExactNumber::Float32Bits((cfg["scale"].as_f64().unwrap() as f32).to_bits())),
                },
                FilterArgument { name: "srgb".into(), value: FilterValue::Bool(cfg["srgb"].as_bool().unwrap()) },
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
                stored_blend_space: raw,
                stored_composite_space: raw,
                stored_operator: raw,
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
        println!("{} Sepia native {}/255 live {}/255", row["id"], worst * 255., live_worst * 255.);
        assert!(worst <= 2. / 255.);
        assert!(live_worst <= 2. / 255.);
        let mut invalid = node.clone();
        invalid.operation.schema_epoch = "unknown".into();
        assert!(PointStack::from_nodes(std::slice::from_ref(&invalid), profile, stored, 22).is_err());
        let mut invalid = node.clone();
        invalid.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(1.0_f32.to_bits()));
        assert!(PointStack::from_nodes(std::slice::from_ref(&invalid), profile, stored, 22).is_err());
        let mut invalid = node.clone();
        invalid.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()));
        assert!(PointStack::from_nodes(std::slice::from_ref(&invalid), profile, stored, 22).is_err());
        assert!(PointStack::from_nodes(std::slice::from_ref(&node), photocraft_cms::Builtin::SGray.profile(), stored, 22).is_err());
        count += 1;
    }
    assert_eq!(count, 24);
}
