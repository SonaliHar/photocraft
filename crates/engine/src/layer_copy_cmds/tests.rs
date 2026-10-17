use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId, Pattern};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::Session;

/// Document 0 (`w`×`h`, `depth`) with a red 10×10 square at (4, 6) on layer "paint" above the
/// Background, and document 1 (60×50, 8-bit RGB). Document 0 is active.
fn two_docs(w: u32, h: u32, depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 60, "height": 50})).unwrap();
    s.set_active(0);
    (s, paint)
}

fn doc(s: &Session, i: usize) -> &Document {
    &s.documents()[i].doc
}

fn copied(r: &Value) -> LayerId {
    LayerId(r["layers"][0].as_u64().unwrap())
}

fn bounds(s: &Session, i: usize, id: LayerId) -> Rect {
    crate::layer_multi_cmds::layer_bounds(doc(s, i).layer(id).unwrap()).unwrap()
}

#[test]
fn copies_the_selected_layers_above_the_destinations_active_layer_in_one_step() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let source_before = doc(&s, 0).clone();
    let r = s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
    assert_eq!(r["document"], 1);
    assert_eq!(s.active_index(), Some(1), "the destination becomes the active document");
    let d = doc(&s, 1);
    let id = copied(&r);
    assert_ne!(id, paint, "a copy has its own id");
    assert_eq!(d.layers.len(), 2);
    assert_eq!(d.layers[1].id, id, "above the Background, the destination's active layer");
    assert_eq!(d.layers[1].name, "paint");
    assert_eq!(s.active().unwrap().active_layer, Some(id));
    assert_eq!(bounds(&s, 1, id), Rect::new(4, 6, 14, 16), "same canvas position by default");
    assert_eq!(d.layers[1].surface().unwrap().read_region(Rect::new(5, 7, 6, 8)), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(*doc(&s, 0), source_before, "the source is untouched");
    // One history step in the destination.
    assert!(s.undo());
    assert_eq!(doc(&s, 1).layers.len(), 1);
}

#[test]
fn converts_to_the_destinations_depth_and_colour_mode() {
    for depth in [8, 16, 32] {
        let (mut s, _) = two_docs(40, 30, depth);
        s.set_active(1);
        s.execute("image.mode.grayscale", json!({})).unwrap();
        s.execute("image.mode.bits16", json!({})).unwrap();
        s.set_active(0);
        let r = s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
        let id = copied(&r);
        let surf = doc(&s, 1).layer(id).unwrap().surface().unwrap();
        assert_eq!(surf.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true), "from {depth}-bit RGB");
        let px = surf.read_region(Rect::new(5, 7, 6, 8));
        assert!(px[0] > 0.1 && px[0] < 0.9 && px[1] == 1.0, "red as a mid grey, opaque: {px:?}");
    }
}

#[test]
fn places_the_copies_centred_on_the_canvas_on_a_point_or_offset() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let place = |s: &mut Session, p: Value| {
        s.set_active(0);
        let mut p = p;
        p["document"] = json!(1);
        p["layers"] = json!([paint.0]);
        let r = s.execute("layer.copyToDocument", p).unwrap();
        bounds(s, 1, copied(&r))
    };
    assert_eq!(place(&mut s, json!({"center": true})), Rect::new(25, 20, 35, 30));
    assert_eq!(place(&mut s, json!({"at": [10.0, 40.0]})), Rect::new(5, 35, 15, 45));
    assert_eq!(place(&mut s, json!({"offset": [-4, 20]})), Rect::new(0, 26, 10, 36));
    assert_eq!(doc(&s, 1).layers.len(), 4);
}

#[test]
fn a_copied_background_is_an_ordinary_layer_and_several_layers_keep_their_order() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let bg = doc(&s, 0).layers[0].id;
    let r = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [paint.0, bg]})).unwrap();
    let d = doc(&s, 1);
    let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "Layer 1", "paint"]);
    assert!(!d.layers[1].locks.transparency && !d.layers[1].locks.position);
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    assert_eq!(s.active().unwrap().selected_layers().unwrap().len(), 2, "the copies are selected");
}

#[test]
fn the_sources_patterns_come_along_once() {
    let (mut s, _) = two_docs(40, 30, 8);
    let mut tile = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    tile.fill_rect(Rect::new(0, 0, 2, 2), &[0.0, 1.0, 0.0, 1.0]);
    let pat = Pattern::new("dots", tile, 2, 2);
    s.edit("add pattern", |d, _| {
        d.patterns.push(pat.clone());
        Ok(())
    })
    .unwrap();
    s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
    assert_eq!(doc(&s, 1).patterns.iter().filter(|p| p.id == pat.id).count(), 1);
    s.set_active(0);
    s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
    assert_eq!(doc(&s, 1).patterns.iter().filter(|p| p.id == pat.id).count(), 1, "not twice");
}

#[test]
fn bad_params_and_states_fail_without_changing_anything() {
    let (mut s, _) = two_docs(40, 30, 8);
    let cmd = "layer.copyToDocument";
    for p in [
        json!({}),
        json!({"document": 0}),
        json!({"document": 7}),
        json!({"document": -1}),
        json!({"document": "1"}),
        json!({"document": 1, "source": 9}),
        json!({"document": 1, "layers": [u64::MAX]}),
        json!({"document": 1, "layers": "all"}),
        json!({"document": 1, "layers": []}),
        json!({"document": 1, "at": [1.0]}),
        json!({"document": 1, "at": [f64::MAX, 0.0]}),
        json!({"document": 1, "offset": ["a", 2]}),
        Value::Null,
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{p}");
        assert_eq!(s.active_index(), Some(0), "{p}: the source stays active");
        assert_eq!(doc(&s, 1).layers.len(), 1, "{p}");
    }
    // An Indexed destination has no layers to add to.
    s.set_active(1);
    s.edit("indexed", |d, _| {
        d.mode = ColorMode::Indexed;
        Ok(())
    })
    .unwrap();
    s.set_active(0);
    assert!(s.execute(cmd, json!({"document": 1})).is_err());
    // A single open document: nothing to copy to.
    s.close(1);
    assert!(!s.is_enabled(cmd));
}

