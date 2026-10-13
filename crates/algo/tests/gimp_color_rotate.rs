use photocraft_algo::gimp::point_color_rotate::{ColorRotate, GrayMode, HueArc};
fn operation() -> ColorRotate {
    ColorRotate {
        source: HueArc { from_degrees: 0.0, to_degrees: 90.0, clockwise: false },
        destination: HueArc { from_degrees: 180.0, to_degrees: 270.0, clockwise: false },
        gray_threshold: 0.0,
        gray_mode: GrayMode::ChangeTo,
        gray_hue_degrees: 0.0,
        gray_saturation: 0.0,
    }
}
#[test]
fn measured_directed_perceptual_red_to_cyan_alpha_exact() {
    for alpha in [0.0f32, -0.0, 0.25, 1.0] {
        let mut output = [17.0; 4];
        operation().try_eval_perceptual_float(&[1.0, 0.0, 0.0, alpha], &mut output).unwrap();
        assert_eq!(output[..3], [0.0, 1.0, 1.0]);
        assert_eq!(output[3].to_bits(), alpha.to_bits());
    }
}
#[test]
fn invalid_and_unqualified_inputs_are_atomic() {
    let baseline = operation();
    let mut cases = Vec::new();
    for value in [f32::NAN, f32::INFINITY, -1.0, 361.0] {
        let mut op = baseline;
        op.source.from_degrees = value;
        cases.push(op);
        let mut op = baseline;
        op.destination.to_degrees = value;
        cases.push(op);
        let mut op = baseline;
        op.gray_hue_degrees = value;
        cases.push(op);
    }
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let mut op = baseline;
        op.gray_threshold = value;
        cases.push(op);
        let mut op = baseline;
        op.gray_saturation = value;
        cases.push(op);
    }
    let mut op = baseline;
    op.source.clockwise = true;
    cases.push(op);
    let mut op = baseline;
    op.source.from_degrees = 300.0;
    op.source.to_degrees = 60.0;
    cases.push(op);
    let mut op = baseline;
    op.source.to_degrees = 0.0;
    cases.push(op);
    let mut op = baseline;
    op.destination.to_degrees = 180.0;
    cases.push(op);
    for op in cases {
        let mut output = [17.0; 4];
        assert!(op.try_eval_perceptual_float(&[1.0, 0.0, 0.0, 1.0], &mut output).is_err());
        assert_eq!(output, [17.0; 4]);
    }
    for index in 0..4 {
        for bad in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            let mut input = [0.5; 4];
            input[index] = bad;
            let mut output = [17.0; 4];
            assert!(baseline.try_eval_perceptual_float(&input, &mut output).is_err());
            assert_eq!(output, [17.0; 4]);
        }
    }
    for len in [0, 3, 5] {
        let mut output = [17.0; 4];
        assert!(baseline.try_eval_perceptual_float(&vec![0.5; len], &mut output).is_err());
        assert_eq!(output, [17.0; 4]);
        let mut output = vec![17.0; len];
        assert!(baseline.try_eval_perceptual_float(&[0.5; 4], &mut output).is_err());
        assert_eq!(output, vec![17.0; len]);
    }
}
#[test]
fn gray_threshold_and_directed_destination() {
    let mut op = operation();
    op.gray_threshold = 0.5;
    op.gray_hue_degrees = 220.0;
    op.gray_saturation = 0.75;
    let mut output = [17.0; 4];
    op.try_eval_perceptual_float(&[1.0, 0.5, 0.5, 0.0], &mut output).unwrap();
    for (a, b) in output.iter().zip([0.25, 0.5, 1.0, 0.0]) {
        assert!((a - b).abs() <= 1e-6);
    }
    op.gray_mode = GrayMode::TreatAs;
    op.gray_hue_degrees = 45.0;
    op.try_eval_perceptual_float(&[0.25, 0.25, 0.25, -0.0], &mut output).unwrap();
    for (a, b) in output[..3].iter().zip([0.0625, 0.109375, 0.25]) {
        assert!((a - b).abs() <= 1e-6);
    }
    assert_eq!(output[3].to_bits(), (-0.0f32).to_bits());
    op.gray_threshold = 0.0;
    op.destination.clockwise = true;
    op.try_eval_perceptual_float(&[1.0, 0.5, 0.0, 1.0], &mut output).unwrap();
    assert_eq!(output, [0.5, 1.0, 0.0, 1.0]);
}
#[test]
#[ignore = "frozen independently repeated perceptual Float operation responses; no source admission"]
fn public_perceptual_working_responses_and_nonzero_range_holdouts() {
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|n| format!("{n:02x}")).collect::<String>();
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let source = base.join("color-rotate-response-evidence");
    assert_eq!(hash(&std::fs::read(source.join("batch-manifest.json")).unwrap()), "d64019de7f02c318b56dad0e16bb6fb7ab8e85784e8e794710d114c2c5d15cbb");
    let original: serde_json::Value = serde_json::from_slice(&std::fs::read(source.join("observations.json")).unwrap()).unwrap();
    let mut pixels = 0;
    let mut refusals = 0;
    let mut worst = 0.0f32;
    for (name, manifest_hash) in [
        ("color-rotate-preferred-working-probe-evidence", "cf2cee246b65b37d4bd2b28d008602f5c9a7d43ee3d5bd41e79bd9c614eba9ad"),
        ("color-rotate-directed-holdout-evidence", "aea3510c357485c4f24839573c8ba45e223e10b181c50da80833bed492201739"),
    ] {
        let root = base.join(name);
        let manifest_raw = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(hash(&manifest_raw), manifest_hash);
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_raw).unwrap();
        let read_json = |name: &str| {
            let entry = manifest["files"].as_array().unwrap().iter().find(|e| e["file"] == name).unwrap();
            let raw = std::fs::read(root.join(name)).unwrap();
            assert_eq!(hash(&raw), entry["sha256"].as_str().unwrap());
            serde_json::from_slice::<serde_json::Value>(&raw).unwrap()
        };
        let observed = read_json("observations.json");
        assert_eq!(observed, read_json("fresh.json"));
        for row in observed["records"].as_array().unwrap() {
            assert_eq!(row["preferred_pad_formats"]["input"], "R~G~B~A float");
            assert_eq!(row["preferred_pad_formats"]["output"], "R~G~B~A float");
            let original_row = original["records"].as_array().unwrap().iter().find(|r| r["sha256"] == row["source_sha256"]).unwrap();
            assert_eq!(hash(&std::fs::read(source.join(original_row["file"].as_str().unwrap())).unwrap()), row["source_sha256"].as_str().unwrap());
            let config = if row["config"].is_object() { &row["config"] } else { &original_row["current"]["layers"][0]["filters"][0]["config"] };
            let number = |name: &str| config[name].as_f64().unwrap() as f32;
            let arc = |prefix: &str| HueArc {
                from_degrees: number(&format!("{prefix}-from")),
                to_degrees: number(&format!("{prefix}-to")),
                clockwise: config[format!("{prefix}-clockwise")].as_bool().unwrap(),
            };
            let op = ColorRotate {
                source: arc("src"),
                destination: arc("dest"),
                gray_threshold: number("threshold"),
                gray_mode: match config["gray-mode"].as_str().unwrap() {
                    "treat-as" => GrayMode::TreatAs,
                    "change-to" => GrayMode::ChangeTo,
                    _ => panic!("unknown public gray mode"),
                },
                gray_hue_degrees: number("hue"),
                gray_saturation: number("saturation"),
            };
            let read = |d: &serde_json::Value| {
                let raw = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
                assert_eq!(raw.len() as u64, d["bytes"].as_u64().unwrap());
                assert_eq!(hash(&raw), d["sha256"].as_str().unwrap());
                raw.as_chunks::<4>().0.iter().map(|v| f32::from_le_bytes(*v)).collect::<Vec<_>>()
            };
            let input = read(&row["input"]);
            let expected = read(&row["output"]);
            assert_eq!(input.len(), expected.len());
            for (i, (input, expected)) in input.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()).enumerate() {
                let mut output = [17.0; 4];
                if op.source.clockwise || op.source.from_degrees >= op.source.to_degrees {
                    assert!(op.try_eval_perceptual_float(input, &mut output).is_err());
                    assert_eq!(output, [17.0; 4]);
                    refusals += 1;
                    continue;
                }
                op.try_eval_perceptual_float(input, &mut output).unwrap();
                for (a, b) in output.iter().zip(expected) {
                    let error = (a - b).abs();
                    worst = worst.max(error);
                    assert!(error <= 1e-6, "{} pixel{i}: {output:?} vs {expected:?}", row["id"]);
                }
                assert_eq!(output[3].to_bits(), input[3].to_bits());
                pixels += 1;
            }
        }
    }
    assert_eq!(pixels, 6656);
    assert_eq!(refusals, 2048);
    println!("ColorRotate perceptualFloat public6656 pixels, explicitunsupported2048, worst={worst}");
}
#[test]
#[ignore = "24MP release pure perceptual arithmetic timing; excludes CMS/storage/allocations"]
fn color_rotate_24mp_working_kernel_timing() {
    let op = operation();
    let mut output = [0.0; 4];
    let start = std::time::Instant::now();
    let mut checksum = 0.0f64;
    for pixel in 0..24_000_000u32 {
        let input = [(pixel % 17) as f32 / 16.0, (pixel % 7) as f32 / 6.0, (pixel % 5) as f32 / 4.0, 0.5];
        op.try_eval_perceptual_float(std::hint::black_box(&input), &mut output).unwrap();
        checksum += f64::from(std::hint::black_box(output[0]));
    }
    assert!(checksum.is_finite() && checksum > 0.0);
    println!("ColorRotate perceptualFloat24MP pure kernel: {:.6}s checksum={checksum}", start.elapsed().as_secs_f64());
}

