#![cfg(feature = "corpus")]
//! Independent unfiltered contrast: PNG export binding must not be inferred from ICC presence.
use photocraft_cms::{
    ColorSpace, Intent, Profile, Transform,
    xcf_color::{Encoding, effective_profile},
};
use std::path::PathBuf;
#[test]
fn original_unfiltered_perceptual_png_matches_assigned_trc_conversion() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("profile-context-matrix");
    let id = "rgb-u16-perceptual-wide-parametric-discard";
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{id}.source.state.json"))).unwrap()).unwrap();
    assert!(state["layers"][0]["filters"].as_array().unwrap().is_empty());
    assert_eq!(state["precision"], "GIMP_PRECISION_U16_PERCEPTUAL");
    let native = std::fs::read(root.join(format!("{id}.source.0.native.bin"))).unwrap();
    let original = std::fs::read(root.join(state["assigned_profile"]["file"].as_str().unwrap())).unwrap();
    let source = effective_profile(Some(&original), ColorSpace::Rgb, Encoding::Perceptual).unwrap();
    let reference = photocraft_codecs::decode(&std::fs::read(root.join(format!("{id}.ref.png"))).unwrap()).unwrap();
    assert_eq!(reference.icc.as_ref().unwrap(), &original);
    let target = Profile::parse(&original).unwrap();
    let convert = Transform::new(&source, &target, Intent::RelativeColorimetric, false).unwrap();
    let reference = reference.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U16);
    let mut direct = 0.0_f64;
    let mut transformed = 0.0_f64;
    for (raw, want) in native.as_chunks::<8>().0.iter().zip(reference.data().as_chunks::<8>().0.iter()) {
        let a: Vec<f64> = raw.as_chunks::<2>().0.iter().map(|v| f64::from(u16::from_ne_bytes(*v)) / 65535.0).collect();
        let b: Vec<f64> = want.as_chunks::<2>().0.iter().map(|v| f64::from(u16::from_ne_bytes(*v)) / 65535.0).collect();
        let mut result = [0.0; 3];
        convert.try_eval_f64(&a[..3], &mut result).unwrap();
        for c in 0..3 {
            let native = if a[3] == 0.0 { 0.0 } else { a[c] };
            let converted = if a[3] == 0.0 { 0.0 } else { result[c] };
            direct = direct.max((native - b[c]).abs() * 255.0);
            transformed = transformed.max((converted - b[c]).abs() * 255.0);
        }
    }
    println!("independent unfiltered Perceptual PNG: raw-code max255={direct}, assigned-TRC-conversion max255={transformed}");
    assert!(transformed <= 2.0);
    assert!(direct > 2.0, "preserve measured export contrast, not false raw-code export qualification");
}
