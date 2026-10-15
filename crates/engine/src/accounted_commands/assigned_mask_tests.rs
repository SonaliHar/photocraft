//! UNRUN authentic assigned mask command/direct-entry workflows. Source/native
//! archive captures and deliberate stale-state controls are external harness data.
//! Synthetic locks are CURRENT-only controls under an actual prepaid clone guard;
//! no edited XCF source oracle or quota claim for harness Vecs is fabricated.
use super::*;
use crate::{DocumentSnapshot, accounted_commands::PreparedAccountedCommand};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
const CASES: [(&str, &str, usize); 12] = [
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.xcf", "d09764e320785b11fe20ad38ae29a0a228b8422891e9dd2e6c114762297a9788", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op2.xcf", "a40f9837d56038faed84e09cf5b5127f31843154ecb15ca8762901932386ca02", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op3.xcf", "d8358732abedfbc7e9a5261349a60f3446aac514d3cab4533f02e75f6c5f5b31", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op4.xcf", "605ebdf92089ef77927d3a39581db3d84b741ad9df57fd7b7a930f350c93f6cb", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op1.xcf", "45a731c7dec5f4004ce4ce95cc08a5d7158643f5f92df45fbb7f8a22565277a8", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op2.xcf", "0f58ccb432193ad8d8b2e598a85edcd0d786bdeec38ae2ef4fc55b085bee25d3", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op3.xcf", "ea80f910a08de501fb969803d8fdc348bd73d971e62969e7bdc2dc0dbaae2477", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op4.xcf", "707c317381ab0fc1b390beb5f1e377e0a3c2d61bf0f6ec99eec1f98cef01f02f", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op1.xcf", "a811e5a2f8f3e46251479b0ae5ea855fa84b6b7d14c9c47da6ed1e1969ba75ec", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op2.xcf", "2d886b9e924c57f0ee05c1ab35e6cceac13eae0f90f6017f47149d34b353aad0", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op3.xcf", "d3ded016dc359693fce24312db211df66bcdac6442998349205d578e2bbcaa29", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op4.xcf", "ffa5d4e0152466684f3f6dec172b4005c2c484a7a2a0a45956f88377602f7ef7", 13710),
];
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn session(case: usize) -> Session {
    let (path, pin, len) = CASES[case];
    let bytes = std::fs::read(PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(path)).unwrap();
    assert_eq!(bytes.len(), len);
    assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), pin);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result =
        photocraft_io::import_owned_xcf_from_reader_in_quota(path, &mut Cursor::new(&bytes), bytes.len() as u64, &Default::default(), &limits, q, &control())
            .unwrap();
    let photocraft_io::OwnedXcfImport::AnalyticGeometry(owner) = result else { panic!("sealed actual geometry source") };
    let mut session = Session::new();
    session.add_owned_analytic_geometry_xcf_import(&owner, path, Some(path), &control()).unwrap();
    drop(owner);
    session
}

