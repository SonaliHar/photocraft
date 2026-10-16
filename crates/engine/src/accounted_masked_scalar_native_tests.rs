//! Deferred genuine-source continuation and portable four-family writer errors.
use super::*;
#[path = "accounted_masked_scalar_test_support.rs"]
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
#[ignore = "UNRUN genuine16 masked scalar Source owners, portable writer typed Work before output/refund, exact native11 Point marker routing and wrong generic/mixed denial. No producer numerical oracle."]
fn four_masked_scalar_native_writer_and_handoff_are_closed() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        let owner = source.document();
        fixture::observed(owner.document(), &case);
        let before = owner.quota().usage().unwrap();
        let calls = AtomicU64::new(0);
        let work = |_| if calls.fetch_add(1, Ordering::Relaxed) >= 6 { Err(BackingError::Work) } else { Ok(()) };
        let control = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
        let writes = Arc::new(AtomicU64::new(0));
        let err = photocraft_io::accounted_current_native::save_accounted_current_archive(
            owner,
            Writer { bytes: std::io::Cursor::new(Vec::new()), writes: writes.clone() },
            Default::default(),
            owner.quota(),
            &control,
        )
        .unwrap_err();
        let photocraft_format::FormatError::Io(err) = err else { panic!("typed backing error") };
        assert_eq!(err.get_ref().and_then(|e| e.downcast_ref::<BackingError>()), Some(&BackingError::Work));
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        assert_eq!(owner.quota().usage().unwrap(), before);
        let archive = fixture::archive(owner);
        assert_eq!(archive.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::PointF32Coverage));
        assert!(!is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut wrong = Session::new();
        assert!(wrong.add_owned_native_filtered_flat_archive(archive, "masked.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.documents().is_empty());
        assert!(
            wrong
                .add_owned_native_archive(fixture::archive(owner), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "masked.pcraft", None, &fixture::ctl())
                .is_err()
        );
        assert!(wrong.documents().is_empty());
        let mut right = Session::new();
        right.add_owned_native_point_archive(fixture::archive(owner), "masked.pcraft", None, &fixture::ctl()).unwrap();
        fixture::observed(&right.active().unwrap().doc, &case);
        assert!(right.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
    }
}
