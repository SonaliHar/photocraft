//! Portable binary coverage for a simple monotone cubic arch and its straight closure.
//! The independently observed fixed-grid subdivision and half-open spans are scoped
//! to this topology; they do not qualify arbitrary intersecting curved contours.
use super::{VectorError, color, poll, tessellate_curves_bounded};
use photocraft_color::PixelFormat;
use photocraft_doc::{FillRule, Path, PathOp};
use photocraft_geom::{Point, Rect};
use photocraft_raster::{Interrupt, Surface};

fn controls(path: &Path) -> Result<[Point; 4], VectorError> {
    if path.inverted || path.fill_rule != FillRule::NonZero || path.subpaths.len() != 1 {
        return Err(VectorError::Unsupported("monotone arch components"));
    }
    let source = path.subpaths.first().ok_or(VectorError::Unsupported("missing arch"))?;
    if !source.closed || source.op != PathOp::Combine || source.knots.len() != 2 {
        return Err(VectorError::Unsupported("monotone arch topology"));
    }
    let a = source.knots.first().ok_or(VectorError::Unsupported("arch endpoint"))?;
    let d = source.knots.get(1).ok_or(VectorError::Unsupported("arch endpoint"))?;
    if a.in_ctrl != a.anchor || d.out_ctrl != d.anchor || a.anchor.y != d.anchor.y {
        return Err(VectorError::Unsupported("arch straight baseline"));
    }
    let points = [a.anchor, a.out_ctrl, d.in_ctrl, d.anchor];
    for point in points {
        if !point.x.is_finite()
            || !point.y.is_finite()
            || point.x.abs() > 4096.
            || point.y.abs() > 4096.
            || (point.x * 256.).fract() != 0.
            || (point.y * 256.).fract() != 0.
        {
            return Err(VectorError::Unsupported("arch finite fixed-grid coordinate"));
        }
    }
    let [a, b, c, d] = points;
    let monotone = (a.x < b.x && b.x < c.x && c.x < d.x) || (a.x > b.x && b.x > c.x && c.x > d.x);
    if !monotone || b.y >= a.y || c.y >= a.y {
        return Err(VectorError::Unsupported("arch monotone upper controls"));
    }
    Ok(points)
}

