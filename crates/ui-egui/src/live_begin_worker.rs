//! One captured live-begin mailbox. It never retains a Session or re-captures settings.
use crate::state::Tool;
use photocraft_engine::{
    Session,
    brush_cmds::{CapturedLiveBegin, LiveBeginFence, LiveStroke, PreparedLiveBegin},
    paint::StrokePoint,
};
use photocraft_raster::Interrupt;
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicUsize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, TryRecvError},
};
const MAX_POINTS: usize = 100000;
fn failed() -> String {
    "The stroke target changed. Start the stroke again.".into()
}
struct Pending {
    baseline: photocraft_engine::DocumentSnapshot,
    revision: u64,
    fence: Option<LiveBeginFence>,
    generation: u64,
    cancel: Arc<AtomicBool>,
    rx: Option<Receiver<photocraft_engine::Result<PreparedLiveBegin>>>,
    tool: Tool,
    settings: Value,
    command: &'static str,
    params: Value,
    points: Vec<StrokePoint>,
    initial: usize,
    released: bool,
    ready: bool,
    replay: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[cfg(not(target_arch = "wasm32"))]
struct Completion(Arc<AtomicUsize>);
#[cfg(not(target_arch = "wasm32"))]
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
#[derive(Default)]
pub(crate) struct Worker {
    generation: u64,
    pending: Option<Pending>,
    #[cfg(not(target_arch = "wasm32"))]
    active_threads: Arc<AtomicUsize>,
}
pub(crate) struct Commit {
    pub command: &'static str,
    pub params: Value,
    pub replay: bool,
}
impl Worker {
    pub fn tool(&self) -> Option<Tool> {
        self.pending.as_ref().map(|p| p.tool)
    }
    pub fn command(&self) -> Option<&'static str> {
        self.pending.as_ref().map(|p| p.command)
    }
    pub fn params(&self) -> Option<&Value> {
        self.pending.as_ref().map(|p| &p.params)
    }
    pub fn ready(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.ready)
    }
    pub fn active(&self) -> bool {
        self.pending.is_some()
    }
    pub fn released(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.released)
    }
    pub fn initial(&self) -> usize {
        self.pending.as_ref().map_or(0, |p| p.initial)
    }
    pub fn cancel(&mut self) {
        self.pending = None;
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn start(&mut self, s: &Session, command: &'static str, params: Value, settings: Value, tool: Tool, points: Vec<StrokePoint>) -> Result<(), String> {
        self.cancel();
        if points.is_empty() || points.len() > MAX_POINTS || points.capacity() > MAX_POINTS + 16 {
            return Err("This stroke has too many points.".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.active_threads.load(Ordering::Relaxed) >= 2 {
            return Err("Live stroke worker is busy cancelling an earlier stroke.".into());
        }
        let capture = CapturedLiveBegin::capture(s, command, &params).map_err(|e| e.to_string())?;
        let st = s.active().ok_or_else(failed)?;
        let baseline = st.doc.clone();
        let revision = st.revision;
        let initial = points.len();
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&cancel);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let active = Arc::clone(&self.active_threads);
            active
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < 2).then_some(n + 1))
                .map_err(|_| "Live stroke worker is busy cancelling an earlier stroke.".to_string())?;
            let completion = Completion(active);
            std::thread::Builder::new()
                .name("photocraft-live-begin".into())
                .spawn(move || {
                    let _completion = completion;
                    let cancel_check = || stopped.load(Ordering::Relaxed);
                    let ctl = Interrupt::cancel_only(&cancel_check);
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture.prepare(&ctl)))
                        .unwrap_or_else(|_| Err(photocraft_engine::EngineError::Other("Live stroke preparation failed.".into())));
                    let _ = tx.send(result);
                })
                .map_err(|_| "Live stroke worker is unavailable.".to_string())?;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let cancel_check = || stopped.load(Ordering::Relaxed);
            let ctl = Interrupt::cancel_only(&cancel_check);
            let result = capture.prepare(&ctl);
            let _ = tx.send(result);
        }
        self.pending = Some(Pending {
            baseline,
            revision,
            fence: None,
            generation: self.generation,
            cancel,
            rx: Some(rx),
            tool,
            settings,
            command,
            params,
            points,
            initial,
            released: false,
            ready: false,
            replay: false,
        });
        Ok(())
    }
    pub fn validate(&self, s: &Session) -> Result<(), String> {
        let p = self.pending.as_ref().ok_or_else(failed)?;
        let st = s.active().ok_or_else(failed)?;
        if st.doc.id != p.baseline.id || st.revision != p.revision || !p.baseline.downgrade().is_current(&st.doc) {
            return Err(failed());
        }
        if let Some(fence) = &p.fence {
            fence.validate(s).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn check(&self, tool: Tool, settings: &Value) -> Result<(), String> {
        let p = self.pending.as_ref().ok_or_else(failed)?;
        if p.generation != self.generation || p.cancel.load(Ordering::Relaxed) || p.tool != tool || &p.settings != settings {
            return Err(failed());
        }
        Ok(())
    }
    pub fn poll(&mut self, s: &Session) -> Option<Result<LiveStroke, String>> {
        let p = self.pending.as_mut()?;
        if p.ready {
            return None;
        }
        let rx = p.rx.as_ref()?;
        match rx.try_recv() {
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("Live stroke worker disconnected.".into())),
            Ok(result) => {
                p.rx = None;
                if p.generation != self.generation || p.cancel.load(Ordering::Relaxed) {
                    return Some(Err(failed()));
                }
                let result = result.and_then(|prepared| prepared.install_with_fence(s)).map_err(|e| e.to_string());
                match result {
                    Ok((stroke, fence)) => {
                        p.fence = Some(fence);
                        p.ready = true;
                        Some(Ok(stroke))
                    }
                    Err(e) => Some(Err(e)),
                }
            }
        }
    }
    /// Append only new ordered samples; one-point peak pressure may update a tap,
    /// requiring a captured one-shot commit rather than offering mismatched preview.
    pub fn append(&mut self, from: usize, points: &[StrokePoint]) -> Result<(), String> {
        let p = self.pending.as_mut().ok_or_else(failed)?;
        if p.released {
            return Err(failed());
        }
        if from == 0 && p.points.len() == 1 && points.len() == 1 {
            let old = *p.points.first().ok_or_else(failed)?;
            let next = *points.first().ok_or_else(failed)?;
            let mut compare = next;
            compare.pressure = old.pressure;
            if compare != old || next.pressure < old.pressure {
                return Err(failed());
            }
            if next != old {
                *p.points.first_mut().ok_or_else(failed)? = next;
                p.replay = true;
            }
            return Ok(());
        }
        if from != p.points.len() {
            return Err(failed());
        }
        let end = from.checked_add(points.len()).filter(|n| *n <= MAX_POINTS).ok_or_else(|| "This stroke has too many points.".to_string())?;
        if end > p.points.capacity() {
            let target = end.saturating_add(1024).min(MAX_POINTS);
            p.points.try_reserve_exact(target - p.points.len()).map_err(|_| "Stroke point allocation failed.".to_string())?;
            if p.points.capacity() > MAX_POINTS + 16 {
                return Err("Stroke point allocation failed.".into());
            }
        }
        p.points.extend_from_slice(points);
        Ok(())
    }
    pub fn count(&self) -> usize {
        self.pending.as_ref().map_or(0, |p| p.points.len())
    }
    pub fn release(&mut self) -> Result<(), String> {
        let p = self.pending.as_mut().ok_or_else(failed)?;
        p.released = true;
        Ok(())
    }
    pub fn tail(&self, from: usize) -> Result<&[StrokePoint], String> {
        self.pending.as_ref().and_then(|p| p.points.get(from..)).ok_or_else(failed)
    }
    pub fn commit(&mut self, seed: u64) -> Result<Commit, String> {
        let mut p = self.pending.take().ok_or_else(failed)?;
        if !p.ready || !p.released {
            return Err(failed());
        }
        let rows = points_value(&p.points)?;
        let object = p.params.as_object_mut().ok_or_else(failed)?;
        object.insert("points".into(), rows);
        object.insert("seed".into(), Value::from(seed));
        Ok(Commit { command: p.command, params: std::mem::take(&mut p.params), replay: p.replay })
    }
}

