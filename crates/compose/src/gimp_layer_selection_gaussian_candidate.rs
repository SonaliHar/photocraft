//! Private layer/selection numerical candidate, separate from channel encoding.
//! Standard sRGB decoding/encoding is an explicit caller choice supported only
//! as a matched-display approximation (<=2/255), not an ICC alias or unique GIMP
//! Float schedule. Original source/mask/opaque ownership and qualification stay
//! with the enclosing caller. No mask normalization, filter admission or edit.
//! Raster COW/leaf internals remain bounded provider allocation boundaries.
use crate::gimp_channel_gaussian_candidate::{DerivedPrecision, Limits};
use crate::gimp_composite::{CompositionError, Result};
use crate::gimp_render::PreparationBudget;
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianLimits, GaussianParams, try_gaussian_finite_clamp};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_write_sample_f64};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, RasterInputError, Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDomain {
    SrgbNonlinearU8,
    SrgbNonlinearF64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskDomain {
    NativeNonlinearU8,
    NativeLinearF64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawBinary {
    /// Actual all-positive-zero native mask is required, then filtering is unrestricted.
    EmptySelectionUnrestricted,
    /// Exact caller-declared raw endpoints; no arithmetic coverage conversion.
    BinaryCoverage { zero_word: u64, full_word: u64 },
}
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub source_domain: SourceDomain,
    pub mask_domain: MaskDomain,
    pub mask_policy: RawBinary,
    pub precision: DerivedPrecision,
    pub gaussian: GaussianParams,
    pub filter: GaussianFilter,
    pub output: Rect,
}
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub source: &'a Surface,
    pub mask: &'a Surface,
    pub bounds: Rect,
    pub mask_bounds: Rect,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_resident_bytes: u64,
    pub mask_resident_bytes: u64,
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
/// Owned private output and accounting lease, with no retained borrowed Inputs.
/// The enclosing caller must keep original source/mask owners and fences alive;
/// moving this stage conveys no publication or source/filter authority.
pub struct StagedOutput<L> {
    surface: Surface,
    config: Config,
    requirements: Requirements,
    _lease: L,
}
impl<L> StagedOutput<L> {
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
    /// Move the native output and lease without cloning any source/mask pixels.
    /// Original and mask ownership remain explicit enclosing-caller duties.
    pub fn into_staged(self) -> StagedOutput<L> {
        StagedOutput { surface: self.surface, config: self.config, requirements: self.requirements, _lease: self._lease }
    }
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
const EXTENT: Rect = Rect::new(0, 0, 32, 32);
const FULL_F64: u64 = 1.0000000361224046_f64.to_bits();
const LEAF_RESIDENT: u64 = 8 * 256 * 256 * 8 + 65536;
const LEAF_WORK: u64 = 1024 * 256;
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
fn fmt(d: SourceDomain) -> PixelFormat {
    PixelFormat::new(
        ColorMode::Grayscale,
        match d {
            SourceDomain::SrgbNonlinearU8 => SampleType::U8,
            SourceDomain::SrgbNonlinearF64 => SampleType::F64,
        },
        false,
    )
}
fn endpoints(config: Config) -> Result<(u64, u64)> {
    let expected = match config.mask_domain {
        MaskDomain::NativeNonlinearU8 => (0, 255),
        MaskDomain::NativeLinearF64 => (0, FULL_F64),
    };
    match config.mask_policy {
        RawBinary::EmptySelectionUnrestricted => Ok(expected),
        RawBinary::BinaryCoverage { zero_word, full_word } if (zero_word, full_word) == expected => Ok(expected),
        _ => Err(CompositionError::Unsupported("declared raw selection endpoints")),
    }
}
/// Structural/quota admission only; sample/mask checks occur under the held lease.
pub fn requirements(input: Inputs<'_>, config: Config, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    let max = Limits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(limit());
    }
    endpoints(config)?;
    if input.bounds != EXTENT
        || input.mask_bounds != EXTENT
        || config.output.is_empty()
        || config.output.intersect(&EXTENT) != config.output
        || !matches!(
            (config.source_domain, config.mask_domain),
            (SourceDomain::SrgbNonlinearU8, MaskDomain::NativeNonlinearU8) | (SourceDomain::SrgbNonlinearF64, MaskDomain::NativeLinearF64)
        )
        || input.source.format() != fmt(config.source_domain)
        || input.mask.format() != fmt(config.source_domain)
        || input.source.tile_count() > 1
        || input.mask.tile_count() > 1
        || config.filter != GaussianFilter::Auto
        || config.gaussian.abyss != GaussianAbyss::Clamp
        || !config.gaussian.clip_extent
        || !(config.gaussian.std_dev_x == 1.0 || config.gaussian.std_dev_x == 3.75)
        || config.gaussian.std_dev_y != 1.0
    {
        return Err(CompositionError::Unsupported("explicit layer/selection Gaussian domain"));
    }
    for surface in [input.source, input.mask] {
        surface.try_validate_tile_origins_with(c)?;
        for (p, t) in surface.tiles() {
            poll(c)?;
            if *p != TileCoord::new(0, 0) || t.bytes().len() != 256 * 256 * surface.format().bytes_per_pixel() {
                return Err(CompositionError::Invalid("layer/selection native tile descriptor"));
            }
        }
    }
    let source = input.source.resident_accounted_bytes()?;
    let mask = input.mask.resident_accounted_bytes()?;
    let index = 32 * 256;
    let scratch = 8 * 1024 * 8;
    // Complete simultaneous source/mask/working/filter/output/leaf maps and planes.
    let peak = add(add(source, mask)?, add(16 * 256 * 256 * 8, add(scratch, index + 8192)?)?)?;
    let work = add(LEAF_WORK, 1024 * 512 + 256 * 256 * 96)?;
    if peak > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(limit());
    }
    poll(c)?;
    Ok(Requirements {
        source_resident_bytes: source,
        mask_resident_bytes: mask,
        peak_resident_bytes: peak,
        index_bytes: index,
        scratch_bytes: scratch,
        work_units: work,
    })
}
fn raw_word(mask: &Surface, x: i32, y: i32, c: &Interrupt<'_>) -> Result<u64> {
    poll(c)?;
    let mut bytes = [0u8; 8];
    let n = mask.format().bytes_per_pixel();
    mask.try_read_pixel_bytes(x, y, bytes.get_mut(..n).ok_or(RasterInputError::LengthMismatch)?)?;
    match mask.format().sample {
        SampleType::U8 => Ok(u64::from(bytes[0])),
        SampleType::F64 => Ok(u64::from_ne_bytes(bytes)),
        _ => Err(CompositionError::Unsupported("raw selection sample type")),
    }
}
fn scan_mask(input: Inputs<'_>, config: Config, c: &Interrupt<'_>) -> Result<bool> {
    let (zero, full) = endpoints(config)?;
    let mut any = false;
    for y in 0..32 {
        poll(c)?;
        for x in 0..32 {
            let word = raw_word(input.mask, x, y, c)?;
            match config.mask_policy {
                RawBinary::EmptySelectionUnrestricted if word == zero => (),
                RawBinary::BinaryCoverage { .. } if word == zero => (),
                RawBinary::BinaryCoverage { .. } if word == full => {
                    if config.output.contains(x, y) {
                        any = true;
                    }
                }
                _ => return Err(CompositionError::Unsupported("mask is not declared raw binary selection")),
            }
        }
    }
    Ok(matches!(config.mask_policy, RawBinary::EmptySelectionUnrestricted) || any)
}
fn decode(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
fn encode(v: f64) -> f64 {
    if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}
/// Full source line is decoded for the blur halo; coverage-zero output pixels
/// are never re-encoded. Raw F64 mask overshoot is a literal endpoint only.
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
    let apply = scan_mask(input, config, c)?;
    if !apply {
        let surface = input.source.try_convert_with(input.source.format(), c)?;
        poll(c)?;
        return Ok(PrivateOutput { original: input, surface, config, requirements: req, _lease: lease });
    }
    // Source arithmetic is closed finite-unit. Full source is needed for the
    // halo, but native defaults/off-canvas words remain uninterpreted. Newly
    // computed F64 output is finite-only and never silently clamped.
    let mut sample = [0.0];
    for y in 0..32 {
        poll(c)?;
        for x in 0..32 {
            input.source.try_read_pixel_f64(x, y, &mut sample)?;
            if !sample[0].is_finite() || !(0.0..=1.0).contains(&sample[0]) {
                return Err(RasterInputError::InvalidSample.into());
            }
            let v = decode(sample[0]);
            if !v.is_finite() {
                return Err(RasterInputError::InvalidSample.into());
            }
        }
    }
    let mut values = input.source.try_read_region_f64_with(EXTENT, c)?;
    if mul(values.capacity() as u64, 8)? > req.scratch_bytes {
        return Err(limit());
    }
    for (index, value) in values.iter_mut().enumerate() {
        if index.is_multiple_of(32) {
            poll(c)?;
        }
        *value = decode(*value);
    }
    let wide = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut working = Surface::try_with_default_bytes(wide, &0f64.to_ne_bytes())?;
    working.try_write_region_f64_with(EXTENT, &values, c)?;
    drop(values);
    let filtered = try_gaussian_finite_clamp(
        &working,
        config.output,
        EXTENT,
        config.gaussian,
        config.filter,
        GaussianLimits { resident_bytes: LEAF_RESIDENT, work_units: LEAF_WORK },
        c,
    )?;
    let mut bytes = input.source.try_read_interleaved_with(config.output, c)?;
    if bytes.capacity() as u64 > req.scratch_bytes {
        return Err(limit());
    }
    let (_, full) = endpoints(config)?;
    let mut index = 0usize;
    for y in config.output.y0..config.output.y1 {
        poll(c)?;
        for x in config.output.x0..config.output.x1 {
            let selected = matches!(config.mask_policy, RawBinary::EmptySelectionUnrestricted) || raw_word(input.mask, x, y, c)? == full;
            if selected {
                filtered.try_read_pixel_f64(x, y, &mut sample)?;
                let mut v = sample[0];
                if config.precision == DerivedPrecision::FinalF32 {
                    v = f64::from(v as f32);
                }
                v = encode(v);
                if !v.is_finite() {
                    return Err(RasterInputError::InvalidSample.into());
                }
                try_write_sample_f64(&mut bytes, input.source.format().sample, index, v).map_err(|_| RasterInputError::InvalidSample)?;
            }
            index = index.checked_add(1).ok_or_else(limit)?;
        }
    }
    let mut surface = input.source.try_convert_with(input.source.format(), c)?;
    surface.try_write_interleaved_with(config.output, &bytes, c)?;
    if add(add(req.source_resident_bytes, req.mask_resident_bytes)?, surface.resident_accounted_bytes()?)? > req.peak_resident_bytes {
        return Err(limit());
    }
    poll(c)?;
    Ok(PrivateOutput { original: input, surface, config, requirements: req, _lease: lease })
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
    fn config(policy: RawBinary) -> Config {
        Config {
            source_domain: SourceDomain::SrgbNonlinearF64,
            mask_domain: MaskDomain::NativeLinearF64,
            mask_policy: policy,
            precision: DerivedPrecision::F64,
            gaussian: GaussianParams { std_dev_x: 1.0, std_dev_y: 1.0, abyss: GaussianAbyss::Clamp, clip_extent: true },
            filter: GaussianFilter::Auto,
            output: EXTENT,
        }
    }
    #[test]
    fn binary_zero_is_raw_identity_and_not_unrestricted() {
        let f = fmt(SourceDomain::SrgbNonlinearF64);
        let nan = f64::from_bits(0x7ff8000000000123);
        let source = Surface::try_with_default_bytes(f, &nan.to_ne_bytes()).unwrap();
        let mask = Surface::try_with_default_bytes(f, &0f64.to_ne_bytes()).unwrap();
        let input = Inputs { source: &source, mask: &mask, bounds: EXTENT, mask_bounds: EXTENT };
        let policy = RawBinary::BinaryCoverage { zero_word: 0, full_word: FULL_F64 };
        let out = prepare(input, config(policy), Limits::default(), &mut Budget, &Interrupt::NONE).unwrap();
        assert_eq!(out.surface(), &source);
        assert_eq!(out.original().mask, &mask);
        assert!(prepare(input, config(RawBinary::EmptySelectionUnrestricted), Limits::default(), &mut Budget, &Interrupt::NONE).is_err());
    }
    #[test]
    fn raw_full_mask_preserves_zero_coverage_words_and_mask_overshoot() {
        let f = fmt(SourceDomain::SrgbNonlinearF64);
        let nan = f64::from_bits(0x7ff8000000000123);
        let mut source = Surface::try_with_default_bytes(f, &nan.to_ne_bytes()).unwrap();
        source.try_write_region_f64(EXTENT, &vec![0.25; 1024]).unwrap();
        source.try_write_interleaved(Rect::new(0, 0, 1, 1), &(-0f64).to_ne_bytes()).unwrap();
        let mut mask = Surface::try_with_default_bytes(f, &0f64.to_ne_bytes()).unwrap();
        mask.try_write_interleaved(Rect::new(31, 0, 32, 1), &FULL_F64.to_ne_bytes()).unwrap();
        let original_mask = mask.clone();
        let input = Inputs { source: &source, mask: &mask, bounds: EXTENT, mask_bounds: EXTENT };
        let out =
            prepare(input, config(RawBinary::BinaryCoverage { zero_word: 0, full_word: FULL_F64 }), Limits::default(), &mut Budget, &Interrupt::NONE).unwrap();
        let mut word = [0u8; 8];
        out.surface().try_read_pixel_bytes(0, 0, &mut word).unwrap();
        assert_eq!(word, (-0f64).to_ne_bytes());
        out.surface().try_read_pixel_bytes(-1, 0, &mut word).unwrap();
        assert_eq!(word, nan.to_ne_bytes());
        assert_eq!(mask, original_mask);
        assert_eq!(raw_word(&mask, 31, 0, &Interrupt::NONE).unwrap(), FULL_F64);
    }
    #[test]
    fn unsupported_context_and_nonunit_source_refuse() {
        let f = fmt(SourceDomain::SrgbNonlinearF64);
        let source = Surface::try_with_default_bytes(f, &2f64.to_ne_bytes()).unwrap();
        let mask = Surface::try_with_default_bytes(f, &0f64.to_ne_bytes()).unwrap();
        let input = Inputs { source: &source, mask: &mask, bounds: EXTENT, mask_bounds: EXTENT };
        let c = config(RawBinary::EmptySelectionUnrestricted);
        assert!(prepare(input, c, Limits::default(), &mut Budget, &Interrupt::NONE).is_err());
        let mut wrong = c;
        wrong.mask_domain = MaskDomain::NativeNonlinearU8;
        assert!(requirements(input, wrong, Limits::default(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn half_or_wrong_full_endpoint_and_cancel_refuse_without_mutation() {
        let f = fmt(SourceDomain::SrgbNonlinearF64);
        let source = Surface::try_with_default_bytes(f, &0.5f64.to_ne_bytes()).unwrap();
        let mask = Surface::try_with_default_bytes(f, &0.5f64.to_ne_bytes()).unwrap();
        let before = mask.clone();
        let input = Inputs { source: &source, mask: &mask, bounds: EXTENT, mask_bounds: EXTENT };
        assert!(
            prepare(input, config(RawBinary::BinaryCoverage { zero_word: 0, full_word: FULL_F64 }), Limits::default(), &mut Budget, &Interrupt::NONE).is_err()
        );
        assert!(
            requirements(input, config(RawBinary::BinaryCoverage { zero_word: 0, full_word: 1f64.to_bits() }), Limits::default(), &Interrupt::NONE).is_err()
        );
        assert!(matches!(
            prepare(input, config(RawBinary::EmptySelectionUnrestricted), Limits::default(), &mut Budget, &Interrupt::cancel_only(&|| true)),
            Err(CompositionError::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(mask, before);
    }
}
