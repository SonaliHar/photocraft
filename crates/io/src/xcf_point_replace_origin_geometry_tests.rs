//! UNRUN synthetic origin-policy controls over13 pinned source roles. No larger producer/math claim.
use super::*;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
pub(super) const INPUTS: [(&str, &str, PointReplaceFamily); 13] = [
    (
        "corpus/xcf/full/point-absolute-evidence/absolute-rgb-float_linear.small.source.xcf",
        "58ed83cb4285068bf0335d9deb5892572ebe5eb44bba7d47d495122381cee0cc",
        PointReplaceFamily::AbsoluteLinear,
    ),
    (
        "corpus/xcf/full/channel-mixer-precision-evidence/rgb.float_non_linear.source.xcf",
        "60d42ae0f8d677550e6b7659939220cbcdaf75b30ed933fc1d069e5859826c91",
        PointReplaceFamily::ChannelMixer,
    ),
    (
        "corpus/xcf/full/mono-mixer-signed-precision-evidence/rgb.float_non_linear.source.xcf",
        "18ea6916f34e8533c29a5803d217c5d1ca66ab1dc8f091e02bfe3b067653144f",
        PointReplaceFamily::MonoMixerSigned,
    ),
    (
        "corpus/xcf/full/point-color-overlay-evidence/rgb.float_non_linear.srgb0.source.xcf",
        "22c24f13c38da0c1d41ad8089852fcf44603a0bbb1a420ffc1afa99792da62b5",
        PointReplaceFamily::ColorOverlay,
    ),
    (
        "corpus/xcf/full/point-component-extract-evidence/enabled/rgb.float_non_linear.rgb-r.invert0.linear0.xcf",
        "0548c336d209e6616aed151284b8e6aeddc2d8c1fa7de7a50b459137536eccc2",
        PointReplaceFamily::ComponentExtract,
    ),
    (
        "corpus/xcf/full/point-component-extract-cmyk-evidence/states/rgb.float_non_linear.cmyk-c.source.xcf",
        "d7b6cce837e8af0ab01e9830f5fac3c2227626e98cc436eedccbefd3daac5709",
        PointReplaceFamily::ComponentExtractCmyk,
    ),
    (
        "corpus/xcf/full/point-component-extract-ycbcr-evidence/states/rgb.float_non_linear.ycbcr-y.source.xcf",
        "3fad22666d3243feaae37d11cb0a69bde7d996eda95ba7f0b1fcf4969c67089d",
        PointReplaceFamily::ComponentExtractYcbcr,
    ),
    (
        "corpus/xcf/full/point-alpha-clip-evidence/rgb-float_non_linear.source.xcf",
        "4bb21ca78a84e2e51748804a44b07a22ce09eeba3da7ad71fc30b32a1107755a",
        PointReplaceFamily::AlphaClip,
    ),
    (
        "corpus/xcf/full/point-alpha-clip-value-domain-nonlinear-controls/rgb-float_non_linear.unit.ordinary.xcf",
        "e2a2a4bc7008cc3f485f29c4aee58e4702cf72d240f9f7a54d1da6056dc9e1c1",
        PointReplaceFamily::AlphaClipFinite,
    ),
    (
        "corpus/xcf/full/point-absolute-evidence/absolute-gray-double_linear.small.source.xcf",
        "fb896b82562e1d257e2a1d9d4ec9cf2b1d5210a6b827f7b84f2453a3e2d123ba",
        PointReplaceFamily::AbsoluteLinear,
    ),
    (
        "corpus/xcf/full/channel-mixer-precision-evidence/gray.double_perceptual.source.xcf",
        "87adb325e12611b19165c50938d3fab0212a23a6aae94d7d2a446e97df2e79d8",
        PointReplaceFamily::ChannelMixer,
    ),
    (
        "corpus/xcf/full/point-color-overlay-evidence/gray.u16_perceptual.srgb0.source.xcf",
        "fc557ae7bb2615c210b8f384640a25105ae0bec9675455a0d400268fa5f362f3",
        PointReplaceFamily::ColorOverlay,
    ),
    (
        "corpus/xcf/full/point-alpha-clip-evidence/gray-u16_non_linear.source.xcf",
        "ea863761720ae216805eafc326b651ce7dcdce76c8c463ddaf141ef472e22129",
        PointReplaceFamily::AlphaClip,
    ),
];
fn original(index: usize) -> XcfFile {
    let (path, sha, _) = INPUTS[index];
    let b = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)).unwrap();
    assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), sha);
    photocraft_xcf::decode(&b, &Default::default()).unwrap()
}
pub(super) fn file(index: usize, w: u32, h: u32) -> XcfFile {
    let mut f = original(index);
    f.width = w;
    f.height = h;
    let d = &mut f.layers[0];
    d.width = w;
    d.height = h;
    let p = d.plane.as_mut().unwrap();
    let old = p.bytes.clone();
    let stride = p.channels * p.sample.source_bytes();
    p.width = w;
    p.height = h;
    p.bytes = old.iter().copied().cycle().take((w as usize) * (h as usize) * stride).collect();
    let m = d.effects[0].mask.as_mut().unwrap();
    m.width = w;
    m.height = h;
    let p = m.plane.as_mut().unwrap();
    p.width = w;
    p.height = h;
    p.bytes = vec![0; (w as usize) * (h as usize) * p.sample.source_bytes()];
    f
}
pub(super) fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
pub(super) fn expected(i: usize) -> Option<NeighborhoodDomain> {
    Some(NeighborhoodDomain::PointReplace(INPUTS[i].2))
}
#[test]
#[ignore = "UNRUN: thirteen pinned roles plus synthetic1x1/257x259/513x2 fullcanvasorigin, same nativepatterns/parameters/raw and existing disabled-source policy"]
fn checked_shapes_and_existing_roles() {
    for i in 0..INPUTS.len() {
        assert!(classify(&original(i), &Interrupt::NONE, Some(&budget())).unwrap() == expected(i));
        for (w, h) in [(1, 1), (257, 259), (513, 2)] {
            let mut f = file(i, w, h);
            let native = f.layers[0].plane.as_ref().unwrap().bytes.clone();
            let raw = format!("{:?}", f.layers[0].effects[0].state);
            let args = format!("{:?}", f.layers[0].effects[0].arguments);
            for enabled in [true, false] {
                f.layers[0].effects[0].state.enabled = Some(enabled);
                assert!(classify(&f, &Interrupt::NONE, Some(&budget())).unwrap() == expected(i), "{i}/{w}/{h}/{enabled}");
            }
            f.layers[0].effects[0].state.enabled = Some(true);
            assert_eq!(format!("{:?}", f.layers[0].effects[0].arguments), args);
            assert_eq!(format!("{:?}", f.layers[0].effects[0].state), raw);
            assert_eq!(f.layers[0].plane.as_ref().unwrap().bytes, native);
        }
    }
}
#[test]
#[ignore = "UNRUN: malformed origin/side/area/native/physicalmask bytezero/profile/outer-policy controls fail same family finally"]
fn unchanged_source_refusals() {
    for (i, case_row) in INPUTS.iter().enumerate() {
        for bad in 0..14 {
            let mut f = file(i, 257, 2);
            match bad {
                0 => f.width = 0,
                1 => f.height = 262145,
                2 => {
                    f.width = 262144;
                    f.height = 262144;
                }
                3 => f.layers[0].offset = (-1, 0),
                4 => f.layers[0].width = 256,
                5 => f.layers[0].effects[0].mask.as_mut().unwrap().width = 256,
                6 => {
                    f.layers[0].plane.as_mut().unwrap().bytes.pop();
                }
                7 => {
                    *f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes.last_mut().unwrap() = 1;
                }
                8 => f.icc = vec![0; 640],
                9 => f.layers[0].effects[0].state.enabled = None,
                10 => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                11 => f.layers[0].opacity = 0.5,
                12 => f.layers[0].effects[0].state.mode = Some(28),
                _ => f.layers[0].effects[0].state.region = Some(1),
            }
            if bad <= 8 {
                assert!(classify_family_hint(&f) == Some(case_row.2));
            }
            assert!(classify(&f, &Interrupt::NONE, Some(&budget())).unwrap().is_none(), "{i}/{bad}");
        }
    }
    // Existing BYTE-zero scope intentionally refuses signed-zero physical masks.
    for i in 0..9 {
        let mut f = file(i, 257, 2);
        let p = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
        p.bytes[..4].copy_from_slice(&(-0.0f32).to_ne_bytes());
        assert!(classify(&f, &Interrupt::NONE, Some(&budget())).unwrap().is_none());
    }
}
#[test]
#[ignore = "UNRUN: prepared source CMS once-per-qualifier plus per-pixel conversion prepay remains cumulative, sourcecancel/work failures publish nothing"]
fn actual_source_work_and_cancellation() {
    for i in 0..INPUTS.len() {
        let f = file(i, 65, 1);
        let b = budget();
        assert!(classify(&f, &Interrupt::NONE, Some(&b)).unwrap() == expected(i));
        let space = if f.model == Model::Gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
        let required =
            photocraft_cms::xcf_color::prepared_implicit_transfer::requirements_with_control(space, photocraft_cms::control::Limits::default(), &|| false)
                .unwrap();
        assert!(
            b.used(photocraft_xcf::BudgetKind::Work)
                >= photocraft_cms::xcf_color::prepared_implicit_transfer::planning_work_ceiling()
                    + required.preparation_work_units
                    + 65 * (required.conversion_work_units + 128)
        );
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
        let lower = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
        assert!(classify(&f, &Interrupt::NONE, Some(&lower)).is_err());
        let stop = || true;
        assert!(matches!(classify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
    }
}
#[test]
#[ignore = "UNRUN: exact old epoch selection remains final, ComponentGeometry and MonoOriginal are not recaptured by expanded source roles"]
fn other_epoch_priority_and_native_blocks() {
    for i in 0..INPUTS.len() {
        let f = file(i, 257, 2);
        let p = f.layers[0].plane.as_ref().unwrap();
        let stride = p.channels * p.sample.source_bytes();
        let mut seen = 0u64;
        p.visit_native(&Interrupt::NONE, &mut |b| {
            assert!(!b.is_empty() && b.len() <= 64 * stride && b.len() % stride == 0);
            seen += b.len() as u64;
            Ok(())
        })
        .unwrap();
        assert_eq!(Some(seen), p.native_byte_len());
    }
    let mut c = file(4, 257, 2);
    let s = &mut c.layers[0].effects[0].state;
    s.blend_space = Some(87327313);
    s.composite_space = Some(87327313);
    s.composite_mode = Some(87327313);
    assert!(classify_family_hint(&c) == Some(PointReplaceFamily::ComponentExtractGeometry));
    assert!(classify(&c, &Interrupt::NONE, Some(&budget())).unwrap().is_none());
    let mut m = file(2, 257, 2);
    let s = &mut m.layers[0].effects[0].state;
    s.blend_space = Some(-261651887);
    s.composite_space = Some(-261651887);
    s.composite_mode = Some(-261651887);
    assert!(classify_family_hint(&m).is_none());
    assert!(classify(&m, &Interrupt::NONE, Some(&budget())).unwrap().is_none());
}
