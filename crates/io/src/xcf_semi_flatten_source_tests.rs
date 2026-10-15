//! UNRUN authored RGB Semi-Flatten source lifecycle; FMA/Babl numerical schedule remains HOLD.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, usize, &str, bool, bool); 12] = [
    ("rgb-f32-author-v1/outputs/rgb-f32-background.source.xcf", 1909, "e6d705f52c19ac30c4e3d5f92eefe22ee6baab791ce42fdb879397cb140c61da", false, true),
    ("rgb-f32-author-v1/outputs/rgb-f32-background.edit.xcf", 1909, "b12706117bbe1718c4874bfca8a4cdb7964cc4e7e045eff720645e043d022452", false, true),
    ("rgb-f32-author-v1/outputs/rgb-f32-color-alpha.source.xcf", 1909, "3976c92c72840afc57cacb95a5e120cd68b949a501efa0fa7a8d73db5c2987d9", false, true),
    ("rgb-f32-author-v1/outputs/rgb-f32-color-alpha.edit.xcf", 1909, "147035285b053556ac174834eee7d83eca65f7041098cd0d8b4aff3a7dda71cb", false, true),
    ("rgb-f64-author-v1/outputs/rgb-f64-background.source.xcf", 2028, "63278093e370c2e7d0bcea850c1fa3ef494f6975b91746ea71594d7b9bec5b71", true, true),
    ("rgb-f64-author-v1/outputs/rgb-f64-background.edit.xcf", 2028, "41d2ebc1e30203079a129dd2cd7623689207874b9f34967922eeb7fd306507e1", true, true),
    ("rgb-f64-author-v1/outputs/rgb-f64-color-alpha.source.xcf", 2028, "2501ec0a000249bf114158e78d4e634d90fb3e857ad08d7750a16bdc7e938d3a", true, true),
    ("rgb-f64-author-v1/outputs/rgb-f64-color-alpha.edit.xcf", 2028, "8a31886838d0234448a8d1b500a05efa9d60eb3e98d0193a27c2eddf35219b57", true, true),
    (
        "rgb-f32-nonlinear-bytes-author-v1/outputs/rgb-f32-nonlinear-bytes.source.xcf",
        1912,
        "d4debe389177edc01192b09fadb22f5d353b54cd27f7af357698ab5cfb223b89",
        false,
        false,
    ),
    (
        "rgb-f32-nonlinear-bytes-author-v1/outputs/rgb-f32-nonlinear-bytes.edit.xcf",
        1912,
        "4953d0168280fc3ba07fcb897a508f6adade89ed9327c5785781c4c8b964de71",
        false,
        false,
    ),
    (
        "rgb-f64-nonlinear-bytes-author-v1/outputs/rgb-f64-nonlinear-bytes.source.xcf",
        2031,
        "8caac66d87e1ea9599c90600709989a87dd33b458beec4611a764971dae9212a",
        true,
        false,
    ),
    (
        "rgb-f64-nonlinear-bytes-author-v1/outputs/rgb-f64-nonlinear-bytes.edit.xcf",
        2031,
        "e7b4a2e405ccc4804e6e851fdd4bc7d8e1f54c41596489020c7082a5d3a0c45d",
        true,
        false,
    ),
];

