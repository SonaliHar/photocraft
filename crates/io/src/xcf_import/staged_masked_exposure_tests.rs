//! UNRUN masked source contracts. Real source imports retain actual paid storage;
//! decoded adversarial mutations are external harnesses, not producer evidence.
use super::*;
use crate::xcf_filters::{NeighborhoodDomain as D, PointReplaceFamily as F};
use crate::xcf_point_replace_source::masked_exposure_source as masked;
use photocraft_raster::backing::{BackingQuota, QuotaKind, QuotaLimits, ReadControl};
use sha2::{Digest, Sha256};
#[derive(Clone, Copy)]
struct Binding {
    path: &'static str,
    bytes: usize,
    sha256: &'static str,
}
struct Case {
    id: &'static str,
    source: Binding,
    capture: Binding,
    masked: bool,
    precision: u32,
}
const CASES: [Case; 10] = [
    Case {
        id: "f32-none",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-none.xcf",
            bytes: 2005,
            sha256: "95ec573c89e061f69e250a6dd84afc6b51339f991e17bfd565305f77f4e5ad48",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-none.case.json",
            bytes: 38804,
            sha256: "bfb74ceb9eab3639013cd1655150b3ab9e4d6bb9176730e0b218eedfe03d9aa2",
        },
        masked: false,
        precision: 650,
    },
    Case {
        id: "f32-all",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-all.xcf",
            bytes: 2005,
            sha256: "1a9961aaade40d3721549b61057d4ef66751394d93f0ca613f5631e9a9ba8c68",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-all.case.json",
            bytes: 38785,
            sha256: "4fe30f9de43250fafe725b8d21e79e51d6f8890ad84ad3070b0bd08066d946e2",
        },
        masked: true,
        precision: 650,
    },
    Case {
        id: "f32-all-invert-empty",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-all-invert-empty.xcf",
            bytes: 2005,
            sha256: "95ec573c89e061f69e250a6dd84afc6b51339f991e17bfd565305f77f4e5ad48",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-all-invert-empty.case.json",
            bytes: 39058,
            sha256: "4e63bcd26a7167f3f5bfbbfd40b3a27773ba1cb8e3ebc0637f2a147e5a4b34e5",
        },
        masked: false,
        precision: 650,
    },
    Case {
        id: "f32-half-rectangle",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-half-rectangle.xcf",
            bytes: 2049,
            sha256: "f11a6273f28c70ca633db4c33df0fee0364f45f9a7a14369307fe2dc92e8e95e",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-half-rectangle.case.json",
            bytes: 39017,
            sha256: "8271b6493b88db2626307959d19342d9974076d2702f7db31133fda22ec05775",
        },
        masked: true,
        precision: 650,
    },
    Case {
        id: "f32-feathered-half",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-feathered-half.xcf",
            bytes: 2139,
            sha256: "8d29b206ee872d0d74fe833425d44a6a6f4e47d5815cfa476b432bf270bdf43c",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f32-feathered-half.case.json",
            bytes: 39017,
            sha256: "8ebba4609733a79ef5f60b8318574ba3711382530d1fab1e8bc84fd2008e59ab",
        },
        masked: true,
        precision: 650,
    },
    Case {
        id: "f64-none",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-none.xcf",
            bytes: 2007,
            sha256: "11b7180ac1dd1f99f717c71f51b482b89ca6471731fcb7066157b010299900ba",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-none.case.json",
            bytes: 38849,
            sha256: "16433f89e80776e7cdb00469f1e745fed29ad7ea3969d1215269d0f50a21849f",
        },
        masked: false,
        precision: 750,
    },
    Case {
        id: "f64-all",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-all.xcf",
            bytes: 2007,
            sha256: "bb72fabdd1965ce2ac2c508695d7be4ace925c94484efb9ebc46a4dc4bd37a63",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-all.case.json",
            bytes: 38830,
            sha256: "505bd3636368f6a345db47a745c1a7550ff046a5d11499e736292c1aba7fd849",
        },
        masked: true,
        precision: 750,
    },
    Case {
        id: "f64-all-invert-empty",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-all-invert-empty.xcf",
            bytes: 2007,
            sha256: "11b7180ac1dd1f99f717c71f51b482b89ca6471731fcb7066157b010299900ba",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-all-invert-empty.case.json",
            bytes: 39101,
            sha256: "81b0b70ce4b8ae13f09c654188aa1688ca7545315ab38ea1a003cbe22070ba3d",
        },
        masked: false,
        precision: 750,
    },
    Case {
        id: "f64-half-rectangle",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-half-rectangle.xcf",
            bytes: 2051,
            sha256: "b3397b5dd20a71a12e04ba7b577fee1969c674e6d9cdac192529be6cbc688dc2",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-half-rectangle.case.json",
            bytes: 39060,
            sha256: "59a81ce5b971a761d1ce8df7922ee7fdf8f4eecd86ea0156c86436f387a95907",
        },
        masked: true,
        precision: 750,
    },
    Case {
        id: "f64-feathered-half",
        source: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-feathered-half.xcf",
            bytes: 2159,
            sha256: "7e6eb2796592ba0ce6c90eac493fef897c9a19f8751be5c51a6c56b14345e2ef",
        },
        capture: Binding {
            path: "temp/sol3-filtered-flat-nonzero-mask-gimp-20261016/author-v4/outputs/f64-feathered-half.case.json",
            bytes: 39060,
            sha256: "0852550cf0a7d3a26533c780b3aeb89009835dc5d04b915e7a65c7ca9ce48fbb",
        },
        masked: true,
        precision: 750,
    },
];
fn read(b: Binding) -> Vec<u8> {
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(b.path)).unwrap();
    assert_eq!(bytes.len(), b.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), b.sha256);
    bytes
}
fn decoded(i: usize) -> XcfFile {
    photocraft_xcf::decode(&read(CASES[i].source), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn selected(f: &XcfFile) -> SelectedStagedFamily {
    SourceSelectedPolicy::family_controlled(f, &Interrupt::NONE, Some(&budget())).unwrap()
}
#[test]
#[ignore = "UNRUN: ten genuine both-depth source files preserve oldBYTEzero selection vs newrawnonempty Point epoch, rawF64>1 and samequota native/mask/source custody; no rendered mask theorem"]
fn genuine_sources_preserve_raw_masks_and_old_zero_priority() {
    use photocraft_doc::{LayerContent, item_identity::ItemRef};
    for case in &CASES {
        let bytes = read(case.source);
        let _capture = read(case.capture);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(masked::hint(&raw, &Interrupt::NONE, &budget()).unwrap(), case.masked);
        assert_eq!(selected(&raw), SelectedStagedFamily::Point(if case.masked { F::MaskedExposure } else { F::Exposure }));
        assert_eq!(masked::qualify(&raw, &Interrupt::NONE, Some(&budget())).unwrap(), case.masked);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let c = control();
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
        let source = crate::import_owned_xcf_from_reader_in_quota(
            case.id,
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::Point(source) = source else { panic!("actual Point carrier") };
        assert!(source.document().is_readonly_point_storage());
        let doc = source.document().document();
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!((doc.layers.len(), state.filter_stacks.len(), state.filter_masks.len()), (1, 1, 1));
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, case.precision);
        assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
        let layer = &doc.layers[0];
        let node = &state.filter_stacks[0].nodes[0];
        let effect = &raw.layers[0].effects[0];
        assert_eq!(node.operation.name, "gegl:exposure");
        assert_eq!(node.operation.stored_version, "1:0");
        assert_eq!(node.opacity_f64_bits, 1f64.to_bits());
        assert!(node.enabled);
        assert_eq!(
            node.operation.schema_epoch,
            if case.masked { photocraft_compose::EXPOSURE_MASKED_EPOCH } else { "xcf-22-stock-exposure-black-level-precision-v1" }
        );
        if case.masked {
            assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, photocraft_compose::EXPOSURE_MASKED_EPOCH);
        }
        for p in &effect.properties {
            let identity = format!("{}@{}:{}", p.tag, p.offset, p.declared_bytes);
            let kept = state
                .opaque
                .iter()
                .find(|r| r.owner == Some(ItemRef::Layer(layer.id)) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
                .unwrap();
            assert_eq!(kept.bytes.as_slice(), p.payload.as_slice());
        }
        let mask = &state.filter_masks[0];
        assert_eq!(node.mask, Some(ItemRef::Mask(mask.id)));
        assert_eq!(mask.owner, node.id);
        let expected = &effect.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes;
        assert_eq!(&mask.surface.try_read_interleaved(mask.bounds).unwrap(), expected);
        if case.masked {
            let proof = mask.validate_checked_f32_coverage_with(&c).unwrap();
            assert!(proof.raw_nonempty());
            doc.validate_xcf_state_with_checked_filter_mask(&proof, &Interrupt::NONE).unwrap();
        } else {
            doc.validate_xcf_state_with(&Interrupt::NONE).unwrap();
        }
        if case.id == "f64-all" {
            assert!(expected.chunks_exact(8).all(|b| u64::from_ne_bytes(b.try_into().unwrap()) == 0x3ff0000009b25010));
            assert!(mask.validate_with(&Interrupt::NONE).is_err());
        }
        let LayerContent::BackedRaster(native) = &layer.content else { panic!("original native") };
        let view = native.checked_view();
        let mut actual = Vec::new();
        let mut pixel = vec![0; if case.precision == 650 { 16 } else { 32 }];
        for y in 0..raw.height {
            for x in 0..raw.width {
                view.read_pixel_native_into(x as i32, y as i32, &mut pixel, &c).unwrap();
                actual.extend_from_slice(&pixel);
            }
        }
        assert_eq!(actual, raw.layers[0].plane.as_ref().unwrap().bytes);
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
fn mask_words(f: &mut XcfFile, bits: u64) {
    let sample = f.sample;
    let p = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
    let word = if sample == Sample::F64 { bits.to_ne_bytes().to_vec() } else { (bits as u32).to_ne_bytes().to_vec() };
    p.bytes = word.repeat(p.width as usize * p.height as usize);
}
#[test]
#[ignore = "UNRUN: raw logical nonempty precedesF32, observed>1 preservesbits, checked-rounding preimage and negative/NaN/Inf refuse; signedzeroempty never becomes oldBYTEzero Source"]
fn raw_coverage_domain_and_empty_boundaries_are_exact() {
    let original = decoded(6);
    for bits in [1u64, 0x3ff0000009b25010, 0x3ff0000010000000] {
        let mut f = original.clone();
        mask_words(&mut f, bits);
        assert!(masked::hint(&f, &Interrupt::NONE, &budget()).unwrap());
        assert!(masked::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(selected(&f), SelectedStagedFamily::Point(F::MaskedExposure));
    }
    for bits in [0x8000000000000001, 0xbff0000000000000, 0x7ff0000000000000, 0x7ff8000000000001, 0x3ff0000010000001] {
        let mut f = original.clone();
        mask_words(&mut f, bits);
        assert_eq!(selected(&f), SelectedStagedFamily::Point(F::MaskedExposure));
        assert!(!masked::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
    for bits in [0u64, 0x8000000000000000] {
        let mut f = original.clone();
        mask_words(&mut f, bits);
        assert!(!masked::hint(&f, &Interrupt::NONE, &budget()).unwrap());
        assert_eq!(selected(&f), SelectedStagedFamily::Point(F::Exposure));
        assert!(!masked::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(crate::xcf_point_replace_source::classify(&f, &Interrupt::NONE, Some(&budget())).unwrap().is_some(), bits == 0);
    }
}
#[test]
#[ignore = "UNRUN: selected malformed source never acquires fallback, oldPointStack/loaded/legacy route priority and exact SOURCE activity/profile/native/args policy; decoded external controls"]
fn selected_failures_and_old_identities_remain_final() {
    let base = decoded(1);
    for bad in 0..12 {
        let mut f = base.clone();
        match bad {
            0 => f.layers[0].visible = false,
            1 => f.layers[0].opacity = 0.375,
            2 => f.layers[0].effects[0].state.enabled = Some(false),
            3 => f.layers[0].effects[0].state.opacity_bits = Some(0.375f32.to_bits()),
            4 => f.icc = vec![1],
            5 => f.layers[0].offset = (1, 0),
            6 => f.layers[0].effects[0].version = Some("0:0".into()),
            7 => f.layers[0].effects[0].state.blend_space = None,
            8 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float64Bits(0),
            9 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.25f32.to_ne_bytes()),
            10 => f.layers[0].effects[0].mask.as_mut().unwrap().mask_enabled = false,
            _ => f.layers[0].legacy_linked = true,
        }
        assert_eq!(selected(&f), SelectedStagedFamily::Point(F::MaskedExposure));
        assert!(!masked::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
    let mut endpoint = base.clone();
    let black = endpoint.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "black-level").unwrap();
    black.value = ArgumentValue::Float32Bits(0x3dcccccd);
    assert_eq!(selected(&endpoint), SelectedStagedFamily::Point(F::LoadedExposure));
    assert!(!masked::qualify(&endpoint, &Interrupt::NONE, Some(&budget())).unwrap());
    let mut legacy = base.clone();
    let e = &mut legacy.layers[0].effects[0];
    e.state.blend_space = Some(-901479855);
    e.state.composite_space = Some(-901479855);
    e.state.composite_mode = Some(-901479855);
    assert_eq!(selected(&legacy), SelectedStagedFamily::LegacyScalar(LegacyScalarFamily::ExposureGeometry));
    let mut stack = base.clone();
    stack.layers[0].blend_space = 0;
    stack.layers[0].composite_space = 2;
    stack.layers[0].composite_mode = 1;
    let extra = stack.layers[0].effects[0].clone();
    stack.layers[0].effects.push(extra);
    assert_eq!(selected(&stack), SelectedStagedFamily::PointStackEdits);
    let e = &base.layers[0].effects[0];
    let mapped = crate::xcf_filters::qualified_stack(
        std::slice::from_ref(e),
        &[None],
        photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
        1,
        22,
        D::PointReplace(F::MaskedExposure),
    )
    .unwrap();
    assert_eq!(mapped.nodes[0].operation.schema_epoch, photocraft_compose::EXPOSURE_MASKED_EPOCH);
}
#[test]
#[ignore = "UNRUN: native64/declared counts and same original Work/cancellation/quota remain effective before real source publication; logical harness doesnot substitute physical default/padding scan"]
fn same_work_control_and_inactive_bad_source_cannot_publish() {
    let f = decoded(8);
    let b = budget();
    assert!(masked::qualify(&f, &Interrupt::NONE, Some(&b)).unwrap());
    let one = b.used(photocraft_xcf::BudgetKind::Work);
    assert!(masked::qualify(&f, &Interrupt::NONE, Some(&b)).unwrap());
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 2 * one);
    let lower = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
    assert!(masked::qualify(&f, &Interrupt::NONE, Some(&photocraft_xcf::Budget::new(lower, None))).is_err());
    let cancel = Interrupt::cancel_only(&|| true);
    assert!(matches!(masked::qualify(&f, &cancel, Some(&budget())), Err(IoError::Cancelled)));
    let mut short = f.clone();
    short.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes.pop();
    assert!(!masked::qualify(&short, &Interrupt::NONE, Some(&budget())).unwrap());
    let bytes = read(CASES[8].source);
    for deny in [false, true] {
        let q = BackingQuota::new(QuotaLimits { resident: if deny { 256 << 20 } else { 0 }, index: 64 << 20, scratch: 0 });
        let no = |_| Err(photocraft_raster::backing::BackingError::Work);
        let c = if deny { ReadControl { interrupt: &Interrupt::NONE, work: &no } } else { control() };
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
        assert!(
            crate::import_owned_xcf_from_reader_in_quota(
                "denied masked source",
                &mut std::io::Cursor::new(&bytes),
                bytes.len() as u64,
                &Default::default(),
                &limits,
                q.clone(),
                &c
            )
            .is_err()
        );
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
