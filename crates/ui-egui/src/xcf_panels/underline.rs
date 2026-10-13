//! Character-range UI for the checked retained-text underline command.
use crate::PhotocraftApp;
use photocraft_doc::LayerId;
pub(super) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some((captured, revision)) = app.session.active().map(|active| (active.doc.clone(), active.revision)) else { return };
    let hints = match current_hints(ui.ctx(), &captured, revision, layer) {
        Ok(Some(hints)) => hints,
        Ok(None) => { app.ui.xcf_underline_draft = None; return; }
        Err(error) => { ui.label(error); app.ui.xcf_underline_draft = None; return; }
    };
    // Runtime and serialized draft metadata is never qualification authority.
    // Only the bounded character positions survive a matching target.
    let restored = app.ui.xcf_underline_draft.take().filter(|draft| draft.document == captured.id.0 && draft.layer == layer.0 && draft.revision == revision);
    let [start_byte, end_byte] = hints.ranges.first().copied().unwrap_or([0, hints.content.len()]);
    let Some(start_text) = hints.content.get(..start_byte) else { return };
    let Some(end_text) = hints.content.get(..end_byte) else { return };
    let mut draft = crate::state::XcfUnderlineDraft {
        document: captured.id.0, layer: layer.0, revision, validated: true,
        content: hints.content.clone(), ranges: hints.ranges.clone(), editable: hints.editable,
        start: restored.as_ref().map_or_else(|| start_text.chars().count(), |draft| draft.start),
        end: restored.as_ref().map_or_else(|| end_text.chars().count(), |draft| draft.end),
    };
    let editable = hints.editable && live_editable(app, &captured, layer);
    let count = draft.content.chars().count();
    draft.start = draft.start.min(count);
    draft.end = draft.end.min(count);
    ui.separator();
    ui.label(tl!("Underline"));
    let current = draft
        .ranges
        .iter()
        .filter_map(|[a, b]| Some(format!("{}–{}", draft.content.get(..*a)?.chars().count(), draft.content.get(..*b)?.chars().count())))
        .collect::<Vec<_>>()
        .join(", ");
    ui.horizontal(|ui| {
        ui.label(tl!("Underlined characters"));
        ui.label(current);
    });
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Start character"));
            ui.add(egui::DragValue::new(&mut draft.start).speed(1).range(0..=count)).labelled_by(label.id);
        });
        ui.horizontal(|ui| {
            let label = ui.label(tl!("End character (exclusive)"));
            ui.add(egui::DragValue::new(&mut draft.end).speed(1).range(0..=count)).labelled_by(label.id);
        });
    });
    let proposed = byte_range(&draft.content, draft.start, draft.end);
    let changed = proposed.is_some_and(|r| draft.ranges.as_slice() != [r]);
    let mut apply = false;
    let mut clear = false;
    ui.horizontal(|ui| {
        apply = ui.add_enabled(editable && changed, egui::Button::new(tl!("Apply underline"))).clicked();
        clear = ui.add_enabled(editable && !draft.ranges.is_empty(), egui::Button::new(tl!("Clear underline"))).clicked();
    });
    app.ui.xcf_underline_draft = Some(draft);
    if clear {
        apply_checked(app, &captured, revision, layer, serde_json::json!({"layerId":layer.0.to_string(),"ranges":[]}));
    } else if apply && let Some(range) = proposed {
        apply_checked(app, &captured, revision, layer, serde_json::json!({"layerId":layer.0.to_string(),"ranges":[range]}));
    }
}
// Cached derived display data, never a resource or editing capability.
#[derive(Clone)]
struct UnderlineHints { content: String, ranges: Vec<[usize; 2]>, editable: bool }
#[derive(Clone)]
struct HintCache {
    document: photocraft_doc::DocId, revision: u64, layer: LayerId,
    snapshot: photocraft_engine::document_snapshot::WeakDocumentSnapshot,
    result: Result<Option<std::sync::Arc<UnderlineHints>>, &'static str>,
}
fn prepare_hints(doc: &photocraft_doc::Document, layer: LayerId) -> Result<Option<std::sync::Arc<UnderlineHints>>, &'static str> {
    let Some(state) = &doc.xcf_state else { return Ok(None) };
    if state.source_items.len() > 10_000 || state.items.len() > 10_000 || state.resources.len() > 10_000 { return Err("xcf.text.underline.limit"); }
    let mut sources = state.source_items.iter().filter(|source| source.owner == photocraft_doc::item_identity::ItemRef::Layer(layer));
    let Some(source) = sources.next() else { return Ok(None) };
    if sources.next().is_some() { return Ok(None); }
    let photocraft_doc::xcf_items::EditableSource::Text { content, .. } = &source.source else { return Ok(None) };
    if content.len() > 256 { return Ok(None); }
    let Some(ranges) = photocraft_engine::xcf_text_cmds::current_underline_ranges(doc, layer) else { return Ok(None) };
    if ranges.len() > 8 || ranges.iter().any(|[a,b]| *a >= *b || !content.is_char_boundary(*a) || !content.is_char_boundary(*b)) { return Ok(None); }
    let mut owned = String::new();
    owned.try_reserve_exact(content.len()).map_err(|_| "xcf.text.underline.allocation")?;
    owned.push_str(content);
    Ok(Some(std::sync::Arc::new(UnderlineHints { content: owned, ranges, editable: photocraft_engine::xcf_text_cmds::underline_edit_candidate(doc, layer) })))
}
fn hints_with(
    ctx: &egui::Context, snapshot: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, layer: LayerId,
    derive: impl FnOnce(&photocraft_doc::Document, LayerId) -> Result<Option<std::sync::Arc<UnderlineHints>>, &'static str>,
) -> Result<Option<std::sync::Arc<UnderlineHints>>, &'static str> {
    let key = egui::Id::new("xcf_underline_qualified_hints");
    if let Some(cache) = ctx.data_mut(|data| data.get_temp::<HintCache>(key))
        && cache.document == snapshot.id && cache.revision == revision && cache.layer == layer
        && cache.snapshot.upgrade().is_some_and(|old| old.shares_state(snapshot)) { return cache.result; }
    let result = derive(snapshot, layer);
    ctx.data_mut(|data| data.insert_temp(key, HintCache { document: snapshot.id, revision, layer, snapshot: snapshot.downgrade(), result: result.clone() }));
    result
}
fn current_hints(ctx: &egui::Context, snapshot: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, layer: LayerId) -> Result<Option<std::sync::Arc<UnderlineHints>>, &'static str> {
    hints_with(ctx, snapshot, revision, layer, prepare_hints)
}
fn live_editable(app: &PhotocraftApp, doc: &photocraft_doc::Document, layer: LayerId) -> bool {
    use photocraft_engine::xcf_item_locks::{effective_layer_locks, EditKind};
    app.ui.transform.is_none() && !app.session.has_jobs()
        && app.session.active().is_some_and(|active| active.floating.is_none())
        && doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_none())
        && effective_layer_locks(doc, layer).is_ok_and(|locks| locks.require(EditKind::Content).is_ok() && locks.require(EditKind::Position).is_ok())
}
fn apply_checked(app: &mut PhotocraftApp, captured: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, layer: LayerId, params: serde_json::Value) {
    if !live_editable(app, captured, layer) {
        app.ui.status = "xcf.text.underline.context".into(); app.ui.status_error = true; return;
    }
    super::run_checked(app, captured, revision, "xcf.text.underline.set", params);
}
/// UI positions are Unicode scalar indices; the command retains UTF-8 offsets.
fn byte_range(content: &str, start: usize, end: usize) -> Option<[usize; 2]> {
    if content.len() > 256 || start >= end || end > content.chars().count() {
        return None;
    }
    let offset = |index| content.char_indices().map(|(i, _)| i).chain(std::iter::once(content.len())).nth(index);
    Some([offset(start)?, offset(end)?])
}
#[cfg(test)]
mod tests;
