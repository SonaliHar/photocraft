//! Explicit named-set metadata controls; never evaluate saved search expressions.
use crate::{PhotocraftApp, state::XcfItemSetDraft, theme::Tokens};
use photocraft_doc::{
    DocId, Document, LayerId,
    item_identity::ItemRef,
    xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::json;

const CREATE: &str = "xcf.itemSet.create";
const RENAME: &str = "xcf.itemSet.rename";
const DELETE: &str = "xcf.itemSet.delete";
const MEMBERS: &str = "xcf.itemSet.setMembers";
const LIMIT: &str = "xcf.itemSet.ui.limit";
const STALE: &str = "xcf.itemSet.ui.stale-target";
const PREVIEW: &str = "Complete or cancel the pixel-move preview before editing saved item sets.";
struct Row {
    id: u64,
    member_count: usize,
    display: String,
    editor_name: Option<String>,
}
struct View {
    document: DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    rows: Vec<Row>,
    more: bool,
    total: usize,
    page: crate::state::XcfItemSetPage,
}
struct Target {
    document: DocId,
    revision: u64,
    layer: LayerId,
    set: u64,
    snapshot: WeakDocumentSnapshot,
}

fn bounded_state(document: &Document) -> Result<&photocraft_doc::XcfState, String> {
    let state = document.xcf_state.as_ref().ok_or(STALE)?;
    if state.items.len() > 10_000 || state.item_sets.len() > 10_000 {
        return Err(LIMIT.into());
    }
    let mut work = state.items.len().checked_add(state.item_sets.len()).ok_or(LIMIT)?;
    for set in &state.item_sets {
        work = work.checked_add(set.members.len()).filter(|n| *n <= 100_000).ok_or(LIMIT)?;
    }
    Ok(state)
}
fn name_bounded(name: &str) -> bool {
    name.len() <= 1024 && !name.contains('\0') && name.chars().nth(256).is_none()
}

pub(crate) fn has_matching(document: &Document, layer: LayerId) -> bool {
    bounded_state(document).is_ok_and(|state| {
        state
            .item_sets
            .iter()
            .any(|set| set.id.0 != 0 && set.family == ItemFamily::Layer && set.rule == ItemSetRule::Named && set.members.contains(&ItemRef::Layer(layer)))
    })
}
/// First-set creation is available on a canonical current layer even without matching rows.
pub(crate) fn has_creation(document:&Document,layer:LayerId)->bool {
    bounded_state(document).is_ok_and(|state| {
        let mut items=state.items.iter().filter(|item|item.item==ItemRef::Layer(layer));
        layer.0!=0 && items.next().is_some() && items.next().is_none() && document.layer(layer).is_some()
    })
}
fn unused_set(document:&Document)->Result<u64,String> {
    let state=bounded_state(document)?;
    if state.item_sets.len()>=10_000 {return Err(LIMIT.into());}
    let mut used=std::collections::HashSet::new();
    used.try_reserve(state.item_sets.len()).map_err(|_|LIMIT)?;
    for set in &state.item_sets {if set.id.0==0 || !used.insert(set.id.0) {return Err(STALE.into());}}
    let end=u64::try_from(used.len()).ok().and_then(|count|count.checked_add(1)).ok_or(LIMIT)?;
    (1..=end).find(|id|!used.contains(id)).ok_or_else(||LIMIT.into())
}
struct CreateTarget {document:DocId,revision:u64,set:u64,members:Vec<ItemRef>,snapshot:WeakDocumentSnapshot}
fn live_context(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or(STALE)?;
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) { return Err(PREVIEW.into()); }
    if app.ui.transform.is_some() || app.session.has_jobs() { return Err("xcf.itemSet.ui.context".into()); }
    Ok(())
}
fn capture_create(app:&PhotocraftApp,set:u64)->Result<CreateTarget,String> {
    live_context(app)?;
    let active=app.session.active().ok_or(STALE)?;
    let state=bounded_state(&active.doc)?;
    if set==0 || state.item_sets.len()>=10_000 || state.item_sets.iter().any(|item|item.id.0==set) || state.selection.selected.len()>1000 {return Err(LIMIT.into());}
    let mut canonical=std::collections::HashSet::new();
    canonical.try_reserve(state.items.len()).map_err(|_|LIMIT)?;
    for item in &state.items {if item.item.raw()==0 || !canonical.insert(item.item) {return Err(STALE.into());}}
    let mut members=Vec::new();
    members.try_reserve_exact(state.selection.selected.len()).map_err(|_|LIMIT)?;
    let mut seen=std::collections::HashSet::new();
    seen.try_reserve(state.selection.selected.len()).map_err(|_|LIMIT)?;
    for member in &state.selection.selected {
        if let ItemRef::Layer(layer)=member {
            if !seen.insert(*layer) || !canonical.contains(member) || active.doc.layer(*layer).is_none() {return Err(STALE.into());}
            members.push(*member);
        }
    }
    if members.is_empty() {return Err("xcf.itemSet.ui.no-selected-layers".into());}
    Ok(CreateTarget {document:active.doc.id,revision:active.revision,set,members,snapshot:active.doc.downgrade()})
}
fn create(app:&mut PhotocraftApp,target:&CreateTarget,name:&str)->Result<serde_json::Value,String> {
    if !name_bounded(name) || name.trim().is_empty() {return Err("xcf.itemSet.create.draft-limit".into());}
    let active=app.session.active().ok_or(STALE)?;
    let snapshot=target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot) {return Err(STALE.into());}
    let fresh=capture_create(app,target.set)?;
    if fresh.members!=target.members {return Err(STALE.into());}
    let mut members=Vec::new();
    members.try_reserve_exact(target.members.len()).map_err(|_|LIMIT)?;
    for member in &target.members {
        let ItemRef::Layer(layer)=member else {return Err(STALE.into());};
        members.push(json!({"kind":"layer","id":canonical_id(layer.0)?}));
    }
    app.run(CREATE,json!({"setId":canonical_id(target.set)?,"name":name,"family":"layer","members":members}))
}
fn matching_set(document: &Document, layer: LayerId, id: u64) -> Result<&ItemSet, String> {
    if id == 0 || document.layer(layer).is_none() {
        return Err(STALE.into());
    }
    let state = bounded_state(document)?;
    let mut found = state.item_sets.iter().filter(|set| set.id.0 == id);
    let set = found.next().ok_or(STALE)?;
    if found.next().is_some() || set.family != ItemFamily::Layer || set.rule != ItemSetRule::Named || !set.members.contains(&ItemRef::Layer(layer)) {
        return Err(STALE.into());
    }
    Ok(set)
}
const PAGE_SIZE:usize=64;
fn matching(set:&ItemSet,layer:LayerId)->bool {
    set.id.0!=0 && set.family==ItemFamily::Layer && set.rule==ItemSetRule::Named && set.members.contains(&ItemRef::Layer(layer))
}
fn page_offset(page:Option<&crate::state::XcfItemSetPage>,document:DocId,revision:u64,layer:LayerId,count:usize)->usize {
    page.filter(|page|page.document==document.0 && page.revision==revision && page.layer==layer.0
        && page.offset<10_000 && page.offset<count && page.offset%PAGE_SIZE==0).map_or(0,|page|page.offset)
}
fn view(app: &PhotocraftApp, layer: LayerId) -> Result<Option<View>, String> {
    let Some(active) = app.session.active() else { return Ok(None) };
    if active.doc.xcf_state.is_none() {
        return Ok(None);
    }
    let state = bounded_state(&active.doc)?;
    if !has_creation(&active.doc,layer) {return Ok(None);}
    let total=state.item_sets.iter().filter(|set|matching(set,layer)).count();
    let offset=page_offset(app.ui.xcf_item_set_page.as_ref(),active.doc.id,active.revision,layer,total);
    let page=crate::state::XcfItemSetPage {document:active.doc.id.0,revision:active.revision,layer:layer.0,offset};
    let mut rows = Vec::new();
    rows.try_reserve_exact(total.saturating_sub(offset).min(PAGE_SIZE)).map_err(|_| LIMIT)?;
    let more=offset.checked_add(PAGE_SIZE).is_some_and(|next|next<total);
    for set in state.item_sets.iter().filter(|set|matching(set,layer)).skip(offset).take(PAGE_SIZE) {
        let mut display: String = set.name.chars().take(64).map(|c| if c.is_control() { ' ' } else { c }).collect();
        if set.name.chars().nth(64).is_some() {
            display.push('…');
        }
        rows.push(Row { id: set.id.0, member_count: set.members.len(), display, editor_name: name_bounded(&set.name).then(|| set.name.clone()) });
    }
    Ok(Some(View { document: active.doc.id, revision: active.revision, snapshot: active.doc.downgrade(), rows, more, total, page }))
}
fn capture(app: &PhotocraftApp, layer: LayerId, set: u64, document: DocId, revision: u64) -> Result<Target, String> {
    live_context(app)?;
    let active = app.session.active().ok_or(STALE)?;
    if active.doc.id != document || active.revision != revision {
        return Err(STALE.into());
    }
    matching_set(&active.doc, layer, set)?;
    Ok(Target { document, revision, layer, set, snapshot: active.doc.downgrade() })
}
fn validate<'a>(app: &'a PhotocraftApp, target: &Target) -> Result<&'a ItemSet, String> {
    live_context(app)?;
    let active = app.session.active().ok_or(STALE)?;
    let snapshot = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !snapshot.shares_state(&active.doc) {
        return Err(STALE.into());
    }
    matching_set(&active.doc, target.layer, target.set)
}
fn run(app: &mut PhotocraftApp, target: &Target, name: Option<&str>) -> Result<serde_json::Value, String> {
    validate(app, target)?;
    match name {
        Some(name) if !name_bounded(name) || name.trim().is_empty() => Err("xcf.itemSet.rename.draft-limit".into()),
        Some(name) => app.run(RENAME, json!({"documentId":canonical_id(target.document.0)?,"expectedRevision":target.revision.to_string(),"setId":canonical_id(target.set)?,"name":name})),
        None => app.run(DELETE, json!({"documentId":canonical_id(target.document.0)?,"expectedRevision":target.revision.to_string(),"setId":canonical_id(target.set)?})),
    }
}
fn canonical_id(raw: u64) -> Result<String, String> {
    use std::fmt::Write;
    if raw == 0 {
        return Err(STALE.into());
    }
    let mut value = String::new();
    value.try_reserve_exact(20).map_err(|_| LIMIT)?;
    write!(&mut value, "{raw}").map_err(|_| LIMIT)?;
    Ok(value)
}
fn remove_layer(app: &mut PhotocraftApp, target: &Target) -> Result<serde_json::Value, String> {
    let set = validate(app, target)?;
    if set.members.len() > 10_000 {
        return Err(LIMIT.into());
    }
    let remaining = set.members.iter().filter(|member| **member != ItemRef::Layer(target.layer)).count();
    let mut members = Vec::new();
    members.try_reserve_exact(remaining).map_err(|_| LIMIT)?;
    for member in &set.members {
        if *member == ItemRef::Layer(target.layer) {
            continue;
        }
        // A Named Layer-family set never acquires other-family membership here.
        let ItemRef::Layer(layer) = *member else { return Err("xcf.itemSet.ui.member-family".into()) };
        members.push(json!({"kind":"layer","id":canonical_id(layer.0)?}));
    }
    let set_id = canonical_id(target.set)?;
    app.run(MEMBERS, json!({"documentId":canonical_id(target.document.0)?,"expectedRevision":target.revision.to_string(),"setId":set_id,"members":members}))
}

