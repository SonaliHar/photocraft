//! UNRUN assigned Median source controls; eager decoded/native buffers are external harness data.
//! Filename unit is not a native domain claim: five slight HDRRGB words remain exact.
use super::*;
use sha2::{Digest, Sha256};
fn bytes(edit: bool) -> Vec<u8> {
    let (name, pin) = if edit {
        ("assigned-unit-f32.edit.xcf", "c41d90e4c9ef73874c03ba52a80f173b3b558a1740971aff4c84a4abb49fc7cd")
    } else {
        ("assigned-unit-f32.source.xcf", "60540a9d83f0c1998ab4bbf6796028b456698973026d942ae880b050412dad94")
    };
    let b = std::fs::read(std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("median-raster-color-evidence").join(name))
        .unwrap();
    assert_eq!(b.len(), 4998);
    assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), pin);
    b
}
fn source(edit: bool) -> XcfFile {
    photocraft_xcf::decode(&bytes(edit), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
#[test]
#[ignore = "UNRUN: actual2 source/edit retain five HDRRGB words; exact original profile/native/mask/arg source policy"]
fn actual_assigned_source_domain() {
    for edit in [false, true] {
        let f = source(edit);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(f.icc.len(), 640);
        let d = &f.layers[0];
        let e = &d.effects[0];
        assert_eq!(e.arguments.len(), 6);
        assert!(matches!(e.arguments.iter().find(|a|a.name=="radius").unwrap().value,ArgumentValue::Integer(v)if v==if edit{4}else{3}));
        assert!(matches!(e.arguments.iter().find(|a|a.name=="high-precision").unwrap().value,ArgumentValue::Boolean(v)if v==edit));
        for name in ["percentile", "alpha-percentile"] {
            assert!(matches!(e.arguments.iter().find(|a| a.name == name).unwrap().value, ArgumentValue::Float32Bits(1112014848)));
        }
        let mut outside = 0;
        for p in d.plane.as_ref().unwrap().bytes.chunks_exact(16) {
            for w in p[..12].chunks_exact(4) {
                let v = f32::from_ne_bytes(w.try_into().unwrap());
                outside += usize::from(!(0.0..=1.0).contains(&v));
            }
        }
        assert_eq!(outside, 5);
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
        let mut f = source(false);
        match case {
            0 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
            1 => f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.25f32.to_ne_bytes()),
            2 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.5f32.to_ne_bytes()),
            3 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "radius").unwrap().value = ArgumentValue::Integer(6),
            4 => f.icc.clear(),
            5 => f.layers[0].offset = (1, 0),
            6 => f.layers[0].effects[0].state.enabled = Some(false),
            7 => f.sample = Sample::F64,
            8 => {
                f.layers[0].plane.as_mut().unwrap().bytes.pop();
            }
            9 => f.layers[0].effects[0].state.composite_mode = Some(0),
            10 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::INFINITY.to_ne_bytes()),
            11 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "high-precision").unwrap().value = ArgumentValue::Boolean(true),
            12 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "neighborhood").unwrap().value = ArgumentValue::Enum(0),
            13 => f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "percentile").unwrap().value = ArgumentValue::Float32Bits(49f32.to_bits()),
            14 => {
                let a = f.layers[0].effects[0].arguments[0].clone();
                f.layers[0].effects[0].arguments[1] = a;
            }
            _ => {
                f.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "alpha-percentile").unwrap().value = ArgumentValue::Float64Bits(50f64.to_bits())
            }
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "case{case}");
    }
    let f = source(false);
    assert!(!super::super::assigned_box::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    assert!(!super::super::finite(&f, "gegl:median-blur", Model::Rgb, &Interrupt::NONE, Some(&budget())).unwrap());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
}
