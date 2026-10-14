//! Portable bounded quadratic AA primitives, derived from independent public
//! raster observations. These primitives do not authorize a text source/font.
use super::{TextSourceError, poll};
use photocraft_raster::Interrupt;

pub(crate) mod layout;
pub use layout::{PreparedOutlineText, prepare_outline_with_charge, qualify_outline_geometry};

const MAX_EDGES: usize = 4096;
const CLIP_CAPACITY: usize = MAX_EDGES + 8;
const MAX_COORD: i64 = (1_i64 << 29) * 256;
type Point = [i64; 2];
type Result<T> = std::result::Result<T, TextSourceError>;
fn invalid() -> TextSourceError {
    TextSourceError::Unsupported("unsafe quadratic coverage geometry")
}
fn point_valid(p: Point) -> bool {
    p.into_iter().all(|v| (-MAX_COORD..=MAX_COORD).contains(&v))
}

/// Allocated once per prepared raster job; the caller retains its lease until
/// destruction. Scratch is reused across pixels rather than allocating per row.
pub struct CoverageWorkspace {
    first: Vec<Point>,
    second: Vec<Point>,
}
impl CoverageWorkspace {
    pub fn new(charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<Self> {
        charge((CLIP_CAPACITY * std::mem::size_of::<Point>() * 2) as u64)?;
        let mut first = Vec::new();
        let mut second = Vec::new();
        first.try_reserve_exact(CLIP_CAPACITY).map_err(|_| TextSourceError::Allocation)?;
        if first.capacity() > CLIP_CAPACITY {
            return Err(TextSourceError::Budget);
        }
        second.try_reserve_exact(CLIP_CAPACITY).map_err(|_| TextSourceError::Allocation)?;
        if second.capacity() > CLIP_CAPACITY {
            return Err(TextSourceError::Budget);
        }
        Ok(Self { first, second })
    }
}
fn push(points: &mut Vec<Point>, p: Point) -> Result<()> {
    if points.len() >= CLIP_CAPACITY {
        return Err(TextSourceError::Budget);
    }
    points.push(p);
    Ok(())
}
fn floor_div(n: i128, d: i128) -> Result<i128> {
    if d == 0 {
        return Err(invalid());
    }
    let q = n.checked_div(d).ok_or_else(invalid)?;
    let r = n.checked_rem(d).ok_or_else(invalid)?;
    if r != 0 && (r < 0) != (d < 0) { q.checked_sub(1).ok_or_else(invalid) } else { Ok(q) }
}
fn intersection(a: Point, b: Point, axis: usize, boundary: i64) -> Result<Point> {
    let other = 1 - axis;
    let aa = *a.get(axis).ok_or_else(invalid)?;
    let ba = *b.get(axis).ok_or_else(invalid)?;
    let ao = *a.get(other).ok_or_else(invalid)?;
    let bo = *b.get(other).ok_or_else(invalid)?;
    let numerator = i128::from(bo - ao).checked_mul(i128::from(boundary - aa)).ok_or_else(invalid)?;
    let value = i128::from(ao).checked_add(floor_div(numerator, i128::from(ba - aa))?).ok_or_else(invalid)?;
    let mut point = [0, 0];
    *point.get_mut(axis).ok_or_else(invalid)? = boundary;
    *point.get_mut(other).ok_or_else(invalid)? = i64::try_from(value).map_err(|_| invalid())?;
    Ok(point)
}

/// Signed contours remain separate until area accumulation; this preserves
/// holes and measured orientation quantization. Coordinates are exact 24.8.
pub fn coverage_24_8(workspace: &mut CoverageWorkspace, contours: &[&[Point]], pixel: [i32; 2], ctl: &Interrupt<'_>) -> Result<u8> {
    coverage(workspace, contours, pixel, false, ctl)
}
fn coverage(workspace: &mut CoverageWorkspace, contours: &[&[Point]], pixel: [i32; 2], reflected: bool, ctl: &Interrupt<'_>) -> Result<u8> {
    poll(ctl)?;
    if contours.len() > 32 || pixel.into_iter().any(|v| !(-(1 << 29)..(1 << 29)).contains(&v)) {
        return Err(invalid());
    }
    let x = i64::from(pixel[0]) * 256;
    let y = i64::from(pixel[1]) * 256;
    let mut area = 0_i128;
    for contour in contours {
        if !(2..=MAX_EDGES).contains(&contour.len()) || contour.iter().any(|p| !point_valid(*p)) {
            return Err(invalid());
        }
        workspace.first.clear();
        workspace.first.extend_from_slice(contour);
        for (axis, boundary, lower) in [(0, x, true), (0, x + 256, false), (1, y, true), (1, y + 256, false)] {
            workspace.second.clear();
            let len = workspace.first.len();
            for i in 0..len {
                if i.is_multiple_of(256) {
                    poll(ctl)?;
                }
                let a = *workspace.first.get(i).ok_or_else(invalid)?;
                let b = *workspace.first.get((i + 1) % len).ok_or_else(invalid)?;
                let av = *a.get(axis).ok_or_else(invalid)?;
                let bv = *b.get(axis).ok_or_else(invalid)?;
                let inside_a = if lower { av >= boundary } else { av <= boundary };
                let inside_b = if lower { bv >= boundary } else { bv <= boundary };
                if inside_a {
                    push(&mut workspace.second, a)?;
                }
                if inside_a != inside_b {
                    push(&mut workspace.second, intersection(a, b, axis, boundary)?)?;
                }
            }
            std::mem::swap(&mut workspace.first, &mut workspace.second);
        }
        let len = workspace.first.len();
        for i in 0..len {
            if i.is_multiple_of(256) {
                poll(ctl)?;
            }
            let a = *workspace.first.get(i).ok_or_else(invalid)?;
            let b = *workspace.first.get((i + 1) % len).ok_or_else(invalid)?;
            let term = i128::from(a[0]).checked_mul(i128::from(b[1])).and_then(|v| v.checked_sub(i128::from(a[1]) * i128::from(b[0]))).ok_or_else(invalid)?;
            area = area.checked_add(term).ok_or_else(invalid)?;
        }
    }
    poll(ctl)?;
    if reflected {
        area = area.checked_neg().ok_or_else(invalid)?;
    }
    let magnitude = area.checked_abs().ok_or_else(invalid)?;
    let ceil = magnitude.checked_add(511).ok_or_else(invalid)? / 512;
    let alpha = if area > 0 {
        ceil.saturating_sub(1)
    } else if area < 0 {
        magnitude / 512
    } else {
        0
    };
    u8::try_from(alpha.min(255)).map_err(|_| invalid())
}

/// Uniform dyadic conic subdivision with floor-rounded 24.8 midpoints.
pub fn quadratic_24_8(points: [Point; 3], ctl: &Interrupt<'_>, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<Vec<Point>> {
    poll(ctl)?;
    if points.into_iter().any(|p| !point_valid(p)) {
        return Err(invalid());
    }
    let [a, b, c] = points;
    let mut delta = (a[0] - 2 * b[0] + c[0]).abs().max((a[1] - 2 * b[1] + c[1]).abs());
    let mut level = 0;
    while delta > 64 {
        delta >>= 2;
        level += 1;
    }
    if level > 12 {
        return Err(TextSourceError::Budget);
    }
    let count = (1_usize << level) + 1;
    charge((count * std::mem::size_of::<Point>()) as u64)?;
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| TextSourceError::Allocation)?;
    if result.capacity() > count {
        return Err(TextSourceError::Budget);
    }
    result.push(a);
    let mut stack = [([0, 0], [0, 0], [0, 0], 0_u32); 13];
    let mut len = 1;
    *stack.first_mut().ok_or_else(invalid)? = (a, b, c, 0);
    let midpoint = |x: Point, y: Point| [(x[0] + y[0]).div_euclid(2), (x[1] + y[1]).div_euclid(2)];
    let mut iterations = 0_usize;
    while len != 0 {
        if iterations.is_multiple_of(256) {
            poll(ctl)?;
        }
        iterations += 1;
        len -= 1;
        let (a, b, c, depth) = *stack.get(len).ok_or_else(invalid)?;
        if depth == level {
            result.push(c);
            continue;
        }
        let ab = midpoint(a, b);
        let bc = midpoint(b, c);
        let mid = midpoint(ab, bc);
        *stack.get_mut(len).ok_or(TextSourceError::Budget)? = (mid, bc, c, depth + 1);
        *stack.get_mut(len + 1).ok_or(TextSourceError::Budget)? = (a, ab, mid, depth + 1);
        len += 2;
    }
    poll(ctl)?;
    Ok(result)
}
