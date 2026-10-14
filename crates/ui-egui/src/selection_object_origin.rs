//! Runtime-only Object/Quick Selection gesture witness. Geometry, modifier modes and
//! Object Finder previews remain upstream-owned; no source qualification here.
use crate::{PhotocraftApp, state::Tool};
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::{WeakDocumentSnapshot, channel_cmds::ChannelView};
use serde_json::Value;
#[derive(Clone, Debug)]
pub(crate) struct Origin {
    document: DocId, revision: u64, snapshot: WeakDocumentSnapshot, layer: Option<LayerId>, view: ChannelView,
    shell_mask: bool, sample_all: bool, finder: bool, brush_size: u64, tool: Tool, enhance: bool,
}
fn stale() -> String { tl!("The selection target or settings changed. Draw the selection again.").to_owned() }
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active=app.session.active().ok_or_else(stale)?;
    if active.doc.id.0==0 || active.revision==u64::MAX || app.ui.transform.is_some() || app.session.has_jobs()
        || active.floating.is_some() || active.coalesce.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before making a selection.").to_owned());
    }
    Ok(())
}
fn copy_view(view:&ChannelView)->Result<ChannelView,String> {
    fn copy(values:&[bool],cap:usize)->Result<Vec<bool>,String> {
        if values.len()>cap {return Err(stale());}
        let mut output=Vec::new();output.try_reserve_exact(values.len()).map_err(|_|tl!("Object Selection view storage is unavailable.").to_owned())?;
        output.extend_from_slice(values);Ok(output)
    }
    Ok(ChannelView {target:view.target,color_hidden:copy(&view.color_hidden,16)?,alpha_visible:copy(&view.alpha_visible,10_000)?,
        quick_mask_hidden:view.quick_mask_hidden,layer_mask:view.layer_mask,editing_mask:view.editing_mask})
}
pub(crate) fn capture(app:&PhotocraftApp)->Result<Origin,String> {capture_for(app,app.ui.tool)}
pub(crate) fn capture_for(app:&PhotocraftApp,tool:Tool)->Result<Origin,String> {
    idle(app)?;if !matches!(tool,Tool::ObjectSelection|Tool::QuickSelection) || app.ui.tool!=tool {return Err(stale());}
    let active=app.session.active().ok_or_else(stale)?;
    Ok(Origin {document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade(),layer:active.active_layer,
        view:copy_view(&active.channel_view)?,shell_mask:app.layer_mask_targeted(),sample_all:app.ui.tool_options.sample_all_layers,
        finder:app.ui.tool_options.object_finder,brush_size:app.session.tools.brush.size.to_bits().into(),tool,enhance:app.ui.tool_options.enhance_edge})
}
pub(crate) fn validate(app:&PhotocraftApp,origin:Option<&Origin>)->Result<(),String> {
    let origin=origin.ok_or_else(stale)?;idle(app)?;let active=app.session.active().ok_or_else(stale)?;
    if app.ui.tool!=origin.tool || active.doc.id!=origin.document || active.revision!=origin.revision
        || !origin.snapshot.is_current(&active.doc) || active.active_layer!=origin.layer || active.channel_view!=origin.view
        || app.layer_mask_targeted()!=origin.shell_mask || app.ui.tool_options.sample_all_layers!=origin.sample_all
        || (origin.tool==Tool::ObjectSelection && app.ui.tool_options.object_finder!=origin.finder)
        || (origin.tool==Tool::QuickSelection && app.ui.tool_options.enhance_edge!=origin.enhance)
        || u64::from(app.session.tools.brush.size.to_bits())!=origin.brush_size {return Err(stale());}
    Ok(())
}
/// Registered captured request; callers retain the original geometry/defaults.
pub(crate) fn prepare(app:&PhotocraftApp,origin:Option<&Origin>,params:Value)->Result<Value,String> {
    prepare_for(app,origin,"select.object",params)
}
pub(crate) fn prepare_for(app:&PhotocraftApp,origin:Option<&Origin>,command:&str,mut params:Value)->Result<Value,String> {
    validate(app,origin)?;let origin=origin.ok_or_else(stale)?;
    let keys:&[&str]=match command {
        "select.object" if origin.tool==Tool::ObjectSelection=>&["rect","point","mode","sampleAllLayers"],
        "select.quick"=>&["points","size","mode","sampleAllLayers","enhanceEdge"],
        _=>return Err(stale()),
    };
    let object=params.as_object_mut().filter(|object|object.len()<=keys.len()).ok_or_else(stale)?;
    if object.keys().any(|key|!keys.contains(&key.as_str())) {return Err(stale());}
    object.insert("documentId".into(),Value::String(origin.document.0.to_string()));
    object.insert("expectedRevision".into(),Value::String(origin.revision.to_string()));Ok(params)
}
#[cfg(test)]
mod tests {
    use super::*;use serde_json::json;
    fn app()->PhotocraftApp {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();app.ui.tool=Tool::ObjectSelection;app
    }
    #[test]
    fn rectangle_params_carry_captured_pair_without_changing_geometry_or_mode() {
        let app=app();let origin=capture(&app).unwrap();let input=json!({"rect":[1,2,3,4],"mode":"subtract","sampleAllLayers":false});
        let output=prepare(&app,Some(&origin),input.clone()).unwrap();
        assert_eq!(output["rect"],input["rect"]);assert_eq!(output["mode"],input["mode"]);assert_eq!(output["sampleAllLayers"],input["sampleAllLayers"]);
        assert_eq!(output["documentId"],origin.document.0.to_string());assert_eq!(output["expectedRevision"],origin.revision.to_string());
        assert!(prepare(&app,None,input).is_err());
    }
    #[test]
    fn quick_request_preserves_order_modes_and_settings_then_rejects_changed_enhancement() {
        let mut app=app();app.ui.tool=Tool::QuickSelection;let origin=capture(&app).unwrap();
        let points=json!([[1.25,2.75],[3.5,1.0],[1.25,2.75]]);
        let request=prepare_for(&app,Some(&origin),"select.quick",json!({"points":points,"size":30.,"mode":"subtract","enhanceEdge":false,"sampleAllLayers":false})).unwrap();
        assert_eq!(request["points"],points);assert_eq!(request["mode"],"subtract");assert_eq!(request["documentId"],origin.document.0.to_string());
        app.ui.tool_options.enhance_edge=!origin.enhance;assert!(validate(&app,Some(&origin)).is_err());
    }
    #[test]
    fn stale_quick_release_does_not_copy_or_dispatch_points_on_new_document() {
        let mut app=app();app.ui.tool=Tool::QuickSelection;let origin=capture(&app).unwrap();
        app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();let before=app.session.active().unwrap().doc.clone();let journal=app.session.journal.len();
        crate::retouch_ui::finish_quick_selection_captured(&mut app,&[[1.,1.,1.],[2.,2.,0.5]],egui::Modifiers::NONE,Some(&origin));
        assert!(app.ui.status_error);assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.journal.len(),journal);
    }
    #[test]
    fn object_click_quick_fallback_keeps_omitted_enhancement_default() {
        let app=app();let origin=capture(&app).unwrap();
        let request=prepare_for(&app,Some(&origin),"select.quick",json!({"points":[[1.,2.]],"size":30.,"mode":"add","sampleAllLayers":false})).unwrap();
        assert!(request.get("enhanceEdge").is_none());assert!(request.get("documentId").is_some());
    }
    #[test]
    fn point_click_preserves_coordinates_and_intersect_with_captured_pair() {
        let app=app();let origin=capture(&app).unwrap();
        let request=prepare(&app,Some(&origin),json!({"point":[1.25,2.75],"mode":"intersect","sampleAllLayers":true})).unwrap();
        assert_eq!(request["point"],json!([1.25,2.75]));assert_eq!(request["mode"],"intersect");
        assert_eq!(request["documentId"],origin.document.0.to_string());assert_eq!(request["expectedRevision"],origin.revision.to_string());
    }
    #[test]
    fn full_channel_view_and_actual_gesture_options_are_fenced() {
        let mut app=app();let origin=capture(&app).unwrap();assert!(validate(&app,Some(&origin)).is_ok());
        app.session.active_mut().unwrap().channel_view.alpha_visible.push(true);assert!(validate(&app,Some(&origin)).is_err());
        app.session.active_mut().unwrap().channel_view.alpha_visible.clear();
        app.ui.tool_options.sample_all_layers=!origin.sample_all;assert!(validate(&app,Some(&origin)).is_err());app.ui.tool_options.sample_all_layers=origin.sample_all;
        app.ui.tool_options.object_finder=!origin.finder;assert!(validate(&app,Some(&origin)).is_err());app.ui.tool_options.object_finder=origin.finder;
        app.session.tools.brush.size+=1.0;assert!(validate(&app,Some(&origin)).is_err());
    }
    #[test]
    fn release_on_another_document_or_without_origin_never_selects_current_document() {
        let mut app=app();let origin=capture(&app).unwrap();
        app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();let before=app.session.active().unwrap().doc.clone();let count=app.session.journal.len();
        crate::retouch_ui::finish_object_selection_captured(&mut app,[0.,0.],[3.,3.],egui::Modifiers::NONE,Some(&origin));
        assert!(app.ui.status_error);assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.journal.len(),count);
        crate::retouch_ui::finish_object_selection_captured(&mut app,[1.,1.],[1.,1.],egui::Modifiers::NONE,None);
        assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.journal.len(),count);
    }
    #[test]
    fn edit_undo_tool_switch_and_busy_context_refuse_reusing_a_gesture() {
        let mut app=app();let origin=capture(&app).unwrap();app.ui.tool=Tool::Brush;assert!(validate(&app,Some(&origin)).is_err());app.ui.tool=Tool::ObjectSelection;
        app.session.edit("Own gesture redo",|doc,_|{doc.name="Own changed".into();Ok(())}).unwrap();app.session.execute("edit.undo",json!({})).unwrap();
        assert!(validate(&app,Some(&origin)).is_err());assert!(app.session.active().unwrap().history.can_redo());
        let current=capture(&app).unwrap();app.session.active_mut().unwrap().revision=u64::MAX;assert!(validate(&app,Some(&current)).is_err());
    }
}
