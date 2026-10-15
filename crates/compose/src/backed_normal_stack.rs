//! Explicit bounded Normal/Union stack foundation, independent of Document routing.
use crate::backed_normal::{BackedNormalError, NormalBandLimits, NormalMask, OwnedNormalBand, PreparedBackedNormal, ResidentNormalMask};
use photocraft_cms::{ColorSpace, Profile};
use photocraft_doc::{
    BackedLayer, SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
};
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};

#[derive(Clone, Copy, Debug)]
pub struct NormalStackLayer<'a> {
    pub layer: &'a BackedLayer,
    pub visible: bool,
    pub opacity: f64,
    pub mask: Option<NormalMask<'a>>,
}
#[derive(Clone, Copy, Debug)]
pub struct ResidentNormalStackLayer<'a> {
    pub input: NormalStackLayer<'a>,
    pub mask: Option<ResidentNormalMask<'a>>,
}
/// Explicit bottom-to-top operator metadata. The caller retains the actual
/// native/mask owners; this is no Document or import qualification.
#[derive(Clone, Copy, Debug)]
pub struct NormalStackOperatorLayer<'a> {
    pub input: ResidentNormalStackLayer<'a>,
    pub operator: GimpOperator,
}
#[derive(Clone, Copy, Debug)]
pub struct NormalStackLimits {
    pub band: NormalBandLimits,
    pub max_layers: usize,
}
impl Default for NormalStackLimits {
    fn default() -> Self {
        Self { band: NormalBandLimits::default(), max_layers: 10_000 }
    }
}
#[derive(Debug)]
struct Entry<'a> {
    prepared: PreparedBackedNormal<'a>,
    opacity: f64,
    mask: Option<NormalMask<'a>>,
    resident_mask: Option<ResidentNormalMask<'a>>,
}
/// Preparation borrows immutable source/mask owners and retains only accounted
/// bounded metadata/profile preparation. No operation callback is retained.
#[derive(Debug)]
pub struct PreparedBackedNormalStack<'a> {
    entries: Vec<Entry<'a>>,
    quota: BackingQuota,
    limits: NormalStackLimits,
    gray: bool,
    _lease: QuotaLease,
}
/// Complete band only: a failed later layer never exposes an earlier partial result.
#[derive(Debug)]
pub struct OwnedNormalStackBand {
    rect: Rect,
    pixels: StackPixels,
}
#[derive(Debug)]
enum StackPixels {
    Layer(OwnedNormalBand),
    Backdrop { pixels: Vec<[f64; 4]>, _lease: QuotaLease },
}
impl OwnedNormalStackBand {
    pub(crate) fn from_accounted_pixels(rect: Rect, pixels: Vec<[f64; 4]>, lease: QuotaLease) -> Self {
        Self { rect, pixels: StackPixels::Backdrop { pixels, _lease: lease } }
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn pixels(&self) -> &[[f64; 4]] {
        match &self.pixels {
            StackPixels::Layer(band) => band.pixels(),
            StackPixels::Backdrop { pixels, .. } => pixels,
        }
    }
}
impl<'a> PreparedBackedNormalStack<'a> {
    /// Inputs are bottom-to-top, in one explicit current profile/encoding/space.
    /// Only Normal/Union is represented; masks are numeric Gray coverage, without
    /// density/feather/profile conversion. Invisible entries are unused; this API
    /// does not assess hidden semantics or qualify a Document import.
    pub fn new(
        layers: &[NormalStackLayer<'a>],
        quota: &BackingQuota,
        profile: &Profile,
        stored: SampleEncoding,
        composite: GimpSpace,
        limits: NormalStackLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedNormalError> {
        Self::prepare(
            layers
                .iter()
                .map(|input| NormalStackOperatorLayer { input: ResidentNormalStackLayer { input: *input, mask: None }, operator: GimpOperator::Union }),
            quota,
            profile,
            stored,
            composite,
            limits,
            control,
        )
    }
    pub fn new_with_resident_masks(
        layers: &[ResidentNormalStackLayer<'a>],
        quota: &BackingQuota,
        profile: &Profile,
        stored: SampleEncoding,
        composite: GimpSpace,
        limits: NormalStackLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedNormalError> {
        Self::prepare(
            layers.iter().map(|input| NormalStackOperatorLayer { input: *input, operator: GimpOperator::Union }),
            quota,
            profile,
            stored,
            composite,
            limits,
            control,
        )
    }
    /// Additive four-operator stack. Hidden layers are skipped completely;
    /// visible zero-opacity layers still apply ClipToLayer/Intersection and can
    /// clear a backdrop. All operator math belongs to the prepared leaf.
    pub fn new_with_operators(
        layers: &[NormalStackOperatorLayer<'a>],
        quota: &BackingQuota,
        profile: &Profile,
        stored: SampleEncoding,
        composite: GimpSpace,
        limits: NormalStackLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedNormalError> {
        Self::prepare(layers.iter().copied(), quota, profile, stored, composite, limits, control)
    }
    fn prepare(
        layers: impl ExactSizeIterator<Item = NormalStackOperatorLayer<'a>> + Clone,
        quota: &BackingQuota,
        profile: &Profile,
        stored: SampleEncoding,
        composite: GimpSpace,
        limits: NormalStackLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        if limits.max_layers > 10_000 || layers.len() > limits.max_layers {
            return Err(BackedNormalError::Invalid("bounded stack layer count"));
        }
        if limits.band.max_rows == 0 || limits.band.max_rows > 256 || limits.band.max_pixels == 0 || limits.band.max_pixels > 65_536 {
            return Err(BackedNormalError::Invalid("band limits"));
        }
        if !matches!(composite, GimpSpace::LinearRgb | GimpSpace::ProfileRgb | GimpSpace::PerceptualRgb) {
            return Err(BackedNormalError::Unsupported("analytic RGB transfer composite spaces only"));
        }
        let gray = match profile.color_space {
            ColorSpace::Gray => true,
            ColorSpace::Rgb => false,
            _ => return Err(BackedNormalError::Unsupported("RGB/Gray Normal stack only")),
        };
        // Count/validate visible metadata without reading a tile. Disabled mask
        // fields and hidden entries remain unused, as in the single-layer API.
        let mut count = 0_usize;
        for operator_layer in layers.clone() {
            let pair = operator_layer.input;
            let input = pair.input;
            control.check(std::mem::size_of::<NormalStackLayer<'_>>() as u64).map_err(BackedNormalError::Backing)?;
            if !input.visible {
                continue;
            }
            // Explicit exhaustive match prevents future enum variants from
            // silently acquiring this stack contract.
            match operator_layer.operator {
                GimpOperator::Union | GimpOperator::ClipToBackdrop | GimpOperator::ClipToLayer | GimpOperator::Intersection => (),
            }
            valid_coverage(input.opacity)?;
            if !quota.shares_ledger(input.layer.quota()) {
                return Err(BackedNormalError::Invalid("stack persistent ledger mismatch"));
            }
            if let Some(mask) = input.mask.filter(|m| m.enabled) {
                valid_coverage(mask.outside_coverage)?;
                let format = mask.layer.format();
                if format.mode != photocraft_doc::ColorMode::Grayscale || format.alpha {
                    return Err(BackedNormalError::Unsupported("numeric Gray mask without alpha only"));
                }
                if !quota.shares_ledger(mask.layer.quota()) {
                    return Err(BackedNormalError::Invalid("mask persistent ledger mismatch"));
                }
            }
            if input.mask.is_some() && pair.mask.is_some() {
                return Err(BackedNormalError::Invalid("multiple stack mask sources"));
            }
            if let Some(mask) = pair.mask.filter(|m| m.enabled) {
                valid_coverage(mask.outside_coverage)?;
                if mask.view.format().mode != photocraft_doc::ColorMode::Grayscale || mask.view.format().alpha {
                    return Err(BackedNormalError::Unsupported("numeric resident Gray mask without alpha only"));
                }
            }
            count = count.checked_add(1).ok_or(BackedNormalError::Invalid("stack count"))?;
        }
        let payload = count.checked_mul(std::mem::size_of::<Entry<'a>>()).ok_or(BackedNormalError::Invalid("stack index size"))?;
        let owner = std::mem::size_of::<Self>();
        let envelope = payload.checked_mul(2).and_then(|n| n.checked_add(owner)).ok_or(BackedNormalError::Invalid("stack index size"))?;
        let lease = quota.reserve(QuotaKind::Index, envelope as u64).map_err(BackedNormalError::Quota)?;
        let mut out = Self { entries: Vec::new(), quota: quota.clone(), limits, gray, _lease: lease };
        control.check(envelope as u64).map_err(BackedNormalError::Backing)?;
        out.entries.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
        let actual = out
            .entries
            .capacity()
            .checked_mul(std::mem::size_of::<Entry<'a>>())
            .and_then(|n| n.checked_add(owner))
            .ok_or(BackedNormalError::Invalid("stack index capacity"))?;
        out._lease.reconcile_capacity(actual as u64).map_err(BackedNormalError::Quota)?;
        for operator_layer in layers.clone() {
            let pair = operator_layer.input;
            let input = pair.input;
            control.check(1).map_err(BackedNormalError::Backing)?;
            if !input.visible {
                continue;
            }
            let prepared = PreparedBackedNormal::new_with_composite_operator(
                input.layer,
                profile,
                stored,
                composite,
                operator_layer.operator,
                input.layer.descriptor(),
                limits.band,
                control,
            )?;
            out.entries.push(Entry { prepared, opacity: input.opacity, mask: input.mask, resident_mask: pair.mask });
        }
        control.check(1).map_err(BackedNormalError::Backing)?;
        Ok(out)
    }
    pub fn render_band(&self, rect: Rect, backdrop: [f64; 4], control: &ReadControl<'_>) -> Result<OwnedNormalStackBand, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or(BackedNormalError::Invalid("band size"))?;
        if rect.is_empty() || rect.height() > self.limits.band.max_rows || count > self.limits.band.max_pixels {
            return Err(BackedNormalError::Invalid("band size"));
        }
        if backdrop.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(BackedNormalError::Unsupported("finite unit backdrop required"));
        }
        if self.gray && (backdrop[0] != backdrop[1] || backdrop[0] != backdrop[2]) {
            return Err(BackedNormalError::Invalid("Gray backdrop must repeat its stored component"));
        }
        let mut prior: Option<OwnedNormalBand> = None;
        for entry in &self.entries {
            control.check(1).map_err(BackedNormalError::Backing)?;
            let next = if entry.resident_mask.is_some() {
                if let Some(band) = prior.as_ref() {
                    entry.prepared.render_band_over_with_resident_coverage(rect, band.pixels(), entry.opacity, entry.resident_mask, control)?
                } else {
                    entry.prepared.render_band_with_resident_coverage(rect, backdrop, entry.opacity, entry.resident_mask, control)?
                }
            } else if let Some(band) = prior.as_ref() {
                entry.prepared.render_band_over_with_coverage(rect, band.pixels(), entry.opacity, entry.mask, control)?
            } else {
                entry.prepared.render_band_with_coverage(rect, backdrop, entry.opacity, entry.mask, control)?
            };
            // During a transition exactly two output bands coexist. The previous
            // payload/lease is released before processing the following layer.
            prior = Some(next);
        }
        let pixels = if let Some(band) = prior {
            StackPixels::Layer(band)
        } else {
            let bytes = count.checked_mul(32).and_then(|n| n.checked_add(64)).ok_or(BackedNormalError::Invalid("band size"))?;
            let mut lease = self.quota.reserve(QuotaKind::Resident, bytes).map_err(BackedNormalError::Quota)?;
            let count = usize::try_from(count).map_err(|_| BackedNormalError::Invalid("band size"))?;
            control.check(1).map_err(BackedNormalError::Backing)?;
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
            let actual = (pixels.capacity() as u64).checked_mul(32).ok_or(BackedNormalError::Invalid("band capacity"))?;
            lease.reconcile_capacity(actual).map_err(BackedNormalError::Quota)?;
            for _ in 0..count {
                control.check(32).map_err(BackedNormalError::Backing)?;
                pixels.push(backdrop);
            }
            StackPixels::Backdrop { pixels, _lease: lease }
        };
        control.check(1).map_err(BackedNormalError::Backing)?;
        Ok(OwnedNormalStackBand { rect, pixels })
    }
}
fn valid_coverage(value: f64) -> Result<(), BackedNormalError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(BackedNormalError::Invalid("finite unit opacity/mask coverage required"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "backed_normal_stack_operator_tests.rs"]
mod operator_tests;
