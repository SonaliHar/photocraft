//! Registered duplicate current-reference regression; synthetic native controls.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, Layer, LayerContent, LayerId, XcfState,
    item_identity::{FilterId, ItemRef, ItemSetId},
    xcf_editing::{EditingSelection, FloatingAttachment, ItemFamily, ItemSet, ItemSetRule},
    xcf_filters::{FilterNode, FilterStack, OperationIdentity},
    xcf_provenance::OpaqueRecord,
    xcf_state::{ItemLocks, ItemState},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};
use std::io::Cursor;

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn document(depth: SampleType) -> TestResult<Document> {
    let mut doc = Document::new("Own remap continuation", Size::new(2, 1), ColorMode::Rgb, depth);
    doc.set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.25, y_dpi: 287.75, unit: Default::default() })?;
    let mut state = XcfState::default();
    for id in [1, 2] {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        surface.write_region(Rect::new(0, 0, 2, 1), &[0.25, 0.5, 0.0, 1.0, 0.75, 0.0, 0.125, 0.5]);
        let mut layer = Layer::new("Own identity", LayerContent::Raster(surface));
        layer.id = LayerId(id);
        doc.layers.push(layer);
        state.items.push(ItemState {
            item: ItemRef::Layer(LayerId(id)),
            name: "Own identity".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(17),
            source_flags: 29,
            raster_bounds: Some(Rect::new(0, 0, 2, 1)),
            mask: None,
            composition: None,
        });
    }
    let one = ItemRef::Layer(LayerId(1));
    let two = ItemRef::Layer(LayerId(2));
    state.selection = EditingSelection { selected: vec![one, two], active: Some(two), selected_paths: vec![], mask_target: None };
    state.item_sets.push(ItemSet { id: ItemSetId(1), name: "Own set".into(), family: ItemFamily::Layer, rule: ItemSetRule::Named, members: vec![two, one] });
    state.filter_stacks.push(FilterStack {
        owner: one,
        revision: 7,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "own:inert".into(), stored_version: "0:0".into(), schema_epoch: "own".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: false,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![("aux".into(), two)],
            composition: None,
        }],
    });
    let bytes = vec![41; 4096];
    state.opaque.push(OpaqueRecord {
        owner: Some(two),
        namespace: "own".into(),
        identity: "own".into(),
        source_version: 22,
        order: 0,
        flags: 3,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    state.floating = Some(FloatingAttachment { layer: LayerId(2), target: one });
    let mut group = Layer::group("Own nested group", std::mem::take(&mut doc.layers));
    group.id = LayerId(9000);
    for item in &mut state.items {
        item.parent = Some(ItemRef::Layer(group.id));
    }
    state.items.push(ItemState {
        item: ItemRef::Layer(group.id),
        name: group.name.clone(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(33),
        source_flags: 0,
        raster_bounds: None,
        mask: None,
        composition: None,
    });
    doc.layers.push(group);
    let comp = photocraft_doc::comps::LayerComp {
        id: 7,
        name: "Own comp".into(),
        comment: "retained".into(),
        apply_visibility: true,
        apply_position: false,
        apply_appearance: false,
        states: vec![photocraft_doc::comps::CompLayerState { layer: LayerId(2), visible: Some(false), position: None, appearance: None }],
    };
    doc.layer_comps.push(comp.clone());
    doc.last_document_state = Some(comp);
    doc.last_applied_comp = Some(7);
    doc.variables.defs.push(photocraft_doc::variables::VariableDef {
        name: "Own visibility".into(),
        layer: LayerId(1),
        kind: photocraft_doc::variables::VarKind::Visibility,
    });
    doc.slices.list.push(photocraft_doc::slices::Slice {
        id: 8,
        layer: Some(LayerId(2)),
        origin: photocraft_doc::slices::SliceOrigin::Layer,
        rect: doc.bounds(),
        ..Default::default()
    });
    state.filter_stacks[0].nodes[0].mask = Some(ItemRef::Mask(photocraft_doc::item_identity::MaskId(55)));
    let mask = photocraft_doc::FilterMask::try_new(
        photocraft_doc::item_identity::MaskId(55),
        FilterId(1),
        Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false)),
        doc.bounds(),
    )?;
    state.items.push(ItemState {
        item: ItemRef::Mask(mask.id),
        name: "Own mask".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(44),
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        mask: Some(photocraft_doc::xcf_state::MaskState {
            owner: one,
            bounds: doc.bounds(),
            offset: [0, 0],
            outside_f64_bits: 0.0f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        }),
        composition: None,
    });
    state.filter_masks.push(mask);
    state.provenance = photocraft_doc::xcf_provenance::ProjectProvenance {
        format: "own".into(),
        format_version: 22,
        container: "own".into(),
        producer: Some("Own synthetic".into()),
        release_id: None,
        source_label: "Own inert original".into(),
        source_hash: Some("ab".repeat(32)),
        original_locator: Some("own/inert-not-authority.xcf".into()),
        protected_source_fingerprints: vec!["cd".repeat(32)],
    };
    doc.xcf_state = Some(state);
    doc.validate_xcf_state()?;
    Ok(doc)
}