#[test]
fn generic_directed_color_writeback_preserves_raw_alpha() {
    use photocraft_algo::gimp::point::{PointOperation, try_apply};
    use photocraft_geom::Rect;
    use photocraft_raster::{Interrupt, Surface};
    let bounds = Rect::new(0, 0, 2, 1);
    let bits = [1f32.to_bits(), 0, 0, (-0f32).to_bits(), 1f32.to_bits(), 0, 0, 0.25f32.to_bits()];
    let bytes = bits.into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
    let mut source = Surface::new(photocraft_color::PixelFormat::RGBA32F);
    source.try_write_interleaved(bounds, &bytes).unwrap();
    let output = try_apply(&source, bounds, PointOperation::ColorRotate { operation: operation() }, 8 << 20, 1 << 20, &Interrupt::NONE).unwrap();
    let raw = output.try_read_interleaved(bounds).unwrap();
    let values = raw.as_chunks::<4>().0.iter().map(|b| u32::from_ne_bytes(*b)).collect::<Vec<_>>();
    assert_eq!(values, [0, 1f32.to_bits(), 1f32.to_bits(), (-0f32).to_bits(), 0, 1f32.to_bits(), 1f32.to_bits(), 0.25f32.to_bits()]);
    assert_eq!(source.try_read_interleaved(bounds).unwrap(), bytes);
}

