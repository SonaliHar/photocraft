#![cfg(not(target_arch = "wasm32"))]

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, PhysicalResolution, Size};
use photocraft_engine::Session;
use serde_json::json;
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct OwnedDir(PathBuf);
impl OwnedDir {
    fn new() -> Result<Self, Box<dyn Error>> {
        let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("photocraft-asset-axes-{}-{epoch}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for OwnedDir {
    fn drop(&mut self) {
        for name in ["source.pcraft", "asset.png", "asset@2x.png", "sentinel.png"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn source(spec: &str, legacy: f32, canonical: Option<(f64, f64)>) -> Document {
    let mut doc =
        Document::with_background("Asset physical-size regression", Size::new(24, 16), ColorMode::Rgb, SampleType::U8, Color::rgba(0.25, 0.5, 0.75, 1.0));
    doc.resolution_dpi = legacy;
    doc.physical_resolution = canonical.map(|(x_dpi, y_dpi)| PhysicalResolution { x_dpi, y_dpi, unit: Default::default() });
    doc.layers.first_mut().unwrap().name = spec.into();
    doc
}

fn generate(doc: Document, root: &Path) -> Result<Vec<(u32, u32)>, Box<dyn Error>> {
    let native = photocraft_format::save_to_bytes(&doc, &Default::default())?;
    let path = root.join("source.pcraft");
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?.write_all(&native)?;
    let loaded = photocraft_io::import(&path.to_string_lossy(), &native)?.document;
    let mut session = Session::new();
    session.add_document(loaded, Some(path.to_string_lossy().into_owned()))?;
    session.edit("Own redo seed", |doc, _| {
        doc.name.push_str(" seed");
        Ok(())
    })?;
    assert!(session.undo());
    let before = session.active().ok_or("active")?;
    let before_doc = format!("{:?}", before.doc);
    let before_source = format!("{:?}", before.source);
    let before_native = photocraft_format::save_to_bytes(&before.doc, &Default::default())?;
    let before_state = (before.revision, before.saved_revision, before.history.past_len(), before.history.can_redo());
    let reply = session.execute("file.generate.imageAssets", json!({"on":true,"dir":root.to_string_lossy()}))?;
    let after = session.active().ok_or("active after")?;
    assert_eq!(format!("{:?}", after.doc), before_doc);
    assert_eq!(format!("{:?}", after.source), before_source);
    assert_eq!(photocraft_format::save_to_bytes(&after.doc, &Default::default())?, before_native);
    assert_eq!((after.revision, after.saved_revision, after.history.past_len(), after.history.can_redo()), before_state);
    assert!(reply["errors"].as_array().ok_or("errors")?.is_empty(), "{reply}");
    reply["files"]
        .as_array()
        .ok_or("files")?
        .iter()
        .map(|file| {
            let bytes = std::fs::read(file.as_str().ok_or("file")?)?;
            let image = photocraft_codecs::decode(&bytes)?;
            Ok((image.width(), image.height()))
        })
        .collect()
}

#[test]
fn asset_physical_axes_survive_native_reopen_and_registered_generation() -> Result<(), Box<dyn Error>> {
    let mut mismatches = Vec::new();
    for (name, spec, legacy, canonical, expected) in [
        ("legacy", "1in x 1in asset.png", 72.0, None, (72, 72)),
        ("square", "1in x 1in asset.png", 300.0, Some((144.0, 144.0)), (144, 144)),
        ("unequal", "1in x 1in asset.png", 300.0, Some((144.0, 288.0)), (144, 288)),
        ("metric", "2.54cm x 25.4mm asset.png", 300.0, Some((144.0, 288.0)), (144, 288)),
        ("pixels", "17px x 11px asset.png", 300.0, Some((144.0, 288.0)), (17, 11)),
        ("percent", "200% asset.png", 300.0, Some((144.0, 288.0)), (48, 32)),
        ("width-wildcard", "?x1in asset.png", 300.0, Some((144.0, 288.0)), (432, 288)),
        ("height-wildcard", "1inx? asset.png", 300.0, Some((144.0, 288.0)), (144, 96)),
    ] {
        let dir = OwnedDir::new()?;
        let actual = generate(source(spec, legacy, canonical), &dir.0)?;
        if actual != [expected] {
            mismatches.push((name, actual, expected));
        }
    }
    assert!(mismatches.is_empty(), "physical asset dimensions: {mismatches:?}");
    Ok(())
}

#[test]
fn asset_default_physical_variants_use_both_axes() -> Result<(), Box<dyn Error>> {
    for (default, expected) in [("default 1in x 1in @2x", (144, 288)), ("default 2.54cm x 25.4mm @2x", (144, 288)), ("default 200% @2x", (48, 32))] {
        let dir = OwnedDir::new()?;
        let mut doc = source("asset.png", 300.0, Some((144.0, 288.0)));
        doc.layers.push(Layer::new(default, LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: false, artboard: None })));
        assert_eq!(generate(doc, &dir.0)?, [expected], "{default}");
    }
    Ok(())
}

#[test]
fn asset_valid_subunit_legacy_dpi_is_not_replaced_with_one() -> Result<(), Box<dyn Error>> {
    let dir = OwnedDir::new()?;
    assert_eq!(generate(source("4in x 4in asset.png", 0.5, None), &dir.0)?, [(2, 2)]);
    Ok(())
}

fn refused(doc: Document) -> Result<(), Box<dyn Error>> {
    let dir = OwnedDir::new()?;
    let sentinel = dir.0.join("asset.png");
    std::fs::write(&sentinel, b"own destination sentinel")?;
    let mut session = Session::new();
    session.add_document(doc, None)?;
    session.edit("Own redo seed", |doc, _| {
        doc.name.push_str(" seed");
        Ok(())
    })?;
    assert!(session.undo());
    let before = session.active().ok_or("active")?;
    let before_doc = format!("{:?}", before.doc);
    let before_source = format!("{:?}", before.source);
    let before_native = photocraft_format::save_to_bytes(&before.doc, &Default::default()).map_err(|error| error.to_string());
    let before_state = (before.revision, before.saved_revision, before.history.past_len(), before.history.can_redo());
    let reply = session.execute("file.generate.imageAssets", json!({"on":true,"dir":dir.0.to_string_lossy()}))?;
    assert!(reply["files"].as_array().ok_or("files")?.is_empty(), "{reply}");
    assert!(!reply["errors"].as_array().ok_or("errors")?.is_empty(), "{reply}");
    let after = session.active().ok_or("active after")?;
    assert_eq!(format!("{:?}", after.doc), before_doc);
    assert_eq!(format!("{:?}", after.source), before_source);
    assert_eq!(photocraft_format::save_to_bytes(&after.doc, &Default::default()).map_err(|error| error.to_string()), before_native);
    assert_eq!((after.revision, after.saved_revision, after.history.past_len(), after.history.can_redo()), before_state);
    assert_eq!(std::fs::read(sentinel)?, b"own destination sentinel");
    assert!(!dir.0.join("sentinel.png").exists());
    Ok(())
}

#[test]
fn asset_resolution_and_custom_unit_failures_preserve_destination_and_real_redo() -> Result<(), Box<dyn Error>> {
    for (x, y) in [(0.0, 144.0), (144.0, 0.0), (f64::NAN, 144.0), (144.0, f64::INFINITY), (-1.0, 144.0)] {
        refused(source("1in x 1in asset.png", 300.0, Some((x, y))))?;
    }
    for field in 0..3 {
        let mut doc = source("1in x 1in asset.png", 300.0, Some((144.0, 288.0)));
        let mut unit = photocraft_doc::CustomUnit { name: "own unit".into(), factor: 2.54, digits: 3, symbol: "u".into(), abbreviation: "unit".into() };
        let oversized = "x".repeat((1 << 20) + 1);
        match field {
            0 => unit.name = oversized,
            1 => unit.symbol = oversized,
            _ => unit.abbreviation = oversized,
        }
        doc.physical_resolution.as_mut().unwrap().unit = photocraft_doc::PhysicalUnit::Custom(unit);
        refused(doc)?;
    }
    Ok(())
}

#[test]
fn asset_numeric_overflow_is_rejected_before_any_job_writes() -> Result<(), Box<dyn Error>> {
    refused(source("asset.png, 2in x 1in sentinel.png", 300.0, Some((f64::MAX, 144.0))))?;
    refused(source("asset.png, inf% sentinel.png", 300.0, Some((144.0, 288.0))))?;
    let mut doc = source("asset.png", 300.0, Some((f64::MAX, 144.0)));
    doc.layers.push(Layer::new("default 2in x 1in @2x", LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: false, artboard: None })));
    refused(doc)?;
    Ok(())
}

#[test]
fn asset_tiny_axes_and_custom_unit_preserve_pixel_quantization() -> Result<(), Box<dyn Error>> {
    let dir = OwnedDir::new()?;
    assert_eq!(generate(source("1e-300in x 1e-300in asset.png", 300.0, Some((f64::MIN_POSITIVE, f64::MIN_POSITIVE))), &dir.0)?, [(1, 1)]);
    let dir = OwnedDir::new()?;
    let mut doc = source("1in x 1in asset.png", 300.0, Some((144.25, 287.75)));
    doc.physical_resolution.as_mut().unwrap().unit = photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
        name: "own unit".into(),
        factor: 4.125,
        digits: 6,
        symbol: "u".into(),
        abbreviation: "unit".into(),
    });
    assert_eq!(generate(doc, &dir.0)?, [(144, 288)]);
    Ok(())
}
