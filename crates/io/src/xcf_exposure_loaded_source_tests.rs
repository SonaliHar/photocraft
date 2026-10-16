//! Ignored source-first endpoint transport drafts. Decoded/reference buffers
//! are external harness inputs; only real reader imports prove owner receipts.
use super::*;
use crate::xcf_filters::{NeighborhoodDomain as D, PointReplaceFamily as F};
use sha2::{Digest, Sha256};
#[derive(Clone, Copy)]
struct Case {
    path: &'static str,
    bytes: usize,
    hash: &'static str,
    precision: u32,
    black: u32,
}
const CASES: [Case; 4] = [
    Case {
        path: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f32-exposure-black-positive-endpoint.xcf",
        bytes: 2005,
        hash: "dcf34793cf27896fec4f6696a3fd7d69c13d08002bb72354ef6f7b054e4f3c38",
        precision: 650,
        black: 0x3dcccccd,
    },
    Case {
        path: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f32-exposure-black-negative-endpoint.xcf",
        bytes: 2005,
        hash: "9adb12c808089f13930d60ea3155598ddfe9bfabd2421b131c4664a22cdf6b72",
        precision: 650,
        black: 0xbdcccccd,
    },
    Case {
        path: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f64-exposure-black-positive-endpoint.xcf",
        bytes: 2007,
        hash: "5d834f0df7afd29507c93b83f1391c9ab4f08e56c8db6d6184821bfd720b879c",
        precision: 750,
        black: 0x3dcccccd,
    },
    Case {
        path: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f64-exposure-black-negative-endpoint.xcf",
        bytes: 2007,
        hash: "c4ec2e55dda286741a42995690cc677a4152e65517cdb1b2783c39682f406fd8",
        precision: 750,
        black: 0xbdcccccd,
    },
];
fn bytes(case: Case) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(case.path);
    let b = std::fs::read(p).unwrap();
    assert_eq!(b.len(), case.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), case.hash);
    b
}
fn decoded(case: Case) -> XcfFile {
    photocraft_xcf::decode(&bytes(case), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
#[test]
#[ignore = "UNRUN: four genuine ordinary endpoint sources import as actual Point, retain raw F32 words/native/profile/ownBYTEzero mask under real samequota promotion"]
fn genuine_ordinary_endpoint_sources_retain_point_class_and_raw_words() {
    use photocraft_doc::{
        LayerContent,
        item_identity::ItemRef,
        xcf_filters::{ExactNumber, FilterValue},
    };
    use photocraft_raster::backing::{BackingQuota, QuotaKind, QuotaLimits, ReadControl};
    for case in CASES {
        let encoded = bytes(case);
        let f = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert!(matches!(crate::xcf_point_replace_source::classify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), Some(D::PointReplace(F::LoadedExposure))));
        let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 });
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
        let imported = crate::import_owned_xcf_from_reader_in_quota(
            "endpointoriginal",
            &mut std::io::Cursor::new(&encoded),
            encoded.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::Point(source) = imported else { panic!("actual singleton Point carrier") };
        source.document().validate_readonly_point_storage(&c).unwrap();
        {
            let d = source.document().document();
            let x = d.xcf_state.as_ref().unwrap();
            let raw = &f.layers[0];
            assert_eq!((d.layers.len(), x.filter_stacks.len(), x.filter_masks.len()), (1, 1, 1));
            assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
            assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
            let node = &x.filter_stacks[0].nodes[0];
            assert_eq!(node.operation.schema_epoch, photocraft_compose::EXPOSURE_LOADED_POINT_EPOCH);
            assert_eq!(node.arguments.len(), 2);
            for (a, b) in node.arguments.iter().zip(&raw.effects[0].arguments) {
                assert_eq!(a.name, b.name);
                let ArgumentValue::Float32Bits(word) = b.value else { panic!("storedF32") };
                assert_eq!(a.value, FilterValue::Number(ExactNumber::Float32Bits(word)));
            }
            let black = node.arguments.iter().find(|a| a.name == "black-level").unwrap();
            assert_eq!(black.value, FilterValue::Number(ExactNumber::Float32Bits(case.black)));
            assert!(photocraft_compose::ExposureParameters::from_words(0.5f32.to_bits(), case.black).is_err());
            let p = node.composition.as_ref().unwrap();
            assert_eq!(p.compatibility_epoch, photocraft_compose::EXPOSURE_LOADED_POINT_EPOCH);
            assert!(p.resolved.is_none());
            assert_eq!(
                [p.stored_blend_space, p.stored_composite_space, p.stored_operator],
                [raw.effects[0].state.blend_space.unwrap(), raw.effects[0].state.composite_space.unwrap(), raw.effects[0].state.composite_mode.unwrap()]
            );
            let Some(ItemRef::Mask(id)) = node.mask else { panic!("ownmask") };
            let mask = x.filter_masks.iter().find(|m| m.id == id && m.owner == node.id).unwrap();
            assert_eq!(mask.surface.try_read_interleaved(mask.bounds).unwrap(), raw.effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes);
            let LayerContent::BackedRaster(native) = &d.layers[0].content else { panic!("actualnative") };
            let pin = native.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &c).unwrap();
            let mut all = Vec::new();
            let mut pixel = vec![0; if case.precision == 650 { 16 } else { 32 }];
            for y in 0..6 {
                for xx in 0..8 {
                    pin.read_pixel_native_into(xx, y, &mut pixel, &c).unwrap();
                    all.extend_from_slice(&pixel);
                }
            }
            assert_eq!(all, raw.plane.as_ref().unwrap().bytes);
        }
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
#[test]
#[ignore = "UNRUN: exact outward-only SOURCE identity, strict incoming constructor/invalidneighbors and old normal/PointStack source priority remain unchanged"]
fn endpoint_source_is_exact_and_old_routes_remain() {
    let f = decoded(CASES[0]);
    let e = &f.layers[0].effects[0];
    assert_eq!(parameters(e).unwrap(), [0.5f32.to_bits(), CASES[0].black]);
    for raw in [0x3dcccccc, 0xbdcccccc, 0, 0x80000000, 0x3dccccce, 0xbdccccce, f32::NAN.to_bits(), f32::INFINITY.to_bits()] {
        let mut altered = f.clone();
        let a = altered.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "black-level").unwrap();
        a.value = ArgumentValue::Float32Bits(raw);
        assert!(parameters(&altered.layers[0].effects[0]).is_none());
        assert!(matches!(crate::xcf_point_replace_source::classify_family_hint(&altered), Some(F::Exposure)));
    }
    for control in 0..13 {
        let mut bad = f.clone();
        let e = &mut bad.layers[0].effects[0];
        match control {
            0 => e.arguments[0].name = e.arguments[1].name.clone(),
            1 => e.arguments[0].value = ArgumentValue::Float64Bits(0.1f64.to_bits()),
            2 => e.version = Some("0:0".into()),
            3 => e.state.enabled = Some(false),
            4 => e.state.opacity_bits = Some(0.5f32.to_bits()),
            5 => e.state.composite_mode = None,
            6 => e.mask = None,
            7 => e.mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&(-0.0f32).to_ne_bytes()),
            8 => bad.layers[0].visible = false,
            9 => bad.layers[0].opacity = 0.375,
            10 => bad.layers[0].offset = (1, 0),
            11 => bad.layers[0].plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&1.01f32.to_ne_bytes()),
            _ => bad.icc = vec![1, 2, 3],
        }
        assert!(!qualify(&bad, &Interrupt::NONE, Some(&budget())).unwrap());
    }
    let p =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/exposure-black-level-precision-evidence/rgb-float_non_linear.source.xcf");
    let old = std::fs::read(p).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&old)), "11f0242cedd1e70854903f0d85868ae8d173666f6e6192d17616790d0b97bfa0");
    let old = photocraft_xcf::decode(&old, &Default::default()).unwrap();
    assert!(matches!(crate::xcf_point_replace_source::classify(&old, &Interrupt::NONE, None).unwrap(), Some(D::PointReplace(F::Exposure))));
    // Existing old PointStack tuple remains selected before this Point hint.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/filter-stack-edits-evidence/stack.source.xcf");
    let old = photocraft_xcf::decode(&std::fs::read(p).unwrap(), &Default::default()).unwrap();
    assert_eq!((old.layers[0].blend_space, old.layers[0].composite_space, old.layers[0].composite_mode), (0, 2, 1));
    assert!(!qualify(&old, &Interrupt::NONE, Some(&budget())).unwrap());
}
#[test]
#[ignore = "UNRUN: external decoded mixed mapper harness proves per-node endpointV2 versus normalV1 rawcopy/epoch, no fake singleton or producer endpointmixed oracle"]
fn mixed_mapper_keeps_per_node_loaded_identity_and_activity() {
    use photocraft_doc::item_identity::ItemRef;
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f32-plain-exposure-ordered-point.xcf");
    let mut f = photocraft_xcf::decode(&std::fs::read(p).unwrap(), &Default::default()).unwrap();
    let effects = &mut f.layers[0].effects;
    let ordinal = effects.iter().position(|e| e.operation == "gegl:exposure").unwrap();
    assert!(matches!(crate::xcf_filters::filtered_flat_source_family(&effects[ordinal]), Some(F::MixedExposure)));
    effects[ordinal].state.enabled = Some(false);
    effects[ordinal].arguments.iter_mut().find(|a| a.name == "black-level").unwrap().value = ArgumentValue::Float32Bits(0x3dcccccd);
    assert!(matches!(crate::xcf_filters::filtered_flat_source_family(&effects[ordinal]), Some(F::MixedLoadedExposure)));
    let masks = vec![None; effects.len()];
    let stack = crate::xcf_filters::qualified_ordered_filtered_flat_source_stack(effects, &masks, ItemRef::Layer(photocraft_doc::LayerId(1)), 1, 22).unwrap();
    for (node, effect) in stack.nodes.iter().zip(effects.iter()) {
        if effect.operation == "gegl:exposure" {
            assert_eq!(node.operation.schema_epoch, photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH);
            assert!(!node.enabled);
            assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH);
        } else {
            assert_ne!(node.operation.schema_epoch, photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH);
        }
    }
    assert!(
        crate::xcf_filters::qualified_stack(effects, &masks, ItemRef::Layer(photocraft_doc::LayerId(1)), 1, 22, D::PointReplace(F::MixedLoadedExposure))
            .is_err()
    );
    // The mixed mask is still its existing numeric-zero policy. This helper
    // maps metadata only; actual global compiler/default/fullphysical proof is
    // mandatory and cannot be established by this external decoded harness.
    effects[ordinal].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&(-0.0f32).to_ne_bytes());
    assert!(crate::xcf_filters::filtered_flat_source_envelope(&effects[ordinal]));
}
#[test]
#[ignore = "UNRUN: new Point qualifier fullnative/count/bytezero work shares original Budget and cancellation even malformed finalnative; no runtime acceptance claim"]
fn work_cancellation_and_counted_native_are_unconditional() {
    let f = decoded(CASES[2]);
    let b = budget();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).unwrap());
    let one = b.used(photocraft_xcf::BudgetKind::Work);
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).unwrap());
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 2 * one);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&photocraft_xcf::Budget::new(limits, None))).is_err());
    let cancel = Interrupt::cancel_only(&|| true);
    assert!(matches!(qualify(&f, &cancel, Some(&budget())), Err(IoError::Cancelled)));
    let mut short = f.clone();
    short.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(!qualify(&short, &Interrupt::NONE, Some(&budget())).unwrap());
    let mut extra = f.clone();
    extra.layers[0].plane.as_mut().unwrap().bytes.push(0);
    assert!(!qualify(&extra, &Interrupt::NONE, Some(&budget())).unwrap());
    let mut mask = f.clone();
    mask.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0..8].copy_from_slice(&(-0.0f64).to_ne_bytes());
    assert!(!qualify(&mask, &Interrupt::NONE, Some(&budget())).unwrap());
}
