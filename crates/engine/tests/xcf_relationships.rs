//! Persisted imported relationships are independent of transient pixel-move previews.
use photocraft_doc::item_identity::{ItemRef, ItemSetId};
use photocraft_doc::xcf_editing::{FloatingAttachment, ItemFamily, ItemSet, ItemSetRule};
use photocraft_doc::{Color, ColorMode, Document, Layer, SampleType, Size, XcfState};
use photocraft_engine::Session;
use serde_json::json;

fn original() -> Document {
    let mut doc = Document::with_background("Own relationships", Size::new(2, 2), ColorMode::Rgb, SampleType::U16, Color::WHITE);
    let target = doc.layers[0].id;
    let mut layer = Layer::raster("Own floating", doc.pixel_format());
    let own_bits = [17_u16, 32001, 65521, 127].into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>();
    layer.surface_mut().unwrap().try_write_interleaved(photocraft_doc::Rect::new(0, 1, 1, 2), &own_bits).unwrap();
    let floating = layer.id;
    doc.layers.push(layer);
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.floating = Some(FloatingAttachment { layer: floating, target: ItemRef::Layer(target) });
    state.selection.selected = vec![ItemRef::Layer(floating)];
    state.selection.active = Some(ItemRef::Layer(floating));
    state.item_sets = vec![
        ItemSet {
            id: ItemSetId(701),
            name: "Own named set".into(),
            family: ItemFamily::Layer,
            rule: ItemSetRule::Named,
            members: vec![ItemRef::Layer(target), ItemRef::Layer(floating)],
        },
        ItemSet { id: ItemSetId(702), name: "Own search".into(), family: ItemFamily::Layer, rule: ItemSetRule::Glob("Own *".into()), members: vec![] },
    ];
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    doc
}
fn session(doc: Document) -> Session {
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
fn native(doc: &Document) {
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *doc);
    let fresh = photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    fresh.validate_xcf_state().unwrap();
    let state = fresh.xcf_state.as_ref().unwrap();
    if let Some(attachment) = &doc.xcf_state.as_ref().unwrap().floating {
        let remapped = state.floating.as_ref().unwrap();
        assert_ne!(remapped.layer, attachment.layer);
        assert_eq!(fresh.layer(remapped.layer).unwrap().name, doc.layer(attachment.layer).unwrap().name);
    }
    assert_eq!(
        state.item_sets.iter().map(|set| set.id).collect::<Vec<_>>(),
        doc.xcf_state.as_ref().unwrap().item_sets.iter().map(|set| set.id).collect::<Vec<_>>()
    );
}

#[test]
fn deleting_attachment_target_fails_atomically_while_removing_floating_layer_prunes_refs() {
    let original = original();
    let target = original.layers[0].id;
    let floating = original.layers[1].id;
    let mut session = session(original.clone());
    let revision = session.active().unwrap().revision;
    let history = session.active().unwrap().history.past_len();
    assert!(session.execute("layer.delete", json!({"layer":target.0})).is_err());
    assert_eq!(*session.active().unwrap().doc, original);
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.past_len(), history);
    session.execute("layer.delete", json!({"layer":floating.0})).unwrap();
    let state = session.active().unwrap().doc.xcf_state.as_ref().unwrap();
    assert!(state.floating.is_none());
    assert!(state.selection.selected.is_empty());
    assert!(state.selection.active.is_none());
    assert_eq!(state.item_sets[0].members, vec![ItemRef::Layer(target)]);
    native(&session.active().unwrap().doc);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
}

#[test]
fn layer_rename_keeps_set_and_attachment_ids_through_native_and_undo() {
    let original = original();
    let target = original.layers[0].id;
    let mut session = session(original.clone());
    session.execute("layer.renameLayer", json!({"layer":target.0,"name":"Own renamed Ω"})).unwrap();
    let state = session.active().unwrap().doc.xcf_state.as_ref().unwrap();
    let before = original.xcf_state.as_ref().unwrap();
    assert_eq!(state.floating, before.floating);
    assert_eq!(state.item_sets, before.item_sets);
    native(&session.active().unwrap().doc);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
}

#[test]
fn detach_preserves_pixels_and_registered_ids_and_is_undoable() {
    let original = original();
    let floating = original.layers[1].id;
    let mut session = session(original.clone());
    let result = session.execute("xcf.floating.detach", json!({})).unwrap();
    assert_eq!(result["layer"], floating.0);
    let edited = &session.active().unwrap().doc;
    assert!(edited.xcf_state.as_ref().unwrap().floating.is_none());
    assert_eq!(edited.layers, original.layers);
    assert_eq!(edited.xcf_state.as_ref().unwrap().item_sets, original.xcf_state.as_ref().unwrap().item_sets);
    native(edited);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
    session.execute("edit.redo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().floating.is_none());
}

#[test]
fn named_set_commands_keep_identity_rules_members_native_and_undo() {
    let original = original();
    let target = original.layers[0].id;
    let mut session = session(original.clone());
    session.execute("xcf.itemSet.rename", json!({"setId":701,"name":"Own renamed set Ω"})).unwrap();
    assert_eq!(session.execute("xcf.itemSet.rename", json!({"setId":"701","name":"Own renamed set Ω"})).unwrap()["updated"], false);
    assert_eq!(session.active().unwrap().history.past_len(), 1, "canonical string selects the same saved set without another edit");
    let renamed = (*session.active().unwrap().doc).clone();
    assert_eq!(renamed.xcf_state.as_ref().unwrap().item_sets[0].id, ItemSetId(701));
    assert_eq!(renamed.xcf_state.as_ref().unwrap().item_sets[0].members, original.xcf_state.as_ref().unwrap().item_sets[0].members);
    session.execute("xcf.itemSet.setMembers", json!({"setId":701,"members":[{"Layer":target.0}]})).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().item_sets[0].members, vec![ItemRef::Layer(target)]);
    native(&session.active().unwrap().doc);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, renamed);
    session.execute("xcf.itemSet.delete", json!({"setId":702})).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().item_sets.len(), 1);
    native(&session.active().unwrap().doc);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, renamed);
}

