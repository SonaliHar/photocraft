//! Current text placement is a PhotoCraft edit, separate from source provenance.
use photocraft_doc::xcf_items::EditableSource;
use serde_json::json;

fn text() -> serde_json::Value {
    json!({"Text": {
        "content": "HEFT", "markup": null, "font": 1, "direction": 0,
        "source_modified": false, "configuration": []
    }})
}

#[test]
fn current_text_origin_survives_typed_serialization_without_a_source_transform() {
    for origin in [[11, 2], [-5, 7], [8, 4]] {
        let mut value = text();
        value["Text"]["current_origin"] = json!(origin);
        let source: EditableSource = serde_json::from_value(value).unwrap();
        let restored = serde_json::to_value(source).unwrap();
        assert_eq!(restored["Text"]["current_origin"], json!(origin), "current placement must not silently disappear");
        assert_eq!(restored["Text"]["configuration"], json!([]));
    }
}

#[test]
fn legacy_text_without_current_origin_keeps_its_original_serialized_shape() {
    let original = text();
    let source: EditableSource = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(serde_json::to_value(source).unwrap(), original);
}
