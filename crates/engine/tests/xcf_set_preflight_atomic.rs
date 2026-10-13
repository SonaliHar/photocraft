use photocraft_engine::Session;
use serde_json::json;

#[test]
fn saved_set_refusals_preserve_stale_preview_document_and_redo() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":3,"height":2,"background":"white"})).unwrap();
    session.execute("select.all", json!({})).unwrap();
    session.edit("Saved-set redo", |doc, _| { doc.name = "retained redo".into(); Ok(()) }).unwrap();
    assert!(session.undo());
    session.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
    session.active_mut().unwrap().floating.as_mut().unwrap().revision = u64::MAX;
    let state = session.active().unwrap();
    let before = state.doc.clone();
    let preview = state.floating.clone().unwrap();
    let revision = state.revision;
    for (command, params) in [
        ("xcf.itemSet.rename", json!({"setId":"1","name":"new name"})),
        ("xcf.itemSet.delete", json!({"setId":"1"})),
        ("xcf.itemSet.setMembers", json!({"setId":"1","members":[]})),
        ("xcf.itemSet.setMembers", json!({"setId":"1","members":[{"kind":"layer","id":"01"}]})),
    ] {
        assert!(session.execute(command, params).is_err());
        let state = session.active().unwrap();
        assert_eq!(state.doc, before, "{command}");
        assert_eq!(state.revision, revision, "{command}");
        let retained = state.floating.as_ref().unwrap();
        assert_eq!(retained.revision, preview.revision, "{command}");
        assert!(std::sync::Arc::ptr_eq(&retained.parts, &preview.parts), "{command}");
        assert!(state.history.can_redo(), "{command}");
    }
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.name, "retained redo");
}
