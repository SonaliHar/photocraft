//! Ignored UNRUN source contracts. Genuine import buffers are external harness
//! inputs until actual reader admission; decoded mutations are not producer evidence.
use super::*;
use crate::xcf_filters::{
    self as map,
    mixed_affine_source::{self as affine, Parameters},
};
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use photocraft_xcf::ArgumentValue;
use sha2::{Digest, Sha256};
#[derive(Clone, Copy)]
struct Binding {
    path: &'static str,
    bytes: usize,
    sha256: &'static str,
}
struct Case {
    source: Binding,
    capture: Binding,
    precision: u32,
}
const CASES: [Case; 4] = [
    Case {
        source: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f32-8x6-mixed-affine-pair.xcf",
            bytes: 15540,
            sha256: "a0c8c940d02b6689ce2e0ba8a6beca7de56a7ec78b40d3cdea09bf7bf3e5e640",
        },
        capture: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f32-8x6-mixed-affine-pair.case.json",
            bytes: 374283,
            sha256: "5aacdd010cc174dc113bd2e0f42b1de99e9a5101d6db882b598a2d6c20c55336",
        },
        precision: 650,
    },
    Case {
        source: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f32-130x65-mixed-affine-pair.xcf",
            bytes: 374719,
            sha256: "b1c8971957701c795c5bc2d6a0ee905b6a6b2c4f78cab7b7af1dc5174e17a685",
        },
        capture: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f32-130x65-mixed-affine-pair.case.json",
            bytes: 375986,
            sha256: "30615b0e9e7d592948d284986e19825c90098a7395846741dc38653aaf340815",
        },
        precision: 650,
    },
    Case {
        source: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f64-8x6-mixed-affine-pair.xcf",
            bytes: 15636,
            sha256: "8c346ccb09200b40e016fc91a6c51746232bbf4e4b26fc59c81117c7f0a4cc78",
        },
        capture: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f64-8x6-mixed-affine-pair.case.json",
            bytes: 374546,
            sha256: "56df3a8c0e2a0a8ca5e035818609f6efa4cd2d57560fdd628d9cac5f82e38c90",
        },
        precision: 750,
    },
    Case {
        source: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f64-130x65-mixed-affine-pair.xcf",
            bytes: 331039,
            sha256: "f63d2d224848df7a76a3dc7c5db478a5362a24080acf9395e15436b0de441b46",
        },
        capture: Binding {
            path: "temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs/f64-130x65-mixed-affine-pair.case.json",
            bytes: 376176,
            sha256: "6510207752288fb459cef64d41628d9506e1ab9d73c2a9900334ca17bd4dd65b",
        },
        precision: 750,
    },
];
fn read(b: Binding) -> Vec<u8> {
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(b.path)).unwrap();
    assert_eq!(bytes.len(), b.bytes);
    assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), b.sha256);
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
fn owner() -> photocraft_doc::item_identity::ItemRef {
    photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1))
}
fn effect(f: &XcfFile, name: &str) -> photocraft_xcf::Effect {
    f.layers.iter().flat_map(|d| &d.effects).find(|e| e.operation == name).unwrap().clone()
}
#[test]
#[ignore = "UNRUN: four genuine affine originals both-depth 8x6/130x65 preserve raw/public node/argument order, own epochs/native/masks/source custody through actual same-q carrier; no numerical oracle"]
fn genuine_affine_originals_retain_each_own_node_and_native() {
    use photocraft_doc::{LayerContent, item_identity::ItemRef};
    for case in &CASES {
        let bytes = read(case.source);
        let _capture = read(case.capture);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        qualify(&raw, &Interrupt::NONE, &budget()).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let c = control();
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
        let imported = crate::import_owned_xcf_from_reader_in_quota(
            "affine original",
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::FilteredFlat(imported) = imported else { panic!("actual filtered-flat source") };
        imported.document().validate_readonly_filtered_flat_point_storage(&c).unwrap();
        let doc = imported.document().document();
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!((doc.layers.len(), state.filter_stacks.len(), state.filter_masks.len()), (8, 8, 18));
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, case.precision);
        assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
        for (layer, d) in doc.layers.iter().zip(raw.layers.iter().rev()) {
            let stack = state.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id)).unwrap();
            assert_eq!(stack.nodes.len(), d.effects.len());
            for (node, e) in stack.nodes.iter().zip(&d.effects) {
                assert_eq!(node.operation.name, e.operation);
                assert_eq!(node.operation.stored_version, e.version.as_deref().unwrap());
                assert_eq!(node.arguments.len(), e.arguments.len());
                if let Some(params) = affine::parameters(e) {
                    let expected = match params {
                        Parameters::BrightnessContrast(_) => photocraft_compose::BRIGHTNESS_CONTRAST_MIXED_EPOCH,
                        Parameters::Levels(_) => photocraft_compose::GEGL_LEVELS_MIXED_EPOCH,
                    };
                    assert_eq!(node.operation.schema_epoch, expected);
                    assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, expected);
                    let names: Vec<_> = e.arguments.iter().map(|a| a.name.as_str()).collect();
                    assert_eq!(
                        names,
                        if e.operation == "gegl:levels" { vec!["in-low", "in-high", "out-low", "out-high"] } else { vec!["contrast", "brightness"] }
                    );
                    for (a, b) in node.arguments.iter().zip(&e.arguments) {
                        let ArgumentValue::Float32Bits(bits) = b.value else { panic!("real F32") };
                        assert_eq!(a.name, b.name);
                        assert_eq!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(bits)));
                    }
                }
                let p = node.composition.as_ref().unwrap();
                assert!(p.resolved.is_none());
                assert_eq!(
                    [p.stored_blend_space, p.stored_composite_space, p.stored_operator],
                    [e.state.blend_space.unwrap(), e.state.composite_space.unwrap(), e.state.composite_mode.unwrap()]
                );
                for p in &e.properties {
                    let identity = format!("{}@{}:{}", p.tag, p.offset, p.declared_bytes);
                    let record = state
                        .opaque
                        .iter()
                        .find(|r| r.owner == Some(stack.owner) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
                        .unwrap();
                    assert_eq!(record.bytes.as_slice(), p.payload.as_slice());
                }
                let Some(ItemRef::Mask(id)) = node.mask else { panic!("own mask") };
                let mask = state.filter_masks.iter().find(|m| m.id == id && m.owner == node.id).unwrap();
                assert!(mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
                assert_eq!(mask.surface.try_read_interleaved(mask.bounds).unwrap(), e.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes);
            }
            let LayerContent::BackedRaster(native) = &layer.content else { panic!("native input") };
            let view = native.checked_view();
            let mut pixels = Vec::new();
            let mut pixel = vec![0; if case.precision == 650 { 16 } else { 32 }];
            for y in 0..raw.height {
                for x in 0..raw.width {
                    view.read_pixel_native_into(x as i32, y as i32, &mut pixel, &c).unwrap();
                    pixels.extend_from_slice(&pixel);
                }
            }
            assert_eq!(pixels, d.plane.as_ref().unwrap().bytes);
        }
        assert!(q.usage().unwrap().resident > 0);
        drop(imported);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}