fn archives(doc: &Document) -> TestResult<(Vec<u8>, Vec<u8>)> {
    let legacy = photocraft_format::save_to_bytes(doc, &Default::default())?;
    let (stream, _) = photocraft_format::PcraftWriter::new().save_zip_stream(
        doc,
        &Default::default(),
        Cursor::new(Vec::new()),
        Default::default(),
        64 << 20,
        &Interrupt::NONE,
    )?;
    Ok((legacy, stream.into_inner()))
}

fn roundtrip(doc: &Document) -> TestResult<()> {
    let (legacy, stream) = archives(doc)?;
    if photocraft_format::load_from_bytes(&legacy)? != *doc
        || photocraft_format::load_from_stream(Cursor::new(stream), &Default::default(), Default::default(), &Interrupt::NONE)? != *doc
    {
        return Err("native remap continuation lost document fields or samples".into());
    }
    Ok(())
}

// Unrun sharing-wave draft: identity metadata may change while retained bytes stay shared.
#[test]
fn opaque_payload_clone_shares_bytes_and_private_tamper_keeps_original() {
    let original = document(SampleType::U16).unwrap();
    let before = archives(&original).unwrap();
    let mut private = original.clone();
    let retained = &original.xcf_state.as_ref().unwrap().opaque.first().unwrap().bytes;
    let changed = &mut private.xcf_state.as_mut().unwrap().opaque.first_mut().unwrap().bytes;
    assert!(std::sync::Arc::ptr_eq(retained, changed));
    std::sync::Arc::make_mut(changed)[0] ^= 1;
    assert!(!std::sync::Arc::ptr_eq(retained, changed));
    assert_ne!(retained.as_slice(), changed.as_slice());
    assert_eq!(archives(&original).unwrap(), before);
    assert!(private.validate_xcf_state().is_err());
}

