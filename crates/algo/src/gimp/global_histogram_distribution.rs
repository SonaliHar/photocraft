//! Checked cumulative distribution and exact rank queries over explicit bins.
//! No equalization, sample recoloring, GEGL admission or source identity inferred.
use super::global_histogram::{Channel, Domain, Histogram, MAX_BINS, Specification, Transparency};
use photocraft_color::SampleType;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
use std::mem::size_of;
#[derive(Clone, Copy, Debug)]
pub struct Binding<'a> {
    pub extent: Rect,
    pub sample: SampleType,
    pub transparency: Transparency,
    pub specifications: &'a [Specification],
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Schema,
    Counts,
    Overflow,
    Empty,
    Rank,
    Channel,
}
impl From<RasterInputError> for Error {
    fn from(error: RasterInputError) -> Self {
        Self::Raster(error)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "histogram distribution: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rank {
    /// Zero-based observation rank, strictly less than the included total.
    ZeroBased(u64),
    /// Exact floor((total-1)*numerator/denominator), 0<=numerator<=denominator.
    /// Zero selects the first observation; one selects the last. No interpolation.
    Rational { numerator: u64, denominator: u64 },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinRange {
    IntegerCodes {
        low: u32,
        high: u32,
    },
    /// Symbolic affine domain boundaries: low + (high-low)*numerator/denominator.
    /// This is a bin label, not an exact inverse of rounded floating binning.
    FloatRange {
        domain_low: f64,
        domain_high: f64,
        lower_numerator: u32,
        upper_numerator: u32,
        denominator: u32,
        upper_inclusive: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantile {
    pub specification: Specification,
    pub bin_index: usize,
    pub range: BinRange,
    pub total: u64,
    pub cumulative_before: u64,
    pub cumulative_through: u64,
}
#[derive(Debug)]
pub struct ChannelDistribution {
    specification: Specification,
    cumulative: Vec<u64>,
}
impl ChannelDistribution {
    pub fn specification(&self) -> Specification {
        self.specification
    }
    pub fn cumulative(&self) -> &[u64] {
        &self.cumulative
    }
}
#[derive(Debug)]
pub struct Distribution {
    extent: Rect,
    sample: SampleType,
    transparency: Transparency,
    total: u64,
    channels: Vec<ChannelDistribution>,
}
impl Distribution {
    pub fn extent(&self) -> Rect {
        self.extent
    }
    pub fn sample(&self) -> SampleType {
        self.sample
    }
    pub fn transparency(&self) -> Transparency {
        self.transparency
    }
    pub fn total(&self) -> u64 {
        self.total
    }
    pub fn channels(&self) -> &[ChannelDistribution] {
        &self.channels
    }
    /// First bin whose cumulative count is strictly greater than the selected
    /// zero-based rank; zero-count bins never win ties. Query work is <=17
    /// binary-search comparisons per bounded channel, with cancellation polls.
    pub fn quantile(&self, channel: Channel, rank: Rank, control: &Interrupt<'_>) -> Result<Quantile> {
        poll(control)?;
        let rank = resolve_rank(self.total, rank)?;
        let values = self.channels.iter().find(|entry| entry.specification.channel == channel).ok_or(Error::Channel)?;
        let mut low = 0usize;
        let mut high = values.cumulative.len();
        while low < high {
            poll(control)?;
            let middle = low.checked_add((high - low) / 2).ok_or(Error::Overflow)?;
            if *values.cumulative.get(middle).ok_or(Error::Counts)? <= rank {
                low = middle.checked_add(1).ok_or(Error::Overflow)?;
            } else {
                high = middle;
            }
        }
        let through = *values.cumulative.get(low).ok_or(Error::Counts)?;
        let before = if low == 0 { 0 } else { *values.cumulative.get(low - 1).ok_or(Error::Counts)? };
        let range = bin_range(values.specification, low)?.ok_or(Error::Counts)?;
        poll(control)?;
        Ok(Quantile { specification: values.specification, bin_index: low, range, total: self.total, cumulative_before: before, cumulative_through: through })
    }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::Overflow)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(Error::Overflow)
}
fn same_domain(a: Domain, b: Domain) -> bool {
    match (a, b) {
        (Domain::IntegerCodes { low: a, high: b }, Domain::IntegerCodes { low: c, high: d }) => a == c && b == d,
        (Domain::FloatRange { low: a, high: b }, Domain::FloatRange { low: c, high: d }) => a.to_bits() == c.to_bits() && b.to_bits() == d.to_bits(),
        _ => false,
    }
}
fn specification(spec: Specification, sample: SampleType) -> Result<()> {
    if spec.bins == 0 || spec.bins > MAX_BINS || matches!(spec.channel,Channel::Color(index) if index>=3) {
        return Err(Error::Schema);
    }
    match (sample, spec.domain) {
        (SampleType::U8 | SampleType::U16 | SampleType::U32, Domain::IntegerCodes { low, high }) => {
            let max = match sample {
                SampleType::U8 => u32::from(u8::MAX),
                SampleType::U16 => u32::from(u16::MAX),
                _ => u32::MAX,
            };
            if low > high || high > max {
                return Err(Error::Schema);
            }
        }
        (SampleType::F32 | SampleType::F64, Domain::FloatRange { low, high }) => {
            if !low.is_finite() || !high.is_finite() || low >= high || !(high - low).is_finite() {
                return Err(Error::Schema);
            }
        }
        _ => return Err(Error::Schema),
    }
    Ok(())
}
fn ceil_div(numerator: u64, denominator: u64) -> Result<u64> {
    if denominator == 0 {
        return Err(Error::Schema);
    }
    add(numerator / denominator, u64::from(!numerator.is_multiple_of(denominator)))
}
fn bin_range(spec: Specification, index: usize) -> Result<Option<BinRange>> {
    if index >= spec.bins {
        return Err(Error::Schema);
    }
    let i = u64::try_from(index).map_err(|_| Error::Overflow)?;
    let bins = u64::try_from(spec.bins).map_err(|_| Error::Overflow)?;
    match spec.domain {
        Domain::IntegerCodes { low, high } => {
            let span = add(u64::from(high).checked_sub(u64::from(low)).ok_or(Error::Schema)?, 1)?;
            let start = ceil_div(mul(i, span)?, bins)?;
            let end = ceil_div(mul(add(i, 1)?, span)?, bins)?;
            if start == end {
                return Ok(None);
            }
            let first = u32::try_from(add(u64::from(low), start)?).map_err(|_| Error::Overflow)?;
            let last = u32::try_from(add(u64::from(low), end.checked_sub(1).ok_or(Error::Overflow)?)?).map_err(|_| Error::Overflow)?;
            Ok(Some(BinRange::IntegerCodes { low: first, high: last }))
        }
        Domain::FloatRange { low, high } => Ok(Some(BinRange::FloatRange {
            domain_low: low,
            domain_high: high,
            lower_numerator: u32::try_from(index).map_err(|_| Error::Overflow)?,
            upper_numerator: u32::try_from(index.checked_add(1).ok_or(Error::Overflow)?).map_err(|_| Error::Overflow)?,
            denominator: u32::try_from(spec.bins).map_err(|_| Error::Overflow)?,
            upper_inclusive: index.checked_add(1) == Some(spec.bins),
        })),
    }
}
fn resolve_rank(total: u64, rank: Rank) -> Result<u64> {
    if total == 0 {
        return Err(Error::Empty);
    }
    match rank {
        Rank::ZeroBased(rank) if rank < total => Ok(rank),
        Rank::ZeroBased(_) => Err(Error::Rank),
        Rank::Rational { numerator, denominator } => {
            if denominator == 0 || numerator > denominator {
                return Err(Error::Rank);
            }
            let product = u128::from(total - 1).checked_mul(u128::from(numerator)).ok_or(Error::Overflow)?;
            u64::try_from(product / u128::from(denominator)).map_err(|_| Error::Overflow)
        }
    }
}
/// Validate caller binding and count totals before allocating private cumulative
/// owners. Metadata area is checked arithmetically, without sample allocation;
/// this does not requalify the histogram producer's extent/profile/source policy.
/// Each count sum must equal included_pixels; examined_pixels must equal extent
/// area and include at least those observations. Empty distributions refuse.
/// Known input Vec capacities plus output capacities are charged, not total RSS.
pub fn try_build(histogram: &Histogram, binding: Binding<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Distribution> {
    poll(control)?;
    let specs = binding.specifications;
    if specs.is_empty()
        || specs.len() > 4
        || histogram.channels.len() != specs.len()
        || histogram.extent != binding.extent
        || histogram.sample != binding.sample
        || histogram.transparency != binding.transparency
    {
        return Err(Error::Schema);
    }
    let width = u64::try_from(i64::from(binding.extent.x1) - i64::from(binding.extent.x0)).map_err(|_| Error::Schema)?;
    let height = u64::try_from(i64::from(binding.extent.y1) - i64::from(binding.extent.y0)).map_err(|_| Error::Schema)?;
    if histogram.examined_pixels != mul(width, height)?
        || histogram.included_pixels > histogram.examined_pixels
        || (histogram.transparency == Transparency::IncludeAll && histogram.included_pixels != histogram.examined_pixels)
    {
        return Err(Error::Counts);
    }
    let mut bins = 0u64;
    let mut input = mul(histogram.channels.capacity() as u64, size_of::<super::global_histogram::ChannelHistogram>() as u64)?;
    for (index, (spec, channel)) in specs.iter().zip(&histogram.channels).enumerate() {
        poll(control)?;
        specification(*spec, binding.sample)?;
        if specs.get(..index).ok_or(Error::Schema)?.iter().any(|prior| prior.channel == spec.channel)
            || channel.specification.channel != spec.channel
            || channel.specification.bins != spec.bins
            || !same_domain(channel.specification.domain, spec.domain)
            || channel.counts.len() != spec.bins
        {
            return Err(Error::Schema);
        }
        bins = add(bins, spec.bins as u64)?;
        input = add(input, mul(channel.counts.capacity() as u64, size_of::<u64>() as u64)?)?;
    }
    let work = add(mul(bins, 16)?, mul(specs.len() as u64, 128)?)?;
    let output = add(mul(bins, size_of::<u64>() as u64)?, mul(specs.len() as u64, size_of::<ChannelDistribution>() as u64)?)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if work > limits.work_units || add(input, output)? > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    for channel in &histogram.channels {
        let mut total = 0u64;
        for (index, count) in channel.counts.iter().enumerate() {
            poll(control)?;
            if *count > 0 && bin_range(channel.specification, index)?.is_none() {
                return Err(Error::Counts);
            }
            total = add(total, *count)?;
        }
        if total != histogram.included_pixels {
            return Err(Error::Counts);
        }
    }
    if histogram.included_pixels == 0 {
        return Err(Error::Empty);
    }
    poll(control)?;
    let mut channels = Vec::new();
    channels.try_reserve_exact(specs.len()).map_err(|_| RasterInputError::AllocationFailed)?;
    if channels.capacity() != specs.len() {
        return Err(RasterInputError::AllocationFailed.into());
    }
    for channel in &histogram.channels {
        let mut cumulative = Vec::new();
        cumulative.try_reserve_exact(channel.counts.len()).map_err(|_| RasterInputError::AllocationFailed)?;
        if cumulative.capacity() != channel.counts.len() {
            return Err(RasterInputError::AllocationFailed.into());
        }
        let mut total = 0u64;
        for count in &channel.counts {
            poll(control)?;
            total = add(total, *count)?;
            cumulative.push(total);
        }
        channels.push(ChannelDistribution { specification: channel.specification, cumulative });
    }
    poll(control)?;
    Ok(Distribution { extent: binding.extent, sample: binding.sample, transparency: binding.transparency, total: histogram.included_pixels, channels })
}

#[cfg(test)]
mod tests {
    use super::super::global_histogram::ChannelHistogram;
    use super::*;
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }
    fn spec() -> Specification {
        Specification { channel: Channel::Color(0), domain: Domain::IntegerCodes { low: 0, high: 3 }, bins: 2 }
    }
    fn histogram(counts: Vec<u64>, extent: Rect, included: u64) -> Histogram {
        Histogram {
            extent,
            sample: SampleType::U32,
            transparency: Transparency::IncludeAll,
            examined_pixels: u64::try_from(i64::from(extent.x1) - i64::from(extent.x0)).unwrap()
                * u64::try_from(i64::from(extent.y1) - i64::from(extent.y0)).unwrap(),
            included_pixels: included,
            channels: vec![ChannelHistogram { specification: spec(), counts }],
        }
    }
    fn build(histogram: &Histogram) -> Result<Distribution> {
        let specs = histogram.channels.iter().map(|channel| channel.specification).collect::<Vec<_>>();
        try_build(
            histogram,
            Binding { extent: histogram.extent, sample: histogram.sample, transparency: histogram.transparency, specifications: &specs },
            limits(),
            &Interrupt::NONE,
        )
    }
    #[test]
    fn counts_and_ranks_above_double_exact_integer_range_are_lossless() {
        let first = 9_007_199_254_740_993u64;
        let total = 10_000_000_000_000_000u64;
        let histogram = histogram(vec![first, total - first], Rect::new(0, 0, 100_000_000, 100_000_000), total);
        let distribution = build(&histogram).unwrap();
        assert_eq!(distribution.channels()[0].cumulative(), &[first, total]);
        assert_eq!(distribution.quantile(Channel::Color(0), Rank::ZeroBased(first - 1), &Interrupt::NONE).unwrap().bin_index, 0);
        let upper = distribution.quantile(Channel::Color(0), Rank::Rational { numerator: first, denominator: total - 1 }, &Interrupt::NONE).unwrap();
        assert_eq!(upper.bin_index, 1);
        assert_eq!(upper.cumulative_before, first);
        assert_eq!(upper.range, BinRange::IntegerCodes { low: 2, high: 3 });
        assert_eq!(distribution.quantile(Channel::Color(0), Rank::Rational { numerator: 0, denominator: u64::MAX }, &Interrupt::NONE).unwrap().bin_index, 0);
        assert_eq!(
            distribution.quantile(Channel::Color(0), Rank::Rational { numerator: u64::MAX, denominator: u64::MAX }, &Interrupt::NONE).unwrap().bin_index,
            1
        );
    }
    #[test]
    fn zero_bins_ties_and_float_ranges_are_explicit_labels() {
        let specification = Specification { channel: Channel::Color(0), domain: Domain::FloatRange { low: -2.0, high: 2.0 }, bins: 4 };
        let histogram = Histogram {
            extent: Rect::new(0, 0, 4, 1),
            sample: SampleType::F64,
            transparency: Transparency::IncludeAll,
            examined_pixels: 4,
            included_pixels: 4,
            channels: vec![ChannelHistogram { specification, counts: vec![2, 0, 2, 0] }],
        };
        let distribution = build(&histogram).unwrap();
        assert_eq!(distribution.quantile(Channel::Color(0), Rank::ZeroBased(1), &Interrupt::NONE).unwrap().bin_index, 0);
        assert_eq!(
            distribution.quantile(Channel::Color(0), Rank::ZeroBased(2), &Interrupt::NONE).unwrap().range,
            BinRange::FloatRange { domain_low: -2.0, domain_high: 2.0, lower_numerator: 2, upper_numerator: 3, denominator: 4, upper_inclusive: false }
        );
        for rank in [Rank::ZeroBased(4), Rank::Rational { numerator: 1, denominator: 0 }, Rank::Rational { numerator: 3, denominator: 2 }] {
            assert!(matches!(distribution.quantile(Channel::Color(0), rank, &Interrupt::NONE), Err(Error::Rank)));
        }
    }
    #[test]
    fn empty_corrupt_overflow_empty_code_bin_and_duplicate_schemas_refuse() {
        let empty = histogram(vec![0, 0], Rect::new(0, 0, 0, 0), 0);
        assert!(matches!(build(&empty), Err(Error::Empty)));
        let mismatch = histogram(vec![1, 0], Rect::new(0, 0, 2, 1), 2);
        assert!(matches!(build(&mismatch), Err(Error::Counts)));
        let extent = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        let area = u64::from(u32::MAX) * u64::from(u32::MAX);
        let overflow = histogram(vec![u64::MAX, 1], extent, area);
        assert!(matches!(build(&overflow), Err(Error::Overflow)));
        let specification = Specification { channel: Channel::Color(0), domain: Domain::IntegerCodes { low: 0, high: 0 }, bins: 2 };
        let invalid = Histogram {
            extent: Rect::new(0, 0, 1, 1),
            sample: SampleType::U8,
            transparency: Transparency::IncludeAll,
            examined_pixels: 1,
            included_pixels: 1,
            channels: vec![ChannelHistogram { specification, counts: vec![0, 1] }],
        };
        assert!(matches!(build(&invalid), Err(Error::Counts)));
        let mut duplicate = histogram(vec![1, 1], Rect::new(0, 0, 2, 1), 2);
        duplicate.channels.push(ChannelHistogram { specification: spec(), counts: vec![1, 1] });
        assert!(matches!(build(&duplicate), Err(Error::Schema)));
    }
    #[test]
    fn bit_exact_binding_capacity_budget_and_validation_cancellation_refuse() {
        let mut histogram = histogram(vec![1, 1], Rect::new(0, 0, 2, 1), 2);
        let specs = [spec()];
        let binding = Binding { extent: histogram.extent, sample: histogram.sample, transparency: histogram.transparency, specifications: &specs };
        assert!(matches!(
            try_build(&histogram, binding, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
            Err(Error::Raster(RasterInputError::LimitExceeded))
        ));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 3;
        assert!(matches!(try_build(&histogram, binding, limits(), &Interrupt::cancel_only(&cancel)), Err(Error::Raster(RasterInputError::Cancelled))));
        assert_eq!(histogram.channels[0].counts, vec![1, 1]);
        histogram.sample = SampleType::F64;
        histogram.channels[0].specification.domain = Domain::FloatRange { low: -0.0, high: 1.0 };
        let other = [Specification { domain: Domain::FloatRange { low: 0.0, high: 1.0 }, ..spec() }];
        let binding = Binding { extent: histogram.extent, sample: histogram.sample, transparency: histogram.transparency, specifications: &other };
        assert!(matches!(try_build(&histogram, binding, limits(), &Interrupt::NONE), Err(Error::Schema)));
    }
}
