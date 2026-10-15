//! Explicit CURRENT singleton stock Point canvas display. Original source words,
//! masks and off-canvas backing remain immutable; this is neither a persistence
//! receipt nor source eligibility. The Point context preserves the existing
//! wide schedule before applying Normal/Union over transparent positive zero.
use crate::{
    backed_normal::{BackedNormalError, NormalBandLimits},
    backed_normal_stack::OwnedNormalStackBand,
    backed_stock_point_context::PreparedBackedStockPointContext,
    gimp_composite::normal_over_zero_backdrop,
};
use photocraft_cms::Profile;
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};

pub struct PreparedBackedStockPointDocument<'a> {
    context: PreparedBackedStockPointContext<'a>,
    bounds: Rect,
    quota: BackingQuota,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}
struct Output {
    pixels: Vec<[f64; 4]>,
    lease: QuotaLease,
}
fn invalid() -> BackedNormalError {
    BackedNormalError::Invalid("stock Point display size or ROI")
}
impl<'a> PreparedBackedStockPointDocument<'a> {
    pub fn new(owner: &'a AccountedDocument, profile: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let header = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(invalid)?;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(BackedNormalError::Quota)?;
        let context = PreparedBackedStockPointContext::new(owner, profile, limits, control)?;
        let result = Self { context, bounds: owner.document().bounds(), quota: owner.quota().clone(), limits, _lease: lease };
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(result)
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    /// Includes the canvas outside the drawable as positive-zero transparent
    /// abyss. Hidden layers still take the same operative sample validation
    /// path before composition; visibility never licenses unchecked source data.
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNormalStackBand, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(invalid)?;
        if rect.is_empty() || !self.bounds.contains_rect(&rect) || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(invalid());
        }
        let count = usize::try_from(count).map_err(|_| invalid())?;
        let capacity_limit = count.checked_add(2).ok_or_else(invalid)?;
        let header =
            std::mem::size_of::<OwnedNormalStackBand>().checked_add(std::mem::size_of::<Output>()).and_then(|n| n.checked_add(128)).ok_or_else(invalid)?;
        let envelope = capacity_limit.checked_mul(std::mem::size_of::<[f64; 4]>()).and_then(|n| n.checked_add(header)).ok_or_else(invalid)?;
        let lease = self.quota.reserve(QuotaKind::Resident, envelope as u64).map_err(BackedNormalError::Quota)?;
        let mut output = Output { pixels: Vec::new(), lease };
        let _scratch = self.quota.reserve(QuotaKind::Resident, 256).map_err(BackedNormalError::Quota)?;
        let overlap = rect.intersect(&self.context.descriptor().bounds());
        let source = if overlap.is_empty() { None } else { Some(self.context.render_wide_band(overlap, control)?) };
        if source
            .as_ref()
            .is_some_and(|source| source.rect() != overlap || source.pixels().len() as u64 != u64::from(overlap.width()) * u64::from(overlap.height()))
        {
            return Err(invalid());
        }
        control.check(1).map_err(BackedNormalError::Backing)?;
        output.pixels.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
        if output.pixels.capacity() > capacity_limit {
            return Err(BackedNormalError::Allocation);
        }
        let actual = output.pixels.capacity().checked_mul(std::mem::size_of::<[f64; 4]>()).and_then(|n| n.checked_add(header)).ok_or_else(invalid)?;
        output.lease.reconcile_capacity(actual as u64).map_err(BackedNormalError::Quota)?;
        let opacity = if self.context.layer_visible() { self.context.layer_opacity() } else { 0.0 };
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                control.check(128).map_err(BackedNormalError::Backing)?;
                let value = if overlap.contains(x, y) {
                    let row = usize::try_from(i64::from(y) - i64::from(overlap.y0)).map_err(|_| invalid())?;
                    let column = usize::try_from(i64::from(x) - i64::from(overlap.x0)).map_err(|_| invalid())?;
                    let index = row.checked_mul(overlap.width() as usize).and_then(|n| n.checked_add(column)).ok_or_else(invalid)?;
                    let pixel = *source.as_ref().ok_or_else(invalid)?.pixels().get(index).ok_or_else(invalid)?;
                    normal_over_zero_backdrop(pixel, opacity).map_err(BackedNormalError::Color)?
                } else {
                    [0.0; 4]
                };
                output.pixels.push(value);
            }
        }
        if output.pixels.len() != count {
            return Err(invalid());
        }
        control.check(0).map_err(BackedNormalError::Backing)?;
        let Output { pixels, lease } = output;
        Ok(OwnedNormalStackBand::from_accounted_pixels(rect, pixels, lease))
    }
}

#[cfg(test)]
#[path = "backed_stock_point_document_tests.rs"]
mod tests;