#[test]
fn invalid_relationship_parameters_or_search_members_are_atomic() {
    let original = original();
    for (command, params) in [
        ("xcf.itemSet.rename", json!({"setId":701,"name":""})),
        ("xcf.itemSet.rename", json!({"setId":"0701","name":"Own"})),
        ("xcf.itemSet.delete", json!({"setId":999})),
        ("xcf.itemSet.setMembers", json!({"setId":702,"members":[]})),
        ("xcf.itemSet.setMembers", json!({"setId":701,"members":[{"Layer":999}]})),
        ("xcf.itemSet.setMembers", json!({"setId":701,"members":[{"Channel":999}]})),
        ("xcf.itemSet.setMembers", json!({"setId":701,"members":false})),
    ] {
        let mut session = session(original.clone());
        assert!(session.execute(command, params).is_err(), "{command}");
        assert_eq!(*session.active().unwrap().doc, original);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
}

#[test]
fn inherited_imported_content_lock_blocks_detach_even_when_runtime_layer_locks_are_clear() {
    let mut original = original();
    let floating = original.layers.pop().unwrap();
    let group = Layer::group("Own locked ancestor", vec![floating]);
    let parent = group.id;
    original.layers.push(group);
    let mut state = original.xcf_state.take().unwrap();
    state.reconcile_layers(&original).unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(parent)).unwrap().locks.content = true;
    original.xcf_state = Some(state);
    native(&original);
    let mut session = session(original.clone());
    assert!(!session.is_enabled("xcf.floating.detach"));
    assert!(session.execute("xcf.floating.detach", json!({})).is_err());
    assert_eq!(*session.active().unwrap().doc, original);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    session.execute("layer.setProps", json!({"layer":parent.0,"locks":{"pixels":false}})).unwrap();
    assert!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(parent)).unwrap().locks.content);
    assert!(!session.is_enabled("xcf.floating.detach"));
}

#[test]
fn explicit_ancestor_unlock_updates_current_state_and_preserves_source_flags_native_and_undo() {
    let mut original = original();
    let floating = original.layers.pop().unwrap();
    let mut group = Layer::group("Own locked ancestor", vec![floating]);
    group.locks.pixels = true;
    let parent = group.id;
    original.layers.push(group);
    let mut state = original.xcf_state.take().unwrap();
    state.reconcile_layers(&original).unwrap();
    let item = state.items.iter_mut().find(|item| item.item == ItemRef::Layer(parent)).unwrap();
    item.locks.content = true;
    item.locks.visibility = true;
    item.source_flags = 0x81234567;
    original.xcf_state = Some(state);
    let mut session = session(original.clone());
    assert!(!session.is_enabled("xcf.floating.detach"));
    session.execute("layer.setProps", json!({"layer":parent.0,"locks":{"pixels":false}})).unwrap();
    let edited = &session.active().unwrap().doc;
    let item = edited.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(parent)).unwrap();
    assert!(!item.locks.content);
    assert!(item.locks.visibility);
    assert_eq!(item.source_flags, 0x81234567);
    assert!(session.is_enabled("xcf.floating.detach"));
    native(edited);
    session.execute("xcf.floating.detach", json!({})).unwrap();
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().floating.is_some());
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
    assert!(!session.is_enabled("xcf.floating.detach"));
}

#[test]
fn clearing_all_retains_individual_content_lock_but_clears_changed_position_and_alpha() {
    let mut original = original();
    let target = original.layers[0].id;
    original.layers[0].locks.all = true;
    original.layers[0].locks.pixels = true;
    original.layers[0].locks.position = false;
    original.layers[0].locks.transparency = false;
    let item = original.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(target)).unwrap();
    item.locks.content = true;
    item.locks.position = true;
    item.locks.alpha = true;
    item.locks.visibility = true;
    item.source_flags = 0x10203040;
    let mut session = session(original.clone());
    session.execute("layer.setProps", json!({"layer":target.0,"locks":{"all":false}})).unwrap();
    let edited = &session.active().unwrap().doc;
    let item = edited.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(target)).unwrap();
    assert!(item.locks.content);
    assert!(!item.locks.position);
    assert!(!item.locks.alpha);
    assert!(item.locks.visibility);
    assert_eq!(item.source_flags, 0x10203040);
    native(edited);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
}
