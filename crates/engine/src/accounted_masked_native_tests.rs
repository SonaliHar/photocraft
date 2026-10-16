//! Deferred forced native Point handoff and portable writer failure boundaries.
use super::*;
#[path = "accounted_exposure_masked_test_support.rs"]
mod fixture;
use std::io::{Seek, Write};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[derive(Debug)]
struct Writer {
    bytes: std::io::Cursor<Vec<u8>>,
    writes: Arc<AtomicU64>,
}
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.bytes.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.bytes.flush()
    }
}
impl Seek for Writer {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        self.bytes.seek(p)
    }
}
#[test]
#[ignore = "UNRUN genuine6 masked source owners, portable writer Work after routing/before bytes with quota refunds, v11 exactPoint-only handoff/generic and mixed denial. No numerical oracle."]
fn masked_native_routing_and_writer_errors_stay_typed() {
    for case in fixture::cases() {
        let input = fixture::import(&case);
        let owner = input.document();
        let usage = owner.quota().usage().unwrap();
        let calls = AtomicU64::new(0);
        let work = |_| if calls.fetch_add(1, Ordering::Relaxed) >= 6 { Err(BackingError::Work) } else { Ok(()) };
        let ctl = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
        let writes = Arc::new(AtomicU64::new(0));
        let result = photocraft_io::accounted_current_native::save_accounted_current_archive(
            owner,
            Writer { bytes: std::io::Cursor::new(Vec::new()), writes: writes.clone() },
            Default::default(),
            owner.quota(),
            &ctl,
        );
        let error = result.unwrap_err();
        let photocraft_format::FormatError::Io(error) = error else { panic!("typed backing error") };
        assert_eq!(error.get_ref().and_then(|e| e.downcast_ref::<BackingError>()), Some(&BackingError::Work));
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        assert_eq!(owner.quota().usage().unwrap(), usage);
        fixture::observed(owner.document(), &case);
        let archive = fixture::archive(owner);
        assert_eq!(archive.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::PointF32Coverage));
        assert!(!is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut wrong = Session::new();
        assert!(wrong.add_owned_native_filtered_flat_archive(archive, "wrong.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.documents().is_empty());
        assert!(
            wrong
                .add_owned_native_archive(fixture::archive(owner), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "wrong.pcraft", None, &fixture::ctl())
                .is_err()
        );
        assert!(wrong.documents().is_empty());
        let mut correct = Session::new();
        correct.add_owned_native_point_archive(fixture::archive(owner), "masked.pcraft", None, &fixture::ctl()).unwrap();
        fixture::observed(&correct.active().unwrap().doc, &case);
        assert!(correct.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
    }
}
