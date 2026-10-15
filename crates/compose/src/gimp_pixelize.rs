//! Exact authored Pixelize Square envelope and checked operation-bound work estimate.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
use photocraft_geom::Rect;
pub(crate) const EPOCH: &str = "xcf22-rgb-u8-square-pixelize-v1";
pub(crate) fn parameters(node: &FilterNode, version: u32, extent: Rect) -> Result<photocraft_algo::gimp::neighborhood::PixelizeParams> {
    parameters_mode(node, version, extent, false)
}
/// Owned CURRENT metadata validation; activation does not erase typed arguments.
pub(crate) fn parameters_for_owned(node: &FilterNode, version: u32, extent: Rect) -> Result<photocraft_algo::gimp::neighborhood::PixelizeParams> {
    parameters_mode(node, version, extent, true)
}
fn parameters_mode(node: &FilterNode, version: u32, extent: Rect, owned: bool) -> Result<photocraft_algo::gimp::neighborhood::PixelizeParams> {
    let c = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing Pixelize Square policy"))?;
    if extent != Rect::new(0, 0, 32, 32) {
        return Err(CompositionError::Unsupported("Pixelize source extent"));
    }
    let raw = -261651887;
    if version != 22
        || node.operation.name != "gegl:pixelize"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
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
        || node.arguments.len() != 8
    {
        return Err(CompositionError::Unsupported("Pixelize Square Replace envelope"));
    }
    let mut size = None;
    let mut seen = 0u16;
    for arg in &node.arguments {
        let bit = match (arg.name.as_str(), &arg.value) {
            ("norm", FilterValue::Enum { domain, value: 2 }) if domain == "gegl:pixelize::norm" => 1,
            ("size-x", FilterValue::Number(ExactNumber::Signed { width: 32, value })) if matches!(*value, 16 | 17) => {
                size = Some(*value as u32);
                2
            }
            ("size-y", FilterValue::Number(ExactNumber::Signed { width: 32, value: 16 })) => 4,
            ("offset-x", FilterValue::Number(ExactNumber::Signed { width: 32, value: 0 })) => 8,
            ("offset-y", FilterValue::Number(ExactNumber::Signed { width: 32, value: 0 })) => 16,
            ("ratio-x", FilterValue::Number(ExactNumber::Float32Bits(0x3f800000))) => 32,
            ("ratio-y", FilterValue::Number(ExactNumber::Float32Bits(0x3f800000))) => 64,
            ("background", FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None })
                if components.len() == 4 && components.iter().all(|v| *v == ExactNumber::Float32Bits(0x3f800000)) =>
            {
                128
            }
            _ => return Err(CompositionError::Unsupported("Pixelize measured argument domain")),
        };
        if seen & bit != 0 {
            return Err(CompositionError::Invalid("duplicate Pixelize argument"));
        }
        seen |= bit;
    }
    if seen != 255 {
        return Err(CompositionError::Unsupported("Pixelize missing argument"));
    }
    Ok(photocraft_algo::gimp::neighborhood::PixelizeParams {
        size_x: size.ok_or(CompositionError::Unsupported("Pixelize width absent"))?,
        size_y: 16,
        offset_x: 0,
        offset_y: 0,
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
            operation: OperationIdentity { name: "gegl:pixelize".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() },
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(MaskId(7))),
            auxiliary: vec![],
            arguments: vec![
                arg("norm", FilterValue::Enum { domain: "gegl:pixelize::norm".into(), value: 2 }),
                arg("size-x", FilterValue::Number(ExactNumber::Signed { width: 32, value: 16 })),
                arg("size-y", FilterValue::Number(ExactNumber::Signed { width: 32, value: 16 })),
                arg("offset-x", FilterValue::Number(ExactNumber::Signed { width: 32, value: 0 })),
                arg("offset-y", FilterValue::Number(ExactNumber::Signed { width: 32, value: 0 })),
                arg("ratio-x", FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits()))),
                arg("ratio-y", FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits()))),
                arg(
                    "background",
                    FilterValue::Color {
                        components: vec![ExactNumber::Float32Bits(1f32.to_bits()); 4],
                        encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
                        profile: None,
                    },
                ),
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
    fn rejects_duplicate_missing_wrong_type_and_partial_ratio() {
        let n = node();
        assert_eq!(parameters(&n, 22, Rect::new(0, 0, 32, 32)).unwrap().size_x, 16);
        for variant in 0..5 {
            let mut bad = n.clone();
            match variant {
                0 => bad.arguments[1] = bad.arguments[0].clone(),
                1 => {
                    bad.arguments.pop();
                }
                2 => bad.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(16f32.to_bits())),
                3 => bad.arguments[5].value = FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())),
                _ => bad.source_region = None,
            }
            assert!(parameters(&bad, 22, Rect::new(0, 0, 32, 32)).is_err());
        }
        assert!(parameters(&n, 23, Rect::new(0, 0, 32, 32)).is_err());
        assert!(parameters(&n, 22, Rect::new(0, 0, 16, 12)).is_err());
    }
}
