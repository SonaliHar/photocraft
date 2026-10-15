//! UNRUN real atomic writer with a controlled immutable provider debit. The
//! injected units test ledger plumbing, not measured work or large-image speed.
use super::*;
#[path = "../../../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use photocraft_format::canonical_archive::ImmutableCanonicalArchive;
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
#[derive(Debug)]
struct Trigger {
    enabled: AtomicBool,
    hits: AtomicU64,
    _fee: QuotaLease,
}
#[derive(Debug)]
struct Archive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    trigger: Arc<Trigger>,
    _payload: QuotaLease,
    _header: QuotaLease,
}
impl ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        if out.len() > 65_536 {
            return Err(BackingError::Bounds);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Length)?;
        let input = self.bytes.get(start..end).ok_or(BackingError::Bounds)?;
        if self.trigger.enabled.swap(false, Ordering::Relaxed) {
            self.trigger.hits.fetch_add(1, Ordering::Relaxed);
            c.check(8_000_000_001)?;
        }
        c.check(out.len() as u64)?;
        out.copy_from_slice(input);
        Ok(())
    }
}
fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> (photocraft_format::owned_document::OwnedDocumentArchive, Arc<Trigger>) {
    let raw = fixture::encoded(owner); // external harness serializer capture
    let q = owner.quota().clone();
    let payload = q.reserve(QuotaKind::Resident, (raw.len() * 2) as u64).unwrap();
    let header = q.reserve(QuotaKind::Index, 4096).unwrap();
    let trigger_fee = q.reserve(QuotaKind::Index, 4096).unwrap();
    let trigger = Arc::new(Trigger { enabled: AtomicBool::new(false), hits: AtomicU64::new(0), _fee: trigger_fee });
    let bytes = raw.clone();
    assert!(bytes.capacity() <= raw.len() * 2);
    let source = Arc::new(Archive { bytes, quota: q.clone(), trigger: trigger.clone(), _payload: payload, _header: header });
    let loaded =
        photocraft_format::owned_document::load_document_from_owned_archive(source, &Default::default(), Default::default(), q, &fixture::ctl()).unwrap();
    (loaded, trigger)
}
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-box-save-work-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN: actual Box native atomic save shares selected standard meter into cold provider/writer; lower external budget and original cancellation preserve destination/model/history/source/journal, then bounded save/reopen preserves native/profile"]
fn box_native_save_selected_work_and_external_failure_are_atomic() {
    for assigned in [false, true] {
        let dir = Directory::new();
        let (owner, literal) = fixture::fixture(photocraft_doc::SampleType::F32, assigned, 0);
        let (archive, trigger) = archive(&owner);
        let mut h = Headless::trusted_local();
        h.session.add_owned_native_point_archive(archive, "CURRENT", None, &fixture::ctl()).unwrap();
        let before = h.session.active().unwrap().doc.clone();
        let history = h.session.active().unwrap().history.clone();
        let source = h.session.active().unwrap().source.clone();
        let journal = h.session.journal.clone();
        let output = dir.0.join("saved.pcraft");
        std::fs::write(&output, b"old destination").unwrap();
        trigger.enabled.store(true, Ordering::Relaxed);
        let lower = photocraft_io::accounted_operation_work::OperationWork::legacy(8_000_000_000);
        let charge = |n| lower.check(n);
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
        assert!(h.save_owned_native_active(Some(&output), Some("pcraft"), &control).is_err());
        assert!(trigger.hits.load(Ordering::Relaxed) > 0);
        assert_eq!(std::fs::read(&output).unwrap(), b"old destination");
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().source, source);
        assert_eq!(h.session.journal, journal);
        let cancel = || true;
        assert!(h.save_owned_native_active_bounded(Some(&output), Some("pcraft"), &Interrupt::cancel_only(&cancel)).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"old destination");
        trigger.enabled.store(true, Ordering::Relaxed);
        let hits = trigger.hits.load(Ordering::Relaxed);
        h.save_owned_native_active_bounded(Some(&output), Some("pcraft"), &Interrupt::NONE).unwrap();
        assert!(trigger.hits.load(Ordering::Relaxed) > hits);
        assert_eq!(h.session.active().unwrap().history.past_len(), history.past_len());
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::exact(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(fixture::native(&reopened.session.active().unwrap().doc), literal.raw);
    }
}
