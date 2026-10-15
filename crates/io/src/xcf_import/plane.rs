//! Exact native-plane access. Eager source bytes remain owned until cache transfer/refund.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParsedPlaneDescriptor {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub sample: Sample,
}

pub(crate) trait ExactParsedPlane {
    fn descriptor(&self) -> ParsedPlaneDescriptor;
    /// Exact retained declared native byte count, without visiting samples.
    /// Staged producers have already checked this shape before construction.
    fn native_byte_len(&self) -> Option<u64> {
        let d = self.descriptor();
        u64::from(d.width).checked_mul(u64::from(d.height))?.checked_mul(d.channels as u64)?.checked_mul(d.sample.source_bytes() as u64)
    }

    /// Declared pixels in row-major order, at most 64 pixels per callback.
    /// No conversion or default substitution is allowed on a failed read.
    fn visit_native(&self, ctl: &Interrupt<'_>, visit: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError>;
}

impl ExactParsedPlane for Plane {
    fn native_byte_len(&self) -> Option<u64> { Some(self.bytes.len() as u64) }
    fn descriptor(&self) -> ParsedPlaneDescriptor {
        ParsedPlaneDescriptor { width: self.width, height: self.height, channels: self.channels, sample: self.sample }
    }
    fn visit_native(&self, ctl: &Interrupt<'_>, visit: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        let descriptor = self.descriptor();
        let stride = descriptor
            .channels
            .checked_mul(descriptor.sample.source_bytes())
            .filter(|n| *n != 0)
            .ok_or_else(|| IoError::Unsupported("XCF exact plane stride".into()))?;
        let expected = (descriptor.width as usize)
            .checked_mul(descriptor.height as usize)
            .and_then(|n| n.checked_mul(stride))
            .ok_or_else(|| IoError::Unsupported("XCF exact plane size overflow".into()))?;
        if self.bytes.len() != expected {
            return Err(IoError::Unsupported("XCF exact plane byte length".into()));
        }
        let block = stride.checked_mul(64).ok_or_else(|| IoError::Unsupported("XCF exact plane block overflow".into()))?;
        ctl.check().map_err(|_| IoError::Cancelled)?;
        for bytes in self.bytes.chunks(block) {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            visit(bytes)?;
        }
        Ok(())
    }
}

/// Private mapper dispatch. Unsupported storage families never become dummy eager planes.
pub(super) trait MappingPlane: ExactParsedPlane + Sized {
    fn requires_budgeted_rendering_preparation() -> bool { false }
    /// Dynamic selection is private to a sealed source-selected plane. The
    /// argument comes from the mapper's already admitted original-source
    /// domain, never a caller flag or current operation-name guess.
    fn requires_budgeted_rendering_preparation_for(_threshold_alpha_domain: bool) -> bool {
        Self::requires_budgeted_rendering_preparation()
    }
    /// Only an already admitted original Point domain can select this paid path.
    fn requires_budgeted_point_preparation(_point_domain: bool) -> bool { false }
    fn requires_budgeted_neighborhood_preparation(_domain: bool) -> bool { false }
    fn requires_budgeted_gaussian_group_preparation(_domain: bool) -> bool { false }
    fn requires_budgeted_alpha_separable_preparation(_domain: bool) -> bool { false }
    fn requires_budgeted_component_separable_preparation(_domain: bool) -> bool { false }
    fn requires_budgeted_legacy_separable_preparation(_domain: bool) -> bool { false }
    fn requires_budgeted_separable_preparation(_domain: bool) -> bool { false }
    fn requires_budgeted_general_separable_preparation(_domain: bool) -> bool { false }
    fn effective_profile(file: &XcfFile<Self>, space: photocraft_cms::ColorSpace, ce: photocraft_cms::xcf_color::Encoding, _: &Interrupt<'_>, _: Option<&photocraft_xcf::Budget<'_>>) -> Result<photocraft_cms::Profile, IoError> {
        photocraft_cms::xcf_color::effective_profile(if file.icc.is_empty() {None}else{Some(&file.icc)},space,ce).map_err(|e|IoError::Unsupported(format!("XCF source profile/encoding: {e}")))
    }
    fn admissions(file: &XcfFile<Self>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<admission::AdmissionDomains, IoError>;
    fn late_admissions(file: &XcfFile<Self>) -> admission::LateAdmissionDomains;
    fn eager_original(&self) -> Result<&Plane, IoError>;
    fn into_original(self) -> Result<Plane, IoError>;
    fn retained_source_bytes(&self) -> usize;
    /// Construct or transfer the exact current native Gray mask. Caller has
    /// already checked source eligibility; this hook grants no additional domain.
    /// A staged implementation moves its retained Surface without materializing
    /// a Plane or changing sample words/defaults. Original storage fees stay live.
    /// Ordinary local-zero mask placement, independently validated by the mapper.
    fn current_layer_mask_surface(&mut self, rect: Rect, budget: &mut Budget<'_>, ctl: &Interrupt<'_>) -> Result<Surface, IoError> {
        let p = self.eager_original()?;
        surface(budget, p, rect, ColorMode::Grayscale, false, &p.bytes, ctl)
    }
    fn current_filter_mask_surface(&mut self, rect: Rect, budget: &mut Budget<'_>, ctl: &Interrupt<'_>) -> Result<Surface, IoError>;
    fn current_owner(
        owner: Drawable<Self>,
        palette: &[[u8; 3]],
        budget: &mut Budget<'_>,
        ctl: &Interrupt<'_>,
    ) -> Result<(Drawable<Self>, Layer, Option<IndexedSource>), IoError>;
    fn legacy_point_candidate(owner: &Drawable<Self>, point: bool, levels: bool, canvas: Size) -> Result<bool, IoError>;
}
impl MappingPlane for Plane {
    fn admissions(file: &XcfFile, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<admission::AdmissionDomains, IoError> {
        admission::classify(file, ctl, budget)
    }
    fn late_admissions(file: &XcfFile) -> admission::LateAdmissionDomains {
        admission::classify_late(file)
    }
    fn eager_original(&self) -> Result<&Plane, IoError> {
        Ok(self)
    }
    fn into_original(self) -> Result<Plane, IoError> {
        Ok(self)
    }
    fn retained_source_bytes(&self) -> usize {
        self.bytes.len()
    }
    fn current_filter_mask_surface(&mut self, rect: Rect, budget: &mut Budget<'_>, ctl: &Interrupt<'_>) -> Result<Surface, IoError> {
        surface(budget, self, rect, ColorMode::Grayscale, false, &self.bytes, ctl)
    }
    fn current_owner(
        owner: Drawable,
        palette: &[[u8; 3]],
        budget: &mut Budget<'_>,
        ctl: &Interrupt<'_>,
    ) -> Result<(Drawable, Layer, Option<IndexedSource>), IoError> {
        into_current_owner(owner, palette, budget, ctl)
    }
    fn legacy_point_candidate(d: &Drawable, point_context: bool, levels_context: bool, canvas: Size) -> Result<bool, IoError> {
        Ok((point_context || levels_context && d.effects.len() == 1 && d.effects.iter().all(|effect| effect.operation == "gimp:levels"))
            && d.model == Model::Rgb
            && !d.group
            && d.mask.is_none()
            && d.plane.as_ref().is_some_and(|plane| {
                plane.channels > 0
                    && (!d.alpha
                        || plane.bytes.chunks_exact(plane.channels).all(|pixel| pixel.last() == Some(&255))
                        || canvas == Size::new(12, 9)
                            && bounds(d).ok() == Some(Rect::new(0, 0, 12, 9))
                            && d.effects.iter().all(|effect| matches!(effect.operation.as_str(), "gimp:curves" | "gimp:levels")))
            }))
    }
}

/// Transfers owner storage without cloning its authoritative Plane. The eager
/// adapter retains original bytes for source-cache mapping and the existing
/// final refund; a staged adapter will instead transfer its live Surface lease.
pub(super) fn into_current_owner(
    owner: Drawable,
    palette: &[[u8; 3]],
    budget: &mut Budget<'_>,
    ctl: &Interrupt<'_>,
) -> Result<(Drawable, Layer, Option<IndexedSource>), IoError> {
    let (layer, side) = if owner.group {
        (Layer::group(&owner.name, Vec::new()), None)
    } else {
        let original = owner.plane.as_ref().ok_or_else(|| unsupported(&owner, "missing raster pixels"))?;
        let (surface, side) = raster(&owner, original, palette, budget, ctl)?;
        (Layer::new(&owner.name, LayerContent::Raster(surface)), side)
    };
    Ok((owner, layer, side))
}

#[cfg(test)]
pub(super) use crate::xcf_fixture as fixture;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_transfer_preserves_original_allocation_and_current_bytes() {
        let pixels = [10, 20, 30, 0, 40, 50, 60, 255];
        let source = fixture::image(12, 150, 0, 2, 1, &pixels);
        let mut parsed = photocraft_xcf::decode(&source, &Default::default()).unwrap();
        let owner = parsed.layers.remove(0);
        let original = owner.plane.as_ref().unwrap();
        let pointer = original.bytes.as_ptr();
        let exact = original.bytes.clone();
        let mut budget = Budget { used: parsed.charged_bytes, max: 16 << 20, peak: parsed.charged_bytes, shared: None };
        let (owner, layer, side) = into_current_owner(owner, &parsed.palette, &mut budget, &Interrupt::NONE).unwrap();
        assert!(side.is_none());
        assert_eq!(owner.plane.as_ref().unwrap().bytes.as_ptr(), pointer);
        assert_eq!(owner.plane.as_ref().unwrap().bytes, exact);
        let surface = layer.surface().unwrap();
        let mut output = [0; 4];
        surface.try_read_pixel_bytes(0, 0, &mut output).unwrap();
        assert_eq!(output, pixels[..4]);
        surface.try_read_pixel_bytes(1, 0, &mut output).unwrap();
        assert_eq!(output, pixels[4..]);
    }
    #[test]
    fn exact_visits_preserve_wide_bits_and_bound_blocks() {
        for sample in [Sample::U8, Sample::U16, Sample::U32, Sample::F32, Sample::F64] {
            let bytes: Vec<u8> = (0..130 * 4 * sample.source_bytes()).map(|i| (i % 251) as u8).collect();
            let plane = Plane { width: 130, height: 1, channels: 4, sample, bytes: bytes.clone() };
            let mut result = Vec::new();
            let mut calls = 0;
            plane
                .visit_native(&Interrupt::NONE, &mut |chunk| {
                    assert!(chunk.len() <= 64 * 4 * sample.source_bytes());
                    result.extend_from_slice(chunk);
                    calls += 1;
                    Ok(())
                })
                .unwrap();
            assert_eq!(calls, 3);
            assert_eq!(result, bytes);
            assert_eq!(plane.bytes, bytes);
        }
    }
    #[test]
    fn late_visit_error_and_cancellation_preserve_original() {
        let plane = Plane { width: 130, height: 1, channels: 4, sample: Sample::U8, bytes: vec![7; 520] };
        let mut calls = 0;
        let error = plane
            .visit_native(&Interrupt::NONE, &mut |_| {
                calls += 1;
                if calls == 2 { Err(IoError::Unsupported("late read".into())) } else { Ok(()) }
            })
            .unwrap_err();
        assert!(error.to_string().contains("late read"));
        assert_eq!(calls, 2);
        let delivered = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || delivered.load(std::sync::atomic::Ordering::Relaxed) == 1;
        let ctl = Interrupt::cancel_only(&cancelled);
        assert!(matches!(
            plane.visit_native(&ctl, &mut |_| {
                delivered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }),
            Err(IoError::Cancelled)
        ));
        assert_eq!(delivered.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(plane.bytes, vec![7; 520]);
    }
    #[test]
    fn malformed_and_cancelled_planes_never_deliver_pixels() {
        let mut plane = Plane { width: 1, height: 1, channels: 4, sample: Sample::U8, bytes: vec![0; 3] };
        let mut calls = 0;
        assert!(
            plane
                .visit_native(&Interrupt::NONE, &mut |_| {
                    calls += 1;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(calls, 0);
        plane.bytes.push(0);
        let cancelled = || true;
        let ctl = Interrupt::new(&cancelled, &|_| {});
        assert!(matches!(
            plane.visit_native(&ctl, &mut |_| {
                calls += 1;
                Ok(())
            }),
            Err(IoError::Cancelled)
        ));
        assert_eq!(calls, 0);
    }
}
