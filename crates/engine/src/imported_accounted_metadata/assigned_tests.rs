//! UNRUN authentic assigned CURRENT metadata transactions. Source/archive/native
//! comparison captures and intentional cross-call stale-state mutations are external
//! test harness storage; all product model/profile/history/reply owners use real quota.
use super::*;
use crate::accounted_commands::PreparedAccountedCommand;
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
#[test]
#[ignore = "UNRUN: twelve actual RGBF64 original geometry sources, all operators and encodings, shared upstream parsing/gesture/oneanchor"]
fn assigned_compound_cross_layer_keys_first_label_and_wrongtype_noop_redo() {
    for case in 0..CASES.len() {
        let mut s = session(case);
        let initial = s.active().unwrap().doc.clone();
        let source = s.active().unwrap().source.clone();
        let profile = initial.icc_profile.clone();
        let base = initial.layers[0].id;
        let top = initial.layers[1].id;
        let mask = initial.layers[1].mask.as_ref().unwrap().surface.clone();
        s.execute("layer.setProps", serde_json::json!({"layer":top.0,"visible":false,"coalesce":"same\u{0}key"})).unwrap();
        s.execute("layer.setProps", serde_json::json!({"layer":base.0,"name":"Current backdrop","opacity":0.375,"coalesce":"same\u{0}key"})).unwrap();
        let state = s.active().unwrap();
        assert!(!state.doc.layers[1].visible);
        assert_eq!(state.doc.layers[0].opacity, 0.375);
        assert_eq!(state.doc.layers[0].name, "Current backdrop");
        assert_eq!(state.history.undo_label(), Some("Layer Visibility"));
        assert_eq!(state.history.past_len(), 1);
        assert_eq!(state.source, source);
        assert_eq!(state.doc.icc_profile, profile);
        assert_eq!(state.doc.source_color_context, initial.source_color_context);
        assert!(Arc::ptr_eq(&mask, &state.doc.layers[1].mask.as_ref().unwrap().surface));
        for (a, b) in initial.xcf_state.as_ref().unwrap().items.iter().zip(&state.doc.xcf_state.as_ref().unwrap().items) {
            assert_eq!(a.composition, b.composition);
        }
        assert!(s.undo_with_control(&control()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&initial));
        s.execute("layer.setProps", serde_json::json!({"name":false,"visible":"ignored","opacity":[],"coalesce":"different"})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&initial));
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&control()).unwrap());
        assert_eq!(s.active().unwrap().doc.layers[0].name, "Current backdrop");
    }
}
#[test]
#[ignore = "UNRUN: actual assigned source direct Session typed metadata route retains raw negative zero and validates bothcurrent/staged"]
fn direct_entry_signedzero_noop_and_cumulative_cancel_roll_back() {
    let mut s = session(2);
    let id = s.active().unwrap().doc.layers[0].id;
    let owner = s.active().unwrap().doc.accounted().unwrap();
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    crate::accounted_xcf_open::analytic_geometry::validate_current_analytic_geometry_storage(owner, &ReadControl { interrupt: &Interrupt::NONE, work: &work })
        .unwrap();
    let current_work = spent.load(Ordering::Relaxed);
    assert!(current_work > 100_000);
    let initial = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let revision = s.active().unwrap().revision;
    let spent = AtomicU64::new(0);
    let work =
        |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > current_work + current_work / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    let edit = AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: Some(-0.0) };
    assert!(s.edit_accounted("Layer Properties", edit, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert!(spent.load(Ordering::Relaxed) > current_work);
    assert!(s.active().unwrap().doc.shares_state(&initial));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.active().unwrap().revision, revision);
    s.edit_accounted("Layer Properties", edit, &control()).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].opacity.to_bits(), (-0.0f32).to_bits());
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    s.edit_accounted("Ignored noop", edit, &control()).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
}
#[test]
#[ignore = "UNRUN: actual source stale prepared source/path/readonly/history/max/preferences witnesses refuse atomically"]
fn assigned_prepared_fences_refuse_source_history_and_preferences_changes() {
    for change in 0..6 {
        let mut s = session(1);
        let p = serde_json::json!({"name":"must not land","coalesce":"key"});
        let prepared = PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &control()).unwrap();
        match change {
            0 => s.active_mut().unwrap().path = Some("different-current.pcraft".into()),
            1 => s.active_mut().unwrap().source.as_mut().unwrap().label = "different source".into(),
            2 => s.active_mut().unwrap().source_read_only = false,
            3 => s.active_mut().unwrap().history.max_states += 1,
            4 => s.active_mut().unwrap().history.set_current_label("changed history", &control()).unwrap(),
            _ => s.prefs.edit(|prefs| prefs.performance.history_states = prefs.performance.history_states.saturating_add(1)),
        }
        let doc = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let revision = s.active().unwrap().revision;
        assert!(prepared.execute(&mut s, &control()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&doc));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
        assert_eq!(s.active().unwrap().revision, revision);
    }
}
#[test]
#[ignore = "UNRUN: actual assigned source prospective quota and malformed requests retain installed model/history/key/journal"]
fn assigned_quota_and_unsupported_fields_do_not_apply_a_prefix() {
    let mut s = session(3);
    let q = s.active().unwrap().doc.quota().unwrap().clone();
    let doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("layer.setProps", serde_json::json!({"name":"unpaid"})).is_err());
    assert!(s.active().unwrap().doc.shares_state(&doc));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
    drop(hold);
    for params in [
        serde_json::json!({"name":"prefix","blend":"Multiply"}),
        serde_json::json!({"layer":0,"visible":false}),
        serde_json::json!({"name":"prefix","coalesce":"x".repeat(129)}),
    ] {
        assert!(s.execute("layer.setProps", params).is_err());
        assert!(s.active().unwrap().doc.shares_state(&doc));
        assert_eq!(s.journal, journal);
    }
}
