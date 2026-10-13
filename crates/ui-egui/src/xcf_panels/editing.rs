//! Thin controls for retained editing relationships, separate from shell focus.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{DocId, LayerId, xcf_editing::FloatingAttachment};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::json;

const DETACH: &str = "xcf.floating.detach";
const ANCHOR: &str = "xcf.floating.anchor";
const TO_LAYER:&str="xcf.floating.toLayer";
const DELETE:&str="xcf.floating.delete";
const PREVIEW: &str = "Complete or cancel the pixel-move preview before detaching the saved relationship.";
struct Target {
    document: DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    attachment: FloatingAttachment,
}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let state=app.session.active().ok_or_else(||tl!("Open a document before editing a floating attachment.").to_owned())?;
    if state.floating.is_some() {return Err(tl!(PREVIEW).into());}
    if app.ui.transform.is_some() || app.session.has_jobs() || state.coalesce.is_some() {
        return Err(tl!("Finish pending edits before editing a floating attachment.").into());
    }
    Ok(())
}
fn capture(app: &PhotocraftApp, layer: LayerId) -> Result<Target, String> {
    idle(app)?;
    let state = app.session.active().ok_or("xcf.floating.detach.stale-target")?;
    // Any retained preview is refused, including one whose revision is stale.
    // Calling app.run would otherwise drop or clear it before this command.
    if state.floating.is_some() {
        return Err(PREVIEW.into());
    }
    let imported = state.doc.xcf_state.as_ref().ok_or("xcf.floating.detach.stale-target")?;
    if imported.items.len() > 10_000 {
        return Err("xcf.floating.detach.limit".into());
    }
    let attachment = imported.floating.as_ref().filter(|value| value.layer == layer).ok_or("xcf.floating.detach.stale-target")?;
    Ok(Target { document: state.doc.id, revision: state.revision, snapshot: state.doc.downgrade(), attachment: attachment.clone() })
}
fn validate(app:&PhotocraftApp,target:&Target)->Result<(),String> {
    idle(app)?;
    let state = app.session.active().ok_or("xcf.floating.detach.stale-target")?;
    // Any retained preview is refused, including one whose revision is stale.
    // Calling app.run would otherwise drop or clear it before this command.
    if state.floating.is_some() {
        return Err(PREVIEW.into());
    }
    let snapshot = target.snapshot.upgrade().ok_or("xcf.floating.detach.stale-target")?;
    if state.doc.id != target.document
        || state.revision != target.revision
        || !snapshot.shares_state(&state.doc)
        || state.doc.xcf_state.as_ref().and_then(|value| value.floating.as_ref()) != Some(&target.attachment)
    {
        return Err("xcf.floating.detach.stale-target".into());
    }
    Ok(())
}
fn detach(app:&mut PhotocraftApp,target:&Target)->Result<serde_json::Value,String> {
    validate(app,target)?;
    // Detach preserves its existing no-pixel-edit semantics and parameter contract.
    app.run(DETACH,json!({}))
}
fn controlled(app:&mut PhotocraftApp,target:&Target,id:&str)->Result<serde_json::Value,String> {
    if !matches!(id,ANCHOR|TO_LAYER|DELETE) {return Err(tl!("This floating attachment action is unavailable.").into());}
    validate(app,target)?;
    let command=photocraft_engine::commands::find(id).ok_or_else(||tl!("This floating attachment action is unavailable.").to_owned())?;
    (command.enabled)(&app.session)?;
    app.run(id,json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string()}))
}

pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let target = app.session.active().and_then(|state| {
        let imported = state.doc.xcf_state.as_ref()?;
        if imported.items.len() > 10_000 {
            return None;
        }
        imported.floating.as_ref().filter(|value| value.layer == layer).map(|value| value.target)
    });
    let Some(target) = target else { return };
    let tokens = Tokens::get(ui.ctx());
    ui.separator();
    ui.label(egui::RichText::new(tl!("Saved floating attachment")).color(tokens.text));
    ui.horizontal_wrapped(|ui| {
        ui.label(tl!("Attachment target"));
        // Fixed family and decimal typed ID only; no imported label is copied.
        ui.monospace(format!("{:?}:{}", target.kind(), target.raw()));
    });
    ui.label(tl!("Detach the saved relationship without changing pixels."));
    let preview = app.session.active().is_some_and(|state| state.floating.is_some());
    if preview {
        ui.label(tl!("Complete or cancel the pixel-move preview before detaching the saved relationship."));
    }
    let captured=capture(app,layer);
    let enabled = captured.is_ok() && photocraft_engine::commands::find(DETACH).is_some_and(|command| (command.enabled)(&app.session).is_ok());
    let detach_button=ui.add_enabled(enabled, egui::Button::new(tl!("Detach Floating Layer")));
    if let Err(error)=&captured {detach_button.clone().on_disabled_hover_text(error);}
    if detach_button.clicked() {
        let result = captured.as_ref().map_err(Clone::clone).and_then(|target| detach(app,target));
        if let Err(error) = result {app.ui.status = error;app.ui.status_error = true;}
    }
    for (id,label,description) in [
        (ANCHOR,tl!("Anchor Floating Layer"),tl!("Anchor the floating pixels onto their saved target.")),
        (TO_LAYER,tl!("To Layer"),tl!("Keep the floating pixels as a separate layer.")),
        (DELETE,tl!("Delete Floating Layer"),tl!("Delete the floating layer without changing its saved target.")),
    ] {
        let Some(command)=photocraft_engine::commands::find(id) else {continue};
        ui.label(description);
        let permission=captured.as_ref().map_err(Clone::clone).and_then(|_|(command.enabled)(&app.session));
        let button=ui.add_enabled(permission.is_ok(),egui::Button::new(label));
        if let Err(error)=&permission {button.clone().on_disabled_hover_text(error);}
        if button.clicked() {
            let result=captured.as_ref().map_err(Clone::clone).and_then(|target|controlled(app,target,id));
            if let Err(error)=result {app.ui.status=error;app.ui.status_error=true;}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use egui_kittest::{Harness, kittest::Queryable};
    use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size, XcfState, item_identity::ItemRef};
    fn fixture() -> (PhotocraftApp, LayerId, LayerId) {
        let mut document = Document::new("Own floating relationship", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let target = Layer::raster("Own target", document.pixel_format());
        let mut floating = Layer::raster("Own floating", document.pixel_format());
        floating.surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.25, 0.5, 0.75, 0.5]).unwrap();
        let source = floating.id;
        let owner = target.id;
        document.layers = vec![target, floating];
        let mut imported = XcfState::default();
        imported.reconcile_layers(&document).unwrap();
        imported.floating = Some(FloatingAttachment { layer: source, target: ItemRef::Layer(owner) });
        imported.selection.selected = vec![ItemRef::Layer(owner)];
        imported.selection.active = Some(ItemRef::Layer(owner));
        document.xcf_state = Some(imported);
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        session.select_layer(source).unwrap();
        (PhotocraftApp::new(session, Services::default()), source, owner)
    }
    #[test]
    fn anchor_and_detach_refuse_new_coalescing_context_before_dispatch() {
        let (mut app,source,_)=fixture();
        let target=capture(&app,source).unwrap();
        let before=app.session.active().unwrap().doc.clone();
        let journal=app.session.journal.clone();
        app.session.active_mut().unwrap().coalesce=Some("Own unfinished gesture".into());
        let error=tl!("Finish pending edits before editing a floating attachment.");
        assert_eq!(controlled(&mut app,&target,ANCHOR).unwrap_err(),error);
        assert_eq!(controlled(&mut app,&target,TO_LAYER).unwrap_err(),error);
        assert_eq!(controlled(&mut app,&target,DELETE).unwrap_err(),error);
        assert_eq!(detach(&mut app,&target).unwrap_err(),error);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.journal,journal);
        assert!(app.session.active().unwrap().coalesce.is_some());
        app.session.active_mut().unwrap().coalesce=None;
        app.session.active_mut().unwrap().revision+=1;
        assert!(controlled(&mut app,&target,ANCHOR).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn widget_detaches_only_saved_relationship_and_preserves_semantic_selection_and_focus() {
        let (app, source, owner) = fixture();
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.entry_labels().count();
        let mut h = Harness::builder().with_size(egui::vec2(440., 260.)).build_ui_state(move |ui, app| properties(app, ui, source), app);
        h.run();
        assert_eq!(*h.state().session.active().unwrap().doc, *before);
        h.get_by_label("Detach Floating Layer").click();
        h.run();
        assert!(!h.state().ui.status_error);
        let after = h.state().session.active().unwrap().doc.clone();
        let mut expected = (*before).clone();
        expected.xcf_state.as_mut().unwrap().floating = None;
        assert_eq!(*after, expected);
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(source));
        assert_eq!(after.xcf_state.as_ref().unwrap().selection.active, Some(ItemRef::Layer(owner)));
        assert_eq!(h.state().session.active().unwrap().history.entry_labels().count(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*h.state().session.active().unwrap().doc, *before);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(*h.state().session.active().unwrap().doc, *after);
    }
    #[test]
    fn stale_or_closed_target_refuses_without_editing_another_relationship() {
        let (mut app, source, _) = fixture();
        let target = capture(&app, source).unwrap();
        app.session
            .edit("Own metadata edit", |doc, _| {
                doc.metadata.xmp = Some("own".into());
                Ok(())
            })
            .unwrap();
        let changed = app.session.active().unwrap().doc.clone();
        assert!(detach(&mut app, &target).is_err());
        assert_eq!(*app.session.active().unwrap().doc, *changed);
        let (other, _, _) = fixture();
        app.session.open_document((*other.session.active().unwrap().doc).clone(), None).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(detach(&mut app, &target).is_err());
        assert_eq!(*app.session.active().unwrap().doc, *before);
    }
    #[test]
    fn actual_pixel_move_preview_and_stale_preview_refuse_before_any_drop_or_history_change() {
        use photocraft_doc::PixelFormat;
        use photocraft_geom::Rect;
        let (mut app, source, _) = fixture();
        app.session
            .edit("Own pixel selection", |doc, _| {
                doc.selection = Some(
                    photocraft_raster::Surface::try_from_interleaved(
                        PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false),
                        Rect::new(0, 0, 2, 2),
                        &[255; 4],
                    )
                    .unwrap(),
                );
                Ok(())
            })
            .unwrap();
        let target = capture(&app, source).unwrap();
        app.session.execute("select.float", json!({"dx":1,"dy":0})).unwrap();
        for stale in [false, true] {
            if stale {
                app.session.active_mut().unwrap().floating.as_mut().unwrap().revision = 0;
            }
            let before = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.entry_labels().count();
            let preview = app.session.active().unwrap().floating.as_ref().unwrap().clone();
            assert!(capture(&app, source).is_err());
            assert_eq!(detach(&mut app, &target).unwrap_err(), PREVIEW);
            let state = app.session.active().unwrap();
            assert_eq!(*state.doc, *before);
            assert_eq!(state.history.entry_labels().count(), history);
            let retained = state.floating.as_ref().unwrap();
            assert_eq!(retained.layer, preview.layer);
            assert_eq!(retained.offset, preview.offset);
            assert_eq!(retained.revision, preview.revision);
            assert!(std::sync::Arc::ptr_eq(&retained.parts, &preview.parts));
        }
    }

    #[test]
    fn locked_floating_source_has_no_mutation_through_registered_dispatch() {
        let (mut app, source, _) = fixture();
        app.session
            .edit("Own lock", |doc, _| {
                doc.layer_mut(source).unwrap().locks.pixels = true;
                Ok(())
            })
            .unwrap();
        let target = capture(&app, source).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(detach(&mut app, &target).is_err());
        assert_eq!(*app.session.active().unwrap().doc, *before);
    }
}
