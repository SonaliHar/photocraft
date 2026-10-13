//! Typed Alpha Clip schema; the caller chooses the independently observed working representation.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::{
    point::PointOperation,
    registry::{PointMember, WorkingEncoding, alpha_clip_member},
};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
pub(super) fn operation(node: &FilterNode, encoding: WorkingEncoding) -> Result<(PointMember, PointOperation)> {
    if node.operation.schema_epoch != super::ALPHA_CLIP_PRECISION_EPOCH
        || node
            .composition
            .as_ref()
            .is_none_or(|p| p.stored_blend_space != -1710980527 || p.stored_composite_space != -1710980527 || p.stored_operator != -1710980527)
    {
        return Err(CompositionError::Unsupported("unit Alpha Clip saved compatibility tuple"));
    }
    if node.arguments.len() != 4 {
        return Err(CompositionError::Unsupported("Alpha Clip argument count"));
    }
    let get = |name: &str| {
        let mut values = node.arguments.iter().filter(|a| a.name == name);
        let value = values.next().ok_or(CompositionError::Unsupported("Alpha Clip missing argument"))?;
        if values.next().is_some() {
            return Err(CompositionError::Unsupported("Alpha Clip duplicate argument"));
        }
        Ok(&value.value)
    };
    let boolean = |name| -> Result<bool> {
        let FilterValue::Bool(value) = get(name)? else {
            return Err(CompositionError::Unsupported("Alpha Clip boolean type"));
        };
        Ok(*value)
    };
    let limit = |name| -> Result<f32> {
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = get(name)? else {
            return Err(CompositionError::Unsupported("Alpha Clip threshold type"));
        };
        Ok(f32::from_bits(*bits))
    };
    Ok(alpha_clip_member(
        Some(&node.operation.stored_version),
        boolean("clip-low")?,
        limit("low-limit")?,
        boolean("clip-high")?,
        limit("high-limit")?,
        encoding,
    )?)
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_cms::Builtin;
    use photocraft_doc::{
        SampleEncoding,
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node() -> FilterNode {
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gegl:alpha-clip".into(),
                stored_version: "0:0".into(),
                schema_epoch: super::super::ALPHA_CLIP_PRECISION_EPOCH.into(),
            },
            arguments: vec![
                FilterArgument { name: "clip-low".into(), value: FilterValue::Bool(true) },
                FilterArgument { name: "low-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())) },
                FilterArgument { name: "clip-high".into(), value: FilterValue::Bool(true) },
                FilterArgument { name: "high-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
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
                compatibility_epoch: super::super::POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -1710980527,
                stored_composite_space: -1710980527,
                stored_operator: -1710980527,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    fn typed_member_has_operative_alpha_and_exact_noop_bits() {
        let input = [0.125 + 2f64.powi(-40), 0.375 + 2f64.powi(-39), 0.625 + 2f64.powi(-41), 0.125];
        let original = node();
        operation(&original, WorkingEncoding::Nonlinear).unwrap();
        let stack =
            super::super::PointStack::from_nodes(std::slice::from_ref(&original), Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
        assert_eq!(stack.apply(input, &mut |_| Ok(())).unwrap(), [0.125, 0.375, 0.625, 0.25]);
        assert!(stack.changes_alpha());
        for disabled in [true, false] {
            let mut n = original.clone();
            if disabled {
                n.enabled = false;
            } else {
                n.arguments[0].value = FilterValue::Bool(false);
                n.arguments[2].value = FilterValue::Bool(false);
            }
            let stack = super::super::PointStack::from_nodes(&[n], Builtin::Srgb.profile(), SampleEncoding::Perceptual, 22).unwrap();
            assert_eq!(stack.apply(input, &mut |_| Ok(())).unwrap().map(f64::to_bits), input.map(f64::to_bits));
            assert!(!stack.changes_alpha());
        }
    }
    #[test]
    fn schema_types_ranges_duplicates_and_envelopes_are_closed() {
        let n = node();
        for i in 0..11 {
            let mut changed = n.clone();
            match i {
                0 => changed.operation.stored_version = "0:1".into(),
                1 => changed.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
                2 => changed.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits())),
                3 => changed.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
                4 => changed.arguments[3].value = FilterValue::Number(ExactNumber::Float32Bits(f32::INFINITY.to_bits())),
                5 => changed.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits((-0.1f32).to_bits())),
                6 => changed.arguments[3].value = FilterValue::Number(ExactNumber::Float32Bits(0.1f32.to_bits())),
                7 => changed.arguments[0].name = "unknown".into(),
                8 => changed.arguments[2].name = "clip-low".into(),
                9 => {
                    changed.arguments.pop();
                }
                _ => changed.arguments.push(changed.arguments[0].clone()),
            };
            assert!(operation(&changed, WorkingEncoding::Linear).is_err(), "{i}");
        }
    }
    #[test]
    fn unit_epoch_rejects_mixed_and_finite_tuple_without_migration() {
        for value in [0, -305954223] {
            for field in 0..4 {
                let mut n = node();
                let policy = n.composition.as_mut().unwrap();
                match field {
                    0 => policy.stored_blend_space = value,
                    1 => policy.stored_composite_space = value,
                    2 => policy.stored_operator = value,
                    _ => {
                        policy.stored_blend_space = value;
                        policy.stored_composite_space = value;
                        policy.stored_operator = value;
                    }
                }
                assert!(operation(&n, WorkingEncoding::Linear).is_err());
                assert!(super::super::PointStack::from_nodes(&[n], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).is_err());
            }
        }
        assert!(operation(&node(), WorkingEncoding::Linear).is_ok());
    }
    #[test]
    fn disabled_bad_schema_and_current_envelopes_do_not_bypass_validation() {
        for disabled in [false, true] {
            for i in 0..7 {
                let mut n = node();
                n.enabled = !disabled;
                match i {
                    0 => n.operation.schema_epoch = "other".into(),
                    1 => n.operation.name = "gegl:unknown".into(),
                    2 => n.opacity_f64_bits = 0.5f64.to_bits(),
                    3 => n.source_clipping = None,
                    4 => n.source_region = Some(1),
                    5 => n.composition.as_mut().unwrap().source_mode = 28,
                    _ => n.composition.as_mut().unwrap().pass_through = true,
                };
                assert!(super::super::PointStack::from_nodes(&[n], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).is_err(), "{disabled} {i}");
            }
        }
    }
}
