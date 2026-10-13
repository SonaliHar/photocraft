//! Bounded read-only canonical inspection with exact identity transport.
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_engine::Session;
use serde_json::json;
fn source() -> photocraft_doc::Document {
    photocraft_io::import("original.xcf", &xcf::image(12, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap().document
}
#[test]
fn inspection_preserves_large_ids_and_is_read_only() {
    let mut doc = source();
    let id = photocraft_doc::LayerId(9_007_199_254_740_993);
    doc.layers[0].id = id;
    doc.xcf_state.as_mut().unwrap().remap_layers(|_| Some(id)).unwrap();
    doc.validate_xcf_state().unwrap();
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    let before = session.active().unwrap().try_clone_resident().unwrap();
    let result = session.execute("xcf.inspect", json!({"limit":1})).unwrap();
    assert_eq!(result["items"]["entries"][0]["item"], json!({"kind":"layer","id":id.0.to_string()}));
    assert_eq!(result["items"]["total"], 1);
    assert_eq!(result["formatVersion"], 12);
    assert_eq!(result["sampleType"], "U8");
    assert_eq!(session.active().unwrap().doc, before.doc);
    assert_eq!(session.active().unwrap().revision, before.revision);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
}
#[test]
fn inspection_denies_hostile_params_and_invalid_state_without_mutation() {
    let mut session = Session::new();
    session.add_document(source(), None).unwrap();
    let before = session.active().unwrap().try_clone_resident().unwrap();
    for params in [json!({"limit":0}), json!({"limit":129}), json!({"offset":-1}), json!({"offset":"1"}), json!({"readLocator":true}), json!([])] {
        assert!(
            matches!(session.execute("xcf.inspect",params.clone()),Err(photocraft_engine::EngineError::BadParams { cmd, .. }) if cmd == "xcf.inspect"),
            "{params}"
        );
        assert_eq!(session.active().unwrap().doc, before.doc);
        assert_eq!(session.active().unwrap().revision, before.revision);
    }
    let mut doc = source();
    doc.xcf_state.as_mut().unwrap().items[0].item = photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(u64::MAX));
    let mut invalid = Session::new();
    invalid.add_document(doc.clone(), None).unwrap();
    assert!(matches!(invalid.execute("xcf.inspect",json!({})),Err(photocraft_engine::EngineError::BadParams { cmd, .. }) if cmd == "xcf.inspect"));
    assert_eq!(*invalid.active().unwrap().doc, doc);
}
#[test]
fn inspection_caps_metadata_and_never_exposes_resource_locators() {
    use photocraft_doc::{
        item_identity::ResourceId,
        xcf_resources::{ResourceKind, ResourceReference},
    };
    let mut doc = source();
    for index in 1..=129 {
        doc.xcf_state.as_mut().unwrap().resources.push(ResourceReference {
            id: ResourceId(index),
            kind: ResourceKind::LinkedImage,
            name: "界".repeat(512),
            collection: None,
            internal: false,
            locator: Some("C:/must-not-read/secret.xcf".into()),
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        });
    }
    doc.xcf_state.as_mut().unwrap().provenance.original_locator = Some("C:/must-not-read/source.xcf".into());
    doc.validate_xcf_state().unwrap();
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    let first = session.execute("xcf.inspect", json!({"limit":128})).unwrap();
    assert_eq!(first["resources"]["total"], 129);
    let entries = first["resources"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 128);
    assert_eq!(entries[0]["name"].as_str().unwrap().chars().count(), 256);
    assert_eq!(entries[0]["hasLocator"], true);
    assert_eq!(entries[0]["accessGranted"], false);
    let wire = serde_json::to_string(&first).unwrap();
    assert!(!wire.contains("must-not-read"));
    assert!(wire.len() < 256 * 1024);
    let tail = session.execute("xcf.inspect", json!({"offset":128,"limit":128})).unwrap();
    assert_eq!(tail["resources"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(tail["resources"]["entries"][0]["id"], "129");
    assert_eq!(*session.active().unwrap().doc, doc);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
}
#[test]
fn inspection_exposes_exact_composition_mask_and_saved_membership() {
    use photocraft_doc::{
        item_identity::{ItemRef, ItemSetId, MaskId},
        xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
        xcf_state::{ItemState, MaskState},
    };
    let mut doc = source();
    let owner = ItemRef::Layer(doc.layers[0].id);
    let mask = ItemRef::Mask(MaskId(u64::MAX));
    let state = doc.xcf_state.as_mut().unwrap();
    let composition = state.items[0].composition.as_mut().unwrap();
    composition.compatibility_epoch = "界".repeat(512);
    let expected = composition.clone();
    state.items.push(ItemState {
        item: mask,
        name: "independent mask".into(),
        custom_name: false,
        visible: false,
        locks: Default::default(),
        parent: None,
        source_tattoo: Some(17),
        source_flags: 7,
        raster_bounds: None,
        composition: None,
        mask: Some(MaskState {
            owner,
            bounds: photocraft_doc::Rect::new(-2, 3, 4, 9),
            offset: [-2, 3],
            outside_f64_bits: (-0.0_f64).to_bits(),
            enabled: false,
            shown: true,
            editing: true,
            linked: false,
        }),
    });
    state.item_sets.push(ItemSet {
        id: ItemSetId(9_007_199_254_740_993),
        name: "saved exact set".into(),
        family: ItemFamily::Layer,
        rule: ItemSetRule::Named,
        members: vec![owner],
    });
    doc.validate_xcf_state().unwrap();
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    let result = session.execute("xcf.inspect", json!({"limit":2})).unwrap();
    let actual = &result["items"]["entries"][0]["composition"];
    assert_eq!(actual["sourceMode"], expected.source_mode);
    assert_eq!(actual["storedBlendSpace"], expected.stored_blend_space);
    assert_eq!(actual["storedCompositeSpace"], expected.stored_composite_space);
    assert_eq!(actual["storedOperator"], expected.stored_operator);
    assert_eq!(actual["resolved"], serde_json::to_value(&expected.resolved).unwrap());
    assert_eq!(actual["compatibilityEpoch"].as_str().unwrap().chars().count(), 256);
    assert_eq!(actual["epochTruncated"], true);
    let mask_row = &result["items"]["entries"][1]["mask"];
    assert_eq!(mask_row["owner"]["id"], doc.layers[0].id.0.to_string());
    assert_eq!(mask_row["outsideFloat64Bits"], "0x8000000000000000");
    assert_eq!(mask_row["offset"], json!([-2, 3]));
    assert_eq!(mask_row["enabled"], false);
    assert_eq!(mask_row["shown"], true);
    assert_eq!(mask_row["editing"], true);
    assert_eq!(mask_row["linked"], false);
    assert_eq!(result["savedSets"]["entries"][0]["members"]["entries"][0]["id"], doc.layers[0].id.0.to_string());
    assert_eq!(result["savedSets"]["entries"][0]["ruleKind"], "named");
    assert_eq!(*session.active().unwrap().doc, doc);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
}

#[test]
fn inspection_exposes_bounded_current_unicode_text_without_reading_font_locator() {
    use photocraft_doc::{
        item_identity::{ItemRef, ResourceId},
        xcf_items::{EditableSource, SourceItem},
        xcf_resources::{ResourceKind, ResourceReference},
    };
    let mut doc = source();
    let owner = ItemRef::Layer(doc.layers[0].id);
    let font = ResourceId(9_007_199_254_740_993);
    let content = "é界🙂".repeat(100);
    let markup = format!("<span>{content}</span>");
    let state = doc.xcf_state.as_mut().unwrap();
    state.resources.push(ResourceReference {
        id: font,
        kind: ResourceKind::Font,
        name: "Missing exact font".into(),
        collection: None,
        internal: false,
        locator: Some("C:/must-not-read/font.ttf".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    state.source_items.push(SourceItem {
        owner,
        source: EditableSource::Text {
            content: content.clone(),
            markup: Some(markup.clone()),
            font,
            direction: 2,
            source_modified: true,
            configuration: vec![],
            current_origin: None,
        },
        transform: None,
        cache: None,
    });
    doc.validate_xcf_state().unwrap();
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let mut session = Session::new();
    session.add_document(photocraft_format::load_from_bytes(&native).unwrap(), None).unwrap();
    let before = session.active().unwrap().try_clone_resident().unwrap();
    let view = session.execute("xcf.inspect", json!({"limit":1})).unwrap();
    let text = &view["sources"]["entries"][0]["source"];
    assert_eq!(text["content"], content.chars().take(256).collect::<String>());
    assert_eq!(text["contentTruncated"], true);
    assert_eq!(text["characters"], 300);
    assert_eq!(text["markup"], markup.chars().take(256).collect::<String>());
    assert_eq!(text["markupTruncated"], true);
    assert_eq!(text["direction"], 2);
    assert_eq!(text["fontId"], font.0.to_string());
    assert_eq!(text["sourceModified"], true);
    assert!(!view.to_string().contains("must-not-read"));
    assert_eq!(view["resources"]["entries"][0]["accessGranted"], false);
    assert_eq!(session.active().unwrap().doc, before.doc);
    assert_eq!(session.active().unwrap().revision, before.revision);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
}
