mod common;
use photocraft_xcf::*;
fn string(out: &mut Vec<u8>, value: &str) {
    common::u32be(out, value.len() as u32 + 1);
    out.extend(value.as_bytes());
    out.push(0);
}
fn effect() -> Vec<u8> {
    let mut out = Vec::new();
    for s in ["Own blur Ω", "gimp-gegl", "gegl:gaussian-blur", "0:0"] {
        string(&mut out, s);
    }
    common::prop(&mut out, 35, &(-856260015i32).to_be_bytes());
    common::prop(&mut out, 8, &1u32.to_be_bytes());
    common::prop(&mut out, 33, &0.75f32.to_be_bytes());
    common::prop(&mut out, 7, &28i32.to_be_bytes());
    common::prop(&mut out, 36, &(-1781848959i32).to_be_bytes());
    common::prop(&mut out, 37, &(-1781848959i32).to_be_bytes());
    common::prop(&mut out, 46, &1u32.to_be_bytes());
    let mut arg = Vec::new();
    string(&mut arg, "std-dev-x");
    common::u32be(&mut arg, 3);
    arg.extend(2.25f32.to_be_bytes());
    common::prop(&mut out, 45, &arg);
    common::prop(&mut out, 0, &[]);
    common::pointer(&mut out, 0, 22);
    out
}
#[test]
fn named_effect_and_unknown_properties_are_retained_without_render_claim() {
    let options = common::FixtureOptions { effect_records: vec![effect()], layer_properties: vec![(9001, vec![1, 2, 3])], ..Default::default() };
    let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    let file = decode(&bytes, &Default::default()).unwrap();
    let layer = &file.layers[0];
    assert_eq!(layer.effects[0].operation, "gegl:gaussian-blur");
    assert_eq!(layer.effects[0].version.as_deref(), Some("0:0"));
    assert_eq!(layer.effects[0].name, "Own blur Ω");
    let state = &layer.effects[0].state;
    assert_eq!(state.enabled, Some(true));
    assert_eq!(state.opacity_bits, Some(0.75f32.to_bits()));
    assert_eq!(state.mode, Some(28));
    assert_eq!(state.composite_mode, Some(-856260015));
    assert_eq!(state.composite_space, Some(-1781848959));
    assert_eq!(state.blend_space, Some(-1781848959));
    assert_eq!(state.clipping, Some(1));
    assert_eq!(layer.effects[0].arguments[0].name, "std-dev-x");
    assert_eq!(layer.effects[0].arguments[0].value, ArgumentValue::Float32Bits(2.25f32.to_bits()));
    for p in &layer.effects[0].properties {
        assert_eq!(&bytes[p.offset as usize..p.offset as usize + p.payload.len()], &p.payload);
    }
    assert!(layer.raw_properties.iter().any(|p| p.tag == 9001 && p.payload == [1, 2, 3]));
    assert!(layer.unsupported.iter().any(|s| s.contains("effects")));
}
#[test]
fn effect_limits_and_truncation_are_atomic_errors() {
    let options = common::FixtureOptions { effect_records: vec![effect()], ..Default::default() };
    let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    assert!(decode(&bytes[..bytes.len() - 1], &Default::default()).is_err());
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("filtersPerOwner", 0)]).unwrap();
    assert_eq!(decode_with_resources(&bytes, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
}
#[test]
fn hostile_effect_envelope_fields_fail_atomically() {
    for (tag, payload) in [(8, 2u32.to_be_bytes().to_vec()), (33, f32::NAN.to_be_bytes().to_vec()), (33, 1.01f32.to_be_bytes().to_vec()), (46, vec![0; 3])] {
        let mut record = Vec::new();
        for s in ["Own effect", "icon", "own:test", "0:0"] {
            string(&mut record, s);
        }
        common::prop(&mut record, tag, &payload);
        common::prop(&mut record, 0, &[]);
        common::pointer(&mut record, 0, 22);
        let options = common::FixtureOptions { effect_records: vec![record], ..Default::default() };
        let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
        assert!(decode(&bytes, &Default::default()).is_err());
    }
}
#[test]
fn typed_arguments_preserve_values_and_reject_envelope_overrun() {
    let mut values = vec![
        (1, (-123i32).to_be_bytes().to_vec(), ArgumentValue::Integer(-123)),
        (2, 1u32.to_be_bytes().to_vec(), ArgumentValue::Boolean(true)),
        (3, (-0f64).to_be_bytes().to_vec(), ArgumentValue::Float64Bits((-0f64).to_bits())),
        (5, (-2i32).to_be_bytes().to_vec(), ArgumentValue::Enum(-2)),
        (7, u32::MAX.to_be_bytes().to_vec(), ArgumentValue::Unsigned(u32::MAX)),
        (999, vec![1, 2, 3], ArgumentValue::Unknown(999)),
    ];
    for (kind, text, typed) in
        [(4, "Original Ω", ArgumentValue::String("Original Ω".into())), (6, "(curve (n-samples 256))", ArgumentValue::Config("(curve (n-samples 256))".into()))]
    {
        let mut bytes = Vec::new();
        string(&mut bytes, text);
        values.push((kind, bytes, typed));
    }
    let mut color = Vec::new();
    string(&mut color, "RGBA float");
    common::u32be(&mut color, 4);
    color.extend([1, 2, 3, 4]);
    common::u32be(&mut color, 3);
    color.extend([5, 6, 7]);
    values.push((8, color, ArgumentValue::Color { encoding: "RGBA float".into(), samples: vec![1, 2, 3, 4], icc: vec![5, 6, 7] }));
    for (kind, value, expected) in values {
        let mut record = Vec::new();
        for s in ["Own effect", "icon", "own:test", "0:0"] {
            string(&mut record, s);
        }
        let mut arg = Vec::new();
        string(&mut arg, "own-value");
        common::u32be(&mut arg, kind);
        arg.extend(&value);
        common::prop(&mut record, 45, &arg);
        common::prop(&mut record, 0, &[]);
        common::pointer(&mut record, 0, 22);
        let options = common::FixtureOptions { effect_records: vec![record], ..Default::default() };
        let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
        let file = decode(&bytes, &Default::default()).unwrap();
        assert_eq!(file.layers[0].effects[0].arguments[0].value, expected);
        if kind != 999 {
            let mut corrupt = bytes.clone();
            let p = &file.layers[0].effects[0].properties[0];
            let type_offset = p.offset as usize + 4 + "own-value".len() + 1;
            corrupt[type_offset..type_offset + 4].copy_from_slice(&4u32.to_be_bytes());
            corrupt[type_offset + 4..type_offset + 8].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(decode(&corrupt, &Default::default()).is_err());
        }
    }
}
#[test]
fn linked_paths_and_transform_are_typed_without_accessing_external_files() {
    let mut link = 3u32.to_be_bytes().to_vec();
    common::u32be(&mut link, 0);
    link.extend(48u32.to_be_bytes());
    link.extend(42u32.to_be_bytes());
    let mut matrix = (-12i32).to_be_bytes().to_vec();
    matrix.extend(4i32.to_be_bytes());
    matrix.extend(3u32.to_be_bytes());
    for v in [1f32, 0., 5., 0., 1., 6., 0., 0., 1.] {
        matrix.extend(v.to_be_bytes());
    }
    let options = common::FixtureOptions { layer_properties: vec![(48, link.clone()), (49, matrix)], ..Default::default() };
    let bytes = common::image_custom(25, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    let f = decode(&bytes, &Default::default()).unwrap();
    let d = &f.layers[0];
    let l = d.link.as_ref().unwrap();
    assert_eq!(l.flags, 3);
    assert_eq!(l.path, "");
    assert_eq!((l.width, l.height), (48, 42));
    let t = d.transform.as_ref().unwrap();
    assert_eq!(t.offset, (-12, 4));
    assert_eq!(t.interpolation, 3);
    assert_eq!(t.matrix, [1., 0., 5., 0., 1., 6., 0., 0., 1.]);
    assert!(d.unsupported.iter().any(|s| s.contains("linked")));
}

#[test]
fn null_effect_color_preserves_exact_source_and_rejects_trailing_blobs() {
    for trailing in [false, true] {
        let mut argument = Vec::new();
        let name = "own nullable color";
        common::u32be(&mut argument, name.len() as u32 + 1);
        argument.extend(name.as_bytes());
        argument.push(0);
        common::u32be(&mut argument, 8);
        common::u32be(&mut argument, 0); // NULL, not transparent or zero samples
        if trailing {
            common::u32be(&mut argument, 0);
        }
        let mut record = Vec::new();
        for value in ["Own nullable color effect", "", "gegl:color", "0:0"] {
            common::u32be(&mut record, value.len() as u32 + 1);
            record.extend(value.as_bytes());
            record.push(0);
        }
        common::prop(&mut record, 45, &argument);
        common::prop(&mut record, 0, &[]);
        common::pointer(&mut record, 0, 22);
        let options = common::FixtureOptions { effect_records: vec![record], ..Default::default() };
        let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
        let result = decode(&bytes, &Default::default());
        if trailing {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Structure);
        } else {
            let file = result.expect("documented nullable color framing");
            let effect = &file.layers[0].effects[0];
            assert_eq!(effect.arguments[0].value, ArgumentValue::NullColor);
            assert_eq!(effect.properties[0].payload, argument);
            assert!(file.layers[0].unsupported.iter().any(|value| value.contains("effects")));
        }
    }
}
