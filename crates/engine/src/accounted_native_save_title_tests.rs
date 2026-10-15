//! UNRUN Save As title persistence across paid accounted history travel.
use super::*;
use photocraft_raster::Interrupt;
use std::sync::Arc;
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
fn save_as(s: &mut Session) {
    PreparedAccountedNativeSave::prepare(s, "copy.pcraft", &control()).unwrap().write_and_publish(s, &control(), |_, _| Ok(())).unwrap();
}
fn assert_identity(s: &Session, source: &Option<SourceIdentity>, original: &DocumentSnapshot) {
    let st = s.active().unwrap();
    assert_eq!(st.doc.name, "copy.pcraft");
    assert_eq!(st.path.as_deref(), Some("copy.pcraft"));
    assert_eq!(&st.source, source);
    assert!(st.source_read_only);
    assert_eq!(st.doc.xcf_state.as_ref().unwrap().provenance, original.xcf_state.as_ref().unwrap().provenance);
    assert_eq!(st.doc.source_color_context, original.source_color_context);
    let source = st.source.as_ref().unwrap();
    assert!(source.check_destination("authorized/source.xcf").is_err());
    assert!(source.check_destination("copy.pcraft").is_ok());
    let mut protected = crate::source_policy::WriteProtection::default();
    protected.add_document_with_interrupt(&st.doc, st.source.as_ref(), &Interrupt::NONE).unwrap();
    assert!(protected.sources.iter().any(|p| p == "authorized/source.xcf"));
    assert_eq!(protected.fingerprints, photocraft_io::source_protection::document_fingerprints(original).unwrap());
}
#[test]
fn save_as_then_edit_undo_and_redo_keep_file_identity_and_source_exclusions() {
    let mut s = session();
    let original = s.active().unwrap().doc.clone();
    let source = s.active().unwrap().source.clone();
    s.execute("layer.setProps", serde_json::json!({"name":"before save"})).unwrap();
    save_as(&mut s);
    s.execute("layer.setProps", serde_json::json!({"opacity":0.25})).unwrap();
    assert_identity(&s, &source, &original);
    for _ in 0..2 {
        assert!(s.undo_with_control(&control()).unwrap());
        assert_identity(&s, &source, &original);
    }
    assert!(!s.active().unwrap().history.can_undo());
    for _ in 0..2 {
        assert!(s.redo_with_control(&control()).unwrap());
        assert_identity(&s, &source, &original);
    }
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.25);
}
#[test]
fn late_cancelled_title_repair_undo_preserves_document_history_view_and_identity() {
    for limit in [1u64, 4, 16, 64, 256, 1024, 4096] {
        let mut s = session();
        s.execute("layer.setProps", serde_json::json!({"name":"before save"})).unwrap();
        save_as(&mut s);
        let st = s.active().unwrap();
        let old = st.doc.clone();
        let history = st.history.clone();
        let view = st.accounted_view.as_ref().unwrap().clone();
        let source = st.source.clone();
        let revision = st.revision;
        let saved = st.saved_revision;
        let q = old.quota().unwrap().clone();
        let usage = q.usage().unwrap();
        let units = std::sync::atomic::AtomicU64::new(0);
        let c = ReadControl {
            interrupt: &Interrupt::NONE,
            work: &|n| {
                let total = units.fetch_add(n, std::sync::atomic::Ordering::Relaxed).saturating_add(n);
                if total > limit { Err(BackingError::Cancelled) } else { Ok(()) }
            },
        };
        if s.undo_with_control(&c).is_err() {
            let st = s.active().unwrap();
            assert!(st.doc.shares_state(&old));
            assert!(st.history.shares_accounted_history(&history));
            assert!(st.accounted_view.as_ref().unwrap().shares_state(&view));
            assert_eq!(st.revision, revision);
            assert_eq!(st.saved_revision, saved);
            assert_eq!(st.source, source);
            assert_eq!(st.path.as_deref(), Some("copy.pcraft"));
            assert_eq!(q.usage().unwrap(), usage);
        }
    }
}
#[test]
fn quota_denied_undo_keeps_saved_title_and_undo_redo_owners() {
    let mut s = session();
    s.execute("layer.setProps", serde_json::json!({"name":"before save"})).unwrap();
    save_as(&mut s);
    let old = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let source = s.active().unwrap().source.clone();
    let q = old.quota().unwrap().clone();
    // Test-only bounded search for actual remaining Index headroom; no asserted
    // decoder/private quota limit and no allocation of the represented bytes.
    let (mut low, mut high) = (0u64, 1u64 << 40);
    while low < high {
        let mid = low + (high - low) / 2 + 1;
        if let Ok(held) = q.reserve(QuotaKind::Index, mid) {
            drop(held);
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let held = q.reserve(QuotaKind::Index, low).unwrap();
    let usage = q.usage().unwrap();
    assert!(s.undo_with_control(&control()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&old));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(q.usage().unwrap(), usage);
    assert_identity(&s, &source, &old);
    drop(held);
    assert!(s.undo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().doc.name, "copy.pcraft");
}
