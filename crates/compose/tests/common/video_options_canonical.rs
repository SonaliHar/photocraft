use super::*;
use photocraft_doc::{import_compositing::GimpComposition, item_identity::FilterId, xcf_filters::*};
fn node(operation: VideoDegradation, enabled: bool) -> FilterNode {
    FilterNode {
        id: FilterId(1),
        operation: OperationIdentity {
            name: "gegl:video-degradation".into(),
            stored_version: "0:0".into(),
            schema_epoch: VIDEO_DEGRADATION_OPTIONS_EPOCH.into(),
        },
        source_display_name: None,
        source_icon: None,
        arguments: vec![
            FilterArgument {
                name: "pattern".into(),
                value: FilterValue::Enum { domain: "gegl:video-degradation/pattern".into(), value: i32::from(operation.pattern) },
            },
            FilterArgument { name: "additive".into(), value: FilterValue::Bool(operation.additive) },
            FilterArgument { name: "rotated".into(), value: FilterValue::Bool(operation.rotated) },
        ],
        enabled,
        opacity_f64_bits: 1f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(0),
        mask: None,
        auxiliary: vec![],
        composition: Some(GimpComposition {
            compatibility_epoch: VIDEO_DEGRADATION_OPTIONS_EPOCH.into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: 2026996305,
            stored_composite_space: 2026996305,
            stored_operator: 2026996305,
            resolved: None,
            pass_through: false,
        }),
    }
}
#[test]
fn options_72_and_old_three_remain_separate_canonical_envelopes() {
    let mut admitted = 0;
    let mut old = 0;
    for pattern in 0..9 {
        for additive in [false, true] {
            for rotated in [false, true] {
                let operation = VideoDegradation { pattern, additive, rotated };
                for enabled in [false, true] {
                    let n = node(operation, enabled);
                    assert!(current_epoch_parameters(VIDEO_DEGRADATION_OPTIONS_EPOCH, operation, enabled));
                    assert!(!current_epoch_parameters("future", operation, enabled));
                    assert!(canonical_candidate(&n, Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, Rect::new(0, 0, 16, 16), 22).is_ok());
                    let mut old_node = n.clone();
                    old_node.operation.schema_epoch = VIDEO_DEGRADATION_EPOCH.into();
                    assert!(canonical_candidate(&old_node, Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, Rect::new(0, 0, 16, 16), 22).is_err());
                    let policy = old_node.composition.as_mut().unwrap();
                    policy.compatibility_epoch = crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
                    policy.stored_blend_space = -1877441967;
                    policy.stored_composite_space = -1877441967;
                    policy.stored_operator = -1877441967;
                    let expected = (pattern, additive, rotated) == (5, false, false) || enabled && (pattern, additive, rotated) == (2, true, true);
                    assert_eq!(current_epoch_parameters(VIDEO_DEGRADATION_EPOCH, operation, enabled), expected);
                    assert_eq!(
                        canonical_candidate(&old_node, Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, Rect::new(0, 0, 16, 16), 22).is_ok(),
                        expected
                    );
                    old += usize::from(expected);
                    admitted += 1;
                }
            }
        }
    }
    assert_eq!(admitted, 72);
    assert_eq!(old, 3);
    assert!(!current_epoch_parameters(VIDEO_DEGRADATION_OPTIONS_EPOCH, VideoDegradation { pattern: 9, additive: false, rotated: false }, true));
}
#[test]
fn claimed_options_epoch_cannot_enter_another_operation_or_policy() {
    let original = node(VideoDegradation { pattern: 0, additive: false, rotated: false }, true);
    for mutation in 0..8 {
        let mut bad = original.clone();
        match mutation {
            0 => bad.operation.name = "gegl:invert-gamma".into(),
            1 => bad.operation.schema_epoch = "xcf-22-stock-point-v1".into(),
            2 => bad.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
            3 => bad.composition.as_mut().unwrap().stored_operator = -1877441967,
            4 => bad.source_region = None,
            5 => bad.source_clipping = Some(1),
            6 => bad.opacity_f64_bits = f64::NAN.to_bits(),
            7 => bad.composition.as_mut().unwrap().legacy = true,
            _ => panic!("unexpected mutation ordinal"),
        }
        assert!(claims_video(&bad));
        assert!(
            crate::gimp_filters::PointStack::from_nodes_at(&[bad], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22, Rect::new(0, 0, 16, 16))
                .is_err(),
            "mutation{mutation}"
        );
    }
    for bounds in [Rect::new(0, 0, 32, 24), Rect::new(2, 1, 18, 17), Rect::new(-5, -3, 11, 13)] {
        assert!(canonical_candidate(&original, Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, bounds, 22).is_err());
    }
}