fn imported_copy_fixture() -> (Session, LayerId, LayerId) {
    use photocraft_doc::{Color, Layer, Size, XcfState, item_identity::ItemRef, xcf_provenance::OpaqueRecord};
    let mut source = Document::with_background("Own imported source", Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    source.layers[0].locks = Default::default();
    let imported = source.layers[0].id;
    let ordinary = Layer::raster("Same name", source.pixel_format());
    let ordinary_id = ordinary.id;
    source.layers[0].name = "Same name".into();
    source.layers.push(ordinary);
    let mut state = XcfState::default();
    state.reconcile_layers(&source).unwrap();
    let bytes = b"Own inert imported source metadata".to_vec();
    state.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Layer(imported)),
        namespace: "own".into(),
        identity: "own".into(),
        source_version: 22,
        order: 0,
        flags: 3,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    source.xcf_state = Some(state);
    source.validate_xcf_state().unwrap();
    let native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), source);
    let destination = Document::with_background("Own destination", Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = Session::new();
    s.add_document(source, None).unwrap();
    s.add_document(destination, None).unwrap();
    for index in [0, 1] {
        s.set_active(index);
        s.edit("Own redo seed", |doc, _| {
            doc.name.push('!');
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
    }
    s.set_active(0);
    s.select_layer(imported).unwrap();
    (s, imported, ordinary_id)
}

#[test]
fn imported_cross_document_copy_refuses_owned_metadata_loss_atomically() {
    let (mut s, imported, _) = imported_copy_fixture();
    let before: Vec<_> = s
        .documents()
        .iter()
        .map(|st| {
            (
                st.doc.clone(),
                st.revision,
                st.saved_revision,
                st.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
                st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect();
    let native: Vec<_> = s.documents().iter().map(|st| photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap()).collect();
    let result = s.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[imported.0]}));
    if result.is_ok() {
        assert!(s.documents()[1].doc.xcf_state.is_none(), "unexpected destination imported state");
    }
    assert!(result.is_err(), "registered copy succeeded and dropped layer-owned XCF metadata; source native evidence was validated");
    assert!(result.unwrap_err().to_string().contains("imported layer metadata transfer is unsupported"));
    assert_eq!(s.active_index(), Some(0));
    for (index, (st, expected)) in s.documents().iter().zip(&before).enumerate() {
        assert!(st.doc == expected.0);
        assert_eq!((st.revision, st.saved_revision), (expected.1, expected.2));
        assert_eq!(st.history.entry_labels().collect::<Vec<_>>(), expected.3.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), expected.4.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap(), native[index]);
    }
    for index in [0, 1] {
        s.set_active(index);
        assert!(s.redo());
        assert!(s.documents()[index].doc.name.ends_with('!'));
        assert!(s.undo());
        assert!(s.documents()[index].doc == before[index].0);
    }
}

fn assert_imported_copy_refusal(s: &mut Session, layer: LayerId) {
    let before: Vec<_> = s
        .documents()
        .iter()
        .map(|st| (st.doc.clone(), st.revision, st.saved_revision, st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>()))
        .collect();
    let active = s.active_index();
    let error = s.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[layer.0]})).unwrap_err();
    assert!(error.to_string().contains("imported layer metadata transfer is unsupported"), "{error}");
    assert_eq!(s.active_index(), active);
    for (st, expected) in s.documents().iter().zip(before) {
        assert!(st.doc == expected.0);
        assert_eq!((st.revision, st.saved_revision), (expected.1, expected.2));
        assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), expected.3.iter().map(String::as_str).collect::<Vec<_>>());
    }
}

#[test]
fn unrelated_ordinary_layer_with_same_name_remains_copyable() {
    let (mut s, _, ordinary) = imported_copy_fixture();
    let before = doc(&s, 0).clone();
    let result = s.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[ordinary.0]})).unwrap();
    assert_eq!(doc(&s, 1).layer(copied(&result)).unwrap().name, "Same name");
    assert_eq!(doc(&s, 0), &before);
    assert!(doc(&s, 1).xcf_state.is_none());
}

#[test]
fn imported_metadata_on_nested_descendant_refuses_group_transfer() {
    let (mut s, imported, _) = imported_copy_fixture();
    let group = s
        .edit("Own group", |d, _| {
            let child = d.remove(imported).unwrap();
            let group = photocraft_doc::Layer::group("Own group", vec![child]);
            let id = group.id;
            d.layers.push(group);
            Ok(id)
        })
        .unwrap();
    doc(&s, 0).validate_xcf_state().unwrap();
    assert_imported_copy_refusal(&mut s, group);
}

#[test]
fn inherited_imported_visibility_lock_refuses_an_ordinary_child() {
    use photocraft_doc::item_identity::ItemRef;
    let (mut s, _, ordinary) = imported_copy_fixture();
    s.edit("Own inherited lock", |d, _| {
        let child = d.remove(ordinary).unwrap();
        let group = photocraft_doc::Layer::group("Own locked group", vec![child]);
        let id = group.id;
        d.layers.push(group);
        let mut state = d.xcf_state.take().unwrap();
        state.reconcile_layers(d).unwrap();
        state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().locks.visibility = true;
        d.xcf_state = Some(state);
        Ok(())
    })
    .unwrap();
    doc(&s, 0).validate_xcf_state().unwrap();
    assert_imported_copy_refusal(&mut s, ordinary);
}

#[test]
fn selected_imported_item_fields_and_disabled_filters_are_not_dropped() {
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::{FilterId, ItemRef},
        xcf_filters::{FilterNode, FilterStack, OperationIdentity},
    };
    for case in 0..6 {
        let (mut s, imported, _) = imported_copy_fixture();
        s.edit("Own association", |d, _| {
            let state = d.xcf_state.as_mut().unwrap();
            state.opaque.clear();
            let item = state.items.iter_mut().find(|item| item.item == ItemRef::Layer(imported)).unwrap();
            match case {
                0 => item.source_tattoo = Some(17),
                1 => item.source_flags = 1,
                2 => item.raster_bounds = Some(Rect::new(0, 0, 2, 1)),
                3 => item.locks.content = true,
                4 => {
                    item.composition = Some(GimpComposition {
                        compatibility_epoch: "own-unresolved".into(),
                        source_mode: 0,
                        legacy: false,
                        stored_blend_space: 0,
                        stored_composite_space: 0,
                        stored_operator: 0,
                        resolved: None,
                        pass_through: false,
                    })
                }
                _ => state.filter_stacks.push(FilterStack {
                    owner: ItemRef::Layer(imported),
                    revision: 0,
                    nodes: vec![FilterNode {
                        id: FilterId(1),
                        operation: OperationIdentity { name: "own-disabled".into(), stored_version: "1".into(), schema_epoch: "own".into() },
                        source_display_name: None,
                        source_icon: None,
                        arguments: vec![],
                        enabled: false,
                        opacity_f64_bits: 1.0f64.to_bits(),
                        source_clipping: None,
                        source_region: None,
                        mask: None,
                        auxiliary: vec![],
                        composition: None,
                    }],
                }),
            }
            Ok(())
        })
        .unwrap();
        doc(&s, 0).validate_xcf_state().unwrap();
        assert_imported_copy_refusal(&mut s, imported);
    }
}

