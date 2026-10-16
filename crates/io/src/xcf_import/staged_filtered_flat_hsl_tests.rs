//! UNRUN HSL transport contracts. Old genuine singleton files are old-route
//! oracles only. Synthetic mixed decoded buffers are external compatibility
//! harnesses, never new producer packets or paid owner/source receipts.
use super::*;
use crate::xcf_filters::{
    self as map, NeighborhoodDomain as D, PointReplaceFamily as F,
    mixed_stock_source::{self as stock, Kind},
};
use photocraft_raster::backing::{BackingQuota, QuotaKind, QuotaLimits, ReadControl};
use photocraft_xcf::ArgumentValue;
use sha2::{Digest, Sha256};
#[derive(Clone, Copy)]
struct Case {
    path: &'static str,
    bytes: usize,
    sha256: &'static str,
    family: F,
}
const CASES: [Case; 2] = [
    Case {
        path: "corpus/xcf/full/alien-map-hsl-precision-evidence/rgb-float_non_linear.source.xcf",
        bytes: 2452,
        sha256: "c374439d3bdceac393d83dd58954e33640495324176fdf86ca522e7d124883fd",
        family: F::AlienHsl,
    },
    Case {
        path: "corpus/xcf/full/alien-map-hsl-precision-evidence/rgb-double_non_linear.source.xcf",
        bytes: 2437,
        sha256: "2e7f4491d8433bf27c43c820549ee65d519da26e8aa889b5186c47d200dbf32b",
        family: F::AlienHsl,
    },
];
fn bytes(c: Case) -> Vec<u8> {
    let b = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(c.path)).unwrap();
    assert_eq!(b.len(), c.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), c.sha256);
    b
}
fn decoded(i: usize) -> XcfFile {
    photocraft_xcf::decode(&bytes(CASES[i]), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owner() -> photocraft_doc::item_identity::ItemRef {
    photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1))
}
fn mixed(depth: usize, size: (u32, u32)) -> XcfFile {
    let mut f = decoded(depth);
    let sample = f.sample;
    f.width = size.0;
    f.height = size.1;
    let mut plain = f.layers[0].clone();
    plain.name = "declared plain".into();
    plain.effects.clear();
    let mut filtered = plain.clone();
    filtered.name = "declared HSL then RGB".into();
    let hsl = decoded(depth).layers[0].effects[0].clone();
    let mut rgb = hsl.clone();
    rgb.arguments.iter_mut().find(|a| a.name == "color-model").unwrap().value = ArgumentValue::Enum(0);
    filtered.effects = vec![hsl, rgb];
    f.layers = vec![plain, filtered];
    for root in &mut f.layers {
        root.width = size.0;
        root.height = size.1;
        root.offset = (0, 0);
        let pixel = if sample == Sample::F32 {
            [0.125f32, 0.375, 0.625, 0.75].into_iter().flat_map(|x| x.to_ne_bytes()).collect::<Vec<_>>()
        } else {
            [0.125f64, 0.375, 0.625, 0.75].into_iter().flat_map(|x| x.to_ne_bytes()).collect::<Vec<_>>()
        };
        root.plane = Some(Plane { width: size.0, height: size.1, channels: 4, sample, bytes: pixel.repeat(size.0 as usize * size.1 as usize) });
        for e in &mut root.effects {
            let m = e.mask.as_mut().unwrap();
            m.width = size.0;
            m.height = size.1;
            m.offset = (0, 0);
            m.plane =
                Some(Plane { width: size.0, height: size.1, channels: 1, sample, bytes: vec![0; size.0 as usize * size.1 as usize * sample.source_bytes()] });
        }
    }
    f
}

