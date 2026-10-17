//! UNRUN contracts. Authentic XCF reads establish source custody; decoded
//! mutations are external harness inputs, never producer or paid-owner evidence.
use super::*;
use crate::xcf_filters::filtered_flat_opacity_source as scalar;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
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
}
const CASES: [Case; 8] = [
    Case {
        id: "f32-full",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-full.xcf",
            bytes: 11926,
            sha256: "4bbe3e8d39faa494d1f1390399c45bb6137b44104f3ccccd1416fc521af639f0",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-full.case.json",
            bytes: 231250,
            sha256: "e042c8d7d6f93448e544a121b3b67c757257e27cb2aee6efc4ee95f8735801b1",
        },
    },
    Case {
        id: "f32-partial",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-partial.xcf",
            bytes: 11926,
            sha256: "709d94119cf58b0396a887a0878112abb33b2b803ddb89c4661e016468ebed3d",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-partial.case.json",
            bytes: 231586,
            sha256: "b34181dd4a18bca6067ecaba4f3c15e6ef16095b31d23b442698c39dcde6dc5a",
        },
    },
    Case {
        id: "f32-zero",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-zero.xcf",
            bytes: 11926,
            sha256: "7cb5cfdd276b914c61133aaa6482fe9ff3466c91bfe8aece5dc421ebf276ebca",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-zero.case.json",
            bytes: 231250,
            sha256: "021d7afe230c3bfa6b7b268be8be6615b9289ca4c45a703f5f4420589864f9b6",
        },
    },
    Case {
        id: "f32-disabled",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-disabled.xcf",
            bytes: 11926,
            sha256: "404fafdf0e4039a796da902e5974ce2a410ff08f422130da45523f5afb16fc3d",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f32-author-v1/outputs/f32-disabled.case.json",
            bytes: 250915,
            sha256: "cdd677e55bfdee67b66e86db93ebdf52574505dcc8d4b1e78e282204f294d29d",
        },
    },
    Case {
        id: "f64-full",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-full.xcf",
            bytes: 11950,
            sha256: "2259d957306320dea2d85fa8feee7a52895697f7b90106c7652dc90d5bab210f",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-full.case.json",
            bytes: 231462,
            sha256: "d539db74b4fd1a2034624be31d4352fbbd61b8a10cda29b0194224e611056d83",
        },
    },
    Case {
        id: "f64-partial",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-partial.xcf",
            bytes: 11950,
            sha256: "5c94047f099f09c492b011fe4aa24e68fc33522cc770a6dda02c7b8264aed84b",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-partial.case.json",
            bytes: 231798,
            sha256: "2d8d3ab50f418242a44be10ff077a052560ab55f61ffdd1b13ee09e017ec1fad",
        },
    },
    Case {
        id: "f64-zero",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-zero.xcf",
            bytes: 11950,
            sha256: "3076c15cb8ac8621f31773a254cbcf8653d9be53fe06c26b461dcf2ff011c3e5",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-zero.case.json",
            bytes: 231462,
            sha256: "54bba676885acf8f6d12de674e5fde539d195ca70b26d3fb7984c1453ae32e88",
        },
    },
    Case {
        id: "f64-disabled",
        source: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-disabled.xcf",
            bytes: 11950,
            sha256: "0d90be5d064a0f175043e6ac072a24793b0b37079f49af8cb96460d6832ae1ef",
        },
        capture: Binding {
            path: "temp/sol1-filter-opacity-gimp-merge-released-20261016/f64-author-v1/outputs/f64-disabled.case.json",
            bytes: 251127,
            sha256: "137a0447b26a594d81403a68b5f0dfddc2e4fe7e66ae9564eb2185c1ad9af454",
        },
    },
];
fn read(b: Binding) -> Vec<u8> {
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(b.path)).unwrap();
    assert_eq!(bytes.len(), b.bytes);
    assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), b.sha256);
    bytes
}
fn decoded(index: usize) -> XcfFile {
    photocraft_xcf::decode(&read(CASES[index].source), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn set_opacity(e: &mut photocraft_xcf::Effect, bits: u32) {
    e.state.opacity_bits = Some(bits);
    let p = e.properties.iter_mut().find(|p| p.tag == 33).unwrap();
    p.payload = bits.to_be_bytes().to_vec();
}
#[test]
#[ignore = "UNRUN: eight genuine original mixed opacity files use the same-q owned source carrier and preserve every raw33/native/mask/profile; full sources keep old unit route; no render oracle"]
fn genuine_sources_keep_exact_words_and_paid_native_custody() {
    use photocraft_doc::{LayerContent, item_identity::ItemRef};
    for case in &CASES {
        let c = control();
        let bytes = read(case.source);
        let _capture = read(case.capture);
        let parsed = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let nonunit = opacity::has_nonunit(&parsed, &Interrupt::NONE, &budget()).unwrap();
        assert_eq!(nonunit, !case.id.ends_with("full"));
        if nonunit {
            opacity::qualify(&parsed, &Interrupt::NONE, &budget()).unwrap();
            assert!(qualify(&parsed, &Interrupt::NONE, &budget()).is_err());
        } else {
            qualify(&parsed, &Interrupt::NONE, &budget()).unwrap();
            assert!(opacity::qualify(&parsed, &Interrupt::NONE, &budget()).is_err());
        }
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
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
        let crate::OwnedXcfImport::FilteredFlat(source) = source else { panic!("mixed source carrier") };
        source.document().validate_readonly_filtered_flat_point_storage(&c).unwrap();
        let doc = source.document().document();
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 8);
        assert_eq!(state.filter_masks.len(), 12);
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, parsed.precision);
        assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert!(doc.icc_profile.is_some());
        for (layer, raw) in doc.layers.iter().zip(parsed.layers.iter().rev()) {
            let owner = ItemRef::Layer(layer.id);
            assert_eq!(layer.name, raw.name);
            assert_eq!(layer.visible, raw.visible);
            assert_eq!(layer.opacity.to_bits(), raw.opacity.to_bits());
            for p in &raw.raw_properties {
                let identity = format!("{}@{}:{}", p.tag, p.offset, p.declared_bytes);
                let kept = state.opaque.iter().find(|o| o.owner == Some(owner) && o.namespace == "gimp/xcf/source-property" && o.identity == identity).unwrap();
                assert_eq!(kept.bytes.as_slice(), p.payload.as_slice());
            }
            let stack = state.filter_stacks.iter().find(|s| s.owner == owner).unwrap();
            assert_eq!(stack.nodes.len(), raw.effects.len());
            for (node, effect) in stack.nodes.iter().zip(&raw.effects) {
                let bits = scalar::raw_opacity(effect).unwrap();
                assert_eq!(node.opacity_f64_bits, f64::from(f32::from_bits(bits)).to_bits());
                assert_eq!(node.enabled, effect.state.enabled.unwrap());
                assert_eq!(node.operation.name, effect.operation);
                assert_eq!(node.operation.stored_version, effect.version.as_deref().unwrap());
                assert_eq!(node.arguments.len(), effect.arguments.len());
                let composition = node.composition.as_ref().unwrap();
                assert!(composition.resolved.is_none());
                assert_eq!(
                    [composition.stored_blend_space, composition.stored_composite_space, composition.stored_operator],
                    [effect.state.blend_space.unwrap(), effect.state.composite_space.unwrap(), effect.state.composite_mode.unwrap()]
                );
                for p in &effect.properties {
                    let identity = format!("{}@{}:{}", p.tag, p.offset, p.declared_bytes);
                    let kept =
                        state.opaque.iter().find(|o| o.owner == Some(owner) && o.namespace == "gimp/xcf/source-property" && o.identity == identity).unwrap();
                    assert_eq!(kept.bytes.as_slice(), p.payload.as_slice());
                }
                let Some(ItemRef::Mask(id)) = node.mask else { panic!("own mask") };
                let mask = state.filter_masks.iter().find(|m| m.id == id && m.owner == node.id).unwrap();
                assert!(mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
                assert_eq!(mask.surface.try_read_interleaved(mask.bounds).unwrap(), effect.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes);
            }
            let LayerContent::BackedRaster(native) = &layer.content else { panic!("genuine native backing") };
            let pin = native.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &c).unwrap();
            let mut actual = Vec::new();
            let mut pixel = vec![0; if parsed.precision == 750 { 32 } else { 16 }];
            for y in 0..6 {
                for x in 0..8 {
                    pin.read_pixel_native_into(x, y, &mut pixel, &c).unwrap();
                    actual.extend_from_slice(&pixel);
                }
            }
            assert_eq!(actual, raw.plane.as_ref().unwrap().bytes);
        }
        assert!(q.usage().unwrap().resident > 0);
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}
#[test]
#[ignore = "UNRUN: separately sealed raw33 scalar contract, malformed widths/duplicates/tag6/typed mismatch/adjacent invalid words and old singleton/unit/PointStack policy isolation; external decoded mutations"]
fn raw_scalar_contract_and_old_routes_remain_distinct() {
    let base = decoded(1);
    let effect = base.layers.iter().flat_map(|d| &d.effects).next().unwrap();
    assert!(scalar::family(effect).is_some());
    assert_eq!(crate::xcf_filters::filtered_flat_source_family(effect).is_some(), scalar::raw_opacity(effect) == Some(1f32.to_bits()));
    for bits in [0u32, 0x80000000, 0.375f32.to_bits(), 1f32.to_bits()] {
        let mut e = effect.clone();
        set_opacity(&mut e, bits);
        assert_eq!(scalar::raw_opacity(&e), Some(bits));
        assert!(scalar::family(&e).is_some());
        assert_eq!(crate::xcf_filters::filtered_flat_source_family(&e).is_some(), bits == 1f32.to_bits());
        let owner = photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1));
        let mapped = crate::xcf_filters::qualified_ordered_filtered_flat_opacity_source_stack(std::slice::from_ref(&e), &[None], owner, 1, 22).unwrap();
        assert_eq!(mapped.nodes[0].opacity_f64_bits, f64::from(f32::from_bits(bits)).to_bits());
        if bits != 1f32.to_bits() {
            assert!(crate::xcf_filters::qualified_ordered_filtered_flat_source_stack(std::slice::from_ref(&e), &[None], owner, 1, 22).is_err());
            assert!(
                crate::xcf_filters::qualified_stack(
                    std::slice::from_ref(&e),
                    &[None],
                    owner,
                    1,
                    22,
                    crate::xcf_filters::NeighborhoodDomain::PointReplace(scalar::family(&e).unwrap())
                )
                .is_err()
            );
        }
    }
    for bits in [(-f32::from_bits(1)).to_bits(), 1f32.to_bits() + 1, f32::NAN.to_bits(), f32::INFINITY.to_bits()] {
        let mut e = effect.clone();
        set_opacity(&mut e, bits);
        assert!(scalar::raw_opacity(&e).is_none());
    }
    for bad in 0..8 {
        let mut e = effect.clone();
        match bad {
            0 => e.properties.retain(|p| p.tag != 33),
            1 => {
                let p = e.properties.iter().find(|p| p.tag == 33).unwrap().clone();
                e.properties.push(p);
            }
            2 => e.properties.iter_mut().find(|p| p.tag == 33).unwrap().declared_bytes = 8,
            3 => {
                e.properties.iter_mut().find(|p| p.tag == 33).unwrap().payload.pop();
            }
            4 => {
                let mut p = e.properties.iter().find(|p| p.tag == 33).unwrap().clone();
                p.tag = 6;
                e.properties.push(p);
            }
            5 => {
                assert_eq!(effect.state.opacity_bits, Some(1f32.to_bits()));
                e.state.opacity_bits = Some(0.5f32.to_bits());
            }
            6 => e.state.opacity_u8 = Some(95),
            _ => e.state.opacity_bits = None,
        }
        assert!(scalar::raw_opacity(&e).is_none());
        assert!(scalar::family(&e).is_none());
    }
    let mut single = base.clone();
    single.layers.truncate(1);
    single.layers[0].effects.truncate(1);
    assert!(opacity::qualify(&single, &Interrupt::NONE, &budget()).is_err());
    let mut old_identity = base.clone();
    old_identity.layers.truncate(1);
    old_identity.layers[0].blend_space = 0;
    old_identity.layers[0].composite_space = 2;
    old_identity.layers[0].composite_mode = 1;
    assert!(!hint(&old_identity, &Interrupt::NONE, &budget()).unwrap());
    for malformed in 0..4 {
        let mut f = base.clone();
        let e = f.layers[0].effects.last_mut().unwrap();
        match malformed {
            0 => e.state.enabled = None,
            1 => e.state.mode = Some(28),
            2 => e.state.blend_space = None,
            _ => e.version = Some("99:0".into()),
        }
        assert!(opacity::qualify(&f, &Interrupt::NONE, &budget()).is_err());
    }
    // Canonical CURRENT permutations are not inputs to this fresh-source proof.
    // Scalar raw groups alone never claim to recover same-opacity operation order.
}
#[test]
#[ignore = "UNRUN: original cumulative Work/cancel and inactive malformed native/mask remain final; decoded logical controls do not replace physical mapper/default/padding proof"]
fn inactive_sources_pay_complete_logical_proof_and_cancel() {
    let original = read(CASES[3].source);
    for deny_work in [false, true] {
        let q = BackingQuota::new(QuotaLimits { resident: if deny_work { 256 << 20 } else { 0 }, index: 64 << 20, scratch: 0 });
        let denied = |_| Err(photocraft_raster::backing::BackingError::Work);
        let c = if deny_work { ReadControl { interrupt: &Interrupt::NONE, work: &denied } } else { control() };
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
        assert!(
            crate::import_owned_xcf_from_reader_in_quota(
                "denied inactive source",
                &mut std::io::Cursor::new(&original),
                original.len() as u64,
                &Default::default(),
                &limits,
                q.clone(),
                &c
            )
            .is_err()
        );
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
    let mut f = decoded(3);
    for d in &mut f.layers {
        d.visible = false;
        d.opacity = 0.;
        for e in &mut d.effects {
            e.state.enabled = Some(false);
            set_opacity(e, 0);
        }
    }
    let b = budget();
    opacity::qualify(&f, &Interrupt::NONE, &b).unwrap();
    let one = b.used(photocraft_xcf::BudgetKind::Work);
    opacity::qualify(&f, &Interrupt::NONE, &b).unwrap();
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 2 * one);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
    assert!(opacity::qualify(&f, &Interrupt::NONE, &photocraft_xcf::Budget::new(limits, None)).is_err());
    let cancel = Interrupt::cancel_only(&|| true);
    assert!(matches!(opacity::qualify(&f, &cancel, &budget()), Err(IoError::Cancelled)));
    let mut short = f.clone();
    short.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(opacity::qualify(&short, &Interrupt::NONE, &budget()).is_err());
    let mut hdr = f.clone();
    let p = hdr.layers[0].plane.as_mut().unwrap();
    p.bytes[..4].copy_from_slice(&1.25f32.to_ne_bytes());
    assert!(opacity::qualify(&hdr, &Interrupt::NONE, &budget()).is_err());
    let mut mask = f.clone();
    let p = mask.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
    p.bytes[..4].copy_from_slice(&0.25f32.to_ne_bytes());
    assert!(opacity::qualify(&mask, &Interrupt::NONE, &budget()).is_err());
}
