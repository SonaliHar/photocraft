//! UNRUN registered imported metadata regressions; no runtime qualification.
use super::*;
use photocraft_raster::Interrupt;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
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
fn registered_compound_ignored_types_clamp_and_noop_preserve_redo_and_source() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let source = s.active().unwrap().source.clone();
    s.execute("layer.setProps", serde_json::json!({"name":"renamed","visible":false,"opacity":1e100})).unwrap();
    let layer = &s.active().unwrap().doc.layers[0];
    assert_eq!(layer.name, "renamed");
    assert!(!layer.visible);
    assert_eq!(layer.opacity, 1.0);
    assert!(s.undo_with_control(&control()).unwrap());
    assert!(before.shares_state(&s.active().unwrap().doc));
    s.execute("layer.setProps", serde_json::json!({"name":false,"visible":"ignored","opacity":[]})).unwrap();
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert!(s.redo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().source, source);
    assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().items[0].name, "renamed");
    s.execute("layer.setProps", serde_json::json!({"opacity":-3.0})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.0);
}
#[test]
fn whole_request_refusal_does_not_apply_supported_prefix() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    for p in [
        serde_json::json!({"name":"must not land","blend":"Multiply"}),
        serde_json::json!({"visible":false,"locks":{}}),
        serde_json::json!({"layer":0,"name":"absent"}),
    ] {
        assert!(s.execute("layer.setProps", p).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.journal, journal);
    }
}
#[test]
fn actual_name_and_null_targets_match_existing_registered_policy() {
    let mut s = session();
    let name = s.active().unwrap().doc.layers[0].name.clone();
    s.execute("layer.setProps", serde_json::json!({"layer":name,"name":"named"})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"layer":null,"opacity":0.25})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.25);
}
#[test]
fn prepared_view_and_channel_changes_refuse_before_edit_or_journal() {
    let mut s = session();
    let p = serde_json::json!({"name":"stale"});
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    let prepared = PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &control()).unwrap();
    s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
    assert!(prepared.execute(&mut s, &control()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    let prepared = PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &control()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &control()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
}
#[test]
fn late_cancel_keeps_snapshot_journal_and_returns_all_private_charges() {
    for stop_after in 0..96 {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        let quota = before.quota().unwrap().clone();
        let baseline = quota.usage().unwrap();
        let count = AtomicUsize::new(0);
        let stop = || count.fetch_add(1, Ordering::Relaxed) >= stop_after;
        let interrupt = Interrupt::cancel_only(&stop);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        let p = serde_json::json!({"name":"cancelled","visible":false,"opacity":0.5});
        if super::super::dispatch(&mut s, "layer.setProps", &p, &ctl).is_err() {
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.journal, journal);
            assert_eq!(quota.usage().unwrap(), baseline);
        }
    }
}
#[test]
fn actual_index_exhaustion_refuses_compound_before_mutation() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let quota = before.quota().unwrap().clone();
    let used = quota.usage().unwrap();
    let max = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().get(photocraft_xcf::BudgetKind::Index);
    let filler = quota.reserve(QuotaKind::Index, max - used.index).unwrap();
    let journal = s.journal.clone();
    assert!(s.execute("layer.setProps", serde_json::json!({"name":"quota","visible":false,"opacity":0.5})).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    drop(filler);
}

#[test]
fn imported_visibility_lock_blocks_changed_visibility_atomically_but_not_name_opacity() {
    let options = xcf::FixtureOptions { layer_properties: vec![(42, 1u32.to_be_bytes().to_vec())], ..Default::default() };
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "locked.xcf",
        Arc::new(xcf::image_custom(13, 150, 0, 1, 1, &[31, 61, 91, 255], &options)),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&imported, "locked.xcf", None, &control()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    assert!(s.execute("layer.setProps", serde_json::json!({"name":"refused","visible":false,"opacity":0.25})).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    s.execute("layer.setProps", serde_json::json!({"name":"allowed","visible":true,"opacity":0.25})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].name, "allowed");
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.25);
    assert!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().items[0].locks.visibility);
}

#[test]
fn nul_name_shape_passes_but_paid_prepare_refuses_before_journal_or_edit() {
    let mut s = session();
    let p = serde_json::json!({"name":"bad\0name","visible":false,"opacity":0.25});
    assert!(parse(&p).is_ok());
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    let quota = before.quota().unwrap().clone();
    let baseline = quota.usage().unwrap();
    assert!(PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &control()).is_err());
    assert!(s.execute("layer.setProps", p).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    assert_eq!(quota.usage().unwrap(), baseline);
}