/// Conservative control-hull bounds for the qualified topology. These describe
/// current geometry, independently of the original saved raster's storage extent.
pub fn monotone_arch_descriptor(path: &Path) -> Result<Rect, VectorError> {
    let points = controls(path)?;
    let mut hull = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for point in points {
        hull[0] = hull[0].min(point.x);
        hull[1] = hull[1].min(point.y);
        hull[2] = hull[2].max(point.x);
        hull[3] = hull[3].max(point.y);
    }
    let bounds = Rect::new(hull[0].floor() as i32, hull[1].floor() as i32, hull[2].ceil() as i32, hull[3].ceil() as i32);
    if bounds.width() > 4096 || bounds.height() > 4096 {
        return Err(VectorError::Unsupported("arch control-hull extent"));
    }
    Ok(bounds)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    path: &Path,
    fill: [f64; 4],
    bounds: Rect,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &mut dyn FnMut(u64) -> Result<(), VectorError>,
    decode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
    encode: &dyn Fn([f64; 3]) -> Result<[f64; 3], VectorError>,
) -> Result<Surface, VectorError> {
    poll(ctl)?;
    monotone_arch_descriptor(path)?;
    color(fill)?;
    if fill[3] != 1. {
        return Err(VectorError::Unsupported("arch opaque solid fill"));
    }
    let linear = decode([fill[0], fill[1], fill[2]])?;
    color([linear[0], linear[1], linear[2], 1.])?;
    let encoded = encode(linear)?;
    color([encoded[0], encoded[1], encoded[2], 1.])?;
    let ink = [(encoded[0] * 255.).round() as u8, (encoded[1] * 255.).round() as u8, (encoded[2] * 255.).round() as u8, 255];
    let width = i64::from(bounds.x1) - i64::from(bounds.x0);
    let height = i64::from(bounds.y1) - i64::from(bounds.y0);
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 || [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().any(|v| i64::from(*v).abs() > 1 << 29)
    {
        return Err(VectorError::Unsupported("arch output extent"));
    }
    let pixels = usize::try_from(width.checked_mul(height).ok_or(VectorError::Budget)?).map_err(|_| VectorError::Budget)?;
    if pixels > 1 << 20 {
        return Err(VectorError::Unsupported("arch output pixel bound"));
    }
    let bytes = pixels.checked_mul(4).ok_or(VectorError::Budget)?;
    let tiles_x = i64::from((bounds.x1 - 1).div_euclid(256)) - i64::from(bounds.x0.div_euclid(256)) + 1;
    let tiles_y = i64::from((bounds.y1 - 1).div_euclid(256)) - i64::from(bounds.y0.div_euclid(256)) + 1;
    let tiles = usize::try_from(tiles_x.checked_mul(tiles_y).ok_or(VectorError::Budget)?).map_err(|_| VectorError::Budget)?;
    charge(bytes.checked_add(tiles.checked_mul(256 * 256 * 4).ok_or(VectorError::Budget)?).ok_or(VectorError::Budget)?)?;
    let contours = tessellate_curves_bounded(path, ctl, charge, work)?;
    let contour = contours.first().ok_or(VectorError::Unsupported("arch contour"))?;
    charge(contour.points.len().checked_mul(std::mem::size_of::<i64>()).ok_or(VectorError::Budget)?)?;
    let mut crossings = Vec::new();
    crossings.try_reserve_exact(contour.points.len()).map_err(|_| VectorError::Allocation)?;
    let mut output = Vec::new();
    output.try_reserve_exact(bytes).map_err(|_| VectorError::Allocation)?;
    for y in bounds.y0..bounds.y1 {
        poll(ctl)?;
        work((contour.points.len() as u64).checked_add((width as u64).checked_mul(4).ok_or(VectorError::Budget)?).ok_or(VectorError::Budget)?)?;
        crossings.clear();
        let scan_y = i64::from(y).checked_mul(256).and_then(|n| n.checked_add(127)).ok_or(VectorError::Budget)?;
        for (index, a) in contour.points.iter().enumerate() {
            if index.is_multiple_of(64) {
                poll(ctl)?;
            }
            let next = index.checked_add(1).ok_or(VectorError::Budget)?;
            let b = contour.points.get(if next == contour.points.len() { 0 } else { next }).ok_or(VectorError::Budget)?;
            let (top, bottom) = if a.y <= b.y { (a, b) } else { (b, a) };
            let ty = (top.y * 256.) as i64;
            let by = (bottom.y * 256.) as i64;
            if (ty + 127).div_euclid(256) > i64::from(y) || i64::from(y) >= (by + 127).div_euclid(256) {
                continue;
            }
            let tx = (top.x * 256.) as i64;
            let bx = (bottom.x * 256.) as i64;
            let dy = by.checked_sub(ty).ok_or(VectorError::Budget)?;
            if dy <= 0 {
                return Err(VectorError::Unsupported("arch horizontal active edge"));
            }
            let numerator = scan_y.checked_sub(ty).and_then(|n| n.checked_mul(bx - tx)).ok_or(VectorError::Budget)?;
            let crossing = tx.checked_add(numerator.div_euclid(dy)).and_then(|n| n.checked_add(127)).ok_or(VectorError::Budget)?.div_euclid(256);
            crossings.push(crossing);
        }
        crossings.sort_unstable();
        if !crossings.len().is_multiple_of(2) {
            return Err(VectorError::Unsupported("arch unpaired active edges"));
        }
        for x in bounds.x0..bounds.x1 {
            if (i64::from(x) - i64::from(bounds.x0)) % 512 == 0 {
                poll(ctl)?;
            }
            let painted =
                crossings.as_chunks::<2>().0.iter().any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| *a <= i64::from(x) && i64::from(x) < *b));
            output.extend_from_slice(if painted { &ink } else { &[0; 4] });
        }
    }
    poll(ctl)?;
    Surface::try_from_interleaved_with(PixelFormat::RGBA8, bounds, &output, ctl)
        .map_err(|_| if ctl.cancelled() { VectorError::Cancelled } else { VectorError::Allocation })
}
