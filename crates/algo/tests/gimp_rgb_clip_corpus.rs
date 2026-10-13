#![cfg(feature = "corpus")]
use photocraft_algo::gimp::registry::{WorkingEncoding, point_member};
use photocraft_cms::{Builtin, Intent, Transform};

#[test]
#[ignore = "local unpublished independent finite HDR RGB Clip originals"]
fn independently_authored_hdr_rgb_clip_limits_and_boolean_edit() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("rgb-clip-hdr-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", sha2::Sha256::digest(&bytes)), "45f79892e29ebb9bd3ff07a9c6231063bfe4028e86e3a79562a7daab85fa9532");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            format!("{:x}", sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())),
            artifact["sha256"].as_str().unwrap()
        );
    }
    let encoded = Transform::new(Builtin::LinearSrgb.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
    for stage in ["source", "limits", "low-disabled"] {
        let prefix = format!("clip.{stage}.fresh");
        let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.state.json"))).unwrap()).unwrap();
        assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_LINEAR");
        let config = &state["layers"][0]["filters"][0]["config"];
        let args = [
            ("clip-low", if config["clip-low"].as_bool().unwrap() { 1.0 } else { 0.0 }),
            ("low-limit", config["low-limit"].as_f64().unwrap()),
            ("clip-high", if config["clip-high"].as_bool().unwrap() { 1.0 } else { 0.0 }),
            ("high-limit", config["high-limit"].as_f64().unwrap()),
        ];
        let (member, operation) = point_member("gegl:rgb-clip", Some("0:0"), &args).unwrap();
        assert_eq!(member.encoding, WorkingEncoding::Linear);
        let native = std::fs::read(root.join(format!("{prefix}.layer0.bin"))).unwrap();
        let projection = std::fs::read(root.join(format!("{prefix}.projection.bin"))).unwrap();
        assert_eq!(native.len(), 8 * 6 * 16);
        assert_eq!(projection.len(), 8 * 6 * 32);
        let mut worst = 0.0_f64;
        for (pixel, reference) in native.as_chunks::<16>().0.iter().zip(projection.as_chunks::<32>().0.iter()) {
            let input: Vec<_> = pixel.as_chunks::<4>().0.iter().map(|sample| f64::from(f32::from_le_bytes(*sample))).collect();
            let mut output = [0.0; 4];
            operation.try_eval_pixel(&input, 3, &mut output).unwrap();
            assert_eq!(output[3].to_bits(), input[3].to_bits());
            let mut color = [0.0; 3];
            encoded.try_eval_f64(&output[..3], &mut color).unwrap();
            for (actual, expected) in color.into_iter().chain([output[3]]).zip(reference.as_chunks::<8>().0.iter()) {
                worst = worst.max((actual - f64::from_le_bytes(*expected)).abs());
            }
        }
        println!("RGB Clip {stage}: {} /255", worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
    }
}
