//! Exact authored Box Blur envelope and checked operation-bound work estimate.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
use photocraft_geom::Rect;
pub(crate) const EPOCH: &str = "xcf22-rgb-box-replace-v1";
pub(crate) fn radius(node: &FilterNode, version: u32, extent: Rect) -> Result<u32> {
    radius_mode(node, version, extent, false)
}
/// Owned CURRENT metadata validation; activation does not erase typed arguments.
pub(crate) fn radius_for_owned(node: &FilterNode, version: u32, extent: Rect) -> Result<u32> {
    radius_mode(node, version, extent, true)
}
fn radius_mode(node: &FilterNode, version: u32, extent: Rect, owned: bool) -> Result<u32> {
    let c = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing Box Blur policy"))?;
    let raw = match extent {
        Rect { x0: 0, y0: 0, x1: 16, y1: 12 } => -1474592175,
        Rect { x0: 0, y0: 0, x1: 32, y1: 32 } => -261651887,
        _ => return Err(CompositionError::Unsupported("Box Blur source extent")),
    };
    radius_policy(node, c, version, raw, owned)
}
/// Additive CURRENT float recipe: geometry is checked by its owned consumer,
/// while this reuses the unchanged typed/raw float policy. No source admission.
pub(crate) fn radius_for_current_float(node: &FilterNode, version: u32) -> Result<u32> {
    let c = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing Box Blur policy"))?;
    radius_policy(node, c, version, -1474592175, true)
}
fn radius_policy(node: &FilterNode, c: &photocraft_doc::import_compositing::GimpComposition, version: u32, raw: i32, owned: bool) -> Result<u32> {
    if version != 22
        || node.operation.name != "gegl:box-blur"
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
        || node.arguments.len() != 1
    {
        return Err(CompositionError::Unsupported("Box Blur Replace envelope"));
    }
    match node.arguments.first() {
        Some(argument) if argument.name == "radius" => match argument.value {
            FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 }) => Ok(4),
            FilterValue::Number(ExactNumber::Signed { width: 32, value: 5 }) => Ok(5),
            _ => Err(CompositionError::Unsupported("Box Blur signed radius domain")),
        },
        _ => Err(CompositionError::Unsupported("Box Blur radius absent")),
    }
}
pub(crate) fn work(output: Rect, radius: u32) -> Result<u64> {
    let mut work = 0u64;
    let r = u64::from(radius);
    let window = r.checked_mul(2).and_then(|v| v.checked_add(1)).ok_or(CompositionError::Invalid("Box Blur window"))?;
    for tile in output.tiles() {
        let part = tile.rect().intersect(&output);
        let w = u64::from(part.width());
        let h = u64::from(part.height());
        let units = h
            .checked_add(r * 2)
            .and_then(|h| w.checked_mul(4).and_then(|w| w.checked_add(window)).and_then(|w| h.checked_mul(w)))
            .and_then(|v| v.checked_mul(4 * 16))
            .ok_or(CompositionError::Invalid("Box Blur work"))?;
        work = work.checked_add(units).ok_or(CompositionError::Invalid("Box Blur aggregate work"))?;
    }
    Ok(work)
}
