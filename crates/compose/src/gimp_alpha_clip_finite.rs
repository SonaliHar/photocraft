//! Typed finite Alpha Clip schema; the caller chooses the independently observed working representation.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::{
    point::PointOperation,
    registry::{PointMember, WorkingEncoding, alpha_clip_finite_member},
};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
pub(super) fn operation(node: &FilterNode, encoding: WorkingEncoding) -> Result<(PointMember, PointOperation)> {
    if node.operation.schema_epoch != super::ALPHA_CLIP_FINITE_PRECISION_EPOCH
        || node
            .composition
            .as_ref()
            .is_none_or(|p| p.stored_blend_space != -305954223 || p.stored_composite_space != -305954223 || p.stored_operator != -305954223)
    {
        return Err(CompositionError::Unsupported("finite Alpha Clip saved compatibility tuple"));
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
    Ok(alpha_clip_finite_member(
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
                schema_epoch: super::super::ALPHA_CLIP_FINITE_PRECISION_EPOCH.into(),
            },
            arguments: vec![
                FilterArgument { name: "clip-low".into(), value: FilterValue::Bool(true) },
                FilterArgument { name: "low-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.875f32.to_bits())) },
                FilterArgument { name: "clip-high".into(), value: FilterValue::Bool(true) },
                FilterArgument { name: "high-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.125f32.to_bits())) },
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
                stored_blend_space: -305954223,
                stored_composite_space: -305954223,
                stored_operator: -305954223,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    fn finite_crossed_current_member_and_native_identity() {
        let n = node();
        let input = [0.125 + 2f64.powi(-40), 0.375, 0.625, 0.25];
        let stack = super::super::PointStack::from_nodes(std::slice::from_ref(&n), Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
        assert_eq!(stack.apply(input, &mut |_| Ok(())).unwrap(), [0.125, 0.375, 0.625, 0.125]);
        for disabled in [true, false] {
            let mut n = n.clone();
            if disabled {
                n.enabled = false;
            } else {
                n.arguments[0].value = FilterValue::Bool(false);
                n.arguments[2].value = FilterValue::Bool(false);
            }
            let stack = super::super::PointStack::from_nodes(&[n], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
            assert_eq!(stack.apply(input, &mut |_| Ok(())).unwrap().map(f64::to_bits), input.map(f64::to_bits));
            assert!(!stack.changes_alpha());
        }
    }
    #[test]
    fn finite_tuple_schema_nonfinite_and_nonunit_output_are_closed() {
        let n = node();
        for i in 0..8 {
            let mut n = n.clone();
            match i {
                0 => n.operation.stored_version = "0:1".into(),
                1 => n.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
                2 => n.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(0.875f64.to_bits())),
                3 => n.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
                4 => n.arguments[3].value = FilterValue::Number(ExactNumber::Float32Bits(f32::INFINITY.to_bits())),
                5 => n.arguments.push(n.arguments[0].clone()),
                6 => n.composition.as_mut().unwrap().stored_blend_space = -1710980527,
                _ => n.operation.schema_epoch = super::super::ALPHA_CLIP_PRECISION_EPOCH.into(),
            }
            assert!(operation(&n, WorkingEncoding::Nonlinear).is_err());
        }
        let mut outside = n;
        outside.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(1.125f32.to_bits()));
        outside.arguments[2].value = FilterValue::Bool(false);
        let stack = super::super::PointStack::from_nodes(&[outside], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
        assert!(stack.apply([0.125, 0.375, 0.625, 0.25], &mut |_| Ok(())).is_err());
    }
}
