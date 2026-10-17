//! UNRUN typed native save completion lifecycle regressions, no filesystem IO.
use super::*;
use photocraft_raster::Interrupt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn session() -> Session {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "source.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&imported, "source.xcf", Some("authorized/source.xcf"), &control()).unwrap();
    s
}
#[test]
fn prepares_exact_renamed_snapshot_and_arms_path_without_undo_or_source_loss() {
    let mut s = session();
    let old = s.active().unwrap().doc.clone();
    let source = s.active().unwrap().source.clone();
    let revision = s.active().unwrap().revision;
    let p = PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control()).unwrap();
    assert_eq!(p.snapshot().name, "copy.pcraft");
    assert!(s.active().unwrap().path.is_none());
    assert_eq!(p.snapshot().xcf_state, old.xcf_state);
    assert_eq!(p.snapshot().source_color_context, old.source_color_context);
    p.write_and_publish(&mut s, &control(), |snapshot, path| {
        assert_eq!(snapshot.name, path);
        assert!(snapshot.accounted().unwrap().quota().shares_ledger(old.quota().unwrap()));
        Ok(())
    })
    .unwrap();
    let state = s.active().unwrap();
    assert_eq!(state.doc.name, "copy.pcraft");
    assert_eq!(state.path.as_deref(), Some("copy.pcraft"));
    assert_eq!(state.source, source);
    assert!(state.source_read_only);
    assert_eq!(state.revision, revision);
    assert_eq!(state.saved_revision, revision);
    assert!(!state.history.can_undo());
}
#[test]
fn failed_writer_keeps_all_identity_history_and_quota() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let quota = before.quota().unwrap().clone();
    let baseline = quota.usage().unwrap();
    let p = PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control()).unwrap();
    assert!(p.write_and_publish(&mut s, &control(), |_, _| Err::<(), _>(EngineError::Other("disk denied".into()))).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert!(history.shares_accounted_history(&s.active().unwrap().history));
    assert!(s.active().unwrap().path.is_none());
    assert_eq!(quota.usage().unwrap(), baseline);
}
#[test]
fn stale_source_view_and_journal_refuse_before_writer() {
    for variant in 0..3 {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let p = PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control()).unwrap();
        match variant {
            0 => s.active_mut().unwrap().source_read_only = false,
            1 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            _ => s.journal.push(("external".into(), serde_json::Value::Null)),
        };
        assert!(
            p.write_and_publish(&mut s, &control(), |_, _| -> Result<()> {
                panic!("stale writer reached");
            })
            .is_err()
        );
        assert!(before.shares_state(&s.active().unwrap().doc));
    }
}
#[test]
fn final_success_does_not_poll_after_committed_writer() {
    let mut s = session();
    let p = PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control()).unwrap();
    let stop = AtomicBool::new(false);
    let cancelled = || stop.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    p.write_and_publish(&mut s, &c, |_, _| {
        stop.store(true, Ordering::Relaxed);
        Ok(())
    })
    .unwrap();
    assert_eq!(s.active().unwrap().path.as_deref(), Some("copy.pcraft"));
}
#[test]
fn canceled_final_fence_cannot_reach_writer_or_mutate() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let p = PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control()).unwrap();
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(
        p.write_and_publish(&mut s, &c, |_, _| -> Result<()> {
            panic!("cancelled writer reached");
        })
        .is_err()
    );
    assert!(before.shares_state(&s.active().unwrap().doc));
}
#[test]
fn prepared_reply_exactly_one_save_record_and_keeps_existing_redo() {
    let mut s = session();
    s.execute("layer.setProps", serde_json::json!({"name":"edited"})).unwrap();
    s.undo_with_control(&control()).unwrap();
    assert!(s.active().unwrap().history.can_redo());
    let params = serde_json::json!({"path":"copy.pcraft"});
    let reply = crate::accounted_commands::PreparedAccountedSaveReply::prepare(&s, "copy.pcraft", Some(("file.saveAs", &params)), &control()).unwrap();
    let count = s.journal.len();
    let p = PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control()).unwrap();
    reply.validate(&s, &control()).unwrap();
    p.write_and_publish(&mut s, &control(), |_, _| Ok(())).unwrap();
    let result = reply.publish(&mut s);
    assert_eq!(result, serde_json::json!({"path":"copy.pcraft","warnings":[]}));
    assert_eq!(s.journal.len(), count + 1);
    assert_eq!(s.journal.last().unwrap(), &("file.saveAs".into(), params));
    assert!(s.active().unwrap().history.can_redo());
}

#[test]
fn changed_history_limits_refuse_before_writer_without_restoring_old_preferences() {
    let mut s = session();
    let p = PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control()).unwrap();
    s.active_mut().unwrap().history.max_states += 1;
    let maximum = s.active().unwrap().history.max_states;
    assert!(p.write_and_publish(&mut s, &control(), |_, _| -> Result<()> { panic!("stale preferences reached writer") }).is_err());
    assert_eq!(s.active().unwrap().history.max_states, maximum);
    assert!(s.active().unwrap().path.is_none());
}

#[test]
fn journal_and_reply_byte_copy_work_is_admitted_before_private_copy_allocations() {
    let mut s = session();
    s.journal.push(("label".into(), serde_json::Value::String("x".repeat(4096))));
    let before = s.active().unwrap().doc.clone();
    let quota = before.quota().unwrap().clone();
    let baseline = quota.usage().unwrap();
    let work = |units| if units >= 4096 { Err(BackingError::Quota) } else { Ok(()) };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(PreparedSaveJournalWitness::capture(&s, &quota, &c).is_err());
    assert_eq!(quota.usage().unwrap(), baseline);
    assert!(crate::accounted_commands::PreparedAccountedSaveReply::prepare(&s, "copy.pcraft", None, &c).is_err());
    assert_eq!(quota.usage().unwrap(), baseline);
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal[0].1.as_str().unwrap().len(), 4096);
}
