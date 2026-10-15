//! Paid, explicit flat RGB/Gray blend bands. This primitive grants no import policy.
use crate::{
    backed_normal::{BackedNormalError as E, NormalBandLimits},
    backed_normal_stack::OwnedNormalStackBand,
    gimp_blend::{self, GimpBlendMode},
    gimp_composite::{CompositionError, WorkingSpace, encoding, space},
};
use photocraft_cms::Profile;
use photocraft_doc::{
    BackedLayer, ColorMode, SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
    checked_view::RasterViewError,
};

/// Closed modern channel-separable inventory; Normal is a separate kernel.
pub fn is_supported_mode(mode: i32) -> bool {
    matches!(mode, 23 | 30..=36 | 41..=53)
}
pub(crate) fn error() -> E {
    E::Unsupported("bounded modern separable RGB transfer stack")
}
fn length() -> E {
    E::Backing(BackingError::Length)
}
pub(crate) fn color(e: CompositionError) -> E {
    match e {
        CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled) => E::Backing(BackingError::Cancelled),
        other => E::Color(other),
    }
}
fn cms(e: photocraft_cms::control::Error) -> E {
    match e {
        photocraft_cms::control::Error::Cancelled => E::Backing(BackingError::Cancelled),
        photocraft_cms::control::Error::Allocation => E::Allocation,
        photocraft_cms::control::Error::Limit(v) => E::Unsupported(v),
        photocraft_cms::control::Error::Cms(v) => E::Color(CompositionError::Color(v)),
    }
}
pub(crate) fn slot(s: GimpSpace) -> Result<usize, E> {
    match s {
        GimpSpace::LinearRgb => Ok(0),
        GimpSpace::ProfileRgb => Ok(1),
        GimpSpace::PerceptualRgb => Ok(2),
        GimpSpace::Lab => Err(error()),
    }
}
/// Constructor-owned prospective analytic preparation. Variable tables/LUTs are
/// refused; the bound covers concurrent curve/string copies and actual headers.
pub(crate) fn space_requirements(profile: &Profile, stored: SampleEncoding, needed: [bool; 3], header: usize) -> Result<(u64, u64), E> {
    if !matches!(profile.channels(), 1 | 3)
        || profile.description.len() > 65536
        || profile.copyright.len() > 65536
        || profile
            .trc
            .iter()
            .flatten()
            .chain(profile.gray_trc.iter())
            .any(|v| matches!(v, photocraft_cms::curve::Curve::Table(_) | photocraft_cms::curve::Curve::Table64(_)))
    {
        return Err(error());
    }
    let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(profile).map_err(cms)?;
    let strings = profile.description.len().checked_add(profile.copyright.len()).ok_or_else(length)? as u64;
    let mut bytes = header as u64;
    let mut work = semantic;
    for (index, kind) in [GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpSpace::PerceptualRgb].into_iter().enumerate() {
        if !needed.get(index).copied().ok_or_else(error)? {
            continue;
        }
        let cross = kind != space(stored);
        bytes = bytes.checked_add(strings.checked_mul(if cross { 8 } else { 2 }).ok_or_else(length)?).and_then(|v| v.checked_add(32768)).ok_or_else(length)?;
        work = work
            .checked_add(semantic.checked_mul(if cross { 2 } else { 1 }).ok_or_else(length)?)
            .and_then(|v| v.checked_add(strings * 8 + 1))
            .ok_or_else(length)?;
    }
    Ok((bytes, work))
}
pub(crate) struct Spaces<L> {
    values: [Option<WorkingSpace>; 3],
    _fee: L,
}
impl<L> Spaces<L> {
    pub(crate) fn prepare(
        profile: &Profile,
        stored: SampleEncoding,
        needed: [bool; 3],
        fee: L,
        interrupt: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Self, E> {
        // Payload-before-fee guard is installed before any fallible constructor.
        let mut out = Self { values: [None, None, None], _fee: fee };
        photocraft_cms::xcf_color::qualify_effective_analytic_profile(profile, encoding(stored)).map_err(|e| color(CompositionError::Color(e)))?;
        for (i, kind) in [GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpSpace::PerceptualRgb].into_iter().enumerate() {
            if needed.get(i).copied().ok_or_else(error)? {
                *out.values.get_mut(i).ok_or_else(error)? = Some(WorkingSpace::new_with_interrupt(profile, stored, kind, interrupt).map_err(color)?);
            }
        }
        Ok(out)
    }
    fn get(&self, kind: GimpSpace) -> Result<&WorkingSpace, E> {
        self.values.get(slot(kind)?).and_then(Option::as_ref).ok_or_else(error)
    }
}
#[derive(Clone, Copy)]
pub struct SeparableStackLayer<'a> {
    pub layer: &'a BackedLayer,
    pub visible: bool,
    pub opacity: f64,
    pub source_mode: i32,
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
}
struct Entry<'a> {
    input: SeparableStackLayer<'a>,
    mode: GimpBlendMode,
}
pub struct PreparedBackedSeparableStack<'a> {
    entries: Vec<Entry<'a>>,
    spaces: Spaces<QuotaLease>,
    quota: BackingQuota,
    limits: NormalBandLimits,
    _index: QuotaLease,
}
fn valid_pixel(p: [f64; 4]) -> Result<(), E> {
    if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) { Err(error()) } else { Ok(()) }
}
pub(crate) fn decode(raw: &[u8], f: photocraft_doc::PixelFormat) -> Result<[f64; 4], E> {
    let mut values = [0.; 4];
    for channel in 0..f.channels() {
        *values.get_mut(channel).ok_or_else(error)? =
            photocraft_color::try_read_sample_f64(raw, f.sample, channel).map_err(|e| E::View(RasterViewError::Sample(e)))?;
    }
    let p = match f.mode {
        ColorMode::Rgb => [values[0], values[1], values[2], if f.alpha { values[3] } else { 1. }],
        ColorMode::Grayscale => [values[0], values[0], values[0], if f.alpha { values[1] } else { 1. }],
        _ => return Err(error()),
    };
    valid_pixel(p)?;
    Ok(p)
}
impl<'a> PreparedBackedSeparableStack<'a> {
    pub fn new(
        layers: &[SeparableStackLayer<'a>],
        q: &BackingQuota,
        profile: &Profile,
        stored: SampleEncoding,
        limits: NormalBandLimits,
        c: &ReadControl<'_>,
    ) -> Result<Self, E> {
        c.check(1).map_err(E::Backing)?;
        if layers.is_empty() || layers.len() > 4096 || limits.max_rows == 0 || limits.max_rows > 256 || limits.max_pixels == 0 || limits.max_pixels > 65536 {
            return Err(error());
        }
        let mut needed = [false; 3];
        for input in layers {
            c.check(std::mem::size_of::<SeparableStackLayer<'_>>() as u64).map_err(E::Backing)?;
            if !q.shares_ledger(input.layer.quota())
                || !input.opacity.is_finite()
                || !(0.0..=1.0).contains(&input.opacity)
                || !matches!(input.layer.format().mode, ColorMode::Rgb | ColorMode::Grayscale)
                || input.layer.format().mode.color_channels() != profile.channels()
                || !(input.source_mode == 28 || is_supported_mode(input.source_mode))
            {
                return Err(error());
            }
            *needed.get_mut(slot(input.blend_space)?).ok_or_else(error)? = true;
            *needed.get_mut(slot(input.composite_space)?).ok_or_else(error)? = true;
        }
        let (bytes, work) = space_requirements(profile, stored, needed, std::mem::size_of::<Spaces<QuotaLease>>())?;
        let fee = q.reserve(QuotaKind::Resident, bytes).map_err(E::Quota)?;
        c.check(work).map_err(E::Backing)?;
        let spaces = Spaces::prepare(profile, stored, needed, fee, c.interrupt)?;
        let indexbytes = layers
            .len()
            .checked_mul(std::mem::size_of::<Entry<'a>>())
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(length)?;
        let index = q.reserve(QuotaKind::Index, indexbytes as u64).map_err(E::Quota)?;
        let mut out = Self { entries: Vec::new(), spaces, quota: q.clone(), limits, _index: index };
        c.check(indexbytes as u64).map_err(E::Backing)?;
        out.entries.try_reserve_exact(layers.len()).map_err(|_| E::Allocation)?;
        if out.entries.capacity() > layers.len().checked_mul(2).ok_or_else(length)? {
            return Err(E::Allocation);
        }
        let actual =
            out.entries.capacity().checked_mul(std::mem::size_of::<Entry<'a>>()).and_then(|v| v.checked_add(std::mem::size_of::<Self>())).ok_or_else(length)?;
        out._index.reconcile_capacity(actual as u64).map_err(E::Quota)?;
        for input in layers {
            c.check(1).map_err(E::Backing)?;
            out.entries.push(Entry { input: *input, mode: gimp_blend::mode(input.source_mode).map_err(color)? });
        }
        Ok(out)
    }
    /// Validate original defaults and every operative sample, including hidden
    /// entries and content outside the requested display canvas.
    pub fn validate_operative(&self, c: &ReadControl<'_>) -> Result<(), E> {
        let _scratch =
            self.quota.reserve(QuotaKind::Resident, (std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() + 128) as u64).map_err(E::Quota)?;
        for entry in &self.entries {
            let view = entry.input.layer.checked_view();
            let format = view.format();
            c.check(format.channels() as u64).map_err(E::Backing)?;
            decode(view.default_bytes(), format)?;
            for coord in view.logical_bounds().tiles() {
                let pin = view.pin_tile(coord, c).map_err(E::View)?;
                let r = coord.rect().intersect(&view.logical_bounds());
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        c.check(format.channels() as u64).map_err(E::Backing)?;
                        let mut bytes = [0; 40];
                        let raw = bytes.get_mut(..format.bytes_per_pixel()).ok_or_else(error)?;
                        pin.read_pixel_native_into(x, y, raw, c).map_err(E::View)?;
                        decode(raw, format)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn render_band(&self, rect: Rect, backdrop: [f64; 4], c: &ReadControl<'_>) -> Result<OwnedNormalStackBand, E> {
        let _scratch =
            self.quota.reserve(QuotaKind::Resident, (std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() + 128) as u64).map_err(E::Quota)?;
        valid_pixel(backdrop)?;
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(length)?;
        if rect.is_empty() || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(error());
        }
        let count = usize::try_from(count).map_err(|_| length())?;
        let mut prior: Option<Band> = None;
        for entry in &self.entries {
            c.check(1).map_err(E::Backing)?;
            if !entry.input.visible {
                continue;
            }
            let mut next = Band::new(&self.quota, count, c)?;
            let composite = self.spaces.get(entry.input.composite_space)?;
            let blend = self.spaces.get(entry.input.blend_space)?;
            for _y in rect.y0..rect.y1 {
                for _x in rect.x0..rect.x1 {
                    c.check(32 + 128).map_err(E::Backing)?;
                    let bg = prior.as_ref().map_or(Ok(backdrop), |b| b.pixels.get(next.pixels.len()).copied().ok_or_else(error))?;
                    // Zero source must still reach the existing operator equation.
                    let value = if entry.mode == GimpBlendMode::Normal {
                        composite.normal(bg, [0.; 4], 0., entry.input.operator)
                    } else {
                        composite.separable(blend, bg, [0.; 4], 0., entry.input.operator, entry.mode)
                    }
                    .map_err(color)?;
                    next.pixels.push(value);
                }
            }
            let view = entry.input.layer.checked_view();
            let format = view.format();
            let active = rect.intersect(&view.logical_bounds());
            for coord in active.tiles() {
                let pin = view.pin_tile(coord, c).map_err(E::View)?;
                let r = coord.rect().intersect(&active);
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        c.check(512 + format.channels() as u64).map_err(E::Backing)?;
                        let mut bytes = [0; 40];
                        let raw = bytes.get_mut(..format.bytes_per_pixel()).ok_or_else(error)?;
                        pin.read_pixel_native_into(x, y, raw, c).map_err(E::View)?;
                        let source = decode(raw, format)?;
                        let offset = u64::try_from(y - rect.y0)
                            .ok()
                            .and_then(|dy| dy.checked_mul(u64::from(rect.width())))
                            .and_then(|v| u64::try_from(x - rect.x0).ok().and_then(|dx| v.checked_add(dx)))
                            .and_then(|v| usize::try_from(v).ok())
                            .ok_or_else(length)?;
                        let bg = prior.as_ref().map_or(Ok(backdrop), |b| b.pixels.get(offset).copied().ok_or_else(error))?;
                        let value = if entry.mode == GimpBlendMode::Normal {
                            composite.normal(bg, source, entry.input.opacity, entry.input.operator)
                        } else {
                            composite.separable(blend, bg, source, entry.input.opacity, entry.input.operator, entry.mode)
                        }
                        .map_err(color)?;
                        *next.pixels.get_mut(offset).ok_or_else(error)? = value;
                    }
                }
            }
            prior = Some(next);
        }
        let out = if let Some(out) = prior {
            out
        } else {
            let mut out = Band::new(&self.quota, count, c)?;
            for _ in 0..count {
                c.check(32).map_err(E::Backing)?;
                out.pixels.push(backdrop);
            }
            out
        };
        c.check(0).map_err(E::Backing)?;
        Ok(OwnedNormalStackBand::from_accounted_pixels(rect, out.pixels, out.fee))
    }
}
struct Band {
    pixels: Vec<[f64; 4]>,
    fee: QuotaLease,
}
impl Band {
    fn new(q: &BackingQuota, count: usize, c: &ReadControl<'_>) -> Result<Self, E> {
        let header = std::mem::size_of::<Self>().max(std::mem::size_of::<OwnedNormalStackBand>()) as u64;
        let bytes = (count as u64).checked_mul(64).and_then(|v| v.checked_add(header)).ok_or_else(length)?;
        let fee = q.reserve(QuotaKind::Resident, bytes).map_err(E::Quota)?;
        let mut out = Self { pixels: Vec::new(), fee };
        c.check(bytes).map_err(E::Backing)?;
        out.pixels.try_reserve_exact(count).map_err(|_| E::Allocation)?;
        if out.pixels.capacity() > count.checked_mul(2).ok_or_else(length)? {
            return Err(E::Allocation);
        }
        let actual = (out.pixels.capacity() as u64).checked_mul(32).and_then(|v| v.checked_add(header)).ok_or_else(length)?;
        out.fee.reconcile_capacity(actual).map_err(E::Quota)?;
        Ok(out)
    }
}
