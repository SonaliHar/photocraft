//! Closed native RGB Float Linear Unpremultiply consumer; admission is caller-owned.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_unpremultiply::Unpremultiply;
use photocraft_cms::{Builtin, Profile};
use photocraft_color::PixelFormat;
use photocraft_doc::{SampleEncoding, xcf_filters::FilterArgument};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError};
pub const UNPREMULTIPLY_EPOCH: &str = "xcf-22-stock-unpremultiply-rgb-f32-linear-v1";
pub fn typed_arguments(arguments: &[FilterArgument]) -> Result<Unpremultiply> {
    if !arguments.is_empty() {
        return Err(CompositionError::Unsupported("Unpremultiply has no stored arguments"));
    }
    Ok(Unpremultiply)
}
pub(crate) fn canonical_candidate(node: &photocraft_doc::xcf_filters::FilterNode, profile: &Profile, stored: SampleEncoding, owner: Rect, version: u32) -> Result<UnpremultiplyCandidate> {
    if version != 22 || node.operation.name != "gegl:unpremultiply" || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != UNPREMULTIPLY_EPOCH || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0) || node.source_region != Some(0) || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || node.composition.as_ref().is_none_or(|p| p.compatibility_epoch != UNPREMULTIPLY_EPOCH
            || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some()
            || (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (-1877441967, -1877441967, -1877441967)) {
        return Err(CompositionError::Unsupported("Unpremultiply canonical singleton envelope"));
    }
    typed_arguments(&node.arguments)?;
    UnpremultiplyCandidate::new(profile, stored, PixelFormat::RGBA32F, owner)
}
#[derive(Clone, Copy, Debug)]
pub struct UnpremultiplyCandidate;
impl UnpremultiplyCandidate {
    pub fn new(profile: &Profile, encoding: SampleEncoding, format: PixelFormat, owner: Rect) -> Result<Self> {
        if profile != Builtin::Srgb.profile() || encoding != SampleEncoding::Linear || format != PixelFormat::RGBA32F || owner != Rect::new(0, 0, 8, 6) {
            return Err(CompositionError::Unsupported("Unpremultiply native RGB Float Linear extent/profile"));
        }
        Ok(Self)
    }
    pub fn working_input(&self, pixel: &[f64]) -> Result<[f32; 4]> {
        let native: [f64; 4] = pixel.try_into().map_err(|_| CompositionError::Invalid("Unpremultiply native pixel length"))?;
        if native.iter().any(|v| !v.is_finite() || !(*v as f32).is_finite() || f64::from(*v as f32).to_bits() != v.to_bits()) || !(0.0..=1.0).contains(&native[3]) {
            return Err(CompositionError::Unsupported("Unpremultiply exact finite Float colors/unit alpha"));
        }
        Ok(native.map(|v| v as f32))
    }
    pub fn apply_with(&self, pixel: &[f64], enabled: bool, interrupt: &Interrupt<'_>, work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>) -> Result<[f64; 4]> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let native = self.working_input(pixel)?;
        work(if enabled { 32 } else { 1 })?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let result = if enabled {
            let mut output = [0.; 4];
            Unpremultiply.try_eval_linear_float(&native, &mut output)?;
            output
        } else { native };
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(result.map(f64::from))
    }
}
