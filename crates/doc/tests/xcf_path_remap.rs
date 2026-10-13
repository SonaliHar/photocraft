//! Remap actual saved paths/vector bindings without duplicating owned payloads.
use photocraft_doc::{
    Color, ColorMode, Document, LayerId, NamedPath, Path, PixelFormat, SampleType, Size, XcfState,
    item_identity::{ItemRef, ItemSetId, PathId, ResourceId},
    xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
    xcf_items::{EditableSource, SourceCache, SourceItem},
    xcf_provenance::OpaqueRecord,
    xcf_resources::{ResourceKind, ResourceReference},
};
use std::collections::BTreeMap;

fn document() -> Document {
    let mut doc = Document::with_background("Own path model", Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.layers[0].id = LayerId(10);
    for id in [1, 2] {
        let mut path = NamedPath::new("Own path", Path::default());
        path.id = PathId(id);
        doc.paths.push(path);
    }
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.selection.selected_paths = vec![ItemRef::Path(PathId(1)), ItemRef::Path(PathId(2))];
    state.item_sets.push(ItemSet {
        id: ItemSetId(1),
        name: "Own paths".into(),
        family: ItemFamily::Path,
        rule: ItemSetRule::Named,
        members: vec![ItemRef::Path(PathId(2)), ItemRef::Path(PathId(1))],
    });
    let bytes = vec![255; 8];
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(LayerId(10)),
        source: EditableSource::Vector { path: PathId(1), source_modified: true, configuration: vec![] },
        transform: None,
        cache: Some(SourceCache { blake3: blake3::hash(&bytes).to_hex().to_string(), bytes: bytes.into(), width: 2, height: 1, format: PixelFormat::RGBA8 }),
    });
    let bytes = b"Own inactive pattern payload".to_vec();
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::Pattern,
        name: "Own inert resource".into(),
        collection: None,
        internal: false,
        locator: None,
        content_hash: Some(blake3::hash(&bytes).to_hex().to_string()),
        owned_bytes: Some(bytes.into()),
        resolution: None,
    });
    let bytes = vec![41; 32 * 1024];
    state.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Path(PathId(2))),
        namespace: "own".into(),
        identity: "own".into(),
        source_version: 22,
        order: 0,
        flags: 3,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    doc
}

#[test]
fn callback_once_preserves_saved_paths_and_actual_vector_binding() {
    let mut doc = document();
    let state = doc.xcf_state.as_mut().unwrap();
    let mut calls = 0;
    state
        .remap_paths(|_| {
            calls += 1;
            Some(PathId(10 + calls))
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(state.selection.selected_paths, vec![ItemRef::Path(PathId(11)), ItemRef::Path(PathId(12))]);
    assert!(matches!(state.source_items[0].source, EditableSource::Vector { path: PathId(11), .. }));
    doc.paths[0].id = PathId(11);
    doc.paths[1].id = PathId(12);
    doc.validate_xcf_state().unwrap();
}

#[test]
fn identity_remap_preserves_source_resource_and_opaque_payload_pointers() {
    let mut doc = document();
    let original = doc.clone();
    let state = doc.xcf_state.as_mut().unwrap();
    let cache = state.source_items[0].cache.as_ref().unwrap().bytes.as_ptr();
    let resource = state.resources[0].owned_bytes.as_ref().unwrap().as_ptr();
    let opaque = state.opaque[0].bytes.as_ptr();
    state.remap_paths(Some).unwrap();
    assert_eq!(state.source_items[0].cache.as_ref().unwrap().bytes.as_ptr(), cache);
    assert_eq!(state.resources[0].owned_bytes.as_ref().unwrap().as_ptr(), resource);
    assert_eq!(state.opaque[0].bytes.as_ptr(), opaque);
    assert!(doc == original, "identity remap changed model");
}

#[test]
fn failures_preserve_valid_document_and_all_payloads() {
    for case in 0..3 {
        let mut doc = document();
        let original = doc.clone();
        let state = doc.xcf_state.as_mut().unwrap();
        let opaque = state.opaque[0].bytes.as_ptr();
        let result = state.remap_paths(|id| match case {
            0 => Some(PathId(0)),
            1 => Some(PathId(11)),
            _ => (id != PathId(2)).then_some(PathId(11)),
        });
        assert!(result.is_err());
        assert_eq!(state.opaque[0].bytes.as_ptr(), opaque);
        assert!(doc == original, "refusal changed document");
        doc.validate_xcf_state().unwrap();
    }
}

#[test]
fn path_permutation_preserves_layer_resource_and_set_namespaces() {
    let mut doc = document();
    let state = doc.xcf_state.as_mut().unwrap();
    let mut calls = BTreeMap::new();
    state
        .remap_paths(|id| {
            *calls.entry(id).or_insert(0) += 1;
            Some(PathId(if id == PathId(1) { 2 } else { 1 }))
        })
        .unwrap();
    assert_eq!(calls, BTreeMap::from([(PathId(1), 1), (PathId(2), 1)]));
    assert_eq!(state.source_items[0].owner, ItemRef::Layer(LayerId(10)));
    assert_eq!(state.resources[0].id, ResourceId(1));
    assert_eq!(state.item_sets[0].id, ItemSetId(1));
    doc.paths[0].id = PathId(2);
    doc.paths[1].id = PathId(1);
    doc.validate_xcf_state().unwrap();
}

#[test]
fn late_vector_dependency_failure_is_atomic() {
    let mut doc = document();
    let state = doc.xcf_state.as_mut().unwrap();
    let EditableSource::Vector { path, .. } = &mut state.source_items[0].source else { panic!("fixture") };
    *path = PathId(3);
    let before = state.clone();
    assert!(state.remap_paths(|id| (id != PathId(3)).then_some(PathId(id.0 + 10))).is_err());
    assert!(state == &before, "late dependency failure changed state");
}

#[test]
fn existing_drawable_ceiling_is_accepted_and_oversized_is_atomic() {
    let mut doc = document();
    let mut state = doc.xcf_state.take().unwrap();
    state.source_items.clear();
    state.item_sets.clear();
    state.opaque.clear();
    state.selection.selected_paths.clear();
    let template = state.items.iter().find(|item| matches!(item.item, ItemRef::Path(_))).unwrap().clone();
    state.items = (1..=10000)
        .map(|id| {
            let mut item = template.clone();
            item.item = ItemRef::Path(PathId(id));
            item
        })
        .collect();
    state.remap_paths(|id| Some(PathId(10001 - id.0))).unwrap();
    assert_eq!(state.items[0].item, ItemRef::Path(PathId(10000)));
    let mut extra = template;
    extra.item = ItemRef::Path(PathId(30000));
    state.items.push(extra);
    let original = state.clone();
    assert!(state.remap_paths(Some).is_err());
    assert!(state == original, "oversized input changed");
}
