//! Checked half-open ROI support and cumulative graph sub-budget planning.
//! No storage reads, profile conversion, evaluator or serialized-source admission.
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Halo {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abyss {
    None,
    Clamp,
}

/// Accept only finite integral pixel edges. Fractional graph geometry needs an
/// operation-specific rounding policy and is not silently truncated here.
pub fn rect_from_f64(edges: [f64; 4], format: PixelFormat) -> Result<Rect, RasterInputError> {
    let mut encoded = [0i32; 4];
    for (target, edge) in encoded.iter_mut().zip(edges) {
        if !edge.is_finite() || edge.fract() != 0.0 || edge < f64::from(i32::MIN) || edge > f64::from(i32::MAX) {
            return Err(RasterInputError::InvalidBounds);
        }
        *target = edge as i32;
    }
    let [x0, y0, x1, y1] = encoded;
    let rect = Rect::new(x0, y0, x1, y1);
    checked_region_byte_len(rect, format)?;
    Ok(rect)
}

/// Validated rectangular support. Coordinates may be negative within the existing
/// checked raster envelope; reversed or overflowing bounds never saturate.
#[derive(Clone, Copy, Debug)]
pub struct RoiPlan {
    output: Rect,
    support: Rect,
    source_extent: Rect,
    source_read: Rect,
    source_overlap: Rect,
    abyss: Abyss,
    format: PixelFormat,
}
impl RoiPlan {
    pub fn new(output: Rect, source_extent: Rect, halo: Halo, abyss: Abyss, format: PixelFormat, control: &Interrupt<'_>) -> Result<Self, RasterInputError> {
        poll(control)?;
        checked_region_byte_len(output, format)?;
        checked_region_byte_len(source_extent, format)?;
        let support = if output.is_empty() { output } else { expand(output, halo)? };
        checked_region_byte_len(support, format)?;
        let source_overlap = output.intersect(&source_extent);
        let source_read = match abyss {
            Abyss::None => support.intersect(&source_extent),
            Abyss::Clamp if support.is_empty() => Rect::EMPTY,
            Abyss::Clamp if source_extent.is_empty() => return Err(RasterInputError::InvalidBounds),
            Abyss::Clamp => {
                let x_max = source_extent.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
                let y_max = source_extent.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
                let x_last = support.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
                let y_last = support.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
                Rect::new(
                    support.x0.clamp(source_extent.x0, x_max),
                    support.y0.clamp(source_extent.y0, y_max),
                    x_last.clamp(source_extent.x0, x_max).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
                    y_last.clamp(source_extent.y0, y_max).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
                )
            }
        };
        checked_region_byte_len(source_read, format)?;
        poll(control)?;
        Ok(Self { output, support, source_extent, source_read, source_overlap, abyss, format })
    }
    pub fn output(self) -> Rect {
        self.output
    }
    pub fn support(self) -> Rect {
        self.support
    }
    /// Distinct source pixels needed, including the edge pixel for wholly external Clamp support.
    pub fn source_read(self) -> Rect {
        self.source_read
    }
    pub fn source_overlap(self) -> Rect {
        self.source_overlap
    }
    pub fn source_extent(self) -> Rect {
        self.source_extent
    }
    pub fn abyss(self) -> Abyss {
        self.abyss
    }

    /// Resolve only a coordinate inside this plan's support. None means zero/default
    /// abyss supplied by the evaluator, not an ambient or source-default read.
    pub fn address(self, x: i64, y: i64, control: &Interrupt<'_>) -> Result<Option<(i32, i32)>, RasterInputError> {
        poll(control)?;
        let in_support = x >= i64::from(self.support.x0) && x < i64::from(self.support.x1) && y >= i64::from(self.support.y0) && y < i64::from(self.support.y1);
        if !in_support {
            return Err(RasterInputError::InvalidBounds);
        }
        let inside = x >= i64::from(self.source_extent.x0)
            && x < i64::from(self.source_extent.x1)
            && y >= i64::from(self.source_extent.y0)
            && y < i64::from(self.source_extent.y1);
        if inside {
            return Ok(Some((i32::try_from(x).map_err(|_| RasterInputError::InvalidBounds)?, i32::try_from(y).map_err(|_| RasterInputError::InvalidBounds)?)));
        }
        match self.abyss {
            Abyss::None => Ok(None),
            Abyss::Clamp => {
                if self.source_extent.is_empty() {
                    return Err(RasterInputError::InvalidBounds);
                }
                let x = x.clamp(i64::from(self.source_extent.x0), i64::from(self.source_extent.x1) - 1);
                let y = y.clamp(i64::from(self.source_extent.y0), i64::from(self.source_extent.y1) - 1);
                Ok(Some((i32::try_from(x).map_err(|_| RasterInputError::InvalidBounds)?, i32::try_from(y).map_err(|_| RasterInputError::InvalidBounds)?)))
            }
        }
    }
}

fn expand(rect: Rect, halo: Halo) -> Result<Rect, RasterInputError> {
    let edge = |value: i32, amount: u32, negative: bool| {
        let amount = i64::from(amount);
        let value = if negative { i64::from(value).checked_sub(amount) } else { i64::from(value).checked_add(amount) };
        i32::try_from(value.ok_or(RasterInputError::InvalidBounds)?).map_err(|_| RasterInputError::InvalidBounds)
    };
    Ok(Rect::new(edge(rect.x0, halo.left, true)?, edge(rect.y0, halo.top, true)?, edge(rect.x1, halo.right, false)?, edge(rect.y1, halo.bottom, false)?))
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn area(rect: Rect) -> Result<u64, RasterInputError> {
    u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or(RasterInputError::LimitExceeded)
}

#[derive(Clone, Copy, Debug)]
pub struct RoiLimits {
    pub area: u64,
    pub samples: u64,
    pub work: u64,
    pub retained_bytes: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoiUsage {
    pub area: u64,
    pub samples: u64,
    pub work: u64,
    pub retained_bytes: u64,
}
/// An explicit graph-owned cumulative ledger; reuse one ledger across nodes.
/// It accounts declared retained output/support buffers and caller extra retention,
/// not allocator/RSS overhead or undeclared source/history allocations.
pub struct RoiBudget {
    limits: RoiLimits,
    used: RoiUsage,
}
impl RoiBudget {
    pub fn new(limits: RoiLimits) -> Result<Self, RasterInputError> {
        if limits.retained_bytes == 0 || limits.retained_bytes > MAX_CHECKED_RASTER_BYTES {
            return Err(RasterInputError::LimitExceeded);
        }
        Ok(Self { limits, used: RoiUsage::default() })
    }
    pub fn used(&self) -> RoiUsage {
        self.used
    }
    /// Conservative no-dedup charging of output plus complete halo support.
    /// work_per_sample must include evaluator arithmetic/reads; zero refuses.
    /// Charge failures/cancellation leave this ledger unchanged.
    pub fn charge(&mut self, plan: &RoiPlan, work_per_sample: u64, extra_retained_bytes: u64, control: &Interrupt<'_>) -> Result<RoiUsage, RasterInputError> {
        poll(control)?;
        if work_per_sample == 0 {
            return Err(RasterInputError::LimitExceeded);
        }
        let pixels = area(plan.output)?.checked_add(area(plan.support)?).ok_or(RasterInputError::LimitExceeded)?;
        let samples = pixels.checked_mul(plan.format.channels() as u64).ok_or(RasterInputError::LimitExceeded)?;
        let work = samples.checked_mul(work_per_sample).ok_or(RasterInputError::LimitExceeded)?;
        let bytes = (checked_region_byte_len(plan.output, plan.format)? as u64)
            .checked_add(checked_region_byte_len(plan.support, plan.format)? as u64)
            .and_then(|n| n.checked_add(extra_retained_bytes))
            .ok_or(RasterInputError::LimitExceeded)?;
        let next = RoiUsage {
            area: self.used.area.checked_add(pixels).ok_or(RasterInputError::LimitExceeded)?,
            samples: self.used.samples.checked_add(samples).ok_or(RasterInputError::LimitExceeded)?,
            work: self.used.work.checked_add(work).ok_or(RasterInputError::LimitExceeded)?,
            retained_bytes: self.used.retained_bytes.checked_add(bytes).ok_or(RasterInputError::LimitExceeded)?,
        };
        if next.area > self.limits.area
            || next.samples > self.limits.samples
            || next.work > self.limits.work
            || next.retained_bytes > self.limits.retained_bytes
        {
            return Err(RasterInputError::LimitExceeded);
        }
        poll(control)?;
        self.used = next;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    fn format() -> PixelFormat {
        PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false }
    }
    #[test]
    fn negative_asymmetric_support_and_none_addresses() {
        let plan = RoiPlan::new(
            Rect::new(-3, -2, 2, 4),
            Rect::new(-4, -4, 4, 4),
            Halo { left: 2, top: 1, right: 3, bottom: 0 },
            Abyss::None,
            format(),
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(plan.support(), Rect::new(-5, -3, 5, 4));
        assert_eq!(plan.source_read(), Rect::new(-4, -3, 4, 4));
        assert_eq!(plan.address(-5, -3, &Interrupt::NONE).unwrap(), None);
        assert_eq!(plan.address(-4, -3, &Interrupt::NONE).unwrap(), Some((-4, -3)));
    }
    #[test]
    fn entirely_external_clamp_reads_exact_edge_pixel() {
        let plan = RoiPlan::new(Rect::new(-8, -8, -6, -6), Rect::new(1, 2, 4, 5), Halo::default(), Abyss::Clamp, format(), &Interrupt::NONE).unwrap();
        assert_eq!(plan.source_read(), Rect::new(1, 2, 2, 3));
        assert_eq!(plan.address(-7, -7, &Interrupt::NONE).unwrap(), Some((1, 2)));
    }
    #[test]
    fn reversed_halo_overflow_and_empty_clamp_refuse() {
        assert!(RoiPlan::new(Rect::new(2, 0, 1, 1), Rect::new(0, 0, 3, 3), Halo::default(), Abyss::None, format(), &Interrupt::NONE).is_err());
        assert!(
            RoiPlan::new(Rect::new(0, 0, 1, 1), Rect::new(0, 0, 3, 3), Halo { left: u32::MAX, ..Halo::default() }, Abyss::None, format(), &Interrupt::NONE)
                .is_err()
        );
        assert!(RoiPlan::new(Rect::new(0, 0, 1, 1), Rect::EMPTY, Halo::default(), Abyss::Clamp, format(), &Interrupt::NONE).is_err());
        for edge in [f64::NAN, f64::INFINITY, 0.5, f64::MAX] {
            assert!(rect_from_f64([edge, 0.0, 1.0, 1.0], format()).is_err());
        }
    }
    #[test]
    fn aggregate_refusal_and_cancel_leave_ledger_unchanged() {
        let plan = RoiPlan::new(Rect::new(0, 0, 1, 1), Rect::new(0, 0, 3, 3), Halo::default(), Abyss::None, format(), &Interrupt::NONE).unwrap();
        let mut budget = RoiBudget::new(RoiLimits { area: 3, samples: 3, work: 3, retained_bytes: 100 }).unwrap();
        budget.charge(&plan, 1, 0, &Interrupt::NONE).unwrap();
        let before = budget.used();
        assert!(budget.charge(&plan, 1, 0, &Interrupt::NONE).is_err());
        assert_eq!(budget.used(), before);
        assert!(budget.charge(&plan, 1, 0, &Interrupt::cancel_only(&|| true)).is_err());
        assert_eq!(budget.used(), before);
    }
}