#[test]
fn selected_vector_source_binding_refuses_transfer_but_unrelated_paths_are_shared_only_in_source() {
    use photocraft_doc::{
        NamedPath, Path,
        item_identity::ItemRef,
        xcf_items::{EditableSource, SourceItem},
    };
    let (mut s, imported, ordinary) = imported_copy_fixture();
    s.edit("Own source binding", |d, _| {
        let path = NamedPath::new("Same name", Path::default());
        let id = path.id;
        d.paths.push(path);
        let state = d.xcf_state.as_mut().unwrap();
        state.opaque.clear();
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(imported),
            source: EditableSource::Vector { path: id, source_modified: true, configuration: vec![] },
            transform: None,
            cache: None,
        });
        Ok(())
    })
    .unwrap();
    doc(&s, 0).validate_xcf_state().unwrap();
    assert_imported_copy_refusal(&mut s, imported);
    s.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[ordinary.0]})).unwrap();
}

#[test]
fn channel_with_same_numeric_id_does_not_make_an_ordinary_layer_imported() {
    use photocraft_doc::{
        AlphaChannel,
        item_identity::{ChannelId, ItemRef},
    };
    let (mut s, imported, _) = imported_copy_fixture();
    s.edit("Own typed association", |d, _| {
        let mut channel = AlphaChannel::new("Same name", Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)));
        channel.id = ChannelId(imported.0);
        d.channels.push(channel);
        d.xcf_state.as_mut().unwrap().opaque[0].owner = Some(ItemRef::Channel(ChannelId(imported.0)));
        Ok(())
    })
    .unwrap();
    doc(&s, 0).validate_xcf_state().unwrap();
    s.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[imported.0]})).unwrap();
}

#[test]
fn unknown_imported_owner_is_rejected_without_name_or_index_recovery() {
    use photocraft_doc::item_identity::ItemRef;
    let (s, imported, _) = imported_copy_fixture();
    let mut source = doc(&s, 0).clone();
    source.xcf_state.as_mut().unwrap().opaque[0].owner = Some(ItemRef::Layer(LayerId(u64::MAX)));
    let before = source.clone();
    let error = super::qualify_imported_transfer(&source, &[imported]).unwrap_err();
    assert!(error.to_string().contains("invalid imported source state"));
    assert_eq!(source, before);
}

#[test]
fn explicit_source_refusal_keeps_the_other_active_document() {
    let (mut s, imported, _) = imported_copy_fixture();
    s.set_active(1);
    assert_imported_copy_refusal(&mut s, imported);
}
fn current_encoding_copy_fixture(
    source_encoding: photocraft_doc::SampleEncoding,
    destination_encoding: photocraft_doc::SampleEncoding,
    unrelated: bool,
) -> (Session, LayerId) {
    use photocraft_doc::{Layer, Size, SourceColorContext};
    let mut source = Document::new("Own current source", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    source.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    source.sample_encoding = Some(source_encoding);
    source.source_color_context =
        Some(SourceColorContext { original_icc: Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes()), source_precision: 675 });
    let mut layer = Layer::raster("Same name", source.pixel_format());
    layer.surface_mut().unwrap().fill_rect(source.bounds(), &[0.25, 0.25, 0.25, 1.0]);
    let selected = layer.id;
    source.layers.push(layer);
    if unrelated {
        use photocraft_doc::{XcfState, item_identity::ItemRef, xcf_provenance::OpaqueRecord};
        let layer = Layer::raster("Same name", source.pixel_format());
        let other = layer.id;
        source.layers.push(layer);
        let mut state = XcfState::default();
        state.reconcile_layers(&source).unwrap();
        let bytes = b"Own unrelated imported metadata".to_vec();
        state.opaque.push(OpaqueRecord {
            owner: Some(ItemRef::Layer(other)),
            namespace: "own".into(),
            identity: "own".into(),
            source_version: 22,
            order: 0,
            flags: 0,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes: bytes.into(),
            rendering_required: false,
        });
        source.xcf_state = Some(state);
        source.validate_xcf_state().unwrap();
    }
    let mut destination = Document::new("Different current destination", source.size, source.mode, source.depth);
    destination.icc_profile = source.icc_profile.clone();
    destination.sample_encoding = Some(destination_encoding);
    destination.layers.push(Layer::raster("Own transparent destination", destination.pixel_format()));
    for doc in [&source, &destination] {
        let saved = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), *doc);
    }
    let mut session = Session::new();
    session.add_document(source, None).unwrap();
    session.add_document(destination, None).unwrap();
    for index in [0, 1] {
        session.set_active(index);
        session
            .edit("Own current redo seed", |doc, _| {
                doc.name.push('!');
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
    }
    session.set_active(0);
    session.select_layer(selected).unwrap();
    (session, selected)
}

#[test]
fn different_explicit_current_sample_encodings_refuse_atomically() {
    use photocraft_doc::SampleEncoding::{Linear, NonlinearProfile, Perceptual};
    for (source, destination) in [
        (Linear, NonlinearProfile),
        (NonlinearProfile, Linear),
        (Linear, Perceptual),
        (Perceptual, Linear),
        (NonlinearProfile, Perceptual),
        (Perceptual, NonlinearProfile),
    ] {
        let (mut session, selected) = current_encoding_copy_fixture(source, destination, false);
        let before: Vec<_> = session
            .documents()
            .iter()
            .map(|state| {
                (
                    state.doc.clone(),
                    state.revision,
                    state.saved_revision,
                    state.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
                    state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),
                )
            })
            .collect();
        let native: Vec<_> = before.iter().map(|state| photocraft_format::save_to_bytes(&state.0, &Default::default()).unwrap()).collect();
        session.set_active(1);
        let result = session.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[selected.0]}));
        assert!(result.is_err(), "registered copy reinterpreted identical native words under unequal current encodings");
        assert!(result.unwrap_err().to_string().contains("current sample encoding transfer is unsupported"));
        assert_eq!(session.active_index(), Some(1));
        for (index, (state, expected)) in session.documents().iter().zip(&before).enumerate() {
            assert_eq!(state.doc, expected.0);
            assert_eq!((state.revision, state.saved_revision), (expected.1, expected.2));
            assert_eq!(state.history.entry_labels().collect::<Vec<_>>(), expected.3.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), expected.4.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native[index]);
        }
        for index in [0, 1] {
            session.set_active(index);
            assert!(session.redo());
            assert!(session.active().unwrap().doc.name.ends_with('!'));
            assert!(session.undo());
            assert_eq!(session.active().unwrap().doc, before[index].0);
        }
    }
}

