//! Exact authored quantized Median Blur envelope and checked operation-bound work estimate.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
use photocraft_geom::Rect;
pub(crate) const GRAY_EPOCH: &str = "xcf22-gray-alpha-median-v1";
pub(crate) const COLOR_EPOCH: &str = "xcf22-rgb-alpha-median-v1";
pub(crate) const EPOCH: &str = "xcf22-rgb-u8-quantized-median-v1";
pub(crate) fn radius(node: &FilterNode, version: u32, extent: Rect) -> Result<u32> {
    radius_mode(node,version,extent,false)
}
fn radius_mode(node: &FilterNode, version: u32, extent: Rect, owned: bool) -> Result<u32> {
    let c = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing quantized Median Blur policy"))?;
    let color = matches!(node.operation.schema_epoch.as_str(), COLOR_EPOCH | GRAY_EPOCH);
    if extent != if color { Rect::new(0, 0, 16, 12) } else { Rect::new(0, 0, 32, 32) } {
        return Err(CompositionError::Unsupported("Median source extent"));
    }
    let raw = if color { -1474592175 } else { -261651887 };
    if version != 22
        || node.operation.name != "gegl:median-blur"
        || node.operation.stored_version != "0:0"
        || (!color && node.operation.schema_epoch != EPOCH)
        || (!owned && !node.enabled)
        || node.opacity_f64_bits != 1.0f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || !node.auxiliary.is_empty()
        || c.source_mode != 62
        || c.legacy
        || c.pass_through
        || c.resolved.is_some()
        || [c.stored_blend_space, c.stored_composite_space, c.stored_operator] != [raw; 3]
        || node.arguments.len() != 6
    {
        return Err(CompositionError::Unsupported("quantized Median Blur Replace envelope"));
    }
    let mut radius = None;
    let mut high = false;
    let mut seen = 0u8;
    for arg in &node.arguments {
        let bit = match (arg.name.as_str(), &arg.value) {
            ("radius", FilterValue::Number(ExactNumber::Signed { width: 32, value: v })) if *v == 3 || *v == 4 => {
                radius = Some(*v as u32);
                1
            }
            ("neighborhood", FilterValue::Enum { domain, value: 1 }) if domain == "gegl:median-blur::neighborhood" => 2,
            ("percentile", FilterValue::Number(ExactNumber::Float32Bits(1112014848))) => 4,
            ("alpha-percentile", FilterValue::Number(ExactNumber::Float32Bits(1112014848))) => 8,
            ("abyss-policy", FilterValue::Enum { domain, value: 1 }) if domain == "gegl:median-blur::abyss-policy" => 16,
            ("high-precision", FilterValue::Bool(value)) if !*value || color => {
                high = *value;
                32
            }
            _ => return Err(CompositionError::Unsupported("Median exact argument domain")),
        };
        if seen & bit != 0 {
            return Err(CompositionError::Invalid("duplicate Median argument"));
        }
        seen |= bit;
    }
    if seen != 63 {
        return Err(CompositionError::Unsupported("Median missing argument"));
    }
    let radius = radius.ok_or(CompositionError::Unsupported("Median radius absent"))?;
    if color && high != (radius == 4) {
        return Err(CompositionError::Unsupported("Median measured radius/precision pair"));
    }
    Ok(radius)
}
pub(crate) fn parameters(node: &FilterNode, version: u32, extent: Rect) -> Result<photocraft_algo::gimp::neighborhood::MedianParams> {
    parameters_mode(node,version,extent,false)
}
/// Owned CURRENT metadata validation retains all exact typed/epoch predicates.
pub(crate) fn parameters_for_owned(node: &FilterNode, version: u32, extent: Rect) -> Result<photocraft_algo::gimp::neighborhood::MedianParams> {
    parameters_mode(node,version,extent,true)
}
fn parameters_mode(node: &FilterNode, version: u32, extent: Rect, owned:bool) -> Result<photocraft_algo::gimp::neighborhood::MedianParams> {
    let radius = if owned {radius_mode(node,version,extent,true)?} else {radius(node, version, extent)?};
    Ok(photocraft_algo::gimp::neighborhood::MedianParams {
        radius: radius as i32,
        neighborhood: photocraft_algo::gimp::neighborhood::MedianNeighborhood::Circle,
        percentile: 50.,
        alpha_percentile: 50.,
        high_precision: node.arguments.iter().any(|a| a.name == "high-precision" && a.value == FilterValue::Bool(true)),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::{FilterId, ItemRef, MaskId},
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node() -> FilterNode {
        let arg = |name: &str, value| FilterArgument { name: name.into(), value };
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:median-blur".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() },
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(MaskId(7))),
            auxiliary: vec![],
            arguments: vec![
                arg("radius", FilterValue::Number(ExactNumber::Signed { width: 32, value: 3 })),
                arg("neighborhood", FilterValue::Enum { domain: "gegl:median-blur::neighborhood".into(), value: 1 }),
                arg("percentile", FilterValue::Number(ExactNumber::Float32Bits(50f32.to_bits()))),
                arg("alpha-percentile", FilterValue::Number(ExactNumber::Float32Bits(50f32.to_bits()))),
                arg("abyss-policy", FilterValue::Enum { domain: "gegl:median-blur::abyss-policy".into(), value: 1 }),
                arg("high-precision", FilterValue::Bool(false)),
            ],
            composition: Some(GimpComposition {
                source_mode: 62,
                stored_blend_space: -261651887,
                stored_composite_space: -261651887,
                stored_operator: -261651887,
                legacy: false,
                pass_through: false,
                compatibility_epoch: "observed".into(),
                resolved: None,
            }),
        }
    }
    #[test]
    fn rejects_duplicate_missing_wrong_type_and_unqualified_activation() {
        let n = node();
        assert_eq!(radius(&n, 22, Rect::new(0, 0, 32, 32)).unwrap(), 3);
        for variant in 0..5 {
            let mut bad = n.clone();
            match variant {
                0 => bad.arguments[1] = bad.arguments[0].clone(),
                1 => {
                    bad.arguments.pop();
                }
                2 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(3f32.to_bits())),
                3 => bad.arguments[5].value = FilterValue::Bool(true),
                _ => bad.source_region = None,
            }
            assert!(radius(&bad, 22, Rect::new(0, 0, 32, 32)).is_err());
        }
        assert!(radius(&n, 23, Rect::new(0, 0, 32, 32)).is_err());
        assert!(radius(&n, 22, Rect::new(0, 0, 16, 12)).is_err());
    }
}
