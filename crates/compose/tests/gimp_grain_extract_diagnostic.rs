#![cfg(feature = "corpus")]
//! Explicit diagnostic only: no Grain Extract importer admission or tolerance change.
use photocraft_cms::{
    Builtin, Profile,
    xcf_color::{Encoding, convert_encoding_f64},
};
use photocraft_compose::gimp_composite::WorkingSpace;
use photocraft_doc::{
    SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn read(root: &Path, name: &str, hashes: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(name)).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, hashes[name].as_str().unwrap());
    bytes
}
fn pixel(raw: &[u8], w: i64, h: i64, x: i64, y: i64) -> [f64; 4] {
    if x < 0 || y < 0 || x >= w || y >= h {
        return [0.0; 4];
    }
    let pos = ((y * w + x) * 16) as usize;
    std::array::from_fn(|c| f64::from(f32::from_le_bytes(raw[pos + c * 4..pos + c * 4 + 4].try_into().unwrap())))
}
#[test]
#[ignore = "retained one-code Grain Extract precision diagnostic, not admission"]
fn grain_extract_profile_precision_diagnostic() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("gimp_basic_blends_hashes.json")).unwrap();
    for phase in ["source", "edit"] {
        let state: Value = serde_json::from_slice(&read(&root, &format!("mode-grain-extract.fresh-{phase}_state.json"), &hashes)).unwrap();
        let reference = photocraft_codecs::decode(&read(&root, &format!("mode-grain-extract.fresh-{phase}.ref.png"), &hashes)).unwrap();
        let layers = state["layers"].as_array().unwrap();
        let raws: Vec<_> = layers
            .iter()
            .map(|l| {
                let raw = fs::read(root.join(l["native"]["file"].as_str().unwrap())).unwrap();
                let hash: String = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect();
                assert_eq!(hash, l["native"]["sha256"].as_str().unwrap());
                raw
            })
            .collect();
        for (label, profile) in [
            ("exact analytic implicit", Builtin::Srgb.profile().clone()),
            ("native serialized builtin", Profile::parse(&Builtin::Srgb.profile().to_bytes()).unwrap()),
            ("PNG embedded", Profile::parse(reference.icc.as_ref().unwrap()).unwrap()),
        ] {
            let working = WorkingSpace::new(&profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
            let mut worst = 0.0_f64;
            let mut location = (0, 0, 0);
            let mut diagnostic = [0.0; 4];
            for y in 0..12 {
                for x in 0..16 {
                    let b = working.normal([0.0; 4], pixel(&raws[1], 16, 12, x, y), 1.0, GimpOperator::Union).unwrap();
                    let s = pixel(&raws[0], 10, 9, x - layers[0]["offset"][0].as_i64().unwrap(), y - layers[0]["offset"][1].as_i64().unwrap());
                    let mut bp = [0.0; 3];
                    let mut sp = [0.0; 3];
                    convert_encoding_f64(&profile, Encoding::NonlinearProfile, Encoding::Perceptual, &b[..3], &mut bp).unwrap();
                    convert_encoding_f64(&profile, Encoding::NonlinearProfile, Encoding::Perceptual, &s[..3], &mut sp).unwrap();
                    let overlap: [f64; 3] = std::array::from_fn(|c| bp[c] - sp[c] + 0.5);
                    let mut stored = [0.0; 3];
                    convert_encoding_f64(&profile, Encoding::Perceptual, Encoding::NonlinearProfile, &overlap, &mut stored).unwrap();
                    let out = working
                        .normal(
                            b,
                            [stored[0], stored[1], stored[2], s[3]],
                            (layers[0]["opacity"].as_f64().unwrap() / 100.0) as f32 as f64,
                            GimpOperator::ClipToBackdrop,
                        )
                        .unwrap();
                    for c in 0..4 {
                        let error = (out[c].clamp(0.0, 1.0) - f64::from(reference.get(x as u32, y as u32, c))).abs();
                        if error > worst {
                            worst = error;
                            location = (x, y, c);
                            diagnostic = [b[c], s[c], out[c], f64::from(reference.get(x as u32, y as u32, c))];
                        }
                    }
                }
            }
            println!("Grain Extract {phase} {label}: {:.9}/255 at {location:?}; b/s/out/ref {diagnostic:?}", worst * 255.0);
            if label == "exact analytic implicit" {
                assert!(worst <= 1.0 / 65535.0);
            }
            if label == "native serialized builtin" && phase == "source" {
                assert!(worst > 1.0 / 65535.0);
            }
        }
    }
}
