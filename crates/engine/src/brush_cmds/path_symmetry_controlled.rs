//! Controlled snapshot and reflection of the upstream sampled path axis.
//! This uses the stored segments, not a new curve sampling or symmetry policy.
use crate::symmetry_cmds::SymmetryAxis;
use crate::{EngineError, Result};
use photocraft_paint::{StrokePoint, stroke_controlled::StrokeLimits};
use photocraft_raster::Interrupt;
use std::mem::size_of;

const MAX_SEGMENTS: usize = 1024;
const MAX_POINTS: usize = 100_000;

fn invalid(message: &str) -> EngineError {
    EngineError::Other(format!("path symmetry: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| invalid("accounting overflow"))
}
fn mul(a: usize, b: usize) -> Result<u64> {
    (a as u64).checked_mul(b as u64).ok_or_else(|| invalid("accounting overflow"))
}

pub(super) struct PreparedAxis {
    axis: SymmetryAxis,
    /// Original tool-state storage and the private captured axis; mirrors are charged separately.
    pub(super) retained_bytes: u64,
}

impl PreparedAxis {
    /// Admit capture, nearest-segment searches, deduplication and final axis comparison
    /// together before reserving any private storage. Limits are lower-only.
    pub(super) fn prepare(
        axis: &SymmetryAxis,
        points: &[StrokePoint],
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<(Self, Vec<Vec<StrokePoint>>, u64)> {
        poll(control)?;
        let standard = StrokeLimits::default();
        if limits.resident_bytes > standard.resident_bytes
            || limits.work_units > standard.work_units
            || axis.segments.is_empty()
            || axis.segments.len() > MAX_SEGMENTS
            || points.len() > MAX_POINTS
        {
            return Err(invalid("axis, point or resource limit"));
        }
        let original = add(axis.source.capacity() as u64, mul(axis.segments.capacity(), size_of::<[[f64; 2]; 2]>())?)?;
        let capture = add(axis.source.len() as u64, mul(axis.segments.len(), size_of::<[[f64; 2]; 2]>())?)?;
        let output = add(mul(points.len(), size_of::<StrokePoint>())?, size_of::<Vec<StrokePoint>>() as u64)?;
        let overhead = (size_of::<Self>() + size_of::<Vec<Vec<StrokePoint>>>()) as u64;
        let required = add(add(add(original, capture)?, output)?, overhead)?;
        // 64 elementary work units per nearest-segment visit, plus validation,
        // capture, primary/mirror comparison and the final source/axis fence.
        let visits = (points.len() as u64).checked_mul(axis.segments.len() as u64).ok_or_else(|| invalid("work overflow"))?;
        let work = visits
            .checked_mul(64)
            .and_then(|work| work.checked_add((points.len() as u64).checked_mul(16)?))
            .and_then(|work| work.checked_add((axis.segments.len() as u64).checked_mul(32)?))
            .and_then(|work| work.checked_add((axis.source.len() as u64).checked_mul(4)?))
            .ok_or_else(|| invalid("work overflow"))?;
        if required > limits.resident_bytes || work > limits.work_units {
            return Err(invalid("resource limit"));
        }
        for segment in &axis.segments {
            poll(control)?;
            if !segment.iter().flatten().all(|coordinate| coordinate.is_finite() && coordinate.abs() <= 1_000_000.0) {
                return Err(invalid("invalid stored segment"));
            }
        }
        for point in points {
            poll(control)?;
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(invalid("nonfinite point"));
            }
        }
        let mut source = String::new();
        source.try_reserve_exact(axis.source.len()).map_err(|_| invalid("source allocation failed"))?;
        let mut segments = Vec::new();
        segments.try_reserve_exact(axis.segments.len()).map_err(|_| invalid("axis allocation failed"))?;
        let mut mirrors = Vec::new();
        mirrors.try_reserve_exact(1).map_err(|_| invalid("pass allocation failed"))?;
        let mut reflected = Vec::new();
        reflected.try_reserve_exact(points.len()).map_err(|_| invalid("point allocation failed"))?;
        let captured = add(source.capacity() as u64, mul(segments.capacity(), size_of::<[[f64; 2]; 2]>())?)?;
        let retained = add(add(original, captured)?, overhead)?;
        let actual_output = add(mul(reflected.capacity(), size_of::<StrokePoint>())?, mul(mirrors.capacity(), size_of::<Vec<StrokePoint>>())?)?;
        if add(retained, actual_output)? > limits.resident_bytes {
            return Err(invalid("allocated capacity limit"));
        }
        for (index, character) in axis.source.chars().enumerate() {
            if index.is_multiple_of(256) {
                poll(control)?;
            }
            source.push(character);
        }
        for segment in &axis.segments {
            poll(control)?;
            segments.push(*segment);
        }
        let prepared = Self { axis: SymmetryAxis { source, segments }, retained_bytes: retained };
        for point in points {
            poll(control)?;
            reflected.push(prepared.reflect(*point, control)?);
        }
        mirrors.push(reflected);
        poll(control)?;
        Ok((prepared, mirrors, work))
    }

    fn reflect(&self, point: StrokePoint, control: &Interrupt<'_>) -> Result<StrokePoint> {
        let mut best = None::<(f64, [f64; 2], [f64; 2])>;
        for &[a, b] in &self.axis.segments {
            poll(control)?;
            let vx = b[0] - a[0];
            let vy = b[1] - a[1];
            let len2 = vx * vx + vy * vy;
            if len2 <= f64::EPSILON {
                continue;
            }
            let t = (((point.x - a[0]) * vx + (point.y - a[1]) * vy) / len2).clamp(0.0, 1.0);
            let q = [a[0] + vx * t, a[1] + vy * t];
            let dist2 = (point.x - q[0]).powi(2) + (point.y - q[1]).powi(2);
            if best.is_none_or(|(distance, _, _)| dist2 < distance) {
                best = Some((dist2, q, [vx, vy]));
            }
        }
        let Some((_, q, tangent)) = best else { return Ok(point) };
        let len2 = tangent[0] * tangent[0] + tangent[1] * tangent[1];
        let dx = point.x - q[0];
        let dy = point.y - q[1];
        let along = (dx * tangent[0] + dy * tangent[1]) / len2;
        let mut mirrored = point;
        mirrored.x = q[0] + 2.0 * along * tangent[0] - dx;
        mirrored.y = q[1] + 2.0 * along * tangent[1] - dy;
        if !mirrored.x.is_finite() || !mirrored.y.is_finite() {
            return Err(invalid("nonfinite reflection"));
        }
        Ok(mirrored)
    }

    pub(super) fn matches(&self, axis: &SymmetryAxis, control: &Interrupt<'_>) -> Result<bool> {
        poll(control)?;
        if self.axis.source.len() != axis.source.len() || self.axis.segments.len() != axis.segments.len() {
            return Ok(false);
        }
        for (left, right) in self.axis.source.as_bytes().chunks(1024).zip(axis.source.as_bytes().chunks(1024)) {
            poll(control)?;
            if left != right {
                return Ok(false);
            }
        }
        for (left, right) in self.axis.segments.iter().zip(&axis.segments) {
            poll(control)?;
            // Same PartialEq semantics as PaintingSymmetry (including signed zero).
            if left != right {
                return Ok(false);
            }
        }
        poll(control)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_segment_ties_endpoints_and_stylus_match_upstream() {
        let axis = SymmetryAxis { source: "Own axis".into(), segments: vec![[[0.0, 0.0], [0.0, 10.0]], [[10.0, 0.0], [10.0, 10.0]], [[2.0, 2.0], [2.0, 2.0]]] };
        let points = [
            StrokePoint { x: 5.0, y: 2.0, pressure: 0.3, tilt_x: 17.0, ..Default::default() },
            StrokePoint::new(-3.0, -4.0, 0.7),
            StrokePoint::new(0.0, 5.0, 1.0),
        ];
        let (captured, mirrors, _) = PreparedAxis::prepare(&axis, &points, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(mirrors, vec![axis.reflect_points(&points)]);
        assert!(captured.matches(&axis, &Interrupt::NONE).unwrap());
        let mut changed = axis.clone();
        changed.segments.reverse();
        assert!(!captured.matches(&changed, &Interrupt::NONE).unwrap());
        assert_eq!(mirrors[0][0].tilt_x, points[0].tilt_x);
    }
    #[test]
    fn budget_and_cancel_refuse_before_private_results_escape() {
        let axis = SymmetryAxis { source: "Own axis".into(), segments: vec![[[0.0, 0.0], [0.0, 10.0]]] };
        let points = [StrokePoint::new(1.0, 2.0, 1.0)];
        assert!(PreparedAxis::prepare(&axis, &points, StrokeLimits { resident_bytes: 1, ..Default::default() }, &Interrupt::NONE).is_err());
        let stop = || true;
        assert!(matches!(PreparedAxis::prepare(&axis, &points, StrokeLimits::default(), &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
    }
}
