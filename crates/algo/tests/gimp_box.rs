use photocraft_algo::gimp::neighborhood::{GaussianLimits, box_blur_extent, try_box_blur};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
#[test]
fn checked_box_mean_matches_direct_f64_clamp_and_keeps_untouched_bits() {
    let area = Rect::new(-2, -1, 3, 2);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            source.try_write_pixel_f64(x, y, &[f64::from(x) * 0.125, f64::from(y) * 0.25, 1.25, 0.5]).unwrap();
        }
    }
    source.try_write_pixel_f64(200, 200, &[-0., f64::from_bits(0x3fe0000000000001), 2., 0.]).unwrap();
    let before = source.clone();
    let output = box_blur_extent(area, 2).unwrap();
    let actual = try_box_blur(&source, output, area, 2, Default::default(), &Interrupt::NONE).unwrap();
    for y in output.y0..output.y1 {
        for x in output.x0..output.x1 {
            let mut expected = [0.; 4];
            for yy in y - 2..=y + 2 {
                for xx in x - 2..=x + 2 {
                    let mut p = [0.; 4];
                    source.try_read_pixel_f64(xx.clamp(area.x0, area.x1 - 1), yy.clamp(area.y0, area.y1 - 1), &mut p).unwrap();
                    for (c, v) in expected.iter_mut().enumerate() {
                        *v += if c == 3 { p[c] } else { p[c] * p[3] };
                    }
                }
            }
            for v in &mut expected {
                *v /= 25.;
            }
            for v in expected.iter_mut().take(3) {
                *v /= 0.5;
            }
            let mut got = [0.; 4];
            actual.try_read_pixel_f64(x, y, &mut got).unwrap();
            for (a, b) in got.into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-12);
            }
        }
    }
    assert_eq!(source, before);
    assert_eq!(actual.default_bytes(), source.default_bytes());
    let marker = Rect::new(200, 200, 201, 201);
    assert_eq!(actual.try_read_interleaved(marker).unwrap(), source.try_read_interleaved(marker).unwrap());
    assert_eq!(try_box_blur(&source, area, area, 0, Default::default(), &Interrupt::NONE).unwrap(), source);
}
#[test]
fn checked_box_limits_cancel_and_hostile_coordinates_are_atomic() {
    let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
    let area = Rect::new(0, 0, 5, 7);
    let before = source.clone();
    assert_eq!(try_box_blur(&source, area, area, 1001, Default::default(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion));
    assert_eq!(
        try_box_blur(&source, area, area, 4, GaussianLimits { resident_bytes: 1, work_units: u64::MAX }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(try_box_blur(&source, area, area, 4, Default::default(), &Interrupt::new(&|| true, &|_| {})), Err(RasterInputError::Cancelled));
    assert_eq!(box_blur_extent(Rect::new(i32::MIN, 0, 0, 1), 4), Err(RasterInputError::InvalidBounds));
    assert_eq!(source, before);
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || {
        let n = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        n >= 5
    };
    let large = Rect::new(0, 0, 256, 256);
    assert_eq!(try_box_blur(&source, large, large, 4, Default::default(), &Interrupt::new(&cancel, &|_| {})), Err(RasterInputError::Cancelled));
    assert!(polls.load(std::sync::atomic::Ordering::Relaxed) >= 5);
    assert_eq!(source, before);
}
#[test]
#[ignore = "independent official runtime native Box basis; set PHOTOCRAFT_BOX_BASIS"]
fn independently_authored_stock_box_raw_native_basis() {
    check_native_basis(std::path::PathBuf::from(std::env::var("PHOTOCRAFT_BOX_BASIS").unwrap()), 1. / 65535.);
}
#[test]
#[ignore = "independent maximum-radius native Box basis; set PHOTOCRAFT_BOX_BASIS to data"]
fn independently_authored_large_radius_box_raw_native_basis() {
    let data = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_BOX_BASIS").unwrap());
    check_native_basis(data.parent().unwrap().join("large"), 2. / 255.);
}
fn check_native_basis(root: std::path::PathBuf, bound: f64) {
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::fs;
    let records: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    assert!(records["errors"].as_array().unwrap().is_empty());
    let mut maximum = 0f64;
    for record in records["records"].as_array().unwrap() {
        let load = |key: &str| {
            let d = &record[key];
            let bytes = fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), d["sha256"].as_str().unwrap());
            bytes
        };
        let raw = fs::read(root.join(record["source_file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(raw)), record["source_sha256"].as_str().unwrap());
        let width = record["input"]["size"][0].as_u64().unwrap() as i32;
        let height = record["input"]["size"][1].as_u64().unwrap() as i32;
        let input = Rect::new(0, 0, width, height);
        let source = Surface::try_from_interleaved(PixelFormat::RGBA64F, input, &load("input")).unwrap();
        let radius = record["filter_config"]["radius"].as_u64().unwrap() as u32;
        let full_output = box_blur_extent(input, radius).unwrap();
        let output = if let Some(roi) = record["output"]["global_roi"].as_array() {
            Rect::new(roi[0].as_i64().unwrap() as i32, roi[1].as_i64().unwrap() as i32, roi[2].as_i64().unwrap() as i32, roi[3].as_i64().unwrap() as i32)
        } else {
            full_output
        };
        assert_eq!(output.intersect(&full_output), output);
        assert_eq!(record["output"]["size"][0].as_u64().unwrap(), output.width() as u64);
        assert_eq!(record["output"]["offset"][0].as_i64().unwrap(), i64::from(output.x0));
        let actual = try_box_blur(&source, output, input, radius, Default::default(), &Interrupt::NONE).unwrap();
        let expected = load("output");
        let mut worst = 0f64;
        for (i, pixel) in expected.as_chunks::<32>().0.iter().enumerate() {
            let target: [f64; 4] = std::array::from_fn(|c| f64::from_le_bytes(pixel[c * 8..c * 8 + 8].try_into().unwrap()));
            let mut p = [0.; 4];
            actual.try_read_pixel_f64(output.x0 + (i % output.width() as usize) as i32, output.y0 + (i / output.width() as usize) as i32, &mut p).unwrap();
            for c in 0..4 {
                let e = if c == 3 { (p[c] - target[c]).abs() } else { (p[c] * p[3] - target[c] * target[3]).abs() };
                worst = worst.max(e);
            }
        }
        maximum = maximum.max(worst);
        assert!(worst <= bound, "{} {worst}", record["id"]);
    }
    println!("Box native {} originals maximum {maximum}", records["records"].as_array().unwrap().len());
}
