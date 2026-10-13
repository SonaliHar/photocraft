//! Bounded portable fixed-grid subdivision; raster coverage admission is separate.
use super::{VectorError, poll};
use photocraft_doc::{Path, PathOp};
use photocraft_geom::Point;
use photocraft_raster::Interrupt;

#[derive(Clone, Debug, PartialEq)]
pub struct CurveContour {
    pub points: Vec<Point>,
    pub closed: bool,
    pub operation: PathOp,
}

const MAX_VERTICES: usize = 1024;
const MAX_DEPTH: u8 = 16;
#[derive(Clone, Copy)]
struct FixedPoint {
    x: i64,
    y: i64,
}
impl FixedPoint {
    fn checked(p: Point) -> Result<Self, VectorError> {
        if !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 4096. || p.y.abs() > 4096. {
            return Err(VectorError::Unsupported("curve coordinate outside finite bounds"));
        }
        let x = p.x * 256.;
        let y = p.y * 256.;
        if x.fract() != 0. || y.fract() != 0. {
            return Err(VectorError::Unsupported("curve coordinate outside observed 1/256 grid"));
        }
        Ok(Self { x: x as i64, y: y as i64 })
    }
    fn midpoint(self, other: Self) -> Result<Self, VectorError> {
        Ok(Self {
            x: self.x.checked_add(other.x).ok_or(VectorError::Budget)?.div_euclid(2),
            y: self.y.checked_add(other.y).ok_or(VectorError::Budget)?.div_euclid(2),
        })
    }
    fn point(self) -> Point {
        Point::new(self.x as f64 / 256., self.y as f64 / 256.)
    }
}
fn flat_enough([a, b, c, d]: [FixedPoint; 4]) -> bool {
    let dx = (d.x - a.x) as f64;
    let dy = (d.y - a.y) as f64;
    let length_squared = dx * dx + dy * dy;
    let distance_squared = |p: FixedPoint| {
        let px = (p.x - a.x) as f64;
        let py = (p.y - a.y) as f64;
        let t = if length_squared == 0. { 0. } else { ((px * dx + py * dy) / length_squared).clamp(0., 1.) };
        (px - t * dx).powi(2) + (py - t * dy).powi(2)
    };
    // Public deviation contract 0.1 pixel, expressed in this fixed grid. The
    // convex control hull gives a conservative distance-to-chord bound.
    distance_squared(b).max(distance_squared(c)) <= 25.6f64.powi(2)
}
struct Builder<'a> {
    ctl: &'a Interrupt<'a>,
    charge: &'a mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &'a mut dyn FnMut(u64) -> Result<(), VectorError>,
    vertices: usize,
}
impl Builder<'_> {
    fn push(&mut self, points: &mut Vec<Point>, p: FixedPoint) -> Result<(), VectorError> {
        poll(self.ctl)?;
        let p = p.point();
        if points.last() == Some(&p) {
            return Ok(());
        }
        if self.vertices >= MAX_VERTICES {
            return Err(VectorError::Unsupported("curve tessellation vertex bound"));
        }
        if points.len() == points.capacity() {
            (self.charge)(32 * std::mem::size_of::<Point>())?;
            points.try_reserve_exact(32).map_err(|_| VectorError::Budget)?;
        }
        (self.work)(1)?;
        poll(self.ctl)?;
        points.push(p);
        self.vertices += 1;
        Ok(())
    }
    fn subdivide(&mut self, seg: [FixedPoint; 4], depth: u8, out: &mut Vec<Point>) -> Result<(), VectorError> {
        poll(self.ctl)?;
        (self.work)(1)?;
        poll(self.ctl)?;
        let [a, b, c, d] = seg;
        if flat_enough(seg) {
            return self.push(out, d);
        }
        if depth >= MAX_DEPTH {
            return Err(VectorError::Unsupported("curve subdivision depth bound"));
        }
        let ab = a.midpoint(b)?;
        let bc = b.midpoint(c)?;
        let cd = c.midpoint(d)?;
        let abc = ab.midpoint(bc)?;
        let bcd = bc.midpoint(cd)?;
        let m = abc.midpoint(bcd)?;
        self.subdivide([a, ab, abc, m], depth + 1, out)?;
        self.subdivide([m, bcd, cd, d], depth + 1, out)
    }
}

/// Portable subdivision for finite source control points already on the observed
/// 1/256-pixel grid, with floored fixed-grid midpoints and 0.1-pixel chord distance.
/// Limits: 16 contours, 256 knots, 1024 output vertices and depth 16. Allocation
/// callbacks conservatively charge retained capacity and bounded recursion scratch;
/// work callbacks are cumulative. Current geometry is borrowed and never mutated.
/// Matching a flattened path does not qualify fill/stroke coverage semantics.
pub fn tessellate_curves_bounded(
    path: &Path,
    ctl: &Interrupt,
    charge: &mut dyn FnMut(usize) -> Result<(), VectorError>,
    work: &mut dyn FnMut(u64) -> Result<(), VectorError>,
) -> Result<Vec<CurveContour>, VectorError> {
    poll(ctl)?;
    let knots = path.subpaths.iter().try_fold(0usize, |sum, s| sum.checked_add(s.knots.len())).ok_or(VectorError::Budget)?;
    if path.subpaths.is_empty() || path.subpaths.len() > 16 || knots > 256 || path.subpaths.iter().any(|s| s.knots.len() < 2) {
        return Err(VectorError::Unsupported("curve source structure bound"));
    }
    for k in path.subpaths.iter().flat_map(|s| &s.knots) {
        poll(ctl)?;
        work(1)?;
        for p in [k.anchor, k.in_ctrl, k.out_ctrl] {
            FixedPoint::checked(p)?;
        }
    }
    charge(path.subpaths.len() * std::mem::size_of::<CurveContour>() + (usize::from(MAX_DEPTH) + 1) * std::mem::size_of::<[FixedPoint; 4]>())?;
    let mut result = Vec::new();
    result.try_reserve_exact(path.subpaths.len()).map_err(|_| VectorError::Budget)?;
    let mut builder = Builder { ctl, charge, work, vertices: 0 };
    for source in &path.subpaths {
        poll(ctl)?;
        let mut points = Vec::new();
        let first = source.knots.first().ok_or(VectorError::Unsupported("empty curve source"))?;
        builder.push(&mut points, FixedPoint::checked(first.anchor)?)?;
        for pair in source.knots.windows(2) {
            let a = pair.first().ok_or(VectorError::Unsupported("curve endpoint missing"))?;
            let b = pair.get(1).ok_or(VectorError::Unsupported("curve endpoint missing"))?;
            builder.subdivide(
                [FixedPoint::checked(a.anchor)?, FixedPoint::checked(a.out_ctrl)?, FixedPoint::checked(b.in_ctrl)?, FixedPoint::checked(b.anchor)?],
                0,
                &mut points,
            )?;
        }
        if source.closed {
            let last = source.knots.last().ok_or(VectorError::Unsupported("empty curve source"))?;
            builder.subdivide(
                [
                    FixedPoint::checked(last.anchor)?,
                    FixedPoint::checked(last.out_ctrl)?,
                    FixedPoint::checked(first.in_ctrl)?,
                    FixedPoint::checked(first.anchor)?,
                ],
                0,
                &mut points,
            )?;
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
        }
        result.push(CurveContour { points, closed: source.closed, operation: source.op });
    }
    poll(ctl)?;
    Ok(result)
}
