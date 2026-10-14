//! Read-only Object Finder preparation and fenced publication of derived cache entries.
//! Uses the existing 60k point algorithm; no selection edit or source qualification.
use crate::channel_cmds::ChannelView;
use crate::selection_object_controlled::{BoundedSampler, copy_view, find_surface};
use crate::smartselect_cmds::{ObjectFinderCache, ObjectFinderSnapshot};
use crate::{DocumentSnapshot, EngineError, Result, Session};
pub use photocraft_algo::segment::grabcut::ObjectSelectionLimits as FinderLimits;
use photocraft_algo::segment::{Region, grabcut};
use photocraft_color::convert::CmykSpace;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use std::sync::Arc;
const MAX_WORK: u64 = 8_000_000_000;
fn bad(message: &str) -> EngineError {
    EngineError::Other(format!("Object Finder: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn map_error(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(&error.to_string()) }
}
/// Cheap captured owner/context; large payloads remain shared and are admitted in the worker.
#[derive(Clone)]
pub struct FinderTarget {
    document: DocumentSnapshot,
    revision: u64,
    active: Option<LayerId>,
    all: bool,
    point: [f32; 2],
    view: Arc<ChannelView>,
    color: Option<Arc<CmykSpace>>,
    cache: ObjectFinderSnapshot,
    limits: FinderLimits,
}
pub struct PreparedFinder {
    target: FinderTarget,
    region: Option<Arc<Region>>,
    cache: Option<ObjectFinderCache>,
}
impl PreparedFinder {
    pub fn region(&self) -> Option<&Arc<Region>> {
        self.region.as_ref()
    }
}
fn same_color(a: &Option<Arc<CmykSpace>>, b: &Option<Arc<CmykSpace>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
fn idle(session: &Session) -> bool {
    session.documents().len() <= 1024
        && !session.has_jobs()
        && !session.command_has_pending_job()
        && session.coalesce_request.is_none()
        && session.color_restrict.is_none()
        && session.documents().iter().all(|state| state.floating.is_none() && state.doc.xcf_state.as_ref().is_none_or(|x| x.floating.is_none()))
        && session.active().is_some_and(|state| state.coalesce.is_none() && state.doc.accounted().is_none())
}
fn point_valid(point: [f32; 2]) -> bool {
    point.iter().all(|v| v.is_finite() && f64::from(*v) >= f64::from(i32::MIN) && f64::from(*v) < f64::from(i32::MAX))
}
pub fn capture(session: &Session, all_layers: bool, point: [f32; 2], limits: FinderLimits, control: &Interrupt<'_>) -> Result<FinderTarget> {
    poll(control)?;
    if !idle(session) {
        return Err(bad("idle resident context required"));
    }
    if !point_valid(point) {
        return Err(bad("finite in-range point required"));
    }
    if limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units == 0 || limits.work_units > MAX_WORK {
        return Err(bad("lower-only Finder limits required"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.xcf_state.is_some() {
        return Err(bad("canonical imported sampling is not qualified"));
    }
    crate::selection_sample_policy::sampled_pixels(session, "Object Finder", all_layers, control)?;
    let target = FinderTarget {
        document: state.doc.clone(),
        revision: state.revision,
        active: state.active_layer,
        all: all_layers,
        point,
        view: Arc::new(copy_view(&state.channel_view, "Object Finder")?),
        color: photocraft_color::convert::active_cmyk_space(),
        cache: session.object_finder.capture_controlled()?,
        limits,
    };
    poll(control)?;
    Ok(target)
}
/// Context only; publishing our own cache does not invalidate the displayed result.
pub fn is_current(session: &Session, target: &FinderTarget) -> bool {
    idle(session)
        && session.active().is_some_and(|state| {
            state.doc.id == target.document.id
                && state.revision == target.revision
                && target.document.downgrade().is_current(&state.doc)
                && state.active_layer == target.active
                && state.channel_view == *target.view
        })
        && same_color(&target.color, &photocraft_color::convert::active_cmyk_space())
}
/// Fixed twelve-entry pointer check, suitable for detecting stationary external cache changes.
pub fn cache_is_current(session: &Session, target: &FinderTarget) -> bool {
    target.cache.is_current_cache(&session.object_finder)
}
fn cache_lookup(
    target: &FinderTarget,
    cache: &ObjectFinderSnapshot,
    point: [f32; 2],
    control: &Interrupt<'_>,
) -> Result<(Option<Arc<Region>>, bool, u64, u64)> {
    poll(control)?;
    let bytes = cache.retained_with_interrupt(control)?;
    if bytes > target.limits.resident_bytes {
        return Err(bad("retained cache exceeds resident limit"));
    }
    let direct = if target.all { None } else { find_surface(&target.document, target.active, control)? };
    let sampled = if direct.is_some() { target.active } else { None };
    let mut work = 0u64;
    let mut charge = |n: u64| {
        poll(control)?;
        work = work.checked_add(n).filter(|n| *n <= target.limits.work_units).ok_or_else(|| bad("cache comparison work limit"))?;
        Ok(())
    };
    let (region, valid) = cache.lookup_controlled(&target.document, sampled, point[0], point[1], control, &mut charge)?;
    Ok((region, valid, bytes, work))
}
// Admission covers retained current and previous cache documents, lower private
// point stages and known ordinary compositor preparation. Provider/style internals
// retain their inherited boundaries; this is not a total allocator/RSS guarantee.
fn admission(target: &FinderTarget, cache_bytes: u64, cache_work: u64, hit: bool, control: &Interrupt<'_>) -> Result<FinderLimits> {
    poll(control)?;
    let doc = &target.document;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300_000 || doc.size.height > 300_000 {
        return Err(bad("canvas bounds"));
    }
    let req = if hit {
        grabcut::ObjectSelectionRequirements { window: doc.bounds(), step: 1, peak_resident_bytes: 0, work_units: 1 }
    } else {
        grabcut::point_requirements(doc.bounds(), target.point[0], target.point[1], grabcut::OBJECT_AT_WORK_PX).map_err(|e| map_error(control, e))?
    };
    let prep = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, prep, control).map_err(|e| map_error(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, prep, control).map_err(|e| map_error(control, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let direct = if target.all { None } else { find_surface(doc, target.active, control)? };
    let (mut render_bytes, mut render_work) = (0u64, 0u64);
    if !hit && direct.is_none() && !req.window.is_empty() {
        let strip_h =
            if req.step == 1 { req.window.height() as usize } else { (64usize.div_ceil(req.step).max(1) * req.step).min(req.window.height() as usize) }.max(1);
        let strip = Rect::from_xywh(req.window.x0, req.window.y0, req.window.width().max(1), strip_h as u32);
        let halo = Rect::from_xywh(req.window.x0, req.window.y0, req.window.width().min(216).max(1), req.window.height().min(216).max(1));
        let a = photocraft_compose::ordinary_render_requirements(doc, strip, control).map_err(|e| map_error(control, e))?;
        let b = photocraft_compose::ordinary_render_requirements(doc, halo, control).map_err(|e| map_error(control, e))?;
        render_bytes = (a.scratch_bytes as u64)
            .checked_add(a.preparation_reserve_bytes as u64)
            .and_then(|n| Some(n.max((b.scratch_bytes as u64).checked_add(b.preparation_reserve_bytes as u64)?)))
            .ok_or_else(|| bad("render resident overflow"))?;
        let strips = u64::from(req.window.height()).div_ceil(strip_h as u64);
        let tiles = u64::from(req.window.width())
            .div_ceil(192)
            .checked_mul(u64::from(req.window.height()).div_ceil(192))
            .ok_or_else(|| bad("render visits overflow"))?;
        render_work =
            a.work_units.checked_mul(strips).and_then(|n| n.checked_add(b.work_units.checked_mul(tiles)?)).ok_or_else(|| bad("render work overflow"))?;
    }
    let view_bytes =
        (target.view.color_hidden.capacity() as u64).checked_add(target.view.alpha_visible.capacity() as u64).ok_or_else(|| bad("view capacity overflow"))?;
    raster
        .retained_bytes
        .checked_add(raster.clone_metadata_bytes)
        .and_then(|n| n.checked_add(model))
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(cache_bytes))
        .and_then(|n| n.checked_add(req.peak_resident_bytes))
        .and_then(|n| n.checked_add(render_bytes))
        .and_then(|n| n.checked_add(view_bytes))
        .and_then(|n| n.checked_add(4096))
        .filter(|n| *n <= target.limits.resident_bytes)
        .ok_or_else(|| bad("aggregate resident limit"))?;
    let overhead = cache_work
        .checked_add(render_work)
        .and_then(|n| n.checked_add(cache_bytes.checked_mul(8)?))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(view_bytes))
        .and_then(|n| n.checked_add(4096))
        .ok_or_else(|| bad("aggregate work overflow"))?;
    let remaining = target.limits.work_units.checked_sub(overhead).filter(|n| *n >= req.work_units).ok_or_else(|| bad("aggregate work limit"))?;
    poll(control)?;
    Ok(FinderLimits { resident_bytes: req.peak_resident_bytes, work_units: remaining })
}
pub fn prepare(target: FinderTarget, control: &Interrupt<'_>) -> Result<PreparedFinder> {
    photocraft_color::convert::with_cmyk_space(target.color.as_ref(), || {
        let (cached, valid, bytes, work) = cache_lookup(&target, &target.cache, target.point, control)?;
        let hit = cached.is_some();
        let limits = admission(&target, bytes, work, hit, control)?;
        let direct = if target.all { None } else { find_surface(&target.document, target.active, control)? };
        let sampled = if direct.is_some() { target.active } else { None };
        let sampler = BoundedSampler { doc: &target.document, direct, control };
        let region = if hit {
            cached
        } else {
            grabcut::object_at_with_interrupt(&sampler, target.document.bounds(), target.point[0], target.point[1], grabcut::OBJECT_AT_WORK_PX, limits, control)
                .map_err(|e| map_error(control, e))?
                .map(Arc::new)
        };
        let cache = target.cache.prepare_update(target.document.id, sampled, valid, hit, region.clone(), control)?;
        poll(control)?;
        Ok(PreparedFinder { target: target.clone(), region, cache })
    })
}
/// Move only a fully prepared cache into the live session. Document/history are unchanged.
pub fn publish(session: &mut Session, prepared: PreparedFinder, control: &Interrupt<'_>) -> Result<Option<Arc<Region>>> {
    poll(control)?;
    if !is_current(session, &prepared.target) || !cache_is_current(session, &prepared.target) {
        return Err(bad("target or cache changed during preparation"));
    }
    let document = session.active().and_then(|s| s.doc.resident_arc()).ok_or_else(|| bad("resident owner required"))?;
    let mut cache = prepared.cache;
    if let Some(cache) = cache.as_mut() {
        cache.bind_controlled(document);
    }
    poll(control)?;
    if let Some(cache) = cache {
        session.object_finder = cache;
    }
    Ok(prepared.region)
}
/// Call only after a pointer/context/cache change, not on every UI frame.
pub fn lookup(session: &Session, target: &FinderTarget, point: [f32; 2], control: &Interrupt<'_>) -> Result<Option<Arc<Region>>> {
    poll(control)?;
    if !is_current(session, target) || !point_valid(point) {
        return Err(bad("lookup target changed"));
    }
    let cache = session.object_finder.capture_controlled()?;
    let (region, _, bytes, work) = cache_lookup(target, &cache, point, control)?;
    admission(target, bytes, work, true, control)?;
    poll(control)?;
    Ok(region)
}
