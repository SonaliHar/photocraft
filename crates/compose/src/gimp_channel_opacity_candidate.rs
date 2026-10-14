//! Private explicit Gray opacity mathematics for the retained 32x32 domains.
//! This is a derived display candidate under the existing <=2/255 contract,
//! not an assertion of GIMP's unique internal rounding schedule. Source words
//! remain separate and exact. No mask, filter-stack, profile or source authority
//! is inferred; enclosing callers retain all original metadata/backing owners.
//! Identity/COW and region writer internals remain bounded raster provider
//! boundaries, not a claim of complete allocator/RSS control.
use crate::gimp_channel_gaussian_candidate::{DerivedPrecision, Limits, NativeDomain};
use crate::gimp_composite::{CompositionError, Result};
use crate::gimp_render::PreparationBudget;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, RasterInputError, Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixSpace {
    SrgbDecodedLinear,
    LinearF64,
}
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub domain: NativeDomain,
    pub mix_space: MixSpace,
    pub precision: DerivedPrecision,
    /// Exact observed positive-zero, one-half or one binary64 scalar only.
    pub opacity_f64_bits: u64,
    pub output: Rect,
}
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub source: &'a Surface,
    pub derived: &'a Surface,
    pub bounds: Rect,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_resident_bytes: u64,
    pub derived_resident_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub work_units: u64,
}
pub struct PrivateOutput<'a, L> {
    original: Inputs<'a>,
    surface: Surface,
    config: Config,
    requirements: Requirements,
    _lease: L,
}
impl<L> PrivateOutput<'_, L> {
    pub fn original(&self) -> Inputs<'_> {
        self.original
    }
    pub fn surface(&self) -> &Surface {
        &self.surface
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
}
fn limit() -> CompositionError {
    RasterInputError::LimitExceeded.into()
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn opacity(config: Config) -> Result<f64> {
    match config.opacity_f64_bits {
        0 => Ok(0.0),
        0x3fe0000000000000 => Ok(0.5),
        0x3ff0000000000000 => Ok(1.0),
        _ => Err(CompositionError::Unsupported("observed Gray opacity scalar")),
    }
}
fn format(domain: NativeDomain) -> PixelFormat {
    match domain {
        NativeDomain::SrgbNonlinearU8 => PixelFormat::GRAY8,
        NativeDomain::SrgbLinearF64 => PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
    }
}
/// Allocation-free structural/quota admission. This does not qualify input
/// samples; prepare validates selected arithmetic inputs after holding its lease.
pub fn requirements(input: Inputs<'_>, config: Config, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    let max = Limits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(limit());
    }
    opacity(config)?;
    let fmt = format(config.domain);
    if input.bounds != Rect::new(0, 0, 32, 32)
        || config.output.is_empty()
        || config.output.intersect(&input.bounds) != config.output
        || !matches!(
            (config.domain, config.mix_space),
            (NativeDomain::SrgbNonlinearU8, MixSpace::SrgbDecodedLinear) | (NativeDomain::SrgbLinearF64, MixSpace::LinearF64)
        )
        || input.source.format() != fmt
        || input.derived.format() != fmt
        || input.source.tile_count() > 1
        || input.derived.tile_count() > 1
    {
        return Err(CompositionError::Unsupported("explicit Gray opacity domain or geometry"));
    }
    let tile = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, fmt.bytes_per_pixel() as u64)?;
    for s in [input.source, input.derived] {
        s.try_validate_tile_origins_with(c)?;
        for (position, t) in s.tiles() {
            poll(c)?;
            if *position != TileCoord::new(0, 0) || t.bytes().len() as u64 != tile {
                return Err(CompositionError::Invalid("Gray opacity tile descriptor"));
            }
        }
    }
    let source = input.source.resident_accounted_bytes()?;
    let derived = input.derived.resident_accounted_bytes()?;
    // Eight full F64 tile bodies price output COW and bounded raster read/write
    // staging; eight 32x32 F64 planes cover actual native/decoded Vec capacities.
    let index = 32 * 256;
    let scratch = 8 * 1024 * 8;
    let peak = add(add(source, derived)?, add(mul(8, 256 * 256 * 8)?, add(scratch, index + 8192)?)?)?;
    let work = 1024 * 512 + 256 * 256 * 128;
    if peak > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(limit());
    }
    poll(c)?;
    Ok(Requirements {
        source_resident_bytes: source,
        derived_resident_bytes: derived,
        peak_resident_bytes: peak,
        index_bytes: index,
        scratch_bytes: scratch,
        work_units: work,
    })
}
fn decode(x: f64) -> f64 {
    if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}
fn encode(x: f64) -> f64 {
    if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}
