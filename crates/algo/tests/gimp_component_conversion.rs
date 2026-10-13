// Diagnostic inverse only: this does not implement a native/profile converter.
fn rgb_from_hsv([h, s, v]: [f64; 3]) -> Result<[f64; 3], &'static str> {
    if ![h, s, v].iter().all(|n| n.is_finite()) || !(0.0..=1.0).contains(&h) || !(0.0..=1.0).contains(&s) || v < 0.0 {
        return Err("invalid explicit HSV coordinates");
    }
    let sector = h * 6.0;
    let chroma = v * s;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let rgb = match sector as u8 {
        0 | 6 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        5 => [chroma, 0.0, x],
        _ => return Err("invalid HSV sector"),
    };
    Ok(rgb.map(|n| n + v - chroma))
}

// Conversion rounding can produce slightly nonunit model coordinates. Retain
// them here without broadening the production kernel's unit-domain guard.
fn diagnostic_hsv([r, g, b]: [f64; 3]) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let chroma = max - min;
    let hue = if chroma == 0.0 {
        0.0
    } else {
        let sector = if max == r {
            (g - b) / chroma
        } else if max == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        };
        (sector / 6.0).rem_euclid(1.0)
    };
    [hue, if max == 0.0 { 0.0 } else { chroma / max }, max]
}

#[test]
fn inverse_hexcone_model_controls() {
    assert_eq!(rgb_from_hsv([0., 1., 1.]).unwrap(), [1., 0., 0.]);
    assert_eq!(rgb_from_hsv([1. / 3., 1., 1.]).unwrap(), [0., 1., 0.]);
    assert_eq!(rgb_from_hsv([2. / 3., 1., 1.]).unwrap(), [0., 0., 1.]);
    assert_eq!(rgb_from_hsv([0.75, 0., 0.5]).unwrap(), [0.5; 3]);
    assert!(rgb_from_hsv([f64::NAN, 1., 1.]).is_err());
    assert!(rgb_from_hsv([0., -0.1, 1.]).is_err());
}

#[test]
#[ignore = "independent public format-space observation; no source converter admission"]
fn actual_srgb_space_does_not_make_intermediate_conversion_associative() {
    use sha2::{Digest, Sha256};
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("component-polar-space-route-evidence-v2");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let raw = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&raw), "265339ed2c4c800add380baad53d07c4811bec173cb829a97fbb153d8ff85a1b");
    let manifest: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        let entry = manifest["artifacts"].as_array().unwrap().iter().find(|v| v["path"] == path).unwrap();
        assert_eq!(bytes.len() as u64, entry["bytes"].as_u64().unwrap());
        assert_eq!(hash(&bytes), entry["sha256"].as_str().unwrap());
        bytes
    };
    let observation: serde_json::Value = serde_json::from_slice(&read("g/observations.json")).unwrap();
    assert!(observation["errors"].as_array().unwrap().is_empty());
    let rows = observation["records"].as_array().unwrap();
    assert_eq!(rows.len(), 36);
    let data = |descriptor: &serde_json::Value| {
        let bytes = read(&format!("g/{}", descriptor["file"].as_str().unwrap()));
        assert_eq!(hash(&bytes), descriptor["sha256"].as_str().unwrap());
        bytes
    };
    for row in rows {
        assert_eq!(row["native_format"]["space_name"], "sRGB");
        assert_eq!(row["assigned_profile"], false);
        for request in row["requests"].as_object().unwrap().values() {
            assert_eq!(request["plain_format"]["space_name"], "sRGB");
            let plain = data(&request["plain_buffer_get"]);
            for target in ["native_explicit_color", "srgb_explicit_color"] {
                assert_eq!(request[target]["format"]["space_name"], "sRGB");
                assert_eq!(plain, data(&request[target]["color_get_bytes"]));
            }
        }
    }
    let row = rows.iter().find(|v| v["id"] == "rgb.u32_linear.hue.source").unwrap();
    let request = &row["requests"]["HSV double"];
    let direct = data(&request["plain_buffer_get"]);
    let via = data(&request["public_intermediates"]["R'G'B'A float"]["output"]);
    let sample = |bytes: &[u8]| f64::from_le_bytes(bytes[480..488].try_into().unwrap());
    assert!((sample(&direct) - sample(&via)).abs() > 0.016);
    println!(
        "36 sources share native/requested sRGB identity; exact direct public conversions still differ after a nonlinear Float intermediate. This is a conversion-route regression, not RGB pipeline qualification."
    );
}

