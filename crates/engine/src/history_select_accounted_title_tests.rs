//! UNRUN registered History jump/Save As identity and private rollback drafts.
use super::*;
use crate::accounted_native_save::PreparedAccountedNativeSave;
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl, quota::QuotaKind},
};
use std::sync::Arc;
#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn saved_session() -> Session {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "source.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "source.xcf", Some("authorized/source.xcf"), &control()).unwrap();
    session.execute("layer.setProps", json!({"name":"first"})).unwrap();
    session.execute("layer.setProps", json!({"name":"second"})).unwrap();
    PreparedAccountedNativeSave::prepare(&session, "saved.pcraft", &control()).unwrap().write_and_publish(&mut session, &control(), |_, _| Ok(())).unwrap();
    session
}

#[test]
fn registered_jump_preserves_saved_title_and_redo_with_original_source_owners() {
    let mut session = saved_session();
    let before = session.active().unwrap().doc.clone();
    let end = session.active().unwrap().history.past_len();
    assert!(end >= 2);
    session.execute(CMD, json!({"state":0})).unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.doc.name, "saved.pcraft");
    assert_eq!(state.path.as_deref(), Some("saved.pcraft"));
    assert_eq!(state.doc.layers[0].name, "Layer");
    assert_eq!(state.doc.source_color_context, before.source_color_context);
    assert_eq!(state.doc.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
    assert!(state.history.can_redo());
    session.execute(CMD, json!({"state":end})).unwrap();
    assert_eq!(session.active().unwrap().doc.name, "saved.pcraft");
    assert_eq!(session.active().unwrap().doc.layers[0].name, "second");
}

#[test]
fn cancelled_or_quota_denied_title_repair_preserves_cursor_snapshot_and_redo() {
    for cancelled in [true, false] {
        let mut session = saved_session();
        let state = session.active().unwrap();
        let snapshot = state.doc.clone();
        let history = state.history.clone();
        let revision = state.revision;
        let journal = session.journal.clone();
        let params = json!({"state":0});
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&session, CMD, &params, &control()).unwrap();
        let quota = snapshot.quota().unwrap();
        let held = if cancelled {
            None
        } else {
            // Fill available index quota without changing the current model.
            let mut low = 0u64;
            let mut high = u64::MAX;
            while low < high {
                let mid = low + (high - low) / 2 + 1;
                match quota.reserve(QuotaKind::Index, mid) {
                    Ok(lease) => {
                        drop(lease);
                        low = mid;
                    }
                    Err(_) => high = mid - 1,
                }
            }
            Some(quota.reserve(QuotaKind::Index, low).unwrap())
        };
        let work = |_| if cancelled { Err(BackingError::Cancelled) } else { Ok(()) };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(prepared.execute(&mut session, &c).is_err());
        let state = session.active().unwrap();
        assert!(state.doc.shares_state(&snapshot));
        assert!(state.history.shares_accounted_history(&history));
        assert_eq!(state.revision, revision);
        assert_eq!(state.path.as_deref(), Some("saved.pcraft"));
        assert_eq!(session.journal, journal);
        drop(held);
    }
}
