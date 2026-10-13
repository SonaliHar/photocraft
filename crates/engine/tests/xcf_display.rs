//! Independent GIMP-exported PNG display references, both converted through their ICC to sRGB.
#![cfg(feature = "corpus")]
use photocraft_cms::{Builtin, Intent, Profile, Transform};
use std::path::PathBuf;
fn rgb_profile(p: Profile) -> Profile {
    if p.color_space == photocraft_cms::ColorSpace::Gray { p.gray_as_rgb().unwrap() } else { p }
}
#[test]
fn gimp_projection_matches_imported_cpu_display() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).expect("missing XCF corpus: run cargo xtask corpus --xcf")).unwrap();
    let mut pass = 0;
    let mut failures = Vec::new();
    let mut worst = 0.0f32;
    for case in manifest["cases"].as_array().unwrap() {
        if case["expected_status"] != "supported" {
            continue;
        }
        let name = case["input"].as_str().unwrap();
        let imported = match photocraft_io::import(name, &std::fs::read(root.join(name)).unwrap()) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{name}: import {e}"));
                continue;
            }
        };
        let d = imported.document;
        let before = d.clone();
        let buffer = photocraft_compose::render(&d, d.bounds()).unwrap();
        let reference = photocraft_codecs::decode(&std::fs::read(root.join(case["reference"].as_str().unwrap())).unwrap()).unwrap();
        let rp = reference.icc.as_ref().map(|p| Profile::parse(p).unwrap()).unwrap_or_else(|| Builtin::Srgb.profile().clone());
        let rt = Transform::new(&rgb_profile(rp), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
        let actual_profile = photocraft_engine::color_cmds::composite_profile(&d);
        let at = Transform::new(&actual_profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
        let reference_pixels = reference.to_rgba_f32();
        assert_eq!(reference_pixels.len(), buffer.px.len() * 4);
        let mut max = 0.0f32;
        let mut detail = String::new();
        for (a, r) in buffer.px.iter().zip(reference_pixels.as_chunks::<4>().0.iter()) {
            let mut ac = [0.0; 3];
            let mut rc = [0.0; 3];
            at.eval(&a[..3], &mut ac);
            rt.eval(&r[..3], &mut rc);
            if ac.iter().zip(rc).any(|(a, r)| (a - r).abs() > max) {
                detail = format!("actual={ac:?} reference={rc:?} raw={a:?}");
            }
            for (a, r) in ac.iter().zip(rc) {
                max = max.max((a - r).abs());
            }
            max = max.max((a[3] - r[3]).abs());
        }
        assert_eq!(d, before, "render rewrote original samples");
        worst = worst.max(max);
        println!("{name}: max display difference {:.4}/255", max * 255.0);
        if max <= 2.0 / 255.0 {
            pass += 1;
        } else {
            failures.push(format!("{name}: max {} /255 {detail}", max * 255.0));
        }
    }
    println!("GIMP display: {pass} passed; worst {:.4}/255", worst * 255.0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(pass >= 58, "at least 58 positive GIMP display references required; never lower the measured floor");
}
