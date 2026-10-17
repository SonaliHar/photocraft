//! XCF behavior through the existing control request/response bridge.
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_ui_egui::{ControlRequest, PhotocraftApp, Services, control};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let (request, _) = ControlRequest::new(method, params);
    match control::handle(app, &egui::Context::default(), &request) {
        control::Outcome::Done(value) => value,
        _ => panic!("expected immediate control reply"),
    }
}

#[test]
fn authorized_control_opens_xcf_and_migrates_without_ambient_access() {
    let bytes = xcf::image(13, 550, 0, 1, 1, &[0x38, 0, 0x34, 0, 0x30, 0, 0x3c, 0]);
    let written = Rc::new(RefCell::new(Vec::<(String, Vec<u8>)>::new()));
    let writer = written.clone();
    let services = Services {
        import: Some(Box::new(|name, bytes, depth| photocraft_io::import_with_svg_group_depth(name, bytes, depth).map_err(|e| e.to_string()))),
        export: Some(Box::new(|doc, name, _| {
            let result = photocraft_io::export(doc, name, &Default::default()).map_err(|e| e.to_string())?;
            Ok((result.bytes, result.warnings))
        })),
        automation_read: Some(Box::new(move |path| {
            if path == "renamed.psd" { Ok((path.to_owned(), bytes.clone())) } else { Err("outside granted read capability".into()) }
        })),
        automation_write: Some(Box::new(move |path, bytes, _protection| {
            writer.borrow_mut().push((path.to_owned(), bytes.to_vec()));
            Ok(())
        })),
        automation_authorize: Some(|id, _| {
            if matches!(id, "file.open" | "file.save") {
                Err(photocraft_engine::EngineError::Other("ambient filesystem command disabled".into()))
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let opened = call(&mut app, "app.open", json!({"path":"renamed.psd"}));
    assert_eq!(opened["ok"], true, "{opened}");
    assert!(opened["result"]["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("F16")));
    assert!(app.session.active().unwrap().path.is_none());
    let before = app.session.active().unwrap().doc.clone();
    for path in ["../renamed.psd", "C:/renamed.psd", "/renamed.psd"] {
        assert_ne!(call(&mut app, "app.open", json!({"path":path}))["ok"], true);
    }
    assert_eq!(app.session.active().unwrap().doc, before);
    for params in [json!({}), json!({"path":"renamed.psd"}), json!({"path":"new.XCF"})] {
        assert_ne!(call(&mut app, "app.save", params)["ok"], true);
    }
    assert!(written.borrow().is_empty());
    assert_eq!(call(&mut app, "app.save", json!({"path":"edited.pcraft"}))["ok"], true);
    assert_eq!(call(&mut app, "app.save", json!({}))["ok"], true);
    assert_ne!(call(&mut app, "app.save", json!({"path":"renamed.psd"}))["ok"], true);
    for command in ["file.open", "file.save"] {
        assert_ne!(call(&mut app, "engine.execute", json!({"command":command,"params":{"path":"renamed.psd"}}))["ok"], true);
    }
    let writes = written.borrow();
    assert_eq!(writes.len(), 2);
    let reopened = photocraft_io::import("edited.pcraft", &writes[0].1).unwrap();
    assert_eq!(reopened.document.sample_encoding, before.sample_encoding);
    assert_eq!(reopened.document.layers[0].surface(), before.layers[0].surface());
    assert!(!reopened.origin.is_read_only());
}
