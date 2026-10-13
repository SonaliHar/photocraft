#![cfg(not(target_arch = "wasm32"))]

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_io::{ExportOptions, IoError, export_path_with_commit, needs_psb_stream};
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicBool, Ordering};

struct TempRoot(std::path::PathBuf);
impl TempRoot {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("photocraft-save-{}-{nonce}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn document() -> Document {
    let mut doc = Document::new("save", Size::new(32, 16), ColorMode::Rgb, SampleType::U16);
    let mut layer = Layer::raster("pixels", doc.pixel_format());
    layer.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.25, 0.5, 0.75, 1.0]);
    doc.layers.push(layer);
    doc
}

#[test]
fn all_native_formats_share_the_final_commit_boundary() {
    let root = TempRoot::new();
    let doc = document();
    for ext in ["psb", "png"] {
        let target = root.path().join(format!("document.{ext}"));
        std::fs::write(&target, b"precious original").unwrap();
        let result = export_path_with_commit(&doc, &target, &ExportOptions::default(), &Interrupt::NONE, || {
            assert_eq!(std::fs::read(&target).unwrap(), b"precious original");
            Err(IoError::Cancelled)
        });
        assert!(matches!(result, Err(IoError::Cancelled)));
        assert_eq!(std::fs::read(&target).unwrap(), b"precious original");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), if ext == "psb" { 1 } else { 2 });
    }
}

#[test]
fn cancellation_after_commit_does_not_report_a_published_save_as_failed() {
    let root = TempRoot::new();
    let doc = document();
    for ext in ["psb", "png"] {
        let target = root.path().join(format!("document.{ext}"));
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::Relaxed);
        let interrupt = Interrupt::cancel_only(&check);
        export_path_with_commit(&doc, &target, &ExportOptions::default(), &interrupt, || {
            cancelled.store(true, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        let reopened = photocraft_io::import_path_with("saved", &target, &Interrupt::NONE).unwrap();
        assert_eq!(reopened.document.size, doc.size);
    }
}

#[test]
fn automatic_psd_size_promotion_uses_the_streaming_writer() {
    let opts = ExportOptions::default();
    let small = document();
    assert!(!needs_psb_stream(&small, "small.PSD", &opts));
    assert!(needs_psb_stream(&small, "small.PSB", &opts));
    let sparse = Document::new("large", Size::new(30_000, 30_000), ColorMode::Rgb, SampleType::U8);
    assert!(needs_psb_stream(&sparse, "large.psd", &opts));
    assert!(!needs_psb_stream(&sparse, "large.png", &opts));
}

#[test]
fn streaming_save_respects_compatibility_and_thumbnail_options() {
    let doc = document();
    for compatibility in [false, true] {
        for previews in [false, true] {
            let options = ExportOptions { psd_maximize_compatibility: compatibility, image_previews: previews, ..Default::default() };
            let reference = photocraft_io::export(&doc, "reference.psb", &options).unwrap();
            let reference = photocraft_psd::PsdFile::from_bytes(&reference.bytes).unwrap();
            let mut output = std::io::Cursor::new(Vec::new());
            photocraft_io::export_psb_into_with_options(&doc, &mut output, &options, &Interrupt::NONE).unwrap();
            let output = photocraft_psd::PsdFile::from_bytes(output.get_ref()).unwrap();
            assert_eq!(output.resources, reference.resources, "compatibility={compatibility}, previews={previews}");
            let expected = reference.image_data.decode(&reference.header).unwrap();
            let actual = output.image_data.decode(&output.header).unwrap();
            // Streaming keeps a merged alpha plane for compatibility; the colour planes agree.
            assert_eq!(&actual[..3], &expected[..3], "compatibility={compatibility}");
            assert_eq!(output.layers().len(), reference.layers().len());
        }
    }
}
