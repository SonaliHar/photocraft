//! Lossless canonical layer selector drafts; intentionally unrun during implementation.
use photocraft_doc::item_identity::{ItemRef, ItemSetId};
use photocraft_doc::xcf_editing::{ItemFamily, ItemSet, ItemSetRule};
use photocraft_doc::{Color, ColorMode, Document, LayerId, SampleType, Size, XcfState};
use photocraft_engine::{Session, layer_multi_cmds::rename_layer_with_interrupt};
use photocraft_raster::Interrupt;
use serde_json::json;
fn fixture(id: u64) -> Session {
    let mut doc = Document::with_background("Canonical rename", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.layers[0].id = LayerId(id);
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.selection.selected = vec![ItemRef::Layer(LayerId(id))];
    state.selection.active = Some(ItemRef::Layer(LayerId(id)));
    state.item_sets.push(ItemSet {
        id: ItemSetId(701),
        name: "Exact typed set Ω".into(),
        family: ItemFamily::Layer,
        rule: ItemSetRule::Named,
        members: vec![ItemRef::Layer(LayerId(id))],
    });
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
#[test]
fn canonical_ids_keep_scalar_history_exact_and_native_obeys_documented_remap() {
    // Nearby IDs are preserved; implausibly far-ahead IDs intentionally remap on load.
    let nearby = LayerId::fresh().0;
    for id in [nearby, (1u64 << 53) + 1, u64::MAX] {
        let mut session = fixture(id);
        let before = session.active().unwrap().doc.clone();
        let active = session.active().unwrap();
        let p = json!({"layerId":id.to_string(),"name":"Exact high identity","documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string()});
        rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
        let after = session.active().unwrap().doc.clone();
        assert_eq!(after.layers[0].id, LayerId(id));
        assert_eq!(after.layers[0].name, "Exact high identity");
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: true, ..Default::default() }).unwrap();
        let mut expected_native = (*after).clone();
        if id == nearby {
            assert_eq!(reopened.layers[0].id, LayerId(id));
        } else {
            // The format's bounded allocator remaps identities, including every typed reference.
            // Exact high-ID preservation is outside that documented native authority.
            let remapped = reopened.layers[0].id;
            assert_ne!(remapped, LayerId(id));
            assert_ne!(reopened.id, after.id);
            expected_native.id = reopened.id;
            expected_native.layers[0].id = remapped;
            expected_native
                .xcf_state
                .as_mut()
                .unwrap()
                .remap_layers(|old| {
                    assert_eq!(old, LayerId(id));
                    Some(remapped)
                })
                .unwrap();
        }
        reopened.validate_xcf_state().unwrap();
        assert_eq!(reopened, expected_native);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, after);
    }
}
#[test]
fn stale_duplicate_and_malformed_selectors_preserve_real_redo() {
    let id = (1u64 << 53) + 1;
    let mut session = fixture(id);
    rename_layer_with_interrupt(&mut session, &json!({"layer":id,"name":"Seed"}), &Interrupt::NONE).unwrap();
    session.execute("edit.undo", json!({})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    for p in [
        json!({"layer":id,"layerId":id.to_string(),"name":"Ambiguous"}),
        json!({"layerId":id.to_string(),"name":"Stale","documentId":before.id.0.to_string(),"expectedRevision":revision.saturating_sub(1).to_string()}),
        json!({"layerId":"01","name":"Bad"}),
        json!({"layerId":"0","name":"Bad"}),
        json!({"layerId":"18446744073709551616","name":"Overflow"}),
        json!({"layerId":id,"name":"Wrong type"}),
        json!({"layer":id,"name":"Unknown","unknown":true}),
    ] {
        assert!(rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
    }
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.layers[0].name, "Seed");
}
