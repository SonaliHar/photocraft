#![cfg(not(target_arch = "wasm32"))]
//! Native compatibility of the fallible command-facing font adapters.
use photocraft_doc::{LayerContent, TextLayer};
use photocraft_engine::{Session, type_cmds, type_extra_cmds};
use serde_json::json;

#[path = "../../../apps/photocraft-web/src/boot_contract_probe.rs"]
mod boot_contract_probe;

#[test]
fn diagnostic_ready_paths_use_real_fonts_and_prepare_the_fixed_psd() {
    let report = boot_contract_probe::after_publication().unwrap();
    let value: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(value["phase"], "after-publication");
    let checks = value["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 9);
    assert!(checks.iter().all(|check| check["passed"] == true));
    assert!(boot_contract_probe::before_publication().unwrap_err().contains("incorrectly ready"));
}

#[test]
fn fallible_refresh_keeps_native_pixels_and_editable_psd_data() {
    for depth in [8, 16, 32] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":128,"height":96,"depth":depth})).unwrap();
        let doc = &session.active().unwrap().doc;
        let mut native = TextLayer { text: "AV mutable text".into(), ..Default::default() };
        let mut fallible = native.clone();
        type_cmds::refresh(doc, &mut native);
        type_cmds::try_refresh(doc, &mut fallible).unwrap();
        assert_eq!(native, fallible);
        assert!(fallible.psd_raw.as_ref().is_some_and(|bytes| !bytes.is_empty()));
        assert!(fallible.cache.as_ref().is_some_and(|surface| !surface.content_bounds().is_empty()));
    }
}

#[test]
fn fallible_font_query_keeps_native_missing_family_result() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":128,"height":96})).unwrap();
    let id = session.execute("type.create", json!({"x":5,"y":40,"text":"Unavailable family"})).unwrap()["layer"].as_u64().unwrap();
    session
        .edit("Set missing font witness", |doc, _| {
            let layer = doc.layer_mut(photocraft_doc::LayerId(id)).unwrap();
            let LayerContent::Text(text) = &mut layer.content else { panic!("type.create must create editable text") };
            text.runs[0].style.font_family = "PhotoCraft definitely absent adapter test family".into();
            Ok(())
        })
        .unwrap();
    let doc = &session.active().unwrap().doc;
    let missing = type_extra_cmds::try_missing_fonts(doc).unwrap();
    assert_eq!(missing, type_extra_cmds::missing_fonts(doc));
    assert_eq!(missing, ["PhotoCraft definitely absent adapter test family"]);
}
