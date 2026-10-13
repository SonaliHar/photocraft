#[path = "../../io/tests/common/xcf.rs"]
mod common;
use photocraft_automation::{AuthorizedWorkspace, Headless, files, rpc};
use serde_json::json;
use std::{io::Write, sync::Mutex};

#[test]
fn wrapped_xcf_under_native_suffix_is_classified_by_content() {
    let input = common::image(12, 150, 0, 1, 1, &[13, 27, 59, 255]);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&input).unwrap();
    let bytes = encoder.finish().unwrap();
    let imported = files::open_bytes("misleading.pcraft", &bytes).unwrap();
    assert_eq!(imported.origin, photocraft_io::ImportOrigin::xcf());
    assert_eq!(imported.document.layers[0].surface().unwrap().try_read_interleaved(imported.document.bounds()).unwrap(), [13, 27, 59, 255]);
}

#[test]
fn rpc_named_failure_preserves_existing_document_and_error_envelope() {
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-failure-details-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("future.pcraft");
    let bytes = b"gimp xcf v099\0";
    std::fs::write(&path, bytes).unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&dir), None).unwrap();
    let mut headless = Headless::with_workspace(workspace);
    headless.command_run("file.new", json!({"width":2,"height":3,"name":"keep dirty"})).unwrap();
    let before = headless.inspect(None).unwrap();
    let headless = Mutex::new(headless);
    let reply = rpc::respond(&headless, &json!({"id": 7, "method": "doc.open", "params":{"path":"future.pcraft"}}).to_string());
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().contains("Version"));
    assert_eq!(reply["errorDetails"]["sourceFormat"], "xcf");
    assert_eq!(reply["errorDetails"]["diagnostics"][0]["code"], "unsupported_xcf_version");
    assert!(reply.get("result").is_none());
    assert_eq!(headless.lock().unwrap().inspect(None).unwrap(), before);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let denied = rpc::respond(&headless, &json!({"id":8,"method":"doc.open","params":{"path":"../future.pcraft"}}).to_string());
    assert_eq!(denied["ok"], false);
    assert!(denied.get("errorDetails").is_none(), "denied reads must not claim a file's format");
    drop(headless);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}
#[test]
fn rpc_semantic_inspection_keeps_exact_ids_and_denies_locator_access() {
    use photocraft_doc::{
        LayerId,
        item_identity::ResourceId,
        xcf_resources::{ResourceKind, ResourceReference},
    };
    let mut doc = photocraft_io::import("original.xcf", &common::image(12, 150, 0, 1, 1, &[19, 39, 79, 255])).unwrap().document;
    let layer_id = LayerId(9_007_199_254_740_993);
    doc.layers[0].id = layer_id;
    let state = doc.xcf_state.as_mut().unwrap();
    state.remap_layers(|_| Some(layer_id)).unwrap();
    state.resources.push(ResourceReference {
        id: ResourceId(u64::MAX),
        kind: ResourceKind::LinkedImage,
        name: "inert dependency".into(),
        collection: None,
        internal: false,
        locator: Some("C:/must-not-read/private.xcf".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    doc.validate_xcf_state().unwrap();
    let mut headless = Headless::new();
    headless.session.add_document(doc.clone(), None).unwrap();
    let state_before = headless.session.active().unwrap().try_clone_resident().unwrap();
    let headless = Mutex::new(headless);
    let request = |params| json!({"id":19,"method":"engine.execute","params":{"command":"xcf.inspect","params":params}}).to_string();
    let reply = rpc::respond(&headless, &request(json!({"limit":1})));
    assert_eq!(reply["id"], 19);
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"]["items"]["entries"][0]["item"]["id"], layer_id.0.to_string());
    assert_eq!(reply["result"]["resources"]["entries"][0]["id"], u64::MAX.to_string());
    assert_eq!(reply["result"]["resources"]["entries"][0]["accessGranted"], false);
    assert!(!reply.to_string().contains("must-not-read"));
    for params in [json!({"limit":0}), json!({"limit":129}), json!({"readLocator":true})] {
        let failed = rpc::respond(&headless, &request(params));
        assert_eq!(failed["ok"], false);
        assert!(failed.get("result").is_none());
    }
    let actual = headless.lock().unwrap();
    assert_eq!(*actual.session.active().unwrap().doc, doc);
    assert_eq!(actual.session.active().unwrap().revision, state_before.revision);
    assert_eq!(actual.session.active().unwrap().history.past_len(), 0);
}
