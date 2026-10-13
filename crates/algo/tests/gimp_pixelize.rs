use photocraft_algo::gimp::neighborhood::{GaussianLimits, PixelizeParams, try_pixelize_square};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use sha2::{Digest, Sha256};
#[test]
fn partial_blocks_negative_offsets_and_atomic_wide_input() {
    let area = Rect::new(-1, -1, 2, 2);
    let mut src = Surface::new(PixelFormat::RGBA64F);
    for y in -1..2 {
        for x in -1..2 {
            src.try_write_pixel_f64(x, y, &[f64::from(x), f64::from(y), 2., 0.5]).unwrap();
        }
    }
    src.try_write_pixel_f64(100, 100, &[-0., f64::from_bits(0x3fe0000000000001), 3., 0.]).unwrap();
    let before = src.clone();
    let params = PixelizeParams { size_x: 16, size_y: 16, offset_x: -2, offset_y: -2 };
    let out = try_pixelize_square(&src, area, area, params, Default::default(), &Interrupt::NONE).unwrap();
    let mut pixel = [0.; 4];
    out.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
    assert_eq!(pixel, [0., 0., 2., 0.5]);
    assert_eq!(out.default_bytes(), src.default_bytes());
    assert_eq!(out.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap(), src.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap());
    assert_eq!(
        try_pixelize_square(&src, area, area, params, GaussianLimits { resident_bytes: 1, work_units: u64::MAX }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(try_pixelize_square(&src, area, area, params, Default::default(), &Interrupt::new(&|| true, &|_| {})), Err(RasterInputError::Cancelled));
    assert_eq!(
        try_pixelize_square(&src, area, area, PixelizeParams { size_x: 0, ..params }, Default::default(), &Interrupt::NONE),
        Err(RasterInputError::InvalidBounds)
    );
    assert_eq!(src, before);
}
#[test]
#[ignore = "independent Pixelize native basis in owned scratch"]
fn independent_native_pixelize_basis() {
    for batch in ["data", "holdouts"] {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol2-xcf-pixelize-probes").join(batch);
        let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        assert_eq!(data["errors"].as_array().unwrap().len(), 0);
        let mut maximum = 0f64;
        for rec in data["records"].as_array().unwrap() {
            let raw = std::fs::read(root.join(rec["input"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&raw)), rec["input"]["sha256"].as_str().unwrap());
            let cfg = &rec["filter_config"];
            let area = Rect::new(0, 0, rec["input"]["size"][0].as_i64().unwrap() as i32, rec["input"]["size"][1].as_i64().unwrap() as i32);
            let src = Surface::try_from_interleaved(PixelFormat::RGBA64F, area, &raw).unwrap();
            let params = PixelizeParams {
                size_x: cfg["size-x"].as_u64().unwrap() as u32,
                size_y: cfg["size-y"].as_u64().unwrap() as u32,
                offset_x: cfg["offset-x"].as_i64().unwrap() as i32,
                offset_y: cfg["offset-y"].as_i64().unwrap() as i32,
            };
            let out = try_pixelize_square(&src, area, area, params, Default::default(), &Interrupt::NONE).unwrap().try_read_region_f64(area).unwrap();
            let expected = std::fs::read(root.join(rec["output"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&expected)), rec["output"]["sha256"].as_str().unwrap());
            assert_eq!(out.len() * 8, expected.len());
            for (actual, reference) in out.as_chunks::<4>().0.iter().zip(expected.as_chunks::<32>().0.iter()) {
                let reference = std::array::from_fn::<_, 4, _>(|c| f64::from_le_bytes(reference[c * 8..c * 8 + 8].try_into().unwrap()));
                for c in 0..4 {
                    let delta = if c < 3 { (actual[c] * actual[3] - reference[c] * reference[3]).abs() } else { (actual[c] - reference[c]).abs() };
                    maximum = maximum.max(delta);
                    assert!(delta < 1. / 65535., "{} {delta}", rec["id"]);
                }
            }
        }
        println!("Pixelize native maximum {maximum}");
        assert_eq!(data["records"].as_array().unwrap().len(), if batch == "data" { 24 } else { 12 });
    }
}
#[test]
fn pixelize_large_blocks_mid_cancel_and_nonfinite_are_atomic() {
    let area = Rect::new(0, 0, 256, 256);
    let src = Surface::new(PixelFormat::RGBA64F);
    let before = src.clone();
    let params = PixelizeParams { size_x: i32::MAX as u32, size_y: i32::MAX as u32, offset_x: i32::MIN, offset_y: i32::MAX };
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 8;
    assert_eq!(try_pixelize_square(&src, area, area, params, Default::default(), &Interrupt::new(&cancel, &|_| {})), Err(RasterInputError::Cancelled));
    assert_eq!(src, before);
    let mut bad = src;
    bad.try_write_pixel_f64(0, 0, &[f64::INFINITY, 0., 0., 1.]).unwrap();
    let before = bad.clone();
    assert_eq!(try_pixelize_square(&bad, Rect::new(0, 0, 1, 1), area, params, Default::default(), &Interrupt::NONE), Err(RasterInputError::InvalidSample));
    assert_eq!(bad, before);
}