#[test]
fn equal_current_encoding_ignores_original_profile_names_and_unrelated_imported_owners() {
    use photocraft_doc::SampleEncoding::{Linear, NonlinearProfile, Perceptual};
    for encoding in [Linear, NonlinearProfile, Perceptual] {
        let (mut session, selected) = current_encoding_copy_fixture(encoding, encoding, true);
        let source = doc(&session, 0).clone();
        let rendered = photocraft_compose::gimp_render::try_render(&source, source.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let source_display = session.color.canvas_display(&source).unwrap().to_rgba8(&rendered).pixels;
        let result = session.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[selected.0]})).unwrap();
        let destination = doc(&session, 1);
        let copied_layer = destination.layer(copied(&result)).unwrap();
        assert_eq!(copied_layer.surface().unwrap().read_region(destination.bounds()), vec![0.25, 0.25, 0.25, 1.0]);
        let rendered = photocraft_compose::gimp_render::try_render(destination, destination.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
        assert_eq!(session.color.canvas_display(destination).unwrap().to_rgba8(&rendered).pixels, source_display);
        assert_eq!(doc(&session, 0), &source);
        assert_ne!(source.source_color_context, destination.source_color_context);
        assert!(destination.xcf_state.is_none());
    }
}
fn current_profile_copy_fixture(
    encoding: photocraft_doc::SampleEncoding,
    source_implicit: bool,
    source_p3: bool,
    target_implicit: bool,
    target_p3: bool,
) -> (Session, LayerId) {
    use photocraft_cms::{
        Builtin, ColorSpace,
        xcf_color::{Encoding, effective_profile},
    };
    use photocraft_doc::{Layer, Size, SourceColorContext, WorkingProfileInterpretation};
    let encoded = match encoding {
        photocraft_doc::SampleEncoding::Linear => Encoding::Linear,
        photocraft_doc::SampleEncoding::NonlinearProfile => Encoding::NonlinearProfile,
        photocraft_doc::SampleEncoding::Perceptual => Encoding::Perceptual,
    };
    let make = |name: &str, implicit: bool, p3: bool| {
        let mut doc = Document::new(name, Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        let bytes = if p3 { Builtin::DisplayP3.profile().to_bytes() } else { effective_profile(None, ColorSpace::Rgb, encoded).unwrap().to_bytes() };
        if implicit {
            doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&bytes);
        }
        doc.icc_profile = Some(bytes);
        doc.sample_encoding = Some(encoding);
        crate::color_cmds::try_document_profile(&doc).unwrap();
        doc
    };
    let mut source = make("Own current profile source", source_implicit, source_p3);
    source.source_color_context = Some(SourceColorContext { original_icc: Some(Builtin::AdobeRgbCompat.profile().to_bytes()), source_precision: 675 });
    let pixel = match encoding {
        photocraft_doc::SampleEncoding::NonlinearProfile => [0.014_515_731, 0.507_828_8, 0.611_580_5, 1.0],
        photocraft_doc::SampleEncoding::Perceptual => [0.014_492_448, 0.507_824_4, 0.611_576_9, 1.0],
        photocraft_doc::SampleEncoding::Linear => [0.15, 0.5, 0.7, 1.0],
    };
    let mut layer = Layer::raster("Own bounded current pixel", source.pixel_format());
    layer.surface_mut().unwrap().fill_rect(source.bounds(), &pixel);
    let selected = layer.id;
    source.layers.push(layer);
    let mut target = make("Different current profile destination", target_implicit, target_p3);
    target.layers.push(Layer::raster("Own transparent destination", target.pixel_format()));
    for doc in [&source, &target] {
        let native = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *doc);
    }
    let mut session = Session::new();
    session.add_document(source, None).unwrap();
    session.add_document(target, None).unwrap();
    for index in [0, 1] {
        session.set_active(index);
        session
            .edit("Own current profile redo seed", |doc, _| {
                doc.name.push('!');
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
    }
    session.set_active(0);
    session.select_layer(selected).unwrap();
    (session, selected)
}

fn assert_current_profile_copy_reference(session: &mut Session, selected: LayerId) {
    let source = doc(session, 0).clone();
    let target = doc(session, 1).clone();
    let source_history = (
        session.documents()[0].revision,
        session.documents()[0].saved_revision,
        session.documents()[0].history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
        session.documents()[0].history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),
    );
    let source_native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let from = crate::color_cmds::try_document_profile(&source).unwrap();
    let to = crate::color_cmds::try_document_profile(&target).unwrap();
    let transform = photocraft_cms::Transform::new(&from, &to, session.color.settings.intent(), session.color.settings.bpc).unwrap();
    let reference =
        crate::color_cmds::convert_surface(source.layer(selected).unwrap().surface().unwrap(), source.mode, target.pixel_format(), &transform).unwrap();
    session.set_active(1);
    let result = session.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[selected.0]})).unwrap();
    let actual = doc(session, 1).clone();
    let id = copied(&result);
    let mut expected = actual.clone();
    *expected.layer_mut(id).unwrap().surface_mut().unwrap() = reference;
    let actual_words = actual.layer(id).unwrap().surface().unwrap().read_region(actual.bounds());
    let expected_words = expected.layer(id).unwrap().surface().unwrap().read_region(expected.bounds());
    let display = |d: &Document| {
        let rendered = photocraft_compose::gimp_render::try_render(d, d.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
        session.color.canvas_display(d).unwrap().to_rgba8(&rendered).pixels
    };
    let actual_canvas = display(&actual);
    let expected_canvas = display(&expected);
    assert_eq!(
        actual_words, expected_words,
        "registered copy failed exact checked CURRENT profile conversion; actual CPU {actual_canvas:?}, checked CPU {expected_canvas:?}"
    );
    assert_eq!(actual_canvas, expected_canvas);
    assert_eq!(doc(session, 0), &source);
    assert_eq!(photocraft_format::save_to_bytes(doc(session, 0), &Default::default()).unwrap(), source_native);
    let state = &session.documents()[0];
    assert_eq!(
        (
            state.revision,
            state.saved_revision,
            state.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
            state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>()
        ),
        source_history
    );
    for d in [&actual, &expected] {
        let bytes = photocraft_format::save_to_bytes(d, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *d);
    }
    assert!(session.undo());
    assert_eq!(doc(session, 1), &target);
    assert!(session.redo());
    assert_eq!(doc(session, 1), &actual);
    session.set_active(0);
    assert!(session.redo());
    assert!(doc(session, 0).name.ends_with('!'));
    assert!(session.undo());
    assert_eq!(doc(session, 0), &source);
}

#[test]
fn current_working_profile_nonlinear_implicit_copy_matches_checked_conversion() {
    let (mut session, selected) = current_profile_copy_fixture(photocraft_doc::SampleEncoding::NonlinearProfile, true, false, false, true);
    assert_current_profile_copy_reference(&mut session, selected);
}

#[test]
fn current_working_profile_perceptual_implicit_copy_matches_checked_conversion() {
    let (mut session, selected) = current_profile_copy_fixture(photocraft_doc::SampleEncoding::Perceptual, true, false, false, true);
    assert_current_profile_copy_reference(&mut session, selected);
}

#[test]
fn current_working_profile_perceptual_icc_copy_matches_checked_conversion() {
    let (mut session, selected) = current_profile_copy_fixture(photocraft_doc::SampleEncoding::Perceptual, false, false, false, true);
    assert_current_profile_copy_reference(&mut session, selected);
}

