//! UNRUN assigned Box source controls; eager decoded/native buffers are external harness data.
//! Filename unit is not a native domain claim: five slight HDRRGB words remain exact.
use super::*;
use sha2::{Digest, Sha256};
fn bytes(edit: bool) -> Vec<u8> {
    let (name, pin) = if edit {
        ("assigned-unit-f32.edit.xcf", "0720b4beee6ebca7484dc8ef3f152b0c22f1430cf32c31811db5a011ccec22aa")
    } else {
        ("assigned-unit-f32.source.xcf", "e4f31ba998692bb7f57ebc82433a4c8d90a7aeaeb2d67a9276079bb594d4cec8")
    };
    let b =
        std::fs::read(std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("box-raster-color-evidence").join(name)).unwrap();
    assert_eq!(b.len(), 4817);
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
        assert!(matches!(e.arguments[0].value,ArgumentValue::Integer(v)if v==if edit{5}else{4}));
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
    for case in 0..11 {
        let mut f = source(false);
        match case {
            0 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
            1 => f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.25f32.to_ne_bytes()),
            2 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.5f32.to_ne_bytes()),
            3 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(6),
            4 => f.icc.clear(),
            5 => f.layers[0].offset = (1, 0),
            6 => f.layers[0].effects[0].state.enabled = Some(false),
            7 => f.sample = Sample::F64,
            8 => {
                f.layers[0].plane.as_mut().unwrap().bytes.pop();
            }
            9 => f.layers[0].effects[0].state.composite_mode = Some(0),
            _ => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::INFINITY.to_ne_bytes()),
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "case{case}");
    }
    let f = source(false);
    assert!(!super::super::finite(&f, "gegl:box-blur", Model::Rgb, &Interrupt::NONE, Some(&budget())).unwrap());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
}