#[test]
#[ignore = "UNRUN: exact two/four F32 scalar domains and per-node schema/operation pairing, no gdouble alias/tangent candidate/structured gimp levels; old one-root/one-node and PointStack selectors remain separate"]
fn scalar_schema_domains_and_old_route_priority() {
    use map::{NeighborhoodDomain as D, PointReplaceFamily as F};
    let f = decoded(0);
    for (name, family) in [("gegl:brightness-contrast", F::MixedBrightnessContrast), ("gegl:levels", F::MixedGeglLevels)] {
        let e = effect(&f, name);
        assert!(affine::parameters(&e).is_some());
        assert!(map::qualified_stack(std::slice::from_ref(&e), &[None], owner(), 1, 22, D::PointReplace(family)).is_err());
        let mapped = map::qualified_ordered_filtered_flat_source_stack(std::slice::from_ref(&e), &[None], owner(), 1, 22).unwrap();
        assert_eq!(mapped.nodes.len(), 1);
        let wrong = if name == "gegl:levels" { F::MixedBrightnessContrast } else { F::MixedGeglLevels };
        assert!(map::qualified_filtered_flat_source_stack(std::slice::from_ref(&e), &[None], owner(), 1, 22, D::PointReplace(wrong)).is_err());
        for bad in 0..7 {
            let mut x = e.clone();
            match bad {
                0 => x.version = Some("1:0".into()),
                1 => {
                    x.arguments.pop();
                }
                2 => x.arguments[0].name = "unexpected".into(),
                3 => x.arguments[0].value = ArgumentValue::Float64Bits(0),
                4 => x.arguments[0].value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
                5 => x.arguments[1].name = x.arguments[0].name.clone(),
                _ => x.operation = "gimp:levels".into(),
            }
            assert!(affine::parameters(&x).is_none());
        }
        let mut single = f.clone();
        let index = single.layers.iter().position(|d| d.effects.iter().any(|e| e.operation == name)).unwrap();
        single.layers = vec![single.layers[index].clone()];
        single.layers[0].effects = vec![e.clone()];
        assert!(!hint(&single, &Interrupt::NONE, &budget()).unwrap());
        assert!(crate::xcf_point_replace_source::classify_family_hint(&single) == Some(if name == "gegl:levels" { F::Levels } else { F::Brightness }));
        single.layers[0].effects.push(e);
        single.layers[0].blend_space = 0;
        single.layers[0].composite_space = 2;
        single.layers[0].composite_mode = 1;
        assert!(!hint(&single, &Interrupt::NONE, &budget()).unwrap());
    }
    let mut b = effect(&f, "gegl:brightness-contrast");
    b.arguments.iter_mut().find(|a| a.name == "brightness").unwrap().value = ArgumentValue::Float32Bits(3.0001f32.to_bits());
    assert!(affine::parameters(&b).is_none());
    let mut levels = effect(&f, "gegl:levels");
    levels.arguments.iter_mut().find(|a| a.name == "in-high").unwrap().value = ArgumentValue::Float32Bits(0.125f32.to_bits());
    assert!(affine::parameters(&levels).is_none());
}
#[test]
#[ignore = "UNRUN: explicitly declared SourceOpacity affine cross-product uses raw33 exact promotions and original purpose, external decoded controls only; genuine affine packets observe opacity1"]
fn declared_opacity_crossproduct_is_separately_sealed() {
    let original = decoded(0);
    for bits in [0u32, 0x80000000, 0.375f32.to_bits()] {
        let mut f = original.clone();
        for d in &mut f.layers {
            for e in &mut d.effects {
                e.state.opacity_bits = Some(bits);
                e.properties.iter_mut().find(|p| p.tag == 33).unwrap().payload = bits.to_be_bytes().to_vec();
            }
        }
        opacity::qualify(&f, &Interrupt::NONE, &budget()).unwrap();
        assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
        for d in &f.layers {
            let masks = vec![None; d.effects.len()];
            let s = map::qualified_ordered_filtered_flat_opacity_source_stack(&d.effects, &masks, owner(), 1, 22).unwrap();
            assert!(s.nodes.iter().all(|n| n.opacity_f64_bits == f64::from(f32::from_bits(bits)).to_bits()));
            assert!(map::qualified_ordered_filtered_flat_source_stack(&d.effects, &masks, owner(), 1, 22).is_err());
        }
        let bad = f.layers.iter_mut().flat_map(|d| &mut d.effects).find(|e| affine::parameters(e).is_some()).unwrap();
        bad.properties.retain(|p| p.tag != 33);
        assert!(opacity::qualify(&f, &Interrupt::NONE, &budget()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: complete source proof for inactive affine nodes keeps original Work/quota/cancel and final malformed later-native/mask refusal; logical controls do not replace physical mapper proof"]
fn inactive_native_mask_work_and_cancellation_remain_final() {
    let mut f = decoded(1);
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
    let lower = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, &photocraft_xcf::Budget::new(lower, None)).is_err());
    let cancel = Interrupt::cancel_only(&|| true);
    assert!(matches!(qualify(&f, &cancel, &budget()), Err(IoError::Cancelled)));
    let mut native = f.clone();
    native.layers.last_mut().unwrap().plane.as_mut().unwrap().bytes.pop();
    assert!(qualify(&native, &Interrupt::NONE, &budget()).is_err());
    let mut mask = f.clone();
    mask.layers.last_mut().unwrap().effects.last_mut().unwrap().mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1;
    assert!(qualify(&mask, &Interrupt::NONE, &budget()).is_err());
    let bytes = read(CASES[1].source);
    let q = BackingQuota::new(QuotaLimits { resident: 0, index: 64 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    assert!(
        crate::import_owned_xcf_from_reader_in_quota(
            "no quota",
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &control()
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap().resident, 0);
    assert_eq!(q.usage().unwrap().index, 0);
}
