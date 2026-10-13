#![cfg(feature = "corpus")]
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianLimits, GaussianParams, try_gaussian_small_auto};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

#[test]
fn independently_authored_finite_terminal_basis() {
    let root = PathBuf::from(std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS to independent full corpus"))
        .join("gaussian-terminal-evidence");
    let observations: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    check_records(&root, observations["records"].as_array().unwrap());
}

#[test]
fn independently_authored_axis_and_group_pairs() {
    let root = PathBuf::from(std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS to independent full corpus"))
        .join("gaussian-axis-evidence");
    let observations: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut records = observations["records"].as_array().unwrap().clone();
    for record in &mut records {
        record["input"] = record["input"]["owner"].clone();
        record["output"] = record["baked"].clone();
    }
    check_records(&root, &records);
}

fn check_records(root: &std::path::Path, records: &[Value]) {
    let mut maximum = 0.0_f64;
    for record in records {
        let original = fs::read(root.join(record["source_file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(original)), record["source_sha256"].as_str().unwrap());
        let width = record["input"]["size"][0].as_u64().unwrap() as i32;
        let height = record["input"]["size"][1].as_u64().unwrap() as i32;
        let area = Rect::new(-2, -3, width - 2, height - 3);
        let load = |key: &str| {
            let descriptor = record[key]["reads"].as_array().unwrap().iter().find(|d| d["requested_format"] == "RGBA double").unwrap();
            let bytes = fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), descriptor["sha256"].as_str().unwrap());
            bytes
        };
        let mut source = Surface::new(PixelFormat::RGBA64F);
        source.try_write_interleaved_with(area, &load("input"), &Interrupt::NONE).unwrap();
        let before = source.clone();
        let output = try_gaussian_small_auto(
            &source,
            area,
            area,
            GaussianParams {
                std_dev_x: record["filter_config"]["std-dev-x"].as_f64().unwrap(),
                std_dev_y: record["filter_config"]["std-dev-y"].as_f64().unwrap(),
                abyss: GaussianAbyss::Clamp,
                clip_extent: true,
            },
            GaussianLimits { resident_bytes: 64 * 1024 * 1024, work_units: 20_000_000 },
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(source, before);
        let expected = load("output");
        let mut worst = 0.0_f64;
        for (index, pixel) in expected.as_chunks::<32>().0.iter().enumerate() {
            let target: [f64; 4] = std::array::from_fn(|c| f64::from_le_bytes(pixel[c * 8..c * 8 + 8].try_into().unwrap()));
            let mut actual = [0.; 4];
            output.try_read_pixel_f64(area.x0 + (index % width as usize) as i32, area.y0 + (index / width as usize) as i32, &mut actual).unwrap();
            for c in 0..4 {
                let error = if c == 3 { (actual[c] - target[c]).abs() } else { (actual[c] * actual[3] - target[c] * target[3]).abs() };
                worst = worst.max(error);
            }
        }
        maximum = maximum.max(worst);
        assert!(worst <= 1.0 / 65535.0, "{} native premultiplied maximum {worst}", record["id"]);
    }
    println!("finite original basis{} maximum {maximum}", records.len());
}