fn bytes(case: usize) -> Vec<u8> {
    let root = std::env::var_os("PHOTOCRAFT_SEMI_FLATTEN_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-semi-flatten-gimp-20261016"));
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
    crate::import_owned_xcf_from_reader_in_quota("Own Semi.xcf", &mut Cursor::new(b), b.len() as u64, &Default::default(), &limits, q.clone(), &ctl())
}
#[test]
#[ignore = "UNRUN: twelve authored F32/F64 Linear/NL colour sources retain tags/words/native/profile/mask/raw3 andpaid title class"]
fn actual_sources_keep_literal_and_native_custody() {
    for case in 0..12 {
        let f = file(case);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = import(&bytes(case), &q).unwrap() else { panic!("Point") };
        let owner = source.document();
        owner.validate_readonly_point_storage(&ctl()).unwrap();
        let d = owner.document();
        assert!(!owner.is_readonly_assigned_point_storage());
        assert_eq!(d.sample_encoding, Some(SampleEncoding::NonlinearProfile));
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, if CASES[case].3 { 750 } else { 650 });
        let x = d.xcf_state.as_ref().unwrap();
        let n = &x.filter_stacks[0].nodes[0];
        assert_eq!(n.operation.schema_epoch, photocraft_compose::SEMI_FLATTEN_EPOCH);
        let p = n.composition.as_ref().unwrap();
        assert_eq!(p.compatibility_epoch, n.operation.schema_epoch);
        assert!(p.resolved.is_none());
        assert_eq!(
            (p.stored_blend_space, p.stored_composite_space, p.stored_operator),
            (i32::from_be_bytes([0xc8, 0x75, 0x82, 0x51]), i32::from_be_bytes([0xc8, 0x75, 0x82, 0x51]), i32::from_be_bytes([0xc8, 0x75, 0x82, 0x51]))
        );
        let (words, encoding) = parameters(&f.layers[0].effects[0]).unwrap();
        assert_eq!(encoding, if CASES[case].4 { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile });
        let photocraft_doc::xcf_filters::FilterValue::Color { components, encoding: stored, profile: None } = &n.arguments[0].value else { panic!("literal") };
        assert_eq!(*stored, encoding);
        assert_eq!(
            components
                .iter()
                .map(|v| match v {
                    photocraft_doc::xcf_filters::ExactNumber::Float32Bits(b) => *b,
                    _ => panic!("f32"),
                })
                .collect::<Vec<_>>(),
            words
        );
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("native") };
        let width = if CASES[case].3 { 32 } else { 16 };
        let r = photocraft_geom::Rect::new(0, 0, 8, 6);
        let pin = layer.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &ctl()).unwrap();
        let mut raw = vec![0; 48 * width];
        for y in 0..6 {
            for xx in 0..8 {
                let at = (y * 8 + xx) as usize * width;
                pin.read_pixel_native_into(xx, y, &mut raw[at..at + width], &ctl()).unwrap();
            }
        }
        assert_eq!(layer.operative_bounds(), r);
        assert_eq!(raw, f.layers[0].plane.as_ref().unwrap().bytes);
        assert!(layer.default_bytes().iter().all(|v| *v == 0));
        assert_eq!(layer.default_bytes().len(), width);
        let m = &x.filter_masks[0];
        assert_eq!(m.bounds, r);
        assert_eq!(m.surface.default_bytes().len(), width / 4);
        assert!(m.is_empty_selection_with(&Interrupt::NONE).unwrap());
        let title = owner.try_with_document_name("Semi.pcraft", &ctl()).unwrap();
        assert!(title.is_readonly_point_storage());
        assert_eq!(title.document().xcf_state, d.xcf_state);
        assert_eq!(title.document().source_color_context, d.source_color_context);
        assert_eq!(title.document().icc_profile, d.icc_profile);
    }
}
#[test]
#[ignore = "UNRUN: strict source roles/activity/geometry/literal/outer/rawmissing/native/mask refuse with selectedfamily final failure"]
fn exact_source_refusals() {
    for case in [0, 4, 8, 10] {
        for bad in 0..16 {
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
                9 => f.layers[0].effects[0].state.clipping = Some(1),
                10 => f.layers[0].effects[0].state.region = Some(1),
                11 => {
                    if CASES[case].3 {
                        f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&1.01f64.to_ne_bytes())
                    } else {
                        f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes())
                    }
                }
                12 => {
                    f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1;
                }
                13 => {
                    let ArgumentValue::Color { encoding, .. } = &mut f.layers[0].effects[0].arguments[0].value else { panic!("color") };
                    *encoding = "RGBA double".into()
                }
                14 => {
                    let ArgumentValue::Color { samples, .. } = &mut f.layers[0].effects[0].arguments[0].value else { panic!("color") };
                    samples[..4].copy_from_slice(&f32::INFINITY.to_le_bytes())
                }
                _ => f.layers[0].effects[0].properties[0].tag = 99,
            };
            assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "{case}/{bad}");
            assert!(matches!(crate::xcf_point_replace_source::classify_family_hint(&f), Some(crate::xcf_filters::PointReplaceFamily::SemiFlatten)));
            assert!(crate::xcf_point_replace_source::classify(&f, &Interrupt::NONE, Some(&budget())).unwrap().is_none());
        }
    }
}
fn raw_words(mut b: Vec<u8>, values: [i32; 3]) -> Vec<u8> {
    for (tag, value) in [37u32, 36, 35].into_iter().zip(values) {
        let mut pattern = Vec::new();
        pattern.extend(tag.to_be_bytes());
        pattern.extend(4u32.to_be_bytes());
        pattern.extend([0xc8, 0x75, 0x82, 0x51]);
        let positions = b.windows(12).enumerate().filter_map(|(i, w)| (w == pattern.as_slice()).then_some(i)).collect::<Vec<_>>();
        assert_eq!(positions.len(), 1);
        let at = positions[0] + 8;
        b[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    b
}
#[test]
#[ignore = "UNRUN: separate opaque provenance policy retains mixedMIN/0/MAX only inside exact SemiReplace envelope; no enum alias"]
fn opaque_words_are_custody_not_other_policy() {
    for case in [0, 4] {
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
    for case in [0, 4, 8, 10] {
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
