use photocraft_algo::gimp::neighborhood::{GaussianLimits, MotionBlurParams, motion_blur_extent, try_motion_blur_linear};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use sha2::{Digest, Sha256};
#[test]
fn centered_line_preserves_current_wide_values_and_atomic_caps() {
    let area = Rect::new(0, 0, 3, 1);
    let mut src = Surface::new(PixelFormat::RGBA64F);
    let tiny = f64::from_bits(0x3fe0000000000001);
    for x in 0..3 {
        src.try_write_pixel_f64(x, 0, &[tiny, 1., 2., 1.]).unwrap();
    }
    src.try_write_pixel_f64(100, 100, &[-0., tiny, 3., 0.]).unwrap();
    let before = src.clone();
    let params = MotionBlurParams { length: 2., angle_degrees: 0. };
    let out = try_motion_blur_linear(&src, area, area, params, Default::default(), &Interrupt::NONE).unwrap();
    let mut p = [0.; 4];
    out.try_read_pixel_f64(1, 0, &mut p).unwrap();
    assert!((p[0] - tiny).abs() < f64::EPSILON);
    assert_eq!(out.default_bytes(), src.default_bytes());
    assert_eq!(out.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap(), src.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap());
    assert_eq!(
        try_motion_blur_linear(&src, area, area, params, GaussianLimits { resident_bytes: 1, work_units: u64::MAX }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(try_motion_blur_linear(&src, area, area, params, Default::default(), &Interrupt::new(&|| true, &|_| {})), Err(RasterInputError::Cancelled));
    assert_eq!(motion_blur_extent(area, MotionBlurParams { length: f64::NAN, angle_degrees: 0. }), Err(RasterInputError::InvalidSample));
    assert_eq!(src, before);
}
#[test]
#[ignore = "independent raw-native motion basis in owned scratch"]
fn independent_native_motion_basis() {
    for batch in ["data", "holdouts"] {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol2-xcf-motion-probes").join(batch);
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        assert_eq!(proof["errors"].as_array().unwrap().len(), 0);
        let mut worst = 0f64;
        for rec in proof["records"].as_array().unwrap() {
            let input = std::fs::read(root.join(rec["input"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&input)), rec["input"]["sha256"].as_str().unwrap());
            let input_area = Rect::new(0, 0, rec["input"]["size"][0].as_i64().unwrap() as i32, rec["input"]["size"][1].as_i64().unwrap() as i32);
            let source = Surface::try_from_interleaved(PixelFormat::RGBA64F, input_area, &input).unwrap();
            let cfg = &rec["filter_config"];
            let params = MotionBlurParams { length: cfg["length"].as_f64().unwrap(), angle_degrees: cfg["angle"].as_f64().unwrap() };
            let area = motion_blur_extent(input_area, params).unwrap();
            assert_eq!(area.x0, rec["output"]["offset"][0].as_i64().unwrap() as i32);
            assert_eq!(area.y0, rec["output"]["offset"][1].as_i64().unwrap() as i32);
            let out =
                try_motion_blur_linear(&source, area, input_area, params, Default::default(), &Interrupt::NONE).unwrap().try_read_region_f64(area).unwrap();
            let expected = std::fs::read(root.join(rec["output"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&expected)), rec["output"]["sha256"].as_str().unwrap());
            assert_eq!(out.len() * 8, expected.len());
            for (actual, expected) in out.as_chunks::<4>().0.iter().zip(expected.as_chunks::<32>().0.iter()) {
                let refpx = std::array::from_fn::<_, 4, _>(|c| f64::from_le_bytes(expected[c * 8..c * 8 + 8].try_into().unwrap()));
                for c in 0..4 {
                    let error = if c < 3 { (actual[c] * actual[3] - refpx[c] * refpx[3]).abs() } else { (actual[c] - refpx[c]).abs() };
                    worst = worst.max(error);
                    assert!(error < 1. / 65535., "{} channel{c} error{error}", rec["id"]);
                }
            }
        }
        println!("Motion56 native premultiplied maximum {worst}");
        assert_eq!(proof["records"].as_array().unwrap().len(), if batch == "data" { 56 } else { 24 });
    }
}
#[test]
fn motion_mid_work_cancel_bad_extent_and_nonfinite_are_atomic() {
    let area = Rect::new(0, 0, 256, 256);
    let src = Surface::new(PixelFormat::RGBA64F);
    let before = src.clone();
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 8;
    assert_eq!(
        try_motion_blur_linear(&src, area, area, MotionBlurParams { length: 10., angle_degrees: 17. }, Default::default(), &Interrupt::new(&cancel, &|_| {})),
        Err(RasterInputError::Cancelled)
    );
    assert_eq!(src, before);
    assert_eq!(
        try_motion_blur_linear(
            &src,
            Rect::new(i32::MAX - 1, 0, i32::MAX, 1),
            area,
            MotionBlurParams { length: 10., angle_degrees: 0. },
            Default::default(),
            &Interrupt::NONE
        ),
        Err(RasterInputError::InvalidBounds)
    );
    let mut invalid = src;
    invalid.try_write_pixel_f64(0, 0, &[f64::INFINITY, 0., 0., 1.]).unwrap();
    let before = invalid.clone();
    assert_eq!(
        try_motion_blur_linear(&invalid, Rect::new(0, 0, 1, 1), area, MotionBlurParams { length: 0., angle_degrees: 0. }, Default::default(), &Interrupt::NONE),
        Err(RasterInputError::InvalidSample)
    );
    assert_eq!(invalid, before);
}
