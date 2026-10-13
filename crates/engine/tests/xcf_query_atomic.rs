//! Metadata queries must never publish a stale transient pixel-move preview.
use photocraft_engine::Session;
use serde_json::json;

#[test]
fn query_success_or_refusal_keeps_stale_preview_and_existing_redo() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":3,"height":2,"background":"white"})).unwrap();
    session.execute("select.all", json!({})).unwrap();
    session.edit("Real query redo", |doc, _| { doc.name = "query redo".into(); Ok(()) }).unwrap();
    assert!(session.undo());
    session.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
    session.active_mut().unwrap().floating.as_mut().unwrap().revision = u64::MAX;
    let state = session.active().unwrap();
    let source = state.doc.clone();
    let revision = state.revision;
    let floating = state.floating.clone().unwrap();
    let redo = state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
    let layer = state.active_layer.unwrap();
    for (command, params) in [
        ("xcf.assess", json!({})), ("xcf.inspect", json!({"offset":0,"limit":1})),
        ("xcf.composition.inspect", json!({"layerId":layer.0.to_string()})),
        ("xcf.filter.inspect", json!({"filterId":"1"})),
        ("xcf.resource.inspect", json!(null)),
        ("xcf.inspect", json!({"limit":{"malformed":"must refuse before preview handling"}})),
    ] {
        // Missing imported metadata may refuse. Either result must be read-only.
        let _ = session.execute(command, params);
        let state = session.active().unwrap();
        assert_eq!(state.doc, source, "{command}");
        assert_eq!(state.revision, revision, "{command}");
        assert_eq!(state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo, "{command}");
        let retained = state.floating.as_ref().unwrap();
        assert_eq!(retained.revision, floating.revision, "{command}");
        assert!(std::sync::Arc::ptr_eq(&retained.parts, &floating.parts), "{command}");
    }
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.name, "query redo");
}
