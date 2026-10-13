//! CIE Lab/Yuv saturation derived from public chromatic-coordinate equations.
//! https://www.color.org/specification/ICC.1-2022-05.pdf (Lab) and independent
//! public GIMP buffer conversions qualify the implicit RGB matrix below.
//! Equal-energy Yuv neutral is observed, not inferred from source ICC bytes.
use photocraft_raster::RasterInputError;
const MATRIX: [[f64; 3]; 3] = [
    [0.4360348442062176, 0.3851166536268291, 0.1430513718061043],
    [0.2224879400268659, 0.7169038388792531, 0.060608257216285655],
    [0.01391588474914934, 0.09706050659327542, 0.7139289547138952],
];
const INVERSE: [[f64; 3]; 3] = [
    [3.134275682732312, -1.6172766464441777, -0.4907240625871371],
    [-0.9787936148487894, 1.9161604978562157, 0.033452377634677156],
    [0.07197632845009819, -0.22898311118653591, 1.4057168408035896],
];
fn apply(matrix: &[[f64; 3]; 3], rgb: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row.into_iter().zip(rgb).map(|(a, b)| a * b).sum())
}
/// Public float-buffer conversions normalize XYZ by its D50 white before
/// chromaticity interpolation. Double-buffer conversions retain physical XYZ.
/// Caller qualifies the actual source/current float route.
pub(super) fn eval_float_yuv(rgb: [f64; 3], scale: f64) -> Result<[f64; 3], RasterInputError> {
    if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) || !scale.is_finite() || !(0.0..=10.0).contains(&scale) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let white = apply(&MATRIX, [1.0; 3]);
    let xyz = apply(&MATRIX, rgb.map(|v| f64::from(v as f32)));
    let [x, y, z] = std::array::from_fn(|i| f64::from((xyz[i] / white[i]) as f32));
    let denominator = x + 15.0 * y + 3.0 * z;
    if denominator == 0.0 {
        return Ok([0.0; 3]);
    }
    let neutral_u = f64::from((4.0_f64 / 19.0) as f32);
    let neutral_v = f64::from((9.0_f64 / 19.0) as f32);
    let input_u = f64::from((4.0 * x / denominator) as f32);
    let input_v = f64::from((9.0 * y / denominator) as f32);
    let u = f64::from((neutral_u + (input_u - neutral_u) * scale) as f32);
    let v = f64::from((neutral_v + (input_v - neutral_v) * scale) as f32);
    if !v.is_finite() || v.abs() < 1e-6 {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let normalized = [9.0 * y * u / (4.0 * v), y, y * (12.0 - 3.0 * u - 20.0 * v) / (4.0 * v)];
    let output = apply(&INVERSE, std::array::from_fn(|i| normalized[i] * white[i]));
    if output.iter().any(|v| !v.is_finite() || v.abs() > 256.0) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    Ok(output.map(|v| f64::from(v as f32)))
}
pub(super) fn eval(rgb: [f64; 3], scale: f64, colorspace: u8) -> Result<[f64; 3], RasterInputError> {
    let xyz = apply(&MATRIX, rgb);
    let out = match colorspace {
        1 => {
            const WHITE: [f64; 3] = [0.9642, 1.0, 0.8249];
            const E: f64 = 216.0 / 24389.0;
            const K: f64 = 24389.0 / 27.0;
            let f = |t: f64| if t > E { t.cbrt() } else { (K * t + 16.0) / 116.0 };
            let [x, y, z] = std::array::from_fn(|i| f(xyz[i] / WHITE[i]));
            let adjusted = [y + (x - y) * scale, y, y + (z - y) * scale];
            let result = apply(
                &INVERSE,
                std::array::from_fn(|i| {
                    let f = adjusted[i];
                    (if f * f * f > E { f * f * f } else { (116.0 * f - 16.0) / K }) * WHITE[i]
                }),
            );
            result.map(|v| f64::from(v as f32))
        }
        2 => {
            let [x, y, z] = xyz;
            let denominator = x + 15.0 * y + 3.0 * z;
            if denominator == 0.0 {
                return Ok([0.0; 3]);
            }
            let u = 4.0 / 19.0 + (4.0 * x / denominator - 4.0 / 19.0) * scale;
            let v = 9.0 / 19.0 + (9.0 * y / denominator - 9.0 / 19.0) * scale;
            if !v.is_finite() || v.abs() < 1e-6 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            apply(&INVERSE, [9.0 * y * u / (4.0 * v), y, y * (12.0 - 3.0 * u - 20.0 * v) / (4.0 * v)])
        }
        _ => return Err(RasterInputError::UnsupportedConversion),
    };
    if out.iter().any(|v| !v.is_finite() || v.abs() > 256.0) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    Ok(out)
}

