//! Synthetic canonical schema tests, separate from independent source qualification.
use crate::gimp_filters::PointStack;
use photocraft_cms::Builtin;
use photocraft_doc::{
    SampleEncoding, import_compositing::GimpComposition, item_identity::FilterId,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue, OperationIdentity},
};
use photocraft_geom::Rect;

const NEW: &str = "xcf-22-stock-red-eye-removal-owner-geometry-v1";
fn node(threshold: f32, enabled: bool) -> FilterNode {
    FilterNode {
        id: FilterId(1),
        operation: OperationIdentity { name: "gegl:red-eye-removal".into(), stored_version: "0:0".into(), schema_epoch: NEW.into() },
        source_display_name: None, source_icon: None,
        arguments: vec![FilterArgument { name: "threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(threshold.to_bits())) }],
        enabled, opacity_f64_bits: 1f64.to_bits(), source_clipping: Some(0), source_region: Some(0), mask: None, auxiliary: vec![],
        composition: Some(GimpComposition { compatibility_epoch: NEW.into(), source_mode: 62, legacy: false, pass_through: false,
            stored_blend_space: -487751087, stored_composite_space: -487751087, stored_operator: -487751087, resolved: None }),
    }
}
fn prepare(node: FilterNode, extent: Rect) -> bool {
    PointStack::from_nodes_at(&[node], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22, extent).is_ok()
}
#[test]
fn new_geometry_canonical_states_require_their_own_epoch() {
    let mut count = 0;
    for extent in [Rect::new(-7, 5, 12, 18), Rect::new(-3, -4, 259, 5)] {
        for (threshold, enabled) in [(0.4, true), (0.675, true), (0.4, false)] {
            assert!(prepare(node(threshold, enabled), extent), "NEW geometry extent{extent:?} threshold{threshold} enabled{enabled}");
            count += 1;
        }
    }
    assert_eq!(count, 6);
}
#[test]
fn mixed_geometry_epoch_policy_and_operation_claims_refuse() {
    let mut old = node(0.25, true);
    old.operation.schema_epoch = crate::gimp_filters::RED_EYE_PRECISION_EPOCH.into();
    let policy = old.composition.as_mut().unwrap();
    policy.compatibility_epoch = crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
    policy.stored_blend_space = 127500881;
    policy.stored_composite_space = 127500881;
    policy.stored_operator = 127500881;
    assert!(prepare(old.clone(), Rect::new(0, 0, 8, 6)), "OLD canonical behavior remains valid");
    for mutation in 0..7 {
        let mut bad = node(0.4, true);
        match mutation {
            0 => bad.operation.schema_epoch = crate::gimp_filters::RED_EYE_PRECISION_EPOCH.into(),
            1 => bad.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
            2 => bad.operation.name = "gegl:invert-gamma".into(),
            3 => bad.operation.stored_version = "future".into(),
            4 => bad.composition.as_mut().unwrap().stored_operator = 127500881,
            5 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
            _ => bad.arguments.push(bad.arguments[0].clone()),
        }
        assert!(!prepare(bad, Rect::new(-7, 5, 12, 18)), "mixed or invalid NEW marker mutation{mutation}");
    }
    old.composition.as_mut().unwrap().compatibility_epoch = NEW.into();
    assert!(!prepare(old, Rect::new(0, 0, 8, 6)), "OLDschema plus NEWpolicy refuses");
}

#[test]
fn geometry_metadata_is_exact_and_new_profile_encoding_extent_are_closed() {
    for (canvas, owner, offset) in [((37, 29), (19, 13), (-7, 5)), ((259, 17), (262, 9), (-3, -4))] {
        assert!(super::qualified_owner_geometry(NEW, canvas, owner, offset));
        for (bad_canvas, bad_owner, bad_offset) in [
            ((canvas.0 + 1, canvas.1), owner, offset), (canvas, (owner.0 + 1, owner.1), offset),
            (canvas, owner, (offset.0 + 1, offset.1)), ((canvas.0, canvas.1 + 1), owner, offset),
            (canvas, (owner.0, owner.1 + 1), offset), (canvas, owner, (offset.0, offset.1 + 1)),
        ] {
            assert!(!super::qualified_owner_geometry(NEW, bad_canvas, bad_owner, bad_offset));
        }
        assert!(!super::qualified_owner_geometry(crate::gimp_filters::RED_EYE_PRECISION_EPOCH, canvas, owner, offset));
        assert!(!super::qualified_owner_geometry("future", canvas, owner, offset));
    }
    assert!(super::qualified_owner_geometry(crate::gimp_filters::RED_EYE_PRECISION_EPOCH, (8, 6), (8, 6), (0, 0)));
    assert!(!super::qualified_owner_geometry(NEW, (8, 6), (8, 6), (0, 0)));
    for extent in [Rect::new(0, 0, 19, 13), Rect::new(-8, 5, 11, 18), Rect::new(-7, 5, 13, 18), Rect::new(i32::MIN, 0, i32::MAX, 1)] {
        assert!(!prepare(node(0.4, false), extent));
    }
    for profile in [Builtin::SGray.profile(), Builtin::DisplayP3.profile()] {
        assert!(PointStack::from_nodes_at(&[node(0.4, false)], profile, SampleEncoding::NonlinearProfile, 22, Rect::new(-7, 5, 12, 18)).is_err());
    }
    for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
        assert!(PointStack::from_nodes_at(&[node(0.4, true)], Builtin::Srgb.profile(), encoding, 22, Rect::new(-7, 5, 12, 18)).is_err());
    }
}

#[test]
fn disabled_geometry_uses_canonical_ordered_dispatch_before_enabled_filtering() {
    use photocraft_doc::{LayerId, item_identity::ItemRef, xcf_filters::FilterStack};
    let prepare = |node: FilterNode, extent| {
        crate::gimp_neighborhood::OrderedStack::new(&FilterStack { owner: ItemRef::Layer(LayerId(1)), nodes: vec![node], revision: 0 },
            Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22, extent, [false; 3]).is_ok()
    };
    let extent = Rect::new(-7, 5, 12, 18);
    assert!(prepare(node(0.4, false), extent));
    for mutation in 0..5 {
        let mut bad = node(0.4, false);
        match mutation {
            0 => bad.operation.schema_epoch = crate::gimp_filters::RED_EYE_PRECISION_EPOCH.into(),
            1 => bad.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
            2 => bad.operation.stored_version = "future".into(),
            3 => bad.operation.name = "gegl:invert-gamma".into(),
            _ => bad.composition.as_mut().unwrap().stored_operator = 127500881,
        }
        assert!(!prepare(bad, extent), "disabled malformed NEW claim mutation{mutation}");
    }
    assert!(!prepare(node(0.4, false), Rect::new(0, 0, 19, 13)));
}
