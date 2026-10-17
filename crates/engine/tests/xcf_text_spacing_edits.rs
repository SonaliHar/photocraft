#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
use sha2::{Digest, Sha256};
fn root() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-spacing-evidence");
    let bytes = std::fs::read(path.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "2c987cfa774b38d9004717f68a657c6f7c2b3ff00004b43fcf03c8f61a934687");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(path.join(name)).unwrap())), hash);
    }
    path
}
fn projection(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}

fn imported(root: &std::path::Path, name: &str) -> photocraft_doc::Document {
    photocraft_io::import("text.xcf", &std::fs::read(root.join(format!("{name}.xcf"))).unwrap()).unwrap().document
}
fn compare(doc: &photocraft_doc::Document, expected: &photocraft_doc::Document) {
    assert_eq!(doc.xcf_state.as_ref().unwrap().source_items[0].source, expected.xcf_state.as_ref().unwrap().source_items[0].source);
    assert_eq!(projection(doc), projection(expected));
}
#[test]
#[ignore = "requires independently hash-bound spacing originals"]
fn registered_spacing_history_native_and_preserved_source_evidence() {
    let root = root();
    for name in ["none-32-II", "aa-16-HEFT", "aa-32-H-E", "aa-40-HEFT"] {
        let doc = imported(&root, &format!("{name}-source"));
        let expected = imported(&root, &format!("{name}-edit"));
        let original = doc.xcf_state.clone().unwrap();
        let layers = doc.layers.clone();
        let owner = layers[0].id;
        let mut session = Session::new();
        session.add_document(doc.clone(), None).unwrap();
        let params = json!({"layerId":owner.0.to_string(),"letterSpacingPx":4});
        assert_eq!(session.execute("xcf.text.letterSpacing.set", params.clone()).unwrap()["updated"], true);
        let edited = (*session.active().unwrap().doc).clone();
        compare(&edited, &expected);
        assert_eq!(edited.layers, layers);
        let state = edited.xcf_state.as_ref().unwrap();
        assert_eq!(state.resources, original.resources);
        assert_eq!(state.opaque, original.opaque);
        assert_eq!(state.source_items[0].cache, original.source_items[0].cache);
        let saved = photocraft_io::export(&edited, "text.pcraft", &Default::default()).unwrap();
        let mut expected_native = edited.clone();
        expected_native.name = "text.pcraft".into();
        assert_eq!(photocraft_io::import("text.pcraft", &saved.bytes).unwrap().document, expected_native);
        assert_eq!(session.execute("xcf.text.letterSpacing.set", params).unwrap()["updated"], false);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, doc);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, edited);
    }
}
#[test]
#[ignore = "requires independently hash-bound spacing originals"]
fn content_size_color_preserve_spacing_and_atomic_denials() {
    let root = root();
    let doc = imported(&root, "aa-16-II-source");
    let owner = doc.layers[0].id;
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    session.execute("xcf.text.content.set", json!({"layerId":owner.0.to_string(),"content":"HEFT"})).unwrap();
    compare(&session.active().unwrap().doc, &imported(&root, "aa-16-HEFT-source"));
    session.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":32})).unwrap();
    compare(&session.active().unwrap().doc, &imported(&root, "aa-32-HEFT-source"));
    session
        .execute("xcf.text.color.set", json!({"layerId":owner.0.to_string(),"encoding":"linear","rgbaF32Bits":["3f800000","00000000","00000000","3f000000"]}))
        .unwrap();
    assert_eq!(photocraft_engine::xcf_text_cmds::current_letter_spacing(&session.active().unwrap().doc, owner), Some(2.));
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.past_len();
    for params in [
        json!({}),
        json!({"layerId":owner.0.to_string(),"letterSpacingPx":1}),
        json!({"layerId":owner.0.to_string(),"letterSpacingPx":2.5}),
        json!({"layerId":owner.0.to_string(),"letterSpacingPx":"4"}),
    ] {
        assert!(session.execute("xcf.text.letterSpacing.set", params).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
    assert!(session.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":24})).is_err());
    assert_eq!(session.active().unwrap().doc, before);
    use std::sync::atomic::{AtomicBool, Ordering};
    let cancelled = AtomicBool::new(true);
    let check = || cancelled.load(Ordering::Relaxed);
    let noop = |_: f32| {};
    let ctl = photocraft_raster::Interrupt::new(&check, &noop);
    assert!(
        photocraft_engine::xcf_text_cmds::set_letter_spacing_with_interrupt(&mut session, &json!({"layerId":owner.0.to_string(),"letterSpacingPx":4}), &ctl)
            .is_err()
    );
    assert_eq!(session.active().unwrap().doc, before);
    cancelled.store(false, Ordering::Relaxed);
    let progress = |_: f32| {
        cancelled.store(true, Ordering::Relaxed);
    };
    let ctl = photocraft_raster::Interrupt::new(&check, &progress);
    assert!(
        photocraft_engine::xcf_text_cmds::set_letter_spacing_with_interrupt(&mut session, &json!({"layerId":owner.0.to_string(),"letterSpacingPx":4}), &ctl)
            .is_err()
    );
    assert_eq!(session.active().unwrap().doc, before);
    let mut locked = doc.clone();
    locked.layers[0].locks.pixels = true;
    assert_eq!(photocraft_engine::xcf_text_cmds::current_letter_spacing(&locked, owner), Some(2.));
    assert!(!photocraft_engine::xcf_text_cmds::letter_spacing_edit_candidate(&locked, owner));
    let mut session = Session::new();
    session.add_document(locked.clone(), None).unwrap();
    assert!(session.execute("xcf.text.letterSpacing.set", json!({"layerId":owner.0.to_string(),"letterSpacingPx":4})).is_err());
    assert_eq!(*session.active().unwrap().doc, locked);
}

#[test]
#[ignore = "requires frozen zero-spacing inverse proof and independent originals"]
fn registered_zero_spacing_all48_transitions_native_history_and_current_geometry() {
    let root = root();
    let proof = root.parent().unwrap().join("text-spacing-zero-evidence");
    let manifest = std::fs::read(proof.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "251d5cd635e48e2faff9b7558ee9484ada312efe2ae19cf85c5934544bbd3b92");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(proof.join(name)).unwrap())), hash);
    }
    let mut transitions = 0;
    for context in ["none-32", "aa-16", "aa-32", "aa-40"] {
        for content in ["II", "HEFT", "H-E"] {
            let key = format!("{context}-{content}");
            for (from, to, spacing) in [("control", "source", 2), ("source", "control", 0), ("control", "edit", 4), ("edit", "control", 0)] {
                let original = imported(&root, &format!("{key}-{from}"));
                let expected = imported(&root, &format!("{key}-{to}"));
                let owner = original.layers[0].id;
                let evidence = original.xcf_state.clone().unwrap();
                let mut session = Session::new();
                session.add_document(original.clone(), None).unwrap();
                let p = json!({"layerId":owner.0.to_string(),"letterSpacingPx":spacing});
                assert_eq!(session.execute("xcf.text.letterSpacing.set", p.clone()).unwrap()["updated"], true);
                let edited = (*session.active().unwrap().doc).clone();
                compare(&edited, &expected);
                assert_eq!(edited.layers, original.layers);
                let state = edited.xcf_state.as_ref().unwrap();
                assert_eq!(state.resources, evidence.resources);
                assert_eq!(state.opaque, evidence.opaque);
                assert_eq!(state.source_items[0].cache, evidence.source_items[0].cache);
                let saved = photocraft_io::export(&edited, "zero.pcraft", &Default::default()).unwrap();
                let mut expected_native = edited.clone();
                expected_native.name = "zero.pcraft".into();
                assert_eq!(photocraft_io::import("zero.pcraft", &saved.bytes).unwrap().document, expected_native);
                assert_eq!(session.execute("xcf.text.letterSpacing.set", p).unwrap()["updated"], false);
                assert_eq!(session.active().unwrap().history.past_len(), 1);
                session.execute("edit.undo", json!({})).unwrap();
                assert_eq!(*session.active().unwrap().doc, original);
                session.execute("edit.redo", json!({})).unwrap();
                assert_eq!(*session.active().unwrap().doc, edited);
                transitions += 1;
            }
        }
    }
    assert_eq!(transitions, 48);
}

