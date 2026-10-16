//! Deferred generated-source hierarchy painting/native workflow contracts.
//! Encoded/comparison buffers are external harness allocations, never receipts.
use super::*;
#[path = "accounted_plain_hierarchy_paint_test_support.rs"]
mod f;
use serde_json::json;

fn patch() -> Value {
    json!({"points":[[254.0,254.0,0.7],[257.0,258.0,1.0]],"size":7.0,
        "hardness":0.6,"spacing":0.3,"color":[0.8,0.2,0.1,0.75],"seed":53})
}
fn session(depth: usize) -> Session {
    let source = f::import(depth);
    let photocraft_io::OwnedXcfImport::NormalHierarchy(source) = &source else { panic!("actual hierarchy Source") };
    let mut s = Session::new();
    s.add_owned_normal_hierarchy_xcf_import(source, "hierarchy-paint.xcf", Some("authorized/hierarchy-paint.xcf"), &f::ctl()).unwrap();
    let target = f::target_leaf(&s.active().unwrap().doc);
    s.execute("layer.select", json!({"layer":target.0})).unwrap();
    s.tools.brush.pressure_size = false;
    s
}
fn unchanged_siblings(a: &photocraft_doc::Document, b: &photocraft_doc::Document, target: photocraft_doc::LayerId) {
    let old = f::payload(a);
    let new = f::payload(b);
    assert_eq!(old.len(), new.len());
    for ((x, a), (y, b)) in old.iter().zip(&new) {
        assert_eq!(x, y);
        if *x == target {
            assert_ne!(a, b);
        } else {
            assert_eq!(a, b);
        }
    }
    assert_eq!(a.xcf_state, b.xcf_state);
    f::source_payload(a, b);
}

#[test]
#[ignore = "UNRUN/uncompiled: actual generated all-five-depth plain hierarchy nested leaf COW, native13 empty-anchor persistence, reopen/second stroke/exact undo-redo and source/sibling custody; no external numerical oracle"]
fn hierarchy_stroke_native_reopen_and_second_edit_preserve_tree() {
    for depth in 0..5 {
        let mut s = session(depth);
        let original = s.active().unwrap().doc.clone();
        let raw = f::payload(&original);
        let target = f::target_leaf(&original);
        let identity = s.active().unwrap().source.clone();
        s.execute("paint.stroke", patch()).unwrap();
        let changed = s.active().unwrap().doc.clone();
        unchanged_siblings(&original, &changed, target);
        assert_eq!(f::payload(&original), raw);
        assert_eq!(s.active().unwrap().source, identity);
        assert!(s.active().unwrap().source_read_only);
        assert!(s.undo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::payload(&s.active().unwrap().doc), raw);
        assert!(s.redo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::payload(&s.active().unwrap().doc), f::payload(&changed));

        // A separately paid CURRENT copy adds a real retained empty anchor. It
        // is deliberately not supplied to any original-Source constructor.
        let owner = f::anchored(changed.accounted().unwrap());
        let a = f::archive(&owner);
        assert_eq!(a.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::NormalHierarchy));
        let mut current = Session::new();
        current.add_owned_native_normal_hierarchy_archive(a, "hierarchy-copy.pcraft", Some("authorized/hierarchy-copy.pcraft"), &f::ctl()).unwrap();
        f::same(owner.document(), &current.active().unwrap().doc);
        assert!(!current.active().unwrap().source_read_only);
        current.tools.brush.pressure_size = false;
        let target = f::target_leaf(&current.active().unwrap().doc);
        current.execute("layer.select", json!({"layer":target.0})).unwrap();
        let before = current.active().unwrap().doc.clone();
        let before_bytes = f::payload(&before);
        let source = current.active().unwrap().source.clone();
        let mut p = patch();
        p["color"] = json!([0.12500000000000003, 0.6250000000000001, 0.8750000000000001, 1.0]);
        current.execute("paint.stroke", p).unwrap();
        let after = current.active().unwrap().doc.clone();
        unchanged_siblings(&before, &after, target);
        assert_eq!(current.active().unwrap().source, source);
        assert_eq!(f::payload(&before), before_bytes);
        let stack = after.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == photocraft_doc::item_identity::ItemRef::Layer(target)).unwrap();
        assert_eq!(stack.revision, 7);
        assert!(stack.nodes.is_empty());
        assert!(current.undo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::payload(&current.active().unwrap().doc), before_bytes);
        assert!(current.redo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::payload(&current.active().unwrap().doc), f::payload(&after));
        let mut reopen = Session::new();
        reopen.add_owned_native_normal_hierarchy_archive(f::archive(after.accounted().unwrap()), "second-copy.pcraft", None, &f::ctl()).unwrap();
        f::same(&after, &reopen.active().unwrap().doc);
        assert_eq!(f::payload(&original), raw);
    }
}

#[test]
#[ignore = "UNRUN/uncompiled: painted hierarchy Save As protects the original XCF identity, failed writer cannot arm native destination or publish save/history/source state; no filesystem or runtime acceptance"]
fn hierarchy_stroke_save_failure_preserves_source_and_history() {
    for depth in [2usize, 4] {
        let mut s = session(depth);
        s.execute("paint.stroke", patch()).unwrap();
        let before = s.active().unwrap().doc.clone();
        let raw = f::payload(&before);
        let history = s.active().unwrap().history.clone();
        let source = s.active().unwrap().source.clone();
        let revision = s.active().unwrap().revision;
        let saved_revision = s.active().unwrap().saved_revision;
        let path = s.active().unwrap().path.clone();
        assert!(PreparedAccountedNativeSave::prepare(&s, "authorized/hierarchy-paint.xcf", &f::ctl()).is_err());
        let prepared = PreparedAccountedNativeSave::prepare(&s, "authorized/hierarchy-copy.pcraft", &f::ctl()).unwrap();
        assert!(prepared.write_and_publish(&mut s, &f::ctl(), |_, _| Err::<(), _>(EngineError::Other("disk denied".into()))).is_err());
        let state = s.active().unwrap();
        assert!(before.shares_state(&state.doc));
        assert!(history.shares_accounted_history(&state.history));
        assert_eq!(state.source, source);
        assert!(state.source_read_only);
        assert_eq!(state.path, path);
        assert_eq!(state.revision, revision);
        assert_eq!(state.saved_revision, saved_revision);
        assert_eq!(f::payload(&state.doc), raw);
    }
}
