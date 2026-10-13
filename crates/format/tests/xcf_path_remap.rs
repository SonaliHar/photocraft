//! Valid saved paths and vector bindings continue exactly through both native containers.
use photocraft_doc::{
    Color, ColorMode, Document, LayerId, NamedPath, Path, PixelFormat, SampleType, Size, XcfState,
    item_identity::{ItemRef, ItemSetId, PathId, ResourceId},
    xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
    xcf_items::{EditableSource, SourceCache, SourceItem},
    xcf_provenance::OpaqueRecord,
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_raster::Interrupt;
use std::{collections::BTreeMap, io::Cursor};

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

fn archives(doc: &Document) -> (Vec<u8>, Vec<u8>) {
    let legacy = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    let (stream, _) = photocraft_format::PcraftWriter::new()
        .save_zip_stream(doc, &Default::default(), Cursor::new(Vec::new()), Default::default(), 64 << 20, &Interrupt::NONE)
        .unwrap();
    (legacy, stream.into_inner())
}
fn roundtrip(doc: &Document) {
    let (legacy, stream) = archives(doc);
    assert_eq!(photocraft_format::load_from_bytes(&legacy).unwrap(), *doc);
    assert_eq!(photocraft_format::load_from_stream(Cursor::new(stream), &Default::default(), Default::default(), &Interrupt::NONE).unwrap(), *doc);
}

#[test]
fn successful_path_remap_continues_through_both_native_containers() {
    let mut doc = document();
    roundtrip(&doc);
    let original = doc.clone();
    let state = doc.xcf_state.as_mut().unwrap();
    let cache = state.source_items[0].cache.as_ref().unwrap().bytes.as_ptr();
    let resource = state.resources[0].owned_bytes.as_ref().unwrap().as_ptr();
    let opaque = state.opaque[0].bytes.as_ptr();
    let mut calls = BTreeMap::new();
    state
        .remap_paths(|id| {
            *calls.entry(id).or_insert(0) += 1;
            Some(PathId(id.0 + 20))
        })
        .unwrap();
    assert_eq!(calls, BTreeMap::from([(PathId(1), 1), (PathId(2), 1)]));
    assert_eq!(state.source_items[0].cache.as_ref().unwrap().bytes.as_ptr(), cache);
    assert_eq!(state.resources[0].owned_bytes.as_ref().unwrap().as_ptr(), resource);
    assert_eq!(state.opaque[0].bytes.as_ptr(), opaque);
    assert!(matches!(state.source_items[0].source, EditableSource::Vector { path: PathId(21), .. }));
    assert_eq!(state.selection.selected_paths, vec![ItemRef::Path(PathId(21)), ItemRef::Path(PathId(22))]);
    assert_eq!(state.item_sets[0].members, vec![ItemRef::Path(PathId(22)), ItemRef::Path(PathId(21))]);
    assert_eq!(state.opaque[0].owner, Some(ItemRef::Path(PathId(22))));
    assert_eq!(state.resources, original.xcf_state.as_ref().unwrap().resources);
    assert_eq!(state.source_items[0].cache, original.xcf_state.as_ref().unwrap().source_items[0].cache);
    for path in &mut doc.paths {
        path.id = PathId(path.id.0 + 20);
    }
    assert_eq!(doc.layers, original.layers);
    doc.validate_xcf_state().unwrap();
    roundtrip(&doc);
}

#[test]
fn failed_path_remaps_leave_exact_native_bytes_unchanged() {
    for case in 0..3 {
        let mut doc = document();
        let original = doc.clone();
        let before = archives(&doc);
        let result = doc.xcf_state.as_mut().unwrap().remap_paths(|id| match case {
            0 => Some(PathId(0)),
            1 => Some(PathId(21)),
            _ => (id != PathId(2)).then_some(PathId(21)),
        });
        assert!(result.is_err());
        assert!(doc == original, "refusal changed document");
        assert_eq!(archives(&doc), before);
        roundtrip(&doc);
    }
}
