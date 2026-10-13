use photocraft_algo::gimp::point_color_overlay::ColorOverlay;

#[test]
fn overlay_alpha_blends_color_and_retains_source_coverage() {
    let op = ColorOverlay { color: [0.25, 0.5, 0.75, 0.25] };
    for alpha in [0.0, 0.125, 0.5, 1.0] {
        let mut output = [17.0; 4];
        op.try_eval_pixel(&[1.0, 0.0, 0.5, alpha], 3, &mut output).unwrap();
        assert_eq!(output, [0.8125, 0.125, 0.5625, alpha]);
    }
}

#[test]
fn invalid_operands_are_atomic_and_color_alpha_endpoints_are_explicit() {
    let base = ColorOverlay { color: [0.25, 0.5, 0.75, 0.25] };
    for value in [f32::NAN, f32::INFINITY] {
        for channel in 0..4 {
            let mut operation = base;
            operation.color[channel] = value;
            let mut output = [17.; 4];
            assert!(operation.try_eval_pixel(&[0.2, 0.4, 0.6, 0.5], 3, &mut output).is_err());
            assert_eq!(output, [17.; 4]);
        }
    }
    for value in [-0.001, 1.001] {
        let mut operation = base;
        operation.color[3] = value;
        let mut output = [17.; 4];
        assert!(operation.try_eval_pixel(&[0.2, 0.4, 0.6, 0.5], 3, &mut output).is_err());
        assert_eq!(output, [17.; 4]);
    }
    for value in [f64::NAN, f64::INFINITY, f64::MAX] {
        for channel in 0..4 {
            let mut input = [0.2, 0.4, 0.6, 0.5];
            input[channel] = value;
            let mut output = [17.; 4];
            assert!(base.try_eval_pixel(&input, 3, &mut output).is_err());
            assert_eq!(output, [17.; 4]);
        }
    }
    for channels in [0, 1, 2, 4, u8::MAX] {
        let mut output = [17.; 4];
        assert!(base.try_eval_pixel(&[0.; 4], channels, &mut output).is_err());
        assert_eq!(output, [17.; 4]);
    }
    for alpha in [0., 0.5, 1.] {
        let input = [0.125, 0.25, 0.5, alpha];
        let mut output = [17.; 4];
        ColorOverlay { color: [0.25, 0.5, 0.75, 0.] }.try_eval_pixel(&input, 3, &mut output).unwrap();
        assert_eq!(output, input);
        ColorOverlay { color: [0.25, 0.5, 0.75, 1.] }.try_eval_pixel(&input, 3, &mut output).unwrap();
        assert_eq!(output, [0.25, 0.5, 0.75, alpha]);
    }
}

#[test]
#[ignore = "private independent original working oracle, no shared source admission"]
fn original_operation_working_outputs() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let root = std::env::var_os("PHOTOCRAFT_COLOR_OVERLAY_PACKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/point-color-overlay-evidence"));
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|n| format!("{n:02x}")).collect::<String>();
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "828c1da757d0706cd6eb502699c74c0c2856745d3603c6ab6d1dd56d70df9831");
    let census: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let entry = census["artifacts"].as_array().unwrap().iter().find(|e| e["path"] == "observations.json").unwrap();
    assert_eq!(hash(&std::fs::read(root.join("observations.json")).unwrap()), entry["sha256"].as_str().unwrap());
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 360);
    let read = |d: &serde_json::Value| {
        let encoded = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&encoded), d["sha256"].as_str().unwrap());
        let size = d["decoded_bytes"].as_u64().unwrap();
        assert_eq!(size, 768);
        let mut bytes = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(size + 1).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len() as u64, size);
        assert_eq!(hash(&bytes), d["decoded_sha256"].as_str().unwrap());
        bytes
    };
    let mut worst = 0.0f64;
    for row in rows {
        let w = &row["working"];
        let input = read(&w["input"]);
        let output = read(&w["output"]);
        let color = std::array::from_fn(|i| w["color_in_working"][i].as_f64().unwrap() as f32);
        let op = ColorOverlay { color };
        for (a, b) in input.as_chunks::<16>().0.iter().zip(output.as_chunks::<16>().0.iter()) {
            let pixel = std::array::from_fn::<_, 4, _>(|i| f64::from(f32::from_le_bytes(a[i * 4..i * 4 + 4].try_into().unwrap())));
            let expected = std::array::from_fn::<_, 4, _>(|i| f64::from(f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())));
            let mut actual = [17.0; 4];
            op.try_eval_pixel(&pixel, 3, &mut actual).unwrap();
            assert_eq!(actual[3], expected[3], "input alpha must remain exact");
            for (actual, expected) in actual.iter().zip(expected) {
                worst = worst.max((actual - expected).abs());
            }
        }
    }
    assert!(worst <= 1e-6, "working Float max {worst}");
    println!(
        "360 direct public operation working states (including disabled image controls whose separate graph intentionally remains active), max {worst}; native/profile pipeline not admitted"
    );
}

#[test]
#[ignore = "release24MP standalone working-space performance and bounded raster gate"]
fn color_overlay_twenty_four_megapixel_checked_working_raster() {
    use photocraft_algo::gimp::point::{PointOperation, try_apply};
    use photocraft_geom::Rect;
    use photocraft_raster::{Interrupt, Surface};
    let source = Surface::new(photocraft_color::PixelFormat::RGBA8);
    let area = Rect::new(0, 0, 4800, 5000);
    let op = PointOperation::ColorOverlay { color: [0.25, 0.5, 0.75, 0.25] };
    assert!(try_apply(&source, area, op, 1, 1 << 36, &Interrupt::NONE).is_err());
    assert!(try_apply(&source, area, op, 1 << 29, 1, &Interrupt::NONE).is_err());
    assert!(try_apply(&source, area, op, 1 << 29, 1 << 36, &Interrupt::cancel_only(&|| true)).is_err());
    let started = std::time::Instant::now();
    let result = try_apply(&source, area, op, 1 << 29, 1 << 36, &Interrupt::NONE).unwrap();
    eprintln!("ColorOverlay standalone24MP working-spaceRGBA8: {:?}; output resident {} bytes", started.elapsed(), result.resident_payload_bytes().unwrap());
    assert_eq!(result.try_read_interleaved(Rect::new(4799, 4999, 4800, 5000)).unwrap(), [16, 32, 48, 0]);
    assert!(source.tiles().next().is_none());
}

#[test]
fn checked_overlay_raster_writes_color_and_float_coverage_together() {
    use photocraft_algo::gimp::point::{PointOperation, try_apply};
    use photocraft_geom::Rect;
    use photocraft_raster::{Interrupt, Surface};
    let source = Surface::new(photocraft_color::PixelFormat::RGBA8);
    let output =
        try_apply(&source, Rect::new(0, 0, 2, 1), PointOperation::ColorOverlay { color: [0.25, 0.5, 0.75, 0.25] }, 8 << 20, 1 << 20, &Interrupt::NONE).unwrap();
    assert_eq!(output.try_read_interleaved(Rect::new(0, 0, 2, 1)).unwrap(), [16, 32, 48, 0, 16, 32, 48, 0]);
    assert_eq!(source.try_read_interleaved(Rect::new(0, 0, 2, 1)).unwrap(), [0; 8]);
}
