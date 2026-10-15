//! Separately qualified literal RGB/alpha/HDR finite Gaussian member.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianParams};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
pub(crate) const EPOCH: &str = "xcf22-rgb-alpha-finite-gaussian-v1";
pub(crate) fn parameters(node: &FilterNode, version: u32) -> Result<(GaussianParams, GaussianFilter)> {
    parameters_mode(node, version, false)
}
/// Owned CURRENT metadata validation; activation does not erase typed arguments.
pub(crate) fn parameters_for_owned(node: &FilterNode, version: u32) -> Result<(GaussianParams, GaussianFilter)> {
    parameters_mode(node, version, true)
}
fn parameters_mode(node: &FilterNode, version: u32, owned: bool) -> Result<(GaussianParams, GaussianFilter)> {
    let c = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing finite Gaussian policy"))?;
    if version != 22
        || node.operation.name != "gegl:gaussian-blur"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
        || (!owned && !node.enabled)
        || node.opacity_f64_bits != 1.0f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || node.arguments.len() != 5
        || c.source_mode != 62
        || c.legacy
        || c.pass_through
        || c.resolved.is_some()
        || [c.stored_blend_space, c.stored_composite_space, c.stored_operator] != [-1474592175; 3]
    {
        return Err(CompositionError::Unsupported("finite Gaussian raster envelope"));
    }
    let get = |name: &str| -> Result<&FilterValue> {
        let mut args = node.arguments.iter().filter(|a| a.name == name);
        let value = &args.next().ok_or(CompositionError::Unsupported("finite Gaussian argument absent"))?.value;
        if args.next().is_some() {
            return Err(CompositionError::Invalid("duplicate finite Gaussian argument"));
        }
        Ok(value)
    };
    let number = |name| -> Result<f64> {
        match get(name)? {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) => Ok(f64::from(f32::from_bits(*bits))),
            _ => Err(CompositionError::Unsupported("finite Gaussian numeric type")),
        }
    };
    let x = number("std-dev-x")?;
    let y = number("std-dev-y")?;
    let filter = match get("filter")? {
        FilterValue::Enum { domain, value: 0 } if domain == "gegl:gaussian-blur::filter" && x == 2.25 && y == f64::from(1.3f32) => GaussianFilter::Auto,
        FilterValue::Enum { domain, value: 1 } if domain == "gegl:gaussian-blur::filter" && x == 3.75 && y == f64::from(0.8f32) => GaussianFilter::Fir,
        _ => return Err(CompositionError::Unsupported("finite Gaussian algorithm/parameter pair")),
    };
    if !matches!(get("abyss-policy")?,FilterValue::Enum{domain,value:1} if domain=="gegl:gaussian-blur::abyss-policy")
        || get("clip-extent")? != &FilterValue::Bool(true)
    {
        return Err(CompositionError::Unsupported("finite Gaussian Clamp/clip policy"));
    }
    Ok((GaussianParams { std_dev_x: x, std_dev_y: y, abyss: GaussianAbyss::Clamp, clip_extent: true }, filter))
}
