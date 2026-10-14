//! Explicit bounded resource drilldown. Reported byte/activation metadata is not
//! a file-access grant, recovery action, or live render qualification.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::DocId;
use photocraft_engine::WeakDocumentSnapshot;
use serde_json::{Value, json};
use std::sync::Arc;
const PAGE: usize = 64;
const MAX_ROWS: usize = 1000;
const MAX_NAMES: usize = 1 << 20;
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourcePageDraft {
    pub document: u64,
    pub revision: u64,
    pub offset: usize,
    pub open: bool,
}
#[derive(Clone)]
struct Target { document: DocId, revision: u64, snapshot: WeakDocumentSnapshot }
struct Row { id: u64, name: String, kind: &'static str, status: &'static str, activated: Option<bool>, bytes: u64, hash: Option<String> }
struct Report { target: Target, rows: Vec<Row> }
fn invalid() -> String { tl!("The resource details exceed the supported display limit or are unavailable.").to_owned() }
fn stale() -> String { tl!("The document changed. Inspect Resources again.").to_owned() }
fn stable(app: &PhotocraftApp, target: &Target) -> bool {
    app.session.active().is_some_and(|active| active.doc.id == target.document && active.revision == target.revision
        && target.snapshot.upgrade().is_some_and(|original| original.shares_state(&active.doc)))
}
fn page_offset(draft: &ResourcePageDraft, document: DocId, revision: u64, count: usize) -> usize {
    if draft.document == document.0 && draft.revision == revision && count <= MAX_ROWS
        && draft.offset.is_multiple_of(PAGE) && draft.offset < count { draft.offset } else { 0 }
}
fn id(value: &Value) -> Result<u64, String> {
    let text = value.as_str().ok_or_else(invalid)?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) { return Err(invalid()); }
    text.parse().map_err(|_| invalid())
}
fn decode(value: &Value, target: Target) -> Result<Report, String> {
    let rows = value.get("resources").and_then(Value::as_array).filter(|rows| rows.len() <= MAX_ROWS).ok_or_else(invalid)?;
    if value.get("count").and_then(Value::as_u64) != Some(rows.len() as u64) { return Err(invalid()); }
    let mut output = Vec::new(); output.try_reserve_exact(rows.len()).map_err(|_| invalid())?;
    let mut identities = std::collections::HashSet::new(); identities.try_reserve(rows.len()).map_err(|_| invalid())?;
    let mut names = 0usize;
    for row in rows {
        let resource = id(row.get("id").ok_or_else(invalid)?)?;
        if !identities.insert(resource) { return Err(invalid()); }
        let kind = match row.get("kind").and_then(Value::as_str) {
            Some("font") => "Font", Some("pattern") => "Pattern", Some("gradient") => "Gradient",
            Some("brush") => "Brush", Some("palette") => "Palette", Some("color-profile") => "Color Profile",
            Some("linked-image") => "Linked Image", _ => return Err(invalid()),
        };
        let status = match row.get("status").and_then(Value::as_str) {
            Some("exact-resolved-bytes") => "Exact supplied bytes", Some("substituted-resolved-bytes") => "Explicit substitution",
            Some("exact-hash-bound") => "Hash-bound stored bytes", Some("owned-unverified") => "Unverified stored bytes",
            Some("unresolved") => "Unavailable", _ => return Err(invalid()),
        };
        let name = row.get("name").and_then(Value::as_str).filter(|name| name.len() <= (16 << 10) && !name.contains('\0')).ok_or_else(invalid)?;
        names = names.checked_add(name.len()).filter(|sum| *sum <= MAX_NAMES).ok_or_else(invalid)?;
        // Copy only bounded display names. Never copy locators, resolution data,
        // resource payloads, or collection/provider details into view state.
        let end = name.char_indices().nth(256).map_or(name.len(), |(offset, _)| offset);
        let prefix = name.get(..end).ok_or_else(invalid)?;
        let mut display = String::new(); display.try_reserve_exact(prefix.len() + 3).map_err(|_| invalid())?;
        display.push_str(prefix); if end < name.len() { display.push('…'); }
        let activated = match row.get("activated") { Some(Value::Bool(value)) => Some(*value), Some(Value::Null) => None, _ => return Err(invalid()) };
        let bytes = row.get("ownedByteLength").and_then(Value::as_u64).ok_or_else(invalid)?;
        let hash = match row.get("contentHash") {
            Some(Value::Null) => None,
            Some(Value::String(hash)) if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
                let mut copy = String::new(); copy.try_reserve_exact(64).map_err(|_| invalid())?; copy.push_str(hash); Some(copy)
            }
            _ => return Err(invalid()),
        };
        output.push(Row { id: resource, name: display, kind, status, activated, bytes, hash });
    }
    Ok(Report { target, rows: output })
}
fn inspect(app: &mut PhotocraftApp, target: Target) -> Result<Report, String> {
    if !stable(app, &target) { return Err(stale()); }
    // Existing registered inspection is synchronous and may spend its bounded
    // 4,000,000-sample activation budget. This is an explicit request boundary,
    // not an asynchronous cancellation or per-frame qualification promise.
    let value = app.run("xcf.resource.inspect", json!({}))?;
    if !stable(app, &target) { return Err(stale()); }
    decode(&value, target)
}
pub(super) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(active) = app.session.active() else { return; };
    let Some(state) = active.doc.xcf_state.as_ref() else { return; };
    if state.resources.len() > MAX_ROWS { ui.label(invalid()); return; }
    let target = Target { document: active.doc.id, revision: active.revision, snapshot: active.doc.downgrade() };
    let key = egui::Id::new("xcf_resource_details_report");
    let cached = ui.data(|data| data.get_temp::<Arc<Report>>(key)).filter(|report| stable(app, &report.target));
    let old = app.ui.xcf_resource_page_draft.take();
    let mut cursor = old.filter(|draft| draft.document == target.document.0 && draft.revision == target.revision)
        .unwrap_or(ResourcePageDraft { document: target.document.0, revision: target.revision, offset: 0, open: false });
    cursor.offset = page_offset(&cursor, target.document, target.revision, cached.as_ref().map_or(state.resources.len(), |report| report.rows.len()));
    let busy = app.session.has_jobs() || app.ui.transform.is_some() || active.floating.is_some() || active.coalesce.is_some()
        || state.floating.is_some();
    let mut query = false;
    if ui.add_enabled(!busy, egui::Button::new(tl!("Resources…"))).clicked() { cursor.open = true; query = true; }
    if cursor.open {
        ui.label(egui::RichText::new(tl!("Resources")).color(Tokens::get(ui.ctx()).text));
        ui.label(tl!("Reported resource metadata only. Inspection does not grant access or activate recovery."));
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new(tl!("Inspect"))).clicked() { query = true; }
            if ui.button(tl!("Close")).clicked() { cursor.open = false; }
        });
        if let Some(report) = &cached {
            if report.rows.is_empty() { ui.label(tl!("No retained resources.")); }
            else {
                ui.horizontal(|ui| {
                    if ui.add_enabled(cursor.offset >= PAGE, egui::Button::new(tl!("Previous"))).clicked() { cursor.offset = cursor.offset.saturating_sub(PAGE); }
                    if ui.add_enabled(cursor.offset.saturating_add(PAGE) < report.rows.len(), egui::Button::new(tl!("Next"))).clicked() { cursor.offset = cursor.offset.saturating_add(PAGE); }
                    ui.label(format!("{}–{} / {}", cursor.offset + 1, cursor.offset.saturating_add(PAGE).min(report.rows.len()), report.rows.len()));
                });
                egui::ScrollArea::vertical().max_height(320.0).id_salt("xcf_resource_rows").show(ui, |ui| {
                    for row in report.rows.iter().skip(cursor.offset).take(PAGE) {
                        ui.push_id(row.id, |ui| {
                            ui.label(format!("{} {} · {}", tl!("Resource"), row.id, crate::i18n::t(row.kind)));
                            if !row.name.is_empty() { ui.label(&row.name); }
                            ui.label(crate::i18n::t(row.status));
                            ui.label(format!("{}: {}", tl!("Stored bytes"), row.bytes));
                            if let Some(hash) = &row.hash { ui.label(format!("{}: {hash}", tl!("Content hash"))); }
                            ui.label(match row.activated { Some(true) => tl!("Reported available consumer"), Some(false) => tl!("No reported available consumer"), None => tl!("Consumer status unavailable") });
                            ui.separator();
                        });
                    }
                });
            }
        } else { ui.label(tl!("Inspect to view the current resource details.")); }
    }
    if query {
        match inspect(app, target) {
            Ok(report) => { cursor.offset = page_offset(&cursor, report.target.document, report.target.revision, report.rows.len()); ui.data_mut(|data| data.insert_temp(key, Arc::new(report))); }
            Err(error) => {
                ui.data_mut(|data| data.remove::<Arc<Report>>(key));
                // Never echo backend source locators or provider payloads.
                app.ui.status = if error == stale() { error } else { tl!("Resource details could not be inspected. Finish pending edits and try again.").to_owned() };
                app.ui.status_error = true;
            }
        }
    }
    app.ui.xcf_resource_page_draft = Some(cursor);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(app: &PhotocraftApp) -> Target {
        let active = app.session.active().unwrap();
        Target { document: active.doc.id, revision: active.revision, snapshot: active.doc.downgrade() }
    }
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap(); app
    }
    fn row(id: u64) -> Value {
        json!({"id":id.to_string(),"name":"Own resource","kind":"linked-image","status":"unresolved",
            "activated":false,"ownedByteLength":0,"contentHash":null,
            "locator":"private-locator-must-not-be-view-state","resolution":{"payload":"private-payload"}})
    }
    #[test]
    fn later_page_preserves_resource_order_and_restored_cursor_is_only_view_state() {
        let app = app(); let target = target(&app);
        let report = decode(&json!({"resources":(1..=1000).map(row).collect::<Vec<_>>(),"count":1000}), target.clone()).unwrap();
        let cursor = ResourcePageDraft { document: target.document.0, revision: target.revision, offset: 960, open: true };
        let restored: ResourcePageDraft = serde_json::from_value(serde_json::to_value(&cursor).unwrap()).unwrap();
        let offset = page_offset(&restored, target.document, target.revision, report.rows.len());
        assert_eq!(offset, 960); assert_eq!(report.rows.iter().skip(offset).take(PAGE).map(|row| row.id).collect::<Vec<_>>(), (961..=1000).collect::<Vec<_>>());
        assert_eq!(page_offset(&restored, target.document, target.revision + 1, report.rows.len()), 0);
        let invalid = ResourcePageDraft { offset: 65, ..restored };
        assert_eq!(page_offset(&invalid, target.document, target.revision, report.rows.len()), 0);
    }
    #[test]
    fn display_keeps_identity_hash_but_no_locator_resolution_or_payload_fields() {
        let app = app(); let mut value = row(u64::MAX);
        let hash = "A".repeat(64); value["contentHash"] = json!(hash);
        let report = decode(&json!({"resources":[value],"count":1}), target(&app)).unwrap();
        let display = report.rows.first().unwrap(); assert_eq!(display.id, u64::MAX); assert_eq!(display.hash.as_deref(), Some(hash.as_str()));
        assert_eq!(display.name, "Own resource"); assert_eq!(display.status, "Unavailable"); assert_eq!(display.activated, Some(false));
        // The typed display Row has no locator, resolution, payload or action.
    }
    #[test]
    fn count_duplicate_id_shape_and_excessive_output_refuse_without_partial_page() {
        let app = app();
        assert!(decode(&json!({"resources":[row(1),row(1)],"count":2}), target(&app)).is_err());
        assert!(decode(&json!({"resources":[row(1)],"count":2}), target(&app)).is_err());
        assert!(decode(&json!({"resources":(1..=1001).map(row).collect::<Vec<_>>(),"count":1001}), target(&app)).is_err());
        let mut malformed = row(1); malformed["id"] = json!("01");
        assert!(decode(&json!({"resources":[malformed],"count":1}), target(&app)).is_err());
        let empty = decode(&json!({"resources":[],"count":0}), target(&app)).unwrap(); assert!(empty.rows.is_empty());
    }
    #[test]
    fn explicit_registered_inspection_keeps_native_document_and_inert_locator_unchanged() {
        use photocraft_doc::{XcfState, item_identity::ResourceId, xcf_resources::{ResourceKind, ResourceReference}};
        let mut app = app();
        let doc = app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let mut state = XcfState::default(); state.reconcile_layers(doc).unwrap();
        state.resources.push(ResourceReference { id: ResourceId(77), kind: ResourceKind::LinkedImage,
            name: "Own dependency".into(), collection: None, internal: false,
            locator: Some("../private-never-opened.png".into()), content_hash: None, owned_bytes: None, resolution: None });
        doc.xcf_state = Some(state); doc.validate_xcf_state().unwrap();
        let original = target(&app); let before = app.session.active().unwrap().doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let history = app.session.active().unwrap().history.past_len();
        let report = inspect(&mut app, original.clone()).unwrap();
        assert_eq!(report.rows.len(), 1); assert_eq!(report.rows.first().unwrap().id, 77);
        assert!(stable(&app, &original)); assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap(), native);
    }
    #[test]
    fn old_report_is_not_reused_after_edit_undo_or_active_document_switch() {
        let mut app = app(); let original = target(&app);
        app.session.edit("Own revision", |document, _| { document.name = "Own edited".into(); Ok(()) }).unwrap();
        app.session.execute("edit.undo", json!({})).unwrap(); assert!(!stable(&app, &original));
        let after_undo = target(&app); app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        assert!(!stable(&app, &after_undo));
    }
}