fn retained_session(doc: Document) -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::var_os("PHOTOCRAFT_DUPLICATE_TEST_OUTPUT").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let root = root.join(format!(
        "photocraft-duplicate-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        sequence
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("own-source.pcraft");
    std::fs::write(&path, photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
    s.add_document(doc, None).unwrap();
    s.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(), "Own retained runtime source", Some(path.to_string_lossy().into_owned()));
    s.edit("Own redo", |d, _| {
        d.name.push_str(" redo");
        Ok(())
    })
    .unwrap();
    s.execute("edit.undo", serde_json::json!({})).unwrap();
    s
}
#[derive(Debug, PartialEq, Eq)]
struct RetainedHistory {
    revision: u64,
    saved: u64,
    entries: Vec<String>,
    redo: Vec<String>,
    source: Option<photocraft_engine::SourceIdentity>,
    file_hash: String,
}
fn history(s: &photocraft_engine::Session) -> RetainedHistory {
    let d = &s.documents()[0];
    RetainedHistory {
        revision: d.revision,
        saved: d.saved_revision,
        entries: d.history.entry_labels().map(str::to_owned).collect(),
        redo: d.history.redo_labels().map(str::to_owned).collect(),
        source: d.source.clone(),
        file_hash: blake3::hash(&std::fs::read(d.source.as_ref().unwrap().path.as_ref().unwrap()).unwrap()).to_hex().to_string(),
    }
}
#[test]
fn ordinary_duplicate_remaps_nested_typed_and_all_four_document_holders() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let source = document(depth).unwrap();
        roundtrip(&source).unwrap();
        let mut s = retained_session(source.clone());
        let snapshot = s.documents()[0].doc.clone();
        let h = history(&s);
        let native = archives(&snapshot).unwrap();
        s.execute("image.duplicate", serde_json::json!({"name":"Own duplicate"})).unwrap();
        assert!(s.documents()[1].source.is_none() && s.documents()[1].path.is_none());
        let out = &s.documents()[1].doc;
        out.validate_xcf_state().unwrap();
        roundtrip(out).unwrap();
        let old = source.walk();
        let new = out.walk();
        let map = old.iter().zip(&new).map(|(a, b)| (a.2.id, b.2.id)).collect::<std::collections::HashMap<_, _>>();
        assert!(new.iter().all(|x| !map.contains_key(&x.2.id) && x.2.id.0 != 0));
        let mut expected = (*snapshot).clone();
        expected.id = out.id;
        expected.name = out.name.clone();
        fn ids(ls: &mut [Layer], m: &std::collections::HashMap<LayerId, LayerId>) {
            for l in ls {
                l.id = m[&l.id];
                if let LayerContent::Group(c) = &mut l.content {
                    ids(&mut c.children, m);
                }
            }
        }
        ids(&mut expected.layers, &map);
        expected.xcf_state.as_mut().unwrap().remap_layers(|id| map.get(&id).copied()).unwrap();
        for c in expected.layer_comps.iter_mut().chain(expected.last_document_state.iter_mut()) {
            for st in &mut c.states {
                st.layer = map[&st.layer];
            }
        }
        for v in &mut expected.variables.defs {
            v.layer = map[&v.layer];
        }
        for sl in &mut expected.slices.list {
            sl.layer = sl.layer.map(|id| map[&id]);
        }
        assert!(**out == expected, "duplicate differs from current-reference remap only");
        assert!(s.documents()[0].doc.shares_state(&snapshot));
        assert_eq!(history(&s), h);
        assert!(archives(&s.documents()[0].doc).unwrap() == native, "source native bytes changed");
    }
}
#[test]
fn historical_comp_refs_survive_but_stale_variable_and_slice_refs_refuse() {
    for field in 0..4 {
        let mut doc = document(SampleType::U8).unwrap();
        doc.xcf_state = None;
        match field {
            0 => doc.layer_comps[0].states[0].layer = LayerId(99999),
            1 => doc.last_document_state.as_mut().unwrap().states[0].layer = LayerId(99999),
            2 => doc.variables.defs[0].layer = LayerId(99999),
            _ => doc.slices.list[0].layer = Some(LayerId(99999)),
        }
        // Publication refreshes/drops layer slices. Inject the hostile retained
        // association after setup so it is genuinely present at dispatch.
        let hostile = doc.clone();
        let mut s = retained_session(doc);
        if field == 3 {
            let mut current = (*s.documents()[0].doc).clone();
            current.slices = hostile.slices;
            s.active_mut().unwrap().doc = photocraft_engine::DocumentSnapshot::from_resident(std::sync::Arc::new(current)).unwrap();
            assert_eq!(s.documents()[0].doc.slices.list[0].layer, Some(LayerId(99999)));
        }
        let snapshot = s.documents()[0].doc.clone();
        let native = archives(&snapshot).unwrap();
        let events = s.file_menu.event_log.clone();
        let journal = s.journal.clone();
        let h = history(&s);
        if field < 2 {
            // Upstream preserves unresolved historical comp references, while
            // reserving those IDs against the duplicate's fresh live identities.
            s.execute("image.duplicate", serde_json::json!({})).unwrap();
            assert_eq!(s.documents().len(), 2);
            let copied = &s.documents()[1].doc;
            if field == 0 {
                assert_eq!(copied.layer_comps[0], snapshot.layer_comps[0]);
            } else {
                assert_eq!(copied.last_document_state, snapshot.last_document_state);
            }
            assert!(copied.walk().into_iter().all(|(_, _, layer)| layer.id != LayerId(99999)));
            assert!(s.documents()[0].doc.shares_state(&snapshot));
            assert_eq!(history(&s), h);
            assert_eq!(archives(&s.documents()[0].doc).unwrap(), native);
            assert_eq!(&s.journal[..journal.len()], journal.as_slice());
            assert_eq!(s.journal.last().unwrap().0, "image.duplicate");
            roundtrip(copied).unwrap();
            continue;
        }
        let err = s.execute("image.duplicate", serde_json::json!({})).expect_err("unresolved variable/slice association must refuse");
        assert!(err.to_string().contains("duplicate"));
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.file_menu.event_log, events);
        assert_eq!(s.journal, journal);
        assert!(s.documents()[0].doc.shares_state(&snapshot));
        assert_eq!(history(&s), h);
        assert!(archives(&s.documents()[0].doc).unwrap() == native, "source native bytes changed");
    }
}
#[test]
fn merged_comp_state_drops_but_variable_and_slice_refs_refuse() {
    let mut doc = document(SampleType::U8).unwrap();
    doc.xcf_state = None;
    for field in 0..4 {
        let mut one = doc.clone();
        if field != 0 {
            one.layer_comps.clear();
        }
        if field != 1 {
            one.last_document_state = None;
        }
        if field != 2 {
            one.variables.defs.clear();
        }
        if field != 3 {
            one.slices.list.clear();
        }
        let mut s = retained_session(one);
        let snapshot = s.documents()[0].doc.clone();
        let events = s.file_menu.event_log.clone();
        let journal = s.journal.clone();
        let h = history(&s);
        if field < 2 {
            // A merged duplicate has no layer-comp continuation; only those
            // obsolete comp holders are dropped, without changing the source.
            s.execute("image.duplicate", serde_json::json!({"mergedOnly":true})).unwrap();
            assert_eq!(s.documents().len(), 2);
            let copied = &s.documents()[1].doc;
            assert_eq!(copied.layers.len(), 1);
            assert!(copied.layer_comps.is_empty());
            assert!(copied.last_document_state.is_none());
            assert!(copied.last_applied_comp.is_none());
            assert!(s.documents()[0].doc.shares_state(&snapshot));
            assert_eq!(history(&s), h);
            assert_eq!(archives(&s.documents()[0].doc).unwrap(), archives(&snapshot).unwrap());
            assert_eq!(&s.journal[..journal.len()], journal.as_slice());
            assert_eq!(s.journal.last().unwrap().0, "image.duplicate");
            roundtrip(copied).unwrap();
            continue;
        }
        let err = s.execute("image.duplicate", serde_json::json!({"mergedOnly":true})).expect_err("merged variable/slice association is unqualified");
        assert!(err.to_string().contains("duplicate"));
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.file_menu.event_log, events);
        assert_eq!(s.journal, journal);
        assert!(s.documents()[0].doc.shares_state(&snapshot));
        assert_eq!(history(&s), h);
    }
    doc.layer_comps.clear();
    doc.last_document_state = None;
    doc.variables.defs.clear();
    doc.slices.list.clear();
    let mut s = retained_session(doc);
    s.execute("image.duplicate", serde_json::json!({"mergedOnly":true})).unwrap();
    assert_eq!(s.documents()[1].doc.layers.len(), 1);
    roundtrip(&s.documents()[1].doc).unwrap();
}

