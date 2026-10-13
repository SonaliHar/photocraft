#![cfg(not(target_arch = "wasm32"))]

use photocraft_cms::{
    ColorSpace,
    xcf_color::{Encoding, effective_profile},
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, PhysicalResolution, SampleEncoding, Size, WorkingProfileInterpretation};
use photocraft_engine::Session;
use photocraft_raster::Surface;
use serde_json::json;
use std::{
    error::Error,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct OwnedDir(PathBuf);
impl OwnedDir {
    fn new() -> Result<Self, Box<dyn Error>> {
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("photocraft-stack-marker-{}-{time}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for OwnedDir {
    fn drop(&mut self) {
        for name in ["first.pcraft", "second.pcraft"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn run(depth: SampleType, implicit: bool) -> Result<(), Box<dyn Error>> {
    let root = OwnedDir::new()?;
    let mut source = Document::new("Own identical stack source", Size::new(3, 2), ColorMode::Rgb, depth);
    let icc = effective_profile(None, ColorSpace::Rgb, Encoding::NonlinearProfile)?.to_bytes();
    if implicit {
        source.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&icc);
    }
    source.icc_profile = Some(icc);
    source.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    source.set_physical_resolution(PhysicalResolution { x_dpi: 144.25, y_dpi: 287.75, unit: Default::default() })?;
    let mut surface = Surface::new(source.pixel_format());
    surface.write_region(source.bounds(), &[0.25, 0.5, 0.75, 1.0].repeat(6));
    source.layers.push(Layer::new("Own pixels", LayerContent::Raster(surface)));
    let bytes = photocraft_format::save_to_bytes(&source, &Default::default())?;
    let paths = [root.0.join("first.pcraft"), root.0.join("second.pcraft")];
    for path in &paths {
        std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(&bytes)?;
    }
    let mut session = Session::new();
    session.add_document(Document::new("Existing dirty document", Size::new(2, 2), ColorMode::Rgb, SampleType::U8), None)?;
    session.edit("Own redo seed", |doc, _| {
        doc.name.push_str(" changed");
        Ok(())
    })?;
    assert!(session.undo());
    let existing = session.active().ok_or("existing")?;
    let before = photocraft_format::save_to_bytes(&existing.doc, &Default::default())?;
    let before_history = (existing.revision, existing.saved_revision, existing.history.past_len(), existing.history.can_redo());
    let reply = session.execute("file.scripts.loadFilesIntoStack", json!({"paths": paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>() }))?;
    assert_eq!(reply["layers"], 2);
    let stacked = &session.active().ok_or("stack")?.doc;
    assert_eq!(stacked.working_profile_interpretation, source.working_profile_interpretation, "{depth:?}/{implicit}");
    assert_eq!(stacked.icc_profile, source.icc_profile);
    assert_eq!(stacked.sample_encoding, source.sample_encoding);
    assert_eq!(stacked.effective_resolution(), source.effective_resolution());
    let source_pixels = source.layers.first().and_then(Layer::surface).ok_or("source raster")?.read_region(source.bounds());
    for layer in &stacked.layers {
        assert_eq!(layer.surface().ok_or("stack raster")?.read_region(stacked.bounds()), source_pixels);
    }
    let round = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(stacked, &Default::default())?)?;
    assert_eq!(round.working_profile_interpretation, source.working_profile_interpretation);
    assert_eq!(round, **stacked);
    let existing = session.documents().first().ok_or("original")?;
    assert_eq!(photocraft_format::save_to_bytes(&existing.doc, &Default::default())?, before);
    assert_eq!((existing.revision, existing.saved_revision, existing.history.past_len(), existing.history.can_redo()), before_history);
    for path in &paths {
        assert_eq!(std::fs::read(path)?, bytes);
    }
    assert!(session.set_active(0));
    assert!(session.redo());
    assert_eq!(session.active().ok_or("redone original")?.doc.name, "Existing dirty document changed");
    Ok(())
}

#[test]
fn stack_preserves_implicit_u8() -> Result<(), Box<dyn Error>> {
    run(SampleType::U8, true)
}
#[test]
fn stack_preserves_implicit_u16() -> Result<(), Box<dyn Error>> {
    run(SampleType::U16, true)
}
#[test]
fn stack_preserves_implicit_f32() -> Result<(), Box<dyn Error>> {
    run(SampleType::F32, true)
}
#[test]
fn stack_retains_icc_marker_control() -> Result<(), Box<dyn Error>> {
    run(SampleType::F32, false)
}
