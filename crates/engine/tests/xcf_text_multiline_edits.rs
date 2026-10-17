#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
use sha2::{Digest, Sha256};
fn root() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-multiline-evidence");
    let bytes = std::fs::read(path.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "b399e6e64e4dbb2390ff52a037a0b6eb60071721b32bbfd3ef922552317dfb50");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(path.join(name)).unwrap())), hash);
    }
    path
}
fn imported(root: &std::path::Path, name: &str) -> photocraft_doc::Document {
    photocraft_io::import("text.xcf", &std::fs::read(root.join(format!("{name}.xcf"))).unwrap()).unwrap().document
}
fn pixels(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}
fn compare(root: &std::path::Path, name: &str, doc: &photocraft_doc::Document) {
    let expected = imported(root, name);
    assert_eq!(doc.xcf_state.as_ref().unwrap().source_items[0].source, expected.xcf_state.as_ref().unwrap().source_items[0].source);
    let reference = std::fs::read(root.join(format!("{name}.projection.bin"))).unwrap();
    let actual = pixels(doc);
    assert_eq!(actual.len(), reference.len());
    assert!(actual.iter().zip(reference).map(|(a, b)| a.abs_diff(b)).max().unwrap() <= 2, "{name}");
    let saved = photocraft_io::export(doc, "text.pcraft", &Default::default()).unwrap();
    let mut native_expected = doc.clone();
    native_expected.name = "text.pcraft".into();
    assert_eq!(photocraft_io::import("text.pcraft", &saved.bytes).unwrap().document, native_expected);
}
#[test]
#[ignore = "requires independent exact-font multiline sources"]
fn registered_all96_spacing_controls_native_history_and_source_immutability() {
    let root = root();
    let mut controls = 0;
    for context in ["none-32-HEFT-control", "aa-16-HEFT-source", "aa-32-H-E-edit", "aa-40-II-source"] {
        for content in ["two-stems", "mixed-lines"] {
            for original_spacing in [0, 2, 4] {
                let name = format!("{context}-{content}-line{original_spacing}");
                let doc = imported(&root, &name);
                let owner = doc.layers[0].id;
                let original = doc.xcf_state.clone().unwrap();
                let layers = doc.layers.clone();
                let mut session = Session::new();
                session.add_document(doc.clone(), None).unwrap();
                for target in [0, 2, 4, original_spacing] {
                    let before = session.active().unwrap().doc.clone();
                    let previous = photocraft_engine::xcf_text_cmds::current_line_spacing(&before, owner).unwrap();
                    let history = session.active().unwrap().history.past_len();
                    let params = json!({"layerId":owner.0.to_string(),"lineSpacingPx":target});
                    assert_eq!(session.execute("xcf.text.lineSpacing.set", params).unwrap()["updated"], previous != target as f32);
                    let current = (*session.active().unwrap().doc).clone();
                    compare(&root, &format!("{context}-{content}-line{target}"), &current);
                    assert_eq!(current.layers, layers);
                    let state = current.xcf_state.as_ref().unwrap();
                    assert_eq!(state.resources, original.resources);
                    assert_eq!(state.items, original.items);
                    assert_eq!(state.opaque, original.opaque);
                    assert_eq!(state.source_items[0].cache, original.source_items[0].cache);
                    if previous != target as f32 {
                        assert_eq!(session.active().unwrap().history.past_len(), history + 1);
                        session.execute("edit.undo", json!({})).unwrap();
                        assert_eq!(session.active().unwrap().doc, before);
                        session.execute("edit.redo", json!({})).unwrap();
                        assert_eq!(*session.active().unwrap().doc, current);
                    } else {
                        assert_eq!(session.active().unwrap().history.past_len(), history);
                    }
                    controls += 1;
                }
                assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().source_items, original.source_items);
            }
        }
    }
    assert_eq!(controls, 96);
}
#[test]
#[ignore = "requires independent exact-font multiline sources"]
fn registered_two_line_content_matches_independent_controls_and_keeps_spacing() {
    let root = root();
    for context in ["none-32-HEFT-control", "aa-16-HEFT-source", "aa-32-H-E-edit", "aa-40-II-source"] {
        for spacing in [0, 2, 4] {
            let doc = imported(&root, &format!("{context}-two-stems-line{spacing}"));
            let owner = doc.layers[0].id;
            let original = doc.xcf_state.clone().unwrap();
            let layers = doc.layers.clone();
            let mut session = Session::new();
            session.add_document(doc.clone(), None).unwrap();
            session.execute("xcf.text.content.set", json!({"layerId":owner.0.to_string(),"content":"HE\nFT"})).unwrap();
            let current = (*session.active().unwrap().doc).clone();
            compare(&root, &format!("{context}-mixed-lines-line{spacing}"), &current);
            assert_eq!(current.layers, layers);
            let state = current.xcf_state.as_ref().unwrap();
            assert_eq!(state.resources, original.resources);
            assert_eq!(state.opaque, original.opaque);
            assert_eq!(state.source_items[0].cache, original.source_items[0].cache);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, doc);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, current);
        }
    }
}
#[test]
#[ignore = "requires independent exact-font multiline sources"]
fn multiline_params_locks_domain_transitions_and_cancellation_are_atomic() {
    let root = root();
    let doc = imported(&root, "aa-32-H-E-edit-mixed-lines-line2");
    let owner = doc.layers[0].id;
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    for p in [
        json!({}),
        json!({"layerId":owner.0.to_string(),"lineSpacingPx":1}),
        json!({"layerId":owner.0.to_string(),"lineSpacingPx":2.0}),
        json!({"layerId":owner.0.to_string(),"lineSpacingPx":"2"}),
        json!({"layerId":owner.0.to_string(),"lineSpacingPx":-2}),
        json!({"layerId":owner.0.to_string(),"lineSpacingPx":4,"extra":1}),
    ] {
        assert!(session.execute("xcf.text.lineSpacing.set", p).is_err());
        assert_eq!(*session.active().unwrap().doc, doc);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    for content in ["I", "I\n", "\nI", "I\nI\nI", "O\nI", "HEFTL\nHEFTL"] {
        assert!(session.execute("xcf.text.content.set", json!({"layerId":owner.0.to_string(),"content":content})).is_err());
        assert_eq!(*session.active().unwrap().doc, doc);
    }
    assert!(!photocraft_engine::xcf_text_cmds::color_edit_candidate(&doc, owner));
    assert!(!photocraft_engine::xcf_text_cmds::font_size_edit_candidate(&doc, owner));
    assert!(!photocraft_engine::xcf_text_cmds::letter_spacing_edit_candidate(&doc, owner));
    assert!(!photocraft_engine::xcf_text_cmds::position_edit_candidate(&doc, owner));
    let mut locked = doc.clone();
    locked.layers[0].locks.pixels = true;
    assert_eq!(photocraft_engine::xcf_text_cmds::current_line_spacing(&locked, owner), Some(2.));
    assert!(!photocraft_engine::xcf_text_cmds::line_spacing_edit_candidate(&locked, owner));
    let mut locked_session = Session::new();
    locked_session.add_document(locked.clone(), None).unwrap();
    assert!(locked_session.execute("xcf.text.lineSpacing.set", json!({"layerId":owner.0.to_string(),"lineSpacingPx":4})).is_err());
    assert_eq!(*locked_session.active().unwrap().doc, locked);
    use std::sync::atomic::{AtomicBool, Ordering};
    for late in [false, true] {
        let cancelled = AtomicBool::new(!late);
        let check = || cancelled.load(Ordering::Relaxed);
        let progress = |_: f32| {
            cancelled.store(true, Ordering::Relaxed);
        };
        let ctl = photocraft_raster::Interrupt::new(&check, &progress);
        assert!(matches!(
            photocraft_engine::xcf_text_cmds::set_line_spacing_with_interrupt(&mut session, &json!({"layerId":owner.0.to_string(),"lineSpacingPx":4}), &ctl),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*session.active().unwrap().doc, doc);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    let single_root = root.parent().unwrap().join("text-spacing-evidence");
    let single = imported(&single_root, "none-32-HEFT-control");
    let owner = single.layers[0].id;
    let mut session = Session::new();
    session.add_document(single.clone(), None).unwrap();
    assert!(session.execute("xcf.text.lineSpacing.set", json!({"layerId":owner.0.to_string(),"lineSpacingPx":2})).is_err());
    assert!(session.execute("xcf.text.content.set", json!({"layerId":owner.0.to_string(),"content":"I\nI"})).is_err());
    assert_eq!(*session.active().unwrap().doc, single);
}
#[test]
fn registered_multiline_schema_preserves_exact_spacing_and_domain_policy() {
    let specs = photocraft_engine::xcf_text_cmds::specs();
    let command = specs.iter().find(|s| s.id == "xcf.text.lineSpacing.set").unwrap();
    assert!(command.params.contains("lineSpacingPx\":0|2|4"));
    assert!(command.params.contains("exact integer"));
    assert!(command.params.contains("Zero omits"));
    assert!(command.params.contains("Single-line domain transitions refused"));
}
