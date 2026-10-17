//! Controlled incremental generator deltas, preserving upstream state and math.
//! Preflight advances copied scalar walkers only; prior stroke history is never
//! replayed. Generator and append buffers publish together after a final poll.
use super::*;
use crate::render::incremental_clone_controlled::{CloneBudget, vector_requirement};
use crate::render::{IncrementalCloneRequirements, StrokeError, StrokeLimits};
use photocraft_raster::Interrupt;
use std::mem::size_of;
type Result<T> = std::result::Result<T, StrokeError>;
const MAX_POINTS: usize = 100_000;
const MAX_STEPS: usize = 1_000_000;
const MAX_DABS: usize = 1_000_000;
#[derive(Clone, Copy, Debug)]
pub struct IncrementalGeneratorRequirements {
    pub retained_bytes: u64,
    pub staged_bytes: u64,
    pub work_units: u64,
    pub smoothed_points: usize,
    pub primary_dabs: usize,
    pub dual_dabs: usize,
}
#[derive(Default)]
struct Count {
    smoothed: usize,
    primary: usize,
    dual: usize,
    work: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| StrokeError::Cancelled)
}
fn limits(limits: StrokeLimits) -> Result<()> {
    let standard = StrokeLimits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units { Err(StrokeError::Limit) } else { Ok(()) }
}
fn cost(count: usize, width: usize) -> Result<u64> {
    (count as u64).checked_mul(width as u64).ok_or(StrokeError::Limit)
}
fn valid_point(point: &StrokePoint) -> Result<()> {
    if !point.x.is_finite() || !point.y.is_finite() || point.x.abs() > 1_000_000.0 || point.y.abs() > 1_000_000.0 {
        return Err(StrokeError::InvalidInput("incremental point coordinates"));
    }
    Ok(())
}
impl PathWalker {
    fn controlled_emit(&mut self, point: StrokePoint, out: &mut impl FnMut(StepInput) -> Result<()>, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        let step = StepInput { point, direction: self.dir, initial_direction: self.initial_dir.unwrap_or(self.dir), step: self.step };
        self.step = self.step.checked_add(1).ok_or(StrokeError::Limit)?;
        out(step)
    }
    fn controlled_grid_steps(
        &mut self,
        a: &StrokePoint,
        p: &StrokePoint,
        offset: f64,
        out: &mut impl FnMut(StepInput) -> Result<()>,
        control: &Interrupt<'_>,
    ) -> Result<bool> {
        let cell = |v: f64| if offset > 0.0 { v.floor() } else { v.round() };
        let (x0, y0) = (cell(a.x), cell(a.y));
        let (dx, dy) = (cell(p.x) - x0, cell(p.y) - y0);
        let n = dx.abs().max(dy.abs());
        if !n.is_finite() || n > MAX_GRID_STEPS {
            return Ok(false);
        }
        let steps = n as u64;
        for i in 1..=steps {
            poll(control)?;
            let i = i as f64;
            let mut q = lerp_pt(a, p, i / n);
            q.x = x0 + (dx * i / n).round() + offset;
            q.y = y0 + (dy * i / n).round() + offset;
            self.controlled_emit(q, out, control)?;
        }
        Ok(true)
    }
    fn controlled_push(
        &mut self,
        p: StrokePoint,
        step_len: &impl Fn(&StrokePoint) -> f64,
        out: &mut impl FnMut(StepInput) -> Result<()>,
        control: &Interrupt<'_>,
    ) -> Result<()> {
        poll(control)?;
        valid_point(&p)?;
        if self.interval.is_some_and(|v| !v.is_finite() || v <= 0.0)
            || self.speed.is_some_and(|v| !v.is_finite() || v <= 0.0)
            || self.grid.is_some_and(|v| !v.is_finite())
        {
            return Err(StrokeError::InvalidInput("incremental walker policy"));
        }
        let Some(a) = self.last else {
            self.last = Some(p);
            self.pending_first = Some(p);
            self.next_at = step_len(&p);
            return Ok(());
        };
        valid_point(&a)?;
        let (dx, dy) = (p.x - a.x, p.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-9 {
            self.dir = (-dy).atan2(dx) as f32;
            if self.initial_dir.is_none() {
                self.initial_dir = Some(self.dir);
            }
        }
        if let Some(first) = self.pending_first.take() {
            self.controlled_emit(first, out, control)?;
        }
        if len > 1e-9 {
            let dt = p.time - a.time;
            match self.speed.filter(|_| dt > 0.0 && dt.is_finite()) {
                None => {
                    let gridded = if let Some(g) = self.grid { self.controlled_grid_steps(&a, &p, g, out, control)? } else { false };
                    if !gridded {
                        if !self.next_at.is_finite() {
                            return Err(StrokeError::InvalidInput("incremental spacing state"));
                        }
                        while self.next_at <= len + 1e-9 {
                            poll(control)?;
                            let q = lerp_pt(&a, &p, (self.next_at / len).min(1.0));
                            self.controlled_emit(q, out, control)?;
                            let increment = step_len(&q).max(0.25);
                            if !increment.is_finite() {
                                return Err(StrokeError::InvalidInput("incremental spacing"));
                            }
                            self.next_at += increment;
                        }
                        self.next_at -= len;
                    }
                }
                Some(iv) => {
                    self.speed_acc += dt;
                    let max_n = (len / MIN_SPEED_STEP).ceil().max(1.0) as usize;
                    let mut n = 0;
                    while self.speed_acc >= iv && n < max_n {
                        poll(control)?;
                        self.speed_acc -= iv;
                        let f = ((dt - self.speed_acc) / dt).clamp(0.0, 1.0);
                        self.controlled_emit(lerp_pt(&a, &p, f), out, control)?;
                        n += 1;
                    }
                    if n == max_n {
                        self.speed_acc = self.speed_acc.rem_euclid(iv);
                    }
                }
            }
        }
        if let Some(iv) = self.interval {
            let dt = p.time - a.time;
            if dt > 0.0 && dt.is_finite() {
                if dt > iv * MAX_AIRBRUSH_DABS_PER_SEGMENT as f64 {
                    self.time_acc = 0.0;
                    self.controlled_emit(p, out, control)?;
                    self.last = Some(p);
                    return Ok(());
                }
                self.time_acc += dt;
                let mut n = 0;
                while self.time_acc >= iv && n < MAX_AIRBRUSH_DABS_PER_SEGMENT {
                    poll(control)?;
                    self.time_acc -= iv;
                    let f = ((dt - self.time_acc) / dt).clamp(0.0, 1.0);
                    self.controlled_emit(lerp_pt(&a, &p, f), out, control)?;
                    n += 1;
                }
            }
        }
        self.last = Some(p);
        poll(control)
    }
    fn controlled_finish(&mut self, out: &mut impl FnMut(StepInput) -> Result<()>, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        if let Some(first) = self.pending_first.take() {
            self.controlled_emit(first, out, control)?;
        }
        Ok(())
    }
}
impl Count {
    fn step(&mut self, dual: bool, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        let count = if dual { &mut self.dual } else { &mut self.primary };
        *count = count.checked_add(1).filter(|n| *n <= MAX_STEPS).ok_or(StrokeError::Limit)?;
        self.work = self.work.checked_add(128).filter(|n| *n <= limits.work_units).ok_or(StrokeError::Limit)?;
        Ok(())
    }
}
impl DabGenerator {
    fn count_delta(&self, points: &[StrokePoint], finish: bool, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<Count> {
        poll(control)?;
        let mut smoother = self.smoother.clone();
        let mut walker = self.walker.clone();
        let mut dual_walker = self.dual_walker.clone();
        // Each Smoother call emits at most64 points. This fixed scratch is paid
        // before reservation; its equations are the original bounded helper.
        let scratch_bytes = cost(64, size_of::<StrokePoint>())?;
        if scratch_bytes > limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        let mut smoothed = Vec::new();
        smoothed.try_reserve_exact(64).map_err(|_| StrokeError::Allocation)?;
        if cost(smoothed.capacity(), size_of::<StrokePoint>())? > limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        let mut count = Count::default();
        let mut walk = |positions: &[StrokePoint]| -> Result<()> {
            count.smoothed = count.smoothed.checked_add(positions.len()).ok_or(StrokeError::Limit)?;
            for point in positions {
                poll(control)?;
                let p = apply_pose(&self.builder.brush, *point);
                valid_point(&p)?;
                walker.controlled_push(p, &|q| self.builder.step_len(q), &mut |_| count.step(false, limits, control), control)?;
                if let (Some(w), Some(builder)) = (dual_walker.as_mut(), self.dual.as_ref()) {
                    w.controlled_push(p, &|q| builder.step_len(q), &mut |_| count.step(true, limits, control), control)?;
                }
            }
            Ok(())
        };
        if finish {
            smoothed.clear();
            smoother.finish(&mut smoothed);
            walk(&smoothed)?;
        } else {
            for point in points {
                poll(control)?;
                valid_point(point)?;
                smoothed.clear();
                smoother.push(*point, &mut smoothed);
                walk(&smoothed)?;
            }
        }
        drop(walk);
        if finish {
            walker.controlled_finish(&mut |_| count.step(false, limits, control), control)?;
            if let Some(w) = dual_walker.as_mut() {
                w.controlled_finish(&mut |_| count.step(true, limits, control), control)?;
            }
        }
        poll(control)?;
        Ok(count)
    }
    fn delta_requirements(
        &self,
        points: &[StrokePoint],
        primary: &Vec<Dab>,
        dual: &Vec<Dab>,
        finish: bool,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<IncrementalGeneratorRequirements> {
        poll(control)?;
        self::limits(limits)?;
        if points.len() > MAX_POINTS {
            return Err(StrokeError::Limit);
        }
        let mut req = self.incremental_clone_requirements(limits, control)?;
        req.add(vector_requirement(primary)?, limits)?;
        req.add(vector_requirement(dual)?, limits)?;
        let scratch = cost(64, size_of::<StrokePoint>())?;
        req.add(IncrementalCloneRequirements { retained_bytes: 0, staged_bytes: scratch, work_units: 64 }, limits)?;
        let other_staged = req.staged_bytes.checked_sub(scratch).ok_or(StrokeError::Limit)?;
        let available = limits.resident_bytes.checked_sub(req.retained_bytes).and_then(|v| v.checked_sub(other_staged)).ok_or(StrokeError::Limit)?;
        // Count scratch itself is already included above; the remaining limit
        // keeps a fixed temporary count pass from exceeding the aggregate peak.
        if available < scratch {
            return Err(StrokeError::Limit);
        }
        let counted = self.count_delta(
            points,
            finish,
            StrokeLimits { resident_bytes: available, work_units: limits.work_units.checked_sub(req.work_units).ok_or(StrokeError::Limit)? },
            control,
        )?;
        let copies = if self.builder.brush.scattering.enabled { self.builder.brush.scattering.count.clamp(1, 16) as usize } else { 1 };
        let primary_dabs = counted.primary.checked_mul(copies).ok_or(StrokeError::Limit)?;
        let dual_dabs = counted.dual.checked_mul(self.dual.as_ref().map_or(0, |b| b.brush.dual_brush.count.clamp(1, 16) as usize)).ok_or(StrokeError::Limit)?;
        if primary.len().checked_add(primary_dabs).is_none_or(|n| n > MAX_DABS) || dual.len().checked_add(dual_dabs).is_none_or(|n| n > MAX_DABS) {
            return Err(StrokeError::Limit);
        }
        self.builder.next_index.checked_add(primary_dabs as u64).ok_or(StrokeError::Limit)?;
        if let Some(builder) = &self.dual {
            builder.next_index.checked_add(dual_dabs as u64).ok_or(StrokeError::Limit)?;
        }
        let smoothed = cost(counted.smoothed, size_of::<StrokePoint>())?;
        let output = cost(primary_dabs.checked_add(dual_dabs).ok_or(StrokeError::Limit)?, size_of::<Dab>())?;
        let work = counted
            .work
            .checked_mul(2)
            .and_then(|v| v.checked_add((primary_dabs as u64).checked_add(dual_dabs as u64)?.checked_mul(512)?))
            .and_then(|v| v.checked_add((points.len() as u64).checked_mul(128)?))
            .ok_or(StrokeError::Limit)?;
        req.add(
            IncrementalCloneRequirements { retained_bytes: 0, staged_bytes: smoothed.checked_add(output).ok_or(StrokeError::Limit)?, work_units: work },
            limits,
        )?;
        Ok(IncrementalGeneratorRequirements {
            retained_bytes: req.retained_bytes,
            staged_bytes: req.staged_bytes,
            work_units: req.work_units,
            smoothed_points: counted.smoothed,
            primary_dabs,
            dual_dabs,
        })
    }
    pub fn push_requirements(
        &self,
        points: &[StrokePoint],
        primary: &Vec<Dab>,
        dual: &Vec<Dab>,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<IncrementalGeneratorRequirements> {
        self.delta_requirements(points, primary, dual, false, limits, control)
    }
    pub fn finish_requirements(
        &self,
        primary: &Vec<Dab>,
        dual: &Vec<Dab>,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<IncrementalGeneratorRequirements> {
        self.delta_requirements(&[], primary, dual, true, limits, control)
    }
    fn controlled_delta(
        &mut self,
        points: &[StrokePoint],
        primary: &mut Vec<Dab>,
        dual: &mut Vec<Dab>,
        finish: bool,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<()> {
        let req = self.delta_requirements(points, primary, dual, finish, limits, control)?;
        let clone = IncrementalCloneRequirements { retained_bytes: req.retained_bytes, staged_bytes: req.staged_bytes, work_units: req.work_units };
        let mut budget = CloneBudget::new(clone, limits, control)?;
        let mut next = self.clone_with_budget(&mut budget)?;
        // Reserve complete append buffers once; all existing raw dabs survive.
        let allocate = |source: &Vec<Dab>, extra: usize, budget: &mut CloneBudget<'_, '_>| -> Result<Vec<Dab>> {
            let total = source.len().checked_add(extra).ok_or(StrokeError::Limit)?;
            budget.admit(cost(total, size_of::<Dab>())?)?;
            let mut out = Vec::new();
            out.try_reserve_exact(total).map_err(|_| StrokeError::Allocation)?;
            budget.charge(cost(out.capacity(), size_of::<Dab>())?)?;
            for chunk in source.chunks(256) {
                budget.poll()?;
                out.extend_from_slice(chunk);
            }
            Ok(out)
        };
        let mut dabs = allocate(primary, req.primary_dabs, &mut budget)?;
        let mut duals = allocate(dual, req.dual_dabs, &mut budget)?;
        budget.admit(cost(req.smoothed_points, size_of::<StrokePoint>())?)?;
        let mut smoothed = Vec::new();
        smoothed.try_reserve_exact(req.smoothed_points).map_err(|_| StrokeError::Allocation)?;
        budget.charge(cost(smoothed.capacity(), size_of::<StrokePoint>())?)?;
        if finish {
            next.smoother.finish(&mut smoothed);
        } else {
            for point in points {
                poll(control)?;
                next.smoother.push(*point, &mut smoothed);
            }
        }
        if smoothed.len() != req.smoothed_points {
            return Err(StrokeError::InvalidInput("smoothing preflight changed"));
        }
        for point in &smoothed {
            poll(control)?;
            let p = apply_pose(&next.builder.brush, *point);
            // Walker step length depends only on the unchanged brush, not the
            // builder's jitter index. Streaming its steps directly to the same
            // builder keeps the exact dab order and avoids a temporary step Vec.
            let builder = &mut next.builder;
            next.walker.controlled_push(
                p,
                &|q| self.builder.step_len(q),
                &mut |step| {
                    poll(control)?;
                    if dabs.capacity().saturating_sub(dabs.len())
                        < if builder.brush.scattering.enabled { builder.brush.scattering.count.clamp(1, 16) as usize } else { 1 }
                    {
                        return Err(StrokeError::Limit);
                    }
                    budget.work(512)?;
                    builder.build(&step, &mut dabs);
                    Ok(())
                },
                control,
            )?;
            if let (Some(w), Some(builder), Some(original)) = (next.dual_walker.as_mut(), next.dual.as_mut(), self.dual.as_ref()) {
                w.controlled_push(
                    p,
                    &|q| original.step_len(q),
                    &mut |step| {
                        poll(control)?;
                        if duals.capacity().saturating_sub(duals.len()) < builder.brush.dual_brush.count.clamp(1, 16) as usize {
                            return Err(StrokeError::Limit);
                        }
                        budget.work(512)?;
                        builder.build(&step, &mut duals);
                        Ok(())
                    },
                    control,
                )?;
            }
        }
        if finish {
            let mut pending = None;
            next.walker.controlled_finish(
                &mut |step| {
                    pending = Some(step);
                    Ok(())
                },
                control,
            )?;
            if let Some(step) = pending {
                poll(control)?;
                next.builder.build(&step, &mut dabs);
            }
            if let (Some(w), Some(builder)) = (next.dual_walker.as_mut(), next.dual.as_mut()) {
                let mut pending = None;
                w.controlled_finish(
                    &mut |step| {
                        pending = Some(step);
                        Ok(())
                    },
                    control,
                )?;
                if let Some(step) = pending {
                    poll(control)?;
                    builder.build(&step, &mut duals);
                }
            }
        } else {
            next.scratch = smoothed;
        }
        if dabs.len() > primary.len() + req.primary_dabs || duals.len() > dual.len() + req.dual_dabs {
            return Err(StrokeError::Limit);
        }
        poll(control)?;
        *self = next;
        *primary = dabs;
        *dual = duals;
        Ok(())
    }
    pub fn push_with_interrupt(
        &mut self,
        points: &[StrokePoint],
        primary: &mut Vec<Dab>,
        dual: &mut Vec<Dab>,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<()> {
        self.controlled_delta(points, primary, dual, false, limits, control)
    }
    pub fn finish_with_interrupt(&mut self, primary: &mut Vec<Dab>, dual: &mut Vec<Dab>, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<()> {
        self.controlled_delta(&[], primary, dual, true, limits, control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incremental_generator_keeps_spacing_time_pixelgrid_and_finish_dabs_exact() {
        let points = [
            StrokePoint { x: 2.0, y: 3.0, time: 0.0, ..Default::default() },
            StrokePoint { x: 5.0, y: 4.0, time: 8.0, ..Default::default() },
            StrokePoint { x: 9.0, y: 7.0, time: 48.0, ..Default::default() },
            StrokePoint { x: 9.0, y: 7.0, time: 96.0, ..Default::default() },
        ];
        for mode in 0..4 {
            let brush = BrushSettings {
                size: 2.0,
                pressure_size: false,
                spacing: 0.25,
                aliased: mode == 1,
                spacing_enabled: mode != 2,
                build_up: mode == 3,
                smoothing: Smoothing { enabled: mode == 3, amount: 0.5, catch_up: true, catch_up_on_end: true, ..Default::default() },
                ..Default::default()
            };
            let mut old = DabGenerator::new(&brush, 1.0);
            let mut checked = DabGenerator::new(&brush, 1.0);
            let (mut a, mut ad, mut b, mut bd) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for chunk in points.chunks(2) {
                old.push(chunk, &mut a, &mut ad);
                checked.push_with_interrupt(chunk, &mut b, &mut bd, StrokeLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(a, b);
                assert_eq!(ad, bd);
            }
            old.finish(&mut a, &mut ad);
            checked.finish_with_interrupt(&mut b, &mut bd, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(a, b);
            assert_eq!(ad, bd);
            assert_eq!(old.smoothing_string(), checked.smoothing_string());
        }
    }
    #[test]
    fn finish_reported_peak_is_sufficient_and_one_less_refuses_atomically() {
        let brush = BrushSettings { size: 2.0, pressure_size: false, ..Default::default() };
        let mut generator = DabGenerator::new(&brush, 1.0);
        let (mut primary, mut dual) = (Vec::new(), Vec::new());
        generator.push(&[StrokePoint::new(2.0, 3.0, 1.0)], &mut primary, &mut dual);
        let req = generator.finish_requirements(&primary, &dual, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let peak = req.retained_bytes.checked_add(req.staged_bytes).unwrap();
        let before_primary = primary.clone();
        let before_dual = dual.clone();
        let before = generator.smoothing_string();
        assert!(matches!(
            generator.finish_with_interrupt(&mut primary, &mut dual, StrokeLimits { resident_bytes: peak - 1, work_units: req.work_units }, &Interrupt::NONE),
            Err(StrokeError::Limit)
        ));
        assert_eq!(primary, before_primary);
        assert_eq!(dual, before_dual);
        assert_eq!(generator.smoothing_string(), before);
        let mut expected = generator.clone();
        let (mut expected_primary, mut expected_dual) = (primary.clone(), dual.clone());
        expected.finish(&mut expected_primary, &mut expected_dual);
        generator.finish_with_interrupt(&mut primary, &mut dual, StrokeLimits { resident_bytes: peak, work_units: req.work_units }, &Interrupt::NONE).unwrap();
        assert_eq!(primary, expected_primary);
        assert_eq!(dual, expected_dual);
    }
    #[test]
    fn late_refusal_leaves_generator_and_existing_append_buffers_unchanged() {
        let brush = BrushSettings { size: 2.0, ..Default::default() };
        let mut generator = DabGenerator::new(&brush, 1.0);
        let (mut primary, mut dual) = (Vec::new(), Vec::new());
        generator.push(&[StrokePoint::new(2.0, 2.0, 1.0)], &mut primary, &mut dual);
        let mut expected = generator.clone();
        let old_primary = primary.clone();
        let old_dual = dual.clone();
        let count = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            let n = count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            n > 20
        };
        let delta = [StrokePoint::new(20.0, 20.0, 1.0)];
        assert!(generator.push_with_interrupt(&delta, &mut primary, &mut dual, StrokeLimits::default(), &Interrupt::cancel_only(&stop)).is_err());
        assert_eq!(primary, old_primary);
        assert_eq!(dual, old_dual);
        let (mut ep, mut ed) = (old_primary, old_dual);
        expected.push(&delta, &mut ep, &mut ed);
        generator.push_with_interrupt(&delta, &mut primary, &mut dual, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(primary, ep);
        assert_eq!(dual, ed);
    }
}
