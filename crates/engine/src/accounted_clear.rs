//! Bounded native Clear for the existing plain imported accounted owner.
//! The original Clear kernel runs on one private native cell at a time; one
//! final typed region write owns the history step. No filter/source grant.
use crate::{
    DocumentSnapshot, EngineError, Result, Session,
    accounted_view::AccountedView,
    channel_cmds::{ChannelTarget, ChannelView},
    document_history::DocumentHistory,
    source_policy::SourceIdentity,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    LayerContent, LayerId,
    accounted_document::{AccountedEdit, accounted_layer_access},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    RasterInputError, Surface,
    backing::{
        BackingError, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
    checked_view::RasterViewError,
};
use serde_json::Value;

const ID: &str = "edit.clear";
// This synchronous first lane owns one contiguous native region, not a whole
// image materialization. Larger requests require a separately owned transaction.
const MAX_NATIVE_BYTES: usize = 16 << 20;
pub(crate) const REGISTERED_WORK_LIMIT: u64 = 1 << 32;
fn bad(s: &str) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: s.into() }
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b).ok_or_else(|| BackingError::Length.into())
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or_else(|| BackingError::Length.into())
}
fn units(n: usize) -> Result<u64> {
    u64::try_from(n).map_err(|_| BackingError::Length.into())
}
fn prepay(c: &ReadControl<'_>, mut n: u64) -> Result<()> {
    while n != 0 {
        let part = n.min(4096);
        c.check(part)?;
        n -= part;
    }
    c.check(0)?;
    Ok(())
}
fn raster(e: RasterInputError) -> EngineError {
    match e {
        RasterInputError::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(format!("accounted Clear private raster: {e}")),
    }
}
fn view_error(e: RasterViewError) -> EngineError {
    match e {
        RasterViewError::Backed(e) | RasterViewError::Control(e) | RasterViewError::Descriptor(e) => e.into(),
        RasterViewError::Resident(e) => raster(e),
        e => EngineError::Other(format!("accounted Clear native read: {e}")),
    }
}
fn decimal(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    if s.is_empty() || s.len() > 20 || s.len() > 1 && s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Cheap bounded request shape only. The paid owner and target gates below are
/// mandatory even when the command's enabled query succeeds.
pub(crate) fn validate_request(session: &Session, p: &Value) -> Result<()> {
    if !p.is_null() {
        let o = p.as_object().filter(|o| o.len() <= 4).ok_or_else(|| bad("bounded Clear parameters required"))?;
        if o.keys().any(|k| !matches!(k.as_str(), "layer" | "target" | "documentId" | "expectedRevision")) {
            return Err(bad("unsupported accounted Clear parameter"));
        }
    }
    if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
        return Err(bad("accounted Clear supports native layer pixels only"));
    }
    if p.get("layer").and_then(Value::as_str).is_some_and(|v| v.len() > 4096) {
        return Err(bad("Clear layer name is too long"));
    }
    let st = session.active().ok_or(EngineError::NoDocument)?;
    if st.doc.accounted().is_none_or(|o| o.is_readonly_threshold_alpha_storage()) || st.doc.xcf_state.is_none() {
        return Err(bad("accounted Clear requires the plain imported native owner"));
    }
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(d), Some(r)) if decimal(d) == Some(st.doc.id.0) && decimal(r) == Some(st.revision) => {}
        _ => return Err(bad("captured Clear document or revision changed")),
    }
    Ok(())
}
fn channel(v: &ChannelView) -> Result<()> {
    if v.target != ChannelTarget::Composite
        || v.layer_mask.is_some()
        || v.editing_mask.is_some()
        || v.color_hidden.len() > 32
        || v.alpha_visible.len() > 4096
        || v.color_hidden.iter().any(|v| *v)
        || v.alpha_visible.iter().any(|v| *v)
    {
        return Err(bad("accounted Clear requires all ordinary color channels"));
    }
    Ok(())
}
fn idle(s: &Session) -> Result<()> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if s.active_job().is_some()
        || s.command_has_job_on(st.doc.id)
        || s.command_has_pending_job()
        || s.coalesce_request.is_some()
        || s.color_restrict.is_some()
        || st.floating.is_some()
        || st.coalesce.is_some()
    {
        return Err(bad("accounted Clear requires an idle pixel target"));
    }
    channel(&st.channel_view)
}
fn locks(doc: &photocraft_doc::accounted_document::AccountedDocument, id: LayerId, c: &ReadControl<'_>) -> Result<()> {
    let a = accounted_layer_access(doc.document(), id, c)?.ok_or(EngineError::NoLayer(id))?;
    if a.locks.all || a.locks.pixels || a.locks.transparency || a.imported_locks.content || a.imported_locks.alpha {
        return Err(bad("the Clear pixel target is locked"));
    }
    if doc.document().is_background(id) {
        return Err(bad("accounted Background clear is not supported"));
    }
    Ok(())
}
fn text(v: &str) -> Result<String> {
    let mut s = String::new();
    s.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if s.capacity() > mul(v.len(), 2)? {
        return Err(BackingError::Allocation.into());
    }
    s.push_str(v);
    Ok(s)
}
fn flags(v: &[bool], c: &ReadControl<'_>) -> Result<Vec<bool>> {
    let mut out = Vec::new();
    out.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if out.capacity() > mul(v.len(), 2)? {
        return Err(BackingError::Allocation.into());
    }
    for b in v {
        c.check(1)?;
        out.push(*b);
    }
    Ok(out)
}
struct NativeStage {
    input: Vec<u8>,
    output: Vec<u8>,
    _lease: QuotaLease,
}
fn buffer(n: usize, c: &ReadControl<'_>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| BackingError::Allocation)?;
    if out.capacity() > mul(n, 2)? {
        return Err(BackingError::Allocation.into());
    }
    while out.len() < n {
        let end = add(out.len(), (n - out.len()).min(65536))?;
        c.check(units(end - out.len())?)?;
        out.resize(end, 0);
    }
    Ok(out)
}
fn offset(area: Rect, x: i32, y: i32, bpp: usize) -> Result<usize> {
    let row = usize::try_from(i64::from(y) - i64::from(area.y0)).map_err(|_| BackingError::Bounds)?;
    let col = usize::try_from(i64::from(x) - i64::from(area.x0)).map_err(|_| BackingError::Bounds)?;
    mul(add(mul(row, area.width() as usize)?, col)?, bpp)
}