#[cfg(test)]
mod float_yuv_tests {
    use crate::gimp::point::PointOperation;
    #[test]
    #[ignore = "local unpublished independent public Float Yuv conversion controls"]
    fn independent_float_yuv_cube_conversion_and_live_controls() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("saturation-yuv-float-controls-v5");
        let hash = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
        let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(hash(&manifest), "48796fa3d6e2e8d5b90b1baf4532622524dd4dfa7e8aff2a98bc62b9037b7d0f");
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        for a in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(hash(&std::fs::read(root.join(a["path"].as_str().unwrap())).unwrap()), a["sha256"]);
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let source = std::fs::read(root.join(proof["source"]["file"].as_str().unwrap())).unwrap();
        let decode = |p: &[u8]| std::array::from_fn::<_, 4, _>(|i| f64::from_le_bytes(p[i * 8..i * 8 + 8].try_into().unwrap()));
        let mut count = 0;
        let mut worst = 0.0_f64;
        for row in proof["records"].as_array().unwrap() {
            let op = PointOperation::Saturation { scale: row["scale"].as_f64().unwrap(), colorspace: 2 };
            let native = std::fs::read(root.join(row["live"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(native.len(), source.len());
            for (pixel, expected) in source.as_chunks::<32>().0.iter().zip(native.as_chunks::<32>().0.iter()) {
                let input = decode(pixel);
                let expected = decode(expected);
                let mut output = [0.; 4];
                op.try_eval_float_yuv_pixel(&input, &mut output).unwrap();
                assert_eq!(output[3].to_bits(), input[3].to_bits());
                worst = worst.max(output.into_iter().zip(expected).map(|(a, b)| (a - b).abs()).fold(0.0_f64, f64::max));
                count += 1;
            }
        }
        assert_eq!(count, 625);
        println!("625 independent public Float Yuv holdouts max {}/255", worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
    }
    #[test]
    fn checked_float_yuv_preserves_alpha_and_rejects_invalid_input_atomically() {
        let op = PointOperation::Saturation { scale: 0.5, colorspace: 2 };
        let mut output = [17.0; 4];
        let input = [0.125, 0.375, 0.75, 0.25];
        op.try_eval_float_yuv_pixel(&input, &mut output).unwrap();
        assert_eq!(output[3].to_bits(), input[3].to_bits());
        for invalid in [[f64::NAN, 0., 0., 1.], [0., 0., 0., f64::INFINITY], [-1., 0., 0., 1.], [0., 0., 0., 2.]] {
            let before = output;
            assert!(op.try_eval_float_yuv_pixel(&invalid, &mut output).is_err());
            assert_eq!(output, before);
        }
        assert!(op.try_eval_float_yuv_pixel(&input[..3], &mut output).is_err());
        assert!(PointOperation::Saturation { scale: f64::NAN, colorspace: 2 }.try_eval_float_yuv_pixel(&input, &mut output).is_err());
        assert!(PointOperation::Saturation { scale: 0.5, colorspace: 1 }.try_eval_float_yuv_pixel(&input, &mut output).is_err());
    }
    #[test]
    #[ignore = "24MP release-only float Yuv kernel timing; excludes storage/CMS"]
    fn float_yuv_24mp_release_kernel() {
        assert!(!std::hint::black_box(cfg!(debug_assertions)), "run this benchmark in release");
        let input = std::hint::black_box([0.125, 0.375, 0.75, 0.5]);
        let operation = PointOperation::Saturation { scale: 0.75, colorspace: 2 };
        let mut output = [0.; 4];
        let start = std::time::Instant::now();
        for _ in 0..24_000_000 {
            operation.try_eval_float_yuv_pixel(std::hint::black_box(&input), &mut output).unwrap();
            std::hint::black_box(output);
        }
        println!("24MP float Yuv kernel {:?}; source decode, CMS, tiles and persistence excluded", start.elapsed());
    }
}
