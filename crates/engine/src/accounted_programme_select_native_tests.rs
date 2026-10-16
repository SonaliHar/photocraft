//! Deferred genuine mixed second-root focus/native continuation. No new Source
//! or numerical evidence. Encoded/native comparison buffers are external harness.
use crate::Session;
#[path = "accounted_programme_paint_test_support.rs"]
mod f;
#[path = "accounted_masked_filtered_flat_test_support.rs"]
mod masked;
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod ordered;
use photocraft_doc::LayerId;
use serde_json::json;
fn input(mask: bool, depth: usize) -> (photocraft_io::OwnedXcfImport, std::path::PathBuf, Vec<u8>) {
    if mask {
        let id = if depth == 0 { "f32-two-roots" } else { "f64-two-roots" };
        let case = masked::cases().into_iter().find(|c| c["id"].as_str() == Some(id)).unwrap();
        let path = masked::path(&case);
        let bytes = masked::bytes(&case);
        (masked::import(&case), path, bytes)
    } else {
        let case = &ordered::data::CASES[depth];
        let path = ordered::workspace().join(case.source.path);
        let bytes = ordered::bytes(case.source);
        (ordered::import(case), path, bytes)
    }
}
fn root(s: &Session, name: &str) -> LayerId {
    s.active().unwrap().doc.layers.iter().find(|l| l.name == name).unwrap().id
}
/// This is the registered view command, never a fake model/current edit.
fn select_second(s: &mut Session, first: LayerId, second: LayerId) {
    s.execute("layer.select", json!({"layer":first.0})).unwrap();
    let state = s.active().unwrap();
    let before = state.doc.clone();
    let past = state.history.past_len();
    let redo = state.history.can_redo();
    let revision = state.revision;
    let saved = state.saved_revision;
    let clean = revision == saved;
    let source = state.source.clone();
    let protected = state.source_read_only;
    s.execute("layer.select", json!({"layer":second.0,"mode":"replace"})).unwrap();
    let state = s.active().unwrap();
    assert!(before.shares_state(&state.doc));
    assert_eq!(state.active_layer, Some(second));
    assert_eq!(state.layer_anchor, Some(second));
    assert!(state.selected_layers.is_empty());
    let view = state.accounted_view.as_ref().unwrap();
    assert_eq!(view.target().active, Some(second));
    assert_eq!(view.selected(), &[second]);
    assert_eq!(view.anchor(), Some(second));
    assert_eq!(state.history.past_len(), past);
    assert_eq!(state.history.can_redo(), redo);
    assert_eq!(state.revision, revision + 1);
    assert_eq!(state.saved_revision, if clean { revision + 1 } else { saved });
    assert_eq!(state.source, source);
    assert_eq!(state.source_read_only, protected);
}
#[test]
#[ignore = "UNRUN/uncompiled: four genuine ordered/masked F32/F64 originals select actual second root, preserve immutable programme/source/native mask custody, paint/undo/redo then typed native10/12 reopen/reselect/secondpaint; no producer brush oracle"]
fn genuine_second_root_selection_survives_native_paint_continuation() {
    for mask in [false, true] {
        for depth in 0..2 {
            let (input, path, encoded) = input(mask, depth);
            let owner = input.document().clone();
            let original = f::Witness::capture(&owner, &f::ctl());
            let mut s = Session::new();
            let sourcepath = path.to_string_lossy();
            match &input {
                photocraft_io::OwnedXcfImport::FilteredFlat(v) => {
                    s.add_owned_filtered_flat_xcf_import(v, "second-root.xcf", Some(&sourcepath), &f::ctl()).unwrap();
                }
                photocraft_io::OwnedXcfImport::MaskedFilteredFlat(v) => {
                    s.add_owned_masked_filtered_flat_xcf_import(v, "second-root.xcf", Some(&sourcepath), &f::ctl()).unwrap();
                }
                _ => panic!("exact actual mixed class"),
            }
            assert_eq!(s.active().unwrap().doc.depth, if depth == 0 { photocraft_doc::SampleType::F32 } else { photocraft_doc::SampleType::F64 });
            let first = root(&s, "Own root 0");
            let target = root(&s, "Own root 1");
            assert_eq!(s.active().unwrap().doc.layers[1].id, target);
            assert_ne!(first, target);
            let raw = f::raw(&s.active().unwrap().doc, target);
            let sibling = f::raw(&s.active().unwrap().doc, first);
            let source = s.active().unwrap().source.clone();
            select_second(&mut s, first, target);
            original.preserved(&s.active().unwrap().doc, target, &f::ctl());
            assert_eq!(f::raw(&s.active().unwrap().doc, target), raw);
            s.tools.brush.pressure_size = false;
            // Omit explicit target only after the registered focus command; active view
            // must choose the second root. No selection mutation is hidden in the patch.
            let mut patch = f::patch(&s.active().unwrap().doc, target);
            patch.as_object_mut().unwrap().remove("layer");
            s.execute("paint.stroke", patch).unwrap();
            let painted = f::raw(&s.active().unwrap().doc, target);
            assert_ne!(painted, raw);
            assert_eq!(f::raw(&s.active().unwrap().doc, first), sibling);
            original.preserved(&s.active().unwrap().doc, target, &f::ctl());
            assert_eq!(s.active().unwrap().source, source);
            assert!(s.active().unwrap().source_read_only);
            assert!(s.undo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&s.active().unwrap().doc, target), raw);
            assert!(s.redo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&s.active().unwrap().doc, target), painted);
            let current = f::Witness::capture(s.active().unwrap().doc.accounted().unwrap(), &f::ctl());
            let archive = f::archive(s.active().unwrap().doc.accounted().unwrap());
            assert_eq!(
                archive.current_storage_hint(),
                Some(if mask {
                    photocraft_format::NativeCurrentStorageHint::MaskedFilteredFlatPoint
                } else {
                    photocraft_format::NativeCurrentStorageHint::FilteredFlatPoint
                })
            );
            let case = if mask { f::Case::MaskedFilteredFlat } else { f::Case::FilteredFlat };
            let mut native = Session::new();
            f::reopen(&mut native, case, archive);
            assert!(!native.active().unwrap().source_read_only);
            assert_eq!(root(&native, "Own root 1"), target);
            assert_eq!(f::raw(&native.active().unwrap().doc, target), painted);
            current.preserved(&native.active().unwrap().doc, target, &f::ctl());
            // Owned document serialization need not persist external AccountedView focus.
            select_second(&mut native, first, target);
            let identity = native.active().unwrap().source.clone();
            native.tools.brush.pressure_size = false;
            let mut patch = f::patch(&native.active().unwrap().doc, target);
            patch.as_object_mut().unwrap().remove("layer");
            patch["color"] = json!([0.125, 0.625, 0.875, 1.0]);
            native.execute("paint.stroke", patch).unwrap();
            let second = f::raw(&native.active().unwrap().doc, target);
            assert_ne!(second, painted);
            assert_eq!(f::raw(&native.active().unwrap().doc, first), sibling);
            current.preserved(&native.active().unwrap().doc, target, &f::ctl());
            assert_eq!(native.active().unwrap().source, identity);
            assert!(native.undo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&native.active().unwrap().doc, target), painted);
            assert!(native.redo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&native.active().unwrap().doc, target), second);
            let mut again = Session::new();
            f::reopen(&mut again, case, f::archive(native.active().unwrap().doc.accounted().unwrap()));
            current.preserved(&again.active().unwrap().doc, target, &f::ctl());
            assert_eq!(f::raw(&again.active().unwrap().doc, target), second);
            assert_eq!(f::raw(original.before(), target), raw);
            assert_eq!(std::fs::read(&path).unwrap(), encoded);
        }
    }
}
