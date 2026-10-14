//! Explicit binary pixel-center outline candidate, not GIMP MONO/dropout.
//! Caller owns shaping, baseline placement, bounds, font authority and policy.
//! Existing 26.6 outline quantization and bounded quadratic24.8 subdivision are
//! reused; no AA threshold, source cache, font lookup, layout or CMS is inferred.
use super::outline::{
    layout::{Command, Pen},
    quadratic_24_8,
};
use super::{ExactFont, TextSourceError, poll};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::DrawSettings,
};
type Result<T> = std::result::Result<T, TextSourceError>;
type Point = [i64; 2];
const MAX_POINTS: usize = 4096;
const MAX_COORD: i64 = (1i64 << 29) * 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoveragePolicy {
    GlyphUnionNonzeroPixelCenter,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPolicy {
    GrayCoverage,
    WhiteWithCoverageAlpha,
}
/// Position is a baseline offset in exact24.8 canvas coordinates, not an advance.
#[derive(Clone, Copy, Debug)]
pub struct PositionedGlyph {
    pub glyph_id: u32,
    pub baseline_offset_24_8: Point,
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub glyphs: &'a [PositionedGlyph],
    pub origin_pixels: [f64; 2],
    pub size_px: f32,
    pub bounds: Rect,
    pub requested_output_format: PixelFormat,
    pub coverage: CoveragePolicy,
    pub output: OutputPolicy,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: 64 << 20, work_units: 8_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub peak_bytes: u64,
    pub work_units: u64,
}
struct Glyph {
    points: Vec<Point>,
    ends: [usize; 32],
    contours: usize,
}
fn unsupported() -> TextSourceError {
    TextSourceError::Unsupported("unsupported binary outline candidate")
}
fn origin(request: Request<'_>) -> Result<Point> {
    let mut out = [0; 2];
    for (dst, v) in out.iter_mut().zip(request.origin_pixels) {
        let scaled = v * 256.;
        if !v.is_finite() || scaled.abs() > MAX_COORD as f64 || scaled.fract() != 0. {
            return Err(unsupported());
        }
        *dst = scaled as i64;
    }
    Ok(out)
}
pub fn requirements(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Requirements> {
    poll(control)?;
    origin(request)?;
    if request.glyphs.is_empty()
        || request.glyphs.len() > 32
        || !matches!(request.size_px, 16. | 32. | 40.)
        || request.bounds.x1 <= request.bounds.x0
        || request.bounds.y1 <= request.bounds.y0
        || [request.bounds.x0, request.bounds.y0, request.bounds.x1, request.bounds.y1].into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v))
    {
        return Err(unsupported());
    }
    let format = request.requested_output_format;
    if !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        || !match request.output {
            OutputPolicy::GrayCoverage => format.mode == ColorMode::Grayscale && !format.alpha,
            OutputPolicy::WhiteWithCoverageAlpha => format.mode == ColorMode::Rgb && format.alpha,
        }
    {
        return Err(unsupported());
    }
    for g in request.glyphs {
        poll(control)?;
        if g.glyph_id == 0 || g.baseline_offset_24_8.into_iter().any(|v| !(-MAX_COORD..=MAX_COORD).contains(&v)) {
            return Err(unsupported());
        }
    }
    let width = u64::from(request.bounds.width());
    let height = u64::from(request.bounds.height());
    let count = width.checked_mul(height).ok_or(TextSourceError::Budget)?;
    let bpp = u64::try_from(format.bytes_per_pixel()).map_err(|_| TextSourceError::Budget)?;
    let tx = i64::from(request.bounds.x1 - 1).div_euclid(256) - i64::from(request.bounds.x0).div_euclid(256) + 1;
    let ty = i64::from(request.bounds.y1 - 1).div_euclid(256) - i64::from(request.bounds.y0).div_euclid(256) + 1;
    let tiles = u64::try_from(tx).ok().and_then(|x| u64::try_from(ty).ok().and_then(|y| x.checked_mul(y))).ok_or(TextSourceError::Budget)?;
    let glyphs = u64::try_from(request.glyphs.len()).map_err(|_| TextSourceError::Budget)?;
    let geometry = glyphs.checked_mul((MAX_POINTS * std::mem::size_of::<Point>() + std::mem::size_of::<Glyph>()) as u64).ok_or(TextSourceError::Budget)?;
    let peak_bytes = count
        .checked_mul(bpp)
        .and_then(|v| tiles.checked_mul(65536 * bpp + 256).and_then(|t| v.checked_add(t)))
        .and_then(|v| v.checked_add(geometry))
        .and_then(|v| v.checked_add(411640 + 65552 + 32768 + 4096))
        .ok_or(TextSourceError::Budget)?;
    // Includes worst bounded quadratic subdivision and every potential winding edge.
    let work_units = count
        .checked_mul(glyphs)
        .and_then(|v| v.checked_mul(MAX_POINTS as u64))
        .and_then(|v| glyphs.checked_mul(256 * 8192).and_then(|p| v.checked_add(p)))
        .ok_or(TextSourceError::Budget)?;
    let defaults = Limits::default();
    if limits.resident_bytes > defaults.resident_bytes
        || limits.work_units > defaults.work_units
        || peak_bytes > limits.resident_bytes
        || work_units > limits.work_units
    {
        return Err(TextSourceError::Budget);
    }
    poll(control)?;
    Ok(Requirements { peak_bytes, work_units })
}
fn append(g: &mut Glyph, p: Point) -> Result<()> {
    if g.points.len() >= MAX_POINTS {
        return Err(TextSourceError::Budget);
    }
    g.points.push(p);
    Ok(())
}
fn prepare(font: &ExactFont<'_>, request: Request<'_>, control: &Interrupt<'_>) -> Result<Vec<Glyph>> {
    let face = skrifa::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let base = origin(request)?;
    let mut glyphs = Vec::new();
    glyphs.try_reserve_exact(request.glyphs.len()).map_err(|_| TextSourceError::Allocation)?;
    if glyphs.capacity() > request.glyphs.len() {
        return Err(TextSourceError::Budget);
    }
    let mut memory = [0u8; 16384];
    for positioned in request.glyphs {
        poll(control)?;
        let outline = face.outline_glyphs().get(skrifa::GlyphId::new(positioned.glyph_id)).ok_or_else(unsupported)?;
        let mut pen = Pen::new();
        outline
            .draw(
                DrawSettings::unhinted(Size::new(request.size_px), LocationRef::default())
                    .with_memory(Some(memory.get_mut(..outline.draw_memory_size(skrifa::outline::Hinting::None)).ok_or(TextSourceError::Budget)?)),
                &mut pen,
            )
            .map_err(|_| TextSourceError::FontIdentity)?;
        poll(control)?;
        if pen.invalid || pen.len == 0 {
            return Err(unsupported());
        }
        let mut glyph = Glyph { points: Vec::new(), ends: [0; 32], contours: 0 };
        glyph.points.try_reserve_exact(MAX_POINTS).map_err(|_| TextSourceError::Allocation)?;
        if glyph.points.capacity() > MAX_POINTS {
            return Err(TextSourceError::Budget);
        }
        let mut current = None;
        let mut open = false;
        for command in pen.commands.iter().take(pen.len) {
            poll(control)?;
            match *command {
                Command::Move(p) => {
                    if open {
                        return Err(unsupported());
                    }
                    append(&mut glyph, p)?;
                    current = Some(p);
                    open = true;
                }
                Command::Line(p) => {
                    if !open {
                        return Err(unsupported());
                    }
                    append(&mut glyph, p)?;
                    current = Some(p);
                }
                Command::Quad(c, end) => {
                    if !open {
                        return Err(unsupported());
                    }
                    let start = current.ok_or_else(unsupported)?;
                    let poly = quadratic_24_8([start, c, end], control, &mut |_| Ok(()))?;
                    for (i, p) in poly.iter().skip(1).enumerate() {
                        if i % 256 == 0 {
                            poll(control)?
                        }
                        append(&mut glyph, *p)?;
                    }
                    current = Some(end);
                }
                Command::Close => {
                    if !open {
                        return Err(unsupported());
                    }
                    *glyph.ends.get_mut(glyph.contours).ok_or(TextSourceError::Budget)? = glyph.points.len();
                    glyph.contours += 1;
                    current = None;
                    open = false;
                }
                Command::Empty => return Err(unsupported()),
            }
        }
        if open || glyph.contours == 0 {
            return Err(unsupported());
        }
        let bx = base[0].checked_add(positioned.baseline_offset_24_8[0]).ok_or(TextSourceError::Budget)?;
        let by = base[1].checked_add(positioned.baseline_offset_24_8[1]).ok_or(TextSourceError::Budget)?;
        for (i, p) in glyph.points.iter_mut().enumerate() {
            if i % 256 == 0 {
                poll(control)?
            }
            p[0] = p[0].checked_add(bx).ok_or(TextSourceError::Budget)?;
            p[1] = by.checked_sub(p[1]).ok_or(TextSourceError::Budget)?;
            if p.iter().any(|v| !(-MAX_COORD..=MAX_COORD).contains(v)) {
                return Err(unsupported());
            }
        }
        glyphs.push(glyph);
    }
    Ok(glyphs)
}
fn nonzero(glyph: &Glyph, p: Point, control: &Interrupt<'_>) -> Result<bool> {
    let mut winding = 0i32;
    let mut start = 0;
    for end in glyph.ends.iter().take(glyph.contours) {
        let points = glyph.points.get(start..*end).ok_or(TextSourceError::Budget)?;
        if points.len() < 2 {
            return Err(unsupported());
        }
        for (i, a) in points.iter().enumerate() {
            if i % 256 == 0 {
                poll(control)?
            }
            let b = points.get((i + 1) % points.len()).ok_or(TextSourceError::Budget)?;
            let cross = i128::from(b[0] - a[0])
                .checked_mul(i128::from(p[1] - a[1]))
                .and_then(|v| i128::from(b[1] - a[1]).checked_mul(i128::from(p[0] - a[0])).and_then(|w| v.checked_sub(w)))
                .ok_or(TextSourceError::Budget)?;
            if a[1] <= p[1] && b[1] > p[1] && cross > 0 {
                winding = winding.checked_add(1).ok_or(TextSourceError::Budget)?;
            } else if a[1] > p[1] && b[1] <= p[1] && cross < 0 {
                winding = winding.checked_sub(1).ok_or(TextSourceError::Budget)?;
            }
        }
        start = *end;
    }
    Ok(winding != 0)
}
fn one(sample: SampleType, out: &mut [u8]) -> Result<()> {
    if out.len() != sample.bytes() {
        return Err(TextSourceError::Budget);
    }
    match sample {
        SampleType::U8 => out.copy_from_slice(&[255]),
        SampleType::U16 => out.copy_from_slice(&u16::MAX.to_ne_bytes()),
        SampleType::U32 => out.copy_from_slice(&u32::MAX.to_ne_bytes()),
        SampleType::F32 => out.copy_from_slice(&1f32.to_ne_bytes()),
        SampleType::F64 => out.copy_from_slice(&1f64.to_ne_bytes()),
    }
    Ok(())
}
pub fn render_with_charge(
    font: &ExactFont<'_>,
    request: Request<'_>,
    limits: Limits,
    control: &Interrupt<'_>,
    resident: &mut dyn FnMut(u64) -> Result<()>,
    work: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<Surface> {
    let req = requirements(request, limits, control)?;
    resident(req.peak_bytes)?;
    work(req.work_units)?;
    poll(control)?;
    let glyphs = prepare(font, request, control)?;
    let width = usize::try_from(request.bounds.width()).map_err(|_| TextSourceError::Budget)?;
    let height = usize::try_from(request.bounds.height()).map_err(|_| TextSourceError::Budget)?;
    let format = request.requested_output_format;
    let bpp = format.bytes_per_pixel();
    let bytes = width.checked_mul(height).and_then(|v| v.checked_mul(bpp)).ok_or(TextSourceError::Budget)?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(bytes).map_err(|_| TextSourceError::Allocation)?;
    if pixels.capacity() > bytes {
        return Err(TextSourceError::Budget);
    }
    while pixels.len() < bytes {
        poll(control)?;
        pixels.resize(pixels.len().saturating_add(65536).min(bytes), 0u8);
    }
    for row in 0..height {
        poll(control)?;
        let y = i64::from(request.bounds.y0) + i64::try_from(row).map_err(|_| TextSourceError::Budget)?;
        for col in 0..width {
            if col % 64 == 0 {
                poll(control)?
            }
            let x = i64::from(request.bounds.x0) + i64::try_from(col).map_err(|_| TextSourceError::Budget)?;
            let center = [x * 256 + 128, y * 256 + 128];
            let mut inside = false;
            for glyph in &glyphs {
                if nonzero(glyph, center, control)? {
                    inside = true;
                    break;
                }
            }
            if inside {
                let offset = row.checked_mul(width).and_then(|v| v.checked_add(col)).and_then(|v| v.checked_mul(bpp)).ok_or(TextSourceError::Budget)?;
                let pixel = pixels.get_mut(offset..offset.checked_add(bpp).ok_or(TextSourceError::Budget)?).ok_or(TextSourceError::Budget)?;
                for channel in pixel.chunks_exact_mut(format.sample.bytes()) {
                    one(format.sample, channel)?;
                }
            }
        }
    }
    poll(control)?;
    let surface = Surface::try_from_interleaved_with(format, request.bounds, &pixels, control)
        .map_err(|_| if control.cancelled() { TextSourceError::Cancelled } else { TextSourceError::Allocation })?;
    poll(control)?;
    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonzero_winding_preserves_hole_and_half_open_edges() {
        let glyph = Glyph {
            points: vec![[0, 0], [1024, 0], [1024, 1024], [0, 1024], [256, 256], [256, 768], [768, 768], [768, 256]],
            ends: {
                let mut e = [0; 32];
                e[0] = 4;
                e[1] = 8;
                e
            },
            contours: 2,
        };
        assert!(nonzero(&glyph, [128, 128], &Interrupt::NONE).expect("outer"));
        assert!(!nonzero(&glyph, [512, 512], &Interrupt::NONE).expect("opposite winding hole"));
        assert!(!nonzero(&glyph, [1024, 128], &Interrupt::NONE).expect("right boundary excluded"));
    }
    fn request<'a>(glyphs: &'a [PositionedGlyph], format: PixelFormat) -> Request<'a> {
        Request {
            glyphs,
            origin_pixels: [0., 0.],
            size_px: 32.,
            bounds: Rect::new(0, 0, 32, 40),
            requested_output_format: format,
            coverage: CoveragePolicy::GlyphUnionNonzeroPixelCenter,
            output: OutputPolicy::GrayCoverage,
        }
    }
    #[test]
    fn explicit_fractional_grid_and_limits_refuse_before_preparation() {
        let glyphs = [PositionedGlyph { glyph_id: 197, baseline_offset_24_8: [0, 31 * 256] }];
        let r = request(&glyphs, PixelFormat::GRAY8);
        assert!(requirements(r, Limits::default(), &Interrupt::NONE).is_ok());
        assert!(requirements(Request { origin_pixels: [0.5, 0.25], ..r }, Limits::default(), &Interrupt::NONE).is_ok());
        assert!(requirements(Request { origin_pixels: [0.1, 0.], ..r }, Limits::default(), &Interrupt::NONE).is_err());
        assert!(requirements(r, Limits { resident_bytes: 1, ..Limits::default() }, &Interrupt::NONE).is_err());
        assert!(requirements(r, Limits::default(), &Interrupt::cancel_only(&|| true)).is_err());
        assert!(requirements(Request { requested_output_format: PixelFormat::CMYKA8, ..r }, Limits::default(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn unit_endpoints_are_exact_at_all_native_depths() {
        for sample in SampleType::ALL {
            let mut out = vec![0; sample.bytes()];
            one(sample, &mut out).expect("exact unit");
            match sample {
                SampleType::U8 => assert_eq!(out, vec![255]),
                SampleType::U16 => assert_eq!(out, u16::MAX.to_ne_bytes()),
                SampleType::U32 => assert_eq!(out, u32::MAX.to_ne_bytes()),
                SampleType::F32 => assert_eq!(out, 1f32.to_ne_bytes()),
                SampleType::F64 => assert_eq!(out, 1f64.to_ne_bytes()),
            }
        }
    }
    #[test]
    fn external_exact_face_curved_o_has_a_binary_hole() {
        let control = Interrupt::NONE;
        let font = ExactFont::bundled_inter(&control).expect("existing authorized exact face");
        let glyphs = [PositionedGlyph { glyph_id: 321, baseline_offset_24_8: [0, 31 * 256] }];
        let output = render_with_charge(&font, request(&glyphs, PixelFormat::GRAY8), Limits::default(), &control, &mut |_| Ok(()), &mut |_| Ok(()))
            .expect("candidate not GIMP MONO oracle");
        let mut row = [0u8; 32];
        output.try_read_interleaved_into_with(Rect::new(0, 20, 32, 21), &mut row, &control).expect("native row");
        assert_eq!(row[12], 0);
        assert!(row.contains(&255));
        assert!(row.iter().all(|v| matches!(v, 0 | 255)));
    }
}