#[test]
fn current_profile_copy_preserves_same_icc_markers_linear_and_implicit_targets() {
    use photocraft_doc::SampleEncoding::{Linear, NonlinearProfile, Perceptual};
    for encoding in [Linear, NonlinearProfile, Perceptual] {
        for (source_implicit, target_implicit) in [(false, false), (false, true), (true, false), (true, true)] {
            let (mut session, selected) = current_profile_copy_fixture(encoding, source_implicit, false, target_implicit, false);
            let source = doc(&session, 0).clone();
            let source_render = photocraft_compose::gimp_render::try_render(&source, source.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
            let before_canvas = session.color.canvas_display(&source).unwrap().to_rgba8(&source_render).pixels;
            let result = session.execute("layer.copyToDocument", json!({"document":1,"layers":[selected.0]})).unwrap();
            let target = doc(&session, 1);
            assert_eq!(
                target.layer(copied(&result)).unwrap().surface().unwrap().read_region(target.bounds()),
                source.layers[0].surface().unwrap().read_region(source.bounds())
            );
            let rendered = photocraft_compose::gimp_render::try_render(target, target.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
            assert_eq!(session.color.canvas_display(target).unwrap().to_rgba8(&rendered).pixels, before_canvas);
            assert_eq!(doc(&session, 0), &source);
        }
        let (mut session, selected) = current_profile_copy_fixture(encoding, false, true, true, false);
        assert_current_profile_copy_reference(&mut session, selected);
    }
    let (mut session, selected) = current_profile_copy_fixture(Linear, true, false, false, true);
    assert_current_profile_copy_reference(&mut session, selected);
}
#[test]
fn malformed_current_conversion_target_fails_without_document_or_history_mutation() {
    let (mut session, selected) = current_profile_copy_fixture(photocraft_doc::SampleEncoding::NonlinearProfile, true, false, false, true);
    session.set_active(1);
    session
        .edit("Own malformed assigned ICC", |doc, _| {
            doc.icc_profile = Some(std::sync::Arc::new(b"Own malformed current ICC".to_vec()));
            Ok(())
        })
        .unwrap();
    session
        .edit("Own malformed redo seed", |doc, _| {
            doc.name.push('!');
            Ok(())
        })
        .unwrap();
    assert!(session.undo());
    let before: Vec<_> = session
        .documents()
        .iter()
        .map(|state| {
            (
                state.doc.clone(),
                state.revision,
                state.saved_revision,
                state.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
                state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect();
    let native: Vec<_> = before.iter().map(|state| photocraft_format::save_to_bytes(&state.0, &Default::default()).unwrap()).collect();
    let error = session.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[selected.0]})).unwrap_err();
    assert!(error.to_string().contains("colour management"), "{error}");
    assert_eq!(session.active_index(), Some(1));
    for (index, (state, expected)) in session.documents().iter().zip(&before).enumerate() {
        assert_eq!(state.doc, expected.0);
        assert_eq!((state.revision, state.saved_revision), (expected.1, expected.2));
        assert_eq!(state.history.entry_labels().collect::<Vec<_>>(), expected.3.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), expected.4.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native[index]);
    }
    for index in [0, 1] {
        session.set_active(index);
        assert!(session.redo());
        assert!(doc(&session, index).name.ends_with('!'));
        assert!(session.undo());
        assert_eq!(doc(&session, index), before[index].0.as_ref());
    }
}

#[test]
fn ordinary_cmyk_and_lab_copy_targets_keep_the_legacy_profile_route() {
    for mode in [ColorMode::Cmyk, ColorMode::Lab] {
        let mut source = Document::new("Own ordinary RGB", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        let mut layer = photocraft_doc::Layer::raster("Own ordinary layer", source.pixel_format());
        layer.surface_mut().unwrap().fill_rect(source.bounds(), &[0.4, 0.5, 0.6, 1.0]);
        let selected = layer.id;
        source.layers.push(layer);
        let mut target = Document::new("Own legacy target", source.size, mode, source.depth);
        target.layers.push(photocraft_doc::Layer::raster("Own transparent", target.pixel_format()));
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.add_document(target, None).unwrap();
        session.set_active(0);
        session.select_layer(selected).unwrap();
        let result = session.execute("layer.copyToDocument", json!({"document":1})).unwrap();
        let copied = doc(&session, 1).layer(copied(&result)).unwrap();
        assert_eq!(copied.surface().unwrap().format().mode, mode);
        assert!(copied.surface().unwrap().read_region(source.bounds()).iter().all(|sample| sample.is_finite()));
        assert_eq!(doc(&session, 0), &source);
        let native = photocraft_format::save_to_bytes(doc(&session, 1), &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *doc(&session, 1));
    }
}

fn mixed_legacy_copy_fixture(
    mode: ColorMode,
    depths: [SampleType; 2],
    encodings: [Option<photocraft_doc::SampleEncoding>; 2],
    bytes: Option<std::sync::Arc<Vec<u8>>>,
    implicit: bool,
) -> (Session, LayerId) {
    use photocraft_doc::{Layer, Size, WorkingProfileInterpretation};
    let mut session = Session::new();
    let mut selected = LayerId(0);
    for index in 0..2 {
        let mut d = Document::new(if index == 0 { "Own mixed source" } else { "Different mixed destination" }, Size::new(1, 1), mode, depths[index]);
        d.sample_encoding = encodings[index];
        d.icc_profile = bytes.clone();
        if implicit {
            d.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(bytes.as_ref().unwrap());
        }
        let mut layer = Layer::raster("Own mixed pixel", d.pixel_format());
        if index == 0 {
            selected = layer.id;
            layer.surface_mut().unwrap().fill_rect(d.bounds(), if mode == ColorMode::Rgb { &[0.25, 0.5, 0.75, 1.0] } else { &[0.25, 1.0] });
        }
        d.layers.push(layer);
        let native = photocraft_format::save_to_bytes(&d, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), d);
        session.add_document(d, None).unwrap();
    }
    for index in [0, 1] {
        session.set_active(index);
        session
            .edit("Own mixed real redo seed", |d, _| {
                d.name.push('!');
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        assert_eq!(session.active().unwrap().history.redo_labels().collect::<Vec<_>>(), vec!["Own mixed real redo seed"]);
    }
    session.set_active(1);
    let target_layer = doc(&session, 1).layers[0].id;
    session.select_layer(target_layer).unwrap();
    (session, selected)
}

fn assert_mixed_copy_atomic_refusal(session: &mut Session, selected: LayerId, expected_error: Option<&str>) {
    let before: Vec<_> = session
        .documents()
        .iter()
        .map(|s| {
            (
                s.doc.clone(),
                s.revision,
                s.saved_revision,
                s.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
                s.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect();
    let native: Vec<_> = before.iter().map(|s| photocraft_format::save_to_bytes(&s.0, &Default::default()).unwrap()).collect();
    let result = session.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[selected.0]}));
    assert!(result.is_err(), "registered copy admitted unsupported mixed current meaning: {result:?}");
    let error = result.unwrap_err().to_string();
    if let Some(expected) = expected_error {
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(session.active_index(), Some(1));
    for (index, (state, old)) in session.documents().iter().zip(&before).enumerate() {
        assert_eq!(state.doc, old.0);
        assert_eq!((state.revision, state.saved_revision), (old.1, old.2));
        assert_eq!(state.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), old.3);
        assert_eq!(state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), old.4);
        assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native[index]);
    }
    for index in [0, 1] {
        session.set_active(index);
        assert!(session.redo());
        assert!(doc(session, index).name.ends_with('!'));
        assert!(session.undo());
        assert_eq!(doc(session, index), before[index].0.as_ref());
    }
}

fn assert_mixed_copy_equivalent_admission(session: &mut Session, selected: LayerId) {
    let source = doc(session, 0).clone();
    let target = doc(session, 1).clone();
    let history = (
        session.documents()[0].revision,
        session.documents()[0].saved_revision,
        session.documents()[0].history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
        session.documents()[0].history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),
    );
    let source_native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let mut expected = vec![source.layer(selected).unwrap().clone()];
    crate::image_cmds::convert_layers_depth(&mut expected, target.depth).unwrap();
    let result = session.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[selected.0]})).unwrap();
    let actual = doc(session, 1).clone();
    assert_eq!(
        actual.layer(copied(&result)).unwrap().surface().unwrap().read_region(actual.bounds()),
        expected[0].surface().unwrap().read_region(source.bounds())
    );
    assert_eq!(actual.sample_encoding, target.sample_encoding);
    assert_eq!(actual.icc_profile, target.icc_profile);
    assert_eq!(actual.working_profile_interpretation, target.working_profile_interpretation);
    assert_eq!(doc(session, 0), &source);
    assert_eq!(photocraft_format::save_to_bytes(doc(session, 0), &Default::default()).unwrap(), source_native);
    let state = &session.documents()[0];
    assert_eq!(
        (
            state.revision,
            state.saved_revision,
            state.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
            state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>()
        ),
        history
    );
    assert!(session.documents()[1].history.redo_labels().next().is_none());
    let native = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), actual);
    assert!(session.undo());
    assert_eq!(doc(session, 1), &target);
    assert!(session.redo());
    assert_eq!(doc(session, 1), &actual);
    session.set_active(0);
    assert!(session.redo());
    assert!(doc(session, 0).name.ends_with('!'));
    assert!(session.undo());
    assert_eq!(doc(session, 0), &source);
}

