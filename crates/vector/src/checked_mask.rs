//! Additive bounded ordinary vector-mask preparation; legacy APIs remain unchanged.
use crate::{CompiledVectorMask, DEFAULT_TOLERANCE, MaskGeometry, Rasterizer};
use photocraft_doc::VectorMask;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError};
/// Per-call limits for checked geometry and raster staging. These bound known
/// payloads/work, not allocator overhead or the document's retained metadata.
#[derive(Clone, Copy, Debug)]
pub struct VectorPreparationLimits {
    pub max_bytes: usize,
    pub max_edges: usize,
    pub max_components: usize,
    pub max_work: u64,
}
impl Default for VectorPreparationLimits {
    fn default() -> Self {
        Self { max_bytes: 64 << 20, max_edges: 1 << 20, max_components: 100000, max_work: 1 << 32 }
    }
}
impl CompiledVectorMask {
    pub fn try_new_with_interrupt(mask: &VectorMask, limits: VectorPreparationLimits, control: &Interrupt<'_>) -> Result<Self, RasterInputError> {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let geometry = if !mask.enabled {
            MaskGeometry::Constant(1.0)
        } else {
            let mut empty = true;
            for subpath in &mask.path.subpaths {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
                if !subpath.knots.is_empty() {
                    empty = false;
                    break;
                }
            }
            if empty {
                MaskGeometry::Constant(if mask.path.inverted { 0.0 } else { 1.0 })
            } else {
                MaskGeometry::Path(Rasterizer::try_fill_path(&mask.path, DEFAULT_TOLERANCE, limits, control)?)
            }
        };
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(Self { geometry, density: (mask.enabled && mask.density < 1.0).then(|| mask.density.clamp(0.0, 1.0)) })
    }
    /// Bounds of prepared path geometry. Constant masks have no finite path bound.
    pub fn pixel_bounds(&self) -> Option<Rect> {
        match &self.geometry {
            MaskGeometry::Constant(_) => None,
            MaskGeometry::Path(rasterizer) => rasterizer.pixel_bounds(),
        }
    }

    /// Known retained geometry payload capacities; excludes allocator overhead.
    pub fn preparation_bytes(&self) -> usize {
        match &self.geometry {
            MaskGeometry::Constant(_) => 0,
            MaskGeometry::Path(r) => r.preparation_bytes(),
        }
    }
    pub fn try_render_with_interrupt(&self, rect: Rect, limits: VectorPreparationLimits, control: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let bytes = photocraft_raster::checked_region_byte_len(
            rect,
            photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false),
        )?;
        let density_work = if self.density.is_some() { (bytes / 4) as u64 } else { 0 };
        let limits = VectorPreparationLimits { max_work: limits.max_work.checked_sub(density_work).ok_or(RasterInputError::LimitExceeded)?, ..limits };
        let mut result = match &self.geometry {
            MaskGeometry::Path(r) => r.try_render_checked(rect, limits, control)?,
            MaskGeometry::Constant(value) => {
                let bytes = photocraft_raster::checked_region_byte_len(
                    rect,
                    photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false),
                )?;
                if bytes > limits.max_bytes || (bytes / 4) as u64 > limits.max_work {
                    return Err(RasterInputError::LimitExceeded);
                }
                let mut values = Vec::new();
                values.try_reserve_exact(bytes / 4).map_err(|_| RasterInputError::AllocationFailed)?;
                if values.capacity().checked_mul(4).is_none_or(|n| n > limits.max_bytes) {
                    return Err(RasterInputError::LimitExceeded);
                }
                while values.len() < bytes / 4 {
                    control.check().map_err(|_| RasterInputError::Cancelled)?;
                    values.resize(values.len().saturating_add(256).min(bytes / 4), *value);
                }
                values
            }
        };
        if let Some(d) = self.density {
            for values in result.chunks_mut(256) {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
                for value in values {
                    *value = 1.0 - d * (1.0 - *value);
                }
            }
        }
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Path, Subpath};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn checked_compile_cancels_inside_long_path_and_preserves_source() {
        let points: Vec<_> = (0..10000).map(|i| (i as f64, ((i * 73) % 101) as f64)).collect();
        let mask = VectorMask::new(Path::new(vec![Subpath::polygon(&points)]));
        let before = mask.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        assert!(matches!(
            CompiledVectorMask::try_new_with_interrupt(&mask, VectorPreparationLimits::default(), &Interrupt::cancel_only(&cancel)),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(mask, before);
    }
    #[test]
    fn checked_compile_rejects_zero_budget_before_returning_geometry() {
        let mask = VectorMask::new(Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])]));
        let before = mask.clone();
        assert!(matches!(
            CompiledVectorMask::try_new_with_interrupt(&mask, VectorPreparationLimits { max_bytes: 0, ..Default::default() }, &Interrupt::NONE),
            Err(RasterInputError::LimitExceeded)
        ));
        assert_eq!(mask, before);
    }
    #[test]
    fn checked_render_cancels_inside_scanline_work() {
        let mask = VectorMask::new(Path::new(vec![Subpath::polygon(&[(0.1, 0.7), (1023.9, 240.3), (15.2, 255.7)])]));
        let compiled = CompiledVectorMask::try_new_with_interrupt(&mask, VectorPreparationLimits::default(), &Interrupt::NONE).unwrap();
        let before = mask.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        assert!(matches!(
            compiled.try_render_with_interrupt(Rect::new(0, 0, 1024, 256), VectorPreparationLimits::default(), &Interrupt::cancel_only(&cancel)),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(mask, before);
    }
    #[test]
    fn checked_curves_boolean_components_density_and_negative_origins_are_exact() {
        use photocraft_doc::{FillRule, PathOp};
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            for inverted in [false, true] {
                let mut path = crate::shapes::ellipse(-13.7, -5.2, 110.3, 93.7);
                path.subpaths.push(Subpath::polygon(&[(10.2, 13.3), (97.8, 4.9), (48.1, 80.7)]).with_op(PathOp::Join));
                path.subpaths.push(Subpath::polygon(&[(23.5, 20.9), (75.1, 23.8), (70.3, 51.6), (22.7, 61.4)]).with_op(PathOp::Subtract));
                path.subpaths.push(Subpath::polygon(&[(-8.1, -7.3), (101.6, 17.9), (66.7, 99.2)]).with_op(PathOp::Intersect));
                path.subpaths.push(Subpath::polygon(&[(41.3, 40.5), (65.8, 11.2), (91.2, 53.4)]).with_op(PathOp::Exclude));
                path.fill_rule = rule;
                path.inverted = inverted;
                let mut mask = VectorMask::new(path);
                mask.density = 0.37;
                let before = mask.clone();
                let legacy = CompiledVectorMask::new(&mask);
                let checked = CompiledVectorMask::try_new_with_interrupt(&mask, VectorPreparationLimits::default(), &Interrupt::NONE).unwrap();
                for rect in [Rect::new(-21, -13, 110, 101), Rect::new(11, 7, 49, 53), Rect::new(-11, -7, -1, 1)] {
                    assert_eq!(
                        checked.try_render_with_interrupt(rect, VectorPreparationLimits::default(), &Interrupt::NONE).unwrap(),
                        legacy.render(rect),
                        "{rule:?} invert{inverted} rect{rect:?}"
                    );
                }
                assert!(matches!(
                    checked.try_render_with_interrupt(
                        Rect::new(-21, -13, 110, 101),
                        VectorPreparationLimits { max_work: 1, ..Default::default() },
                        &Interrupt::NONE
                    ),
                    Err(RasterInputError::LimitExceeded)
                ));
                assert_eq!(mask, before);
            }
        }
    }
    #[test]
    fn checked_constant_masks_bounds_and_payload_limits_are_exact() {
        for enabled in [false, true] {
            for inverted in [false, true] {
                let mut mask = VectorMask::new(Path::default());
                mask.enabled = enabled;
                mask.path.inverted = inverted;
                mask.density = 0.13;
                let legacy = CompiledVectorMask::new(&mask);
                let checked = CompiledVectorMask::try_new_with_interrupt(&mask, VectorPreparationLimits::default(), &Interrupt::NONE).unwrap();
                let rect = Rect::new(-4, 3, 27, 17);
                assert_eq!(checked.try_render_with_interrupt(rect, VectorPreparationLimits::default(), &Interrupt::NONE).unwrap(), legacy.render(rect));
                assert!(matches!(
                    checked.try_render_with_interrupt(rect, VectorPreparationLimits { max_bytes: 0, ..Default::default() }, &Interrupt::NONE),
                    Err(RasterInputError::LimitExceeded)
                ));
                assert!(matches!(
                    checked.try_render_with_interrupt(Rect::new(i32::MIN, 0, i32::MAX, 2), VectorPreparationLimits::default(), &Interrupt::NONE),
                    Err(RasterInputError::InvalidBounds)
                ));
            }
        }
    }
}