fn selected_finite(input: Inputs<'_>, config: Config, c: &Interrupt<'_>) -> Result<()> {
    // Zero opacity is raw source identity and never interprets source NaNs.
    // One is raw derived ROI identity, but its selected words retain this
    // candidate's finite unit admission. Off-ROI/default words stay untouched.
    let t = opacity(config)?;
    if t == 0.0 {
        return poll(c);
    }
    let mut sample = [0.0];
    for y in config.output.y0..config.output.y1 {
        poll(c)?;
        for x in config.output.x0..config.output.x1 {
            for (index, s) in [input.source, input.derived].into_iter().enumerate() {
                if t == 1.0 && index == 0 {
                    continue;
                }
                s.try_read_pixel_f64(x, y, &mut sample)?;
                if !sample[0].is_finite() || !(0.0..=1.0).contains(&sample[0]) {
                    return Err(RasterInputError::InvalidSample.into());
                }
            }
        }
    }
    poll(c)
}
/// Explicit unfused binary64 weighted sum in the declared linear mix space.
/// FinalF32 rounds only the computed linear midpoint before native encoding;
/// U8 quantization uses the existing raster writer, not a new rounding oracle.
/// Zero returns raw source COW; one copies raw derived ROI. Defaults and all
/// outside-ROI native words always come from source, including NaN/signed zero.
pub fn prepare<'a, B: PreparationBudget>(
    input: Inputs<'a>,
    config: Config,
    limits: Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<PrivateOutput<'a, B::Lease>> {
    let req = requirements(input, config, limits, c)?;
    budget.consume_preparation_work(req.work_units)?;
    let lease = budget.reserve_prepared_bytes(req.peak_resident_bytes)?;
    selected_finite(input, config, c)?;
    poll(c)?;
    let t = opacity(config)?;
    let mut output = input.source.try_convert_with(input.source.format(), c)?;
    if t == 1.0 {
        let bytes = input.derived.try_read_interleaved_with(config.output, c)?;
        if bytes.capacity() as u64 > req.scratch_bytes {
            return Err(limit());
        }
        output.try_write_interleaved_with(config.output, &bytes, c)?;
    } else if t == 0.5 {
        let count = usize::try_from(config.output.size().area()).map_err(|_| limit())?;
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
        if mul(values.capacity() as u64, 8)? > req.scratch_bytes {
            return Err(limit());
        }
        let mut a = [0.0];
        let mut b = [0.0];
        for y in config.output.y0..config.output.y1 {
            poll(c)?;
            for x in config.output.x0..config.output.x1 {
                input.source.try_read_pixel_f64(x, y, &mut a)?;
                input.derived.try_read_pixel_f64(x, y, &mut b)?;
                let (a, b) = match config.mix_space {
                    MixSpace::LinearF64 => (a[0], b[0]),
                    MixSpace::SrgbDecodedLinear => (decode(a[0]), decode(b[0])),
                };
                let mut v = a * (1.0 - t) + b * t;
                if config.precision == DerivedPrecision::FinalF32 {
                    v = f64::from(v as f32);
                }
                if config.mix_space == MixSpace::SrgbDecodedLinear {
                    v = encode(v);
                }
                if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                    return Err(RasterInputError::InvalidSample.into());
                }
                values.push(v);
            }
        }
        output.try_write_region_f64_with(config.output, &values, c)?;
    }
    if add(add(req.source_resident_bytes, req.derived_resident_bytes)?, output.resident_accounted_bytes()?)? > req.peak_resident_bytes {
        return Err(limit());
    }
    poll(c)?;
    Ok(PrivateOutput { original: input, surface: output, config, requirements: req, _lease: lease })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Budget;
    impl PreparationBudget for Budget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, _: u64) -> Result<()> {
            Ok(())
        }
        fn consume_preparation_work(&mut self, _: u64) -> Result<()> {
            Ok(())
        }
    }
    fn config(bits: u64) -> Config {
        Config {
            domain: NativeDomain::SrgbLinearF64,
            mix_space: MixSpace::LinearF64,
            precision: DerivedPrecision::F64,
            opacity_f64_bits: bits,
            output: Rect::new(0, 0, 32, 32),
        }
    }
    #[test]
    fn raw_zero_and_one_keep_original_default_and_off_roi() {
        let fmt = format(NativeDomain::SrgbLinearF64);
        let nan = f64::from_bits(0x7ff8000000000123);
        let source = Surface::try_with_default_bytes(fmt, &nan.to_le_bytes()).unwrap();
        let derived = Surface::try_with_default_bytes(fmt, &0.25f64.to_le_bytes()).unwrap();
        let input = Inputs { source: &source, derived: &derived, bounds: Rect::new(0, 0, 32, 32) };
        let zero = prepare(input, config(0), Limits::default(), &mut Budget, &Interrupt::NONE).unwrap();
        assert_eq!(zero.surface().default_bytes(), source.default_bytes());
        assert_eq!(zero.surface(), &source);
        let one = prepare(input, config(1f64.to_bits()), Limits::default(), &mut Budget, &Interrupt::NONE).unwrap();
        let mut word = [0u8; 8];
        one.surface().try_read_pixel_bytes(0, 0, &mut word).unwrap();
        assert_eq!(word, 0.25f64.to_le_bytes());
        one.surface().try_read_pixel_bytes(-1, 0, &mut word).unwrap();
        assert_eq!(word, nan.to_le_bytes());
        assert_eq!(one.surface().default_bytes(), source.default_bytes());
    }
    #[test]
    fn finite_refusal_limits_and_cancel_leave_borrowed_inputs() {
        let fmt = format(NativeDomain::SrgbLinearF64);
        let source = Surface::try_with_default_bytes(fmt, &f64::NAN.to_le_bytes()).unwrap();
        let derived = Surface::try_with_default_bytes(fmt, &0.5f64.to_le_bytes()).unwrap();
        let before = source.clone();
        let input = Inputs { source: &source, derived: &derived, bounds: Rect::new(0, 0, 32, 32) };
        assert!(prepare(input, config(0.5f64.to_bits()), Limits::default(), &mut Budget, &Interrupt::NONE).is_err());
        assert!(prepare(input, config(0), Limits { resident_bytes: 0, ..Limits::default() }, &mut Budget, &Interrupt::NONE).is_err());
        assert!(matches!(
            prepare(input, config(0), Limits::default(), &mut Budget, &Interrupt::cancel_only(&|| true)),
            Err(CompositionError::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source, before);
    }
}
