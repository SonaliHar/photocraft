//! Deferred actual-programme Clear/Fill transaction contracts. These are
//! internal compatibility/custody expectations, not GIMP command math oracles.
use super::*;
#[path = "accounted_programme_paint_test_support.rs"]
mod f;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn session(case: f::Case) -> Session {
    let input = f::load(case);
    let mut s = Session::new();
    f::adopt(&mut s, case, &input);
    s
}
fn prepared_fill<'a>(s: &Session, p: &'a Value, c: &ReadControl<'_>) -> Result<Prepared<'a>> {
    let recipe = crate::fill_cmds::solid_fill_recipe(s, p, c)?;
    Prepared::prepare(s, p, c, Operation::Fill(recipe))
}
#[test]
#[ignore = "UNRUN/uncompiled: actual nonfirst masked/mixed leaf native Clear/solid Fill staging, full programme/source custody and one history action"]
fn concrete_native_programme_transaction_stages_before_publication() {
    for case in [f::Case::Point, f::Case::MaskedFilteredFlat, f::Case::Hierarchy] {
        for clear in [true, false] {
            let mut s = session(case);
            let before = s.active().unwrap().doc.clone();
            let id = f::target(&before);
            let raw = f::raw(&before, id);
            let witness = f::Witness::capture(before.accounted().unwrap(), &f::ctl());
            let source = s.active().unwrap().source.clone();
            let revision = s.active().unwrap().revision;
            let past = s.active().unwrap().history.past_len();
            let p = json!({"layer":id.0,"contents":"black","opacity":100,"mode":"normal"});
            let clear_p = json!({"layer":id.0});
            let prepared =
                if clear { Prepared::prepare(&s, &clear_p, &f::ctl(), Operation::Clear).unwrap() } else { prepared_fill(&s, &p, &f::ctl()).unwrap() };
            assert_eq!(prepared.witness.layer, id);
            assert!(prepared.changed);
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), past);
            assert_eq!(f::raw(&before, id), raw);
            let staged = f::raw(&prepared.snapshot, id);
            let format = prepared.witness.backed.format();
            assert_eq!((format.mode, format.sample, format.alpha), (ColorMode::Rgb, SampleType::F32, true));
            // Independent canonical endpoint expectations for the existing
            // full-coverage kernels: transparent zero or opaque black.
            let pixel: Vec<u8> = if clear { vec![0; 16] } else { [0f32, 0., 0., 1.].into_iter().flat_map(f32::to_ne_bytes).collect() };
            assert!(staged.chunks_exact(16).all(|v| v == pixel.as_slice()));
            prepared.execute(&mut s, &f::ctl()).unwrap();
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(s.active().unwrap().history.past_len(), past + 1);
            assert_eq!(f::raw(&s.active().unwrap().doc, id), staged);
            assert_eq!(s.active().unwrap().source, source);
            witness.preserved(&s.active().unwrap().doc, id, &f::ctl());
            assert!(s.undo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&s.active().unwrap().doc, id), raw);
            assert!(s.redo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&s.active().unwrap().doc, id), staged);
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: exact Fill no-op retains redo after complete Current proofs; captured view/source/tool and numeric/group target fences remain atomic"]
fn noop_and_final_fences_do_not_publish_prepared_programme_copies() {
    let mut s = session(f::Case::Point);
    let id = f::target(&s.active().unwrap().doc);
    let black = json!({"layer":id.0,"contents":"black","opacity":100,"mode":"normal"});
    prepared_fill(&s, &black, &f::ctl()).unwrap().execute(&mut s, &f::ctl()).unwrap();
    assert!(s.undo_with_control(&f::ctl()).unwrap());
    assert!(s.active().unwrap().history.can_redo());
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let p = json!({"layer":id.0,"contents":"black","opacity":0,"mode":"normal","preserveTransparency":true});
    let prepared = prepared_fill(&s, &p, &f::ctl()).unwrap();
    assert!(!prepared.changed);
    assert!(before.shares_state(&prepared.snapshot));
    prepared.execute(&mut s, &f::ctl()).unwrap();
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().revision, revision);
    assert!(s.active().unwrap().history.can_redo());
    let prepared = prepared_fill(&s, &black, &f::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &f::ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    let captured_view = s.active().unwrap().view_revision;
    let prepared = prepared_fill(&s, &black, &f::ctl()).unwrap();
    let changed_source_policy = !s.active().unwrap().source_read_only;
    s.active_mut().unwrap().source_read_only = changed_source_policy;
    assert!(prepared.execute(&mut s, &f::ctl()).is_err());
    assert_eq!(s.active().unwrap().view_revision, captured_view);
    assert!(before.shares_state(&s.active().unwrap().doc));
    let foreground = json!({"layer":id.0,"contents":"foreground","opacity":100});
    let prepared = prepared_fill(&s, &foreground, &f::ctl()).unwrap();
    s.tools.foreground[0] = if s.tools.foreground[0] == 0.25 { 0.75 } else { 0.25 };
    assert!(prepared.execute(&mut s, &f::ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    for p in [json!({"layer":"a source name"}), json!({"layer":u64::MAX}), json!({"target":"mask"})] {
        assert!(Prepared::prepare(&s, &p, &f::ctl(), Operation::Clear).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
    }
    let tree = session(f::Case::Hierarchy);
    let group = tree.active().unwrap().doc.layers.iter().find(|v| v.is_group()).unwrap().id;
    assert!(Prepared::prepare(&tree, &json!({"layer":group.0}), &f::ctl(), Operation::Clear).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: cumulative original Work late-stage denial and mid-work cancellation drop unpublished native/model/history storage; no allocation/RSS claim"]
fn original_control_owns_all_preparation_and_final_publication_work() {
    let mut s = session(f::Case::Point);
    let before = s.active().unwrap().doc.clone();
    let id = f::target(&before);
    let raw = f::raw(&before, id);
    let p = json!({"layer":id.0});
    // Warm the exact complete recipe/native cache before ledger baselines.
    let warm = Prepared::prepare(&s, &p, &f::ctl(), Operation::Clear).unwrap();
    drop(warm);
    let q = before.accounted().unwrap().quota();
    let baseline = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let prepared = Prepared::prepare(&s, &p, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &count }, Operation::Clear).unwrap();
    drop(prepared);
    assert_eq!(q.usage().unwrap(), baseline);
    let left = AtomicU64::new(used.load(Ordering::Relaxed) - 1);
    let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(Prepared::prepare(&s, &p, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work }, Operation::Clear).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(before.shares_state(&s.active().unwrap().doc));
    let cancel = AtomicBool::new(false);
    let total = AtomicU64::new(0);
    let halfway = used.load(Ordering::Relaxed) / 2;
    let stop = || cancel.load(Ordering::Relaxed);
    let interrupt = photocraft_raster::Interrupt::cancel_only(&stop);
    let work = |n| {
        if total.fetch_add(n, Ordering::Relaxed).saturating_add(n) >= halfway {
            cancel.store(true, Ordering::Relaxed)
        }
        Ok(())
    };
    assert!(Prepared::prepare(&s, &p, &ReadControl { interrupt: &interrupt, work: &work }, Operation::Clear).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let prepared = Prepared::prepare(&s, &p, &f::ctl(), Operation::Clear).unwrap();
    let work = |_| Err(BackingError::Work);
    assert!(prepared.execute(&mut s, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(f::raw(&before, id), raw);
}
