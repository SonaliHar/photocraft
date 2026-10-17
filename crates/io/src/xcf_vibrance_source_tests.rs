//! UNRUN authored RGB Vibrance source lifecycle; FMA/Babl numerical schedule remains HOLD.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, usize, &str, bool, [u32; 2]); 12] = [
    (
        "f32-author-v1/outputs/f32-neutral-rounding.source.xcf",
        1955,
        "4e239a92719be5b66fb040b9d815295e5fb2cb9744ff55c8cc18980e80ec311c",
        false,
        [0x0, 0x3f800000],
    ),
    (
        "f32-author-v1/outputs/f32-neutral-rounding.edit.xcf",
        1955,
        "64faa19282abb91fd24d3eaadd73c8c51ac57c192ce092dae767938d060b8272",
        false,
        [0x41c80000, 0x3fa00000],
    ),
    (
        "f32-author-v1/outputs/f32-sign-desaturation.source.xcf",
        1955,
        "088fd5b88a1ceb2a1de231bb0cdecbc3f5ab33f8b30af6bc82b82191cc380a27",
        false,
        [0xc2480000, 0x3f800000],
    ),
    (
        "f32-author-v1/outputs/f32-sign-desaturation.edit.xcf",
        1955,
        "95737876c2353188b8ac8e47361b46931343e6560f303ef3560be0ab0cd8bac6",
        false,
        [0x42480000, 0x3f000000],
    ),
    (
        "f32-author-v1/outputs/f32-bounded-extremes.source.xcf",
        1955,
        "581627573b92a776bc9dff4f82b8a88d7cbd50df6b4382332cd78832a024e5b5",
        false,
        [0xc2c80000, 0x0],
    ),
    (
        "f32-author-v1/outputs/f32-bounded-extremes.edit.xcf",
        1955,
        "0b19695189e32b59dfad2ada8e3ca54bb70863cf8cd6f6bcb29fcefa1651ca48",
        false,
        [0x42c80000, 0x41200000],
    ),
    (
        "f64-author-v1/outputs/f64-neutral-rounding.source.xcf",
        2083,
        "120c3e4bf2bb0655b9af3637f6bd9eccc5ca272bf6a960ce86e86a22489fc156",
        true,
        [0x0, 0x3f800000],
    ),
    (
        "f64-author-v1/outputs/f64-neutral-rounding.edit.xcf",
        2083,
        "6ed2c0ae8cb6f7500bdf0596b7b684e71eb50155d8150ef03080033be99bbe1e",
        true,
        [0x41c80000, 0x3fa00000],
    ),
    (
        "f64-author-v1/outputs/f64-sign-desaturation.source.xcf",
        2083,
        "431932e37076996719d54ca620e4235d2dd8580c2b640a821feeb35dffee7daa",
        true,
        [0xc2480000, 0x3f800000],
    ),
    (
        "f64-author-v1/outputs/f64-sign-desaturation.edit.xcf",
        2083,
        "744f79b4826f7cf9d969e30fec4e07059e3d0c767e7980f1ce1707d82c025184",
        true,
        [0x42480000, 0x3f000000],
    ),
    (
        "f64-author-v1/outputs/f64-bounded-extremes.source.xcf",
        2083,
        "f2e64ca7e6210f4610c19caa6fc53c509754e491bdc7afc8d3e679fcd583693a",
        true,
        [0xc2c80000, 0x0],
    ),
    (
        "f64-author-v1/outputs/f64-bounded-extremes.edit.xcf",
        2083,
        "43fa256526e60c865caab7581564d7a5812787809420de6333efc414c6a64614",
        true,
        [0x42c80000, 0x41200000],
    ),
];