pub(crate) struct Prepared<'a> {
    params: &'a Value,
    snapshot: DocumentSnapshot,
    history: DocumentHistory,
    view: AccountedView,
    revision: u64,
    view_revision: u64,
    layer: LayerId,
    area: Rect,
    stage: Option<NativeStage>,
    channel: ChannelView,
    path: Option<String>,
    source: Option<SourceIdentity>,
    source_read_only: bool,
    _lease: QuotaLease,
}
impl<'a> Prepared<'a> {
    pub(crate) fn prepare(s: &Session, p: &'a Value, c: &ReadControl<'_>) -> Result<Self> {
        c.check(1)?;
        validate_request(s, p)?;
        idle(s)?;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let owner = st.doc.accounted().ok_or(BackingError::Unsupported)?;
        owner.validate_selection_replacement(c)?;
        let view = st.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if st.active_layer != view.target().active
            || st.layer_anchor != view.anchor()
            || !st.selected_layers.is_empty()
            || !st.history.current()?.shares_state(&st.doc)
        {
            return Err(BackingError::Integrity.into());
        }
        // The imported gate is flat/bounded. Pay the actual name comparisons
        // before reusing the original upstream layer selector (including names).
        for layer in &st.doc.layers {
            c.check(units(add(layer.name.len(), p.get("layer").and_then(Value::as_str).map_or(0, str::len))?)?)?;
        }
        // The paid imported gate guarantees a flat layer list. Match the
        // upstream reverse-name / numeric / null-active semantics without its
        // resident Document::walk Vec allocation.
        let id = match p.get("layer") {
            Some(Value::Number(n)) => LayerId(n.as_u64().ok_or_else(|| bad("layer id must be a nonnegative integer"))?),
            Some(Value::String(name)) => st.doc.layers.iter().rev().find(|l| l.name == *name).map(|l| l.id).ok_or_else(|| bad("no layer with this name"))?,
            Some(Value::Null) | None => st.active_layer.ok_or_else(|| bad("no active layer"))?,
            _ => return Err(bad("layer must be an id or name")),
        };
        locks(owner, id, c)?;
        let access = accounted_layer_access(owner.document(), id, c)?.ok_or(EngineError::NoLayer(id))?;
        let LayerContent::BackedRaster(backed) = &access.layer.content else { return Err(BackingError::Unsupported.into()) };
        let format = backed.format();
        if !format.alpha
            || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
            || !backed.quota().shares_ledger(owner.quota())
        {
            return Err(bad("unsupported accounted Clear native format"));
        }
        let selection = owner.document().selection.as_ref();
        let area = if let Some(mask) = selection {
            // Existing native content-bounds scan is a polled provider. Price
            // complete stored tiles/defaults, not just the selected rectangle.
            let scan = add(mul(mask.tile_count(), add(mul(65536, mask.format().bytes_per_pixel())?, 256)?)?, mask.default_bytes().len())?;
            prepay(c, units(mul(scan, 32)?)?)?;
            mask.try_content_bounds_with_interrupt(c.interrupt).map_err(raster)?
        } else {
            owner.document().bounds()
        };
        let bpp = format.bytes_per_pixel();
        let length = mul(mul(area.width() as usize, area.height() as usize)?, bpp)?;
        if !area.is_empty() && (!backed.operative_bounds().contains_rect(&area) || length > MAX_NATIVE_BYTES) {
            return Err(bad("accounted Clear requires a contained native ROI of at most 16 MiB"));
        }
        let cells = if area.is_empty() {
            0
        } else {
            let last_x = area.x1.checked_sub(1).ok_or(BackingError::Bounds)?.div_euclid(TILE_SIZE);
            let last_y = area.y1.checked_sub(1).ok_or(BackingError::Bounds)?.div_euclid(TILE_SIZE);
            let columns = usize::try_from(i64::from(last_x) - i64::from(area.x0.div_euclid(TILE_SIZE)) + 1).map_err(|_| BackingError::Bounds)?;
            let rows = usize::try_from(i64::from(last_y) - i64::from(area.y0.div_euclid(TILE_SIZE)) + 1).map_err(|_| BackingError::Bounds)?;
            mul(columns, rows)?
        };
        let tile_bytes = mul(65536, bpp)?;
        if st.path.as_ref().is_some_and(|v| v.len() > 4096)
            || st.source.as_ref().is_some_and(|v| v.label.len() > 4096 || v.path.as_ref().is_some_and(|v| v.len() > 4096))
        {
            return Err(BackingError::Length.into());
        }
        let source_text = st.source.as_ref().map_or(Ok(0), |v| add(v.label.len(), v.path.as_ref().map_or(0, String::len)))?;
        let text_bytes = add(st.path.as_ref().map_or(0, String::len), source_text)?;
        let count = add(st.channel_view.color_hidden.len(), st.channel_view.alpha_visible.len())?;
        let metadata = add(mul(add(text_bytes, count)?, 2)?, add(std::mem::size_of::<Self>(), 4096)?)?;
        let mut lease = owner.quota().reserve(QuotaKind::Index, units(metadata)?).map_err(|_| BackingError::Quota)?;
        c.check(units(add(text_bytes, count)?)?)?;
        let path = st.path.as_deref().map(text).transpose()?;
        let source = st
            .source
            .as_ref()
            .map(|v| Ok::<_, EngineError>(SourceIdentity { label: text(&v.label)?, path: v.path.as_deref().map(text).transpose()?, origin: v.origin }))
            .transpose()?;
        let channel = ChannelView {
            target: st.channel_view.target,
            color_hidden: flags(&st.channel_view.color_hidden, c)?,
            alpha_visible: flags(&st.channel_view.alpha_visible, c)?,
            quick_mask_hidden: st.channel_view.quick_mask_hidden,
            layer_mask: None,
            editing_mask: None,
        };
        let actual = add(
            add(path.as_ref().map_or(0, String::capacity), source.as_ref().map_or(0, |v| v.label.capacity() + v.path.as_ref().map_or(0, String::capacity)))?,
            add(add(channel.color_hidden.capacity(), channel.alpha_visible.capacity())?, add(std::mem::size_of::<Self>(), 4096)?)?,
        )?;
        if actual > metadata {
            return Err(BackingError::Allocation.into());
        }
        lease.reconcile_capacity(units(actual)?).map_err(|_| BackingError::Quota)?;
        let stage = if area.is_empty() {
            None
        } else {
            // Two full native Vec capacities and the maximum simultaneous
            // single-cell input/COW/default/reader/writer/planner storage. The
            // original source and final backing overrides keep their own fees.
            let envelope = add(mul(length, 4)?, add(mul(tile_bytes, 8)?, add(mul(cells, 512)?, 65536)?)?)?;
            let fee = owner.quota().reserve(QuotaKind::Resident, units(envelope)?).map_err(|_| BackingError::Quota)?;
            let mut stage = NativeStage { input: buffer(length, c)?, output: buffer(length, c)?, _lease: fee };
            if add(stage.input.capacity(), stage.output.capacity())? > mul(length, 4)? {
                return Err(BackingError::Allocation.into());
            }
            let native_view = backed.checked_view();
            for cell in area.tiles() {
                c.check(1)?;
                let part = cell.rect().intersect(&area);
                let pin = native_view.pin_tile(cell, c).map_err(view_error)?;
                let mut raw = [0u8; 40];
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        let at = offset(area, x, y, bpp)?;
                        let target = stage.input.get_mut(at..add(at, bpp)?).ok_or(BackingError::Length)?;
                        let native = raw.get_mut(..bpp).ok_or(BackingError::Length)?;
                        pin.read_pixel_native_into(x, y, native, c).map_err(view_error)?;
                        c.check(units(bpp)?)?;
                        target.copy_from_slice(native);
                    }
                }
                drop(pin);
            }
            for cell in area.tiles() {
                c.check(1)?;
                let part = cell.rect().intersect(&area);
                let local_length = mul(mul(part.width() as usize, part.height() as usize)?, bpp)?;
                // No new allowance: pay complete bounded original provider work
                // before its Surface/maps/COW buffers. It polls between cells;
                // the ordinary kernel's own per-cell rayon body is unchanged.
                let mask_metadata = selection.map_or(Ok(0), |m| add(mul(m.tile_count(), 512)?, m.default_bytes().len()))?;
                prepay(c, units(add(add(mul(tile_bytes, 16)?, mul(local_length, 64)?)?, mask_metadata)?)?)?;
                let mut bytes = buffer(local_length, c)?;
                for (row, y) in (part.y0..part.y1).enumerate() {
                    let from = offset(area, part.x0, y, bpp)?;
                    let n = mul(part.width() as usize, bpp)?;
                    let at = mul(row, n)?;
                    c.check(units(n)?)?;
                    bytes
                        .get_mut(at..add(at, n)?)
                        .ok_or(BackingError::Length)?
                        .copy_from_slice(stage.input.get(from..add(from, n)?).ok_or(BackingError::Length)?);
                }
                let mut surface = Surface::try_with_default_bytes(format, backed.default_bytes()).map_err(raster)?;
                surface.try_write_interleaved_with(part, &bytes, c.interrupt).map_err(raster)?;
                if surface.resident_accounted_bytes().map_err(raster)? > units(mul(tile_bytes, 2)?)? {
                    return Err(BackingError::Allocation.into());
                }
                crate::pixels::try_clear_surface(&mut surface, part, selection, c.interrupt)?;
                c.check(0)?;
                surface.try_read_interleaved_into_with(part, &mut bytes, c.interrupt).map_err(raster)?;
                for (row, y) in (part.y0..part.y1).enumerate() {
                    let to = offset(area, part.x0, y, bpp)?;
                    let n = mul(part.width() as usize, bpp)?;
                    let at = mul(row, n)?;
                    c.check(units(n)?)?;
                    stage.output.get_mut(to..add(to, n)?).ok_or(BackingError::Length)?.copy_from_slice(bytes.get(at..add(at, n)?).ok_or(BackingError::Length)?);
                }
                drop(surface);
                drop(bytes);
                c.check(1)?;
            }
            owner.validate_imported_native_edit(AccountedEdit::NativePixels { layer: id, rect: area, bytes: &stage.output }, c)?;
            Some(stage)
        };
        c.check(1)?;
        Ok(Self {
            params: p,
            snapshot: st.doc.clone(),
            history: st.history.clone(),
            view: view.clone(),
            revision: st.revision,
            view_revision: st.view_revision,
            layer: id,
            area,
            stage,
            channel,
            path,
            source,
            source_read_only: st.source_read_only,
            _lease: lease,
        })
    }
    pub(crate) fn execute(self, s: &mut Session, c: &ReadControl<'_>) -> Result<Value> {
        c.check(1)?;
        validate_request(s, self.params)?;
        idle(s)?;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        c.check(units(add(self.channel.color_hidden.len(), self.channel.alpha_visible.len())?)?)?;
        for v in self.path.iter().chain(self.source.iter().flat_map(|v| std::iter::once(&v.label).chain(v.path.iter()))) {
            c.check(units(v.len())?)?;
        }
        if !st.doc.shares_state(&self.snapshot)
            || st.revision != self.revision
            || st.view_revision != self.view_revision
            || !st.accounted_view.as_ref().is_some_and(|v| v.shares_state(&self.view))
            || st.active_layer != self.view.target().active
            || st.layer_anchor != self.view.anchor()
            || !st.selected_layers.is_empty()
            || st.channel_view != self.channel
            || st.path != self.path
            || st.source != self.source
            || st.source_read_only != self.source_read_only
            || !st.history.shares_accounted_history(&self.history)
            || st.history.max_states != self.history.max_states
            || st.history.max_bytes != self.history.max_bytes
        {
            return Err(bad("the prepared Clear targets older Session state"));
        }
        locks(st.doc.accounted().ok_or(BackingError::Integrity)?, self.layer, c)?;
        let Some(stage) = &self.stage else {
            c.check(1)?;
            return Ok(Value::Null);
        };
        let mut changed = false;
        for (a, b) in stage.input.chunks(4096).zip(stage.output.chunks(4096)) {
            c.check(units(a.len())?)?;
            changed |= a != b;
        }
        if changed {
            s.edit_accounted("Clear", AccountedEdit::NativePixels { layer: self.layer, rect: self.area, bytes: &stage.output }, c)?;
        } else {
            c.check(1)?;
        }
        Ok(Value::Null)
    }
}

#[cfg(test)]
#[path = "accounted_clear_tests.rs"]
mod tests;