#[test]
#[ignore = "UNRUN: genuine old HSL singleton bytes retain old carrier/schema/native/raw custody; no mixed producer/numerical acceptance"]
fn old_hsl_singletons_remain_original_routes() {
    for c in CASES {
        let encoded = bytes(c);
        let f = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
        assert!(crate::xcf_point_replace_source::classify_family_hint(&f) == Some(c.family));
        assert!(!hint(&f, &Interrupt::NONE, &budget()).unwrap());
        assert!(crate::xcf_point_replace_source::classify(&f, &Interrupt::NONE, Some(&budget())).unwrap() == Some(D::PointReplace(c.family)));
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let c = control();
        let source = crate::import_owned_xcf_from_reader_in_quota(
            "old HSL original",
            &mut std::io::Cursor::new(&encoded),
            encoded.len() as u64,
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::Point(source) = source else { panic!("old Point carrier") };
        let d = source.document().document();
        let state = d.xcf_state.as_ref().unwrap();
        let n = &state.filter_stacks[0].nodes[0];
        assert_eq!(n.operation.schema_epoch, "xcf-22-stock-alien-map-hsl-precision-v1");
        assert_ne!(n.operation.schema_epoch, photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH);
        let photocraft_doc::LayerContent::BackedRaster(native) = &d.layers[0].content else { panic!("native") };
        let mut actual = Vec::new();
        let mut pixel = vec![0; if f.precision == 750 { 32 } else { 16 }];
        for y in 0..f.height {
            for x in 0..f.width {
                native.checked_view().read_pixel_native_into(x as i32, y as i32, &mut pixel, &c).unwrap();
                actual.extend_from_slice(&pixel);
            }
        }
        assert_eq!(actual, f.layers[0].plane.as_ref().unwrap().bytes);
        for p in &f.layers[0].effects[0].properties {
            let identity = format!("{}@{}:{}", p.tag, p.offset, p.declared_bytes);
            let r = state.opaque.iter().find(|r| r.identity == identity && r.owner == Some(state.filter_stacks[0].owner)).unwrap();
            assert_eq!(r.bytes.as_slice(), p.payload.as_slice());
        }
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
#[test]
#[ignore = "UNRUN: declared external decoded mixed HSL1/RGB0 exact fields/order/paired domains and SourceOpacity; not mixed GIMP or SourceOpaque receipt"]
fn declared_hsl_rgb_and_opacity_keep_own_identity() {
    for depth in 0..2 {
        for size in [(1, 1), (65, 67)] {
            let f = mixed(depth, size);
            qualify(&f, &Interrupt::NONE, &budget()).unwrap();
            assert!(root_domain(&f.layers[1], &Interrupt::NONE, &budget()).unwrap() == Some(D::PointReplace(F::MixedAlienHsl)));
            let e = &f.layers[1].effects;
            let masks = vec![None; e.len()];
            let mapped = map::qualified_ordered_filtered_flat_source_stack(e, &masks, owner(), 1, 22).unwrap();
            for ((n, raw), epoch) in
                mapped.nodes.iter().zip(e).zip([photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH, photocraft_compose::ALIEN_MAP_RGB_MIXED_EPOCH])
            {
                assert_eq!(n.operation.schema_epoch, epoch);
                assert_eq!(n.composition.as_ref().unwrap().compatibility_epoch, epoch);
                for (a, b) in n.arguments.iter().zip(&raw.arguments) {
                    assert_eq!(a.name, b.name);
                    match (&a.value, &b.value) {
                        (
                            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(a)),
                            ArgumentValue::Float32Bits(b),
                        ) => assert_eq!(a, b),
                        (photocraft_doc::xcf_filters::FilterValue::Bool(a), ArgumentValue::Boolean(b)) => assert_eq!(a, b),
                        (photocraft_doc::xcf_filters::FilterValue::Enum { domain, value }, ArgumentValue::Enum(b)) => {
                            assert_eq!(domain, "gegl:alien-map/color-model");
                            assert_eq!(value, b);
                        }
                        _ => panic!("exact storage"),
                    }
                }
            }
            let mut changed = f.clone();
            for r in &mut changed.layers {
                r.visible = false;
                r.opacity = 0.;
                for e in &mut r.effects {
                    e.state.enabled = Some(false);
                    e.state.opacity_bits = Some(0.375f32.to_bits());
                    e.properties.iter_mut().find(|p| p.tag == 33).unwrap().payload = 0.375f32.to_be_bytes().to_vec();
                }
            }
            opacity::qualify(&changed, &Interrupt::NONE, &budget()).unwrap();
            assert!(qualify(&changed, &Interrupt::NONE, &budget()).is_err());
            let mapped = map::qualified_ordered_filtered_flat_opacity_source_stack(&changed.layers[1].effects, &masks, owner(), 1, 22).unwrap();
            assert!(mapped.nodes.iter().all(|n| n.opacity_f64_bits == 0.375f64.to_bits()));
        }
    }
}
#[test]
#[ignore = "UNRUN: fixed model1 cheap shape/domain rejects wrong model/type/duplicate/range and RGB-HSL crosspair; old PointStack and unsealed mapper remain closed"]
fn malformed_and_crossmodel_have_no_fallback() {
    let f = mixed(0, (8, 6));
    let h = &f.layers[1].effects[0];
    let r = &f.layers[1].effects[1];
    assert!(stock::kind(h) == Some(Kind::AlienHsl));
    assert!(stock::kind(r) == Some(Kind::AlienRgb));
    for (e, wrong) in [(h, F::MixedAlienRgb), (r, F::MixedAlienHsl)] {
        assert!(map::qualified_filtered_flat_source_stack(std::slice::from_ref(e), &[None], owner(), 1, 22, D::PointReplace(wrong)).is_err());
    }
    assert!(map::qualified_stack(std::slice::from_ref(h), &[None], owner(), 1, 22, D::PointReplace(F::MixedAlienHsl)).is_err());
    for bad in 0..8 {
        let mut e = h.clone();
        match bad {
            0 => e.arguments.iter_mut().find(|a| a.name == "color-model").unwrap().value = ArgumentValue::Enum(2),
            1 => e.version = Some("1:0".into()),
            2 => {
                e.arguments.pop();
            }
            3 => e.arguments[1].name = e.arguments[0].name.clone(),
            4 => e.arguments[1].value = ArgumentValue::Float64Bits(1f64.to_bits()),
            5 => e.arguments.iter_mut().find(|a| a.name == "cpn-1-frequency").unwrap().value = ArgumentValue::Float32Bits(20.001f32.to_bits()),
            6 => e.arguments.iter_mut().find(|a| a.name == "cpn-1-phaseshift").unwrap().value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
            _ => e.arguments.iter_mut().find(|a| a.name == "cpn-1-keep").unwrap().value = ArgumentValue::Enum(1),
        };
        assert!(stock::kind(&e).is_none());
        assert!(map::filtered_flat_source_family(&e).is_none());
    }
    let mut old = f.clone();
    old.layers = vec![old.layers[1].clone()];
    old.layers[0].blend_space = 0;
    old.layers[0].composite_space = 2;
    old.layers[0].composite_mode = 1;
    assert!(!hint(&old, &Interrupt::NONE, &budget()).unwrap());
}
#[test]
#[ignore = "UNRUN: original cumulative parser/native/mask work and cancel remain effective inactive; actual2N mapper Resident bound distinct from work, no HSL math in IO"]
fn work_resident_and_inactive_proofs_remain() {
    let mut f = mixed(1, (65, 67));
    for r in &mut f.layers {
        r.visible = false;
        r.opacity = 0.;
        for e in &mut r.effects {
            e.state.enabled = Some(false);
        }
    }
    let b = budget();
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    let one = b.used(photocraft_xcf::BudgetKind::Work);
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 2 * one);
    assert!(one > 8192);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, &photocraft_xcf::Budget::new(limits, None)).is_err());
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&|| true), &budget()), Err(IoError::Cancelled)));
    for e in &f.layers[1].effects {
        assert_eq!(stock::parsing_work(&e.operation), 4096);
        let n = 2 * e.arguments.len();
        let floor = n * std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>()
            + n * std::mem::size_of::<f64>()
            + std::mem::size_of::<Vec<photocraft_doc::xcf_filters::FilterArgument>>()
            + std::mem::size_of::<Vec<f64>>()
            + 64;
        assert!(stock::mapping_bytes(e).unwrap() >= floor);
    }
    let mut malformed = f.clone();
    malformed.layers[1].plane.as_mut().unwrap().bytes.pop();
    assert!(qualify(&malformed, &Interrupt::NONE, &budget()).is_err());
    let mut malformed = f.clone();
    malformed.layers[1].effects[1].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1;
    assert!(qualify(&malformed, &Interrupt::NONE, &budget()).is_err());
}
