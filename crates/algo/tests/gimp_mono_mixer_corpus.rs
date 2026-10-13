#![cfg(feature = "corpus")]
use photocraft_algo::gimp::{
    point::try_apply,
    registry::{WorkingEncoding, point_member},
};
use photocraft_cms::{Builtin, Intent, Profile, Transform};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use std::path::PathBuf;

#[test]
#[ignore = "local unpublished independent GIMP false-luminosity weight edits"]
fn independently_authored_false_luminosity_weight_edits() {
    use sha2::Digest;
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-edits-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", sha2::Sha256::digest(&bytes)), "37935d87fac7c0e8aed14f326c4c39eeedd2e450fb6a7074d90c75d34dcb84f2");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            format!("{:x}", sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())),
            artifact["sha256"].as_str().unwrap()
        );
    }
    let to_linear = Transform::new(Builtin::Srgb.profile(), Builtin::LinearSrgb.profile(), Intent::RelativeColorimetric, false).unwrap();
    let to_encoded = Transform::new(Builtin::LinearSrgb.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
    for stage in ["source", "weights"] {
        let prefix = format!("mono.{stage}.fresh");
        let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.state.json"))).unwrap()).unwrap();
        let config = &state["layers"][0]["filters"][0]["config"];
        assert_eq!(config["preserve-luminosity"], false);
        let args = [
            ("red", config["red"].as_f64().unwrap()),
            ("green", config["green"].as_f64().unwrap()),
            ("blue", config["blue"].as_f64().unwrap()),
            ("preserve-luminosity", 0.0),
        ];
        let (_, operation) = point_member("gegl:mono-mixer", Some("0:0"), &args).unwrap();
        let native = std::fs::read(root.join(format!("{prefix}.layer0.bin"))).unwrap();
        let projection = std::fs::read(root.join(format!("{prefix}.projection.bin"))).unwrap();
        assert_eq!(native.len(), 32 * 32 * 4);
        assert_eq!(projection.len(), 32 * 32 * 32);
        let mut worst = 0.0_f64;
        for (pixel, expected) in native.as_chunks::<4>().0.iter().zip(projection.as_chunks::<32>().0.iter()) {
            let encoded = [f64::from(pixel[0]) / 255.0, f64::from(pixel[1]) / 255.0, f64::from(pixel[2]) / 255.0];
            let mut linear = [0.0; 3];
            to_linear.try_eval_f64(&encoded, &mut linear).unwrap();
            let mut output = [0.0; 4];
            operation.try_eval_pixel(&[linear[0], linear[1], linear[2], f64::from(pixel[3]) / 255.0], 3, &mut output).unwrap();
            let mut encoded = [0.0; 3];
            to_encoded.try_eval_f64(&output[..3], &mut encoded).unwrap();
            for (actual, expected) in encoded.into_iter().chain([output[3]]).zip(expected.as_chunks::<8>().0.iter()) {
                worst = worst.max((actual - f64::from_le_bytes(*expected)).abs());
            }
        }
        println!("Mono Mixer {stage}: {} /255", worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
    }
}

#[test]
#[ignore = "local unpublished independent GIMP mono mixer originals; run explicitly with corpus"]
fn authored_false_luminosity_mono_mixer_source_oracle() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    assert!(root.is_dir(), "stage full XCF corpus or set PHOTOCRAFT_XCF_FULL_CORPUS");
    let (name, version) = ("gegl-mono-mixer", "0:0");
    let dir = root.join("operations").join(name);
    let base = format!("operation-{name}");
    let raw = std::fs::read(dir.join(format!("{base}.source.0.native.bin"))).unwrap();
    assert_eq!(raw.len(), 32 * 32 * 4);
    assert!(raw.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    let proof_bytes = std::fs::read(dir.join("probe.json")).unwrap();
    use sha2::Digest;
    assert_eq!(format!("{:x}", sha2::Sha256::digest(&proof_bytes)), "4694a515fd36176177bf8e04b39f7adff365d986be8c5a05ee8547aa9f0bb87c");
    let proof: serde_json::Value = serde_json::from_slice(&proof_bytes).unwrap();
    for (field, hash) in [
        ("input", "input_sha256"),
        ("reference", "reference_sha256"),
        ("reference_edit", "reference_edit_sha256"),
        ("reference_edit_render", "reference_edit_render_sha256"),
    ] {
        let bytes = std::fs::read(dir.join(proof["cases"][0][field].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", sha2::Sha256::digest(&bytes)), proof["cases"][0][hash].as_str().unwrap());
    }
    assert_eq!(format!("{:x}", sha2::Sha256::digest(&raw)), proof["cases"][0]["state_reopened"]["layers"][0]["native"]["sha256"].as_str().unwrap());
    let strings = &proof["cases"][0]["serialization"]["tables"][0]["records"][0]["effects"][0]["strings"];
    assert_eq!(strings[2], name.replacen('-', ":", 1));
    assert_eq!(strings[3], version);
    for (stage, suffix) in [("reopened", "ref.png")] {
        let state: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{base}.{stage}_state.json"))).unwrap()).unwrap();
        let filter = &state["layers"][0]["filters"][0];
        let config = filter["config"].as_object().unwrap();
        let args: Vec<(&str, f64)> =
            config.iter().map(|(n, v)| (n.as_str(), v.as_f64().unwrap_or_else(|| if v.as_bool().unwrap() { 1.0 } else { 0.0 }))).collect();
        let operation = name.replacen('-', ":", 1);
        let (member, evaluator) = point_member(&operation, Some(version), &args).unwrap();
        let reference = photocraft_codecs::decode(&std::fs::read(dir.join(format!("{base}.{suffix}"))).unwrap()).unwrap();
        let working = Profile::parse(reference.icc.as_ref().unwrap()).unwrap();
        let linear = Builtin::LinearSrgb.profile();
        let to_linear = Transform::new(&working, linear, Intent::RelativeColorimetric, false).unwrap();
        let from_linear = Transform::new(linear, &working, Intent::RelativeColorimetric, false).unwrap();
        let mut samples = Vec::new();
        for px in raw.as_chunks::<4>().0.iter() {
            let encoded = [f64::from(px[0]) / 255.0, f64::from(px[1]) / 255.0, f64::from(px[2]) / 255.0];
            let mut value = encoded;
            if member.encoding == WorkingEncoding::Linear {
                to_linear.try_eval_f64(&encoded, &mut value).unwrap();
            }
            samples.extend_from_slice(&value);
            samples.push(1.0);
        }
        let area = Rect::new(0, 0, 32, 32);
        let mut source = Surface::new(PixelFormat::RGBA64F);
        source.try_write_region_f64(area, &samples).unwrap();
        let result = if filter["visible"] == false {
            source.clone()
        } else {
            try_apply(&source, area, evaluator, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).unwrap()
        };
        let reference = reference.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
        let pixels = result.try_read_region_f64(area).unwrap();
        let mut max = [0.0_f64; 4];
        let mut sum = [0.0_f64; 4];
        for (px, expected) in pixels.as_chunks::<4>().0.iter().zip(reference.data().as_chunks::<4>().0.iter()) {
            let mut encoded = [px[0], px[1], px[2]];
            if member.encoding == WorkingEncoding::Linear {
                from_linear.try_eval_f64(&px[..3], &mut encoded).unwrap();
            }
            for c in 0..4 {
                let v = if c == 3 { px[3] } else { encoded[c] };
                let d = (v.clamp(0.0, 1.0) * 255.0 - f64::from(expected[c])).abs();
                max[c] = max[c].max(d);
                sum[c] += d;
            }
        }
        println!("{name}/{stage}: max={max:?} mean={:?}", sum.map(|v| v / 1024.0));
        assert!(max.iter().all(|v| *v <= 2.0), "existing 2/255 contract failed: {name}/{stage} {max:?}");
        assert_eq!(source.try_read_region_f64(area).unwrap(), samples);
    }
}
