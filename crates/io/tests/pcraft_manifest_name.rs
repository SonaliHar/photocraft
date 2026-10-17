//! A `.pcraft` bundle's manifest reports the file it is in (#3312): writing a document to a new
//! name must not leave the previous one inside, on every path that produces `.pcraft` bytes.

mod common;

use common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_format::zip::stream::{Limits, Reader};
use photocraft_io::*;
use photocraft_raster::Interrupt;
use std::io::Cursor;

fn doc() -> photocraft_doc::Document {
    let mut d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    d.name = "FirstTest.psd".into();
    d
}

/// `document.name` as the bundle actually stores it.
fn manifest_name(bytes: &[u8]) -> String {
    let mut archive = Reader::new(Cursor::new(bytes), Limits::default(), &Interrupt::NONE).expect("ZIP64 stream");
    let manifest: serde_json::Value =
        serde_json::from_slice(&archive.read_by_name("manifest.json", 1 << 20, &Interrupt::NONE).expect("manifest")).expect("manifest json");
    manifest["document"]["name"].as_str().expect("document.name").to_string()
}

/// Save As to a new name: the written file names itself, so reopening it agrees with the session
/// that wrote it instead of reporting the file it came from.
#[test]
fn export_names_the_manifest_after_the_file() {
    let r = export(&doc(), "/tmp/somewhere/_nametest.pcraft", &ExportOptions::default()).expect("export");
    assert_eq!(manifest_name(&r.bytes), "_nametest.pcraft");
    assert_eq!(photocraft_format::load_from_bytes(&r.bytes).expect("load").name, "_nametest.pcraft");
}

/// A bare extension names no file and leaves the document's own name alone.
#[test]
fn a_bare_extension_names_no_file() {
    let r = export(&doc(), "pcraft", &ExportOptions::default()).expect("export");
    assert_eq!(manifest_name(&r.bytes), "FirstTest.psd");
}
