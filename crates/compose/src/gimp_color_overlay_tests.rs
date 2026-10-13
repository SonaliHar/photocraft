use super::*;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::FilterId,
    xcf_filters::{FilterArgument, OperationIdentity},
};
fn node() -> FilterNode {
    FilterNode {
        id: FilterId(1),
        operation: OperationIdentity { name: "gegl:color-overlay".into(), stored_version: "0:0".into(), schema_epoch: COLOR_OVERLAY_PRECISION_EPOCH.into() },
        arguments: vec![
            FilterArgument {
                name: "value".into(),
                value: FilterValue::Color {
                    components: [0.25f32, 0.5, 0.75, 0.25].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
            },
            FilterArgument { name: "srgb".into(), value: FilterValue::Bool(true) },
        ],
        source_display_name: None,
        source_icon: None,
        enabled: true,
        opacity_f64_bits: 1f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(0),
        mask: None,
        auxiliary: vec![],
        composition: Some(GimpComposition {
            compatibility_epoch: POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: 1435337297,
            stored_composite_space: 1435337297,
            stored_operator: 1435337297,
            resolved: None,
            pass_through: false,
        }),
    }
}
#[test]
fn typed_overlay_uses_float_working_boundary_and_preserves_raw_configuration() {
    let n = node();
    let original = n.clone();
    let p = photocraft_cms::Builtin::Srgb.profile();
    let stack = PointStack::from_nodes(std::slice::from_ref(&n), p, SampleEncoding::NonlinearProfile, 22).unwrap();
    let out = stack.apply([1., 0., 0.5, 0.125], &mut |_| Ok(())).unwrap();
    let expected = crate::gimp_color_overlay::ColorOverlayCandidate::new(p, SampleEncoding::NonlinearProfile, [0.25, 0.5, 0.75, 0.25], true)
        .unwrap()
        .apply(&[1., 0., 0.5, 0.125], true)
        .unwrap();
    assert_eq!(out, expected);
    assert_eq!(n, original);
    assert!(stack.changes_alpha());
    assert!(stack.apply([1., 0., 0.5, 0.125], &mut |_| Err(CompositionError::Invalid("cancel control"))).is_err());
    let gray = photocraft_cms::Builtin::SGray.profile();
    let stack = PointStack::from_nodes(&[n], gray, SampleEncoding::Linear, 22).unwrap();
    assert!(stack.apply([1.0000000361224046; 4], &mut |_| Ok(())).is_err());
    assert!(stack.apply([1.0000000361224046, 0., 0., 1.], &mut |_| Ok(())).is_ok());
    assert!(stack.apply([1.1, 0., 0., 1.], &mut |_| Ok(())).is_err());
}
#[test]
fn typed_overlay_rejects_unknown_epoch_fields_and_argument_shapes_even_disabled() {
    for mutation in 0..9 {
        let mut n = node();
        n.enabled = false;
        match mutation {
            0 => n.operation.schema_epoch = "unknown".into(),
            1 => n.operation.stored_version = "future".into(),
            2 => n.composition.as_mut().unwrap().stored_operator = 0,
            3 => n.arguments[0].value = FilterValue::Bool(true),
            4 => n.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
            5 => n.arguments.push(n.arguments[0].clone()),
            6 => n.opacity_f64_bits = 0.5f64.to_bits(),
            7 => n.source_region = Some(1),
            _ => n.source_clipping = Some(1),
        }
        assert!(PointStack::from_nodes(&[n], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Linear, 22).is_err(), "mutation {mutation}");
    }
    let mut n = node();
    n.enabled = false;
    let stack = PointStack::from_nodes(&[n], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Linear, 22).unwrap();
    let input = [0.123456789123, 0.4, 0.6, 0.3456789123];
    assert_eq!(stack.apply(input, &mut |_| Ok(())).unwrap(), input);
    assert!(!stack.changes_alpha());
}
