//! Charged row-band Color Range worker, separate from the frozen small inline lane.
//! Exact original coordinates and query math, with separate closed current-profile
//! admission for imported pixels. No merged or wide-sample interpretation.
use crate::{
    DocumentSnapshot, EngineError, Result, Session,
    accounted_selection_session::AccountedSelectionTarget,
    jobs::JobCtx,
    selection_cmds::{BackedColorRangeSamples, CapturedBackedColorRange, ColorRangeLimits},
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    LayerContent,
    accounted_document::accounted_layer_access,
    accounted_selection::{AccountedSelection, AccountedSelectionBandBuilder},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
const LIMIT: u64 = 8_000_000_000;
const BAND_ROWS: i32 = 32;
struct Work(AtomicU64);
impl Work {
    fn consume(&self, n: u64, interrupt: &Interrupt<'_>) -> std::result::Result<(), BackingError> {
        interrupt.check().map_err(|_| BackingError::Cancelled)?;
        self.0.try_update(Ordering::Relaxed, Ordering::Relaxed, |old| old.checked_add(n).filter(|v| *v <= LIMIT)).map_err(|_| BackingError::Work)?;
        interrupt.check().map_err(|_| BackingError::Cancelled)
    }
}
fn bad(msg: &str) -> EngineError {
    EngineError::BadParams { cmd: "select.colorRange".into(), msg: msg.into() }
}
pub(crate) struct CapturedAccountedColorRange {
    query: CapturedBackedColorRange,
    source: DocumentSnapshot,
    target: AccountedSelectionTarget,
    area: Rect,
    invert: bool,
    work: Arc<Work>,
    _header: QuotaLease,
}
pub(crate) struct PreparedAccountedColorRange {
    query: BackedColorRangeSamples,
    source: DocumentSnapshot,
    target: AccountedSelectionTarget,
    selection: Option<AccountedSelection>,
    noop: bool,
    result: Value,
    work: Arc<Work>,
    _header: QuotaLease,
}
/// Cheap closed branch admission before command/journal copies.
pub(crate) fn validate_request(session: &Session, p: &Value) -> Result<()> {
    crate::accounted_color_range::validate_request(p)?;
    crate::selection_inverse_controlled::storage_dispatch_preflight(session, "select.all", &json!({}), false)?;
    if session.active_job().is_some() {
        return Err(bad("original target is not idle"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let doc = &state.doc;
    crate::imported_backed_sampling::metadata(doc)?;
    if doc.accounted().is_none() || doc.selection_channel_id.is_some() || doc.quick_mask.is_some() {
        return Err(bad("banded Color Range requires an admitted anonymous current selection"));
    }
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 262144 || doc.size.height > 262144 {
        return Err(bad("banded canvas bounds"));
    }
    let view = &state.channel_view;
    if view.target != crate::channel_cmds::ChannelTarget::Composite
        || view.layer_mask.is_some()
        || view.editing_mask.is_some()
        || view.color_hidden.len() > 3
        || view.alpha_visible.len() > 16
        || view.color_hidden.iter().any(|v| *v)
        || view.alpha_visible.iter().any(|v| *v)
    {
        return Err(bad("ordinary color view required"));
    }
    if doc.selection.as_ref().is_some_and(|s| {
        s.format().mode != ColorMode::Grayscale || s.format().alpha || !matches!(s.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
    }) {
        return Err(bad("finite narrow prior selection required"));
    }
    Ok(())
}
impl CapturedAccountedColorRange {
    pub(crate) fn capture(session: &Session, p: &Value, outer: &ReadControl<'_>) -> Result<Self> {
        validate_request(session, p)?;
        let work = Work(AtomicU64::new(0));
        let charge = |n| {
            work.consume(n, outer.interrupt)?;
            outer.check(n)
        };
        let control = ReadControl { interrupt: outer.interrupt, work: &charge };
        control.check(1)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let source = state.doc.clone();
        let area = source.bounds();
        let target = AccountedSelectionTarget::capture(session)?;
        let quota = source.quota().ok_or(BackingError::Unsupported)?;
        let header = quota
            .reserve(
                QuotaKind::Index,
                (std::mem::size_of::<Self>() + std::mem::size_of::<PreparedAccountedColorRange>() + std::mem::size_of::<Work>() + 4096) as u64,
            )
            .map_err(|_| BackingError::Quota)?;
        let id = state.active_layer.ok_or_else(|| bad("active native owner required"))?;
        let layer = accounted_layer_access(&source, id, &control)?.ok_or(BackingError::Bounds)?.layer;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(BackingError::Unsupported.into()) };
        if !backed.operative_bounds().contains_rect(&area) {
            return Err(bad("full canvas must be inside native owner extent"));
        }
        let query = CapturedBackedColorRange::capture_with_read_control(session, p, &control)?;
        let invert = p.get("invert").and_then(Value::as_bool).unwrap_or(false);
        control.check(1)?;
        Ok(Self { query, source, target, area, invert, work: Arc::new(work), _header: header })
    }
    pub(crate) fn with_control<R>(&self, interrupt: &Interrupt<'_>, f: impl FnOnce(&ReadControl<'_>) -> Result<R>) -> Result<R> {
        let charge = |n| self.work.consume(n, interrupt);
        f(&ReadControl { interrupt, work: &charge })
    }
    pub(crate) fn prepare(self, ctx: &JobCtx) -> Result<PreparedAccountedColorRange> {
        ctx.stage(0., 0.95, "Color Range", |interrupt| {
            let charge = |n| self.work.consume(n, interrupt);
            let control = ReadControl { interrupt, work: &charge };
            self.source.accounted().ok_or(BackingError::Unsupported)?.validate_selection_replacement(&control)?;
            let query = self.query.prepare_with_read_control(&control)?;
            let quota = self.source.quota().ok_or(BackingError::Unsupported)?;
            let mut builder = AccountedSelectionBandBuilder::new(self.area, quota, &control)?;
            let width = usize::try_from(self.area.width()).map_err(|_| BackingError::Length)?;
            let length = width.checked_mul(BAND_ROWS as usize).ok_or(BackingError::Length)?;
            let lease = quota
                .reserve(QuotaKind::Scratch, (length as u64).checked_mul(2).and_then(|v| v.checked_add(128)).ok_or(BackingError::Length)?)
                .map_err(|_| BackingError::Quota)?;
            let mut native = Vec::new();
            native.try_reserve_exact(length).map_err(|_| BackingError::Allocation)?;
            if native.capacity() > length.checked_mul(2).ok_or(BackingError::Length)? {
                return Err(BackingError::Allocation.into());
            }
            let mut y = self.area.y0;
            while y < self.area.y1 {
                control.check(1)?;
                let end = y.checked_add(BAND_ROWS).ok_or(BackingError::Bounds)?.min(self.area.y1);
                let band = Rect::new(self.area.x0, y, self.area.x1, end);
                // These owned RGB/Lab/coverage leases drop before the next band.
                let pixels = query.backed_pixels_with_read_control(band, &control)?;
                let coverage = query.coverage_backed_pixels_with_read_control(&pixels, ColorRangeLimits::default(), &control)?;
                native.clear();
                for (i, raw) in coverage.values().iter().copied().enumerate() {
                    if i.is_multiple_of(256) {
                        control.check(256 * 64)?;
                    }
                    if !raw.is_finite() {
                        return Err(bad("finite coverage required"));
                    }
                    let mut value = if self.invert { 1. - raw } else { raw };
                    if let Some(prior) = &self.source.selection {
                        let x = self.area.x0.checked_add(i32::try_from(i % width).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
                        let py = y.checked_add(i32::try_from(i / width).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
                        let mut word = [0; 4];
                        let bpp = prior.format().bytes_per_pixel();
                        prior.try_read_pixel_bytes(x, py, word.get_mut(..bpp).ok_or(BackingError::Length)?)?;
                        let old = photocraft_color::read_sample(&word, prior.format().sample, 0);
                        if !old.is_finite() || !(0. ..=1.).contains(&old) {
                            return Err(bad("finite unit prior required"));
                        }
                        value = value.min(old);
                    }
                    native.push((value.clamp(0., 1.) * 255. + 0.5) as u8);
                }
                if native.len() != width.checked_mul(usize::try_from(end - y).map_err(|_| BackingError::Length)?).ok_or(BackingError::Length)? {
                    return Err(BackingError::Length.into());
                }
                builder.push_band(band, &native, &control)?;
                y = end;
                ctx.progress((y - self.area.y0) as f32 / self.area.height() as f32 * 0.95, "");
            }
            drop(native);
            drop(lease);
            let selection = builder.finish(&control)?;
            let noop = match (&selection, &self.source.selection) {
                (None, None) => true,
                (Some(selection), Some(prior)) => selection.matches_native_surface(prior, &control)?,
                _ => false,
            };
            let result = json!({"selected":selection.is_some()});
            control.check(1)?;
            Ok(PreparedAccountedColorRange { query, source: self.source, target: self.target, selection, noop, result, work: self.work, _header: self._header })
        })
    }
}
impl PreparedAccountedColorRange {
    pub(crate) fn with_control<R>(&self, interrupt: &Interrupt<'_>, f: impl FnOnce(&ReadControl<'_>) -> Result<R>) -> Result<R> {
        let charge = |n| self.work.consume(n, interrupt);
        f(&ReadControl { interrupt, work: &charge })
    }
    pub(crate) fn apply(self, session: &mut Session, ctx: &JobCtx) -> Result<Value> {
        ctx.stage(0.95, 1., "Color Range", |interrupt| {
            self.with_control(interrupt, |control| {
                if self.query.backed_source_is_current(session) != Some(true) {
                    return Err(bad("original source or view changed"));
                }
                let state = session.active().ok_or(EngineError::NoDocument)?;
                if state.revision != self.target.revision
                    || state.view_revision != self.target.view_revision
                    || !state.doc.shares_state(&self.source)
                    || !state.history.current()?.shares_state(&self.source)
                {
                    return Err(bad("original publication target changed"));
                }
                if self.noop {
                    validate_request(session, &json!({"sampleAllLayers":false}))?;
                    control.check(1)?;
                    return Ok(());
                }
                session.replace_accounted_selection(self.target.clone(), "Color Range", self.selection.as_ref(), control)
            })
        })?;
        Ok(self.result)
    }
}
#[cfg(test)]
#[path = "accounted_color_range_banded_tests.rs"]
mod tests;
