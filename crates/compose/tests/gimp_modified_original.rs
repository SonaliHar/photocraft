#![cfg(feature = "corpus")]
//! Live-before-save vector pixels differ from reopened pixels; no serialized-cache claim.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{ColorMode, Document, ImportCompositing, Layer, Rect, SampleEncoding, SampleType, Size};
use photocraft_raster::Surface;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn vector_live_before_save_and_reopened_native_pixels_are_distinct() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("gimp_modified_hashes.json")).unwrap();
    let read = |name: &str| {
        let bytes = fs::read(root.join(name)).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash, hashes[name].as_str().unwrap());
        bytes
    };
    read("vector-modified.xcf");
    let reference = photocraft_codecs::decode(&read("vector-modified.ref.png")).unwrap();
    let mut results = Vec::new();
    for phase in ["source", "reopened", "serialized-offset-control"] {
        let state_phase = if phase == "serialized-offset-control" { "reopened" } else { phase };
        let state: Value = serde_json::from_slice(&read(&format!("vector-modified.{state_phase}_state.json"))).unwrap();
        let mut doc = Document::new(phase, Size::new(96, 80), ColorMode::Rgb, SampleType::U8);
        doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        for item in state["layers"].as_array().unwrap().iter().rev() {
            let mut bounds = Rect::from_xywh(
                item["offset"][0].as_i64().unwrap() as i32,
                item["offset"][1].as_i64().unwrap() as i32,
                item["size"][0].as_u64().unwrap() as u32,
                item["size"][1].as_u64().unwrap() as u32,
            );
            // Actual saved XCF declares (9,11), while GIMP's reopened drawable
            // reports (10,12). Keep identical reopened bytes to isolate placement.
            if phase == "serialized-offset-control" && bounds == Rect::from_xywh(10, 12, 73, 54) {
                bounds = Rect::from_xywh(9, 11, 73, 54);
            }
            let mut layer = Layer::raster(item["name"].as_str().unwrap(), doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), bounds, &read(item["native"]["file"].as_str().unwrap())).unwrap();
            layer.import_compositing =
                Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
            doc.layers.push(layer);
        }
        let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let mut worst = 0.0_f64;
        for (index, pixel) in actual.px.iter().enumerate() {
            for (channel, value) in pixel.iter().enumerate() {
                worst = worst.max((value - f64::from(reference.get((index % 96) as u32, (index / 96) as u32, channel))).abs());
            }
        }
        println!("modified vector {phase}: {}/255", worst * 255.0);
        results.push((actual, worst));
    }
    assert!(results[0].1 > 200.0 / 255.0);
    assert!(results[1].1 <= 2.0 / 255.0);
    assert!(results[2].1 > 200.0 / 255.0);
    // Live before-save green pixel differs from the serialized/reopened orange vector.
    assert_eq!(results[0].0.px[14 * 96 + 12], [0.0, 1.0, 0.0, 1.0]);
    assert!(results[1].0.px[14 * 96 + 12][0] > 0.8);
}