#[test]
#[ignore = "requires original zero/nonzero text sources for atomic boundary cases"]
fn zero_transition_locks_cancel_noop_and_unobserved_size24_are_atomic() {
    use photocraft_engine::xcf_text_cmds::{current_letter_spacing, letter_spacing_edit_candidate, set_letter_spacing_with_interrupt};
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = root();
    for (name, next) in [("aa-16-HEFT-control", 2), ("aa-32-HEFT-source", 0)] {
        let original = imported(&root, name);
        let owner = original.layers[0].id;
        let mut locked = original.clone();
        locked.layers[0].locks.pixels = true;
        assert_eq!(current_letter_spacing(&locked, owner), Some(if next == 0 { 2. } else { 0. }));
        assert!(!letter_spacing_edit_candidate(&locked, owner));
        let p = json!({"layerId":owner.0.to_string(),"letterSpacingPx":next});
        let mut session = Session::new();
        session.add_document(locked.clone(), None).unwrap();
        assert!(session.execute("xcf.text.letterSpacing.set", p.clone()).is_err());
        assert_eq!(*session.active().unwrap().doc, locked);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        for late in [false, true] {
            let mut session = Session::new();
            session.add_document(original.clone(), None).unwrap();
            let flag = AtomicBool::new(!late);
            let check = || flag.load(Ordering::SeqCst);
            let progress = |v: f32| {
                if v == 1. {
                    flag.store(true, Ordering::SeqCst)
                }
            };
            let ctl = photocraft_raster::Interrupt::new(&check, &progress);
            assert!(set_letter_spacing_with_interrupt(&mut session, &p, &ctl).is_err());
            assert_eq!(*session.active().unwrap().doc, original);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
        }
    }
    let path = root.parent().unwrap().join("text-size-evidence/aa-size-edit.xcf");
    let doc = photocraft_io::import("size.xcf", &std::fs::read(path).unwrap()).unwrap().document;
    let owner = doc.layers[0].id;
    assert_eq!(photocraft_engine::xcf_text_cmds::current_font_size(&doc, owner), Some(24));
    assert_eq!(current_letter_spacing(&doc, owner), Some(0.));
    assert!(!letter_spacing_edit_candidate(&doc, owner));
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    for next in [2, 4] {
        assert!(session.execute("xcf.text.letterSpacing.set", json!({"layerId":owner.0.to_string(),"letterSpacingPx":next})).is_err());
        assert_eq!(*session.active().unwrap().doc, doc);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    assert_eq!(session.execute("xcf.text.letterSpacing.set", json!({"layerId":owner.0.to_string(),"letterSpacingPx":0})).unwrap()["updated"], false);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
}

#[test]
fn registered_spacing_schema_documents_exact_values_and_omission() {
    let command = photocraft_engine::commands::command_specs().iter().find(|c| c.id == "xcf.text.letterSpacing.set").unwrap();
    assert!(command.params.contains("letterSpacingPx\":0|2|4"));
    assert!(command.params.contains("zero omits"));
    assert!(command.params.contains("sizes 16/32/40"));
    assert!(command.params.contains("current 24px nonzero changes are refused"));
}
