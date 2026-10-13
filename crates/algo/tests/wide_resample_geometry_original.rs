#![cfg(feature = "corpus")]
use photocraft_algo::transform::{Homography, Interp, try_warp_cubic_surface_with_limits, try_warp_surface_with_limits};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use sha2::{Digest, Sha256};
use std::path::Path;
fn bytes(root: &Path, entry: &serde_json::Value) -> Vec<u8> {
    let data = std::fs::read(root.join(entry["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&data)), entry["sha256"].as_str().unwrap());
    data
}
#[test]
fn explicit_linear_linear_matches_independent_gimp_wide_native_source_edits() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS to local independent originals");
    let root = Path::new(&base).join("wide-resample-geometry-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let fresh_root = root.join("fresh-process");
    let fresh: serde_json::Value = serde_json::from_slice(&std::fs::read(fresh_root.join("observations.json")).unwrap()).unwrap();
    let mut worst = 0_f64;
    for case in observations["records"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let rows = fresh["records"].as_array().unwrap();
        let before = &rows.iter().find(|r| r["id"] == id && r["phase"] == "source").unwrap()["state"];
        let after = &rows.iter().find(|r| r["id"] == id && r["phase"] == "edit").unwrap()["state"];
        assert!(before["profiles"]["assigned"].is_null());
        assert!(after["profiles"]["assigned"].is_null());
        for state in [before, after] {
            assert!(!bytes(&fresh_root, &state["profiles"]["effective"]).is_empty());
        }
        for (phase, state) in [("source", before), ("edit", after)] {
            let original = bytes(&root, &case[phase]);
            let row = rows.iter().find(|r| r["id"] == id && r["phase"] == phase).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&original)), row["source_sha256"].as_str().unwrap());
            assert_eq!(bytes(&fresh_root, &state["native"]), bytes(&root, &case[if phase == "source" { "fresh_source" } else { "fresh_edit" }]["native"]));
        }
        let mode = if id.contains("-rgb-") { ColorMode::Rgb } else { ColorMode::Grayscale };
        let format = PixelFormat::new(mode, SampleType::F64, true);
        let offset = before["offset"].as_array().unwrap();
        let size = before["size"].as_array().unwrap();
        let x = offset[0].as_i64().unwrap() as i32;
        let y = offset[1].as_i64().unwrap() as i32;
        let bounds = Rect::new(x, y, x + size[0].as_i64().unwrap() as i32, y + size[1].as_i64().unwrap() as i32);
        let source = Surface::try_from_interleaved(format, bounds, &bytes(&fresh_root, &before["linear_double"])).unwrap();
        let original = source.clone();
        let matrix: [f64; 9] = case["matrix"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>().try_into().unwrap();
        let output = if id.starts_with("linear-") {
            try_warp_surface_with_limits(&source, bounds, &Homography(matrix), Interp::Bilinear, Default::default(), &Interrupt::NONE)
        } else {
            try_warp_cubic_surface_with_limits(&source, bounds, &Homography(matrix), Default::default(), &Interrupt::NONE)
        }
        .unwrap();
        // Integer target storage clips reconstruction overshoot; F64 keeps it.
        // This comparison exercises that explicit target policy in linear working samples.
        let output = if id.contains("u32_") { output.try_convert(PixelFormat::new(mode, SampleType::U32, true)).unwrap() } else { output };
        let expected = bytes(&fresh_root, &after["linear_double"]);
        let ox = after["offset"][0].as_i64().unwrap() as i32;
        let oy = after["offset"][1].as_i64().unwrap() as i32;
        let width = after["size"][0].as_i64().unwrap() as usize;
        let height = after["size"][1].as_i64().unwrap() as usize;
        let mut index = 0;
        let mut pixel = [0.; 4];
        let mut maximum = 0_f64;
        for y in 0..height {
            for x in 0..width {
                output.try_read_pixel_f64(ox + x as i32, oy + y as i32, &mut pixel[..format.channels()]).unwrap();
                for value in &pixel[..format.channels()] {
                    let expected = f64::from_le_bytes(expected[index..index + 8].try_into().unwrap());
                    index += 8;
                    maximum = maximum.max((value - expected).abs() * 255.);
                }
            }
        }
        assert_eq!(index, expected.len());
        assert!(maximum.is_finite());
        assert_eq!(source, original);
        worst = worst.max(maximum);
        println!("{id}: {maximum}/255");
    }
    println!("independent resample geometry maximum {worst}/255");
    assert!(worst <= 2., "unqualified general resample geometry {worst}/255");
}
