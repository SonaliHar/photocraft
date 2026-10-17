//! Atomic point/reflection preparation for the existing incremental painter.
//! Renderer generation, coverage maps and compositing are not admitted by this
//! ledger: they require controlled incremental APIs in the paint crate. This
//! seam must not be represented as cancellation-atomic live raster rendering.
use super::path_symmetry_controlled::PreparedAxis;
use crate::brush_cmds::{LiveStroke, WideStrokeBudget};
use crate::symmetry_cmds::PaintingSymmetry;
use crate::{EngineError, Result};
use photocraft_paint::{
    StrokePoint,
    stroke_controlled::{StrokeLimits, reflected_passes_with_interrupt},
    symmetry::PresetSymmetry,
};
use photocraft_raster::Interrupt;
use std::mem::size_of;

#[derive(Clone, Copy, Default)]
pub(crate) struct LiveBatchLedger {
    work: u64,
}
impl LiveBatchLedger {
    pub(crate) fn work_units(&self) -> u64 {
        self.work
    }
}

pub(crate) struct PreparedLiveBatch {
    points: Vec<StrokePoint>,
    old_count: usize,
    mirrors: Vec<Vec<StrokePoint>>,
    distinct: Vec<bool>,
    captured: CapturedSymmetry,
    wide_before: Option<WideStrokeBudget>,
    wide_after: Option<WideStrokeBudget>,
    base: (photocraft_doc::DocId, u64),
    seed: u64,
    owner: crate::document_snapshot::WeakDocumentSnapshot,
    used_work: u64,
    next_work: u64,
}
enum CapturedSymmetry {
    None,
    Preset(PresetSymmetry),
    Path(PreparedAxis),
}
fn bad(message: &str) -> EngineError {
    EngineError::Other(format!("live stroke preparation: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn bytes<T>(count: usize) -> Result<u64> {
    (count as u64).checked_mul(size_of::<T>() as u64).ok_or_else(|| bad("accounting overflow"))
}
fn plus(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| bad("accounting overflow"))
}
fn same_point(a: &StrokePoint, b: &StrokePoint) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.pressure.to_bits() == b.pressure.to_bits()
        && a.tilt_x.to_bits() == b.tilt_x.to_bits()
        && a.tilt_y.to_bits() == b.tilt_y.to_bits()
        && a.rotation.to_bits() == b.rotation.to_bits()
        && a.wheel.to_bits() == b.wheel.to_bits()
        && a.time.to_bits() == b.time.to_bits()
}
fn same_budget(left: &Option<WideStrokeBudget>, right: Option<&WideStrokeBudget>) -> bool {
    match (left.as_ref(), right) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.count == b.count
                && a.length.to_bits() == b.length.to_bits()
                && match (&a.last, &b.last) {
                    (Some(a), Some(b)) => same_point(a, b),
                    (None, None) => true,
                    _ => false,
                }
                && match (&a.bounds, &b.bounds) {
                    (Some(a), Some(b)) => a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

impl PreparedLiveBatch {
    /// Limits cover held/staged point and axis storage plus cumulative preparation
    /// work. The caller must separately admit the incremental renderer and rasters.
    pub(crate) fn prepare(live: &LiveStroke, batch: &[StrokePoint], ledger: &LiveBatchLedger, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<Self> {
        poll(control)?;
        let standard = StrokeLimits::default();
        let total = live.points.len().checked_add(batch.len()).filter(|count| *count <= 100_000).ok_or_else(|| bad("accumulated point limit"))?;
        let mirror_count = live.symmetry.as_ref().map_or(0, PaintingSymmetry::mirror_count);
        if limits.resident_bytes > standard.resident_bytes
            || limits.work_units > standard.work_units
            || mirror_count > 3
            || live.mirrors.len() != mirror_count
            || live.mirror_distinct.len() != mirror_count
        {
            return Err(bad("limits or mirror count mismatch"));
        }
        let base_bytes = plus(
            plus(bytes::<StrokePoint>(live.points.capacity())?, bytes::<StrokePoint>(total)?)?,
            plus(bytes::<bool>(live.mirror_distinct.capacity())?, bytes::<bool>(mirror_count)?)?,
        )?;
        let base_bytes = plus(base_bytes, size_of::<Self>() as u64)?;
        let base_work = (total as u64)
            .checked_mul(16)
            .and_then(|work| work.checked_add((batch.len() as u64).checked_mul(256 * (mirror_count as u64 + 1))?))
            .ok_or_else(|| bad("work overflow"))?;
        let available = limits.resident_bytes.checked_sub(base_bytes).ok_or_else(|| bad("point ownership limit"))?;
        let work_left =
            limits.work_units.checked_sub(ledger.work).and_then(|work| work.checked_sub(base_work)).ok_or_else(|| bad("cumulative preparation work limit"))?;
        for point in batch {
            poll(control)?;
            if !(point.x.abs() <= crate::brush_cmds::MAX_COORD && point.y.abs() <= crate::brush_cmds::MAX_COORD) {
                return Err(bad("invalid point coordinates"));
            }
        }
        let (captured, mirrors, reflection_work, axis_bytes) = match live.symmetry.as_ref() {
            None => (CapturedSymmetry::None, Vec::new(), 0, 0),
            Some(PaintingSymmetry::Preset(preset)) => {
                let work =
                    (batch.len() as u64).checked_mul(mirror_count as u64).and_then(|n| n.checked_mul(32)).ok_or_else(|| bad("reflection work overflow"))?;
                if work > work_left {
                    return Err(bad("reflection work limit"));
                }
                let mirrors = reflected_passes_with_interrupt(*preset, batch, StrokeLimits { resident_bytes: available, work_units: work_left }, control)
                    .map_err(|error| super::paint_error(&live.cmd, error))?;
                (CapturedSymmetry::Preset(*preset), mirrors, work, 0)
            }
            Some(PaintingSymmetry::Path(axis)) => {
                let (captured, mirrors, work) = PreparedAxis::prepare(axis, batch, StrokeLimits { resident_bytes: available, work_units: work_left }, control)?;
                let retained = captured.retained_bytes;
                (CapturedSymmetry::Path(captured), mirrors, work, retained)
            }
        };
        let mut reflected_bytes = bytes::<Vec<StrokePoint>>(mirrors.capacity())?;
        for pass in &mirrors {
            poll(control)?;
            reflected_bytes = plus(reflected_bytes, bytes::<StrokePoint>(pass.capacity())?)?;
        }
        let mut points = Vec::new();
        points.try_reserve_exact(total).map_err(|_| bad("point allocation failed"))?;
        let mut distinct = Vec::new();
        distinct.try_reserve_exact(mirror_count).map_err(|_| bad("flag allocation failed"))?;
        let actual = plus(
            plus(bytes::<StrokePoint>(live.points.capacity())?, bytes::<StrokePoint>(points.capacity())?)?,
            plus(bytes::<bool>(live.mirror_distinct.capacity())?, bytes::<bool>(distinct.capacity())?)?,
        )?;
        if plus(plus(plus(actual, axis_bytes)?, reflected_bytes)?, size_of::<Self>() as u64)? > limits.resident_bytes {
            return Err(bad("allocated capacity limit"));
        }
        for point in live.points.iter().chain(batch) {
            poll(control)?;
            points.push(*point);
        }
        for (index, pass) in mirrors.iter().enumerate() {
            poll(control)?;
            if pass.len() != batch.len() {
                return Err(bad("reflected point count mismatch"));
            }
            let mut differs = live.mirror_distinct.get(index).copied().ok_or_else(|| bad("missing mirror flag"))?;
            for (primary, reflected) in batch.iter().zip(pass) {
                poll(control)?;
                differs |= (primary.x - reflected.x).abs() > 1.0e-6 || (primary.y - reflected.y).abs() > 1.0e-6;
            }
            distinct.push(differs);
        }
        let wide_before = live.wide.as_ref().map(|wide| wide.budget.clone());
        let mut wide_after = wide_before.clone();
        if let Some(budget) = wide_after.as_mut() {
            // The old budget's equations and sequential primary/mirror order are
            // retained. Chunk boundaries add only cancellation checks; the sums
            // and last-point continuity remain identical.
            for pass in std::iter::once(batch).chain(mirrors.iter().map(Vec::as_slice)) {
                for chunk in pass.chunks(64) {
                    poll(control)?;
                    budget.extend(&live.renderer.ctx.brush, chunk)?;
                }
            }
        }
        let next_work = plus(plus(ledger.work, base_work)?, reflection_work)?;
        if next_work > limits.work_units {
            return Err(bad("cumulative preparation work limit"));
        }
        poll(control)?;
        Ok(Self {
            points,
            old_count: live.points.len(),
            mirrors,
            distinct,
            captured,
            wide_before,
            wide_after,
            base: live.base,
            seed: live.seed,
            owner: live.doc.downgrade(),
            used_work: ledger.work,
            next_work,
        })
    }

    /// Publish prepared point/axis bookkeeping only. All fallible checks finish
    /// before replacing fields. Raster generation remains the existing caller.
    pub(crate) fn install(self, live: &mut LiveStroke, used_work: u64, control: &Interrupt<'_>) -> Result<(Vec<Vec<StrokePoint>>, LiveBatchLedger)> {
        poll(control)?;
        if used_work != self.used_work
            || live.base != self.base
            || live.seed != self.seed
            || live.points.len() != self.old_count
            || !self.owner.is_current(&live.doc)
            || !same_budget(&self.wide_before, live.wide.as_ref().map(|wide| &wide.budget))
        {
            return Err(bad("live state changed before point publication"));
        }
        for (old, captured) in live.points.iter().zip(&self.points) {
            poll(control)?;
            if !same_point(old, captured) {
                return Err(bad("old points changed"));
            }
        }
        let matches = match (&self.captured, live.symmetry.as_ref()) {
            (CapturedSymmetry::None, None) => true,
            (CapturedSymmetry::Preset(old), Some(PaintingSymmetry::Preset(current))) => old == current,
            (CapturedSymmetry::Path(old), Some(PaintingSymmetry::Path(current))) => old.matches(current, control)?,
            _ => false,
        };
        if !matches || live.mirrors.len() != self.distinct.len() || live.mirror_distinct.len() != self.distinct.len() {
            return Err(bad("live axis changed"));
        }
        poll(control)?;
        live.points = self.points;
        live.mirror_distinct = self.distinct;
        if let (Some(wide), Some(budget)) = (live.wide.as_mut(), self.wide_after) {
            wide.budget = budget;
        }
        Ok((self.mirrors, LiveBatchLedger { work: self.next_work }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn staged_path_delta_preserves_native_preview_and_rejects_axis_change() {
        let mut session = crate::Session::new();
        session.execute("file.new", json!({"width":16,"height":16})).unwrap();
        let axis = crate::symmetry_cmds::SymmetryAxis { source: "Own axis".into(), segments: vec![[[8.0, 0.0], [8.0, 16.0]]] };
        session.active_mut().unwrap().symmetry = Some(PaintingSymmetry::Path(axis.clone()));
        let mut live = LiveStroke::begin(&session, &json!({"points":[[2,2,1]],"size":2,"seed":3})).unwrap();
        let doc = live.doc.clone();
        let old = live.points.clone();
        let ledger = LiveBatchLedger::default();
        let batch = [StrokePoint { x: 3.0, y: 4.0, pressure: 0.7, tilt_x: 13.0, ..Default::default() }];
        let prepared = PreparedLiveBatch::prepare(&live, &batch, &ledger, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(live.points, old);
        assert!(live.doc.shares_state(&doc));
        let (mirrors, next) = prepared.install(&mut live, ledger.work_units(), &Interrupt::NONE).unwrap();
        assert_eq!(mirrors, vec![axis.reflect_points(&batch)]);
        assert_eq!(live.points.len(), old.len() + 1);
        assert!(next.work_units() > 0);
        assert!(live.doc.shares_state(&doc));
        let prepared = PreparedLiveBatch::prepare(&live, &batch, &next, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        if let Some(PaintingSymmetry::Path(axis)) = live.symmetry.as_mut() {
            axis.source.push_str(" changed");
        }
        let before = live.points.clone();
        assert!(prepared.install(&mut live, next.work_units(), &Interrupt::NONE).is_err());
        assert_eq!(live.points, before);
        assert!(live.doc.shares_state(&doc));
    }
    #[test]
    fn point_preparation_limit_and_cancel_leave_live_bookkeeping_unchanged() {
        let mut session = crate::Session::new();
        session.execute("file.new", json!({"width":8,"height":8})).unwrap();
        let live = LiveStroke::begin(&session, &json!({"points":[[2,2,1]],"size":2,"seed":3})).unwrap();
        let ledger = LiveBatchLedger::default();
        let batch = [StrokePoint::new(3.0, 3.0, 1.0)];
        assert!(PreparedLiveBatch::prepare(&live, &batch, &ledger, StrokeLimits { resident_bytes: 1, ..Default::default() }, &Interrupt::NONE).is_err());
        let stop = || true;
        assert!(matches!(
            PreparedLiveBatch::prepare(&live, &batch, &ledger, StrokeLimits::default(), &Interrupt::cancel_only(&stop)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(live.points.len(), 1);
        assert_eq!(ledger.work_units(), 0);
    }
}