fn mixed_rgb_refusal_case(profile: photocraft_cms::Builtin, explicit: photocraft_doc::SampleEncoding, reverse: bool) {
    let encodings = if reverse { [Some(explicit), None] } else { [None, Some(explicit)] };
    let (mut s, selected) = mixed_legacy_copy_fixture(ColorMode::Rgb, [SampleType::F32; 2], encodings, Some(profile.profile().to_bytes()), false);
    assert_mixed_copy_atomic_refusal(&mut s, selected, Some("mixed legacy/current sample interpretation transfer is unsupported"));
}
#[test]
fn mixed_legacy_srgb_to_linear_refuses_atomically() {
    mixed_rgb_refusal_case(photocraft_cms::Builtin::Srgb, photocraft_doc::SampleEncoding::Linear, false);
}
#[test]
fn mixed_legacy_linear_to_srgb_refuses_atomically() {
    mixed_rgb_refusal_case(photocraft_cms::Builtin::Srgb, photocraft_doc::SampleEncoding::Linear, true);
}
#[test]
fn mixed_legacy_adobe_to_perceptual_refuses_atomically() {
    mixed_rgb_refusal_case(photocraft_cms::Builtin::AdobeRgbCompat, photocraft_doc::SampleEncoding::Perceptual, false);
}
#[test]
fn mixed_legacy_perceptual_to_adobe_refuses_atomically() {
    mixed_rgb_refusal_case(photocraft_cms::Builtin::AdobeRgbCompat, photocraft_doc::SampleEncoding::Perceptual, true);
}
#[test]
fn mixed_legacy_linear_icc_to_perceptual_refuses_atomically() {
    mixed_rgb_refusal_case(photocraft_cms::Builtin::LinearSrgb, photocraft_doc::SampleEncoding::Perceptual, false);
}
#[test]
fn mixed_legacy_perceptual_to_linear_icc_refuses_atomically() {
    mixed_rgb_refusal_case(photocraft_cms::Builtin::LinearSrgb, photocraft_doc::SampleEncoding::Perceptual, true);
}

#[test]
fn mixed_legacy_equivalent_rgb_and_gray_depths_remain_admitted() {
    use photocraft_cms::Builtin;
    use photocraft_doc::SampleEncoding::NonlinearProfile;
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let ordinary = if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::GrayGamma22 };
        for source_depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for target_depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for (explicit, bytes) in [(NonlinearProfile, ordinary.profile().to_bytes())] {
                    for reverse in [false, true] {
                        let encodings = if reverse { [Some(explicit), None] } else { [None, Some(explicit)] };
                        let (mut s, selected) = mixed_legacy_copy_fixture(mode, [source_depth, target_depth], encodings, Some(bytes.clone()), false);
                        assert_mixed_copy_equivalent_admission(&mut s, selected);
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_legacy_implicit_default_equal_presence_controls_stay_admitted() {
    use photocraft_doc::SampleEncoding::{Linear, NonlinearProfile, Perceptual};
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for encodings in [
            [None, None],
            [Some(Linear); 2],
            [Some(NonlinearProfile); 2],
            [Some(Perceptual); 2],
            [None, Some(NonlinearProfile)],
            [Some(NonlinearProfile), None],
        ] {
            let (mut s, selected) = mixed_legacy_copy_fixture(ColorMode::Rgb, [depth; 2], encodings, None, false);
            assert_mixed_copy_equivalent_admission(&mut s, selected);
        }
        for encodings in [[None, Some(NonlinearProfile)], [Some(NonlinearProfile), None]] {
            let bytes = photocraft_cms::Builtin::Srgb.profile().to_bytes();
            let (mut s, selected) = mixed_legacy_copy_fixture(ColorMode::Rgb, [depth; 2], encodings, Some(bytes), true);
            assert_mixed_copy_equivalent_admission(&mut s, selected);
        }
    }
}

#[test]
fn mixed_legacy_malformed_checked_current_profiles_refuse_atomically() {
    for encodings in [[None, Some(photocraft_doc::SampleEncoding::NonlinearProfile)], [Some(photocraft_doc::SampleEncoding::NonlinearProfile), None]] {
        let bytes = std::sync::Arc::new(b"Own malformed assigned ICC".to_vec());
        let (mut s, selected) = mixed_legacy_copy_fixture(ColorMode::Rgb, [SampleType::F32; 2], encodings, Some(bytes), false);
        assert_mixed_copy_atomic_refusal(&mut s, selected, None);
    }
}

#[test]
fn mixed_legacy_gray_meaning_changes_refuse_at_all_depths() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for encodings in [[None, Some(photocraft_doc::SampleEncoding::Linear)], [Some(photocraft_doc::SampleEncoding::Linear), None]] {
            let (mut s, selected) =
                mixed_legacy_copy_fixture(ColorMode::Grayscale, [depth; 2], encodings, Some(photocraft_cms::Builtin::GrayGamma22.profile().to_bytes()), false);
            assert_mixed_copy_atomic_refusal(&mut s, selected, Some("mixed legacy/current sample interpretation transfer is unsupported"));
        }
    }
}

