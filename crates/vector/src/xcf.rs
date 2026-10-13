//! Checked solid saved-path regeneration; source caches and resources are never read.
mod arch;
mod curves;
use crate::{Rasterizer, StrokeStyle, fill_rasterizer, stroke_rasterizer};
pub use arch::monotone_arch_descriptor;
pub use curves::{CurveContour, tessellate_curves_bounded};
fn dashed_stroke(path: &Path, style: &StrokeStyle) -> Rasterizer {
    let lines = crate::flatten_path(path, 0.001);
    let mut pieces = Vec::new();
    for line in lines {
        let mut dashes = crate::stroke::dash_polyline(&line, &style.dashes, style.dash_offset);
        // A closed stroke has no cap at its arbitrary source starting knot when
        // the initial and final on-interval are contiguous across that boundary.
        let continuous = line.closed
            && dashes.len() > 1
            && dashes
                .first()
                .and_then(|p| p.pts.first())
                .zip(dashes.last().and_then(|p| p.pts.last()))
                .is_some_and(|(a, b)| (a.0 - b.0).hypot(a.1 - b.1) < 1e-7);
        if continuous {
            let first = dashes.remove(0);
            if let Some(last) = dashes.last_mut() {
                last.pts.extend(first.pts.into_iter().skip(1));
                last.knot.extend(first.knot.into_iter().skip(1));
            }
        }
        pieces.extend(dashes);
    }
    let simple = StrokeStyle { width: style.width, cap: style.cap, join: style.join, miter_limit: style.miter_limit, dashes: Vec::new(), dash_offset: 0. };
    let polygons = crate::stroke_polygons(&pieces, &simple, 0.001);
    let mut raster = Rasterizer::new(false);
    raster.add_component(&polygons, PathOp::Combine, FillRule::NonZero);
    raster
}
use photocraft_doc::{FillRule, LineCap, LineJoin, Path, PathOp, PixelFormat};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

