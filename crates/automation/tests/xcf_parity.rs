#[path = "../../io/tests/common/xcf.rs"]
mod common;
use photocraft_automation::{AuthorizedWorkspace, Headless};
use serde_json::json;
#[test]
fn matched_preserve_policy_agrees_on_samples_and_material_diagnostics() {
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = common::image(12, 550, 0, 1, 1, &[0x38, 0, 0x34, 0, 0x30, 0, 0x3c, 0]);
    std::fs::write(dir.join("half.xcf"), &bytes).unwrap();
    let imported = photocraft_io::import("half.xcf", &bytes).unwrap();
    let warnings = imported.warnings.clone();
    let mut interactive = photocraft_engine::Session::new();
    interactive.execute("edit.colorSettings", json!({"policyRgb":"preserve","askOnMissing":false})).unwrap();
    interactive.open_document(imported.document, None).unwrap();
    let mut headless = Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir), Some(&dir)).unwrap());
    let opened = headless.open(std::path::Path::new("half.xcf")).unwrap();
    assert_eq!(opened["warnings"], json!(warnings));
    let a = &interactive.active().unwrap().doc;
    let b = &headless.session.active().unwrap().doc;
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.layers[0].name, b.layers[0].name);
    assert_eq!(a.layers[0].content, b.layers[0].content);
    assert_eq!(photocraft_compose::render(a, a.bounds()).unwrap().px, photocraft_compose::render(b, b.bounds()).unwrap().px);
    assert!(headless.command_run("edit.colorSettings", json!({"workingRgb":"/outside/custom.icc"})).is_err());
    assert!(headless.command_run("file.open", json!({"path":"half.xcf"})).is_err());
    drop(headless);
    drop(interactive);
    std::fs::remove_dir_all(dir).unwrap();
}
