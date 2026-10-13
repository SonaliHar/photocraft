//! Completion callbacks are part of the controlled handoff, not notifications
//! after a successful result has already been published.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size};
use photocraft_geom::Rect;
use photocraft_io::{IoError, export_with, import_with};
use photocraft_raster::{Interrupt, Surface};
use std::sync::atomic::{AtomicBool, Ordering};

fn source() -> Document {
    let mut doc = Document::new("Completion fixture", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::new(doc.pixel_format());
    pixels.try_write_interleaved(Rect::new(0, 0, 2, 2), &[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 20, 40, 60, 0]).unwrap();
    doc.layers.push(Layer::new("Immutable original", LayerContent::Raster(pixels)));
    doc
}
fn native(doc: &Document) -> Vec<u8> {
    photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()
}

#[test]
fn png_export_completion_cancellation_never_returns_encoded_result() {
    let doc = source();
    let before = native(&doc);
    let completed = AtomicBool::new(false);
    let never = || false;
    let report = |p: f32| {
        if p >= 1.0 {
            completed.store(true, Ordering::Relaxed);
        }
    };
    let ordinary = export_with(&doc, "own.png", &Default::default(), &Interrupt::new(&never, &report)).unwrap();
    assert!(completed.load(Ordering::Relaxed));
    assert!(ordinary.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(native(&doc), before);
    let imported = import_with("own.png", &ordinary.bytes, &Interrupt::NONE).unwrap();
    assert_eq!(imported.document.size, doc.size);

    let cancelled = AtomicBool::new(false);
    let completion = AtomicBool::new(false);
    let check = || cancelled.load(Ordering::Relaxed);
    let report = |p: f32| {
        if p >= 1.0 {
            completion.store(true, Ordering::Relaxed);
            cancelled.store(true, Ordering::Relaxed);
        }
    };
    let result = export_with(&doc, "own.png", &Default::default(), &Interrupt::new(&check, &report));
    assert!(completion.load(Ordering::Relaxed));
    assert!(matches!(result, Err(IoError::Cancelled)));
    assert_eq!(native(&doc), before, "cancelled export changed source model/native data");
}

#[test]
fn png_import_completion_cancellation_never_returns_document() {
    let doc = source();
    let before = native(&doc);
    let bytes = export_with(&doc, "own.png", &Default::default(), &Interrupt::NONE).unwrap().bytes;
    let original_bytes = bytes.clone();
    let completed = AtomicBool::new(false);
    let never = || false;
    let report = |p: f32| {
        if p >= 1.0 {
            completed.store(true, Ordering::Relaxed);
        }
    };
    let ordinary = import_with("own.png", &bytes, &Interrupt::new(&never, &report)).unwrap();
    assert_eq!(ordinary.document.size, doc.size);
    assert!(completed.load(Ordering::Relaxed));
    let imported_native = native(&ordinary.document);
    assert_eq!(photocraft_format::load_from_bytes(&imported_native).unwrap(), ordinary.document);

    let cancelled = AtomicBool::new(false);
    let completion = AtomicBool::new(false);
    let check = || cancelled.load(Ordering::Relaxed);
    let report = |p: f32| {
        if p >= 1.0 {
            completion.store(true, Ordering::Relaxed);
            cancelled.store(true, Ordering::Relaxed);
        }
    };
    let result = import_with("own.png", &bytes, &Interrupt::new(&check, &report));
    assert!(completion.load(Ordering::Relaxed));
    assert!(matches!(result, Err(IoError::Cancelled)));
    assert_eq!(bytes, original_bytes, "cancelled import changed immutable encoded input");
    assert_eq!(native(&doc), before);
    assert_eq!(native(&ordinary.document), imported_native, "cancelled import changed previously accepted model");
}
