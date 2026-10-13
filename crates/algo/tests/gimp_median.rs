use photocraft_algo::gimp::neighborhood::{GaussianLimits, MedianNeighborhood, MedianParams, try_median_blur};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use sha2::{Digest, Sha256};
fn params() -> MedianParams {
    MedianParams { radius: 1, neighborhood: MedianNeighborhood::Square, percentile: 50., alpha_percentile: 50., high_precision: true }
}
#[test]
fn exact_weighted_ranks_untouched_bits_and_atomic_failures() {
    let area = Rect::new(0, 0, 3, 1);
    let mut src = Surface::new(PixelFormat::RGBA64F);
    let tiny = f64::from_bits(0x3fe0000000000001);
    src.try_write_pixel_f64(0, 0, &[0.1, 0.2, 2., 1.]).unwrap();
    src.try_write_pixel_f64(1, 0, &[tiny, 0.3, 3., 0.1]).unwrap();
    src.try_write_pixel_f64(2, 0, &[0.9, 0.4, 4., 1.]).unwrap();
    src.try_write_pixel_f64(100, 100, &[-0., tiny, 1., 0.]).unwrap();
    let before = src.clone();
    let out = try_median_blur(&src, area, area, params(), Default::default(), &Interrupt::NONE).unwrap();
    let mut p = [0.; 4];
    out.try_read_pixel_f64(1, 0, &mut p).unwrap();
    assert_eq!(p, [tiny, 0.3, 3., 1.]);
    assert_eq!(out.default_bytes(), src.default_bytes());
    assert_eq!(out.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap(), src.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap());
    assert_eq!(
        try_median_blur(&src, area, area, params(), GaussianLimits { resident_bytes: 1, work_units: u64::MAX }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(try_median_blur(&src, area, area, params(), Default::default(), &Interrupt::new(&|| true, &|_| {})), Err(RasterInputError::Cancelled));
    let mut invalid = params();
    invalid.percentile = f64::NAN;
    assert_eq!(try_median_blur(&src, area, area, invalid, Default::default(), &Interrupt::NONE), Err(RasterInputError::InvalidSample));
    invalid = params();
    invalid.high_precision = false;
    assert_eq!(try_median_blur(&src, area, area, invalid, Default::default(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion));
    assert_eq!(src, before);
}
#[test]
#[ignore = "independent native basis in owned headless scratch"]
fn independent_native_high_precision_basis() {
    for batch in ["data", "extended"] {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol2-xcf-median-probes").join(batch);
        let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut count = 0;
        for rec in data["records"].as_array().unwrap() {
            assert_eq!(
                format!("{:x}", Sha256::digest(std::fs::read(root.join(rec["source_file"].as_str().unwrap())).unwrap())),
                rec["source_sha256"].as_str().unwrap()
            );
            let cfg = &rec["filter_config"];
            if !cfg["high-precision"].as_bool().unwrap() {
                continue;
            }
            let size = rec["input"]["size"].as_array().unwrap();
            let area = Rect::new(0, 0, size[0].as_i64().unwrap() as i32, size[1].as_i64().unwrap() as i32);
            let bytes = std::fs::read(root.join(rec["input"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), rec["input"]["sha256"].as_str().unwrap());
            let src = Surface::try_from_interleaved(PixelFormat::RGBA64F, area, &bytes).unwrap();
            let params = MedianParams {
                radius: cfg["radius"].as_i64().unwrap() as i32,
                neighborhood: match cfg["neighborhood"].as_str().unwrap() {
                    "circle" => MedianNeighborhood::Circle,
                    "square" => MedianNeighborhood::Square,
                    _ => MedianNeighborhood::Diamond,
                },
                percentile: cfg["percentile"].as_f64().unwrap(),
                alpha_percentile: cfg["alpha-percentile"].as_f64().unwrap(),
                high_precision: true,
            };
            let actual = try_median_blur(&src, area, area, params, Default::default(), &Interrupt::NONE).unwrap().try_read_interleaved(area).unwrap();
            let expected = std::fs::read(root.join(rec["output"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&expected)), rec["output"]["sha256"].as_str().unwrap());
            assert_eq!(actual, expected, "{}", rec["id"]);
            count += 1;
        }
        assert_eq!(count, 36);
    }
}

#[test]
fn cancellation_during_work_and_zero_alpha_are_exact() {
    let src = Surface::new(PixelFormat::RGBA64F);
    let area = Rect::new(0, 0, 256, 256);
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 5;
    assert_eq!(try_median_blur(&src, area, area, params(), Default::default(), &Interrupt::new(&cancel, &|_| {})), Err(RasterInputError::Cancelled));
    let mut src = src;
    src.try_write_pixel_f64(0, 0, &[3., 2., 1., 0.]).unwrap();
    let mut p = params();
    p.radius = 0;
    let out = try_median_blur(&src, Rect::new(0, 0, 1, 1), Rect::new(0, 0, 1, 1), p, Default::default(), &Interrupt::NONE).unwrap();
    let mut pixel = [1.; 4];
    out.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
    assert_eq!(pixel, [0.; 4]);
}
#[test]
#[ignore = "independent quantized native basis in owned headless scratch"]
fn independent_quantized_encoded_basis() {
    use photocraft_algo::gimp::neighborhood::try_median_blur_quantized_encoded;
    fn encode(v: f64) -> f64 {
        if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1. / 2.4) - 0.055 }
    }
    fn decode(v: f64) -> f64 {
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    }
    for batch in ["data", "extended"] {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol2-xcf-median-probes").join(batch);
        let records: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut count = 0;
        for rec in records["records"].as_array().unwrap() {
            let cfg = &rec["filter_config"];
            if cfg["high-precision"].as_bool().unwrap() {
                continue;
            }
            let area = Rect::new(0, 0, 9, 11);
            let input = std::fs::read(root.join(rec["input"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&input)), rec["input"]["sha256"].as_str().unwrap());
            let mut src = Surface::new(PixelFormat::RGBA64F);
            for (i, p) in input.as_chunks::<32>().0.iter().enumerate() {
                let mut values = [0.; 4];
                for (c, v) in values.iter_mut().enumerate() {
                    *v = f64::from_le_bytes(p[c * 8..c * 8 + 8].try_into().unwrap());
                    if c < 3 {
                        *v = encode(*v);
                    }
                }
                src.try_write_pixel_f64(i as i32 % 9, i as i32 / 9, &values).unwrap();
            }
            let params = MedianParams {
                radius: cfg["radius"].as_i64().unwrap() as i32,
                neighborhood: match cfg["neighborhood"].as_str().unwrap() {
                    "circle" => MedianNeighborhood::Circle,
                    "square" => MedianNeighborhood::Square,
                    _ => MedianNeighborhood::Diamond,
                },
                percentile: cfg["percentile"].as_f64().unwrap(),
                alpha_percentile: cfg["alpha-percentile"].as_f64().unwrap(),
                high_precision: false,
            };
            let out = try_median_blur_quantized_encoded(&src, area, area, params, Default::default(), &Interrupt::NONE).unwrap();
            let expected = std::fs::read(root.join(rec["output"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&expected)), rec["output"]["sha256"].as_str().unwrap());
            for (i, p) in expected.as_chunks::<32>().0.iter().enumerate() {
                let mut actual = [0.; 4];
                out.try_read_pixel_f64(i as i32 % 9, i as i32 / 9, &mut actual).unwrap();
                for (c, v) in actual.into_iter().enumerate() {
                    let expected = f64::from_le_bytes(p[c * 8..c * 8 + 8].try_into().unwrap());
                    let actual = if c < 3 { decode(v) } else { v };
                    assert!((actual - expected).abs() < 1. / 65535., "{} pixel{i} channel{c} {actual} {expected}", rec["id"]);
                }
            }
            count += 1;
        }
        assert_eq!(count, 36);
    }
}
#[test]
#[ignore = "independent original source/edit native capture in owned scratch"]
fn original_quantized_native_source_and_edit_exact() {
    use photocraft_algo::gimp::neighborhood::try_median_blur_quantized_encoded;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol2-xcf-median-probes/original-pair");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    assert_eq!(data["errors"].as_array().unwrap().len(), 0);
    for rec in data["records"].as_array().unwrap() {
        let raw = std::fs::read(root.join(rec["input"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&raw)), rec["input"]["sha256"].as_str().unwrap());
        assert_eq!(rec["input"]["format"], "R'G'B'A u8");
        let area = Rect::new(0, 0, 32, 32);
        let src = Surface::try_from_interleaved(PixelFormat::RGBA8, area, &raw).unwrap();
        let mut params = params();
        params.radius = rec["filter_config"]["radius"].as_i64().unwrap() as i32;
        params.neighborhood = MedianNeighborhood::Circle;
        params.high_precision = false;
        let out =
            try_median_blur_quantized_encoded(&src, area, area, params, Default::default(), &Interrupt::NONE).unwrap().try_read_interleaved(area).unwrap();
        let expected = std::fs::read(root.join(rec["output"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&expected)), rec["output"]["sha256"].as_str().unwrap());
        assert_eq!(out, expected, "{}", rec["id"]);
    }
}
