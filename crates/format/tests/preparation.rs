//! Preparation must observe cancellation before archive output, not only afterwards.
use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size};
use photocraft_format::{FormatError, PcraftWriter};
use photocraft_raster::Interrupt;
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
fn cancellation_during_object_preparation_precedes_late_manifest_error() {
    let mut doc = Document::new("preparation", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.metadata.exif = Some(Arc::new(vec![17; 8 << 20]));
    let original_blob = doc.metadata.exif.as_ref().unwrap().clone();
    let mut layer = Layer::raster("invalid late scalar", doc.pixel_format());
    layer.opacity = f32::NAN;
    doc.layers.push(layer);
    let polls = AtomicUsize::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 8;
    let mut output = Cursor::new(Vec::new());
    let result = PcraftWriter::new().save_zip_stream(&doc, &Default::default(), &mut output, Default::default(), 64 << 20, &Interrupt::cancel_only(&cancelled));
    assert!(matches!(result, Err(FormatError::Cancelled)), "preparation returned {result:?}");
    assert!(polls.load(Ordering::Relaxed) >= 9);
    assert!(output.get_ref().is_empty());
    assert!(Arc::ptr_eq(doc.metadata.exif.as_ref().unwrap(), &original_blob));
    assert_eq!(doc.layers[0].opacity.to_bits(), f32::NAN.to_bits());
}

#[test]
fn lowered_metadata_limits_leave_archive_and_document_unchanged() {
    use photocraft_format::PreparationLimits;
    let mut doc = Document::new("Own metadata", Size::new(1, 1), ColorMode::Rgb, SampleType::U32);
    doc.metadata.xmp = Some("literal Ω".repeat(20000));
    let before = doc.metadata.xmp.clone();
    for limits in [PreparationLimits { resident_bytes: 0, ..Default::default() }, PreparationLimits { manifest_bytes: 1, ..Default::default() }] {
        let mut writer = PcraftWriter::new();
        writer.set_preparation_limits(limits).unwrap();
        let mut output = Cursor::new(Vec::new());
        assert!(matches!(
            writer.save_zip_stream(&doc, &Default::default(), &mut output, Default::default(), 64 << 20, &Interrupt::NONE),
            Err(FormatError::LimitExceeded(_))
        ));
        assert!(output.get_ref().is_empty());
        assert_eq!(doc.metadata.xmp, before);
    }
}
#[test]
fn lowered_object_limit_preflights_before_output_and_preserves_blob() {
    use photocraft_format::PreparationLimits;
    let mut doc = Document::new("Own object", Size::new(1, 1), ColorMode::Grayscale, SampleType::F64);
    let blob = Arc::new(vec![91u8; 8193]);
    doc.metadata.exif = Some(blob.clone());
    let mut writer = PcraftWriter::new();
    writer.set_preparation_limits(PreparationLimits { object_bytes: 8192, ..Default::default() }).unwrap();
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        writer.save_zip_stream(&doc, &Default::default(), &mut output, Default::default(), 64 << 20, &Interrupt::NONE),
        Err(FormatError::LimitExceeded(_))
    ));
    assert!(output.get_ref().is_empty());
    assert!(Arc::ptr_eq(doc.metadata.exif.as_ref().unwrap(), &blob));
}

#[test]
fn zero_index_allowance_rejects_owned_objects_before_archive_output() {
    use photocraft_format::PreparationLimits;
    let mut doc = Document::new("Own bounded index", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
    doc.metadata.exif = Some(Arc::new(vec![19u8; 32]));
    let mut writer = PcraftWriter::new();
    writer.set_preparation_limits(PreparationLimits { index_bytes: 0, ..Default::default() }).unwrap();
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        writer.save_zip_stream(&doc, &Default::default(), &mut output, Default::default(), 64 << 20, &Interrupt::NONE),
        Err(FormatError::LimitExceeded(_))
    ));
    assert!(output.get_ref().is_empty());
}

