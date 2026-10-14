//! Explicit-domain, private saved-channel Gaussian preparation.
//!
//! This is not source admission, a filter-stack evaluator, or a destructive Bake
//! command. The caller establishes the channel's native space/encoding from
//! independent evidence, not parent-image encoding or a live format alias. The
//! observed reopened domains are sRGB Y' u8 and sRGB Y double. The mathematical
//! leaf operates on linear Gray; output rounding is an explicit derived policy.
//! Original native state remains exact. Derived matched-profile rendering uses
//! the existing <=2/255 contract, not an invented bit-exact Bake requirement.
//!
//! The enclosing owner must retain and charge ALL borrowed node/mask/opaque and
//! profile/container backing. Local requirements additionally charge source
//! raster, conservative private tile/map stages, conversion planes and work.
//! Existing raster identity/COW and leaf map clones are bounded provider
//! boundaries, not a claim that every allocator or RSS is controlled.
use crate::gimp_composite::{CompositionError, Result};
use crate::gimp_render::PreparationBudget;
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianLimits, GaussianParams, try_gaussian_finite_clamp};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{xcf_filter_masks::FilterMask, xcf_filters::FilterNode, xcf_provenance::OpaqueRecord};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};

/// Explicit caller-established sRGB Gray encoding; no implicit profile default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDomain {
    SrgbNonlinearU8,
    SrgbLinearF64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerivedPrecision {
    F64,
    /// Round newly derived linear samples through f32, then widen. This is not
    /// asserted to reproduce GIMP's complete intermediate rounding schedule.
    FinalF32,
}
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub domain: NativeDomain,
    pub precision: DerivedPrecision,
    /// Explicit independently bound parameters, never decoded from opaque enums.
    pub gaussian: GaussianParams,
    pub filter: GaussianFilter,
    pub output: Rect,
}
#[derive(Clone, Copy)]
pub struct Original<'a> {
    pub surface: &'a Surface,
    pub bounds: Rect,
    pub node: &'a FilterNode,
    /// Borrowed unconsumed record: no layer-mask multiplication or resizing.
    pub mask: Option<&'a FilterMask>,
    pub opaque: &'a [OpaqueRecord],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            resident_bytes: if cfg!(target_arch = "wasm32") { 64 << 20 } else { 256 << 20 },
            index_bytes: if cfg!(target_arch = "wasm32") { 16 << 20 } else { 64 << 20 },
            work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 },
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub source_resident_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
/// Private stages and accounting lease. No mutable access, installation method,
/// qualification token, or implicit original-node replacement is provided.
pub struct DerivedOutput<'a, L> {
    original: Original<'a>,
    surface: Surface,
    config: Config,
    requirements: Requirements,
    _lease: L,
}
/// Owned private stage. Dropping the borrowed record view does not confer
/// publication authority; the enclosing caller must retain its original owner.
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
impl<L> DerivedOutput<'_, L> {
    /// Move private output/accounting without cloning native pixels or records.
    /// Original snapshot ownership and any target fence remain caller duties.
    pub fn into_staged(self) -> StagedOutput<L> {
        StagedOutput { surface: self.surface, config: self.config, requirements: self.requirements, _lease: self._lease }
    }
    pub fn original(&self) -> Original<'_> {
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
fn poll(control: &Interrupt<'_>) -> Result<()> {
    if control.cancelled() {
        return Err(RasterInputError::Cancelled.into());
    }
    Ok(())
}
/// Allocation-free local admission and selected native sample validation;
/// retained original filter records are not
/// interpreted or validated as executable source here. The caller must bind
/// those records separately before selecting this explicit numerical candidate.
pub fn requirements(original: Original<'_>, config: Config, limits: Limits, control: &Interrupt<'_>) -> Result<Requirements> {
    poll(control)?;
    let standard = Limits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.index_bytes > standard.index_bytes || limits.work_units > standard.work_units {
        return Err(limit());
    }
    let expected = match config.domain {
        NativeDomain::SrgbNonlinearU8 => PixelFormat::GRAY8,
        NativeDomain::SrgbLinearF64 => PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
    };
    // Initial observed full-line geometry, not an arbitrary general graph lane.
    if original.bounds != Rect::new(0, 0, 32, 32)
        || config.output.is_empty()
        || config.output.intersect(&original.bounds) != config.output
        || original.surface.format() != expected
        || original.surface.tile_count() > 1
        || !matches!(config.filter, GaussianFilter::Auto)
        || !matches!(config.gaussian.abyss, GaussianAbyss::Clamp)
        || !config.gaussian.clip_extent
        || !(config.gaussian.std_dev_x == 1.0 || config.gaussian.std_dev_x == 3.75)
        || config.gaussian.std_dev_y != 1.0
    {
        return Err(CompositionError::Unsupported("explicit channel Gaussian domain or geometry"));
    }
    original.surface.try_validate_tile_origins_with(control)?;
    for (coordinate, tile) in original.surface.tiles() {
        poll(control)?;
        if *coordinate != photocraft_geom::TileCoord::new(0, 0) || tile.bytes().len() != 256 * 256 * expected.bytes_per_pixel() {
            return Err(CompositionError::Invalid("channel native tile descriptor"));
        }
    }
    let source = original.surface.resident_accounted_bytes()?;
    // Sixteen full F64 tile bodies cover working input, leaf COW/read/write
    // stages, native output COW and conversion scratch simultaneously. Thirty
    // two map entries cover the bounded one-tile provider map clones. Plane
    // capacities are reconciled against the separate eight-plane allowance.
    let index = 32 * 256;
    let peak =
        source.checked_mul(3).and_then(|n| n.checked_add(16 * 256 * 256 * 8)).and_then(|n| n.checked_add(8 * 1024 * 8 + 8192 + index)).ok_or_else(limit)?;
    // Includes both IIR axes (leaf128/sample), conversions, region reads and
    // worst-case full-tile COW/copy passes. No input/work refund is performed.
    let work = 1024 * 384 + 256 * 256 * 96;
    if peak > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(limit());
    }
    // Validate only the full input extent used by the finite leaf. Unselected
    // defaults/off-canvas native words (including NaNs) stay uninterpreted.
    let mut sample = [0_f64];
    for y in original.bounds.y0..original.bounds.y1 {
        poll(control)?;
        for x in original.bounds.x0..original.bounds.x1 {
            original.surface.try_read_pixel_f64(x, y, &mut sample)?;
            if sample.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(RasterInputError::InvalidSample.into());
            }
        }
    }
    poll(control)?;
    Ok(Requirements { source_resident_bytes: source, peak_resident_bytes: peak, index_bytes: index, work_units: work })
}
fn decode(value: f64, domain: NativeDomain) -> f64 {
    match domain {
        NativeDomain::SrgbLinearF64 => value,
        NativeDomain::SrgbNonlinearU8 if value <= 0.04045 => value / 12.92,
        NativeDomain::SrgbNonlinearU8 => ((value + 0.055) / 1.055).powf(2.4),
    }
}
fn encode(value: f64, domain: NativeDomain) -> f64 {
    match domain {
        NativeDomain::SrgbLinearF64 => value,
        NativeDomain::SrgbNonlinearU8 if value <= 0.0031308 => value * 12.92,
        NativeDomain::SrgbNonlinearU8 => 1.055 * value.powf(1. / 2.4) - 0.055,
    }
}
pub fn prepare<'a, B: PreparationBudget>(
    original: Original<'a>,
    config: Config,
    limits: Limits,
    budget: &mut B,
    control: &Interrupt<'_>,
) -> Result<DerivedOutput<'a, B::Lease>> {
    let req = requirements(original, config, limits, control)?;
    budget.consume_preparation_work(req.work_units)?;
    let lease = budget.reserve_prepared_bytes(req.peak_resident_bytes)?;
    poll(control)?;
    let mut values = original.surface.try_read_region_f64_with(original.bounds, control)?;
    if values.capacity().checked_mul(8).ok_or_else(limit)? > 8 * 1024 * 8 {
        return Err(limit());
    }
    for (index, value) in values.iter_mut().enumerate() {
        if index.is_multiple_of(32) {
            poll(control)?;
        }
        if !value.is_finite() || !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample.into());
        }
        *value = decode(*value, config.domain);
    }
    let wide = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut working = Surface::try_with_default_bytes(wide, &0_f64.to_le_bytes())?;
    working.try_write_region_f64_with(original.bounds, &values, control)?;
    drop(values);
    let filtered = try_gaussian_finite_clamp(
        &working,
        config.output,
        original.bounds,
        config.gaussian,
        config.filter,
        GaussianLimits { resident_bytes: limits.resident_bytes, work_units: limits.work_units },
        control,
    )?;
    let mut values = filtered.try_read_region_f64_with(config.output, control)?;
    if values.capacity().checked_mul(8).ok_or_else(limit)? > 8 * 1024 * 8 {
        return Err(limit());
    }
    for (index, value) in values.iter_mut().enumerate() {
        if index.is_multiple_of(32) {
            poll(control)?;
        }
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample.into());
        }
        if config.precision == DerivedPrecision::FinalF32 {
            *value = f64::from(*value as f32);
        }
        *value = encode(*value, config.domain);
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample.into());
        }
    }
    // Identity COW shares original encoded words including default, off-canvas
    // tails and other coordinates. Only newly computed ROI samples are encoded.
    let mut output = original.surface.try_convert_with(original.surface.format(), control)?;
    output.try_write_region_f64_with(config.output, &values, control)?;
    if output.resident_accounted_bytes()? > req.peak_resident_bytes {
        return Err(limit());
    }
    poll(control)?;
    Ok(DerivedOutput { original, surface: output, config, requirements: req, _lease: lease })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        item_identity::{FilterId, MaskId},
        xcf_filters::OperationIdentity,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Budget {
        bytes: u64,
        work: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = u64;
        fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<u64> {
            self.bytes = bytes;
            Ok(bytes)
        }
        fn consume_preparation_work(&mut self, work: u64) -> Result<()> {
            self.work = work;
            Ok(())
        }
    }
    // Numerical synthetic fixture only: this deliberately does not pretend to
    // establish the original-file SourceBinding or a source evaluator epoch.
    fn node() -> FilterNode {
        FilterNode {
            id: FilterId(7),
            operation: OperationIdentity { name: "retained opaque evidence".into(), stored_version: "0:0".into(), schema_epoch: "unresolved".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: true,
            opacity_f64_bits: 0.5_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(1),
            mask: None,
            auxiliary: vec![],
            composition: None,
        }
    }
    fn config() -> Config {
        Config {
            domain: NativeDomain::SrgbLinearF64,
            precision: DerivedPrecision::F64,
            gaussian: GaussianParams { std_dev_x: 3.75, std_dev_y: 1., abyss: GaussianAbyss::Clamp, clip_extent: true },
            filter: GaussianFilter::Auto,
            output: Rect::new(8, 8, 24, 24),
        }
    }
    fn source() -> Surface {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut surface = Surface::try_with_default_bytes(format, &0x7ff8_0000_0000_0042_u64.to_le_bytes()).unwrap();
        let values: Vec<f64> = (0..1024).map(|i| if i % 32 < 16 { f64::from_bits(0x3fd0_0000_0000_0001) } else { 0.75 }).collect();
        surface.try_write_region_f64_with(Rect::new(0, 0, 32, 32), &values, &Interrupt::NONE).unwrap();
        surface
    }
    #[test]
    fn original_default_tail_and_mask_are_unconsumed() {
        let source = source();
        let before = source.clone();
        let node = node();
        let mask = FilterMask { id: MaskId(8), owner: node.id, surface: Surface::new(PixelFormat::GRAY8), bounds: Rect::new(0, 0, 96, 80) };
        let original = Original { surface: &source, bounds: Rect::new(0, 0, 32, 32), node: &node, mask: Some(&mask), opaque: &[] };
        let mut budget = Budget::default();
        let result = prepare(original, config(), Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(source, before);
        assert_eq!(result.surface().default_bytes(), before.default_bytes());
        for (x, y) in [(0, 0), (31, 31), (100, 100)] {
            let mut a = [0; 8];
            let mut b = [0; 8];
            before.try_read_pixel_bytes(x, y, &mut a).unwrap();
            result.surface().try_read_pixel_bytes(x, y, &mut b).unwrap();
            assert_eq!(a, b);
        }
        assert!(std::ptr::eq(result.original().node, &node));
        assert!(std::ptr::eq(result.original().mask.unwrap(), &mask));
        assert_eq!(budget.bytes, result.requirements().peak_resident_bytes);
        assert_eq!(budget.work, result.requirements().work_units);
    }
    #[test]
    fn lower_budget_and_wrong_explicit_encoding_refuse_before_reservation() {
        let source = source();
        let node = node();
        let original = Original { surface: &source, bounds: Rect::new(0, 0, 32, 32), node: &node, mask: None, opaque: &[] };
        let mut budget = Budget::default();
        assert!(prepare(original, config(), Limits { resident_bytes: 1, ..Limits::default() }, &mut budget, &Interrupt::NONE).is_err());
        let wrong = Config { domain: NativeDomain::SrgbNonlinearU8, ..config() };
        assert!(prepare(original, wrong, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.bytes, 0);
        assert_eq!(budget.work, 0);
    }
    #[test]
    fn cancellation_during_private_leaf_keeps_native_source_exact() {
        let source = source();
        let before = source.clone();
        let node = node();
        let original = Original { surface: &source, bounds: Rect::new(0, 0, 32, 32), node: &node, mask: None, opaque: &[] };
        let calls = AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= 100;
        let result = prepare(original, config(), Limits::default(), &mut Budget::default(), &Interrupt::cancel_only(&cancel));
        assert!(matches!(result, Err(CompositionError::Raster(RasterInputError::Cancelled))));
        assert_eq!(source, before);
    }
}