#[test]
fn mixed_legacy_finite_only_curve_aliases_remain_unqualified_and_atomic() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let mut profile =
            if mode == ColorMode::Rgb { photocraft_cms::Builtin::LinearSrgb.profile().clone() } else { photocraft_cms::Builtin::GrayGamma22.profile().clone() };
        if mode == ColorMode::Grayscale {
            profile.gray_trc = Some(photocraft_cms::Curve::Gamma(1.0));
        }
        for source_depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for target_depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for encodings in [[None, Some(photocraft_doc::SampleEncoding::Linear)], [Some(photocraft_doc::SampleEncoding::Linear), None]] {
                    let (mut s, selected) = mixed_legacy_copy_fixture(mode, [source_depth, target_depth], encodings, Some(profile.to_bytes()), false);
                    assert_mixed_copy_atomic_refusal(&mut s, selected, Some("mixed legacy/current sample interpretation transfer is unsupported"));
                }
            }
        }
    }
}

#[test]
fn mixed_legacy_nonfinite_alias_payloads_refuse_without_normalization() {
    for sample in [f32::INFINITY, f32::NEG_INFINITY, f32::from_bits(0x7fc0_0042)] {
        for encodings in [[None, Some(photocraft_doc::SampleEncoding::Linear)], [Some(photocraft_doc::SampleEncoding::Linear), None]] {
            let (mut s, selected) = mixed_legacy_copy_fixture(
                ColorMode::Rgb,
                [SampleType::F32; 2],
                encodings,
                Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes()),
                false,
            );
            s.set_active(0);
            s.edit("Own hostile alias words", |d, _| {
                d.layer_mut(selected).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 1, 1), &[sample, 0.25, 0.5, 1.0]);
                Ok(())
            })
            .unwrap();
            s.edit("Own nonfinite real redo seed", |d, _| {
                d.name.push('!');
                Ok(())
            })
            .unwrap();
            assert!(s.undo());
            s.set_active(1);
            assert_mixed_copy_atomic_refusal(&mut s, selected, Some("mixed legacy/current sample interpretation transfer is unsupported"));
        }
    }
}

#[test]
fn mixed_legacy_distinct_profile_conversion_paths_keep_checked_current_reference() {
    use photocraft_doc::SampleEncoding::{Linear, NonlinearProfile, Perceptual};
    for explicit in [Linear, NonlinearProfile, Perceptual] {
        for encodings in [[None, Some(explicit)], [Some(explicit), None]] {
            let (mut s, selected) =
                mixed_legacy_copy_fixture(ColorMode::Rgb, [SampleType::F32; 2], encodings, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()), false);
            s.set_active(1);
            s.edit("Own distinct conversion destination", |d, _| {
                d.icc_profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
                Ok(())
            })
            .unwrap();
            s.edit("Own conversion real redo seed", |d, _| {
                d.name.push('!');
                Ok(())
            })
            .unwrap();
            assert!(s.undo());
            let source = doc(&s, 0).clone();
            let target = doc(&s, 1).clone();
            let before = (
                s.documents()[0].revision,
                s.documents()[0].history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
                s.documents()[0].history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),
            );
            let from = crate::color_cmds::try_document_profile(&source).unwrap();
            let to = crate::color_cmds::try_document_profile(&target).unwrap();
            let transform = photocraft_cms::Transform::new(&from, &to, s.color.settings.intent(), s.color.settings.bpc).unwrap();
            let reference =
                crate::color_cmds::convert_surface(source.layer(selected).unwrap().surface().unwrap(), source.mode, target.pixel_format(), &transform).unwrap();
            let result = s.execute("layer.copyToDocument", json!({"source":0,"document":1,"layers":[selected.0]})).unwrap();
            let actual = doc(&s, 1).clone();
            let mut expected = actual.clone();
            *expected.layer_mut(copied(&result)).unwrap().surface_mut().unwrap() = reference;
            assert_eq!(
                actual.layer(copied(&result)).unwrap().surface().unwrap().read_region(actual.bounds()),
                expected.layer(copied(&result)).unwrap().surface().unwrap().read_region(expected.bounds())
            );
            let display = |d: &Document| {
                let buffer = photocraft_compose::render_reduced_with_interrupt(d, 1, 1, &photocraft_raster::Interrupt::NONE).unwrap();
                s.color.canvas_display(d).unwrap().to_rgba8(&buffer).pixels
            };
            assert_eq!(display(&actual), display(&expected));
            assert_eq!(doc(&s, 0), &source);
            assert_eq!(
                (
                    s.documents()[0].revision,
                    s.documents()[0].history.entry_labels().map(str::to_owned).collect::<Vec<_>>(),
                    s.documents()[0].history.redo_labels().map(str::to_owned).collect::<Vec<_>>()
                ),
                before
            );
            for d in [&source, &actual, &expected] {
                let native = photocraft_format::save_to_bytes(d, &Default::default()).unwrap();
                assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *d);
            }
            assert!(s.undo());
            assert_eq!(doc(&s, 1), &target);
            assert!(s.redo());
            assert_eq!(doc(&s, 1), &actual);
            s.set_active(0);
            assert!(s.redo());
            assert!(doc(&s, 0).name.ends_with('!'));
            assert!(s.undo());
            assert_eq!(doc(&s, 0), &source);
        }
    }
}