#[test]
fn signed_hue_sector_wrap_and_fractional_arc_boundary() {
    // Independent public Float responses: negative red sector and fractional
    // green sector endpoints distinguish equivalent equations at Float precision.
    let cases = [
        (
            ColorRotate {
                source: HueArc { from_degrees: 30.0, to_degrees: 300.0, clockwise: false },
                destination: HueArc { from_degrees: 0.0, to_degrees: 360.0, clockwise: false },
                gray_threshold: 0.0,
                gray_mode: GrayMode::ChangeTo,
                gray_hue_degrees: 0.0,
                gray_saturation: 0.0,
            },
            [f32::from_bits(0x3f200002), f32::from_bits(0x3ea00006), f32::from_bits(0x3f200001), f32::from_bits(0x00000000)],
            [f32::from_bits(0x3f200002), f32::from_bits(0x3ea00008), f32::from_bits(0x3ea00006), f32::from_bits(0x00000000)],
        ),
        (
            ColorRotate {
                source: HueArc { from_degrees: 17.25, to_degrees: 143.75, clockwise: false },
                destination: HueArc { from_degrees: 33.125, to_degrees: 211.75, clockwise: false },
                gray_threshold: 0.3125,
                gray_mode: GrayMode::TreatAs,
                gray_hue_degrees: 61.125,
                gray_saturation: 0.625,
            },
            [f32::from_bits(0x3dafffff), f32::from_bits(0x3e000000), f32::from_bits(0x3dcfaaab), f32::from_bits(0x3e800000)],
            [f32::from_bits(0x3dafffff), f32::from_bits(0x3dd5aaaa), f32::from_bits(0x3e000000), f32::from_bits(0x3e800000)],
        ),
    ];
    for (op, input, expected) in cases {
        let mut output = [17.0; 4];
        op.try_eval_perceptual_float(&input, &mut output).unwrap();
        for (actual, reference) in output.iter().zip(expected) {
            assert!((actual - reference).abs() <= 1e-6, "{output:?} vs {expected:?}");
        }
        assert_eq!(output[3].to_bits(), input[3].to_bits());
    }
}

#[test]
fn collapsed_float_radian_source_span_is_unsupported() {
    let mut op = operation();
    op.source.from_degrees = 60.0;
    op.source.to_degrees = f32::from_bits(60f32.to_bits() + 1);
    assert!(matches!(op.validate(), Err(photocraft_raster::RasterInputError::UnsupportedConversion)));
    let mut output = [17.0; 4];
    assert!(matches!(op.try_eval_perceptual_float(&[1.0, 1.0, 0.0, 1.0], &mut output), Err(photocraft_raster::RasterInputError::UnsupportedConversion)));
    assert_eq!(output, [17.0; 4]);
}
