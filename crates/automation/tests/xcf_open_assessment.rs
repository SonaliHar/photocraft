#[path = "../../io/tests/common/xcf.rs"]
mod common;
use photocraft_automation::{AuthorizedWorkspace, Headless};
use serde_json::json;
#[test]
fn rpc_open_transports_optional_success_assessment_from_the_qualified_import() {
    let dir = std::env::temp_dir().join(format!("photocraft-assessment-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = common::image(12, 150, 0, 1, 1, &[19, 39, 79, 255]);
    std::fs::write(dir.join("original.xcf"), &bytes).unwrap();
    let mut expected = photocraft_io::import("original.xcf", &bytes).unwrap();
    let expected_report = serde_json::to_value(expected.assessment.unwrap()).unwrap();
    let mut app = Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir), None).unwrap());
    let reply = app.handle("doc.open", json!({"path":"original.xcf"})).unwrap();
    assert_eq!(reply["assessment"], expected_report);
    assert_eq!(reply["assessment"]["fidelity"], "full");
    // Independent imports allocate fresh runtime layer identities. Bind this one-layer
    // fixture by its typed item identity, then compare every source and document field.
    let actual = &app.session.active().unwrap().doc;
    assert_eq!(actual.layers.len(), 1);
    assert_eq!(expected.document.layers.len(), 1);
    let old = expected.document.layers[0].id;
    let current = actual.layers[0].id;
    let state = expected.document.xcf_state.as_mut().unwrap();
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].item, photocraft_doc::item_identity::ItemRef::Layer(old));
    state.items[0].item = photocraft_doc::item_identity::ItemRef::Layer(current);
    expected.document.layers[0].id = current;
    expected.document.id = actual.id;
    assert_eq!(**actual, expected.document);
    assert_eq!(std::fs::read(dir.join("original.xcf")).unwrap(), bytes);
    let native = photocraft_format::save_to_bytes(&expected.document, &Default::default()).unwrap();
    assert!(photocraft_automation::files::open_bytes("current.pcraft", &native).unwrap().assessment.is_none());
    drop(app);
    std::fs::remove_file(dir.join("original.xcf")).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}
