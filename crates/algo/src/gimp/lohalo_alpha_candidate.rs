//! Alpha-only Mitchell reconstruction candidate, NOT a full LoHalo sampler.
//! Mitchell/Netravali (1988), BC equation, B=C=1/3. The public GIMP3.2.6
//! observations in temp/sol6-lohalo-20261014 support only the named small,
//! axis-aligned phase/scale domain. No RGB/sigmoid/ICC/EWA/source admission.
//! Primary equation: https://www.cs.utexas.edu/~fussell/courses/cs384g-fall2013/lectures/mitchell/Mitchell.pdf (equation 8).
//! The caller selects this mathematical candidate explicitly; observations do not
//! qualify arbitrary source pixels/shapes or a stored interpolation identifier.
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as Error};
#[derive(Clone, Copy, Debug)]
pub enum Magnification {
    One,
    OneAndHalf,
}
impl Magnification {
    fn scale(self) -> f64 {
        match self {
            Self::One => 1.,
            Self::OneAndHalf => 1.5,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// Complete caller-owned native byte words; no implicit depth conversion.
    pub raw_source: &'a [u8],
    pub source_format: PixelFormat,
    pub output_format: PixelFormat,
    pub source_extent: Rect,
    /// Exact whole adjusted output extent; no guessed sparse source bounds.
    pub output_extent: Rect,
    pub magnification: Magnification,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_bytes: u64,
    pub output_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
/// Private dense Gray8 coverage, world coordinates retained. No RGB is produced.
pub struct AlphaPlane {
    pub format: PixelFormat,
    pub extent: Rect,
    pub bytes: Vec<u8>,
}
fn poll(control: &Interrupt<'_>) -> Result<(), Error> {
    control.check().map_err(|_| Error::Cancelled)
}
/// Explicit world transform is scale*(source world coordinate)+(0.25,0.375).
pub fn adjusted_extent(source: Rect, magnification: Magnification) -> Result<Rect, Error> {
    if source.is_empty() || source.width() > 32 || source.height() > 32 {
        return Err(Error::UnsupportedConversion);
    }
    let s = magnification.scale();
    let edges = [
        (f64::from(source.x0) * s + 0.25).floor(),
        (f64::from(source.y0) * s + 0.375).floor(),
        (f64::from(source.x1) * s + 0.25).ceil(),
        (f64::from(source.y1) * s + 0.375).ceil(),
    ];
    if edges.iter().any(|v| !v.is_finite() || *v < f64::from(i32::MIN) || *v > f64::from(i32::MAX)) {
        return Err(Error::InvalidBounds);
    }
    let output = Rect::new(edges[0] as i32, edges[1] as i32, edges[2] as i32, edges[3] as i32);
    if output.is_empty() || output.width() > 32 || output.height() > 32 {
        return Err(Error::UnsupportedConversion);
    }
    Ok(output)
}
pub fn requirements(request: Request<'_>) -> Result<Requirements, Error> {
    // Only this exact observed format pair is supported. Refuse all other
    // runtime formats before examining pixels or allocating any storage.
    if request.source_format != PixelFormat::RGBA8 || request.output_format != PixelFormat::GRAY8 {
        return Err(Error::UnsupportedConversion);
    }
    if adjusted_extent(request.source_extent, request.magnification)? != request.output_extent {
        return Err(Error::InvalidBounds);
    }
    let source_bytes = u64::from(request.source_extent.width())
        .checked_mul(u64::from(request.source_extent.height()))
        .and_then(|n| n.checked_mul(4))
        .ok_or(Error::LimitExceeded)?;
    if usize::try_from(source_bytes).ok() != Some(request.raw_source.len()) {
        return Err(Error::LengthMismatch);
    }
    let pixels = u64::from(request.output_extent.width()).checked_mul(u64::from(request.output_extent.height())).ok_or(Error::LimitExceeded)?;
    // Fallible Vec capacity checked against this two-times+64 envelope before fill.
    let output_bytes = pixels.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or(Error::LimitExceeded)?;
    let peak_resident_bytes = source_bytes.checked_add(output_bytes).and_then(|n| n.checked_add(1024)).ok_or(Error::LimitExceeded)?;
    let work_units = pixels.checked_mul(2048).and_then(|n| n.checked_add(source_bytes)).ok_or(Error::LimitExceeded)?;
    Ok(Requirements { source_bytes, output_bytes, peak_resident_bytes, work_units })
}
fn weight(t: f64) -> f64 {
    let x = t.abs();
    let b = 1. / 3.;
    let c = 1. / 3.;
    if x < 1. {
        ((12. - 9. * b - 6. * c) * x * x * x + (-18. + 12. * b + 6. * c) * x * x + 6. - 2. * b) / 6.
    } else if x < 2. {
        ((-b - 6. * c) * x * x * x + (6. * b + 30. * c) * x * x + (-12. * b - 48. * c) * x + 8. * b + 24. * c) / 6.
    } else {
        0.
    }
}
fn alpha(request: Request<'_>, x: i64, y: i64) -> Result<f64, Error> {
    if x < 0 || y < 0 || x >= i64::from(request.source_extent.width()) || y >= i64::from(request.source_extent.height()) {
        return Ok(0.);
    }
    let i = usize::try_from(y)
        .ok()
        .and_then(|y| y.checked_mul(request.source_extent.width() as usize))
        .and_then(|n| n.checked_add(usize::try_from(x).ok()?))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add(3))
        .ok_or(Error::LimitExceeded)?;
    Ok(f64::from(*request.raw_source.get(i).ok_or(Error::LengthMismatch)?) / 255.)
}
pub fn try_reconstruct(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<AlphaPlane, Error> {
    poll(control)?;
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > 8_000_000_000 {
        return Err(Error::LimitExceeded);
    }
    let req = requirements(request)?;
    if req.peak_resident_bytes > limits.resident_bytes || req.work_units > limits.work_units {
        return Err(Error::LimitExceeded);
    }
    let count = usize::try_from(request.output_extent.size().area()).map_err(|_| Error::LimitExceeded)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(count).map_err(|_| Error::AllocationFailed)?;
    if bytes.capacity() as u64 > req.output_bytes {
        return Err(Error::LimitExceeded);
    }
    let scale = request.magnification.scale();
    for y in request.output_extent.y0..request.output_extent.y1 {
        poll(control)?;
        for x in request.output_extent.x0..request.output_extent.x1 {
            poll(control)?;
            let sx = (f64::from(x) + 0.5 - 0.25) / scale - f64::from(request.source_extent.x0) - 0.5;
            let sy = (f64::from(y) + 0.5 - 0.375) / scale - f64::from(request.source_extent.y0) - 0.5;
            if !sx.is_finite() || !sy.is_finite() || sx.abs() > i32::MAX as f64 || sy.abs() > i32::MAX as f64 {
                return Err(Error::InvalidBounds);
            }
            let (ix, iy) = (sx.floor() as i64, sy.floor() as i64);
            let mut sum = 0.;
            for dy in -1..=2 {
                for dx in -1..=2 {
                    poll(control)?;
                    let px = ix + dx;
                    let py = iy + dy;
                    let product = alpha(request, px, py)? * weight(sx - px as f64) * weight(sy - py as f64);
                    sum += product;
                }
            }
            if !sum.is_finite() {
                return Err(Error::InvalidSample);
            }
            bytes.push((sum.clamp(0., 1.) * 255. + 0.5).floor() as u8);
        }
    }
    poll(control)?;
    Ok(AlphaPlane { format: request.output_format, extent: request.output_extent, bytes })
}
