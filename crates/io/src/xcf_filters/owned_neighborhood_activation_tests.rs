//! UNRUN typed mapper tests, not original-source or numerical qualification.
use super::*;
use photocraft_doc::{LayerId, item_identity::MaskId};
use photocraft_xcf::{Argument, EffectState};
fn arg(name: &str, value: ArgumentValue) -> Argument {
    Argument { name: name.into(), value, property_index: 0, configuration: None, graph: None }
}
fn f(name: &str, v: f32) -> Argument {
    arg(name, ArgumentValue::Float32Bits(v.to_bits()))
}
fn effect(domain: NeighborhoodDomain) -> Effect {
    let (operation, args) = match domain {
        NeighborhoodDomain::BoxOriginal | NeighborhoodDomain::BoxFinite => ("gegl:box-blur", vec![arg("radius", ArgumentValue::Integer(4))]),
        NeighborhoodDomain::MedianOriginal | NeighborhoodDomain::MedianFinite | NeighborhoodDomain::MedianGray => (
            "gegl:median-blur",
            vec![
                arg("radius", ArgumentValue::Integer(3)),
                arg("neighborhood", ArgumentValue::Enum(1)),
                arg("abyss-policy", ArgumentValue::Enum(1)),
                f("percentile", 50.),
                f("alpha-percentile", 50.),
                arg("high-precision", ArgumentValue::Boolean(false)),
            ],
        ),
        NeighborhoodDomain::MotionOriginal => ("gegl:motion-blur-linear", vec![f("length", 10.), f("angle", 0.)]),
        NeighborhoodDomain::PixelizeOriginal => (
            "gegl:pixelize",
            vec![
                arg("norm", ArgumentValue::Enum(2)),
                arg("size-x", ArgumentValue::Integer(16)),
                arg("size-y", ArgumentValue::Integer(16)),
                arg("offset-x", ArgumentValue::Integer(0)),
                arg("offset-y", ArgumentValue::Integer(0)),
                f("ratio-x", 1.),
                f("ratio-y", 1.),
                arg("background", ArgumentValue::Color { encoding: "R'G'B'A float".into(), samples: [0, 0, 128, 63].repeat(4), icc: Vec::new() }),
            ],
        ),
        NeighborhoodDomain::RasterFinite => (
            "gegl:gaussian-blur",
            vec![
                f("std-dev-x", 2.25),
                f("std-dev-y", 1.3),
                arg("filter", ArgumentValue::Enum(0)),
                arg("abyss-policy", ArgumentValue::Enum(1)),
                arg("clip-extent", ArgumentValue::Boolean(true)),
            ],
        ),
        _ => unreachable!(),
    };
    let raw = if matches!(
        domain,
        NeighborhoodDomain::BoxFinite | NeighborhoodDomain::MedianFinite | NeighborhoodDomain::MedianGray | NeighborhoodDomain::RasterFinite
    ) {
        -1474592175
    } else {
        -261651887
    };
    Effect {
        name: "source label".into(),
        icon: "source icon".into(),
        operation: operation.into(),
        version: Some("0:0".into()),
        arguments: args,
        properties: Vec::new(),
        mask: None,
        state: EffectState {
            enabled: Some(false),
            opacity_bits: Some(1f32.to_bits()),
            mode: Some(62),
            composite_mode: Some(raw),
            composite_space: Some(raw),
            blend_space: Some(raw),
            clipping: Some(0),
            region: Some(0),
            ..Default::default()
        },
    }
}
fn domains() -> [NeighborhoodDomain; 8] {
    [
        NeighborhoodDomain::BoxOriginal,
        NeighborhoodDomain::BoxFinite,
        NeighborhoodDomain::MedianOriginal,
        NeighborhoodDomain::MedianFinite,
        NeighborhoodDomain::MedianGray,
        NeighborhoodDomain::MotionOriginal,
        NeighborhoodDomain::PixelizeOriginal,
        NeighborhoodDomain::RasterFinite,
    ]
}
fn owner() -> ItemRef {
    ItemRef::Layer(LayerId(7))
}
fn masks() -> [Option<ItemRef>; 1] {
    [Some(ItemRef::Mask(MaskId(12)))]
}
#[test]
fn only_owned_wrapper_preserves_disabled_bool_and_enabled_output_matches_strict_mapping() {
    for domain in domains() {
        let mut e = effect(domain);
        assert!(qualified_stack(std::slice::from_ref(&e), &masks(), owner(), 10, 22, domain).is_err());
        let mapped = qualified_owned_neighborhood_stack(std::slice::from_ref(&e), &masks(), owner(), 10, 22, domain).unwrap();
        assert_eq!(mapped.nodes.len(), 1);
        assert!(!mapped.nodes[0].enabled);
        assert_eq!(mapped.nodes[0].mask, masks()[0]);
        assert_eq!(mapped.nodes[0].id, FilterId(10));
        assert_eq!(mapped.revision, 0);
        e.state.enabled = Some(true);
        let strict = qualified_stack(std::slice::from_ref(&e), &masks(), owner(), 10, 22, domain).unwrap();
        let owned = qualified_owned_neighborhood_stack(std::slice::from_ref(&e), &masks(), owner(), 10, 22, domain).unwrap();
        assert_eq!(strict, owned);
        let mut expected = strict;
        expected.nodes[0].enabled = false;
        assert_eq!(mapped, expected);
    }
}
#[test]
fn disabled_mapping_does_not_relax_any_other_source_envelope_or_argument_gate() {
    for domain in domains() {
        for case in 0..10 {
            let mut e = effect(domain);
            match case {
                0 => e.state.enabled = None,
                1 => e.state.opacity_bits = Some(0.5f32.to_bits()),
                2 => e.state.opacity_u8 = Some(255),
                3 => e.state.mode = Some(28),
                4 => e.state.blend_space = Some(0),
                5 => e.state.clipping = None,
                6 => e.state.region = Some(1),
                7 => e.version = Some("99:99".into()),
                8 => e.arguments[0].value = ArgumentValue::String("unqualified".into()),
                _ => e.arguments.push(e.arguments[0].clone()),
            };
            assert!(qualified_owned_neighborhood_stack(&[e], &masks(), owner(), 10, 22, domain).is_err());
        }
    }
}
#[test]
fn wrapper_is_closed_to_eight_singletons_and_keeps_source_records_immutable() {
    let e = effect(NeighborhoodDomain::BoxOriginal);
    let before = e.clone();
    for domain in
        [NeighborhoodDomain::RasterOriginal, NeighborhoodDomain::IsolatedSmallAuto, NeighborhoodDomain::Unqualified, NeighborhoodDomain::ValueInvertOriginal]
    {
        assert!(qualified_owned_neighborhood_stack(std::slice::from_ref(&e), &masks(), owner(), 10, 22, domain).is_err());
    }
    assert!(qualified_owned_neighborhood_stack(&[e.clone(), e.clone()], &[masks()[0], masks()[0]], owner(), 10, 22, NeighborhoodDomain::BoxOriginal).is_err());
    assert!(qualified_owned_neighborhood_stack(std::slice::from_ref(&e), &[], owner(), 10, 22, NeighborhoodDomain::BoxOriginal).is_err());
    let _ = qualified_owned_neighborhood_stack(std::slice::from_ref(&e), &masks(), owner(), 10, 22, NeighborhoodDomain::BoxOriginal).unwrap();
    assert_eq!(e.operation, before.operation);
    assert_eq!(e.arguments, before.arguments);
    assert_eq!(e.state.enabled, before.state.enabled);
    assert_eq!(e.state.blend_space, before.state.blend_space);
    let mut hostile = e;
    let arg = hostile.arguments[0].clone();
    hostile.arguments = vec![arg; 100000];
    assert!(qualified_owned_neighborhood_stack(&[hostile], &masks(), owner(), 10, 22, NeighborhoodDomain::BoxOriginal).is_err());
}
