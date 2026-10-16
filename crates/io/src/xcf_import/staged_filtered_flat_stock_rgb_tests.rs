//! UNRUN contracts. Six real old singleton oracles stay singleton references.
//! Synthetic mixed originals below are declared compatibility contracts only;
//! decoded buffers are external harnesses, not paid source/producer receipts.
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
const CASES: [Case; 6] = [
    Case {
        path: "corpus/xcf/full/color-temperature-precision-evidence/rgb-float_non_linear.source.xcf",
        bytes: 2143,
        sha256: "256433149dbe49d81b95603391163e3293ca9b3f39f4b4da0051e0fc9d326204",
        family: F::Temperature,
    },
    Case {
        path: "corpus/xcf/full/color-temperature-precision-evidence/rgb-double_non_linear.source.xcf",
        bytes: 2128,
        sha256: "dbe514c596b5bea51fb8742230c38df89ecd7b0f46ac66788c5d848626ee0c25",
        family: F::Temperature,
    },
    Case {
        path: "corpus/xcf/full/sepia-precision-evidence/rgb-float_non_linear.source.xcf",
        bytes: 2131,
        sha256: "f0a07f5135dc12ba0c44e1166a1b01ce1dd1686a9d87bb86b3722939bcfb155a",
        family: F::Sepia,
    },
    Case {
        path: "corpus/xcf/full/sepia-precision-evidence/rgb-double_non_linear.source.xcf",
        bytes: 2116,
        sha256: "0d280858aabf5da23777ada365452b50f36651437d4f42ec4655c50661d49c08",
        family: F::Sepia,
    },
    Case {
        path: "corpus/xcf/full/alien-map-precision-evidence/rgb-float_non_linear.source.xcf",
        bytes: 2440,
        sha256: "ce8146302b8ea62b8ef8b6aaf87f159459fdba7376983471d03e0a545e8d6273",
        family: F::AlienRgb,
    },
    Case {
        path: "corpus/xcf/full/alien-map-precision-evidence/rgb-double_non_linear.source.xcf",
        bytes: 2425,
        sha256: "301f2bcdf487660348687c54727940dea9c8c3cd50b491d2c9d4e9c90185b35a",
        family: F::AlienRgb,
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
    filtered.name = "declared ordered new3".into();
    filtered.effects = [depth, depth + 2, depth + 4].into_iter().map(|i| decoded(i).layers[0].effects[0].clone()).collect();
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
#[ignore = "UNRUN: six genuine old RGBNL singleton oracles retain old source carrier/schema/raw/native custody; no mixed producer or arithmetic acceptance claim"]
fn genuine_old_singletons_remain_original_routes() {
    use photocraft_doc::LayerContent;
    for c in CASES {
        let encoded = bytes(c);
        let f = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
        assert!(crate::xcf_point_replace_source::classify_family_hint(&f) == Some(c.family));
        assert!(!hint(&f, &Interrupt::NONE, &budget()).unwrap());
        assert!(crate::xcf_point_replace_source::classify(&f, &Interrupt::NONE, Some(&budget())).unwrap() == Some(D::PointReplace(c.family)));
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let control = control();
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
        let source = crate::import_owned_xcf_from_reader_in_quota(
            "old singleton original",
            &mut std::io::Cursor::new(&encoded),
            encoded.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &control,
        )
        .unwrap();
        let crate::OwnedXcfImport::Point(source) = source else { panic!("old Point carrier") };
        let d = source.document().document();
        let state = d.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        let raw = &f.layers[0];
        assert_eq!(node.operation.name, raw.effects[0].operation);
        assert!(
            ![photocraft_compose::COLOR_TEMPERATURE_MIXED_EPOCH, photocraft_compose::SEPIA_MIXED_EPOCH, photocraft_compose::ALIEN_MAP_RGB_MIXED_EPOCH]
                .contains(&node.operation.schema_epoch.as_str())
        );
        let LayerContent::BackedRaster(native) = &d.layers[0].content else { panic!("old native") };
        let view = native.checked_view();
        let mut native = Vec::new();
        let mut pixel = vec![0; if f.precision == 750 { 32 } else { 16 }];
        for y in 0..f.height {
            for x in 0..f.width {
                view.read_pixel_native_into(x as i32, y as i32, &mut pixel, &control).unwrap();
                native.extend_from_slice(&pixel);
            }
        }
        assert_eq!(native, raw.plane.as_ref().unwrap().bytes);
        for p in &raw.effects[0].properties {
            let identity = format!("{}@{}:{}", p.tag, p.offset, p.declared_bytes);
            let record = state.opaque.iter().find(|r| r.identity == identity && r.owner == Some(state.filter_stacks[0].owner)).unwrap();
            assert_eq!(record.bytes.as_slice(), p.payload.as_slice());
        }
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
#[test]
#[ignore = "UNRUN: declared synthetic2root/3ordered-leaf Source and SourceOpacity compatibility atbothdepth/sizes preserves exact enum/bool/F32/order/own domains; not GIMP mixed or SourceOpaque-group receipt"]
fn declared_mixed_and_opacity_contract_preserve_each_typed_field() {
    for depth in 0..2 {
        for size in [(1, 1), (65, 67)] {
            let f = mixed(depth, size);
            assert!(hint(&f, &Interrupt::NONE, &budget()).unwrap());
            qualify(&f, &Interrupt::NONE, &budget()).unwrap();
            assert!(root_domain(&f.layers[1], &Interrupt::NONE, &budget()).unwrap() == Some(D::PointReplace(F::MixedTemperature)));
            let e = &f.layers[1].effects;
            let masks = vec![None; e.len()];
            let mapped = map::qualified_ordered_filtered_flat_source_stack(e, &masks, owner(), 1, 22).unwrap();
            for ((node, source), epoch) in mapped.nodes.iter().zip(e).zip([
                photocraft_compose::COLOR_TEMPERATURE_MIXED_EPOCH,
                photocraft_compose::SEPIA_MIXED_EPOCH,
                photocraft_compose::ALIEN_MAP_RGB_MIXED_EPOCH,
            ]) {
                assert_eq!(node.operation.schema_epoch, epoch);
                assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, epoch);
                assert_eq!(node.arguments.len(), source.arguments.len());
                for (a, b) in node.arguments.iter().zip(&source.arguments) {
                    assert_eq!(a.name, b.name);
                    let expected = match b.value {
                        ArgumentValue::Float32Bits(bits) => {
                            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(bits))
                        }
                        ArgumentValue::Boolean(value) => photocraft_doc::xcf_filters::FilterValue::Bool(value),
                        ArgumentValue::Enum(value) => photocraft_doc::xcf_filters::FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value },
                        _ => panic!("exact old raw storage"),
                    };
                    assert_eq!(a.value, expected);
                }
            }
            let mut changed = f.clone();
            for d in &mut changed.layers {
                for e in &mut d.effects {
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
#[ignore = "UNRUN: cheap exact2/2/10 typed parser domains, unknownAlienModel2/wrongtypes/duplicates/unknowns refuse, own-kind mapping and oldPointStack priority remain strict; externaldecoded contract"]
fn cheap_shapes_domains_and_wrong_family_refuse() {
    let f = mixed(0, (8, 6));
    for (index, family, kind) in [(0, F::MixedTemperature, Kind::Temperature), (1, F::MixedSepia, Kind::Sepia), (2, F::MixedAlienRgb, Kind::AlienRgb)] {
        let e = &f.layers[1].effects[index];
        assert!(stock::kind(e) == Some(kind));
        assert!(map::qualified_stack(std::slice::from_ref(e), &[None], owner(), 1, 22, D::PointReplace(family)).is_err());
        let wrong = if index == 0 { F::MixedSepia } else { F::MixedTemperature };
        assert!(map::qualified_filtered_flat_source_stack(std::slice::from_ref(e), &[None], owner(), 1, 22, D::PointReplace(wrong)).is_err());
        for bad in 0..5 {
            let mut x = e.clone();
            match bad {
                0 => x.version = Some("1:0".into()),
                1 => {
                    x.arguments.pop();
                }
                2 => x.arguments[0].name = "unknown".into(),
                3 => x.arguments[0].value = ArgumentValue::Float64Bits(0),
                _ => x.arguments[1].name = x.arguments[0].name.clone(),
            };
            assert!(stock::kind(&x).is_none());
        }
    }
    let mut temp = f.layers[1].effects[0].clone();
    temp.arguments[0].value = ArgumentValue::Float32Bits(999f32.to_bits());
    assert!(stock::kind(&temp).is_none());
    let mut sepia = f.layers[1].effects[1].clone();
    sepia.arguments.iter_mut().find(|a| a.name == "srgb").unwrap().value = ArgumentValue::Float32Bits(1f32.to_bits());
    assert!(stock::kind(&sepia).is_none());
    let mut alien = f.layers[1].effects[2].clone();
    alien.arguments.iter_mut().find(|a| a.name == "color-model").unwrap().value = ArgumentValue::Enum(2);
    assert!(stock::kind(&alien).is_none());
    assert!(map::filtered_flat_source_family(&alien).is_none());
    let mut alien = f.layers[1].effects[2].clone();
    alien.arguments.iter_mut().find(|a| a.name == "cpn-1-frequency").unwrap().value = ArgumentValue::Float32Bits(20.001f32.to_bits());
    assert!(stock::kind(&alien).is_none());
    let mut stack = f.clone();
    stack.layers = vec![stack.layers[1].clone()];
    stack.layers[0].blend_space = 0;
    stack.layers[0].composite_space = 2;
    stack.layers[0].composite_mode = 1;
    assert!(!hint(&stack, &Interrupt::NONE, &budget()).unwrap());
}
#[test]
#[ignore = "UNRUN: original work includes cheap-parser prepay and native/mask scans eveninactive, mapperprospective byte envelope independent ofwork, denial/cancel are final; no hidden gains validation"]
fn work_and_allocation_admission_are_separate_and_inactive_proof_stays() {
    let mut f = mixed(1, (65, 67));
    for d in &mut f.layers {
        d.visible = false;
        d.opacity = 0.;
        for e in &mut d.effects {
            e.state.enabled = Some(false);
        }
    }
    let b = budget();
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    let one = b.used(photocraft_xcf::BudgetKind::Work);
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 2 * one);
    assert!(one > 512 + 512 + 4096);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, &photocraft_xcf::Budget::new(limits, None)).is_err());
    let cancel = Interrupt::cancel_only(&|| true);
    assert!(matches!(qualify(&f, &cancel, &budget()), Err(IoError::Cancelled)));
    for e in &f.layers[1].effects {
        let n = 2 * e.arguments.len();
        let floor = n * std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>()
            + n * std::mem::size_of::<f64>()
            + std::mem::size_of::<Vec<photocraft_doc::xcf_filters::FilterArgument>>()
            + std::mem::size_of::<Vec<f64>>()
            + 64;
        assert!(stock::mapping_bytes(e).unwrap() >= floor);
    }
    let mut native = f.clone();
    native.layers[1].plane.as_mut().unwrap().bytes.pop();
    assert!(qualify(&native, &Interrupt::NONE, &budget()).is_err());
    let mut mask = f.clone();
    mask.layers[1].effects[2].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1;
    assert!(qualify(&mask, &Interrupt::NONE, &budget()).is_err());
    let mut later = f.clone();
    later.layers[1].effects[2].arguments[0].value = ArgumentValue::Enum(2);
    assert!(qualify(&later, &Interrupt::NONE, &budget()).is_err());
}
