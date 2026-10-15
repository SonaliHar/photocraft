//! UNRUN genuine source predicates; decoded/source buffers are external harness data.
use super::*;
use sha2::{Digest, Sha256};
fn source(edit: bool) -> XcfFile {
    let (name, pin, len) = if edit {
        ("composition-matrix/group-28-group-mask.edit.xcf", "bf9f61f61cedbfdd6e3e3c25cb5066e4deef0c46eef16629d6546cfd1018706b", 10514)
    } else {
        ("composition-matrix/group-28-group-mask.xcf", "740064c2b008a9785801b9de4425f685dc2618a7fcc938d37728723dd10ccc49", 10313)
    };
    let bytes = std::fs::read(std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(name)).unwrap();
    assert_eq!(bytes.len(), len);
    assert_eq!(Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(), pin);
    photocraft_xcf::decode(&bytes, &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
#[test]
#[ignore = "UNRUN: authentic two mode28 records preserve exact legacy group source metadata and native words"]
fn authentic_group_geometry_and_numeric_zero_are_the_old_intersection() {
    for edit in [false, true] {
        let mut f = source(edit);
        assert!(qualified_gaussian_group(&f));
        assert!(hint(&f));
        assert!(qualify(&f, &Interrupt::NONE, &budget()).unwrap());
        let mask = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
        mask.bytes[..4].copy_from_slice(&(-0.0f32).to_ne_bytes());
        assert!(qualify(&f, &Interrupt::NONE, &budget()).unwrap());
        // Existing code-qualified disabled tuple, not new independently observed producer evidence.
        let st = &mut f.layers[0].effects[0].state;
        st.enabled = Some(false);
        st.blend_space = Some(-1474592175);
        st.composite_space = Some(-1474592175);
        st.composite_mode = Some(-1474592175);
        assert!(qualify(&f, &Interrupt::NONE, &budget()).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: genuine source mutations refuse mode61, geometry/schema/profile/native/mask faults before any mapping retry"]
fn strict_group_refusals_and_cumulative_control() {
    for case in 0..9 {
        let mut f = source(false);
        match case {
            0 => f.layers[0].mode = 61,
            1 => f.icc = vec![0; 132],
            2 => f.layers[0].offset = (1, 0),
            3 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float32Bits(2.0f32.to_bits()),
            4 => f.layers[1].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
            5 => {
                f.layers[3].plane.as_mut().unwrap().bytes.pop();
            }
            6 => f.layers[0].effects[0].mask = None,
            7 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.5f32.to_ne_bytes()),
            _ => f.layers[0].effects[0].state.composite_mode = Some(0),
        }
        assert!(!qualify(&f, &Interrupt::NONE, &budget()).unwrap(), "case {case}");
    }
    let f = source(false);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, &b).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), &budget()), Err(IoError::Cancelled)));
}