enum Action {
    Begin(u64),
    Apply(u64, String),
    Delete(u64),
    Remove(u64),
    Cancel,
    BeginCreate,
    Create(u64,String),
}

pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let view = match view(app, layer) {
        Ok(Some(view)) => view,
        Ok(None) => {app.ui.xcf_item_set_page=None;return;},
        Err(error) => {
            ui.label(error);
            return;
        }
    };
    // Serialized paging is view state only; row actions still use live typed target witnesses.
    app.ui.xcf_item_set_page=Some(view.page.clone());
    if app.ui.xcf_item_set_draft.as_ref().is_some_and(|draft| {
        draft.document != view.document.0
            || draft.layer != layer.0
            || draft.revision != view.revision
            || !name_bounded(&draft.name)
            || (!view.rows.iter().any(|row| row.id == draft.set_id)
                && app.session.active().is_none_or(|active|bounded_state(&active.doc).is_err()
                    || active.doc.xcf_state.as_ref().is_some_and(|state|state.item_sets.iter().any(|set|set.id.0==draft.set_id))))
    }) {
        app.ui.xcf_item_set_draft = None;
    }
    let preview = app.session.active().is_some_and(|active| active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()));
    let editable_context = live_context(app).is_ok();
    let tokens = Tokens::get(ui.ctx());
    ui.separator();
    ui.label(egui::RichText::new(tl!("Saved named item sets")).color(tokens.text));
    ui.label(tl!("Only sets containing this layer are shown; saved searches are not evaluated."));
    if preview {
        ui.label(tl!("Complete or cancel the pixel-move preview before editing saved item sets."));
    }
    let can_rename = editable_context && photocraft_engine::commands::find(RENAME).is_some_and(|command| (command.enabled)(&app.session).is_ok());
    let can_delete = editable_context && photocraft_engine::commands::find(DELETE).is_some_and(|command| (command.enabled)(&app.session).is_ok());
    let can_members = editable_context && photocraft_engine::commands::find(MEMBERS).is_some_and(|command| (command.enabled)(&app.session).is_ok());
    let mut action = None;
    let creation_id=app.session.active().and_then(|active|unused_set(&active.doc).ok());
    let requested_set=app.ui.xcf_item_set_draft.as_ref().filter(|draft|!view.rows.iter().any(|row|row.id==draft.set_id)).map(|draft|draft.set_id).or(creation_id);
    let creation_target=requested_set.and_then(|id|capture_create(app,id).ok());
    let can_create=editable_context && creation_target.is_some();
    if ui.add_enabled(can_create,egui::Button::new(tl!("Create"))).clicked() {action=Some(Action::BeginCreate);}
    if let Some(draft)=app.ui.xcf_item_set_draft.as_mut().filter(|draft|!view.rows.iter().any(|row|row.id==draft.set_id)) {
        let label=ui.label(tl!("Saved set name"));
        ui.add(egui::TextEdit::singleline(&mut draft.name).char_limit(256).desired_width(ui.available_width())).labelled_by(label.id);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(can_create && name_bounded(&draft.name) && !draft.name.trim().is_empty(),egui::Button::new(tl!("Create"))).clicked() {
                action=Some(Action::Create(draft.set_id,draft.name.clone()));
            }
            if ui.button(tl!("Cancel")).clicked() {action=Some(Action::Cancel);}
        });
    }
    for row in &view.rows {
        ui.push_id(("xcf_named_set", row.id), |ui| {
            ui.label(&row.display);
            ui.monospace(row.id.to_string());
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(can_rename, egui::Button::new(tl!("Rename Saved Item Set…"))).clicked() {
                    action = Some(Action::Begin(row.id));
                }
                if ui.add_enabled(can_delete, egui::Button::new(tl!("Delete Saved Item Set"))).clicked() {
                    action = Some(Action::Delete(row.id));
                }
                if ui.add_enabled(can_members && row.member_count <= 10_000, egui::Button::new(tl!("Remove this layer from set"))).clicked() {
                    action = Some(Action::Remove(row.id));
                }
            });
            if let Some(draft) = app.ui.xcf_item_set_draft.as_mut().filter(|draft| draft.set_id == row.id) {
                if row.editor_name.is_none() {
                    ui.label(tl!("Enter a replacement name for this long saved name."));
                }
                let label = ui.label(tl!("Saved set name"));
                ui.add(egui::TextEdit::singleline(&mut draft.name).char_limit(256).desired_width(ui.available_width())).labelled_by(label.id);
                let changed = row.editor_name.as_deref() != Some(draft.name.as_str());
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            can_rename && changed && name_bounded(&draft.name) && !draft.name.trim().is_empty(),
                            egui::Button::new(tl!("Apply Saved Item Set Name")),
                        )
                        .clicked()
                    {
                        action = Some(Action::Apply(row.id, draft.name.clone()));
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        action = Some(Action::Cancel);
                    }
                });
            }
        });
    }
    if view.total>PAGE_SIZE {
        ui.horizontal(|ui| {
            let previous=view.page.offset.checked_sub(PAGE_SIZE);
            let next=view.page.offset.checked_add(PAGE_SIZE).filter(|offset|view.more && *offset<view.total && *offset<10_000);
            let mut offset=None;
            if ui.add_enabled(previous.is_some(),egui::Button::new(tl!("Previous"))).clicked() {offset=previous;}
            ui.label(format!("{}–{} / {}",view.page.offset+1,view.page.offset+view.rows.len(),view.total));
            if ui.add_enabled(next.is_some(),egui::Button::new(tl!("Next"))).clicked() {offset=next;}
            if let Some(offset)=offset {let mut page=view.page.clone();page.offset=offset;app.ui.xcf_item_set_page=Some(page);}
        });
    }
    let Some(action) = action else { return };
    // Bind the action to the exact snapshot which produced these rows, not a
    // fresh same-revision target captured after rendering the controls.
    if !matches!(action, Action::Cancel) && !app.session.active().is_some_and(|active|
        active.doc.id == view.document && active.revision == view.revision
            && view.snapshot.upgrade().is_some_and(|snapshot| snapshot.shares_state(&active.doc)))
    {
        app.ui.status = STALE.into(); app.ui.status_error = true; return;
    }
    let result = match action {
        Action::BeginCreate=>creation_id.ok_or_else(||LIMIT.into()).and_then(|set|capture_create(app,set)).map(|target| {
            app.ui.xcf_item_set_draft=Some(XcfItemSetDraft {document:target.document.0,layer:layer.0,revision:target.revision,set_id:target.set,name:String::new()});
            json!({"draft":true})
        }),
        Action::Create(set,name)=>creation_target.as_ref().filter(|target|target.set==set).ok_or_else(||STALE.into()).and_then(|target|create(app,target,&name)),
        Action::Cancel => {
            app.ui.xcf_item_set_draft = None;
            Ok(json!({"cancelled":true}))
        }
        Action::Begin(set) => capture(app, layer, set, view.document, view.revision).map(|_| {
            let name = view.rows.iter().find(|row| row.id == set).and_then(|row| row.editor_name.clone()).unwrap_or_default();
            app.ui.xcf_item_set_draft = Some(XcfItemSetDraft { document: view.document.0, layer: layer.0, revision: view.revision, set_id: set, name });
            json!({"draft":true})
        }),
        Action::Apply(set, name) => capture(app, layer, set, view.document, view.revision).and_then(|target| run(app, &target, Some(&name))),
        Action::Delete(set) => capture(app, layer, set, view.document, view.revision).and_then(|target| run(app, &target, None)),
        Action::Remove(set) => capture(app, layer, set, view.document, view.revision).and_then(|target| remove_layer(app, &target)),
    };
    match result {
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
        }
        Ok(value) => {
            if value.get("draft").is_none() {
                app.ui.xcf_item_set_draft = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use photocraft_doc::{Color, ColorMode, SampleType, Size, XcfState, item_identity::ItemSetId};
    fn fixture() -> (PhotocraftApp, LayerId) {
        let mut doc = Document::with_background("Own named sets", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let layer = doc.layers[0].id;
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.item_sets = vec![
            ItemSet { id: ItemSetId(7), name: "Own set".into(), family: ItemFamily::Layer, rule: ItemSetRule::Named, members: vec![ItemRef::Layer(layer)] },
            ItemSet {
                id: ItemSetId(8),
                name: "Own unevaluated search".into(),
                family: ItemFamily::Layer,
                rule: ItemSetRule::Regex(".*".into()),
                members: vec![],
            },
        ];
        state.selection.selected = vec![ItemRef::Layer(layer)];
        state.selection.active = Some(ItemRef::Layer(layer));
        doc.xcf_state = Some(state);
        let mut session = photocraft_engine::Session::new();
        session.open_document(doc, None).unwrap();
        (PhotocraftApp::new(session, Services::default()), layer)
    }
    #[test]
    fn rename_delete_commands_preserve_members_searches_selection_and_undo() {
        let (mut app, layer) = fixture();
        let before = app.session.active().unwrap().doc.clone();
        let current = view(&app, layer).unwrap().unwrap();
        assert_eq!(current.rows.len(), 1);
        let target = capture(&app, layer, 7, current.document, current.revision).unwrap();
        run(&mut app, &target, Some("Own renamed set")).unwrap();
        let renamed = app.session.active().unwrap().doc.clone();
        let mut expected = (*before).clone();
        expected.xcf_state.as_mut().unwrap().item_sets[0].name = "Own renamed set".into();
        assert_eq!(*renamed, expected);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        let active = app.session.active().unwrap();
        let target = capture(&app, layer, 7, active.doc.id, active.revision).unwrap();
        run(&mut app, &target, None).unwrap();
        let mut expected = (*before).clone();
        expected.xcf_state.as_mut().unwrap().item_sets.remove(0);
        assert_eq!(*app.session.active().unwrap().doc, expected);
    }
    #[test]
    fn remove_current_layer_preserves_other_member_order_name_rule_selection_and_undo() {
        let (mut app, layer) = fixture();
        let mut others = Vec::new();
        app.session
            .edit("Own ordered named members", |doc, _| {
                for name in ["Own A", "Own B"] {
                    let other = photocraft_doc::Layer::raster(name, doc.pixel_format());
                    others.push(other.id);
                    doc.layers.push(other);
                }
                let mut state = doc.xcf_state.take().unwrap();
                state.reconcile_layers(doc).unwrap();
                state.item_sets[0].members = vec![ItemRef::Layer(others[1]), ItemRef::Layer(layer), ItemRef::Layer(others[0])];
                doc.xcf_state = Some(state);
                Ok(())
            })
            .unwrap();
        let active = app.session.active().unwrap();
        let before = active.doc.clone();
        let target = capture(&app, layer, 7, active.doc.id, active.revision).unwrap();
        remove_layer(&mut app, &target).unwrap();
        let after = app.session.active().unwrap().doc.clone();
        let mut expected = (*before).clone();
        expected.xcf_state.as_mut().unwrap().item_sets[0].members = vec![ItemRef::Layer(others[1]), ItemRef::Layer(others[0])];
        assert_eq!(*after, expected);
        assert_eq!(after.xcf_state.as_ref().unwrap().selection, before.xcf_state.as_ref().unwrap().selection);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *after);
    }

    #[test]
    fn stale_targets_and_oversized_or_nul_drafts_refuse_without_metadata_changes() {
        let (mut app, layer) = fixture();
        let active = app.session.active().unwrap();
        let target = capture(&app, layer, 7, active.doc.id, active.revision).unwrap();
        let before = active.doc.clone();
        for name in ["x".repeat(1025), "\0".into(), " ".into()] {
            assert!(run(&mut app, &target, Some(&name)).is_err());
            assert_eq!(*app.session.active().unwrap().doc, *before);
        }
        app.session
            .edit("Own revision", |doc, _| {
                doc.metadata.xmp = Some("own".into());
                Ok(())
            })
            .unwrap();
        let after = app.session.active().unwrap().doc.clone();
        assert!(run(&mut app, &target, None).is_err());
        assert_eq!(*app.session.active().unwrap().doc, *after);
    }
    #[test]
    fn matching_rows_are_capped_at_64_and_extra_matching_sets_are_reported() {
        let (mut app, layer) = fixture();
        app.session
            .edit("Own bounded set rows", |doc, _| {
                let state = doc.xcf_state.as_mut().unwrap();
                for id in 100..165 {
                    state.item_sets.push(ItemSet {
                        id: ItemSetId(id),
                        name: format!("Own set {id}"),
                        family: ItemFamily::Layer,
                        rule: ItemSetRule::Named,
                        members: vec![ItemRef::Layer(layer)],
                    });
                }
                Ok(())
            })
            .unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let rows = view(&app, layer).unwrap().unwrap();
        assert_eq!(rows.rows.len(), 64);
        assert!(rows.more);
        app.ui.xcf_item_set_page=Some(crate::state::XcfItemSetPage {document:before.id.0,revision:app.session.active().unwrap().revision,layer:layer.0,offset:64});
        let later=view(&app,layer).unwrap().unwrap();
        let expected=before.xcf_state.as_ref().unwrap().item_sets.iter().filter(|set|matching(set,layer)).skip(64).map(|set|set.id.0).collect::<Vec<_>>();
        assert_eq!(later.rows.iter().map(|row|row.id).collect::<Vec<_>>(),expected);
        assert!(!later.more);assert!(later.rows.len()<=PAGE_SIZE);
        let target=capture(&app,layer,later.rows[0].id,later.document,later.revision).unwrap();
        app.session.active_mut().unwrap().revision+=1;
        assert!(run(&mut app,&target,None).is_err());
        assert_eq!(view(&app,layer).unwrap().unwrap().page.offset,0);
        assert_eq!(*app.session.active().unwrap().doc, *before);
    }

    #[test]
    fn names_do_not_truncate_into_rename_defaults_and_search_rules_are_never_matched() {
        let (mut app, layer) = fixture();
        app.session
            .edit("Own long retained name", |doc, _| {
                doc.xcf_state.as_mut().unwrap().item_sets[0].name = "x".repeat(2000);
                Ok(())
            })
            .unwrap();
        let rows = view(&app, layer).unwrap().unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert!(rows.rows[0].editor_name.is_none());
        assert_eq!(rows.rows[0].display.chars().count(), 65);
    }
    #[test]
    fn first_named_set_creation_uses_semantic_layers_preserves_source_and_undo() {
        let (mut app,layer)=fixture();
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().item_sets.clear();
        assert!(has_creation(&app.session.active().unwrap().doc,layer));
        assert!(view(&app,layer).unwrap().unwrap().rows.is_empty());
        let before=app.session.active().unwrap().doc.clone();
        let id=unused_set(&before).unwrap();
        let target=capture_create(&app,id).unwrap();
        create(&mut app,&target,"Own first named set").unwrap();
        let mut expected=before.as_ref().clone();
        expected.xcf_state.as_mut().unwrap().item_sets.push(ItemSet {id:ItemSetId(id),name:"Own first named set".into(),family:ItemFamily::Layer,rule:ItemSetRule::Named,members:vec![ItemRef::Layer(layer)]});
        assert_eq!(app.session.active().unwrap().doc.as_ref(),&expected);
        app.session.execute("edit.undo",json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc,before);
    }
    #[test]
    fn set_creation_stale_target_and_name_refuse_without_mutation() {
        let (mut app,_)=fixture();
        let id=unused_set(&app.session.active().unwrap().doc).unwrap();
        let target=capture_create(&app,id).unwrap();
        app.session.active_mut().unwrap().revision+=1;
        let before=app.session.active().unwrap().doc.clone();
        assert_eq!(create(&mut app,&target,"Own stale set").unwrap_err(),STALE);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let target=capture_create(&app,id).unwrap();
        for name in ["", "\0", " "] {assert!(create(&mut app,&target,name).is_err());}
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }

    #[test]
    fn frame_witness_rejects_same_revision_snapshot_replacement() {
        let (mut app, layer) = fixture();
        let frame = view(&app, layer).unwrap().unwrap();
        let target = capture(&app, layer, 7, frame.document, frame.revision).unwrap();
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().name = "Own replaced snapshot".into();
        let before = app.session.active().unwrap().doc.clone();
        assert!(frame.snapshot.upgrade().is_none_or(|old| !old.shares_state(&before)));
        assert_eq!(run(&mut app, &target, Some("Own refused rename")).unwrap_err(), STALE);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.active().unwrap().history.past_len(), 0);
    }
    #[test]
    fn canonical_set_selector_keeps_full_width_ids_and_names_bounded() {
        assert_eq!(canonical_id(u64::MAX).unwrap(), "18446744073709551615");
        assert!(canonical_id(0).is_err());
        assert!(name_bounded(&"Ω".repeat(256)));
        assert!(!name_bounded(&"Ω".repeat(257)));
        assert!(!name_bounded("Own\0name"));
    }

    #[test]
    fn page_cursor_resets_wrong_identity_revision_and_invalid_matching_offsets() {
        let page=crate::state::XcfItemSetPage {document:7,revision:3,layer:9,offset:64};
        assert_eq!(page_offset(Some(&page),DocId(7),3,LayerId(9),65),64);
        for offset in [1,65,10_000,usize::MAX] {
            let mut bad=page.clone();bad.offset=offset;
            assert_eq!(page_offset(Some(&bad),DocId(7),3,LayerId(9),10_000),0);
        }
        assert_eq!(page_offset(Some(&page),DocId(8),3,LayerId(9),100),0);
        assert_eq!(page_offset(Some(&page),DocId(7),4,LayerId(9),100),0);
        assert_eq!(page_offset(Some(&page),DocId(7),3,LayerId(10),100),0);
        assert_eq!(page_offset(Some(&page),DocId(7),3,LayerId(9),64),0);
    }

}
