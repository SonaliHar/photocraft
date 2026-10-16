//! Deferred programme-preserving native brush and protected-save contracts.
//! Comparison/encoded buffers belong to the external harness, not receipts.
use super::*;
#[path = "accounted_programme_paint_test_support.rs"]
mod f;
use serde_json::json;

#[test]
#[ignore = "UNRUN/uncompiled: nine actual programme storage classes, native COW/undo/redo, complete programme and physical mask custody, typed native reopen and second edit; masked hierarchy is explicitly paid CURRENT, no numerical oracle"]
fn programme_stroke_native_reopen_second_edit_and_history() {
    for &case in f::cases() {
        let input = f::load(case);
        let original = f::Witness::capture(input.owner(), &f::ctl());
        let mut s = Session::new();
        f::adopt(&mut s, case, &input);
        s.tools.brush.pressure_size = false;
        let target = f::target(&s.active().unwrap().doc);
        let raw = f::raw(&s.active().unwrap().doc, target);
        let source = s.active().unwrap().source.clone();
        let protected = s.active().unwrap().source_read_only;
        s.execute("paint.stroke", f::patch(&s.active().unwrap().doc, target)).unwrap();
        let painted = f::raw(&s.active().unwrap().doc, target);
        assert_ne!(painted, raw, "{case:?}");
        original.preserved(&s.active().unwrap().doc, target, &f::ctl());
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(s.active().unwrap().source_read_only, protected);
        assert_eq!(f::raw(original.before(), target), raw);
        assert!(s.undo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::raw(&s.active().unwrap().doc, target), raw);
        original.preserved(&s.active().unwrap().doc, target, &f::ctl());
        assert!(s.redo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::raw(&s.active().unwrap().doc, target), painted);

        let first = f::Witness::capture(s.active().unwrap().doc.accounted().unwrap(), &f::ctl());
        let archive = f::archive(s.active().unwrap().doc.accounted().unwrap());
        let mut native = Session::new();
        f::reopen(&mut native, case, archive);
        assert!(!native.active().unwrap().source_read_only);
        first.preserved(&native.active().unwrap().doc, target, &f::ctl());
        assert_eq!(f::raw(&native.active().unwrap().doc, target), painted);
        native.tools.brush.pressure_size = false;
        let mut patch = f::patch(&native.active().unwrap().doc, target);
        patch["color"] = json!([0.12500000000000003, 0.6250000000000001, 0.8750000000000001, 1.0]);
        let identity = native.active().unwrap().source.clone();
        native.execute("paint.stroke", patch).unwrap();
        let second = f::raw(&native.active().unwrap().doc, target);
        assert_ne!(second, painted, "{case:?}");
        first.preserved(&native.active().unwrap().doc, target, &f::ctl());
        assert_eq!(native.active().unwrap().source, identity);
        assert!(native.undo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::raw(&native.active().unwrap().doc, target), painted);
        assert!(native.redo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::raw(&native.active().unwrap().doc, target), second);
        let mut reopen = Session::new();
        f::reopen(&mut reopen, case, f::archive(native.active().unwrap().doc.accounted().unwrap()));
        first.preserved(&reopen.active().unwrap().doc, target, &f::ctl());
        assert_eq!(f::raw(&reopen.active().unwrap().doc, target), second);
        assert_eq!(f::raw(original.before(), target), raw);
    }
}

#[test]
#[ignore = "UNRUN/uncompiled: painted genuine programme Sources protect original XCF and failed native Save As atomically retains document/programme/source/path/history/revisions; no filesystem or UI acceptance"]
fn programme_stroke_protected_source_and_failed_native_save() {
    for case in [f::Case::ThresholdAlpha, f::Case::AssignedPoint, f::Case::MaskedFilteredFlat, f::Case::Hierarchy] {
        let input = f::load(case);
        assert_eq!(case.evidence(), f::Evidence::GenuineSource);
        let mut s = Session::new();
        f::adopt(&mut s, case, &input);
        s.tools.brush.pressure_size = false;
        let target = f::target(&s.active().unwrap().doc);
        s.execute("paint.stroke", f::patch(&s.active().unwrap().doc, target)).unwrap();
        let before = s.active().unwrap().doc.clone();
        let witness = f::Witness::capture(before.accounted().unwrap(), &f::ctl());
        let raw = f::raw(&before, target);
        let history = s.active().unwrap().history.clone();
        let source = s.active().unwrap().source.clone();
        let path = s.active().unwrap().path.clone();
        let revision = s.active().unwrap().revision;
        let saved_revision = s.active().unwrap().saved_revision;
        assert!(PreparedAccountedNativeSave::prepare(&s, "protected-original.xcf", &f::ctl()).is_err());
        let prepared = PreparedAccountedNativeSave::prepare(&s, "programme-copy.pcraft", &f::ctl()).unwrap();
        assert!(prepared.write_and_publish(&mut s, &f::ctl(), |_, _| Err::<(), _>(EngineError::Other("disk denied".into()))).is_err());
        let after = s.active().unwrap();
        assert!(before.shares_state(&after.doc));
        assert!(history.shares_accounted_history(&after.history));
        assert_eq!(after.source, source);
        assert!(after.source_read_only);
        assert_eq!(after.path, path);
        assert_eq!(after.revision, revision);
        assert_eq!(after.saved_revision, saved_revision);
        assert_eq!(f::raw(&after.doc, target), raw);
        witness.preserved(&after.doc, target, &f::ctl());
    }
}