#[cfg(feature = "corpus")]
#[path = "../../io/tests/common/red_eye_geometry_packet.rs"]
#[allow(dead_code)] // Shared packet wrapper exposes additional test domains.
mod packet;
#[cfg(feature = "corpus")]
#[test]
#[ignore = "hash-bound independently authored owner geometry XCF"]
fn merged_qualified_xcf_refuses_without_install_or_native_source_mutation() {
    let (root, records) = packet::fresh_records().unwrap();
    for row in records {
        let bytes = packet::fresh_artifact(&root, &row["saved"]).unwrap();
        let doc = photocraft_io::import("actual.xcf", &bytes).unwrap().document;
        doc.validate_xcf_state().unwrap();
        let mut s = retained_session(doc);
        let snapshot = s.documents()[0].doc.clone();
        let native = archives(&snapshot).unwrap();
        let events = s.file_menu.event_log.clone();
        let journal = s.journal.clone();
        let h = history(&s);
        let err = s.execute("image.duplicate", serde_json::json!({"mergedOnly":true})).expect_err("merged imported current state is unqualified");
        assert!(err.to_string().contains("duplicate"));
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.file_menu.event_log, events);
        assert_eq!(s.journal, journal);
        assert!(s.documents()[0].doc.shares_state(&snapshot));
        assert_eq!(history(&s), h);
        assert!(archives(&s.documents()[0].doc).unwrap() == native, "source native bytes changed");
    }
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "hash-bound independently authored owner geometry XCF"]
fn ordinary_qualified_xcf_preserves_all_original_payloads_and_native_roundtrips() {
    let (root, records) = packet::fresh_records().unwrap();
    for row in records {
        let bytes = packet::fresh_artifact(&root, &row["saved"]).unwrap();
        let source = photocraft_io::import("actual.xcf", &bytes).unwrap().document;
        let mut s = retained_session(source.clone());
        let snapshot = s.documents()[0].doc.clone();
        let h = history(&s);
        let native = archives(&snapshot).unwrap();
        let before_pixels = photocraft_compose::flatten(&snapshot).unwrap().px;
        s.execute("image.duplicate", serde_json::json!({})).unwrap();
        assert!(s.documents()[1].source.is_none() && s.documents()[1].path.is_none());
        let out = &s.documents()[1].doc;
        out.validate_xcf_state().unwrap();
        roundtrip(out).unwrap();
        assert!(photocraft_compose::flatten(out).unwrap().px == before_pixels, "duplicate changed composed pixels");
        let old = source.walk();
        let new = out.walk();
        let map = old.iter().zip(&new).map(|(a, b)| (a.2.id, b.2.id)).collect::<std::collections::HashMap<_, _>>();
        let mut expected = source.clone();
        expected.id = out.id;
        expected.name = out.name.clone();
        for l in &mut expected.layers {
            l.id = map[&l.id];
        }
        expected.xcf_state.as_mut().unwrap().remap_layers(|id| map.get(&id).copied()).unwrap();
        assert!(**out == expected, "original imported fields/payloads changed");
        assert!(s.documents()[0].doc.shares_state(&snapshot));
        assert_eq!(history(&s), h);
        assert!(archives(&s.documents()[0].doc).unwrap() == native, "source native bytes changed");
    }
}

#[test]
fn excessive_typed_reference_work_refuses_before_installation() {
    let mut s = retained_session(document(SampleType::U8).unwrap());
    let mut hostile = (*s.documents()[0].doc).clone();
    hostile.xcf_state.as_mut().unwrap().selection.selected = vec![ItemRef::Layer(LayerId(1)); 100_001];
    s.active_mut().unwrap().doc = photocraft_engine::DocumentSnapshot::from_resident(std::sync::Arc::new(hostile)).unwrap();
    let snapshot = s.documents()[0].doc.clone();
    let h = history(&s);
    let events = s.file_menu.event_log.clone();
    let journal = s.journal.clone();
    let error = s.execute("image.duplicate", serde_json::json!({})).expect_err("must refuse before traversing excess references");
    assert!(error.to_string().contains("image.duplicate: reference limit"));
    assert_eq!(s.documents().len(), 1);
    assert!(s.documents()[0].doc.shares_state(&snapshot));
    assert_eq!(history(&s), h);
    assert_eq!(s.file_menu.event_log, events);
    assert_eq!(s.journal, journal);
}
