#![cfg(feature = "corpus")]
use photocraft_algo::transform::{Homography, try_warp_nearest_surface_with_limits};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use sha2::{Digest, Sha256};
use std::path::Path;
fn bytes(root: &Path, e: &serde_json::Value) -> Vec<u8> {
    let b = std::fs::read(root.join(e["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&b)), e["sha256"].as_str().unwrap());
    b
}
fn bounds(s: &serde_json::Value) -> Rect {
    let x = s["offset"][0].as_i64().unwrap() as i32;
    let y = s["offset"][1].as_i64().unwrap() as i32;
    Rect::new(x, y, x + s["size"][0].as_i64().unwrap() as i32, y + s["size"][1].as_i64().unwrap() as i32)
}
#[test]
#[ignore = "requires reviewed unpublished legal-nearest corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn nearest_geometry_matches_original_gimp_native_wide_source_edit_bytes() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS");
    let root = Path::new(&base).join("legal-nearest-geometry-evidence");
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    assert!(proof["errors"].as_array().unwrap().is_empty());
    for c in proof["records"].as_array().unwrap() {
        let id = c["id"].as_str().unwrap();
        for key in ["source", "edit"] {
            let _ = bytes(&root, &c[key]);
        }
        let fmt = PixelFormat::new(
            if id.contains("-rgb-") { ColorMode::Rgb } else { ColorMode::Grayscale },
            if id.starts_with("u8_") {
                SampleType::U8
            } else if id.starts_with("u16_") {
                SampleType::U16
            } else {
                SampleType::F32
            },
            true,
        );
        let source = Surface::try_from_interleaved(fmt, bounds(&c["fresh_source"]), &bytes(&root, &c["fresh_source"]["native"])).unwrap();
        let before = source.clone();
        let m: [f64; 9] = c["matrix"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect::<Vec<_>>().try_into().unwrap();
        let out =
            try_warp_nearest_surface_with_limits(&source, bounds(&c["fresh_source"]), &Homography(m), Default::default(), &photocraft_raster::Interrupt::NONE)
                .unwrap();
        assert_eq!(out.try_read_interleaved(bounds(&c["fresh_edit"])).unwrap(), bytes(&root, &c["fresh_edit"]["native"]), "{id}");
        assert_eq!(source, before);
        println!("nearest native bitexact {id}");
    }
}