#[test]
fn copying_linked_layers_does_not_join_an_unrelated_destination_group() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let src_a = LayerId(s.execute("layer.new.layer", json!({"name": "Source A"})).unwrap()["layer"].as_u64().unwrap());
    let src_b = LayerId(s.execute("layer.new.layer", json!({"name": "Source B"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link source", |d, _| {
        d.layer_mut(src_a).unwrap().link_group = Some(1);
        d.layer_mut(src_b).unwrap().link_group = Some(1);
        Ok(())
    })
    .unwrap();

    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let dst_a = LayerId(s.execute("layer.new.layer", json!({"name": "Destination A"})).unwrap()["layer"].as_u64().unwrap());
    let dst_b = LayerId(s.execute("layer.new.layer", json!({"name": "Destination B"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link destination", |d, _| {
        d.layer_mut(dst_a).unwrap().link_group = Some(1);
        d.layer_mut(dst_b).unwrap().link_group = Some(1);
        Ok(())
    })
    .unwrap();
    s.set_active(0);
    let result = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [src_a.0, src_b.0]})).unwrap();
    let ids: Vec<LayerId> = result["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 2);
    let d = doc(&s, 1);
    assert_eq!(d.layer(dst_a).unwrap().link_group, Some(1));
    assert_eq!(d.layer(dst_b).unwrap().link_group, Some(1));
    let first_group = d.layer(ids[0]).unwrap().link_group;
    assert_eq!(first_group, d.layer(ids[1]).unwrap().link_group, "the copied pair stays linked");
    assert!(first_group.is_some());
    assert_ne!(first_group, Some(1), "copied links must not merge with existing ones");

    assert!(s.undo());
    assert!(doc(&s, 1).layer(ids[0]).is_none());
    assert!(s.redo());
    assert_eq!(doc(&s, 1).layer(ids[0]).unwrap().link_group, first_group);
}

#[test]
fn copied_link_groups_are_distinct_from_each_other_and_destination() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link independent source groups", |d, _| {
        d.layer_mut(a).unwrap().link_group = Some(1);
        d.layer_mut(b).unwrap().link_group = Some(2);
        Ok(())
    })
    .unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.set_active(0);
    let copied = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [a.0, b.0]})).unwrap();
    let ids: Vec<LayerId> = copied["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    let first = doc(&s, 1).layer(ids[0]).unwrap().link_group;
    let second = doc(&s, 1).layer(ids[1]).unwrap().link_group;
    assert!(first.is_some() && second.is_some());
    assert_ne!(first, second);
}

/// One painted layer, `depth`-bit RGB, as the only layer above the Background.
fn one_doc(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    (s, paint)
}

#[test]
fn duplicate_to_a_new_document_keeps_size_pixels_and_place_at_every_depth() {
    for depth in [8, 16, 32] {
        let (mut s, paint) = one_doc(depth);
        let source_before = doc(&s, 0).clone();
        let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
        assert_eq!(r["document"], 1);
        assert_eq!(s.active_index(), Some(1), "the new document becomes active");
        let (src, dst) = (doc(&s, 0), doc(&s, 1));
        assert_eq!(*src, source_before, "the source is untouched");
        assert_eq!(dst.size, src.size, "same canvas, as Photoshop");
        assert_eq!((dst.mode, dst.depth, dst.resolution_dpi), (src.mode, src.depth, src.resolution_dpi));
        assert_eq!(dst.name, format!("{} copy", src.name));
        let layers = r["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 1);
        let id = LayerId(layers[0].as_u64().unwrap());
        assert_eq!(r["layer"], id.0);
        assert_ne!(id, paint, "a copy has its own id");
        assert_eq!(dst.layers.len(), 1, "just the duplicated layer");
        assert_eq!(dst.layers[0].name, "paint");
        assert_eq!(bounds(&s, 1, id), Rect::new(4, 6, 14, 16), "same canvas position");
        assert_eq!(
            dst.layers[0].surface().unwrap().read_region(Rect::new(4, 6, 14, 16)),
            src.layer(paint).unwrap().surface().unwrap().read_region(Rect::new(4, 6, 14, 16)),
            "depth {depth}: identical pixels"
        );
        assert_eq!(s.active().unwrap().selected_layers().unwrap(), vec![id]);
    }
}

#[test]
fn duplicate_to_a_new_document_names_the_copy_and_the_document() {
    let (mut s, _) = one_doc(8);
    let r = s.execute("layer.duplicate", json!({"document": "new", "name": "Ink", "documentName": "Card"})).unwrap();
    assert_eq!(doc(&s, 1).name, "Card");
    assert_eq!(doc(&s, 1).layers[0].name, "Ink");
    assert_eq!(r["layer"].as_u64(), r["layers"][0].as_u64());
}

#[test]
fn duplicate_to_a_new_document_carries_every_selected_layer_in_order() {
    let (mut s, paint) = one_doc(8);
    let bg = doc(&s, 0).layers[0].id;
    s.execute("layer.select", json!({"layer": paint.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": bg, "mode": "add"})).unwrap();
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let names: Vec<&str> = doc(&s, 1).layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "paint"]);
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    assert!(r.get("layer").is_none(), "no single layer to report");
    // A duplicated Background stays a Background: the new document has no destination one.
    let back = &doc(&s, 1).layers[0];
    assert!(back.locks.transparency && back.locks.position);
}

#[test]
fn duplicate_to_a_new_document_cuts_smart_objects_loose_as_a_batch() {
    let (mut s, _) = one_doc(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let original = s.active().unwrap().active_layer.unwrap();
    let sibling = LayerId(s.execute("layer.duplicate", json!({})).unwrap()["layer"].as_u64().unwrap());
    let contents = |s: &Session, i: usize, id: LayerId| {
        let LayerContent::Smart(sm) = &doc(s, i).layer(id).unwrap().content else { panic!("smart object") };
        sm.contents_id
    };
    assert_eq!(contents(&s, 0, original), contents(&s, 0, sibling), "instances share contents");
    s.execute("layer.select", json!({"layer": original.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": sibling.0, "mode": "add"})).unwrap();
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let ids: Vec<LayerId> = r["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    assert_eq!(contents(&s, 1, ids[0]), contents(&s, 1, ids[1]), "sharing survives within the batch");
    assert_ne!(contents(&s, 1, ids[0]), contents(&s, 0, original), "independent of the source");
}

#[test]
fn duplicate_destinations_that_are_not_new_fail_cleanly() {
    let (mut s, _) = one_doc(8);
    for p in [
        json!({"document": 1}),
        json!({"document": 7}),
        json!({"document": true}),
        json!({"document": 1.5}),
        json!({"document": []}),
        json!({"document": {}}),
        json!({"document": "other"}),
        json!({"document": "new", "layer": u64::MAX}),
    ] {
        let before = (doc(&s, 0).clone(), s.documents().len(), s.active_index());
        let err = s.execute("layer.duplicate", p.clone()).unwrap_err().to_string();
        if p == json!({"document": 1}) {
            assert!(err.contains("layer.copyToDocument"), "{p}: {err}");
        }
        assert_eq!(s.active_index(), before.2, "{p}: the source stays active");
        assert_eq!(s.documents().len(), before.1, "{p}: no document created");
        assert_eq!(*doc(&s, 0), before.0, "{p}: the source is untouched");
    }
    // One open document: same-document duplicates still work, and stay enabled.
    assert!(s.is_enabled("layer.duplicate"));
    s.execute("layer.duplicate", json!({})).unwrap();
}

#[test]
fn a_named_duplicate_takes_the_name_in_one_step() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let before = s.active().unwrap().history.past_len();
    let same = LayerId(s.execute("layer.duplicate", json!({"name": "Ink"})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(doc(&s, 0).layer(same).unwrap().name, "Ink");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    s.undo();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    let r = s.execute("layer.copyToDocument", json!({"document": 1, "name": "Ink"})).unwrap();
    assert_eq!(doc(&s, 1).layer(copied(&r)).unwrap().name, "Ink");
}
