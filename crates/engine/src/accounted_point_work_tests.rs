//! UNRUN finite policy routing and real paid CURRENT transactions. Synthetic
//! fixtures are custody controls, not original-source or numerical evidence.
use crate::{
    Session,
    accounted_commands::{self, PreparedAccountedCommand},
};
use photocraft_doc::{SampleType, accounted_document::AccountedDocument};
use photocraft_io::accounted_operation_work::OperationWork;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
#[path = "../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod box_fixture;
#[path = "../../ui-egui/src/pure_point_geometry_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::Family;
fn hint(owner: &AccountedDocument, c: &ReadControl<'_>) -> bool {
    crate::accounted_native_open::is_current_standard_work_candidate(owner, c).unwrap()
}
fn session(owner: &AccountedDocument) -> Session {
    let mut s = Session::new();
    s.add_owned_native_point_archive(fixture::archive(owner), "Paid CURRENT policy control", None, &fixture::ctl()).unwrap();
    s
}
fn owner(s: &Session) -> &AccountedDocument {
    s.active().unwrap().doc.accounted().unwrap()
}
fn filter(s: &Session) -> String {
    s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()
}
struct CloneGuard {
    _root: AccountedDocument,
    _index: QuotaLease,
    _resident: QuotaLease,
}
fn changed(root: AccountedDocument, apply: impl FnOnce(&mut photocraft_doc::Document)) -> AccountedDocument {
    let q = root.quota().clone();
    let index = q.reserve(QuotaKind::Index, 4 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 4 << 20).unwrap();
    let mut doc = root.document().clone();
    apply(&mut doc);
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(actual.retained_bytes() <= 4 << 20);
    AccountedDocument::new_readonly_point_storage(doc, Arc::new(CloneGuard { _root: root, _index: index, _resident: resident }), q, &fixture::ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: exact runtime family hints route finite service, malformed claims still fail typed CURRENT; hint is not semantic permission"]
fn actual_family_and_malformed_claims_are_not_source_or_current_grants() {
    for family in [Family::Semi, Family::Hue, Family::Vibrance] {
        for sample in [SampleType::F32, SampleType::F64] {
            let original = fixture::fixture(family, sample, 1);
            assert!(hint(&original, &fixture::ctl()));
            for variant in 0..3 {
                let malformed = changed(original.clone(), |d| {
                    let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                    match variant {
                        0 => n.operation.name = "gegl:not-supported".into(),
                        1 => n.operation.schema_epoch = "wrong-schema".into(),
                        _ => {
                            n.operation.name = "gegl:not-supported".into();
                            n.operation.schema_epoch = "wrong-schema".into();
                        }
                    }
                });
                assert!(hint(&malformed, &fixture::ctl())); // retained name/schema/compatibility claims only
                let meter = OperationWork::standard(&limits()).unwrap();
                let deadline = || meter.deadline_expired();
                let interrupt = Interrupt::cancel_only(&deadline);
                let debit = |n| meter.check(n);
                let c = ReadControl { interrupt: &interrupt, work: &debit };
                let mut target = Session::new();
                assert!(target.add_owned_native_point_archive(fixture::archive(&malformed), "Malformed claimed family", None, &c).is_err());
                assert!(target.active().is_none());
                assert!(meter.diagnostics().used > 0);
            }
            let unrelated = changed(original, |d| {
                let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                n.operation.name = "gegl:unrelated".into();
                n.operation.schema_epoch = "unrelated".into();
                n.composition.as_mut().unwrap().compatibility_epoch = "unrelated".into();
            });
            assert!(!hint(&unrelated, &fixture::ctl()));
        }
    }
}
#[test]
#[ignore = "UNRUN: separate derived-empty role and origin/precision boundaries; Box compatibility hint unchanged"]
fn derived_empty_is_bounded_current_not_deleted_operation_provenance() {
    for sample in [SampleType::F32, SampleType::F64] {
        let root = fixture::fixture(Family::Vibrance, sample, 1);
        let mut s = session(&root);
        let id = filter(&s);
        s.execute("xcf.filter.remove", json!({"filterId":id})).unwrap();
        let empty = owner(&s).clone();
        assert!(hint(&empty, &fixture::ctl()));
        assert!(empty.document().xcf_state.as_ref().unwrap().filter_masks.is_empty());
        assert!(!crate::accounted_native_open::is_current_box_work_candidate(empty.document(), &fixture::ctl()).unwrap());
        let shifted = changed(empty.clone(), |d| {
            let photocraft_doc::LayerContent::BackedRaster(b) = &d.layers[0].content else { panic!("backed") };
            let next = b.try_translated(1, 0, &fixture::ctl()).unwrap();
            let bounds = next.operative_bounds();
            d.layers[0].content = photocraft_doc::LayerContent::BackedRaster(next);
            d.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(bounds);
        });
        assert!(!hint(&shifted, &fixture::ctl()));
        if sample == SampleType::F32 {
            let half_code = changed(empty, |d| d.source_color_context.as_mut().unwrap().source_precision = 550);
            assert!(!hint(&half_code, &fixture::ctl()));
        }
    }
    for assigned in [false, true] {
        let (boxed, _) = box_fixture::fixture(SampleType::F32, assigned, 1);
        assert!(crate::accounted_native_open::is_current_box_work_candidate(boxed.document(), &fixture::ctl()).unwrap());
        // Combined service includes the unchanged Box hint; the new pure
        // selector remains separate for registered command policy.
        assert!(hint(&boxed, &fixture::ctl()));
        assert!(!super::is_current_pure_point_work_candidate(&boxed, &fixture::ctl()).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: prior work plus selected finite policy uses one meter through real old/staged transaction and exact raw noop"]
fn selected_meter_preserves_usage_and_external_lower_controls() {
    let root = fixture::fixture(Family::Semi, SampleType::F64, 1);
    let mut s = session(&root);
    let p = Family::Semi.changed(&filter(&s));
    let meter = OperationWork::legacy(1 << 32);
    meter.check(17).unwrap();
    let before_select = meter.diagnostics().used;
    assert!(hint(owner(&s), &fixture::ctl()));
    meter.select_standard(&limits()).unwrap();
    assert_eq!(meter.diagnostics().used, before_select);
    assert_eq!(meter.diagnostics().limit, limits().get(photocraft_xcf::BudgetKind::Work));
    assert_eq!(meter.diagnostics().deadline_ms, Some(120_000));
    let token = AtomicBool::new(false);
    let cancelled = || token.load(Ordering::Relaxed) || meter.deadline_expired();
    let interrupt = Interrupt::cancel_only(&cancelled);
    let debit = |n| meter.check(n);
    let c = ReadControl { interrupt: &interrupt, work: &debit };
    let prepared = PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &c).unwrap();
    let prepared_used = meter.diagnostics().used;
    assert!(prepared_used > before_select);
    assert_eq!(prepared.execute(&mut s, &c).unwrap()["updated"], true);
    assert!(meter.diagnostics().used > prepared_used);
    let current = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    assert_eq!(accounted_commands::dispatch(&mut s, "xcf.filter.set", &p, &c).unwrap()["updated"], false);
    assert!(s.active().unwrap().doc.shares_state(&current));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    let lower = |_: u64| Err(BackingError::Work);
    let external = ReadControl { interrupt: &interrupt, work: &lower };
    assert!(PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &external).is_err());
    token.store(true, Ordering::Relaxed);
    assert!(PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &c).is_err());
    assert!(s.active().unwrap().doc.shares_state(&current));
    let expired = OperationWork::standard(&limits().lowered(&[("deadlineMs", 0)]).unwrap()).unwrap();
    let expired_poll = || expired.deadline_expired();
    let interrupt = Interrupt::cancel_only(&expired_poll);
    let debit = |n| expired.check(n);
    assert!(PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &interrupt, work: &debit }).is_err());
    assert!(expired.deadline_expired());
    assert_eq!(expired.diagnostics().used, 0);
}
#[test]
#[ignore = "UNRUN: same finite ledger pays history/view/metadata queries and title copy; no host wrapper or renderer authority inferred"]
fn history_view_query_and_title_have_actual_debits() {
    let root = fixture::fixture(Family::Hue, SampleType::F32, 1);
    let mut s = session(&root);
    let meter = OperationWork::standard(&limits()).unwrap();
    let deadline = || meter.deadline_expired();
    let interrupt = Interrupt::cancel_only(&deadline);
    let debit = |n| meter.check(n);
    let c = ReadControl { interrupt: &interrupt, work: &debit };
    let p = Family::Hue.changed(&filter(&s));
    accounted_commands::dispatch(&mut s, "xcf.filter.set", &p, &c).unwrap();
    for (id, p) in
        [("edit.undo", Value::Null), ("edit.redo", Value::Null), ("history.selectState", json!({"state":0})), ("history.selectState", json!({"state":1}))]
    {
        let used = meter.diagnostics().used;
        accounted_commands::dispatch(&mut s, id, &p, &c).unwrap();
        assert!(meter.diagnostics().used > used);
    }
    let layer = s.active().unwrap().doc.layers[0].id.0;
    let used = meter.diagnostics().used;
    accounted_commands::dispatch(&mut s, "layer.select", &json!({"layer":layer,"mode":"replace"}), &c).unwrap();
    assert!(meter.diagnostics().used > used);
    let used = meter.diagnostics().used;
    let result = crate::accounted_xcf_inspect::prepare(&s, &Value::Null, &c).unwrap();
    assert_eq!(result.value()["filters"]["total"], 1);
    drop(result);
    let result = crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":filter(&s)}), &c).unwrap();
    drop(result);
    assert!(meter.diagnostics().used > used);
    let used = meter.diagnostics().used;
    let named = owner(&s).try_with_document_name("Paid title", &c).unwrap();
    assert_eq!(named.document().name, "Paid title");
    assert!(meter.diagnostics().used > used);
    fixture::storage(root.document(), named.document());
}
#[test]
#[ignore = "UNRUN: old/staged/reply preparation followed by Work/cancel/stale-view refusal preserves document/history/journal"]
fn late_denial_and_stale_view_never_publish() {
    let root = fixture::fixture(Family::Vibrance, SampleType::F64, 1);
    let mut s = session(&root);
    let p = Family::Vibrance.changed(&filter(&s));
    let snapshot = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let meter = OperationWork::standard(&limits()).unwrap();
    let calls = AtomicU64::new(0);
    let deny = AtomicBool::new(false);
    let debit = |n| {
        calls.fetch_add(1, Ordering::Relaxed);
        if deny.load(Ordering::Relaxed) { Err(BackingError::Work) } else { meter.check(n) }
    };
    let deadline = || meter.deadline_expired();
    let interrupt = Interrupt::cancel_only(&deadline);
    let c = ReadControl { interrupt: &interrupt, work: &debit };
    let prepared = PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &c).unwrap();
    let used = meter.diagnostics().used;
    assert!(used > 0);
    deny.store(true, Ordering::Relaxed);
    assert!(prepared.execute(&mut s, &c).is_err());
    assert_eq!(meter.diagnostics().used, used);
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(s.active().unwrap().doc.shares_state(&snapshot));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
    let prepared = PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &fixture::ctl()).unwrap();
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(prepared.execute(&mut s, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert!(s.active().unwrap().doc.shares_state(&snapshot));
    assert_eq!(s.journal, journal);
    let prepared = PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&snapshot));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
}
