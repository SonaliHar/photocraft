//! On-demand typed source pages. Serialized cursors carry no query authority.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{DocId, LayerId, item_identity::ItemRef};
use photocraft_engine::document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot};
use photocraft_engine::{channel_cmds::ChannelTarget, mask_view_cmds::LayerMaskView};
use serde_json::{Value, json};
use std::{io::Write, sync::Arc};

const PAGE: u32 = 32;
const MAX_OFFSET: u32 = 100_000;
const MAX_DISPLAY: usize = 128 * 1024;
const STALE: &str = "xcf.source.ui.stale-target";
const INVALID: &str = "xcf.source.ui.invalid-page";
/// Serializable view cursor only. Legacy layer cursors remain readable; neither
/// this owner nor a restored cursor grants query or editing authority.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourcePageDraft {
    pub document: u64,
    pub revision: u64,
    pub layer: u64,
    pub offset: u32,
    pub owner: Option<ItemRef>,
}
fn cursor_matches(draft: &SourcePageDraft, document: DocId, revision: u64, owner: ItemRef) -> bool {
    draft.document == document.0 && draft.revision == revision && draft.offset <= MAX_OFFSET
        && match draft.owner {
            Some(current) => current == owner,
            None => matches!(owner, ItemRef::Layer(layer) if draft.layer == layer.0),
        }
}
#[derive(Clone)]
struct Target {
    document: DocId,
    revision: u64,
    owner: ItemRef,
    view: View,
    snapshot: WeakDocumentSnapshot,
}
/// Scalar presentation selectors only; no payload, names or allocation authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    layer: Option<LayerId>,
    channel: ChannelTarget,
    editing_mask: Option<LayerId>,
    shown_mask: Option<LayerMaskView>,
    shell_mask: bool,
    saved_path: Option<(u64, u64)>,
}
fn view(app: &PhotocraftApp) -> Option<View> {
    let state = app.session.active()?;
    Some(View {
        layer: state.active_layer,
        channel: state.channel_view.target,
        editing_mask: state.channel_view.editing_mask,
        shown_mask: state.channel_view.layer_mask,
        shell_mask: app.layer_mask_targeted(),
        saved_path: app.ui.selected_saved_path.as_ref().map(|selected| (selected.document, selected.path)),
    })
}
fn owner_kind(owner: ItemRef) -> &'static str {
    match owner { ItemRef::Layer(_) => "layer", ItemRef::Channel(_) => "channel", ItemRef::Mask(_) => "mask", ItemRef::Path(_) => "path" }
}
fn request(owner: ItemRef, offset: u32) -> Value {
    json!({"owner": {"kind": owner_kind(owner), "id": owner.raw().to_string()}, "offset": offset, "limit": PAGE})
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
        view(app) == Some(target.view)
            && state.doc.id == target.document
            && state.revision == target.revision
            && target.snapshot.upgrade().is_some_and(|snapshot| snapshot.shares_state(&state.doc))
    })
}
/// Borrowed, bounded eligibility only: no source strings, payloads or locators copied.
fn has_source(snapshot: &DocumentSnapshot, owner: ItemRef) -> bool {
    owner.raw() != 0
        && snapshot.xcf_state.as_ref().is_some_and(|state| {
            state.source_items.len() <= 10_000 && state.source_items.iter().filter(|source| source.owner == owner).count() == 1
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
    if owner.get("kind").and_then(Value::as_str) != Some(owner_kind(target.owner))
        || owner.get("id").and_then(Value::as_str) != Some(target.owner.raw().to_string().as_str())
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
    if !has_source(&state.doc, target.owner) {
        return Err(INVALID.into());
    }
    let value = app.run("xcf.source.inspect", request(target.owner, offset))?;
    if !stable(app, &target) {
        return Err(STALE.into());
    }
    decode(&value, target, offset)
}
/// Existing layer Properties adapter.
pub(super) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    owner_properties(app, ui, ItemRef::Layer(layer));
}
/// Explicit current typed-owner Details. Caller supplies an actual current selector,
/// never a source tattoo or a name-derived identity.
pub(crate) fn owner_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, owner: ItemRef) {
    let Some(state) = app.session.active() else {
        return;
    };
    if !has_source(&state.doc, owner) {
        return;
    }
    let Some(current_view) = view(app) else { return; };
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let key = egui::Id::new("xcf_source_inspection_page");
    let cached = ui.data(|data| data.get_temp::<Arc<DisplayPage>>(key)).filter(|page| page.target.owner == owner && stable(app, &page.target));
    let cursor = app
        .ui
        .xcf_source_page_draft
        .as_ref()
        .filter(|draft| cursor_matches(draft, snapshot.id, revision, owner));
    let offset = cached.as_ref().map(|page| page.offset).or_else(|| cursor.map(|cursor| cursor.offset)).unwrap_or(0);
    ui.push_id((owner_kind(owner), owner.raw()), |ui| {
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
                let target = Target { document: snapshot.id, revision, owner, view: current_view, snapshot: snapshot.downgrade() };
                match inspect(app, target, offset) {
                    Ok(page) => {
                        app.ui.xcf_source_page_draft = Some(SourcePageDraft {
                            document: snapshot.id.0, revision,
                            layer: if let ItemRef::Layer(layer) = owner { layer.0 } else { 0 },
                            offset, owner: Some(owner),
                        });
                        ui.data_mut(|data| data.insert_temp(key, Arc::new(page)));
                    }
                    Err(error) => {
                        app.ui.status = match error.as_str() {
                            STALE => tl!("The source target changed. Select it again before inspecting.").to_owned(),
                            INVALID => tl!("This source metadata page is unavailable.").to_owned(),
                            _ => error,
                        };
                        app.ui.status_error = true;
                        ui.data_mut(|data| data.remove::<Arc<DisplayPage>>(key));
                    }
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::item_identity::{ChannelId, MaskId, PathId};
    #[test]
    fn requests_keep_family_and_full_width_identity() {
        for owner in [ItemRef::Layer(LayerId(u64::MAX)), ItemRef::Channel(ChannelId(u64::MAX)), ItemRef::Mask(MaskId(u64::MAX)), ItemRef::Path(PathId(u64::MAX))] {
            let value = request(owner, 64);
            assert_eq!(value["owner"]["kind"], owner_kind(owner));
            assert_eq!(value["owner"]["id"], u64::MAX.to_string());
            assert_eq!(value["offset"], 64);
            assert_eq!(value["limit"], PAGE);
        }
    }
    #[test]
    fn legacy_layer_cursor_never_becomes_another_family() {
        let mut draft: SourcePageDraft = serde_json::from_value(json!({"document":1,"revision":2,"layer":7,"offset":32})).unwrap();
        assert!(cursor_matches(&draft, DocId(1), 2, ItemRef::Layer(LayerId(7))));
        assert!(!cursor_matches(&draft, DocId(1), 2, ItemRef::Mask(MaskId(7))));
        draft.owner = Some(ItemRef::Path(PathId(7)));
        let restored: SourcePageDraft = serde_json::from_value(serde_json::to_value(&draft).unwrap()).unwrap();
        assert!(cursor_matches(&restored, DocId(1), 2, ItemRef::Path(PathId(7))));
        assert!(!cursor_matches(&restored, DocId(1), 3, ItemRef::Path(PathId(7))));
        draft.offset = MAX_OFFSET + 1;
        assert!(!cursor_matches(&draft, DocId(1), 2, ItemRef::Path(PathId(7))));
    }
    #[test]
    fn changing_view_or_snapshot_invalidates_query_target() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let active = app.session.active().unwrap();
        let target = Target { document: active.doc.id, revision: active.revision,
            owner: ItemRef::Layer(active.active_layer.unwrap()), view: view(&app).unwrap(), snapshot: active.doc.downgrade() };
        assert!(stable(&app, &target));
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Color(0);
        assert!(!stable(&app, &target));
        app.session.active_mut().unwrap().channel_view.target = target.view.channel;
        assert!(stable(&app, &target));
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        assert!(!stable(&app, &target));
    }
    #[test]
    fn decoded_page_refuses_different_typed_owner_and_grant_claims() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let active = app.session.active().unwrap();
        let target = Target { document: active.doc.id, revision: active.revision, owner: ItemRef::Mask(MaskId(7)),
            view: view(&app).unwrap(), snapshot: active.doc.downgrade() };
        let mut value = json!({"owner":{"kind":"mask","id":"7"},"accessGranted":false,"qualification":false,
            "source":{"kind":"text"},"configuration":{"offset":32,"limit":PAGE,"total":33,"entries":[],"truncated":false}});
        assert!(decode(&value, target.clone(), 32).is_ok());
        value["owner"]["kind"] = json!("channel");
        assert!(decode(&value, target.clone(), 32).is_err());
        value["owner"]["kind"] = json!("mask"); value["accessGranted"] = json!(true);
        assert!(decode(&value, target, 32).is_err());
    }
}
