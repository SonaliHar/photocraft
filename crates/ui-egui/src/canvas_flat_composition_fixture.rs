//! UNRUN CURRENT composition harness. Source/native/model captures are external
//! harness data; only the actual paid import/native handoffs establish custody.
use photocraft_doc::{Document, LayerId, item_identity::ItemRef};
use serde_json::{Value, json};
pub(super) const MODES: [i32; 20] = [23, 28, 30, 31, 32, 33, 34, 35, 36, 41, 42, 43, 44, 45, 48, 49, 50, 51, 52, 53];
pub(super) fn wire(doc: &Document, revision: u64, layer: LayerId, mode: i32, blend: &str, composite: &str, operator: &str) -> Value {
    json!({"layerId":layer.0.to_string(),"documentId":doc.id.0.to_string(),"expectedRevision":revision.to_string(),"sourceMode":mode,"legacy":false,"blendSpace":blend,"compositeSpace":composite,"operator":operator})
}
pub(super) fn preserved(a: &Document, b: &Document) {
    super::fixture::payload(a, b);
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.name, b.name);
        assert_eq!(a.visible, b.visible);
        assert_eq!(a.opacity.to_bits(), b.opacity.to_bits());
        assert_eq!(a.fill_opacity.to_bits(), b.fill_opacity.to_bits());
        assert_eq!(a.locks, b.locks);
        assert_eq!(a.effects, b.effects);
    }
    let mut state = b.xcf_state.clone().unwrap();
    let original = a.xcf_state.as_ref().unwrap();
    let changed = state.items.iter().filter(|item| original.items.iter().find(|p| p.item == item.item).unwrap().composition != item.composition).count();
    assert!(changed <= 1, "one composition target per transaction");
    for item in &mut state.items {
        let prior = original.items.iter().find(|p| p.item == item.item).unwrap();
        assert_eq!(item.composition.as_ref().unwrap().compatibility_epoch, prior.composition.as_ref().unwrap().compatibility_epoch);
        item.composition = prior.composition.clone();
    }
    assert_eq!(state, *original);
}
pub(super) fn tuple(doc: &Document, layer: LayerId, mode: i32, blend: i32, composite: i32, operator: i32) {
    let item = doc.xcf_state.as_ref().unwrap().items.iter().find(|p| p.item == ItemRef::Layer(layer)).unwrap();
    let p = item.composition.as_ref().unwrap();
    assert_eq!((p.source_mode, p.stored_blend_space, p.stored_composite_space, p.stored_operator), (mode, blend, composite, operator));
    assert!(!p.legacy);
    assert!(!p.pass_through);
    p.checked_resolved().unwrap();
    let encoding = |raw| match raw {
        1 => photocraft_doc::SampleEncoding::Linear,
        2 => photocraft_doc::SampleEncoding::NonlinearProfile,
        4 => photocraft_doc::SampleEncoding::Perceptual,
        _ => panic!("explicit RGB fixture"),
    };
    let physical = doc.layer(layer).unwrap().import_compositing.as_ref().unwrap();
    assert_eq!(physical.blend_encoding, encoding(blend));
    assert_eq!(physical.composite_encoding, encoding(composite));
    assert!(!physical.pass_through);
}
