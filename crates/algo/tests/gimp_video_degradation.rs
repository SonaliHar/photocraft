use photocraft_algo::gimp::point_video_degradation::VideoDegradation;
use sha2::{Digest, Sha256};
#[test]
fn point_dispatch_requires_explicit_coordinates() {
    use photocraft_algo::gimp::{point::PointOperation, registry::video_degradation_member};
    let operation = VideoDegradation { pattern: 5, additive: false, rotated: false };
    let (_, point) = video_degradation_member(Some("0:0"), operation).unwrap();
    assert!(video_degradation_member(Some("future"), operation).is_err());
    assert!(video_degradation_member(Some("0:0"), VideoDegradation { pattern: 9, ..operation }).is_err());
    let prepared = point.prepare().unwrap();
    let input = [0.25, 0.5, 0.75, -0.0];
    let mut output = [17.; 4];
    assert!(point.try_eval_pixel(&input, 3, &mut output).is_err());
    assert_eq!(output, [17.; 4]);
    assert!(prepared.try_eval_pixel(&input, 3, &mut output).is_err());
    assert_eq!(output, [17.; 4]);
    prepared.try_eval_pixel_at((1, 0), &input, 3, &mut output).unwrap();
    assert_eq!(output[..3], [0., 0.5, 0.]);
    assert_eq!(output[3].to_bits(), (-0.0f64).to_bits());
    output = [17.; 4];
    assert!(point.try_eval_pixel_at((0, 0), &[0.1, 0.5, 0.75, 1.], 3, &mut output).is_err());
    assert_eq!(output, [17.; 4]);
    assert!(point.try_eval_pixel_at((0, 0), &[0.25, 0.5], 1, &mut output).is_err());
    assert_eq!(output, [17.; 4]);
    PointOperation::Invert.try_eval_pixel_at((i32::MIN, i32::MAX), &[0.25, 1.], 1, &mut [0.; 2]).unwrap();
}
#[test]
#[ignore = "independent local public GIMP evidence"]
fn independently_saved_periodic_native_controls() {
    let full = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let names = ["staggered", "large-staggered", "striped", "wide-striped", "long-staggered", "3x3", "large-3x3", "hex", "dots"];
    let mut count = 0;
    for folder in ["video-degradation-response-evidence", "video-degradation-period-holdout-evidence"] {
        let root = full.join(folder);
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        for r in j["records"].as_array().unwrap() {
            let cfg = &r["current"]["layers"][0]["filters"][0]["config"];
            let pattern = names.iter().position(|n| Some(*n) == cfg["pattern"].as_str()).unwrap() as u8;
            let op = VideoDegradation { pattern, additive: cfg["additive"].as_bool().unwrap(), rotated: cfg["rotated"].as_bool().unwrap() };
            let read = |d: &serde_json::Value| {
                let b = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
                assert_eq!(b.len() as u64, d["bytes"].as_u64().unwrap());
                let h = format!("{:x}", Sha256::digest(&b));
                assert_eq!(h, d["sha256"].as_str().unwrap());
                b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect::<Vec<_>>()
            };
            let input = read(&r["current"]["layers"][0]["native"]);
            let expected = read(&r["baked"]["layers"][0]["native"]);
            let width = r["current"]["size"][0].as_u64().unwrap() as usize;
            for (i, (src, dst)) in input.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()).enumerate() {
                let mut out = [17.; 4];
                op.try_eval_nonlinear_float((i % width) as i32, (i / width) as i32, src, &mut out).unwrap();
                assert_eq!(out.map(f32::to_bits), dst.map(f32::to_bits), "{} ({},{})", r["id"], i % width, i / width);
            }
            count += 1;
        }
    }
    assert_eq!(count, 72);
}

#[test]
fn hostile_inputs_leave_output_unchanged() {
    let op = VideoDegradation { pattern: 2, additive: false, rotated: false };
    let mut out = [17.; 4];
    for bad in [[f32::NAN, 0., 0., 1.], [-0.1, 0., 0., 1.], [0., 0., 0., 1.1]] {
        assert!(op.try_eval_nonlinear_float(0, 0, &bad, &mut out).is_err());
        assert_eq!(out, [17.; 4]);
    }
    assert!(VideoDegradation { pattern: 9, ..op }.try_eval_nonlinear_float(0, 0, &[0.; 4], &mut out).is_err());
    assert_eq!(out, [17.; 4]);
    assert!(op.try_eval_nonlinear_float(0, 0, &[0.; 3], &mut out).is_err());
    assert_eq!(out, [17.; 4]);
}
#[test]
fn checked_periodic_coordinates_and_alpha_bits() {
    let op = VideoDegradation { pattern: 2, additive: false, rotated: false };
    for (x, y) in [(i32::MIN, i32::MIN), (i32::MAX, i32::MAX), (0, -1), (0, 0), (0, 1), (0, 2)] {
        let mut out = [17.; 4];
        op.try_eval_nonlinear_float(x, y, &[0.25, 0.5, 0.75, -0.0], &mut out).unwrap();
        let channel = i64::from(y).rem_euclid(3) as usize;
        assert_eq!(out[3].to_bits(), (-0.0f32).to_bits());
        for (c, v) in out.iter().take(3).enumerate() {
            assert_eq!(*v, if c == channel { [0.25, 0.5, 0.75][c] } else { 0. });
        }
    }
}

#[test]
#[ignore = "release24MP pure kernel timing; excludes conversions/storage/allocations"]
fn pure_kernel_release24mp() {
    let op = VideoDegradation { pattern: 8, additive: true, rotated: false };
    let mut out = [0.; 4];
    let input = [0.25, 0.5, 0.75, 0.5];
    let mut checksum = 0.0f64;
    let start = std::time::Instant::now();
    for i in 0..24_000_000 {
        op.try_eval_nonlinear_float(i % 6000, i / 6000, &input, &mut out).unwrap();
        checksum += f64::from(std::hint::black_box(out)[0]);
    }
    println!("VideoDegradation pure24MP {:.6}s checksum{}", start.elapsed().as_secs_f64(), checksum);
    assert!(checksum.is_finite());
}
