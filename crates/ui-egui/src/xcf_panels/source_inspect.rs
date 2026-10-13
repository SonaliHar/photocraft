//! On-demand typed source pages. Serialized cursors carry no query authority.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{DocId, LayerId, item_identity::ItemRef};
use photocraft_engine::document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot};
use serde_json::{Value, json};
use std::{io::Write, sync::Arc};

const PAGE: u32 = 32;
const MAX_OFFSET: u32 = 100_000;
const MAX_DISPLAY: usize = 128 * 1024;
const STALE: &str = "xcf.source.ui.stale-target";
const INVALID: &str = "xcf.source.ui.invalid-page";
#[derive(Clone)]
struct Target {
    document: DocId,
    revision: u64,
    layer: LayerId,
    snapshot: WeakDocumentSnapshot,
}
#[derive(Clone)]
struct DisplayPage {
    target: Target,
    offset: u32,
    more: bool,
    text: String,
}
fn stable(app: &PhotocraftApp, target: &Target) -> bool {
    app.session.active().is_some_and(|state| {
        state.doc.id == target.document
            && state.revision == target.revision
            && target.snapshot.upgrade().is_some_and(|snapshot| snapshot.shares_state(&state.doc))
    })
}
/// Borrowed, bounded eligibility only: no source strings, payloads or locators copied.
fn has_source(snapshot: &DocumentSnapshot, layer: LayerId) -> bool {
    layer.0 != 0
        && snapshot.xcf_state.as_ref().is_some_and(|state| {
            state.source_items.len() <= 10_000 && state.source_items.iter().filter(|source| source.owner == ItemRef::Layer(layer)).count() == 1
        })
}
struct DisplayWriter(String);
impl Write for DisplayWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let size = self.0.len().checked_add(bytes.len()).filter(|size| *size <= MAX_DISPLAY).ok_or_else(|| std::io::Error::other(INVALID))?;
        let text = std::str::from_utf8(bytes).map_err(std::io::Error::other)?;
        self.0.try_reserve(size - self.0.len()).map_err(std::io::Error::other)?;
        self.0.push_str(text);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn decode(value: &Value, target: Target, offset: u32) -> Result<DisplayPage, String> {
    let owner = value.get("owner").ok_or(INVALID)?;
    if owner.get("kind").and_then(Value::as_str) != Some("layer")
        || owner.get("id").and_then(Value::as_str) != Some(target.layer.0.to_string().as_str())
        || value.get("accessGranted").and_then(Value::as_bool) != Some(false)
        || value.get("qualification").and_then(Value::as_bool) != Some(false)
    {
        return Err(INVALID.into());
    }
    let page = value.get("configuration").ok_or(INVALID)?;
    if page.get("offset").and_then(Value::as_u64) != Some(u64::from(offset))
        || page.get("limit").and_then(Value::as_u64) != Some(u64::from(PAGE))
        || page.get("total").and_then(Value::as_u64).is_none()
        || page.get("entries").and_then(Value::as_array).is_none_or(|rows| rows.len() > PAGE as usize)
        || value.get("source").and_then(|source| source.get("kind")).and_then(Value::as_str).is_none_or(|kind| !matches!(kind, "text" | "vector" | "linked"))
    {
        return Err(INVALID.into());
    }
    let more = page.get("truncated").and_then(Value::as_bool).ok_or(INVALID)?;
    // The engine has already encoded exact integer strings and fixed-width float
    // bits. JSON escapes preserve every string/control character without lossy
    // sanitization, numeric parsing, or reconstructing source values in the UI.
    let mut writer = DisplayWriter(String::new());
    serde_json::to_writer_pretty(&mut writer, value).map_err(|error| error.to_string())?;
    Ok(DisplayPage { target, offset, more, text: writer.0 })
}
fn inspect(app: &mut PhotocraftApp, target: Target, offset: u32) -> Result<DisplayPage, String> {
    if offset > MAX_OFFSET {
        return Err(INVALID.into());
    }
    if !stable(app, &target) {
        return Err(STALE.into());
    }
    let state = app.session.active().ok_or(STALE)?;
    if !has_source(&state.doc, target.layer) {
        return Err(INVALID.into());
    }
    let value = app.run(
        "xcf.source.inspect",
        json!({
            "owner": {"kind": "layer", "id": target.layer.0.to_string()}, "offset": offset, "limit": PAGE
        }),
    )?;
    if !stable(app, &target) {
        return Err(STALE.into());
    }
    decode(&value, target, offset)
}
pub(super) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some(state) = app.session.active() else {
        return;
    };
    if !has_source(&state.doc, layer) {
        return;
    }
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let key = egui::Id::new("xcf_source_inspection_page");
    let cached = ui.data(|data| data.get_temp::<Arc<DisplayPage>>(key)).filter(|page| page.target.layer == layer && stable(app, &page.target));
    let cursor = app
        .ui
        .xcf_source_page_draft
        .as_ref()
        .filter(|draft| draft.document == snapshot.id.0 && draft.revision == revision && draft.layer == layer.0 && draft.offset <= MAX_OFFSET);
    let offset = cached.as_ref().map(|page| page.offset).or_else(|| cursor.map(|cursor| cursor.offset)).unwrap_or(0);
    ui.collapsing(tl!("Source"), |ui| {
        ui.label(egui::RichText::new(tl!("Metadata")).color(Tokens::get(ui.ctx()).text));
        ui.label(tl!("Retained source metadata only; inspection does not grant file access or qualify rendering."));
        let mut query = None;
        ui.horizontal(|ui| {
            if ui.button(tl!("Inspect")).clicked() {
                query = Some(offset);
            }
            if ui.add_enabled(offset >= PAGE, egui::Button::new(tl!("Previous"))).clicked() {
                query = Some(offset.saturating_sub(PAGE));
            }
            if ui.add_enabled(cached.as_ref().is_some_and(|page| page.more) && offset <= MAX_OFFSET - PAGE, egui::Button::new(tl!("Next"))).clicked() {
                query = offset.checked_add(PAGE);
            }
        });
        if let Some(page) = &cached {
            egui::ScrollArea::both().max_height(360.0).id_salt("source_metadata_rows").show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&page.text).monospace()).selectable(true));
            });
        }
        if let Some(offset) = query {
            let target = Target { document: snapshot.id, revision, layer, snapshot: snapshot.downgrade() };
            match inspect(app, target, offset) {
                Ok(page) => {
                    app.ui.xcf_source_page_draft = Some(crate::state::XcfSourcePageDraft { document: snapshot.id.0, revision, layer: layer.0, offset });
                    ui.data_mut(|data| data.insert_temp(key, Arc::new(page)));
                }
                Err(error) => {
                    app.ui.status = error;
                    app.ui.status_error = true;
                    ui.data_mut(|data| data.remove::<Arc<DisplayPage>>(key));
                }
            }
        }
    });
}