#[test]
fn cancellation_during_chunked_metadata_copy_keeps_exact_utf8() {
    let mut doc = Document::new("Own chunk copy", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.metadata.xmp = Some("Own Ω boundary\n".repeat(300000));
    let original = doc.metadata.xmp.clone();
    let mut layer = Layer::raster("late validation", doc.pixel_format());
    layer.opacity = f32::NAN;
    doc.layers.push(layer);
    let polls = AtomicUsize::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 150;
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        PcraftWriter::new().save_zip_stream(&doc, &Default::default(), &mut output, Default::default(), 64 << 20, &Interrupt::cancel_only(&cancelled)),
        Err(FormatError::Cancelled)
    ));
    assert_eq!(doc.metadata.xmp, original);
    assert!(output.get_ref().is_empty());
    assert!(polls.load(Ordering::Relaxed) >= 151);
}
#[test]
fn streamed_object_cap_and_preparation_failure_preserve_existing_destination() {
    let dir = std::env::temp_dir().join(format!(
        "photocraft-preparation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("original.pcraft");
    std::fs::write(&path, b"original destination").unwrap();
    let mut doc = Document::new("Own cap", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.metadata.exif = Some(Arc::new(vec![1u8; 8193]));
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        PcraftWriter::new().save_zip_stream(&doc, &Default::default(), &mut output, Default::default(), 8192, &Interrupt::NONE),
        Err(FormatError::LimitExceeded(_))
    ));
    assert!(output.get_ref().is_empty());
    assert!(matches!(
        PcraftWriter::new().save_path_stream(&doc, &Default::default(), &path, Default::default(), 8192, &Interrupt::NONE),
        Err(FormatError::LimitExceeded(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"original destination");
    std::fs::remove_dir_all(&dir).unwrap();
}
#[test]
fn shared_observer_holds_preparation_until_output_and_releases_on_errors() {
    use photocraft_format::{PcraftWriter, PreparationLimits, PreparationObserver};
    use std::{
        cell::Cell,
        io::{Cursor, Seek, SeekFrom, Write},
    };
    struct Ledger {
        live: Cell<usize>,
        maximum: usize,
        peak: Cell<usize>,
    }
    impl PreparationObserver for Ledger {
        fn reserve(&self, bytes: usize) -> photocraft_format::Result<()> {
            let next = self.live.get().checked_add(bytes).unwrap();
            if next > self.maximum {
                return Err(photocraft_format::FormatError::LimitExceeded("own combined ledger".into()));
            }
            self.live.set(next);
            self.peak.set(self.peak.get().max(next));
            Ok(())
        }
        fn release(&self, bytes: usize) {
            self.live.set(self.live.get().checked_sub(bytes).unwrap());
        }
    }
    struct Output<'a> {
        ledger: &'a Ledger,
        cursor: Cursor<Vec<u8>>,
        saw_prepared: Cell<bool>,
    }
    impl Write for Output<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.saw_prepared.set(self.ledger.live.get() > 0);
            self.cursor.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Seek for Output<'_> {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(from)
        }
    }
    let doc = photocraft_doc::Document::new("Own observed", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F64);
    let ledger = Ledger { live: Cell::new(0), maximum: usize::MAX, peak: Cell::new(0) };
    let output = Output { ledger: &ledger, cursor: Cursor::new(Vec::new()), saw_prepared: Cell::new(false) };
    let (output, _) = PcraftWriter::save_zip_stream_observed(
        &doc,
        &Default::default(),
        output,
        Default::default(),
        PreparationLimits::default(),
        &photocraft_raster::Interrupt::NONE,
        &ledger,
    )
    .unwrap();
    assert!(output.saw_prepared.get());
    assert!(ledger.peak.get() > 0);
    assert_eq!(ledger.live.get(), 0);
    let denied = Ledger { live: Cell::new(0), maximum: 1, peak: Cell::new(0) };
    assert!(
        PcraftWriter::save_zip_stream_observed(
            &doc,
            &Default::default(),
            Cursor::new(Vec::new()),
            Default::default(),
            Default::default(),
            &photocraft_raster::Interrupt::NONE,
            &denied
        )
        .is_err()
    );
    assert_eq!(denied.live.get(), 0);
}
