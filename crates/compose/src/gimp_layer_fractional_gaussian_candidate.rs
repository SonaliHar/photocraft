//! Explicit fractional-coverage Gray display mathematics, not source admission.
//! Author/fresh U8 edited reference HOLD (64 edge words differ by one code) stays
//! separate. This candidate does not identify a unique GIMP/Babl Float schedule,
//! alias native/effective ICCs, normalize retained masks or relax native equality.
//! Source/opaque/profile ownership remains with the enclosing caller. The display
//! contract is <=2/255 only when independently established for that caller's case.
//! Raster/finite-leaf private allocator internals remain bounded provider seams.
use crate::gimp_channel_gaussian_candidate::{DerivedPrecision, Limits};
use crate::gimp_composite::{CompositionError, Result};
use crate::gimp_layer_selection_gaussian_candidate as binary;
use crate::gimp_render::PreparationBudget;
pub use binary::{Inputs, MaskDomain, SourceDomain};
use photocraft_algo::gimp::neighborhood::{GaussianFilter, GaussianParams};
use photocraft_color::try_write_sample_f64;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoveragePrecision {
    NativeF64,
    RoundedF32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixSpace {
    RawEncodedGray,
    SrgbDecodedLinear,
}
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub source_domain: SourceDomain,
    pub mask_domain: MaskDomain,
    pub gaussian: GaussianParams,
    pub filter: GaussianFilter,
    pub gaussian_precision: DerivedPrecision,
    pub coverage_precision: CoveragePrecision,
    pub mix_space: MixSpace,
    pub mix_precision: DerivedPrecision,
    pub output: Rect,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub gaussian: binary::Requirements,
    pub zero_mask_reserve_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub work_units: u64,
}
/// Readonly private output, owning both stages and the enclosing lease. No
/// surface mutation/installation/proof token or implicit original rewrite API.
pub struct PrivateOutput<'a, L> {
    original: Inputs<'a>,
    unmasked: Option<binary::StagedOutput<()>>,
    surface: Surface,
    config: Config,
    requirements: Requirements,
    _lease: L,
}
/// Owned private intermediate and final output. Removing the borrowed Inputs
/// view does not authorize publication or source/selection changes. The enclosing
/// caller retains original source/mask owners and captured-target fences.
pub struct StagedOutput<L> {
    unmasked: Option<binary::StagedOutput<()>>,
    surface: Surface,
    config: Config,
    requirements: Requirements,
    _lease: L,
}
impl<L> StagedOutput<L> {
    pub fn unmasked_surface(&self) -> Option<&Surface> {
        self.unmasked.as_ref().map(binary::StagedOutput::surface)
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
impl<L> PrivateOutput<'_, L> {
    /// Move both native stages and their enclosing lease without cloning or
    /// retaining borrowed Inputs. Original ownership remains the caller's duty.
    pub fn into_staged(self) -> StagedOutput<L> {
        StagedOutput { unmasked: self.unmasked, surface: self.surface, config: self.config, requirements: self.requirements, _lease: self._lease }
    }
    pub fn original(&self) -> Inputs<'_> {
        self.original
    }
    pub fn unmasked_surface(&self) -> Option<&Surface> {
        self.unmasked.as_ref().map(binary::StagedOutput::surface)
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
const EXTENT: Rect = Rect::new(0, 0, 32, 32);
const ZERO_MASK_RESERVE: u64 = 8192;
const MIX_SCRATCH: u64 = 8 * 1024 * 8;
const MIX_INDEX: u64 = 32 * 256;
const MIX_WORK: u64 = 1024 * 1024 + 256 * 256 * 96;
fn limit() -> CompositionError {
    RasterInputError::LimitExceeded.into()
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn core(config: Config) -> binary::Config {
    binary::Config {
        source_domain: config.source_domain,
        mask_domain: config.mask_domain,
        mask_policy: binary::RawBinary::EmptySelectionUnrestricted,
        precision: config.gaussian_precision,
        gaussian: config.gaussian,
        filter: config.filter,
        output: config.output,
    }
}
/// Allocation-free admission of real mask plus private zero-mask/blur stage/final
/// COW/scratch/map owners. No caller-provided retention count is accepted.
pub fn requirements(input: Inputs<'_>, config: Config, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    let g = binary::requirements(input, core(config), limits, c)?;
    // Binary requirements is structural and does not assert this real mask is
    // empty. Its measured retention conservatively exceeds the private zero mask.
    let mask = input.mask.resident_accounted_bytes()?;
    let peak = add(g.peak_resident_bytes, add(mask, add(ZERO_MASK_RESERVE, add(4 * 256 * 256 * 8, MIX_SCRATCH + MIX_INDEX)?)?)?)?;
    let index = add(g.index_bytes, MIX_INDEX)?;
    let work = add(g.work_units, MIX_WORK)?;
    if peak > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(limit());
    }
    poll(c)?;
    Ok(Requirements {
        gaussian: g,
        zero_mask_reserve_bytes: ZERO_MASK_RESERVE,
        peak_resident_bytes: peak,
        index_bytes: index,
        scratch_bytes: MIX_SCRATCH,
        work_units: work,
    })
}
fn mask_value(mask: &Surface, x: i32, y: i32, config: Config, c: &Interrupt<'_>) -> Result<f64> {
    poll(c)?;
    let mut v = [0.0];
    mask.try_read_pixel_f64(x, y, &mut v)?;
    if !v[0].is_finite() || !(0.0..=1.0).contains(&v[0]) {
        return Err(RasterInputError::InvalidSample.into());
    }
    Ok(match config.coverage_precision {
        CoveragePrecision::NativeF64 => v[0],
        CoveragePrecision::RoundedF32 => f64::from(v[0] as f32),
    })
}
fn scan_mask(input: Inputs<'_>, config: Config, c: &Interrupt<'_>) -> Result<bool> {
    let mut active = false;
    for y in 0..32 {
        poll(c)?;
        for x in 0..32 {
            let v = mask_value(input.mask, x, y, config, c)?;
            if v != 0.0 && config.output.contains(x, y) {
                active = true;
            }
        }
    }
    Ok(active)
}
fn decode(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
fn encode(v: f64) -> f64 {
    if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}
struct LocalBudget {
    resident: u64,
    work: u64,
    resident_limit: u64,
    work_limit: u64,
}
impl PreparationBudget for LocalBudget {
    type Lease = ();
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<()> {
        self.resident = add(self.resident, n)?;
        if self.resident > self.resident_limit {
            return Err(limit());
        }
        Ok(())
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
        self.work = add(self.work, n)?;
        if self.work > self.work_limit {
            return Err(limit());
        }
        Ok(())
    }
}
/// Unfused binary64 weighted sum, with declared optional final Float32 rounding
/// in the selected mix space. Zero coverage retains exact original words; full
/// coverage copies exact unmasked derived words. U8 uses the existing native
/// writer's clamp/nearest rule. Newly computed F64 is finite, never clamped.
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
    if !scan_mask(input, config, c)? {
        let surface = input.source.try_convert_with(input.source.format(), c)?;
        poll(c)?;
        return Ok(PrivateOutput { original: input, unmasked: None, surface, config, requirements: req, _lease: lease });
    }
    // This literal temporary mask never replaces or proves the real selection.
    let zero = [0u8; 8];
    let n = input.mask.format().bytes_per_pixel();
    let zero_mask = Surface::try_with_default_bytes(input.mask.format(), zero.get(..n).ok_or(RasterInputError::LengthMismatch)?)?;
    if zero_mask.resident_accounted_bytes()? > req.zero_mask_reserve_bytes {
        return Err(limit());
    }
    let mut local = LocalBudget { resident: 0, work: 0, resident_limit: req.gaussian.peak_resident_bytes, work_limit: req.gaussian.work_units };
    let g = binary::prepare(
        binary::Inputs { source: input.source, mask: &zero_mask, bounds: input.bounds, mask_bounds: EXTENT },
        core(config),
        Limits { resident_bytes: req.gaussian.peak_resident_bytes, index_bytes: req.gaussian.index_bytes, work_units: req.gaussian.work_units },
        &mut local,
        c,
    )?
    .into_staged();
    // Both staged Gaussian and final native output stay live; no quota refunds.
    let mut bytes = input.source.try_read_interleaved_with(config.output, c)?;
    if bytes.capacity() as u64 > req.scratch_bytes {
        return Err(limit());
    }
    let mut a = [0.0];
    let mut b = [0.0];
    let stride = input.source.format().bytes_per_pixel();
    let mut pixel = 0usize;
    for y in config.output.y0..config.output.y1 {
        poll(c)?;
        for x in config.output.x0..config.output.x1 {
            let t = mask_value(input.mask, x, y, config, c)?;
            if t == 1.0 {
                let start = pixel.checked_mul(stride).ok_or_else(limit)?;
                let end = start.checked_add(stride).ok_or_else(limit)?;
                g.surface().try_read_pixel_bytes(x, y, bytes.get_mut(start..end).ok_or(RasterInputError::LengthMismatch)?)?;
            } else if t != 0.0 {
                input.source.try_read_pixel_f64(x, y, &mut a)?;
                g.surface().try_read_pixel_f64(x, y, &mut b)?;
                let (a, b) = match config.mix_space {
                    MixSpace::RawEncodedGray => (a[0], b[0]),
                    MixSpace::SrgbDecodedLinear => (decode(a[0]), decode(b[0])),
                };
                let mut v = a * (1.0 - t) + b * t;
                if config.mix_precision == DerivedPrecision::FinalF32 {
                    v = f64::from(v as f32);
                }
                if config.mix_space == MixSpace::SrgbDecodedLinear {
                    v = encode(v);
                }
                if !v.is_finite() {
                    return Err(RasterInputError::InvalidSample.into());
                }
                try_write_sample_f64(&mut bytes, input.source.format().sample, pixel, v).map_err(|_| RasterInputError::InvalidSample)?;
            }
            pixel = pixel.checked_add(1).ok_or_else(limit)?;
        }
    }
    let mut surface = input.source.try_convert_with(input.source.format(), c)?;
    surface.try_write_interleaved_with(config.output, &bytes, c)?;
    let known = add(
        add(req.gaussian.source_resident_bytes, req.gaussian.mask_resident_bytes)?,
        add(g.surface().resident_accounted_bytes()?, add(zero_mask.resident_accounted_bytes()?, surface.resident_accounted_bytes()?)?)?,
    )?;
    if known > req.peak_resident_bytes {
        return Err(limit());
    }
    poll(c)?;
    Ok(PrivateOutput { original: input, unmasked: Some(g), surface, config, requirements: req, _lease: lease })
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::gimp::neighborhood::GaussianAbyss;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
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
    fn config() -> Config {
        Config {
            source_domain: SourceDomain::SrgbNonlinearF64,
            mask_domain: MaskDomain::NativeLinearF64,
            gaussian: GaussianParams { std_dev_x: 1.0, std_dev_y: 1.0, abyss: GaussianAbyss::Clamp, clip_extent: true },
            filter: GaussianFilter::Auto,
            gaussian_precision: DerivedPrecision::F64,
            coverage_precision: CoveragePrecision::NativeF64,
            mix_space: MixSpace::SrgbDecodedLinear,
            mix_precision: DerivedPrecision::F64,
            output: EXTENT,
        }
    }
    #[test]
    fn zero_mask_is_raw_noop_even_for_nan_source_and_tiny_or_cancel_refuses() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let nan = f64::from_bits(0x7ff8000000001234);
        let s = Surface::try_with_default_bytes(fmt, &nan.to_ne_bytes()).unwrap();
        let m = Surface::try_with_default_bytes(fmt, &0f64.to_ne_bytes()).unwrap();
        let i = Inputs { source: &s, mask: &m, bounds: EXTENT, mask_bounds: EXTENT };
        let r = prepare(i, config(), Limits::default(), &mut Budget, &Interrupt::NONE).unwrap();
        assert_eq!(r.surface(), &s);
        assert!(r.unmasked_surface().is_none());
        assert!(prepare(i, config(), Limits { resident_bytes: 0, ..Limits::default() }, &mut Budget, &Interrupt::NONE).is_err());
        assert!(matches!(
            prepare(i, config(), Limits::default(), &mut Budget, &Interrupt::cancel_only(&|| true)),
            Err(CompositionError::Raster(RasterInputError::Cancelled))
        ));
    }
    #[test]
    fn malformed_coverage_and_source_fail_without_normalization() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let s = Surface::try_with_default_bytes(fmt, &0.25f64.to_ne_bytes()).unwrap();
        for v in [f64::NAN, -0.01, 1.0000000361224046] {
            let m = Surface::try_with_default_bytes(fmt, &v.to_ne_bytes()).unwrap();
            let before = m.clone();
            assert!(
                prepare(Inputs { source: &s, mask: &m, bounds: EXTENT, mask_bounds: EXTENT }, config(), Limits::default(), &mut Budget, &Interrupt::NONE)
                    .is_err()
            );
            assert_eq!(m, before);
        }
        let bad = Surface::try_with_default_bytes(fmt, &2f64.to_ne_bytes()).unwrap();
        let mask = Surface::try_with_default_bytes(fmt, &0.5f64.to_ne_bytes()).unwrap();
        assert!(
            prepare(Inputs { source: &bad, mask: &mask, bounds: EXTENT, mask_bounds: EXTENT }, config(), Limits::default(), &mut Budget, &Interrupt::NONE)
                .is_err()
        );
    }
    #[test]
    fn fractional_mix_preserves_zero_words_and_raw_mask_under_both_policies() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let nan = f64::from_bits(0x7ff8000000001234);
        let mut s = Surface::try_with_default_bytes(fmt, &nan.to_ne_bytes()).unwrap();
        let values: Vec<f64> = (0..1024).map(|i| if i % 32 < 16 { 0.25 } else { 0.75 }).collect();
        s.try_write_region_f64(EXTENT, &values).unwrap();
        s.try_write_interleaved(Rect::new(0, 0, 1, 1), &(-0f64).to_ne_bytes()).unwrap();
        let mut mask = Surface::try_with_default_bytes(fmt, &0f64.to_ne_bytes()).unwrap();
        mask.try_write_interleaved(Rect::new(15, 0, 16, 1), &0.5f64.to_ne_bytes()).unwrap();
        let before = mask.clone();
        let input = Inputs { source: &s, mask: &mask, bounds: EXTENT, mask_bounds: EXTENT };
        for policy in [MixSpace::RawEncodedGray, MixSpace::SrgbDecodedLinear] {
            let mut c = config();
            c.mix_space = policy;
            let out = prepare(input, c, Limits::default(), &mut Budget, &Interrupt::NONE).unwrap();
            let mut word = [0u8; 8];
            out.surface().try_read_pixel_bytes(0, 0, &mut word).unwrap();
            assert_eq!(word, (-0f64).to_ne_bytes());
            out.surface().try_read_pixel_bytes(-1, 0, &mut word).unwrap();
            assert_eq!(word, nan.to_ne_bytes());
            assert!(out.unmasked_surface().is_some());
            assert_eq!(mask, before);
        }
    }
}
