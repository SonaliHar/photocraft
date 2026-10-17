//! Deferred generated-source native continuation. No external numerical oracle.
use super::*;
use crate::accounted_native_open::AccountedNativeOpenPolicy;
#[path = "accounted_wide_plain_xcf_test_support.rs"]
mod f;
use photocraft_color::SampleType;
use photocraft_raster::backing::{BackingError, ReadControl};
use std::{
    io::{Seek, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

fn session(gray: bool, sample: SampleType) -> Session {
    let source = f::import(gray, sample, false);
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&source, "wide-source.xcf", Some("authorized/wide-source.xcf"), &f::ctl()).unwrap();
    s.tools.brush.pressure_size = false;
    s
}
fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    // Encoded/comparison output is external harness memory, not a model receipt.
    let b = photocraft_io::accounted_current_native::save_accounted_current_archive(
        owner,
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        owner.quota(),
        &f::ctl(),
    )
    .unwrap()
    .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(&b),
        b.len() as u64,
        Default::default(),
        owner.quota().clone(),
        &f::ctl(),
    )
    .unwrap()
}

#[test]
#[ignore = "UNRUN/uncompiled: generated RGB/Gray U32/F64 brush edit, portable no-hint .pcraft reopen, second edit and exact undo/redo across repeated native saves; no producer numerical acceptance"]
fn wide_stroke_native_reopen_keeps_words_and_continues_editing() {
    for gray in [false, true] {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut s = session(gray, sample);
            let original = s.active().unwrap().doc.clone();
            let original_bytes = f::payload(&original);
            s.execute("paint.stroke", f::patch()).unwrap();
            assert_ne!(f::payload(&s.active().unwrap().doc), original_bytes);
            assert!(s.active().unwrap().source_read_only);
            assert_eq!(f::payload(&original), original_bytes);
            for pass in 0..2 {
                let saved = s.active().unwrap().doc.clone();
                let raw = f::payload(&saved);
                let a = archive(saved.accounted().unwrap());
                assert_eq!(a.current_storage_hint(), None);
                let mut reopened = Session::new();
                reopened
                    .add_owned_native_archive(
                        a,
                        AccountedNativeOpenPolicy::KeepCurrentInterpretation,
                        "wide-copy.pcraft",
                        Some("authorized/wide-copy.pcraft"),
                        &f::ctl(),
                    )
                    .unwrap();
                let st = reopened.active().unwrap();
                assert!(st.doc.accounted().unwrap().is_legacy_storage());
                assert_eq!(st.path.as_deref(), Some("authorized/wide-copy.pcraft"));
                assert!(!st.source_read_only);
                assert_eq!(st.doc.depth, sample);
                assert_eq!(f::payload(&st.doc), raw);
                assert_eq!(st.doc.source_color_context, saved.source_color_context);
                assert_eq!(st.doc.icc_profile, saved.icc_profile);
                assert_eq!(st.doc.xcf_state, saved.xcf_state);
                let before = st.doc.clone();
                let revision = st.revision;
                let provenance = st.source.clone();
                reopened.tools.brush.pressure_size = false;
                let mut p = f::patch();
                p["color"] = if pass == 0 {
                    serde_json::json!([0.12500000000000003, 0.6250000000000001, 0.8750000000000001, 1.0])
                } else {
                    serde_json::json!([0.8750000000000001, 0.37500000000000006, 0.12500000000000003, 1.0])
                };
                reopened.execute("paint.stroke", p).unwrap();
                let changed = reopened.active().unwrap().doc.clone();
                assert_ne!(f::payload(&changed), raw);
                assert_eq!(reopened.active().unwrap().revision, revision + 1);
                assert_eq!(reopened.active().unwrap().source, provenance);
                assert_eq!(f::payload(&before), raw);
                assert!(reopened.undo_with_control(&f::ctl()).unwrap());
                assert_eq!(f::payload(&reopened.active().unwrap().doc), raw);
                assert!(reopened.redo_with_control(&f::ctl()).unwrap());
                assert_eq!(f::payload(&reopened.active().unwrap().doc), f::payload(&changed));
                assert_eq!(f::payload(&original), original_bytes);
                s = reopened;
            }
        }
    }
}

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
#[ignore = "UNRUN/uncompiled: actual edited wide Legacy portable writer early Work error, no output and exact quota/history/source retention; comparison buffers are external harness allocations"]
fn wide_stroke_native_writer_early_refusal_keeps_current_and_original() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut s = session(false, sample);
        s.execute("paint.stroke", f::patch()).unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let source = s.active().unwrap().source.clone();
        let owner = before.accounted().unwrap();
        let baseline = owner.quota().usage().unwrap();
        let writes = Arc::new(AtomicU64::new(0));
        let deny = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        let error = photocraft_io::accounted_current_native::save_accounted_current_archive(
            owner,
            Writer { bytes: std::io::Cursor::new(Vec::new()), writes: writes.clone() },
            Default::default(),
            owner.quota(),
            &c,
        )
        .err()
        .unwrap();
        let photocraft_format::FormatError::Io(error) = error else { panic!("typed backing Work error") };
        assert_eq!(error.get_ref().and_then(|e| e.downcast_ref::<BackingError>()), Some(&BackingError::Work));
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        assert_eq!(owner.quota().usage().unwrap(), baseline);
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert!(history.shares_accounted_history(&s.active().unwrap().history));
        assert_eq!(s.active().unwrap().source, source);
    }
}
