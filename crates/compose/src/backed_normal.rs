//! Explicit checked Normal composition over an owned-backed layer, in its caller-bound
//! current encoding. This is not a Document consumer or an import qualification.
use crate::gimp_composite::{CompositionError, WorkingSpace, encoding, space};
use photocraft_cms::{Profile, xcf_color::qualify_effective_analytic_profile};
use photocraft_doc::{
    BackedLayer, ColorMode, SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
};
use photocraft_geom::Rect;
use photocraft_raster::backing::quota::{QuotaError, QuotaKind, QuotaLease};
use photocraft_raster::backing::{BackingError, NativePlaneDescriptor, ReadControl};
use photocraft_raster::checked_view::RasterViewError;

#[derive(Debug, PartialEq, Eq)]
pub enum BackedNormalError {
    Unsupported(&'static str),
    Invalid(&'static str),
    Backing(BackingError),
    View(RasterViewError),
    Color(CompositionError),
    Quota(QuotaError),
    Allocation,
}
impl std::fmt::Display for BackedNormalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backed Normal: {self:?}")
    }
}
impl std::error::Error for BackedNormalError {}

#[derive(Clone, Copy, Debug)]
pub struct NormalBandLimits {
    pub max_rows: u32,
    pub max_pixels: u64,
}
impl Default for NormalBandLimits {
    fn default() -> Self {
        Self { max_rows: 256, max_pixels: 65_536 }
    }
}
/// Current-profile preparation owns one bounded metadata lease. Source/store/COW
/// owners remain borrowed, with their original persistent ledger leases intact.
#[derive(Debug)]
pub struct PreparedBackedNormal<'a> {
    layer: &'a BackedLayer,
    working: WorkingSpace,
    limits: NormalBandLimits,
    operator: GimpOperator,
    _lease: QuotaLease,
}
/// Complete returned band in the caller's current stored encoding, straight RGBA
/// f64 (Gray is repeated). Pixels are destroyed before their resident lease.
#[derive(Debug)]
pub struct OwnedNormalBand {
    rect: Rect,
    pixels: Vec<[f64; 4]>,
    lease: QuotaLease,
}
impl OwnedNormalBand {
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn pixels(&self) -> &[[f64; 4]] {
        &self.pixels
    }
    pub fn resident_bytes(&self) -> u64 {
        self.lease.bytes()
    }
}
enum Backdrop<'a> {
    Constant([f64; 4]),
    Varying(&'a [[f64; 4]]),
}
/// Independent operative Gray mask. Outside coverage is supplied explicitly;
/// disabled masks are never inspected or read.
#[derive(Clone, Copy, Debug)]
pub struct NormalMask<'a> {
    pub layer: &'a BackedLayer,
    pub enabled: bool,
    pub outside_coverage: f64,
}
/// Borrowed resident mask whose physical owner remains retained by its caller.
/// The checked view carries explicit world bounds; no Surface or tile map clone.
#[derive(Clone, Copy, Debug)]
pub struct ResidentNormalMask<'a> {
    pub view: photocraft_raster::checked_view::CheckedRasterView<'a>,
    pub enabled: bool,
    pub outside_coverage: f64,
}
impl PreparedBackedNormal<'_> {
    /// Explicit f64 opacity and numeric Gray coverage, without storage
    /// quantization, density, feathering or mask profile interpretation.
    pub fn render_band_with_coverage(
        &self,
        rect: Rect,
        backdrop: [f64; 4],
        opacity: f64,
        mask: Option<NormalMask<'_>>,
        control: &ReadControl<'_>,
    ) -> Result<OwnedNormalBand, BackedNormalError> {
        self.render(rect, Backdrop::Constant(backdrop), opacity, mask, None, control)
    }
    pub fn render_band_over_with_coverage(
        &self,
        rect: Rect,
        backdrop: &[[f64; 4]],
        opacity: f64,
        mask: Option<NormalMask<'_>>,
        control: &ReadControl<'_>,
    ) -> Result<OwnedNormalBand, BackedNormalError> {
        self.render(rect, Backdrop::Varying(backdrop), opacity, mask, None, control)
    }
    /// Caller profile is operative current authority, not immutable provenance.
    /// Only the existing bounded effective analytic RGB/Gray profile class is
    /// accepted; LUT/table curves and mismatched transfer/model are explicit errors.
    pub fn new<'a>(
        layer: &'a BackedLayer,
        profile: &Profile,
        stored: SampleEncoding,
        expected: NativePlaneDescriptor,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<PreparedBackedNormal<'a>, BackedNormalError> {
        Self::new_with_composite_space(layer, profile, stored, space(stored), expected, limits, control)
    }
    /// Explicit resolved current composite space. No Auto/provenance inference.
    /// RGB transfer spaces only; physical Lab is a separately unqualified path.
    pub fn new_with_composite_space<'a>(
        layer: &'a BackedLayer,
        profile: &Profile,
        stored: SampleEncoding,
        composite: GimpSpace,
        expected: NativePlaneDescriptor,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<PreparedBackedNormal<'a>, BackedNormalError> {
        Self::new_with_composite_operator(layer, profile, stored, composite, GimpOperator::Union, expected, limits, control)
    }
    /// Explicit typed current operator; it grants no raw-policy/source eligibility.
    /// Existing constructors remain Union. Working-space equations are shared.
    pub fn new_with_composite_operator<'a>(
        layer: &'a BackedLayer,
        profile: &Profile,
        stored: SampleEncoding,
        composite: GimpSpace,
        operator: GimpOperator,
        expected: NativePlaneDescriptor,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<PreparedBackedNormal<'a>, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        if !matches!(composite, GimpSpace::LinearRgb | GimpSpace::ProfileRgb | GimpSpace::PerceptualRgb) {
            return Err(BackedNormalError::Unsupported("analytic RGB transfer composite spaces only"));
        }
        if layer.descriptor() != expected {
            return Err(BackedNormalError::Invalid("operative descriptor mismatch"));
        }
        if !matches!(expected.format().mode, ColorMode::Rgb | ColorMode::Grayscale) {
            return Err(BackedNormalError::Unsupported("RGB/Gray Normal only"));
        }
        if limits.max_rows == 0 || limits.max_rows > 256 || limits.max_pixels == 0 || limits.max_pixels > 65_536 {
            return Err(BackedNormalError::Invalid("band limits"));
        }
        if profile.channels() != expected.format().mode.color_channels()
            || profile.color_space
                != match expected.format().mode {
                    ColorMode::Rgb => photocraft_cms::ColorSpace::Rgb,
                    _ => photocraft_cms::ColorSpace::Gray,
                }
        {
            return Err(BackedNormalError::Invalid("current profile model"));
        }
        if profile.description.len() > 65_536 || profile.copyright.len() > 65_536 {
            return Err(BackedNormalError::Unsupported("bounded profile strings"));
        }
        // Profile::clone also copies inactive public curve fields. Reject their
        // variable-size tables too, not merely the model's operative curves.
        if profile
            .trc
            .iter()
            .flatten()
            .chain(profile.gray_trc.iter())
            .any(|curve| matches!(curve, photocraft_cms::curve::Curve::Table(_) | photocraft_cms::curve::Curve::Table64(_)))
        {
            return Err(BackedNormalError::Unsupported("analytic profile curves only"));
        }
        // Qualification evaluates operative curves; WorkingSpace validates the
        // stored interpretation and (when different) its target interpretation.
        // Prepay each actual schedule through the CMS-owned O(1) ceiling, before
        // any semantic scan or constructor. No local work allowance is reset.
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(profile).map_err(cms_error)?;
        let cross_space = composite != space(stored);
        let preparation = semantic.checked_mul(if cross_space { 3 } else { 2 }).ok_or(BackedNormalError::Backing(BackingError::Length))?;
        control.check(preparation).map_err(BackedNormalError::Backing)?;
        qualify_effective_analytic_profile(profile, encoding(stored)).map_err(|e| BackedNormalError::Color(CompositionError::Color(e)))?;
        let strings = profile.description.len().checked_add(profile.copyright.len()).ok_or(BackedNormalError::Invalid("profile size"))?;
        // Cross-space preparation builds two analytic profile clones and
        // retains one transfer profile. Prepay bounded copies before cloning.
        let temporary = (strings as u64)
            .checked_mul(if cross_space { 6 } else { 2 })
            .and_then(|n| n.checked_add(if cross_space { 16_384 } else { 1024 }))
            .ok_or(BackedNormalError::Invalid("profile size"))?;
        let transient = layer.quota().reserve(QuotaKind::Resident, temporary).map_err(BackedNormalError::Quota)?;
        let retained = (std::mem::size_of::<WorkingSpace>() as u64)
            .checked_add(if cross_space { strings as u64 + std::mem::size_of::<Profile>() as u64 + 4096 } else { 1024 })
            .ok_or(BackedNormalError::Invalid("profile size"))?;
        let lease = layer.quota().reserve(QuotaKind::Resident, retained).map_err(BackedNormalError::Quota)?;
        control.check(strings as u64 + 1).map_err(BackedNormalError::Backing)?;
        let working = WorkingSpace::new_with_interrupt(profile, stored, composite, control.interrupt).map_err(|error| match error {
            CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled) => BackedNormalError::Backing(BackingError::Cancelled),
            other => BackedNormalError::Color(other),
        })?;
        // Analytic transfer spaces retain at most one bounded Profile; no LUT
        // pipelines or generic Transform allocations are admitted.
        drop(transient);
        control.check(1).map_err(BackedNormalError::Backing)?;
        Ok(PreparedBackedNormal { layer, working, limits, operator, _lease: lease })
    }
    pub fn render_band(&self, rect: Rect, backdrop: [f64; 4], control: &ReadControl<'_>) -> Result<OwnedNormalBand, BackedNormalError> {
        self.render(rect, Backdrop::Constant(backdrop), 1.0, None, None, control)
    }
    /// The caller owns/accounts the immutable varying backdrop, in exactly the
    /// same current profile/encoding. It must contain one pixel per requested cell.
    pub fn render_band_over(&self, rect: Rect, backdrop: &[[f64; 4]], control: &ReadControl<'_>) -> Result<OwnedNormalBand, BackedNormalError> {
        self.render(rect, Backdrop::Varying(backdrop), 1.0, None, None, control)
    }
    pub fn render_band_with_resident_coverage(
        &self,
        rect: Rect,
        backdrop: [f64; 4],
        opacity: f64,
        mask: Option<ResidentNormalMask<'_>>,
        control: &ReadControl<'_>,
    ) -> Result<OwnedNormalBand, BackedNormalError> {
        self.render(rect, Backdrop::Constant(backdrop), opacity, None, mask, control)
    }
    pub fn render_band_over_with_resident_coverage(
        &self,
        rect: Rect,
        backdrop: &[[f64; 4]],
        opacity: f64,
        mask: Option<ResidentNormalMask<'_>>,
        control: &ReadControl<'_>,
    ) -> Result<OwnedNormalBand, BackedNormalError> {
        self.render(rect, Backdrop::Varying(backdrop), opacity, None, mask, control)
    }
    fn render(
        &self,
        rect: Rect,
        backdrop: Backdrop<'_>,
        opacity: f64,
        mask: Option<NormalMask<'_>>,
        resident_mask: Option<ResidentNormalMask<'_>>,
        control: &ReadControl<'_>,
    ) -> Result<OwnedNormalBand, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        coverage(opacity)?;
        let mask = mask.filter(|m| m.enabled);
        if let Some(m) = mask {
            coverage(m.outside_coverage)?;
            let format = m.layer.descriptor().format();
            if format.mode != ColorMode::Grayscale || format.alpha {
                return Err(BackedNormalError::Unsupported("numeric Gray mask without alpha only"));
            }
            if !self.layer.quota().shares_ledger(m.layer.quota()) {
                return Err(BackedNormalError::Invalid("mask persistent ledger mismatch"));
            }
        }
        let resident_mask = resident_mask.filter(|m| m.enabled);
        if mask.is_some() && resident_mask.is_some() {
            return Err(BackedNormalError::Invalid("multiple mask sources"));
        }
        if let Some(m) = resident_mask {
            coverage(m.outside_coverage)?;
            let format = m.view.format();
            if format.mode != ColorMode::Grayscale || format.alpha {
                return Err(BackedNormalError::Unsupported("numeric resident Gray mask without alpha only"));
            }
        }
        let mask = mask.map(|m| (m.layer.checked_view(), m.outside_coverage)).or_else(|| resident_mask.map(|m| (m.view, m.outside_coverage)));
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or(BackedNormalError::Invalid("band size"))?;
        if rect.is_empty() || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(BackedNormalError::Invalid("band size"));
        }
        let count = usize::try_from(count).map_err(|_| BackedNormalError::Invalid("band size"))?;
        if let Backdrop::Varying(input) = &backdrop
            && input.len() != count
        {
            return Err(BackedNormalError::Invalid("backdrop length"));
        }
        let bytes = (count as u64).checked_mul(32).ok_or(BackedNormalError::Invalid("band size"))?;
        let lease = self
            .layer
            .quota()
            .reserve(QuotaKind::Resident, bytes.checked_add(64).ok_or(BackedNormalError::Invalid("band size"))?)
            .map_err(BackedNormalError::Quota)?;
        let mut output = OwnedNormalBand { rect, pixels: Vec::new(), lease };
        control.check(1).map_err(BackedNormalError::Backing)?;
        output.pixels.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
        let actual = (output.pixels.capacity() as u64).checked_mul(32).ok_or(BackedNormalError::Invalid("band capacity"))?;
        output.lease.reconcile_capacity(actual).map_err(BackedNormalError::Quota)?;
        let view = self.layer.checked_view();
        let format = view.format();
        let active = rect.intersect(&self.layer.operative_bounds());
        let clears_zero = matches!(self.operator, GimpOperator::ClipToLayer | GimpOperator::Intersection);
        if clears_zero {
            let active_count = u64::from(active.width()).checked_mul(u64::from(active.height())).ok_or(BackedNormalError::Invalid("active band size"))?;
            let visits =
                if opacity == 0.0 { count as u64 } else { (count as u64).checked_sub(active_count).ok_or(BackedNormalError::Invalid("active band size"))? };
            control.check(visits.checked_mul(128).ok_or(BackedNormalError::Backing(BackingError::Length))?).map_err(BackedNormalError::Backing)?;
        }
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                control.check(32).map_err(BackedNormalError::Backing)?;
                let bg = match &backdrop {
                    Backdrop::Constant(pixel) => *pixel,
                    Backdrop::Varying(input) => *input.get(output.pixels.len()).ok_or(BackedNormalError::Invalid("backdrop length"))?,
                };
                unit(bg)?;
                if format.mode == ColorMode::Grayscale && (bg[0] != bg[1] || bg[0] != bg[2]) {
                    return Err(BackedNormalError::Invalid("Gray backdrop must repeat its stored component"));
                }
                // Outside the operative drawable is zero source coverage, not
                // a skipped layer. ClipLayer/Intersection can therefore clear
                // backdrop even there. Use the existing compositor equations.
                let pixel = if clears_zero && (opacity == 0.0 || !active.contains(x, y)) {
                    self.working.normal(bg, [0.0; 4], 0.0, self.operator).map_err(BackedNormalError::Color)?
                } else {
                    bg
                };
                output.pixels.push(pixel);
            }
        }
        // Zero opacity has applied the operator to all backdrop cells without
        // reading source/mask samples. Union/ClipBackdrop retain their old path.
        // Unused disabled-mask fields are never inspected.
        if opacity == 0.0 {
            control.check(1).map_err(BackedNormalError::Backing)?;
            return Ok(output);
        }
        // Prepay mask pin owner and fixed sample/decode scratch before any pin.
        // Physical pinned tile allocations keep their own common-ledger leases.
        let _mask_scratch = if mask.is_some() {
            Some(
                self.layer
                    .quota()
                    .reserve(QuotaKind::Resident, (std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() as u64) + 128)
                    .map_err(BackedNormalError::Quota)?,
            )
        } else {
            None
        };
        let mut mask_pin: Option<photocraft_raster::checked_view::CheckedTilePin> = None;
        // Pin each intersecting source tile once, rather than repeatedly missing
        // a bounded cache on successive rows of a wide band. No pin-all staging.
        let mut raw = [0; 40];
        for coord in active.tiles() {
            let pin = view.pin_tile(coord, control).map_err(BackedNormalError::View)?;
            let x0 = coord.tx.checked_mul(photocraft_geom::TILE_SIZE).ok_or(BackedNormalError::Invalid("tile bounds"))?;
            let y0 = coord.ty.checked_mul(photocraft_geom::TILE_SIZE).ok_or(BackedNormalError::Invalid("tile bounds"))?;
            let x1 = x0.checked_add(photocraft_geom::TILE_SIZE).ok_or(BackedNormalError::Invalid("tile bounds"))?;
            let y1 = y0.checked_add(photocraft_geom::TILE_SIZE).ok_or(BackedNormalError::Invalid("tile bounds"))?;
            let tile = Rect::new(x0, y0, x1, y1).intersect(&active);
            for y in tile.y0..tile.y1 {
                for x in tile.x0..tile.x1 {
                    control.check(128 + format.channels() as u64).map_err(BackedNormalError::Backing)?;
                    let pixel = raw.get_mut(..format.bytes_per_pixel()).ok_or(BackedNormalError::Invalid("sample layout"))?;
                    pin.read_pixel_native_into(x, y, pixel, control).map_err(BackedNormalError::View)?;
                    let mut native = [0.0; 4];
                    for (channel, value) in native.get_mut(..format.channels()).ok_or(BackedNormalError::Invalid("sample layout"))?.iter_mut().enumerate() {
                        *value = photocraft_color::try_read_sample_f64(pixel, format.sample, channel)
                            .map_err(|e| BackedNormalError::View(RasterViewError::Sample(e)))?;
                    }
                    let source = if format.mode == ColorMode::Grayscale {
                        [native[0], native[0], native[0], if format.alpha { native[1] } else { 1.0 }]
                    } else {
                        [native[0], native[1], native[2], if format.alpha { native[3] } else { 1.0 }]
                    };
                    unit(source)?;
                    let index = pixel_index(rect, x, y)?;
                    let target = output.pixels.get_mut(index).ok_or(BackedNormalError::Invalid("band index"))?;
                    let mask_coverage = if let Some((mask_view, outside)) = mask {
                        if mask_view.logical_bounds().contains(x, y) {
                            let coord = photocraft_geom::TileCoord::containing(x, y);
                            if mask_pin.as_ref().is_none_or(|pin| pin.coord() != coord) {
                                // Release old pin before admitting a different tile.
                                drop(mask_pin.take());
                                mask_pin = Some(mask_view.pin_tile(coord, control).map_err(BackedNormalError::View)?);
                            }
                            let format = mask_view.format();
                            let mut bytes = [0; 8];
                            let bytes = bytes.get_mut(..format.bytes_per_pixel()).ok_or(BackedNormalError::Invalid("mask sample layout"))?;
                            mask_pin
                                .as_ref()
                                .ok_or(BackedNormalError::Invalid("mask pin"))?
                                .read_pixel_native_into(x, y, bytes, control)
                                .map_err(BackedNormalError::View)?;
                            let value = photocraft_color::try_read_sample_f64(bytes, format.sample, 0)
                                .map_err(|e| BackedNormalError::View(RasterViewError::Sample(e)))?;
                            coverage(value)?;
                            value
                        } else {
                            outside
                        }
                    } else {
                        1.0
                    };
                    let result = self.working.normal(*target, source, opacity * mask_coverage, self.operator).map_err(BackedNormalError::Color)?;
                    control.check(1).map_err(BackedNormalError::Backing)?;
                    *target = result;
                }
            }
        }
        control.check(1).map_err(BackedNormalError::Backing)?;
        Ok(output)
    }
}
fn coverage(value: f64) -> Result<(), BackedNormalError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(BackedNormalError::Invalid("finite unit opacity/mask coverage required"));
    }
    Ok(())
}
fn pixel_index(rect: Rect, x: i32, y: i32) -> Result<usize, BackedNormalError> {
    let row = u64::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackedNormalError::Invalid("band index"))?;
    let col = u64::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackedNormalError::Invalid("band index"))?;
    let index = row.checked_mul(u64::from(rect.width())).and_then(|n| n.checked_add(col)).ok_or(BackedNormalError::Invalid("band index"))?;
    usize::try_from(index).map_err(|_| BackedNormalError::Invalid("band index"))
}
fn unit(pixel: [f64; 4]) -> Result<(), BackedNormalError> {
    if pixel.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(BackedNormalError::Unsupported("finite unit colors/alpha only"));
    }
    Ok(())
}

fn cms_error(error: photocraft_cms::control::Error) -> BackedNormalError {
    match error {
        photocraft_cms::control::Error::Cancelled => BackedNormalError::Backing(BackingError::Cancelled),
        photocraft_cms::control::Error::Limit(message) => BackedNormalError::Unsupported(message),
        photocraft_cms::control::Error::Allocation => BackedNormalError::Allocation,
        photocraft_cms::control::Error::Cms(error) => BackedNormalError::Color(CompositionError::Color(error)),
    }
}
#[cfg(test)]
#[path = "backed_normal_operator_tests.rs"]
mod operator_tests;

#[cfg(test)]
#[path = "backed_translation_consumer_tests.rs"]
mod translation_consumer_tests;
