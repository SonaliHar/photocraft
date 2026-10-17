use photocraft_engine::Session;
use serde_json::json;

#[test]
fn numeric_path_transform_pivots_on_explicit_target_not_active_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 128, "background": "transparent"})).unwrap();
    let target = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 10, 10]})).unwrap()["layer"].as_u64().unwrap();
    let active = s.execute("shape.create", json!({"kind": "rect", "rect": [70, 70, 10, 10]})).unwrap()["layer"].as_u64().unwrap();
    let other_before = s.execute("shape.info", json!({"layer": active})).unwrap();
    let before = s.active().unwrap().doc.clone();

    let result = s.execute("path.transform", json!({"name": "layer", "layer": target, "scaleX": 2, "scaleY": 2})).unwrap();
    assert_eq!(result["matrix"], json!([2.0, 0.0, 0.0, 2.0, -15.0, -15.0]));
    let after = s.execute("shape.info", json!({"layer": target})).unwrap();
    assert_eq!(after["bounds"], json!([5, 5, 20, 20]));
    assert_eq!(s.execute("shape.info", json!({"layer": active})).unwrap(), other_before);
    let transformed = s.active().unwrap().doc.clone();
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc, transformed);
    let bytes = photocraft_format::save_to_bytes(&transformed, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *transformed);
}

#[test]
fn explicit_path_matrix_is_not_recentered() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 128})).unwrap();
    let target = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 10, 10]})).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [70, 70, 10, 10]})).unwrap();
    let result = s.execute("path.transform", json!({"name": "layer", "layer": target, "matrix": [2, 0, 0, 2, -15, -15]})).unwrap();
    assert_eq!(result["matrix"], json!([2.0, 0.0, 0.0, 2.0, -15.0, -15.0]));
    let info = s.execute("shape.info", json!({"layer": target})).unwrap();
    assert_eq!(info["bounds"], json!([5, 5, 20, 20]));
}