pub(crate) fn points_value(points: &[StrokePoint]) -> Result<Value, String> {
    if points.len() > MAX_POINTS {
        return Err("This stroke has too many points.".into());
    }
    let mut rows = Vec::new();
    rows.try_reserve_exact(points.len()).map_err(|_| "Stroke point allocation failed.".to_string())?;
    if rows.capacity() > MAX_POINTS + 16 {
        return Err("Stroke point allocation failed.".into());
    }
    for point in points {
        let mut row = Vec::new();
        row.try_reserve_exact(8).map_err(|_| "Stroke point allocation failed.".to_string())?;
        for v in [
            point.x,
            point.y,
            f64::from(point.pressure),
            f64::from(point.tilt_x),
            f64::from(point.tilt_y),
            f64::from(point.rotation),
            point.time,
            f64::from(point.wheel),
        ] {
            if !v.is_finite() {
                return Err("Invalid stroke sample.".into());
            }
            row.push(Value::from(v));
        }
        rows.push(Value::Array(row));
    }
    Ok(Value::Array(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn pending() -> Worker {
        let doc = photocraft_doc::Document::new("stroke", photocraft_geom::Size::new(4, 4), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let p = Pending {
            baseline: photocraft_engine::DocumentSnapshot::from_resident(Arc::new(doc)).unwrap(),
            revision: 0,
            fence: None,
            generation: 1,
            cancel: Arc::new(AtomicBool::new(false)),
            rx: None,
            tool: Tool::Brush,
            settings: json!({"erase":false}),
            command: "paint.stroke",
            params: json!({"erase":false,"points":[[1,1,0.2]]}),
            points: vec![StrokePoint::new(1., 1., 0.2)],
            initial: 1,
            released: false,
            ready: false,
            replay: false,
        };
        Worker { generation: 1, pending: Some(p), ..Default::default() }
    }
    #[test]
    fn release_before_ready_retains_order_settings_and_single_seed() {
        let mut w = pending();
        let mut next = StrokePoint::new(2., 1., 0.4);
        next.time = 4.;
        next.tilt_x = 12.;
        w.append(1, &[next]).unwrap();
        w.release().unwrap();
        assert!(w.released() && !w.ready());
        assert!(w.check(Tool::Brush, &json!({"erase":true})).is_err());
        assert!(w.append(2, &[next]).is_err());
        w.pending.as_mut().unwrap().ready = true;
        let commit = w.commit(91).unwrap();
        assert_eq!(commit.params["seed"], 91);
        assert_eq!(commit.params["erase"], false);
        let pts = photocraft_engine::brush_cmds::parse_points(&commit.params, "paint.stroke").unwrap();
        assert_eq!(pts, [StrokePoint::new(1., 1., 0.2), next]);
        assert!(!w.active());
    }
    #[test]
    fn peak_tap_replays_frozen_settings_not_new_begin() {
        let mut w = pending();
        w.append(0, &[StrokePoint::new(1., 1., 0.8)]).unwrap();
        w.release().unwrap();
        w.pending.as_mut().unwrap().ready = true;
        let commit = w.commit(72).unwrap();
        assert!(commit.replay);
        assert_eq!(commit.params["seed"], 72);
        assert_eq!(photocraft_engine::brush_cmds::parse_points(&commit.params, "paint.stroke").unwrap()[0].pressure, 0.8);
    }
    #[test]
    fn cancellation_signals_owned_generation_and_refuses_old_queue() {
        let mut w = pending();
        let ctl = Arc::clone(&w.pending.as_ref().unwrap().cancel);
        w.cancel();
        assert!(ctl.load(Ordering::Relaxed));
        assert!(w.append(1, &[StrokePoint::new(2., 1., 1.)]).is_err());
        assert!(w.commit(1).is_err());
        assert!(w.poll(&Session::new()).is_none());
    }
}