#[test]
#[ignore = "full900 frozen public model/native stage diagnosis, no RGB source admission"]
fn frozen_conversion_stage_difference_is_not_hsv_arithmetic() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let packet = full.join("point-component-extract-hsv-hsl-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let manifest_bytes = std::fs::read(packet.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest_bytes), "32c9948cc94bdba33979deff8f116da9f1ce5f52e58bf4b32649f173c3d3dcf1");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let load = |relative: &str| {
        let bytes = std::fs::read(packet.join(relative)).unwrap();
        let row = manifest["artifacts"].as_array().unwrap().iter().find(|row| row["path"] == relative).unwrap();
        assert_eq!(hash(&bytes), row["sha256"].as_str().unwrap());
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };
    let source = load("states/observations.json");
    let working = load("working/observations.json");
    let sources = source["records"].as_array().unwrap();
    let models = working["records"].as_array().unwrap();
    assert_eq!(sources.len(), 900);
    assert_eq!(models.len(), 900);
    let read = |root: &str, descriptor: &serde_json::Value| {
        let encoded = std::fs::read(packet.join(root).join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&encoded), descriptor["sha256"].as_str().unwrap());
        let size = descriptor["decoded_bytes"].as_u64().unwrap();
        assert!(size <= 1536);
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(size + 1).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded.len() as u64, size);
        assert_eq!(hash(&decoded), descriptor["decoded_sha256"].as_str().unwrap());
        decoded
    };
    let sample = |b: &[u8], i: usize, width: usize| {
        if width == 8 {
            f64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap())
        } else {
            f64::from(f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
        }
    };
    let mut roundtrip_max = 0f64;
    let mut channel_max = 0f64;
    let mut hue_max = 0f64;
    let mut native_double_rounding_max = 0f64;
    for (source, model) in sources.iter().zip(models) {
        assert_eq!(source["id"], model["id"]);
        assert_eq!(source["sha256"], model["source_sha256"]);
        for (name, width) in [("HSV float", 4), ("HSV double", 8)] {
            let hsv = read("working", &model["input_formats"][name]);
            let nl = read("working", &model["input_formats"]["R'G'B'A double"]);
            for (hsv, nl) in hsv.chunks_exact(3 * width).zip(nl.as_chunks::<32>().0.iter()) {
                let hsv = std::array::from_fn(|i| sample(hsv, i, width));
                let reconstructed = rgb_from_hsv(hsv).unwrap();
                let forward = diagnostic_hsv(reconstructed);
                let direct = std::array::from_fn(|i| sample(nl, i, 8));
                let direct_hsv = diagnostic_hsv(direct);
                for i in 0..3 {
                    roundtrip_max = roundtrip_max.max((forward[i] - hsv[i]).abs());
                    channel_max = channel_max.max((reconstructed[i] - direct[i]).abs());
                }
                hue_max = hue_max.max((direct_hsv[0] - hsv[0]).abs());
            }
        }
        if source["precision"] == "DOUBLE_NON_LINEAR" {
            let layer = &source["before"]["layers"][0];
            assert_eq!(layer["byte_order"], "little");
            let native = read("states", &layer["native"]);
            let nl = read("working", &model["input_formats"]["R'G'B'A float"]);
            let gray = source["model"] == "GRAY";
            let count = if gray { 2 } else { 4 };
            for (native, nl) in native.chunks_exact(count * 8).zip(nl.as_chunks::<16>().0.iter()) {
                for i in 0..3 {
                    let actual_cast = f64::from(sample(native, if gray { 0 } else { i }, 8) as f32);
                    native_double_rounding_max = native_double_rounding_max.max((actual_cast - sample(nl, i, 4)).abs());
                }
            }
        }
    }
    assert!(roundtrip_max <= 1e-10, "{roundtrip_max}");
    assert!(channel_max > 1e-7);
    assert!(hue_max > 0.006);
    assert!(native_double_rounding_max > 1e-7);
    println!(
        "900 HSV inverse/forward error{roundtrip_max}, requested-NL channel gap{channel_max}, near-neutral hue gap{hue_max}, nativeDouble→requestedNLFloat is not a cast gap{native_double_rounding_max}; source converter remains unqualified"
    );
}
