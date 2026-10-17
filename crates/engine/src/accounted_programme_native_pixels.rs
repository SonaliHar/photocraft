//! Concrete Clear/solid-Fill transaction for actual programme storage. Native
//! regions remain derived, bounded staging; every OLD/STAGED recipe is proved.
use super::*;
use crate::accounted_programme_current::{self, ProgrammeKind};
use photocraft_doc::{BackedLayer, accounted_document::AccountedDocument};

struct Witness {
    kind: ProgrammeKind,
    layer: LayerId,
    ordinal: usize,
    parent: Option<(usize, LayerId)>,
    canonical: usize,
    backed: BackedLayer,
}
impl Witness {
    fn capture(owner: &AccountedDocument, kind: ProgrammeKind, layer: LayerId, c: &ReadControl<'_>) -> Result<Self> {
        // Caller retains the prospective Prepared/Witness header envelope.
        let target = owner.checked_programme_native_target(layer, c)?;
        if target.storage_kind() != kind {
            return Err(BackingError::Integrity.into());
        }
        Ok(Self { kind, layer, ordinal: target.ordinal(), parent: target.parent(), canonical: target.canonical_item_index(), backed: target.backed().clone() })
    }
    fn validate(&self, owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
        let target = owner.checked_programme_native_target(self.layer, c)?;
        if target.storage_kind() != self.kind
            || target.layer_id() != self.layer
            || target.ordinal() != self.ordinal
            || target.parent() != self.parent
            || target.canonical_item_index() != self.canonical
            || target.descriptor() != self.backed.descriptor()
            || target.backed() != &self.backed
        {
            return Err(BackingError::Integrity.into());
        }
        c.check(1)?;
        Ok(())
    }
}
fn locks(owner: &AccountedDocument, id: LayerId, operation: Operation, c: &ReadControl<'_>) -> Result<bool> {
    let a = owner.accounted_layer_access(id, c)?.ok_or(EngineError::NoLayer(id))?;
    if a.locks.all || a.locks.pixels || a.imported_locks.content || matches!(operation, Operation::Clear) && (a.locks.transparency || a.imported_locks.alpha) {
        return Err(bad(operation.command(), "the native pixel target is locked"));
    }
    if matches!(operation, Operation::Clear) && owner.document().is_background(id) {
        return Err(bad(operation.command(), "accounted Background clear is not supported"));
    }
    Ok(a.locks.transparency || a.imported_locks.alpha)
}
fn selection_context(owner: &AccountedDocument, command: &str) -> Result<()> {
    if owner.document().selection_channel_id.is_some() {
        return Err(bad(command, "programme native pixels do not edit channel selections"));
    }
    Ok(())
}
pub(crate) struct Prepared<'a> {
    params: &'a Value,
    operation: Operation,
    kind: ProgrammeKind,
    preserve_alpha: bool,
    reply: Value,
    captured: DocumentSnapshot,
    snapshot: DocumentSnapshot,
    old_history: DocumentHistory,
    history: DocumentHistory,
    view: AccountedView,
    next_view: AccountedView,
    revision: u64,
    view_revision: u64,
    active: Option<LayerId>,
    anchor: Option<LayerId>,
    witness: Witness,
    area: Rect,
    changed: bool,
    stage: Option<NativeStage>,
    channel: ChannelView,
    path: Option<String>,
    source: Option<SourceIdentity>,
    source_read_only: bool,
    history_states: u32,
    history_bytes: usize,
    _lease: QuotaLease,
}
impl<'a> Prepared<'a> {
    pub(super) fn prepare(s: &Session, p: &'a Value, c: &ReadControl<'_>, operation: Operation) -> Result<Self> {
        let command = operation.command();
        c.check(1)?;
        operation.validate(s, p)?;
        idle(s, command)?;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let owner = st.doc.accounted().ok_or(BackingError::Unsupported)?;
        let kind = accounted_programme_current::kind(owner).ok_or(BackingError::Unsupported)?;
        selection_context(owner, command)?;
        let selection_guard = owner.checked_programme_selection(c)?;
        if !selection_guard.matches_owner(owner) {
            return Err(BackingError::Integrity.into());
        }
        accounted_programme_current::validate(owner, kind, c)?;
        let view = st.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if st.active_layer != view.target().active
            || st.layer_anchor != view.anchor()
            || !st.selected_layers.is_empty()
            || !st.history.current()?.shares_state(&st.doc)
        {
            return Err(BackingError::Integrity.into());
        }
        let id = match p.get("layer") {
            Some(Value::Number(n)) => LayerId(n.as_u64().filter(|n| *n != 0).ok_or_else(|| bad(command, "nonzero layer identity required"))?),
            Some(Value::Null) | None => st.active_layer.ok_or_else(|| bad(command, "no active layer"))?,
            _ => return Err(bad(command, "programme native target requires numeric identity or active layer")),
        };
        // Reserve the owned witness/header and source strings BEFORE cloning
        // actual backing/view metadata. This is separate from native stage/COW.
        if st.path.as_ref().is_some_and(|v| v.len() > 4096)
            || st.source.as_ref().is_some_and(|v| v.label.len() > 4096 || v.path.as_ref().is_some_and(|p| p.len() > 4096))
        {
            return Err(BackingError::Length.into());
        }
        let source_text = st.source.as_ref().map_or(Ok(0), |v| add(v.label.len(), v.path.as_ref().map_or(0, String::len)))?;
        let text_bytes = add(st.path.as_ref().map_or(0, String::len), source_text)?;
        let count = add(st.channel_view.color_hidden.len(), st.channel_view.alpha_visible.len())?;
        let metadata = add(mul(add(text_bytes, count)?, 2)?, add(std::mem::size_of::<Self>(), 4096 + crate::accounted_fill::REPLY_ENVELOPE)?)?;
        let mut lease = owner.quota().reserve(QuotaKind::Index, units(metadata)?).map_err(|_| BackingError::Quota)?;
        c.check(units(add(text_bytes, count)?)?)?;
        let witness = Witness::capture(owner, kind, id, c)?;
        let alpha_locked = locks(owner, id, operation, c)?;
        let preserve_alpha = alpha_locked || operation.keeps_alpha();
        let backed = &witness.backed;
        let format = backed.format();
        if !format.alpha
            || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
            || !backed.quota().shares_ledger(owner.quota())
        {
            return Err(BackingError::Unsupported.into());
        }
        if matches!(operation, Operation::Fill(_)) && !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
            return Err(bad(command, "ordinary Fill requires U8/U16/F32 native pixels"));
        }
        let selection: Option<&Surface> = selection_guard.selection();
        let area = owner.document().bounds();
        let bpp = format.bytes_per_pixel();
        let length = mul(mul(area.width() as usize, area.height() as usize)?, bpp)?;
        if !area.is_empty() && (!backed.operative_bounds().contains_rect(&area) || length > MAX_NATIVE_BYTES) {
            return Err(bad(command, "programme pixels require a contained native ROI of at most 16 MiB"));
        }
        let mut cells = 0usize;
        for _ in area.tiles() {
            c.check(1)?;
            cells = add(cells, 1)?;
            if cells > 4096 {
                return Err(BackingError::Length.into());
            }
        }
        let tile_bytes = mul(65536, bpp)?;
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
            add(
                add(channel.color_hidden.capacity(), channel.alpha_visible.capacity())?,
                add(std::mem::size_of::<Self>(), 4096 + crate::accounted_fill::REPLY_ENVELOPE)?,
            )?,
        )?;
        if actual > metadata {
            return Err(BackingError::Allocation.into());
        }
        lease.reconcile_capacity(units(actual)?).map_err(|_| BackingError::Quota)?;
        prepay(c, crate::accounted_fill::REPLY_ENVELOPE as u64)?;
        let reply = match operation {
            Operation::Clear => Value::Null,
            Operation::Fill(v) => serde_json::json!({"contents":v.contents}),
        };
        let kernel_extra = match operation {
            Operation::Clear => 0,
            Operation::Fill(v) => v.cell_requirements(Rect::new(0, 0, TILE_SIZE, TILE_SIZE), bpp)?.0,
        };
        let stage = if area.is_empty() {
            None
        } else {
            // Two full native Vec capacities and the maximum simultaneous
            // single-cell input/COW/default/reader/writer/planner storage. The
            // original source and final backing overrides keep their own fees.
            let envelope = add(add(mul(length, 4)?, add(mul(tile_bytes, 8)?, add(mul(cells, 512)?, 65536)?)?)?, kernel_extra)?;
            let fee = owner.quota().reserve(QuotaKind::Resident, units(envelope)?).map_err(|_| BackingError::Quota)?;
            let mut stage = NativeStage { input: buffer(length, c)?, output: buffer(length, c)?, _lease: fee };
            if add(stage.input.capacity(), stage.output.capacity())? > mul(length, 4)? {
                return Err(BackingError::Allocation.into());
            }
            let native_view = backed.checked_view();
            for cell in area.tiles() {
                c.check(1)?;
                let part = cell.rect().intersect(&area);
                let pin = native_view.pin_tile(cell, c).map_err(|e| view_error(e, command))?;
                let mut raw = [0u8; 40];
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        let at = offset(area, x, y, bpp)?;
                        let target = stage.input.get_mut(at..add(at, bpp)?).ok_or(BackingError::Length)?;
                        let native = raw.get_mut(..bpp).ok_or(BackingError::Length)?;
                        pin.read_pixel_native_into(x, y, native, c).map_err(|e| view_error(e, command))?;
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
                // Additional selected-wide Clear work: both coverage walks, native
                // alpha-only preflight and fractional fixed-pixel arithmetic.
                // Existing provider/copy debits are not credit for this work.
                if matches!(operation, Operation::Clear) && selection.is_some() && matches!(format.sample, SampleType::U32 | SampleType::F64) {
                    prepay(c, units(mul(mul(part.width() as usize, part.height() as usize)?, 1024)?)?)?;
                }
                if let Operation::Fill(v) = operation {
                    prepay(c, v.cell_requirements(part, bpp)?.1)?;
                }
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
                let mut surface = Surface::try_with_default_bytes(format, backed.default_bytes()).map_err(|e| raster(e, command))?;
                surface.try_write_interleaved_with(part, &bytes, c.interrupt).map_err(|e| raster(e, command))?;
                if surface.resident_accounted_bytes().map_err(|e| raster(e, command))? > units(mul(tile_bytes, 2)?)? {
                    return Err(BackingError::Allocation.into());
                }
                match operation {
                    Operation::Clear => crate::pixels::try_clear_surface(&mut surface, part, selection, c.interrupt)?,
                    Operation::Fill(v) => v.apply(&mut surface, part, selection, alpha_locked)?,
                }
                c.check(0)?;
                surface.try_read_interleaved_into_with(part, &mut bytes, c.interrupt).map_err(|e| raster(e, command))?;
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
            owner.validate_programme_preserving_native_edit(AccountedEdit::NativePixels { layer: id, rect: area, bytes: &stage.output }, c)?;
            Some(stage)
        };
        let mut changed = false;
        if let Some(stage) = &stage {
            for (a, b) in stage.input.chunks(4096).zip(stage.output.chunks(4096)) {
                c.check(units(a.len())?)?;
                changed |= a != b;
            }
        }
        let mut history = st.history.clone();
        let snapshot = if changed {
            let stage = stage.as_ref().ok_or(BackingError::Integrity)?;
            if preserve_alpha {
                crate::accounted_session::verify_programme_native_stroke_alpha(owner, id, area, &stage.output, c)?;
            }
            let staged = owner.try_edit_programme_preserving_native(AccountedEdit::NativePixels { layer: id, rect: area, bytes: &stage.output }, c)?;
            accounted_programme_current::validate(&staged, kind, c)?;
            history.set_accounted_state_limit(s.prefs().performance.history_states.max(1) as usize)?;
            history.set_current_layers_borrowed(view.target(), c)?;
            history.try_commit_prepared_accounted(owner, staged, operation.label(), view.target(), c)?
        } else {
            // A shared no-op remains a complete STAGED recipe proof.
            accounted_programme_current::validate(owner, kind, c)?;
            st.doc.clone()
        };
        let next_view = view.try_replace(snapshot.accounted().ok_or(BackingError::Integrity)?, st.active_layer, view.selected(), st.layer_anchor, c)?;
        c.check(1)?;
        Ok(Self {
            params: p,
            operation,
            kind,
            preserve_alpha,
            reply,
            captured: st.doc.clone(),
            snapshot,
            old_history: st.history.clone(),
            history,
            view: view.clone(),
            next_view,
            revision: st.revision,
            view_revision: st.view_revision,
            active: st.active_layer,
            anchor: st.layer_anchor,
            witness,
            area,
            changed,
            stage,
            channel,
            path,
            source,
            source_read_only: st.source_read_only,
            history_states: s.prefs().performance.history_states,
            history_bytes: s.prefs().performance.history_budget_bytes(),
            _lease: lease,
        })
    }
    pub(crate) fn execute(self, s: &mut Session, c: &ReadControl<'_>) -> Result<Value> {
        let command = self.operation.command();
        c.check(1)?;
        self.operation.validate(s, self.params)?;
        idle(s, command)?;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let owner = st.doc.accounted().ok_or(BackingError::Unsupported)?;
        c.check(units(add(self.channel.color_hidden.len(), self.channel.alpha_visible.len())?)?)?;
        for value in self.path.iter().chain(self.source.iter().flat_map(|v| std::iter::once(&v.label).chain(v.path.iter()))) {
            c.check(units(value.len())?)?;
        }
        if !st.doc.shares_state(&self.captured)
            || st.revision != self.revision
            || st.view_revision != self.view_revision
            || !st.history.shares_accounted_history(&self.old_history)
            || st.history.max_states != self.old_history.max_states
            || st.history.max_bytes != self.old_history.max_bytes
            || !st.accounted_view.as_ref().is_some_and(|v| v.shares_state(&self.view))
            || st.active_layer != self.active
            || st.layer_anchor != self.anchor
            || !st.selected_layers.is_empty()
            || st.channel_view != self.channel
            || st.path != self.path
            || st.source != self.source
            || st.source_read_only != self.source_read_only
            || s.prefs().performance.history_states != self.history_states
            || s.prefs().performance.history_budget_bytes() != self.history_bytes
            || !self.operation.tools_match(s)
        {
            return Err(bad(command, "the prepared programme pixels target older document/view/source/tool/history state"));
        }
        selection_context(owner, command)?;
        let selection_guard = owner.checked_programme_selection(c)?;
        if !selection_guard.matches_owner(owner) {
            return Err(BackingError::Integrity.into());
        }
        let alpha_locked = locks(owner, self.witness.layer, self.operation, c)?;
        if (alpha_locked || self.operation.keeps_alpha()) != self.preserve_alpha {
            return Err(BackingError::Integrity.into());
        }
        accounted_programme_current::validate(owner, self.kind, c)?;
        self.witness.validate(owner, c)?;
        accounted_programme_current::validate(self.snapshot.accounted().ok_or(BackingError::Integrity)?, self.kind, c)?;
        if let Some(stage) = &self.stage {
            let edit = AccountedEdit::NativePixels { layer: self.witness.layer, rect: self.area, bytes: &stage.output };
            owner.validate_programme_preserving_native_edit(edit, c)?;
            if self.preserve_alpha {
                crate::accounted_session::verify_programme_native_stroke_alpha(owner, self.witness.layer, self.area, &stage.output, c)?;
            }
        }
        let revision = st.revision.checked_add(u64::from(self.changed)).ok_or(BackingError::Length)?;
        c.check(1)?;
        if self.changed {
            let state = s.active_mut().ok_or(EngineError::NoDocument)?;
            state.doc = self.snapshot;
            state.history = self.history;
            state.accounted_view = Some(self.next_view);
            state.revision = revision;
            state.last_damage = Some(self.area);
            state.coalesce = None;
            state._accounted_metadata_gesture = None;
        }
        Ok(self.reply)
    }
}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_programme_native_pixels_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod tests;