#[derive(Clone, Debug)]
pub struct SolidStroke {
    pub color: [f64; 4],
    pub style: StrokeStyle,
}
#[derive(Clone, Debug, Default)]
pub struct SolidSettings {
    pub fill: Option<[f64; 4]>,
    pub stroke: Option<SolidStroke>,
    pub antialias: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VectorError {
    Unsupported(&'static str),
    Cancelled,
    Budget,
    Allocation,
}
impl std::fmt::Display for VectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(f, "XCF vector regeneration: {reason}"),
            Self::Cancelled => f.write_str("XCF vector regeneration cancelled"),
            Self::Budget => f.write_str("XCF vector regeneration budget denied"),
            Self::Allocation => f.write_str("XCF vector regeneration allocation denied"),
        }
    }
}
impl std::error::Error for VectorError {}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleDescriptor {
    pub original_bounds: [f64; 4],
    /// Quantized vertical bounds in fifteenths of a pixel.
    pub subrow_bounds: [i32; 2],
}
/// Pure geometry eligibility only. The caller must separately qualify source
/// epoch, fill/configuration, sample encoding, profile and compositing state.
pub fn rectangle_descriptor(path: &Path) -> Result<RectangleDescriptor, VectorError> {
    if path.inverted || path.fill_rule != FillRule::NonZero || path.subpaths.len() != 1 {
        return Err(VectorError::Unsupported("unqualified rectangle path structure"));
    }
    let subpath = path.subpaths.first().ok_or(VectorError::Unsupported("missing rectangle subpath"))?;
    if !subpath.closed
        || subpath.op != PathOp::Combine
        || subpath.knots.iter().any(|k| !k.anchor.x.is_finite() || !k.anchor.y.is_finite() || k.anchor.x.abs() > 4096. || k.anchor.y.abs() > 4096.)
    {
        return Err(VectorError::Unsupported("unqualified rectangle path structure"));
    }
    let quantized = cubic_rectangle(path).ok_or(VectorError::Unsupported("unqualified rectangle geometry or coordinate grid"))?;
    let mut original = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for knot in &subpath.knots {
        original[0] = original[0].min(knot.anchor.x);
        original[1] = original[1].min(knot.anchor.y);
        original[2] = original[2].max(knot.anchor.x);
        original[3] = original[3].max(knot.anchor.y);
    }
    Ok(RectangleDescriptor { original_bounds: original, subrow_bounds: [quantized[1] as i32, quantized[3] as i32] })
}
/// Measured integer-axis open Butt/Square/Round segment, with stock width padding
/// retained along both axes. Butt leaves endpoint padding blank; Square paints
/// the half-width extensions, as specified by the public Cairo line-cap contract.
pub fn open_axis_stroke_descriptor(path: &Path, style: &StrokeStyle) -> Result<Rect, VectorError> {
    let r = open_axis_segment(path, style)?;
    let half = style.width * 0.5;
    let horizontal = r[2] - r[0] > style.width;
    let outer = if horizontal { [r[0] - half, r[1], r[2] + half, r[3]] } else { [r[0], r[1] - half, r[2], r[3] + half] };
    if outer[2] - outer[0] > 4096. || outer[3] - outer[1] > 4096. {
        return Err(VectorError::Unsupported("open-axis storage extent"));
    }
    Ok(Rect::new(outer[0] as i32, outer[1] as i32, outer[2] as i32, outer[3] as i32))
}
fn open_axis_ink(path: &Path, style: &StrokeStyle) -> Result<[f64; 4], VectorError> {
    let r = open_axis_segment(path, style)?;
    if style.cap == LineCap::Square {
        let half = style.width * 0.5;
        if r[2] - r[0] > style.width { Ok([r[0] - half, r[1], r[2] + half, r[3]]) } else { Ok([r[0], r[1] - half, r[2], r[3] + half]) }
    } else {
        Ok(r)
    }
}
fn open_axis_segment(path: &Path, style: &StrokeStyle) -> Result<[f64; 4], VectorError> {
    if path.inverted
        || path.fill_rule != FillRule::NonZero
        || path.subpaths.len() != 1
        || !matches!(style.cap, LineCap::Butt | LineCap::Square | LineCap::Round)
        || style.join != LineJoin::Miter
        || ![3., 5., 6., 8.].contains(&style.width)
        || style.miter_limit != 10.
        || !style.dashes.is_empty()
        || style.dash_offset != 0.
    {
        return Err(VectorError::Unsupported("unqualified open-axis stroke style"));
    }
    let sub = path.subpaths.first().ok_or(VectorError::Unsupported("missing open-axis path"))?;
    if sub.closed || sub.op != PathOp::Combine || !(2..=4).contains(&sub.knots.len()) {
        return Err(VectorError::Unsupported("unqualified open-axis topology"));
    }
    for k in &sub.knots {
        if k.in_ctrl != k.anchor
            || k.out_ctrl != k.anchor
            || !k.anchor.x.is_finite()
            || !k.anchor.y.is_finite()
            || k.anchor.x.abs() > 4096.
            || k.anchor.y.abs() > 4096.
            || k.anchor.x.fract() != 0.
            || k.anchor.y.fract() != 0.
        {
            return Err(VectorError::Unsupported("unqualified open-axis coordinates"));
        }
    }
    let a = sub.knots.first().ok_or(VectorError::Budget)?.anchor;
    let b = sub.knots.last().ok_or(VectorError::Budget)?.anchor;
    // Additional anchors are qualified only as strictly monotone integer-axis
    // subdivisions. Retraces, zero segments and corners have separate join semantics.
    for pair in sub.knots.windows(2) {
        let left = pair.first().ok_or(VectorError::Budget)?.anchor;
        let right = pair.get(1).ok_or(VectorError::Budget)?.anchor;
        let monotone = if a.y == b.y {
            left.y == a.y && right.y == a.y && ((b.x > a.x && right.x > left.x) || (b.x < a.x && right.x < left.x))
        } else if a.x == b.x {
            left.x == a.x && right.x == a.x && ((b.y > a.y && right.y > left.y) || (b.y < a.y && right.y < left.y))
        } else {
            false
        };
        if !monotone {
            return Err(VectorError::Unsupported("unqualified open-axis subdivision"));
        }
    }
    let half = style.width * 0.5;
    if a.y == b.y && (a.x - b.x).abs() > style.width {
        Ok([a.x.min(b.x), a.y - half, a.x.max(b.x), a.y + half])
    } else if a.x == b.x && (a.y - b.y).abs() > style.width {
        Ok([a.x - half, a.y.min(b.y), a.x + half, a.y.max(b.y)])
    } else {
        Err(VectorError::Unsupported("unqualified open-axis direction or short extent"))
    }
}
/// Direct Euclidean capsule membership for the independently authored integer
/// axis/no-AA Round-cap domain. Integer endpoints/even widths give integer
/// radius squared and half-integer squared pixel-center distance for even widths.
/// Odd-width straight-body boundary ties use the independently observed half-open
/// rectangle rule; the cap circle is a strict-distance region. No curve tessellation
/// or backend runtime is used.
#[allow(clippy::too_many_arguments)]
fn render_open_round(
    path: &Path,
    stroke: &SolidStroke,
    bounds: Rect,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &mut dyn FnMut(u64) -> Result<(), VectorError>,
    decode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
    encode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
) -> Result<Surface, VectorError> {
    let r = open_axis_segment(path, &stroke.style)?;
    open_axis_stroke_descriptor(path, &stroke.style)?;
    color(stroke.color)?;
    if stroke.color[3] != 1. {
        return Err(VectorError::Unsupported("unqualified Round-cap solid alpha"));
    }
    let width = i64::from(bounds.x1) - i64::from(bounds.x0);
    let height = i64::from(bounds.y1) - i64::from(bounds.y0);
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 || [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().any(|v| i64::from(*v).abs() > 1 << 29)
    {
        return Err(VectorError::Unsupported("Round-cap output extent"));
    }
    let pixels = usize::try_from(width.checked_mul(height).ok_or(VectorError::Budget)?).map_err(|_| VectorError::Budget)?;
    if pixels > 1 << 20 {
        return Err(VectorError::Unsupported("Round-cap pixel bound"));
    }
    let bytes = pixels.checked_mul(4).ok_or(VectorError::Budget)?;
    let tx = i64::from((bounds.x1 - 1).div_euclid(256)) - i64::from(bounds.x0.div_euclid(256)) + 1;
    let ty = i64::from((bounds.y1 - 1).div_euclid(256)) - i64::from(bounds.y0.div_euclid(256)) + 1;
    let tile_bytes = usize::try_from(tx.checked_mul(ty).ok_or(VectorError::Budget)?)
        .map_err(|_| VectorError::Budget)?
        .checked_mul(256 * 256 * 4)
        .ok_or(VectorError::Budget)?;
    charge(bytes.checked_add(tile_bytes).ok_or(VectorError::Budget)?)?;
    poll(ctl)?;
    let linear = decode([stroke.color[0], stroke.color[1], stroke.color[2]])?;
    color([linear[0], linear[1], linear[2], 1.])?;
    let rgb = encode(linear)?;
    color([rgb[0], rgb[1], rgb[2], 1.])?;
    let black = encode([0.; 3])?;
    color([black[0], black[1], black[2], 0.])?;
    let ink = [(rgb[0] * 255.).round() as u8, (rgb[1] * 255.).round() as u8, (rgb[2] * 255.).round() as u8, 255];
    let empty = [(black[0] * 255.).round() as u8, (black[1] * 255.).round() as u8, (black[2] * 255.).round() as u8, 0];
    let half = stroke.style.width * 0.5;
    let horizontal = r[2] - r[0] > stroke.style.width;
    let mut output = Vec::new();
    output.try_reserve_exact(bytes).map_err(|_| VectorError::Allocation)?;
    for y in bounds.y0..bounds.y1 {
        poll(ctl)?;
        work((width as u64).checked_mul(4).ok_or(VectorError::Budget)?)?;
        for x in bounds.x0..bounds.x1 {
            if (i64::from(x) - i64::from(bounds.x0)) % 512 == 0 {
                poll(ctl)?;
            }
            let px = f64::from(x) + 0.5;
            let py = f64::from(y) + 0.5;
            let (dx, dy) = if horizontal { (px - px.clamp(r[0], r[2]), py - (r[1] + half)) } else { (px - (r[0] + half), py - py.clamp(r[1], r[3])) };
            let body = px >= r[0] && px < r[2] && py >= r[1] && py < r[3];
            output.extend_from_slice(if body || dx * dx + dy * dy < half * half { &ink } else { &empty });
        }
    }
    poll(ctl)?;
    Surface::try_from_interleaved_with(PixelFormat::RGBA8, bounds, &output, ctl)
        .map_err(|_| if ctl.cancelled() { VectorError::Cancelled } else { VectorError::Allocation })
}
/// Checked no-AA closed rectangle stroke with centered Butt/Miter or integer Bevel/Round geometry.
/// Source schema/profile admission remains the caller's responsibility.
pub fn aliased_rectangle_stroke_descriptor(path: &Path, style: &StrokeStyle) -> Result<[f64; 4], VectorError> {
    if style.cap != LineCap::Butt
        || !matches!(style.join, LineJoin::Miter | LineJoin::Bevel | LineJoin::Round)
        || !style.dashes.is_empty()
        || style.dash_offset != 0.
        || !style.width.is_finite()
        || !(1.0..=32.).contains(&style.width)
        || style.width.fract() != 0.
        || !style.miter_limit.is_finite()
        || !(2.0..=16.).contains(&style.miter_limit)
    {
        return Err(VectorError::Unsupported("unqualified aliased rectangle stroke style"));
    }
    polygon_descriptor(path)?;
    let r = rectangle_descriptor(path)?.original_bounds;
    if style.join != LineJoin::Miter && (r.iter().any(|v| v.fract() != 0.) || style.width >= (r[2] - r[0]).min(r[3] - r[1])) {
        return Err(VectorError::Unsupported("unqualified non-miter rectangle anchors or inner extent"));
    }
    let half = style.width * 0.5;
    Ok([r[0] - half, r[1] - half, r[2] + half, r[3] + half])
}
/// Observed stock vector-layer storage extent for an integer-anchor AA rectangle
/// stroke. The integer span is placed at the minimum truncated toward zero, so an odd width
/// clips its final half-pixel column/row. This is distinct from ideal ink bounds.
pub fn antialiased_rectangle_stroke_descriptor(path: &Path, style: &StrokeStyle) -> Result<Rect, VectorError> {
    integer_rectangle_stroke_storage(path, style)
}
/// Independently observed no-AA Butt/Miter integer-anchor storage extent.
/// Odd widths place the integer span at the minimum truncated toward zero, like the
/// separately observed AA domain; this does not qualify other joins or styles.
pub fn aliased_rectangle_stroke_storage_descriptor(path: &Path, style: &StrokeStyle) -> Result<Rect, VectorError> {
    integer_rectangle_stroke_storage(path, style)
}
fn integer_rectangle_stroke_storage(path: &Path, style: &StrokeStyle) -> Result<Rect, VectorError> {
    if style.join != LineJoin::Miter {
        return Err(VectorError::Unsupported("unqualified integer-storage rectangle join"));
    }
    let outer = aliased_rectangle_stroke_descriptor(path, style)?;
    let r = rectangle_descriptor(path)?.original_bounds;
    if r.iter().any(|v| v.fract() != 0.) || style.width >= (r[2] - r[0]).min(r[3] - r[1]) {
        return Err(VectorError::Unsupported("unqualified integer-storage rectangle stroke anchors or inner extent"));
    }
    let width = outer[2] - outer[0];
    let height = outer[3] - outer[1];
    if width > 4096. || height > 4096. {
        return Err(VectorError::Unsupported("integer rectangle stroke storage extent"));
    }
    let x = outer[0].trunc() as i32;
    let y = outer[1].trunc() as i32;
    Ok(Rect::new(x, y, x.checked_add(width as i32).ok_or(VectorError::Budget)?, y.checked_add(height as i32).ok_or(VectorError::Budget)?))
}
fn antialiased_rectangle_ring(path: &Path, style: &StrokeStyle) -> Result<([f64; 4], [f64; 4]), VectorError> {
    antialiased_rectangle_stroke_descriptor(path, style)?;
    let r = rectangle_descriptor(path)?.original_bounds;
    let half = style.width * 0.5;
    let quantize = |mut r: [f64; 4]| {
        r[1] = (r[1] * 15. + 0.5).floor();
        r[3] = (r[3] * 15. + 0.5).floor();
        r
    };
    Ok((quantize([r[0] - half, r[1] - half, r[2] + half, r[3] + half]), quantize([r[0] + half, r[1] + half, r[2] - half, r[3] - half])))
}
fn rectangle_sample_area(r: [f64; 4], x: f64, y: f64) -> f64 {
    (r[2].min(x + 1.) - r[0].max(x)).max(0.) * (r[3].min((y + 1.) * 15.) - r[1].max(y * 15.)).max(0.)
}
/// Bevel outline sampled at the symbolic lower Y-center limit. The on-edge
/// crossing rule was independently observed for integer no-AA stock strokes,
/// including reversed winding and negative origins; no numerical offset is used.
fn in_bevel_rectangle(outer: [f64; 4], half: f64, x: f64, y: f64) -> bool {
    let [x0, y0, x1, y1] = outer;
    let points = [(x0 + half, y0), (x1 - half, y0), (x1, y0 + half), (x1, y1 - half), (x1 - half, y1), (x0 + half, y1), (x0, y1 - half), (x0, y0 + half)];
    let mut winding = 0i32;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(8) {
        let dx = b.0 - a.0;
        let side = dx * (y - a.1) - (b.1 - a.1) * (x - a.0);
        if a.1 <= y && b.1 > y && (side > 0. || (side == 0. && dx < 0.)) {
            winding += 1;
        } else if a.1 > y && b.1 <= y && (side < 0. || (side == 0. && dx > 0.)) {
            winding -= 1;
        }
    }
    winding != 0
}
fn in_rectangle(rect: [f64; 4], x: f64, y: f64) -> bool {
    rect[0] <= x && x < rect[2] && rect[1] <= y && y < rect[3]
}

/// Scoped simple rectilinear polygon geometry; no curve flattening or cache authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AliasedFillRule {
    NonZero,
    EvenOdd,
}
/// Backward-compatible domain where canonical nonzero agrees with measured parity.
pub fn polygon_descriptor(path: &Path) -> Result<[f64; 4], VectorError> {
    polygon_descriptor_with_rule(path, AliasedFillRule::NonZero)
}
/// Explicit evaluator behavior; this never rewrites canonical path metadata.
pub fn polygon_descriptor_with_rule(path: &Path, rule: AliasedFillRule) -> Result<[f64; 4], VectorError> {
    let count = path.subpaths.iter().try_fold(0usize, |n, s| n.checked_add(s.knots.len())).ok_or(VectorError::Budget)?;
    if path.inverted || path.fill_rule != FillRule::NonZero || !(1..=16).contains(&path.subpaths.len()) || count > 256 {
        return Err(VectorError::Unsupported("unqualified compound polygon structure"));
    }
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (i, sub) in path.subpaths.iter().enumerate() {
        let b = single_polygon_descriptor(sub)?;
        bounds[0] = bounds[0].min(b[0]);
        bounds[1] = bounds[1].min(b[1]);
        bounds[2] = bounds[2].max(b[2]);
        bounds[3] = bounds[3].max(b[3]);
        for other in path.subpaths.iter().skip(i + 1) {
            for (j, a) in sub.knots.iter().enumerate() {
                let b = sub.knots.get((j + 1) % sub.knots.len()).ok_or(VectorError::Budget)?.anchor;
                for (k, c) in other.knots.iter().enumerate() {
                    let d = other.knots.get((k + 1) % other.knots.len()).ok_or(VectorError::Budget)?.anchor;
                    if a.anchor.x.min(b.x) <= c.anchor.x.max(d.x)
                        && c.anchor.x.min(d.x) <= a.anchor.x.max(b.x)
                        && a.anchor.y.min(b.y) <= c.anchor.y.max(d.y)
                        && c.anchor.y.min(d.y) <= a.anchor.y.max(b.y)
                    {
                        return Err(VectorError::Unsupported("touching or intersecting compound contours"));
                    }
                }
            }
        }
    }
    // Stock reverse-stroke control establishes an empty nested region for
    // either orientation. Admit only geometry where our nonzero canonical
    // representation agrees with that observed parity behavior.
    if rule == AliasedFillRule::NonZero {
        for (i, sub) in path.subpaths.iter().enumerate() {
            let first = sub.knots.first().ok_or(VectorError::Budget)?.anchor;
            let mut area = 0.;
            for (j, a) in sub.knots.iter().enumerate() {
                let b = sub.knots.get((j + 1) % sub.knots.len()).ok_or(VectorError::Budget)?.anchor;
                area += a.anchor.x * b.y - b.x * a.anchor.y;
            }
            if area == 0. {
                return Err(VectorError::Unsupported("zero-area contour"));
            }
            let mut winding = if area > 0. { 1i32 } else { -1 };
            let mut depth = 1usize;
            for (j, other) in path.subpaths.iter().enumerate() {
                if i == j {
                    continue;
                }
                let contribution = contour_winding(other, first.x, first.y)?;
                if contribution != 0 {
                    depth += 1;
                    winding += contribution;
                }
            }
            if (winding != 0) == depth.is_multiple_of(2) {
                return Err(VectorError::Unsupported("nested contour winding differs from observed stock fill"));
            }
        }
    }
    Ok(bounds)
}
fn single_polygon_descriptor(sub: &photocraft_doc::Subpath) -> Result<[f64; 4], VectorError> {
    if !sub.closed || sub.op != PathOp::Combine || !(3..=64).contains(&sub.knots.len()) {
        return Err(VectorError::Unsupported("unqualified aliased polygon structure"));
    }
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (index, k) in sub.knots.iter().enumerate() {
        let next = sub.knots.get((index + 1) % sub.knots.len()).ok_or(VectorError::Unsupported("polygon edge"))?;
        if (next.anchor.x == k.anchor.x) == (next.anchor.y == k.anchor.y) {
            return Err(VectorError::Unsupported("unqualified aliased diagonal or degenerate edge"));
        }
        if k.in_ctrl != k.anchor
            || k.out_ctrl != k.anchor
            || [k.anchor.x, k.anchor.y].iter().any(|v| !v.is_finite() || v.abs() > 4096. || (*v * 256.).round() != *v * 256.)
        {
            return Err(VectorError::Unsupported("unqualified aliased polygon geometry"));
        }
        bounds[0] = bounds[0].min(k.anchor.x);
        bounds[1] = bounds[1].min(k.anchor.y);
        bounds[2] = bounds[2].max(k.anchor.x);
        bounds[3] = bounds[3].max(k.anchor.y);
    }
    for (i, a) in sub.knots.iter().enumerate() {
        let b = sub.knots.get((i + 1) % sub.knots.len()).ok_or(VectorError::Unsupported("polygon edge"))?.anchor;
        for (j, c) in sub.knots.iter().enumerate().skip(i + 1) {
            if j == i + 1 || (i == 0 && j + 1 == sub.knots.len()) {
                continue;
            }
            let d = sub.knots.get((j + 1) % sub.knots.len()).ok_or(VectorError::Unsupported("polygon edge"))?.anchor;
            if a.anchor.x.min(b.x) <= c.anchor.x.max(d.x)
                && c.anchor.x.min(d.x) <= a.anchor.x.max(b.x)
                && a.anchor.y.min(b.y) <= c.anchor.y.max(d.y)
                && c.anchor.y.min(d.y) <= a.anchor.y.max(b.y)
            {
                return Err(VectorError::Unsupported("unqualified intersecting rectilinear polygon"));
            }
        }
    }
    if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
        return Err(VectorError::Unsupported("empty polygon extent"));
    }
    Ok(bounds)
}
fn contour_winding(sub: &photocraft_doc::Subpath, x: f64, y: f64) -> Result<i32, VectorError> {
    let knots = &sub.knots;
    let mut winding = 0i32;
    for (i, k) in knots.iter().enumerate() {
        let a = k.anchor;
        let b = knots.get((i + 1) % knots.len()).ok_or(VectorError::Unsupported("polygon edge"))?.anchor;
        let side = (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
        if a.y <= y && b.y > y && side > 0. {
            winding += 1;
        } else if a.y > y && b.y <= y && side < 0. {
            winding -= 1;
        }
    }
    Ok(winding)
}
fn polygon_contains(path: &Path, x: f64, y: f64, rule: AliasedFillRule) -> Result<bool, VectorError> {
    let mut winding = 0i32;
    for sub in &path.subpaths {
        winding += contour_winding(sub, x, y)?;
    }
    Ok(match rule {
        AliasedFillRule::NonZero => winding != 0,
        AliasedFillRule::EvenOdd => !winding.unsigned_abs().is_multiple_of(2),
    })
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), VectorError> {
    ctl.check().map_err(|_| VectorError::Cancelled)
}
fn color(value: [f64; 4]) -> Result<(), VectorError> {
    if value.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) { Err(VectorError::Unsupported("unqualified solid color range")) } else { Ok(()) }
}
/// A scoped model of the observed stock Cairo image-backend cubic-rectangle
/// coverage. The public 1.11.4 release description documents a 256-by-15 grid;
/// independent fractional sweeps/holdouts establish nearest-Y quantization for
/// this geometry. This does not describe diagonal or curved scan conversion.
fn cubic_rectangle(path: &Path) -> Option<[f64; 4]> {
    let subpath = path.subpaths.first()?;
    if subpath.knots.len() != 4 {
        return None;
    }
    let first = subpath.knots.first()?.anchor;
    let mut bounds = [first.x, first.y, first.x, first.y];
    for (index, knot) in subpath.knots.iter().enumerate() {
        if subpath.knots.iter().take(index).any(|previous| previous.anchor == knot.anchor) {
            return None;
        }
        if knot.in_ctrl != knot.anchor
            || knot.out_ctrl != knot.anchor
            || (knot.anchor.x * 256.).round() != knot.anchor.x * 256.
            || (knot.anchor.y * 256.).round() != knot.anchor.y * 256.
        {
            return None;
        }
        let next = subpath.knots.get((index + 1) % 4)?.anchor;
        if (next.x == knot.anchor.x) == (next.y == knot.anchor.y) {
            return None;
        }
        bounds[0] = bounds[0].min(knot.anchor.x);
        bounds[1] = bounds[1].min(knot.anchor.y);
        bounds[2] = bounds[2].max(knot.anchor.x);
        bounds[3] = bounds[3].max(knot.anchor.y);
    }
    // Keep Y in integer subrow units until integration to avoid translated
    // floating-point cancellation changing alpha at quantization ties.
    // floor(v + 0.5) preserves integer-translation equivalence at negative ties.
    bounds[1] = (bounds[1] * 15. + 0.5).floor();
    bounds[3] = (bounds[3] * 15. + 0.5).floor();
    Some(bounds)
}
/// Colors are explicitly resolved by the caller in the requested document sample
/// encoding. This layer performs no color-profile conversion or resource lookup.
/// This bounded candidate produces RGBA8; GIMP raster fidelity remains separately qualified.
pub fn render_solid(
    path: &Path,
    settings: &SolidSettings,
    bounds: Rect,
    format: PixelFormat,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &mut dyn FnMut(u64) -> Result<(), VectorError>,
) -> Result<Surface, VectorError> {
    render_solid_with_transform(path, settings, bounds, format, ctl, charge, work, &|rgb| Ok(rgb), &|rgb| Ok(rgb))
}
/// Explicit sample-encoding conversions for linear-light fill/stroke composition.
/// The caller supplies qualified profile transforms; callbacks cannot grant resource access.
#[allow(clippy::too_many_arguments)]
pub fn render_solid_with_rule(
    path: &Path,
    settings: &SolidSettings,
    bounds: Rect,
    format: PixelFormat,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &mut dyn FnMut(u64) -> Result<(), VectorError>,
    rule: AliasedFillRule,
) -> Result<Surface, VectorError> {
    render_solid_with_rule_and_transform(path, settings, bounds, format, ctl, charge, work, rule, &|rgb| Ok(rgb), &|rgb| Ok(rgb))
}
#[allow(clippy::too_many_arguments)]
pub fn render_solid_with_transform(
    path: &Path,
    settings: &SolidSettings,
    bounds: Rect,
    format: PixelFormat,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &mut dyn FnMut(u64) -> Result<(), VectorError>,
    decode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
    encode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
) -> Result<Surface, VectorError> {
    render_solid_with_rule_and_transform(path, settings, bounds, format, ctl, charge, work, AliasedFillRule::NonZero, decode, encode)
}
#[allow(clippy::too_many_arguments)]
pub fn render_solid_with_rule_and_transform(
    path: &Path,
    settings: &SolidSettings,
    bounds: Rect,
    format: PixelFormat,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &mut dyn FnMut(u64) -> Result<(), VectorError>,
    rule: AliasedFillRule,
    decode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
    encode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
) -> Result<Surface, VectorError> {
    poll(ctl)?;
    if format != PixelFormat::RGBA8 {
        return Err(VectorError::Unsupported("unqualified vector output format"));
    }
    if path.inverted || path.fill_rule != FillRule::NonZero || (path.subpaths.len() != 1 && (settings.antialias || settings.stroke.is_some())) {
        return Err(VectorError::Unsupported("unqualified antialias/fill-rule/path components"));
    }
    let sub = path.subpaths.first().ok_or(VectorError::Unsupported("missing path"))?;
    if sub.closed && sub.knots.len() == 2 && !settings.antialias && settings.stroke.is_none() {
        let fill = settings.fill.ok_or(VectorError::Unsupported("missing arch fill"))?;
        return arch::render(path, fill, bounds, ctl, charge, work, decode, encode);
    }
    if !sub.closed && !settings.antialias && settings.fill.is_none() {
        let stroke = settings.stroke.as_ref().ok_or(VectorError::Unsupported("missing open-axis stroke"))?;
        if stroke.style.cap == LineCap::Round {
            return render_open_round(path, stroke, bounds, ctl, charge, work, decode, encode);
        }
        let r = open_axis_ink(path, &stroke.style)?;
        charge(4 * std::mem::size_of::<photocraft_doc::Knot>() + std::mem::size_of::<photocraft_doc::Subpath>())?;
        work(4)?;
        poll(ctl)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(4).map_err(|_| VectorError::Allocation)?;
        for (x, y) in [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])] {
            let p = photocraft_geom::Point::new(x, y);
            knots.push(photocraft_doc::Knot { anchor: p, in_ctrl: p, out_ctrl: p, smooth: false });
        }
        let mut subpaths = Vec::new();
        subpaths.try_reserve_exact(1).map_err(|_| VectorError::Allocation)?;
        subpaths.push(photocraft_doc::Subpath { knots, closed: true, op: PathOp::Combine });
        let filled = Path { subpaths, fill_rule: FillRule::NonZero, inverted: false };
        let fill = SolidSettings { fill: Some(stroke.color), stroke: None, antialias: false };
        // One bounded delegation to a closed four-knot rectangle; no recursive input graph.
        return render_solid_with_rule_and_transform(&filled, &fill, bounds, format, ctl, charge, work, rule, decode, encode);
    }
    if !sub.closed || sub.op != PathOp::Combine || sub.knots.len() < 2 || sub.knots.len() > 64 {
        return Err(VectorError::Unsupported("unqualified path geometry"));
    }
    charge(64 * std::mem::size_of::<[photocraft_geom::Point; 4]>())?;
    // The analytic ring has constant topology and never builds flattened stroke edges.
    let antialiased_stroke = settings.stroke.as_ref().filter(|_| settings.antialias).and_then(|s| antialiased_rectangle_ring(path, &s.style).ok());
    let mut edge_bound = 0usize;
    let mut length_bound = 0.;
    for knot in path.subpaths.iter().flat_map(|s| &s.knots) {
        poll(ctl)?;
        for p in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
            if !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 4096. || p.y.abs() > 4096. {
                return Err(VectorError::Unsupported("path outside qualified numeric bounds"));
            }
        }
    }
    if !settings.antialias {
        polygon_descriptor_with_rule(path, rule)?;
    }
    for segment in path.subpaths.iter().flat_map(|s| s.segments()) {
        if !settings.antialias || antialiased_stroke.is_some() {
            edge_bound = edge_bound.checked_add(1).ok_or(VectorError::Budget)?;
            continue;
        }
        // Conservative cubic chord-error bound. Reject complex flattening before
        // invoking the existing rasterizer's allocation and global edge ordering.
        let [a, b, c, d] = segment;
        let second = (a.x - 2. * b.x + c.x).hypot(a.y - 2. * b.y + c.y).max((b.x - 2. * c.x + d.x).hypot(b.y - 2. * c.y + d.y));
        let length = (b.x - a.x).hypot(b.y - a.y) + (c.x - b.x).hypot(c.y - b.y) + (d.x - c.x).hypot(d.y - c.y);
        length_bound += length;
        let tolerance = 0.001f64.min(length * 0.001).max(0.0001);
        let bound = (0.75 * second / tolerance).sqrt().ceil().max(1.);
        if !bound.is_finite() || bound > 1024. {
            return Err(VectorError::Unsupported("path flattening work bound"));
        }
        edge_bound = edge_bound.checked_add(bound as usize).ok_or(VectorError::Budget)?;
    }
    if edge_bound > 1024 {
        return Err(VectorError::Unsupported("path edge bound"));
    }
    if let Some(fill) = settings.fill {
        color(fill)?;
    }
    let mut compiled_edges = edge_bound;
    if let Some(stroke) = &settings.stroke {
        color(stroke.color)?;
        if stroke.style.cap != LineCap::Butt
            || !(stroke.style.join == LineJoin::Miter || (!settings.antialias && matches!(stroke.style.join, LineJoin::Bevel | LineJoin::Round)))
            || !stroke.style.width.is_finite()
            || stroke.style.width <= 0.
            || stroke.style.width > 32.
            || !stroke.style.miter_limit.is_finite()
            || !(1.0..=16.).contains(&stroke.style.miter_limit)
            || stroke.style.dashes.len() > 16
            || stroke.style.dashes.iter().any(|dash| !dash.is_finite() || !(1.0..=1024.).contains(dash))
            || stroke.style.dash_offset != 0.
        {
            return Err(VectorError::Unsupported("unqualified stroke geometry"));
        }
        let cuts = stroke.style.dashes.iter().copied().reduce(f64::min).map_or(0., |minimum| (length_bound / minimum).ceil());
        if cuts > 512. {
            return Err(VectorError::Unsupported("dash subdivision bound"));
        }
        // Butt caps add no arcs; bound all segment/join polygon edges and dash cuts.
        compiled_edges = edge_bound.checked_mul(8).and_then(|n| n.checked_add(cuts as usize * 16)).ok_or(VectorError::Budget)?;
    }
    let convert = |rgba: [f64; 4]| -> Result<[f64; 4], VectorError> {
        poll(ctl)?;
        let rgb = decode([rgba[0], rgba[1], rgba[2]])?;
        let out = [rgb[0], rgb[1], rgb[2], rgba[3]];
        color(out)?;
        Ok(out)
    };
    let fill_color = settings.fill.map(convert).transpose()?.unwrap_or([0.; 4]);
    let stroke_color = settings.stroke.as_ref().map(|stroke| convert(stroke.color)).transpose()?.unwrap_or([0.; 4]);
    let width = i64::from(bounds.x1) - i64::from(bounds.x0);
    let height = i64::from(bounds.y1) - i64::from(bounds.y0);
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 || [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().any(|v| i64::from(*v).abs() > 1 << 29)
    {
        return Err(VectorError::Unsupported("output extent"));
    }
    let pixels = usize::try_from(width.checked_mul(height).ok_or(VectorError::Budget)?).map_err(|_| VectorError::Budget)?;
    if pixels > 1 << 20 {
        return Err(VectorError::Unsupported("output pixel bound"));
    }
    let bytes = pixels.checked_mul(4).ok_or(VectorError::Budget)?;
    // Complete destination tiles coexist with the interleaved output, including
    // partial tiles at negative or unaligned bounds.
    let tiles_x = i64::from((bounds.x1 - 1).div_euclid(256)) - i64::from(bounds.x0.div_euclid(256)) + 1;
    let tiles_y = i64::from((bounds.y1 - 1).div_euclid(256)) - i64::from(bounds.y0.div_euclid(256)) + 1;
    let tile_bytes = usize::try_from(tiles_x.checked_mul(tiles_y).ok_or(VectorError::Budget)?)
        .map_err(|_| VectorError::Budget)?
        .checked_mul(256 * 256 * 4)
        .ok_or(VectorError::Budget)?;
    // Account rasterizer edge/intersection scratch conservatively before compiling.
    charge(
        bytes
            .checked_add(tile_bytes)
            .ok_or(VectorError::Budget)?
            .checked_add(compiled_edges.checked_mul(compiled_edges).and_then(|n| n.checked_mul(64)).ok_or(VectorError::Budget)?)
            .and_then(|n| n.checked_add(width as usize * 32 + (2 << 20)))
            .ok_or(VectorError::Budget)?,
    )?;
    work((compiled_edges as u64).checked_mul(compiled_edges as u64).ok_or(VectorError::Budget)?)?;
    let aliased_stroke = if !settings.antialias {
        polygon_descriptor_with_rule(path, rule)?;
        settings
            .stroke
            .as_ref()
            .map(|stroke| {
                let outer = aliased_rectangle_stroke_descriptor(path, &stroke.style)?;
                let r = rectangle_descriptor(path)?.original_bounds;
                let half = stroke.style.width * 0.5;
                Ok::<_, VectorError>((outer, [r[0] + half, r[1] + half, r[2] - half, r[3] - half]))
            })
            .transpose()?
    } else {
        None
    };
    // Rectangle ring integration uses the same independently observed 15-row
    // quantization as rectangle fills. Subtract inner area before rounding once;
    // independently rounding two alpha values loses half-unit corner coverage.
    let rectangle = settings.fill.filter(|_| settings.antialias).and_then(|_| cubic_rectangle(path));
    let fill = settings.fill.filter(|_| settings.antialias && rectangle.is_none()).map(|_| fill_rasterizer(path, 0.001));
    poll(ctl)?;
    let stroke = settings
        .stroke
        .as_ref()
        .filter(|_| settings.antialias && antialiased_stroke.is_none())
        .map(|stroke| if stroke.style.dashes.is_empty() { stroke_rasterizer(path, &stroke.style, 0.001) } else { dashed_stroke(path, &stroke.style) });
    poll(ctl)?;
    let mut output = Vec::new();
    output.try_reserve_exact(bytes).map_err(|_| VectorError::Allocation)?;
    let mut fills = Vec::new();
    fills.try_reserve_exact(width as usize).map_err(|_| VectorError::Allocation)?;
    fills.resize(width as usize, 0.);
    let mut strokes = Vec::new();
    strokes.try_reserve_exact(width as usize).map_err(|_| VectorError::Allocation)?;
    strokes.resize(width as usize, 0.);
    for y in bounds.y0..bounds.y1 {
        poll(ctl)?;
        work((width as u64).checked_mul((compiled_edges as u64).max(4)).ok_or(VectorError::Budget)?)?;
        let row = Rect::new(bounds.x0, y, bounds.x1, y + 1);
        if let Some(fill) = &fill {
            fill.render_into(row, &mut fills);
        }
        if let Some(stroke) = &stroke {
            stroke.render_into(row, &mut strokes);
        }
        for x in 0..width as usize {
            if x % 1024 == 0 {
                poll(ctl)?;
            }
            let fc = if !settings.antialias && settings.fill.is_some() {
                if polygon_contains(path, f64::from(bounds.x0) + x as f64 + 0.5, f64::from(y) + 0.5, rule)? { 1. } else { 0. }
            } else if let Some([x0, y0, x1, y1]) = rectangle {
                let px = f64::from(bounds.x0) + x as f64;
                let py = f64::from(y);
                ((x1.min(px + 1.) - x0.max(px)).max(0.) * (y1.min((py + 1.) * 15.) - y0.max(py * 15.)).max(0.) * 17.).round() / 255.
            } else {
                f64::from(*fills.get(x).ok_or(VectorError::Budget)?)
            };
            let sc = if let Some((outer, inner)) = aliased_stroke {
                let px = f64::from(bounds.x0) + x as f64 + 0.5;
                let py = f64::from(y) + 0.5;
                let outside = if let Some(s) = settings.stroke.as_ref().filter(|s| s.style.join != LineJoin::Miter) {
                    let half = s.style.width * 0.5;
                    if s.style.join == LineJoin::Round {
                        // Euclidean dilation of the source rectangle. Integer anchors and
                        // even integer widths in the measured domain have no circular
                        // boundary passing through a half-integer pixel center.
                        let dx = (outer[0] + half - px).max(0.).max(px - (outer[2] - half));
                        let dy = (outer[1] + half - py).max(0.).max(py - (outer[3] - half));
                        dx * dx + dy * dy < half * half
                    } else {
                        in_bevel_rectangle(outer, half, px, py)
                    }
                } else {
                    in_rectangle(outer, px, py)
                };
                if outside && !in_rectangle(inner, px, py) { 1. } else { 0. }
            } else if let Some((outer, inner)) = antialiased_stroke {
                let px = f64::from(bounds.x0) + x as f64;
                let py = f64::from(y);
                ((rectangle_sample_area(outer, px, py) - rectangle_sample_area(inner, px, py)).max(0.) * 17.).round() / 255.
            } else {
                f64::from(*strokes.get(x).ok_or(VectorError::Budget)?)
            };
            let f = fill_color;
            let s = stroke_color;
            let fa = f[3] * fc;
            let sa = s[3] * sc;
            let alpha = sa + fa * (1. - sa);
            let mut linear = [0.; 3];
            for (c, out) in linear.iter_mut().enumerate() {
                *out = if alpha > 0. {
                    (s.get(c).copied().ok_or(VectorError::Budget)? * sa + f.get(c).copied().ok_or(VectorError::Budget)? * fa * (1. - sa)) / alpha
                } else {
                    0.
                };
            }
            let rgb = encode(linear)?;
            color([rgb[0], rgb[1], rgb[2], alpha])?;
            for value in rgb {
                output.push((value * 255.).round() as u8);
            }
            output.push((alpha.clamp(0., 1.) * 255.).round() as u8);
        }
    }
    poll(ctl)?;
    Surface::try_from_interleaved_with(format, bounds, &output, ctl).map_err(|error| {
        if ctl.cancelled() {
            VectorError::Cancelled
        } else {
            let _ = error;
            VectorError::Allocation
        }
    })
}