fn mask(s: &Session) -> MaskId {
    s.active().unwrap().doc.xcf_state.as_ref().unwrap().items.iter().find_map(|item| if let ItemRef::Mask(id) = item.item { Some(id) } else { None }).unwrap()
}
fn enabled(s: &Session) -> bool {
    s.active().unwrap().doc.layers[1].mask.as_ref().unwrap().enabled
}
fn run(s: &mut Session, value: bool) -> Value {
    let id = mask(s);
    s.execute("xcf.mask.enabled.set", serde_json::json!({"maskId":id.0.to_string(),"enabled":value})).unwrap()
}
#[test]
#[ignore = "UNRUN: twelve original assigned geometry packets, exact reply/no-journal/undo/noopredo/raw mask ownership"]
fn actual_mask_command_keeps_four_fields_raw_owners_and_journal_false() {
    for case in 0..CASES.len() {
        let mut s = session(case);
        s.execute("layer.setProps", serde_json::json!({"name":"Current title","coalesce":"gesture"})).unwrap();
        assert!(s.active().unwrap()._accounted_metadata_gesture.is_some());
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        let source = s.active().unwrap().source.clone();
        let surface = before.layers[1].mask.as_ref().unwrap().surface.clone();
        let id = mask(&s);
        let layer = before.layers[1].id;
        assert_eq!(run(&mut s, false), serde_json::json!({"maskId":id.0.to_string(),"layerId":layer.0.to_string(),"enabled":false,"changed":true}));
        assert!(!enabled(&s));
        assert_eq!(s.journal, journal);
        let state = s.active().unwrap();
        assert!(state._accounted_metadata_gesture.is_none());
        assert_eq!(state.history.undo_label(), Some("Set XCF Layer Mask Enabled"));
        assert!(Arc::ptr_eq(&surface, &state.doc.layers[1].mask.as_ref().unwrap().surface));
        assert_eq!(state.source, source);
        assert_eq!(state.doc.icc_profile, before.icc_profile);
        assert_eq!(state.doc.source_color_context, before.source_color_context);
        let canonical = state.doc.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Mask(id)).unwrap();
        assert!(!canonical.mask.as_ref().unwrap().enabled);
        assert_eq!(state.doc.xcf_state.as_ref().unwrap().opaque, before.xcf_state.as_ref().unwrap().opaque);
        assert!(s.undo_with_control(&control()).unwrap());
        assert!(enabled(&s));
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        assert_eq!(run(&mut s, true)["changed"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&control()).unwrap());
        assert!(!enabled(&s));
        assert_eq!(s.journal, journal);
    }
}
fn locked_current(kind: usize) -> Session {
    let s = session(2);
    let source = s.active().unwrap().doc.accounted().unwrap();
    let q = source.quota().clone();
    // These twelve fixed packets are <64KiB encoded; copied model containers
    // contain only two layers/three items and shared source/native/mask Arcs.
    let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    let target_layer = doc.layers[1].id;
    match kind {
        0 => doc.layers[1].locks.all = true,
        1 => doc.layers[1].locks.pixels = true,
        2 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(target_layer)).unwrap().locks.content = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| matches!(item.item, ItemRef::Mask(_))).unwrap().locks.content = true,
        _ => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(target_layer)).unwrap().locks.visibility = true,
    }
    let owner = AccountedDocument::from_owned(doc, Arc::new((source.clone(), fee)), q.clone(), &control()).unwrap();
    let snapshot = DocumentSnapshot::from_accounted(owner).unwrap();
    let bytes = photocraft_format::owned_document::save_current_document_archive(&snapshot, Cursor::new(Vec::new()), Default::default(), &q, &control())
        .unwrap()
        .into_inner();
    let archive =
        photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&bytes), bytes.len() as u64, Default::default(), q, &control())
            .unwrap();
    let mut current = Session::new();
    current.add_owned_native_analytic_geometry_archive(archive, "actual-current.pcraft", None, &control()).unwrap();
    current
}
#[test]
#[ignore = "UNRUN: native CURRENT owner/mask content locks reject before no-op; pixel-only/visibility locks permit mask flag"]
fn locks_are_enforced_even_for_same_value_and_do_not_confuse_pixels_with_mask_content() {
    for kind in 0..5 {
        let mut s = locked_current(kind);
        let id = mask(&s);
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        for value in [true, false] {
            let response = s.execute("xcf.mask.enabled.set", serde_json::json!({"maskId":id.0.to_string(),"enabled":value}));
            if matches!(kind, 0 | 2 | 3) {
                assert!(response.is_err());
                assert!(s.active().unwrap().doc.shares_state(&before));
            } else {
                assert_eq!(response.unwrap()["enabled"], value);
            }
            assert_eq!(s.journal, journal);
        }
    }
}
#[test]
#[ignore = "UNRUN: Retain journal policy fences full unchanged journal plus actual source/history/channel state"]
fn prepared_mask_fences_and_noncanonical_wire_fail_before_publication() {
    for change in 0..4 {
        let mut s = session(0);
        s.execute("layer.setProps", serde_json::json!({"name":"Journal anchor"})).unwrap();
        let p = serde_json::json!({"maskId":mask(&s).0.to_string(),"enabled":false});
        let prepared = PreparedAccountedCommand::prepare(&s, "xcf.mask.enabled.set", &p, &control()).unwrap();
        match change {
            0 => s.journal.push(("external-harness-change".into(), Value::Null)),
            1 => s.active_mut().unwrap().source_read_only = false,
            2 => {
                let max = s.active().unwrap().history.max_bytes.saturating_sub(1);
                s.active_mut().unwrap().history.max_bytes = max;
            }
            _ => {
                let target = s.active().unwrap().doc.layers[1].id;
                s.active_mut().unwrap().channel_view.editing_mask = Some(target);
            }
        }
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        let revision = s.active().unwrap().revision;
        assert!(prepared.execute(&mut s, &control()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.journal, journal);
        assert_eq!(s.active().unwrap().revision, revision);
    }
    let mut s = session(1);
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    for p in [
        serde_json::json!({"maskId":"01","enabled":true}),
        serde_json::json!({"maskId":"0","enabled":true}),
        serde_json::json!({"maskId":"18446744073709551616","enabled":true}),
        serde_json::json!({"maskId":"١","enabled":true}),
        serde_json::json!({"maskId":mask(&s).0.to_string(),"enabled":false,"coalesce":"x"}),
    ] {
        assert!(s.execute("xcf.mask.enabled.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.journal, journal);
    }
}
#[test]
#[ignore = "UNRUN: direct typed mask label/coalescing boundary and late current/staged cancellation/quota rollback"]
fn direct_mask_transaction_preserves_caller_label_and_all_errors_leave_state_intact() {
    let mut s = session(3);
    let layer = s.active().unwrap().doc.layers[1].id;
    let owner = s.active().unwrap().doc.accounted().unwrap();
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    crate::accounted_xcf_open::analytic_geometry::validate_current_analytic_geometry_storage(owner, &ReadControl { interrupt: &Interrupt::NONE, work: &work })
        .unwrap();
    let current_work = spent.load(Ordering::Relaxed);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let spent = AtomicU64::new(0);
    let work =
        |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > current_work + current_work / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    let edit = AccountedEdit::MaskEnabled { layer, enabled: false };
    assert!(s.edit_accounted("My mask control", edit, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert!(spent.load(Ordering::Relaxed) > current_work);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
    let q = s.active().unwrap().doc.quota().unwrap().clone();
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.edit_accounted("My mask control", edit, &control()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    drop(hold);
    s.coalesce_request = Some("forbidden".into());
    assert!(s.edit_accounted("My mask control", edit, &control()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    s.coalesce_request = None;
    s.edit_accounted("My mask control", edit, &control()).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("My mask control"));
    assert!(!enabled(&s));
    assert_eq!(s.journal, journal);
}
