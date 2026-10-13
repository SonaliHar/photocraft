//! Explicit candidate raster consumer for source-unit text outlines.
//! Ordinary vector area coverage is reused; no GIMP text raster epoch is admitted.
//! Hinting is unsupported upstream of this stage. Colors are already resolved by
//! caller CMS in the declared native sample domain, never inferred from source names.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{FillRule, Knot, Path, PathOp, SampleEncoding, Subpath, VectorMask};
use photocraft_geom::{Point, Rect};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use photocraft_text::xcf::{
    unit_outline::{Command, PreparedUnitOutline},
    unit_raster_policy::CoveragePolicy,
};
use photocraft_vector::{CompiledVectorMask, VectorPreparationLimits};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    pub profile_identity: [u8; 32],
}
#[derive(Clone, Copy, Debug)]
pub enum NativeColor {
    Rgb([f64; 4]),
    Gray([f64; 2]),
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
    pub vector: VectorPreparationLimits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            resident_bytes: 64 * 1024 * 1024,
            work_units: 4_000_000_000,
            vector: VectorPreparationLimits { max_bytes: 8 * 1024 * 1024, max_edges: 4096, max_components: 16384, max_work: 64_000_000 },
        }
    }
}
#[derive(Debug)]
pub enum Error {
    Unsupported(&'static str),
    Budget,
    Allocation,
    Cancelled,
    Raster(RasterInputError),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "candidate text raster: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<RasterInputError> for Error {
    fn from(e: RasterInputError) -> Self {
        if matches!(e, RasterInputError::Cancelled) { Self::Cancelled } else { Self::Raster(e) }
    }
}
type Result<T> = std::result::Result<T, Error>;
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| Error::Cancelled)
}
fn point(p: [f64; 2]) -> Result<Point> {
    if p.iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29)) {
        return Err(Error::Unsupported("outline coordinates"));
    }
    Ok(Point::new(p[0], p[1]))
}
fn path(outline: &PreparedUnitOutline, control: &Interrupt<'_>) -> Result<Path> {
    if outline.commands.len() > 16384 || outline.glyphs.len() > 256 {
        return Err(Error::Budget);
    }
    let contours = outline.commands.iter().filter(|c| matches!(c, Command::Move(_))).count();
    let mut subpaths = Vec::new();
    subpaths.try_reserve_exact(contours).map_err(|_| Error::Allocation)?;
    if subpaths.capacity() > contours {
        return Err(Error::Budget);
    }
    let mut end = 0usize;
    for glyph in &outline.glyphs {
        poll(control)?;
        if glyph.start != end || glyph.end < glyph.start || glyph.end > outline.commands.len() {
            return Err(Error::Unsupported("outline glyph spans"));
        }
        end = glyph.end;
        let commands = outline.commands.get(glyph.start..glyph.end).ok_or(Error::Unsupported("outline span"))?;
        let mut offset = 0usize;
        let mut first = true;
        while offset < commands.len() {
            poll(control)?;
            let Some(Command::Move(start)) = commands.get(offset) else {
                return Err(Error::Unsupported("outline contour start"));
            };
            let remaining = commands.get(offset + 1..).ok_or(Error::Budget)?;
            let span = remaining.iter().position(|c| matches!(c, Command::Close)).ok_or(Error::Unsupported("open glyph contour"))?;
            let count = span.checked_add(2).ok_or(Error::Budget)?;
            let mut knots = Vec::new();
            knots.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
            if knots.capacity() > count {
                return Err(Error::Budget);
            }
            let start = point(*start)?;
            knots.push(Knot::corner(start.x, start.y));
            for command in remaining.iter().take(span) {
                poll(control)?;
                let previous = knots.last_mut().ok_or(Error::Unsupported("empty contour"))?;
                let (end, incoming) = match command {
                    Command::Line(end) => {
                        let e = point(*end)?;
                        (e, e)
                    }
                    Command::Quad(control, end) => {
                        let c = point(*control)?;
                        let e = point(*end)?;
                        let a = previous.anchor;
                        // Exact quadratic-to-cubic degree elevation; existing
                        // vector flatten/raster math performs the later sampling.
                        previous.out_ctrl = Point::new(a.x + (c.x - a.x) * 2.0 / 3.0, a.y + (c.y - a.y) * 2.0 / 3.0);
                        (e, Point::new(e.x + (c.x - e.x) * 2.0 / 3.0, e.y + (c.y - e.y) * 2.0 / 3.0))
                    }
                    Command::Cubic(a, b, end) => {
                        previous.out_ctrl = point(*a)?;
                        (point(*end)?, point(*b)?)
                    }
                    _ => return Err(Error::Unsupported("nested or unclosed glyph contour")),
                };
                let mut knot = Knot::corner(end.x, end.y);
                knot.in_ctrl = incoming;
                knots.push(knot);
            }
            if knots.len() > 1 && knots.last().is_some_and(|k| k.anchor == start) {
                let last = knots.pop().ok_or(Error::Budget)?;
                let first = knots.first_mut().ok_or(Error::Budget)?;
                first.in_ctrl = last.in_ctrl;
            }
            subpaths.push(Subpath { closed: true, knots, op: if first { PathOp::Combine } else { PathOp::Join } });
            first = false;
            offset = offset.checked_add(span).and_then(|n| n.checked_add(2)).ok_or(Error::Budget)?;
        }
    }
    if end != outline.commands.len() {
        return Err(Error::Unsupported("unowned outline commands"));
    }
    Ok(Path { subpaths, fill_rule: FillRule::NonZero, inverted: false })
}
fn binary_coverage(lines: &[photocraft_vector::Polyline], path: &Path, tile: Rect, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    if lines.len() != path.subpaths.len() {
        return Err(Error::Unsupported("binary path component binding"));
    }
    let count = usize::try_from(tile.width()).ok().and_then(|w| usize::try_from(tile.height()).ok().and_then(|h| w.checked_mul(h))).ok_or(Error::Budget)?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
    if out.capacity() > count {
        return Err(Error::Budget);
    }
    for y in tile.y0..tile.y1 {
        poll(control)?;
        for x in tile.x0..tile.x1 {
            let cx = f64::from(x) + 0.5;
            let cy = f64::from(y) + 0.5;
            let mut inside = false;
            let mut component = 0i64;
            for (line, subpath) in lines.iter().zip(&path.subpaths) {
                poll(control)?;
                if !line.closed {
                    return Err(Error::Unsupported("open binary text contour"));
                }
                if subpath.op == PathOp::Combine {
                    inside |= component != 0;
                    component = 0;
                } else if subpath.op != PathOp::Join {
                    return Err(Error::Unsupported("binary text component operator"));
                }
                for (index, a) in line.pts.iter().enumerate() {
                    if index % 256 == 0 {
                        poll(control)?;
                    }
                    let b = line.pts.get(if index + 1 == line.pts.len() { 0 } else { index + 1 }).ok_or(Error::Budget)?;
                    let side = (b.0 - a.0) * (cy - a.1) - (b.1 - a.1) * (cx - a.0);
                    if !side.is_finite() {
                        return Err(Error::Unsupported("binary text winding domain"));
                    }
                    if a.1 <= cy && b.1 > cy && side > 0.0 {
                        component = component.checked_add(1).ok_or(Error::Budget)?;
                    } else if a.1 > cy && b.1 <= cy && side < 0.0 {
                        component = component.checked_sub(1).ok_or(Error::Budget)?;
                    }
                }
            }
            inside |= component != 0;
            out.push(if inside { 1.0 } else { 0.0 });
        }
    }
    Ok(out)
}
/// Render private sparse native tiles over ROI. Outline coordinates already contain
/// the explicitly selected baseline; no extra placement is inferred here.
/// Explicit source policy: non-zero vector area or half-open pixel-center coverage, straight color, alpha
/// multiplied by coverage; outside ROI/default remain native transparent zero.
/// Final integer/float storage quantization is the checked raster writer's policy.
#[allow(clippy::too_many_arguments)]
pub fn render(
    outline: &PreparedUnitOutline,
    roi: Rect,
    format: PixelFormat,
    color: NativeColor,
    prepared_context: WorkingContext,
    output_context: WorkingContext,
    limits: Limits,
    control: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<Surface> {
    poll(control)?;
    let d = Limits::default();
    let v = VectorPreparationLimits::default();
    if limits.resident_bytes > d.resident_bytes
        || limits.work_units > d.work_units
        || limits.vector.max_bytes > v.max_bytes
        || limits.vector.max_edges > v.max_edges
        || limits.vector.max_components > v.max_components
        || limits.vector.max_work > v.max_work
    {
        return Err(Error::Budget);
    }
    if prepared_context != output_context || !format.alpha {
        return Err(Error::Unsupported("caller native text color/profile domain"));
    }
    let (values, channels) = match (format.mode, color) {
        (ColorMode::Rgb, NativeColor::Rgb(c)) => (c, 4usize),
        (ColorMode::Grayscale, NativeColor::Gray(c)) => ([c[0], c[1], 0.0, 0.0], 2usize),
        _ => return Err(Error::Unsupported("native text RGB/Gray color shape")),
    };
    let alpha = *values.get(channels - 1).ok_or(Error::Budget)?;
    if values.iter().take(channels).any(|v| !v.is_finite())
        || !(0.0..=1.0).contains(&alpha)
        || matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32) && values.iter().take(channels - 1).any(|v| !(0.0..=1.0).contains(v))
    {
        return Err(Error::Unsupported("native text color/alpha domain"));
    }
    if roi.x1 <= roi.x0 || roi.y1 <= roi.y0 || [roi.x0, roi.x1, roi.y0, roi.y1].iter().any(|n| !(-(1 << 29)..=(1 << 29)).contains(n)) {
        return Err(Error::Unsupported("text raster ROI"));
    }
    let width = u64::try_from(i64::from(roi.x1) - i64::from(roi.x0)).map_err(|_| Error::Budget)?;
    let height = u64::try_from(i64::from(roi.y1) - i64::from(roi.y0)).map_err(|_| Error::Budget)?;
    let nx = u64::try_from(i64::from(roi.x1 - 1).div_euclid(256) - i64::from(roi.x0).div_euclid(256) + 1).map_err(|_| Error::Budget)?;
    let ny = u64::try_from(i64::from(roi.y1 - 1).div_euclid(256) - i64::from(roi.y0).div_euclid(256) + 1).map_err(|_| Error::Budget)?;
    let tiles = nx.checked_mul(ny).ok_or(Error::Budget)?;
    let output = tiles.checked_mul(65536).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(Error::Budget)?;
    let index = tiles.checked_mul(256).ok_or(Error::Budget)?;
    let scratch = 65536u64.checked_mul((channels * 8 + 4 + format.bytes_per_pixel()) as u64).ok_or(Error::Budget)?;
    let geometry = (outline.commands.len() as u64)
        .checked_mul((std::mem::size_of::<Knot>() + std::mem::size_of::<Subpath>()) as u64)
        .and_then(|n| n.checked_mul(2))
        .ok_or(Error::Budget)?;
    let held_outline = (outline.commands.capacity() as u64)
        .checked_mul(std::mem::size_of::<Command>() as u64)
        .and_then(|n| {
            n.checked_add((outline.glyphs.capacity() as u64).checked_mul(std::mem::size_of::<photocraft_text::xcf::unit_outline::GlyphSpan>() as u64)?)
        })
        .ok_or(Error::Budget)?;
    let resident = output
        .checked_mul(2)
        .and_then(|n| n.checked_add(index.checked_mul(3)?))
        .and_then(|n| n.checked_add(scratch))
        .and_then(|n| n.checked_add(geometry))
        .and_then(|n| n.checked_add(limits.vector.max_bytes as u64))
        .and_then(|n| n.checked_add(held_outline))
        .ok_or(Error::Budget)?;
    let work = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul((limits.vector.max_edges as u64).saturating_add(channels as u64).saturating_add(256)))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(tiles.checked_add(1)?.checked_mul(limits.vector.max_work)?))
        .ok_or(Error::Budget)?;
    if resident > limits.resident_bytes || work > limits.work_units {
        return Err(Error::Budget);
    }
    charge(resident.checked_sub(held_outline).ok_or(Error::Budget)?)?;
    poll(control)?;
    let path = path(outline, control)?;
    if path.subpaths.is_empty() {
        return Ok(Surface::new(format));
    }
    let mask = VectorMask::new(path);
    let coverage_policy = outline.raster_policy.coverage();
    let compiled =
        if coverage_policy == CoveragePolicy::Area { Some(CompiledVectorMask::try_new_with_interrupt(&mask, limits.vector, control)?) } else { None };
    let lines = if coverage_policy == CoveragePolicy::PixelCenterNonZero {
        let lines = photocraft_vector::flatten_path_with_interrupt(&mask.path, photocraft_vector::DEFAULT_TOLERANCE, limits.vector, control)?;
        let mut edges = 0usize;
        for line in &lines {
            poll(control)?;
            edges = edges.checked_add(line.pts.len()).ok_or(Error::Budget)?;
        }
        if edges > limits.vector.max_edges {
            return Err(Error::Budget);
        }
        Some(lines)
    } else {
        None
    };
    let mut native = Vec::new();
    native.try_reserve_exact(65536 * channels).map_err(|_| Error::Allocation)?;
    if native.capacity() > 65536 * channels {
        return Err(Error::Budget);
    }
    let mut result = Surface::new(format);
    for ty in roi.y0.div_euclid(256)..=(roi.y1 - 1).div_euclid(256) {
        for tx in roi.x0.div_euclid(256)..=(roi.x1 - 1).div_euclid(256) {
            poll(control)?;
            let x = tx.checked_mul(256).ok_or(Error::Budget)?;
            let y = ty.checked_mul(256).ok_or(Error::Budget)?;
            let tile = Rect::new(
                x.max(roi.x0),
                y.max(roi.y0),
                x.checked_add(256).ok_or(Error::Budget)?.min(roi.x1),
                y.checked_add(256).ok_or(Error::Budget)?.min(roi.y1),
            );
            let coverage = match (&compiled, &lines) {
                (Some(compiled), None) => compiled.try_render_with_interrupt(tile, limits.vector, control)?,
                (None, Some(lines)) => binary_coverage(lines, &mask.path, tile, control)?,
                _ => return Err(Error::Unsupported("candidate text coverage policy binding")),
            };
            native.clear();
            for (i, c) in coverage.into_iter().enumerate() {
                if i % 256 == 0 {
                    poll(control)?;
                }
                if !c.is_finite() || !(0.0..=1.0).contains(&c) {
                    return Err(Error::Unsupported("vector coverage domain"));
                }
                for value in values.iter().take(channels - 1) {
                    native.push(*value);
                }
                native.push(alpha * f64::from(c));
            }
            result.try_write_region_f64_with(tile, &native, control)?;
        }
    }
    poll(control)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_text::xcf::unit_outline::GlyphSpan;
    fn rectangle() -> PreparedUnitOutline {
        let commands = vec![Command::Move([0.0, 0.0]), Command::Line([2.0, 0.0]), Command::Line([2.0, 2.0]), Command::Line([0.0, 2.0]), Command::Close];
        let raster_policy = photocraft_text::xcf::unit_raster_policy::SourceRasterPolicyDetails {
            switches: photocraft_text::xcf::unit_raster_policy::SourceRasterPolicy { hinting: false, antialias: true },
            hint_style: Some("none"),
            raw_hinting: Some("no"),
            raw_antialias: Some("yes"),
        }
        .require_unhinted()
        .unwrap();
        PreparedUnitOutline { raster_policy, commands, glyphs: vec![GlyphSpan { glyph_id: 1, start: 0, end: 5 }], retained_bytes: 0 }
    }
    #[test]
    fn binary_is_center_winding_not_thresholded_area() {
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let mut outline = rectangle();
        outline.commands = vec![Command::Move([0.6, 0.6]), Command::Line([1.6, 0.6]), Command::Line([1.6, 1.6]), Command::Line([0.6, 1.6]), Command::Close];
        let area = render(
            &outline,
            Rect::new(0, 0, 2, 2),
            PixelFormat::RGBA64F,
            NativeColor::Rgb([0.25, 0.5, 0.75, 1.0]),
            context,
            context,
            Limits::default(),
            &Interrupt::NONE,
            &mut |_| Ok(()),
        )
        .unwrap();
        outline.raster_policy = photocraft_text::xcf::unit_raster_policy::SourceRasterPolicyDetails {
            switches: photocraft_text::xcf::unit_raster_policy::SourceRasterPolicy { hinting: false, antialias: false },
            hint_style: Some("none"),
            raw_hinting: Some("no"),
            raw_antialias: Some("no"),
        }
        .require_unhinted()
        .unwrap();
        let binary = render(
            &outline,
            Rect::new(0, 0, 2, 2),
            PixelFormat::RGBA64F,
            NativeColor::Rgb([0.25, 0.5, 0.75, 1.0]),
            context,
            context,
            Limits::default(),
            &Interrupt::NONE,
            &mut |_| Ok(()),
        )
        .unwrap();
        let mut pixel = [0.0; 4];
        area.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
        assert!(pixel[3] > 0.0 && pixel[3] < 1.0);
        binary.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
        assert_eq!(pixel[3], 0.0);
        binary.try_read_pixel_f64(1, 1, &mut pixel).unwrap();
        assert_eq!(pixel, [0.25, 0.5, 0.75, 1.0]);
    }
    #[test]
    fn explicit_f64_hdr_straight_color_and_roi_default() {
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let out = render(
            &rectangle(),
            Rect::new(0, 0, 2, 2),
            PixelFormat::RGBA64F,
            NativeColor::Rgb([1.25, -0.5, 0.125, 0.5]),
            context,
            context,
            Limits::default(),
            &Interrupt::NONE,
            &mut |_| Ok(()),
        )
        .unwrap();
        let mut pixel = [0.0; 4];
        out.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
        assert_eq!(pixel, [1.25, -0.5, 0.125, 0.5]);
        out.try_read_pixel_f64(3, 0, &mut pixel).unwrap();
        assert_eq!(pixel, [0.0; 4]);
    }
    #[test]
    fn context_and_early_cancel_never_return_partial_surface() {
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let wrong = WorkingContext { profile_identity: [2; 32], ..context };
        let mut charges = 0;
        assert!(matches!(
            render(
                &rectangle(),
                Rect::new(0, 0, 2, 2),
                PixelFormat::RGBA8,
                NativeColor::Rgb([1.0; 4]),
                context,
                wrong,
                Limits::default(),
                &Interrupt::NONE,
                &mut |_| {
                    charges += 1;
                    Ok(())
                }
            ),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(charges, 0);
        let cancel = || true;
        assert!(matches!(
            render(
                &rectangle(),
                Rect::new(0, 0, 2, 2),
                PixelFormat::RGBA8,
                NativeColor::Rgb([1.0; 4]),
                context,
                context,
                Limits::default(),
                &Interrupt::cancel_only(&cancel),
                &mut |_| Ok(())
            ),
            Err(Error::Cancelled)
        ));
    }
}
