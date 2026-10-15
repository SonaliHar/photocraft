//! Exact authored Linear Motion Blur envelope and checked operation-bound work estimate.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
use photocraft_geom::Rect;
pub(crate) const EPOCH: &str = "xcf22-rgb-u8-linear-motion-v1";
pub(crate) fn parameters(node: &FilterNode, version: u32, extent: Rect) -> Result<photocraft_algo::gimp::neighborhood::MotionBlurParams> {
    parameters_mode(node, version, extent, false)
}
/// Owned CURRENT metadata validation; activation does not erase typed arguments.
pub(crate) fn parameters_for_owned(node: &FilterNode, version: u32, extent: Rect) -> Result<photocraft_algo::gimp::neighborhood::MotionBlurParams> {
    parameters_mode(node, version, extent, true)
}
fn parameters_mode(node: &FilterNode, version: u32, extent: Rect, owned: bool) -> Result<photocraft_algo::gimp::neighborhood::MotionBlurParams> {
    let c = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing Linear Motion Blur policy"))?;
    if extent != Rect::new(0, 0, 32, 32) {
        return Err(CompositionError::Unsupported("Motion source extent"));
    }
    let raw = -261651887;
    if version != 22
        || node.operation.name != "gegl:motion-blur-linear"
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
        || node.arguments.len() != 2
    {
        return Err(CompositionError::Unsupported("Linear Motion Blur Replace envelope"));
    }
    let mut length = None;
    let mut angle = false;
    for arg in &node.arguments {
        match (arg.name.as_str(), &arg.value) {
            ("length", FilterValue::Number(ExactNumber::Float32Bits(bits))) if length.is_none() && matches!(*bits, 0x41200000 | 0x41220000) => {
                length = Some(f64::from(f32::from_bits(*bits)))
            }
            ("angle", FilterValue::Number(ExactNumber::Float32Bits(0))) if !angle => angle = true,
            _ => return Err(CompositionError::Unsupported("Motion measured argument domain")),
        }
    }
    if !angle {
        return Err(CompositionError::Unsupported("Motion angle absent"));
    }
    Ok(photocraft_algo::gimp::neighborhood::MotionBlurParams {
        length: length.ok_or(CompositionError::Unsupported("Motion length absent"))?,
        angle_degrees: 0.,
    })
}
