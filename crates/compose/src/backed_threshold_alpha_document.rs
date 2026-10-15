//! CURRENT singleton Threshold Alpha display over positive-zero transparent
//! backdrop. This is neither original-source equivalence, a raw-padding proof,
//! a profile conversion, nor an editable/persistable native output owner.
use crate::{
    backed_normal::{BackedNormalError, NormalBandLimits},
    backed_normal_stack::OwnedNormalStackBand,
    backed_point_filter::BackedPointError,
    backed_threshold_alpha_context::PreparedBackedThresholdAlphaContext,
    gimp_composite::{CompositionError, normal_over_zero_backdrop},
};
use photocraft_cms::Profile;
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64};
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};

pub struct PreparedBackedThresholdAlphaDocument<'a> {
    context: PreparedBackedThresholdAlphaContext<'a>,
    quota: BackingQuota,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}
// Payload precedes its charge throughout every failed allocation/conversion.
struct Output {
    pixels: Vec<[f64; 4]>,
    lease: QuotaLease,
}
fn invalid(message: &'static str) -> BackedNormalError {
    BackedNormalError::Invalid(message)
}
fn native_error(error: BackedPointError) -> BackedNormalError {
    match error {
        BackedPointError::Backing(e) => BackedNormalError::Backing(e),
        BackedPointError::View(e) => BackedNormalError::View(e),
        BackedPointError::Unsupported(s) => BackedNormalError::Unsupported(s),
        BackedPointError::Invalid(s) => BackedNormalError::Invalid(s),
        BackedPointError::Sample(_) => invalid("Threshold Alpha native sample decode"),
        BackedPointError::Operation(e) => BackedNormalError::Color(CompositionError::Raster(e)),
    }
}
impl<'a> PreparedBackedThresholdAlphaDocument<'a> {
    /// Requires the actual readonly current model and its exact retained profile.
    /// The context validates the singleton, native layout, physical empty
    /// selection, visible full-opacity layer and Normal/Union current policy.
    /// This bridge prepares no CMS pipeline: the existing transparent-backdrop
    /// identity uses source color samples directly in their current encoding.
    pub fn new(owner: &'a AccountedDocument, profile: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let header = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(|| invalid("Threshold Alpha document header"))?;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(BackedNormalError::Quota)?;
        let context = PreparedBackedThresholdAlphaContext::new(owner, profile, limits, control)?;
        let result = Self { context, quota: owner.quota().clone(), limits, _lease: lease };
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(result)
    }
    pub fn bounds(&self) -> Rect {
        self.context.descriptor().bounds()
    }
    /// Contained operative ROI only. Source native output and all its genuine
    /// leases remain held while the separately charged straight RGBA64 display
    /// vector is built. Gray repeats the stored component; only the existing
    /// Normal transparent-zero policy discards hidden colors when alpha is zero.
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNormalStackBand, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(|| invalid("Threshold Alpha display band size"))?;
        if rect.is_empty() || !self.bounds().contains_rect(&rect) || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(invalid("Threshold Alpha display ROI outside operative band limits"));
        }
        let format = self.context.descriptor().format();
        if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || !format.alpha
            || !matches!(format.sample, SampleType::U16 | SampleType::F32 | SampleType::F64)
        {
            return Err(BackedNormalError::Unsupported("Threshold Alpha display native layout"));
        }
        let count = usize::try_from(count).map_err(|_| invalid("Threshold Alpha display band address space"))?;
        let capacity_limit = count.checked_add(2).ok_or_else(|| invalid("Threshold Alpha display capacity"))?;
        let header = std::mem::size_of::<OwnedNormalStackBand>()
            .checked_add(std::mem::size_of::<Output>())
            .and_then(|n| n.checked_add(128))
            .ok_or_else(|| invalid("Threshold Alpha display header"))?;
        let envelope = capacity_limit
            .checked_mul(std::mem::size_of::<[f64; 4]>())
            .and_then(|n| n.checked_add(header))
            .ok_or_else(|| invalid("Threshold Alpha display capacity"))?;
        let lease = self.quota.reserve(QuotaKind::Resident, envelope as u64).map_err(BackedNormalError::Quota)?;
        let mut output = Output { pixels: Vec::new(), lease };
        // Stack decode/composition values and control/iterator staging are paid
        // in Resident, as elsewhere in the scratch-free native band path.
        let _scratch = self.quota.reserve(QuotaKind::Resident, 256).map_err(BackedNormalError::Quota)?;
        let native = self.context.render_band(rect, control).map_err(native_error)?;
        if native.rect() != rect
            || native.format() != format
            || native.bytes().len() != count.checked_mul(format.bytes_per_pixel()).ok_or_else(|| invalid("Threshold Alpha native band length"))?
        {
            return Err(invalid("Threshold Alpha native band mismatch"));
        }
        control.check(1).map_err(BackedNormalError::Backing)?;
        output.pixels.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
        if output.pixels.capacity() > capacity_limit {
            return Err(BackedNormalError::Allocation);
        }
        let actual = output
            .pixels
            .capacity()
            .checked_mul(std::mem::size_of::<[f64; 4]>())
            .and_then(|n| n.checked_add(header))
            .ok_or_else(|| invalid("Threshold Alpha actual display capacity"))?;
        output.lease.reconcile_capacity(actual as u64).map_err(BackedNormalError::Quota)?;
        for pixel in native.bytes().chunks_exact(format.bytes_per_pixel()) {
            // Actual native decode, scalar validation, Normal zero identity and
            // one RGBA64 write. No independent per-band work allowance/reset.
            control.check(format.bytes_per_pixel() as u64 + 128).map_err(BackedNormalError::Backing)?;
            let read = |channel| try_read_sample_f64(pixel, format.sample, channel).map_err(|_| invalid("Threshold Alpha native sample decode"));
            let rgb = if format.mode == ColorMode::Grayscale { [read(0)?; 3] } else { [read(0)?, read(1)?, read(2)?] };
            let source = [rgb[0], rgb[1], rgb[2], read(format.mode.color_channels())?];
            output.pixels.push(normal_over_zero_backdrop(source, 1.0).map_err(BackedNormalError::Color)?);
        }
        if output.pixels.len() != count {
            return Err(invalid("Threshold Alpha display pixel count"));
        }
        control.check(1).map_err(BackedNormalError::Backing)?;
        let Output { pixels, lease } = output;
        Ok(OwnedNormalStackBand::from_accounted_pixels(rect, pixels, lease))
    }
}
#[cfg(test)]
#[path = "backed_threshold_alpha_document_tests.rs"]
mod tests;
