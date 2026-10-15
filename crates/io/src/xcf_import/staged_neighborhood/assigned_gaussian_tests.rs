//! UNRUN assigned Gaussian source controls; eager decoded/native buffers are external harness data.
//! Named-unit has five mild HDR words; HDR captures carry their own exact native pin.
//! Missing loose producer ICC roles stay HOLD; no numerical profile equivalence claim.
use super::*;
use sha2::{Digest, Sha256};
const CASES: [(&str, &str, usize, &str); 4] = [
    (
        "assigned-unit-f32.source.xcf",
        "832fdd77cdd8f9e9ac1c9a2a7de1ce52128c91f4f17f4018083aca65faee8f6a",
        4962,
        "3102cfa5ba9eb6cf718b767257d474fc865e0b5fa93e8b3622adeebd7fefaadf",
    ),
    (
        "assigned-unit-f32.edit.xcf",
        "a79bfce1ec95b36ea7d4ccb6e39e705fb5872c844c8e41c7f15b14bbf1c9a2cd",
        4962,
        "3102cfa5ba9eb6cf718b767257d474fc865e0b5fa93e8b3622adeebd7fefaadf",
    ),
    (
        "assigned-hdr-f32.source.xcf",
        "ce2b5b6339a7edd4b657b1cb85100673c33aa1f13fb25acbaf4c7929dc4ca1be",
        4864,
        "77cacaa145ed0a5a07fc80c71fb093483c5eb684df0433a5aa01756dc275d302",
    ),
    (
        "assigned-hdr-f32.edit.xcf",
        "aade08703fbfde31c57bb3d11115c4e82c82ad05fe68df9c13acd68fe43e8be7",
        4864,
        "77cacaa145ed0a5a07fc80c71fb093483c5eb684df0433a5aa01756dc275d302",
    ),
];
fn source(case: usize) -> XcfFile {
    let b = std::fs::read(
        std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("gaussian-raster-color-evidence").join(CASES[case].0),
    )
    .unwrap();
    assert_eq!(b.len(), CASES[case].2);
    assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), CASES[case].1);
    photocraft_xcf::decode(&b, &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
#[test]
#[ignore = "UNRUN: actual4 named-unit/HDR source/edit preserve finite HDRRGB words; exact original profile/native/mask/arg source policy"]
fn actual_assigned_source_domain() {
    for case in 0..4 {
        let edit = case % 2 == 1;
        let f = source(case);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(f.icc.len(), 640);
        let d = &f.layers[0];
        let e = &d.effects[0];
        assert_eq!(e.arguments.len(), 5);
        assert!(
            matches!(e.arguments.iter().find(|a|a.name=="std-dev-x").unwrap().value,ArgumentValue::Float32Bits(v)if v==if edit{3.75f32.to_bits()}else{2.25f32.to_bits()})
        );
        assert!(
            matches!(e.arguments.iter().find(|a|a.name=="std-dev-y").unwrap().value,ArgumentValue::Float32Bits(v)if v==if edit{0.8f32.to_bits()}else{1.3f32.to_bits()})
        );
        assert!(matches!(e.arguments.iter().find(|a|a.name=="filter").unwrap().value,ArgumentValue::Enum(v)if v==if edit{1}else{0}));
        let raw: Vec<u8> = d.plane.as_ref().unwrap().bytes.chunks_exact(4).flat_map(|w| u32::from_ne_bytes(w.try_into().unwrap()).to_le_bytes()).collect();
        assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), CASES[case].3);
        let mut outside = 0;
        for p in d.plane.as_ref().unwrap().bytes.chunks_exact(16) {
            for w in p[..12].chunks_exact(4) {
                let v = f32::from_ne_bytes(w.try_into().unwrap());
                outside += usize::from(!(0.0..=1.0).contains(&v));
            }
        }
        if case < 2 {
            assert_eq!(outside, 5);
        } else {
            assert!(outside > 0);
        }
        // A finite RGB edit is valid domain data; alpha remains separately unit.
        let mut changed = f.clone();
        changed.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes());
        assert!(qualify(&changed, &Interrupt::NONE, Some(&budget())).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: assigned-only source refusals, full-native/nonfinite/unitAlpha/empty mask, old implicit and cumulative controls"]
fn source_failures_and_control() {
    for case in 0..16 {
        let mut f = source(0);
        match case {
            0 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
            1 => f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.25f32.to_ne_bytes()),
            2 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.5f32.to_ne_bytes()),
            3 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "std-dev-x").unwrap().value = ArgumentValue::Float32Bits(6f32.to_bits()),
            4 => f.icc.clear(),
            5 => f.layers[0].offset = (1, 0),
            6 => f.layers[0].effects[0].state.enabled = Some(false),
            7 => f.sample = Sample::F64,
            8 => {
                f.layers[0].plane.as_mut().unwrap().bytes.pop();
            }
            9 => f.layers[0].effects[0].state.composite_mode = Some(0),
            10 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::INFINITY.to_ne_bytes()),
            11 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "filter").unwrap().value = ArgumentValue::Enum(1),
            12 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "abyss-policy").unwrap().value = ArgumentValue::Enum(0),
            13 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "clip-extent").unwrap().value = ArgumentValue::Boolean(false),
            14 => {
                let a = f.layers[0].effects[0].arguments[0].clone();
                f.layers[0].effects[0].arguments[1] = a;
            }
            _ => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "std-dev-y").unwrap().value = ArgumentValue::Float64Bits(1.3f64.to_bits()),
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "case{case}");
    }
    let f = source(0);
    assert!(!super::super::assigned_median::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    assert!(!super::super::assigned_box::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    assert!(!super::super::finite(&f, "gegl:gaussian-blur", Model::Rgb, &Interrupt::NONE, Some(&budget())).unwrap());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
}
