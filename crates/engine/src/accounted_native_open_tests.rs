//! UNRUN native save -> owned reload -> explicit Session publication drafts.
use super::*;
use photocraft_doc::{
    PixelFormat,
    accounted_document::AccountedDocument,
    accounted_selection::{AccountedSelection, CoverageDomain},
};
use photocraft_format::{
    LoadOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{OwnedDocumentLimits, load_document_from_owned_archive, save_current_document_archive},
};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Archive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn packet() -> (Arc<Archive>, BackingQuota, AccountedDocument) {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "original.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let q = imported.document().quota().clone();
    let selected = AccountedSelection::try_from_native(
        PixelFormat::GRAY8,
        imported.document().document().bounds(),
        &[0],
        &[128],
        CoverageDomain::FiniteUnit,
        &q,
        &control(),
    )
    .unwrap();
    let original = imported.document().try_replace_selection(Some(&selected), &control()).unwrap();
    let bytes =
        save_current_document_archive(original.document(), Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &control()).unwrap().into_inner();
    let held = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    (Arc::new(Archive { bytes, quota: q.clone(), _lease: held }), q, original)
}
fn load(source: Arc<Archive>, q: &BackingQuota) -> OwnedDocumentArchive {
    load_document_from_owned_archive(source, &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).unwrap()
}
#[test]
fn actual_saved_native_reopen_preserves_selection_and_inert_source_but_uses_actual_native_identity() {
    let (source, q, original) = packet();
    let archive = load(source, &q);
    let mut session = Session::new();
    session
        .add_owned_native_archive(
            archive,
            AccountedNativeOpenPolicy::KeepCurrentInterpretation,
            "current.pcraft",
            Some("authorized/current.pcraft"),
            &control(),
        )
        .unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.doc.id, original.document().id);
    assert_eq!(state.doc.xcf_state, original.document().xcf_state);
    assert_eq!(state.doc.source_color_context, original.document().source_color_context);
    assert_eq!(state.path.as_deref(), Some("authorized/current.pcraft"));
    assert!(!state.source_read_only);
    let runtime = state.source.as_ref().unwrap();
    assert_eq!(runtime.path.as_deref(), Some("authorized/current.pcraft"));
    assert_eq!(runtime.origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
    assert!(runtime.check_destination("other.xcf").is_err());
    assert_eq!(state.doc.selection.as_ref().unwrap().try_read_interleaved(state.doc.bounds()).unwrap(), vec![128]);
    assert!(!state.history.can_undo());
    let current = state.doc.accounted().unwrap().clone();
    assert!(current.try_replace_selection(None, &control()).unwrap().document().selection.is_none());
}
#[test]
fn duplicate_and_late_cancel_never_publish_partial_native_tab() {
    let (source, q, _original) = packet();
    let first = load(source.clone(), &q);
    let duplicate = load(source.clone(), &q);
    let mut session = Session::new();
    session.add_owned_native_archive(first, AccountedNativeOpenPolicy::KeepCurrentInterpretation, "first", None, &control()).unwrap();
    let old = session.active().unwrap().doc.clone();
    assert!(session.add_owned_native_archive(duplicate, AccountedNativeOpenPolicy::KeepCurrentInterpretation, "second", None, &control()).is_err());
    assert_eq!(session.docs.len(), 1);
    assert!(session.active().unwrap().doc.shares_state(&old));
    for threshold in [0u64, 1, 8, 32, 128] {
        let archive = load(source.clone(), &q);
        let mut target = Session::new();
        let work = AtomicU64::new(0);
        let late = ReadControl {
            interrupt: &Interrupt::NONE,
            work: &|n| {
                let previous = work.fetch_add(n, Ordering::Relaxed);
                if previous.saturating_add(n) > threshold { Err(BackingError::Cancelled) } else { Ok(()) }
            },
        };
        if target
            .add_owned_native_archive(archive, AccountedNativeOpenPolicy::KeepCurrentInterpretation, "opened", Some("authorized/opened.pcraft"), &late)
            .is_err()
        {
            assert!(target.docs.is_empty());
            assert!(target.active().is_none());
        }
    }
}
#[test]
fn malformed_labels_and_foreign_ledger_tabs_refuse() {
    let (source, q, _original) = packet();
    let mut session = Session::new();
    assert!(
        session
            .add_owned_native_archive(load(source.clone(), &q), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "bad\0label", None, &control())
            .is_err()
    );
    assert!(session.docs.is_empty());
    session.add_owned_native_archive(load(source, &q), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "first", None, &control()).unwrap();
    let (foreign, fq, _) = packet();
    assert!(session.add_owned_native_archive(load(foreign, &fq), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "foreign", None, &control()).is_err());
    assert_eq!(session.docs.len(), 1);
}

#[test]
fn catalog_quota_survives_resident_focus_and_last_accounted_tab_close() {
    let (source, quota, _original) = packet();
    let mut session = Session::new();
    assert!(session.accounted_open_quota().is_none());
    session.add_owned_native_archive(load(source.clone(), &quota), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "owned", None, &control()).unwrap();
    session
        .add_document(
            photocraft_doc::Document::new("resident", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
            None,
        )
        .unwrap();
    assert!(session.active().unwrap().doc.accounted().is_none());
    assert!(session.accounted_open_quota().unwrap().shares_ledger(&quota));
    drop(session.close(0).unwrap());
    assert!(session.accounted_open_quota().unwrap().shares_ledger(&quota));
    let (foreign, foreign_quota, _) = packet();
    assert!(
        session
            .add_owned_native_archive(load(foreign, &foreign_quota), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "foreign", None, &control())
            .is_err()
    );
    assert_eq!(session.docs.len(), 1);
    session.add_owned_native_archive(load(source, &quota), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "same ledger", None, &control()).unwrap();
    drop(session.close(1).unwrap());
    drop(session.close(0).unwrap());
    assert!(session.accounted_open_quota().is_none());
}