fn bytes(case: usize) -> Vec<u8> {
    let root = std::env::var_os("PHOTOCRAFT_VIBRANCE_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vibrance-owned-gimp-20261016"));
    let b = std::fs::read(root.join(CASES[case].0)).unwrap();
    assert_eq!(b.len(), CASES[case].1);
    assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), CASES[case].2);
    b
}
fn file(case: usize) -> XcfFile {
    photocraft_xcf::decode(&bytes(case), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 })
}
fn import(b: &[u8], q: &BackingQuota) -> Result<crate::OwnedXcfImport, IoError> {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    crate::import_owned_xcf_from_reader_in_quota("Own Vibrance.xcf", &mut Cursor::new(b), b.len() as u64, &Default::default(), &limits, q.clone(), &ctl())
}
#[test]
#[ignore = "UNRUN: twelve authored sources retain twoF32 args/order/native words, exactprofile/mask/raw andtitle custody; no math claim"]
fn actual_sources_keep_two_stored_words() {
    for (case, case_row) in CASES.iter().enumerate() {
        let f = file(case);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(parameters(&f.layers[0].effects[0]).unwrap(), case_row.4);
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = import(&bytes(case), &q).unwrap() else { panic!("Point") };
        let owner = source.document();
        owner.validate_readonly_point_storage(&ctl()).unwrap();
        let d = owner.document();
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, if case_row.3 { 750 } else { 650 });
        let x = d.xcf_state.as_ref().unwrap();
        let n = &x.filter_stacks[0].nodes[0];
        assert_eq!(n.operation.schema_epoch, photocraft_compose::VIBRANCE_EPOCH);
        assert_eq!(n.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["vibrance", "saturation"]);
        let bits = n
            .arguments
            .iter()
            .map(|a| match &a.value {
                photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(v)) => *v,
                _ => panic!("F32"),
            })
            .collect::<Vec<_>>();
        assert_eq!(bits.as_slice(), case_row.4.as_slice());
        let p = n.composition.as_ref().unwrap();
        assert_eq!(p.compatibility_epoch, n.operation.schema_epoch);
        assert!(p.resolved.is_none());
        assert_eq!((p.stored_blend_space, p.stored_composite_space, p.stored_operator), (720732753, 720732753, 720732753));
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("native") };
        let bpp = if case_row.3 { 32 } else { 16 };
        let r = photocraft_geom::Rect::new(0, 0, 8, 6);
        let pin = layer.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &ctl()).unwrap();
        let mut raw = vec![0; 48 * bpp];
        for y in 0..6 {
            for xx in 0..8 {
                let at = (y * 8 + xx) as usize * bpp;
                pin.read_pixel_native_into(xx, y, &mut raw[at..at + bpp], &ctl()).unwrap();
            }
        }
        assert_eq!(raw, f.layers[0].plane.as_ref().unwrap().bytes);
        assert_eq!(layer.operative_bounds(), r);
        assert_eq!(layer.default_bytes().len(), bpp);
        assert!(layer.default_bytes().iter().all(|v| *v == 0));
        assert_eq!(x.filter_masks[0].bounds, r);
        assert_eq!(x.filter_masks[0].surface.default_bytes().len(), bpp / 4);
        assert!(x.filter_masks[0].is_empty_selection_with(&Interrupt::NONE).unwrap());
        let title = owner.try_with_document_name("Vibrance.pcraft", &ctl()).unwrap();
        assert_eq!(title.document().xcf_state, d.xcf_state);
        assert_eq!(title.document().source_color_context, d.source_color_context);
        assert_eq!(title.document().icc_profile, d.icc_profile);
    }
}
#[test]
#[ignore = "UNRUN: source roles/geometry/activity/twoF32 names-types-ranges/rawmissing/native/mask fail closed with final family refusal"]
fn exact_source_refusals() {
    for case in [0, 6] {
        for bad in 0..18 {
            let mut f = file(case);
            match bad {
                0 => f.model = Model::Gray,
                1 => f.icc = vec![0; 640],
                2 => f.width = 9,
                3 => f.layers[0].visible = false,
                4 => f.layers[0].opacity = 0.5,
                5 => f.layers[0].effects[0].state.enabled = Some(false),
                6 => f.layers[0].effects[0].state.blend_space = None,
                7 => f.layers[0].effects[0].state.mode = Some(28),
                8 => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                9 => f.layers[0].effects[0].state.region = Some(1),
                10 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float64Bits(0),
                11 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float32Bits(100.01f32.to_bits()),
                12 => f.layers[0].effects[0].arguments[1].value = ArgumentValue::Float32Bits((-0.01f32).to_bits()),
                13 => f.layers[0].effects[0].arguments[1].name = "vibrance".into(),
                14 => {
                    f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1;
                }
                15 => {
                    if CASES[case].3 {
                        f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&1.01f64.to_ne_bytes())
                    } else {
                        f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes())
                    }
                }
                16 => {
                    f.sample = Sample::U8;
                    f.precision = 150;
                }
                _ => f.layers[0].effects[0].properties[0].tag = 99,
            };
            assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "{case}/{bad}");
            assert!(matches!(crate::xcf_point_replace_source::classify_family_hint(&f), Some(crate::xcf_filters::PointReplaceFamily::Vibrance)));
            assert!(crate::xcf_point_replace_source::classify(&f, &Interrupt::NONE, Some(&budget())).unwrap().is_none());
        }
    }
    // Actual older storage roles remain separate negative references.
    for (path, sha) in [
        ("corpus/xcf/full/operations/gegl-vibrance/activation-gegl-vibrance.edit.xcf", "d410dda4b36e15471126315bc8cbd12f59bb000bf13341500e6366ff246ea373"),
        ("corpus/xcf/full/operations/gegl-vibrance/activation-gegl-vibrance.xcf", "a6cc530fd2f03da085da7feb2951ce1e33998b0e0857c9c9d043af7804db8dab"),
        ("corpus/xcf/full/operations/gegl-vibrance/operation-gegl-vibrance.edit.xcf", "54d55be3806ebdc022ce52f58139227635f05e2a28536976bdbfedaf513105ee"),
        ("corpus/xcf/full/operations/gegl-vibrance/operation-gegl-vibrance.xcf", "fb96a323c63d4b7be1f559d0779a647ad077932e81c643ecf45e78adaea76883"),
        ("temp/sol2-vibrance-20261014/v3/packet/default.edit.xcf", "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/default.xcf", "8ca257fe0ce1b677d5a39f68d16458d2569276e51bc2e98f7674c0497c80f849"),
        ("temp/sol2-vibrance-20261014/v3/packet/disabled.edit.xcf", "1eb5041f87ce97c253b178ba775aeee4bfbd9bb7fedf39038745c0f5dc27c2fa"),
        ("temp/sol2-vibrance-20261014/v3/packet/disabled.xcf", "929aa6ae08b7e3491431d14b662b007082ea8b454b63611dd54cce8a9bc45499"),
        ("temp/sol2-vibrance-20261014/v3/packet/negative.edit.xcf", "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/negative.xcf", "7c175d599e2435f432783cdde8f49c36229d2a581b26d4963d3578672cde6ab2"),
        ("temp/sol2-vibrance-20261014/v3/packet/positive.edit.xcf", "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/positive.xcf", "5ce751cdc2f2b069753aa708ade31b17ffa68d0812183c186d0bc71e8acc9148"),
        ("temp/sol2-vibrance-20261014/v3/packet/saturation.edit.xcf", "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/saturation.xcf", "3140c9f9b6e2d163fdd262cd712cf1a7d23b799a743553e26461cc3df9f546aa"),
    ] {
        let b = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)).unwrap();
        assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), sha);
        let f = photocraft_xcf::decode(&b, &Default::default()).unwrap();
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
}
fn raw_words(mut b: Vec<u8>, values: [i32; 3]) -> Vec<u8> {
    for (tag, value) in [37u32, 36, 35].into_iter().zip(values) {
        let mut pattern = Vec::new();
        pattern.extend(tag.to_be_bytes());
        pattern.extend(4u32.to_be_bytes());
        pattern.extend([0x2a, 0xf5, 0x82, 0x51]);
        let positions = b.windows(12).enumerate().filter_map(|(i, w)| (w == pattern.as_slice()).then_some(i)).collect::<Vec<_>>();
        assert_eq!(positions.len(), 1);
        let at = positions[0] + 8;
        b[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    b
}
#[test]
#[ignore = "UNRUN: separate opaque provenance policy retains mixedMIN/0/MAX only inside exact Vibrance Replace envelope; no enum alias"]
fn opaque_words_are_custody_not_other_policy() {
    for case in [0, 6] {
        let mut f = file(case);
        let mask = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
        if CASES[case].3 {
            mask.bytes[..8].copy_from_slice(&(-0.0f64).to_ne_bytes())
        } else {
            mask.bytes[..4].copy_from_slice(&(-0.0f32).to_ne_bytes())
        };
        let raw = mask.bytes.clone();
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(f.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes, raw);
    }

    let values = [i32::MIN, 0, i32::MAX];
    for case in [0, 6] {
        let b = raw_words(bytes(case), values);
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = import(&b, &q).unwrap() else { panic!("Point") };
        let n = &source.document().document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let p = n.composition.as_ref().unwrap();
        assert_eq!((p.stored_blend_space, p.stored_composite_space, p.stored_operator), (values[0], values[1], values[2]));
        assert!(p.resolved.is_none());
        let mut f = photocraft_xcf::decode(&b, &Default::default()).unwrap();
        f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits());
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: source shared work/cancellation/native length/quota refusals retain original ownership without fallback"]
fn resource_and_length_refusals() {
    let f = file(0);
    let small = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(small, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
    let mut f = f;
    f.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    let q = quota();
    let hold = q.reserve(QuotaKind::Index, 32 << 20).unwrap();
    assert!(import(&bytes(0), &q).is_err());
    drop(hold);
}
