//! UNRUN regression drafts for the actual layer-export destination route.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-all-open-export-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn output(&self) -> std::path::PathBuf {
        self.0.join("own_0000_pixel.png")
    }
    fn params(&self) -> Value {
        json!({"dir":self.0.to_str().unwrap(),"prefix":"own","format":"png"})
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn active_document() -> Document {
    let mut doc = Document::new("Own active", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut layer = Layer::raster("pixel", PixelFormat::RGBA8);
    if let LayerContent::Raster(surface) = &mut layer.content {
        surface.try_write_interleaved(Rect::new(0, 0, 1, 1), &[30, 60, 90, 255]).unwrap();
    }
    doc.layers.push(layer);
    doc
}
#[test]
fn inactive_runtime_source_name_is_protected_even_after_content_changed() {
    let directory = Directory::new();
    let output = directory.output();
    let original = b"ordinary bytes now at a formerly imported XCF path";
    std::fs::write(&output, original).unwrap();
    let mut session = Session::new();
    session.add_document(Document::new("Inactive source", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, photocraft_color::SampleType::U8), None).unwrap();
    session.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(), "Misnamed source", Some(output.to_str().unwrap().into()));
    session.add_document(active_document(), None).unwrap();
    let snapshot = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    assert!(layers_to_files(&mut session, &directory.params()).is_err());
    assert_eq!(std::fs::read(output).unwrap(), original);
    assert!(session.active().unwrap().doc.shares_state(&snapshot));
    assert_eq!(session.active().unwrap().revision, revision);
}
#[test]
fn inactive_native_reopen_fingerprint_protects_another_destination_name() {
    let directory = Directory::new();
    let output = directory.output();
    let original = b"own inert protected source content";
    std::fs::write(&output, original).unwrap();
    let mut inactive = Document::new("Own saved native", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut state = photocraft_doc::XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 26;
    state.provenance.source_hash = Some(blake3::hash(original).to_hex().to_string());
    inactive.xcf_state = Some(state);
    let native = photocraft_format::save_to_bytes(&inactive, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    let mut session = Session::new();
    session.add_document(reopened, None).unwrap();
    assert!(session.active().unwrap().source.is_none());
    session.add_document(active_document(), None).unwrap();
    assert!(layers_to_files(&mut session, &directory.params()).is_err());
    assert_eq!(std::fs::read(output).unwrap(), original);
    assert!(!session.active().unwrap().history.can_undo());
}
#[test]
fn inactive_runtime_source_hardlink_is_protected_without_matching_name_or_content() {
    let directory = Directory::new();
    let original_path = directory.0.join("own-source.bin");
    let output = directory.output();
    let original = b"ordinary changed bytes behind the retained runtime identity";
    std::fs::write(&original_path, original).unwrap();
    std::fs::hard_link(&original_path, &output).unwrap();
    let mut session = Session::new();
    session.add_document(Document::new("Inactive source", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, photocraft_color::SampleType::U8), None).unwrap();
    session.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(), "Granted source", Some(original_path.to_str().unwrap().into()));
    session.add_document(active_document(), None).unwrap();
    assert!(layers_to_files(&mut session, &directory.params()).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), original);
    assert_eq!(std::fs::read(&original_path).unwrap(), original);
    assert!(same_file::is_same_file(original_path, output).unwrap());
}
#[test]
fn ordinary_existing_layer_export_destination_keeps_upstream_replace_behavior() {
    let directory = Directory::new();
    let output = directory.output();
    std::fs::write(&output, b"ordinary old destination").unwrap();
    let mut session = Session::new();
    session.add_document(active_document(), None).unwrap();
    let result = layers_to_files(&mut session, &directory.params()).unwrap();
    let files: Vec<std::path::PathBuf> = result["files"].as_array().unwrap().iter().map(|path| std::path::PathBuf::from(path.as_str().unwrap())).collect();
    assert_eq!(files, vec![output.clone()]);
    assert!(std::fs::read(output).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(!session.active().unwrap().history.can_undo());
}
