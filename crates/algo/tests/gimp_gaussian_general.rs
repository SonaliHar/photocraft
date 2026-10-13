use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianParams, try_gaussian_finite_clamp};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
#[test]
fn general_finite_is_atomic_bounded_and_preserves_default() {
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[1.25, -0.75, 0.5, 0.5]).unwrap();
    let before = source.clone();
    let area = Rect::new(0, 0, 2, 1);
    let params = GaussianParams { std_dev_x: 2.25, std_dev_y: 0.55, abyss: GaussianAbyss::Clamp, clip_extent: true };
    for filter in [GaussianFilter::Auto, GaussianFilter::Fir, GaussianFilter::Iir] {
        let out = try_gaussian_finite_clamp(&source, area, area, params, filter, Default::default(), &Interrupt::NONE).unwrap();
        assert_eq!(out.default_bytes(), source.default_bytes());
        assert_eq!(source, before);
        assert_eq!(
            try_gaussian_finite_clamp(&source, area, area, params, filter, Default::default(), &Interrupt::new(&|| true, &|_| {})),
            Err(RasterInputError::Cancelled)
        );
    }
    assert_eq!(
        try_gaussian_finite_clamp(&source, area, area, GaussianParams { std_dev_x: 64., ..params }, GaussianFilter::Auto, Default::default(), &Interrupt::NONE),
        Err(RasterInputError::UnsupportedConversion)
    );
}

#[test]
fn sparse_output_tile_growth_is_reserved_before_initialization() {
    use photocraft_algo::gimp::neighborhood::GaussianLimits;
    use photocraft_color::{ColorMode, SampleType};
    let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
    let extent = Rect::new(0, 0, 1024, 1024);
    let params = GaussianParams { std_dev_x: 2.25, std_dev_y: 3.75, abyss: GaussianAbyss::Clamp, clip_extent: true };
    assert_eq!(
        try_gaussian_finite_clamp(
            &source,
            extent,
            extent,
            params,
            GaussianFilter::Auto,
            GaussianLimits { resident_bytes: 28 * 1024 * 1024, work_units: u64::MAX },
            &Interrupt::NONE
        ),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(source.tile_count(), 0);
}
#[test]
#[ignore = "independent official runtime raw-native basis in owned scratch; set PHOTOCRAFT_GAUSSIAN_BASIS"]
fn official_arbitrary_parameters_and_wrong_algorithm_controls() {
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(std::env::var("PHOTOCRAFT_GAUSSIAN_BASIS").unwrap());
    let mut count = 0;
    let mut maximum = 0f64;
    let mut wrong_max = 0f64;
    for folder in ["data", "extended", "anisotropic"] {
        let dir = root.join(folder);
        let records: Value = serde_json::from_slice(&fs::read(dir.join("observations.json")).unwrap()).unwrap();
        assert!(records["errors"].as_array().unwrap().is_empty());
        for record in records["records"].as_array().unwrap() {
            let sigma = record["filter_config"]["std-dev-y"].as_f64().unwrap();
            let sx = record["filter_config"]["std-dev-x"].as_f64().unwrap();
            if sigma > 15. || record["filter_config"]["filter"] == "iir" && [sigma, sx].iter().any(|s| *s > 0. && *s < 0.5) {
                continue;
            }
            let width = record["input"]["size"][0].as_u64().unwrap() as i32;
            let height = record["input"]["size"][1].as_u64().unwrap() as i32;
            let area = Rect::new(-2, -3, width - 2, height - 3);
            let load = |key: &str| {
                let d = &record[key];
                let bytes = fs::read(dir.join(d["file"].as_str().unwrap())).unwrap();
                assert_eq!(format!("{:x}", Sha256::digest(&bytes)), d["sha256"].as_str().unwrap());
                bytes
            };
            let original = fs::read(dir.join(record["source_file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(original)), record["source_sha256"].as_str().unwrap());
            let source = Surface::try_from_interleaved(PixelFormat::RGBA64F, area, &load("input")).unwrap();
            let expected = load("output");
            let filter = match record["filter_config"]["filter"].as_str().unwrap() {
                "auto" => GaussianFilter::Auto,
                "fir" => GaussianFilter::Fir,
                "iir" => GaussianFilter::Iir,
                _ => panic!("unknown algorithm"),
            };
            let params = GaussianParams { std_dev_x: sx, std_dev_y: sigma, abyss: GaussianAbyss::Clamp, clip_extent: true };
            let result = try_gaussian_finite_clamp(&source, area, area, params, filter, Default::default(), &Interrupt::NONE).unwrap();
            for (i, pixel) in expected.as_chunks::<32>().0.iter().enumerate() {
                let target: [f64; 4] = std::array::from_fn(|c| f64::from_le_bytes(pixel[c * 8..c * 8 + 8].try_into().unwrap()));
                let mut actual = [0.; 4];
                result.try_read_pixel_f64(area.x0 + (i % width as usize) as i32, area.y0 + (i / width as usize) as i32, &mut actual).unwrap();
                for c in 0..4 {
                    let error = if c == 3 { (actual[c] - target[c]).abs() } else { (actual[c] * actual[3] - target[c] * target[3]).abs() };
                    maximum = maximum.max(error);
                    assert!(error <= 2. / 255., "{} {error}", record["id"]);
                }
            }
            if sigma == f64::from(0.8_f32) && height == 33 && filter == GaussianFilter::Fir {
                let wrong = try_gaussian_finite_clamp(&source, area, area, params, GaussianFilter::Iir, Default::default(), &Interrupt::NONE).unwrap();
                for y in area.y0..area.y1 {
                    let mut a = [0.; 4];
                    let mut b = [0.; 4];
                    wrong.try_read_pixel_f64(area.x0, y, &mut a).unwrap();
                    result.try_read_pixel_f64(area.x0, y, &mut b).unwrap();
                    wrong_max = wrong_max.max((a[3] - b[3]).abs());
                }
            }
            count += 1;
        }
    }
    assert!(wrong_max > 0.07);
    println!("native general basis {count} cases maximum {maximum}; wrong IIR control {wrong_max}");
}
