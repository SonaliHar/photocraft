//! Implementation drafts: execution is deferred until the implementation pass ends.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use photocraft_engine::{Session, accounted_commands::PreparedAccountedCommand, accounted_selection_session::AccountedSelectionTarget};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use serde_json::{Value, json};
use std::sync::Arc;
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn session() -> (Session, BackingQuota) {
    let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 4 << 20, scratch: 8 << 20 });
    // A real same-quota native source supplies the required raster owner;
    // an empty raw Document is not valid accounted storage.
    let raw = 0x80000001u32.to_be_bytes().repeat(4 * 4 * 4);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 16 << 20), ("indexBytes", 2 << 20), ("scratchBytes", 0)])
        .unwrap();
    let input = photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "selection.xcf",
        Arc::new(xcf::image_custom(13, 350, 0, 4, 4, &raw, &Default::default())),
        &Default::default(),
        &limits,
        quota.clone(),
        &control(),
    )
    .unwrap();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&input, "selection.xcf", None, &control()).unwrap();
    (session, quota)
}
fn selection(quota: &BackingQuota) -> (AccountedSelection, Vec<u8>) {
    let bytes = [1u32, 0x8000_0001, u32::MAX, 0].repeat(4).into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
    let selected = AccountedSelection::try_from_native(
        PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false),
        Rect::new(0, 0, 4, 4),
        &0u32.to_ne_bytes(),
        &bytes,
        CoverageDomain::FiniteUnit,
        quota,
        &control(),
    )
    .unwrap();
    (selected, bytes)
}
#[test]
fn native_selection_replace_deselect_undo_redo_and_noop_retain_exact_words() {
    let (mut session, quota) = session();
    let original = session.active().unwrap().doc.clone();
    let (selected, bytes) = selection(&quota);
    let target = AccountedSelectionTarget::capture(&session).unwrap();
    session.replace_accounted_selection(target, "Own Selection", Some(&selected), &control()).unwrap();
    let current = session.active().unwrap().doc.clone();
    assert_eq!(current.selection.as_ref().unwrap().try_read_interleaved_with(Rect::new(0, 0, 4, 4), &Interrupt::NONE).unwrap(), bytes);
    let revision = session.active().unwrap().revision;
    let past = session.active().unwrap().history.past_len();
    session.replace_accounted_selection(AccountedSelectionTarget::capture(&session).unwrap(), "Same", Some(&selected), &control()).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&current));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.past_len(), past);
    assert_eq!(session.execute("select.deselect", json!({})).unwrap(), Value::Null);
    assert!(session.active().unwrap().doc.selection.is_none());
    assert_eq!(session.execute("edit.undo", Value::Null).unwrap(), Value::Bool(true));
    assert!(session.active().unwrap().doc.shares_state(&current));
    assert_eq!(session.execute("edit.undo", Value::Null).unwrap(), Value::Bool(true));
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(session.execute("edit.redo", Value::Null).unwrap(), Value::Bool(true));
    assert!(session.active().unwrap().doc.shares_state(&current));
    drop(original);
    drop(current);
    drop(selected);
    drop(session);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn stale_target_wrong_ledger_and_work_denial_preserve_model_history_and_revision() {
    let (mut session, quota) = session();
    let original = session.active().unwrap().doc.clone();
    let target = AccountedSelectionTarget::capture(&session).unwrap();
    let other = BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 4 << 20, scratch: 8 << 20 });
    let (foreign, _) = selection(&other);
    assert!(session.replace_accounted_selection(target.clone(), "Foreign", Some(&foreign), &control()).is_err());
    let (selected, _) = selection(&quota);
    let mut stale = target.clone();
    stale.view_revision += 1;
    assert!(session.replace_accounted_selection(stale, "Stale", Some(&selected), &control()).is_err());
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    let before = quota.usage().unwrap();
    assert!(session.replace_accounted_selection(target.clone(), "Denied", Some(&selected), &deny).is_err());
    assert_eq!(quota.usage().unwrap(), before);
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(session.active().unwrap().revision, target.revision);
    assert!(!session.active().unwrap().history.can_undo());
}
#[test]
fn registered_deselect_captured_pair_and_prepared_view_fence_are_atomic() {
    let (mut session, quota) = session();
    let (selected, _) = selection(&quota);
    session.replace_accounted_selection(AccountedSelectionTarget::capture(&session).unwrap(), "Own", Some(&selected), &control()).unwrap();
    let current = session.active().unwrap().doc.clone();
    let target = AccountedSelectionTarget::capture(&session).unwrap();
    let params = json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string()});
    let prepared = PreparedAccountedCommand::prepare(&session, "select.deselect", &params, &control()).unwrap();
    session.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut session, &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&current));
    assert!(session.journal.is_empty());
    for params in [json!({"documentId":"01","expectedRevision":target.revision.to_string()}), json!({"expectedRevision":"0"}), json!({"unexpected":true})] {
        assert!(session.execute("select.deselect", params).is_err());
        assert!(session.active().unwrap().doc.shares_state(&current));
        assert!(session.journal.is_empty());
    }
}

#[test]
fn matching_counters_cannot_replace_captured_model_or_diverge_from_history() {
    let (mut session, quota) = session();
    let target = AccountedSelectionTarget::capture(&session).unwrap();
    let (selected, _) = selection(&quota);
    let current = &session.active().unwrap().doc;
    let owner = current
        .accounted()
        .unwrap()
        .try_edit(photocraft_doc::accounted_document::AccountedEdit::LayerName { layer: current.layers[0].id, name: "replacement model" }, &control())
        .unwrap();
    assert_eq!(owner.document().id, target.document);
    let replaced = photocraft_engine::DocumentSnapshot::from_accounted(owner).unwrap();
    session.active_mut().unwrap().doc = replaced.clone();
    assert!(session.replace_accounted_selection(target, "Old capture", Some(&selected), &control()).is_err());
    let fresh = AccountedSelectionTarget::capture(&session).unwrap();
    assert!(session.replace_accounted_selection(fresh, "History mismatch", Some(&selected), &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&replaced));
    assert!(session.active().unwrap().doc.selection.is_none());
    assert!(!session.active().unwrap().history.can_undo());
}
