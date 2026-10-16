//! UNRUN source transport/regression drafts. Decoded/source/reference buffers
//! are external harness data; genuine import alone establishes actual receipts.
use super::*;
use photocraft_xcf::ArgumentValue;
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
mod inputs {
    #[derive(Clone, Copy)]
    pub struct Binding {
        pub path: &'static str,
        pub bytes: usize,
        pub sha256: &'static str,
    }
    #[derive(Clone, Copy)]
    pub struct Case {
        pub source: Binding,
        pub capture: Binding,
        pub precision: u32,
    }
    pub const ROOT_NAMES: [&str; 4] = ["Own root 0", "Own root 1", "Own root 2", "Own root 3"];
    pub const OPERATIONS: [&[&str]; 4] = [
        &[],
        &["gegl:exposure"],
        &["gegl:vibrance", "gegl:hue-chroma", "gimp:semi-flatten", "gegl:exposure"],
        &["gegl:exposure", "gimp:semi-flatten", "gegl:hue-chroma", "gegl:vibrance"],
    ];
    pub const CASES: [Case; 4] = [
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f32-plain-exposure-ordered-point.xcf",
                bytes: 8051,
                sha256: "b41ff7782734b803494e43a0274f0789757aa67902e335933455e4c58179ca2c",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f32-plain-exposure-ordered-point.case.json",
                bytes: 93477,
                sha256: "d8a67ff8232c162ed1ac04727ff4e642f8b30c36435e788d8b8318782d71cf7e",
            },
            precision: 650,
        },
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f64-plain-exposure-ordered-point.xcf",
                bytes: 8113,
                sha256: "ffa38184286dfa0ef1462223bb55e217822bed5595b350af0ff7ce786c71f015",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f64-plain-exposure-ordered-point.case.json",
                bytes: 93565,
                sha256: "a30284ae72d28587c5f8591fdd7f9136d7059bcb98f1932f331a6e2981e12df9",
            },
            precision: 750,
        },
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-exposure-geometry-gimp-20261016/author-v1/outputs/f32-plain-exposure-ordered-point.xcf",
                bytes: 187751,
                sha256: "c99eabab787ff04f96f24a22f9b2418b6207b5b846514e054543c5926c7575b3",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-exposure-geometry-gimp-20261016/author-v1/outputs/f32-plain-exposure-ordered-point.case.json",
                bytes: 94102,
                sha256: "00064ea3b26ee3d0f54f44d5e5d73ac1d42da35ee2cf193660ed3b386fe8712d",
            },
            precision: 650,
        },
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-exposure-geometry-gimp-20261016/author-v1/outputs/f64-plain-exposure-ordered-point.xcf",
                bytes: 165189,
                sha256: "17d4b8cb7082aec737baa29ec707689c8902303ca303e02f66f33830febea421",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-exposure-geometry-gimp-20261016/author-v1/outputs/f64-plain-exposure-ordered-point.case.json",
                bytes: 94158,
                sha256: "c384b9e6f05400647b81a459ef15b2585eec4460cdea5fef869a3752a2304134",
            },
            precision: 750,
        },
    ];
}
#[test]
#[ignore = "UNRUN: genuine8x6/130x65 both-depth Exposure originals preserve canonical raw/public list, native planes and every own mask; not numerical order acceptance"]
fn genuine_exposure_originals_preserve_each_node_and_all_native_custody() {
    use photocraft_doc::{LayerContent, item_identity::ItemRef};
    use photocraft_raster::backing::{BackingQuota, QuotaKind, QuotaLimits, ReadControl};
    use sha2::{Digest, Sha256};
    for case in inputs::CASES {
        let read = |binding: inputs::Binding| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(binding.path);
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(bytes.len(), binding.bytes);
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), binding.sha256);
            bytes
        };
        let bytes = read(case.source);
        let _capture = read(case.capture);
        let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        qualify(&f, &Interrupt::NONE, &budget()).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
        let source = crate::import_owned_xcf_from_reader_in_quota(
            "ordered original",
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::FilteredFlat(source) = source else { panic!("sealed ordered carrier") };
        source.document().validate_readonly_filtered_flat_point_storage(&c).unwrap();
        let d = source.document().document();
        let x = d.xcf_state.as_ref().unwrap();
        assert_eq!((d.layers.len(), x.filter_stacks.len(), x.filter_masks.len(), x.items.len()), (4, 3, 9, 13));
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
        for (index, (layer, raw)) in d.layers.iter().zip(f.layers.iter().rev()).enumerate() {
            assert_eq!(layer.name, inputs::ROOT_NAMES[index]);
            let stack = x.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id));
            if raw.effects.is_empty() {
                assert!(stack.is_none())
            } else {
                let stack = stack.unwrap();
                assert_eq!(stack.nodes.len(), raw.effects.len());
                for (ordinal, (node, effect)) in stack.nodes.iter().zip(&raw.effects).enumerate() {
                    assert_eq!(node.operation.name, inputs::OPERATIONS[index][ordinal]);
                    assert_eq!(node.operation.name, effect.operation);
                    assert_eq!(node.operation.stored_version, effect.version.as_deref().unwrap());
                    if effect.operation == "gegl:exposure" {
                        assert_eq!(node.arguments.len(), 2);
                        for (a, b) in node.arguments.iter().zip(&effect.arguments) {
                            assert_eq!(a.name, b.name);
                            let ArgumentValue::Float32Bits(bits) = b.value else { panic!("actual F32") };
                            assert_eq!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(bits)));
                        }
                    }
                    let epoch = match effect.operation.as_str() {
                        "gimp:semi-flatten" => photocraft_compose::SEMI_FLATTEN_EPOCH,
                        "gegl:hue-chroma" => photocraft_compose::HUE_CHROMA_EPOCH,
                        "gegl:exposure" => photocraft_compose::EXPOSURE_MIXED_EPOCH,
                        _ => photocraft_compose::VIBRANCE_EPOCH,
                    };
                    assert_eq!(node.operation.schema_epoch, epoch);
                    assert_eq!(node.arguments.len(), effect.arguments.len());
                    let p = node.composition.as_ref().unwrap();
                    assert_eq!(p.compatibility_epoch, epoch);
                    assert!(p.resolved.is_none());
                    assert_eq!(
                        [p.stored_blend_space, p.stored_composite_space, p.stored_operator],
                        [effect.state.blend_space.unwrap(), effect.state.composite_space.unwrap(), effect.state.composite_mode.unwrap()]
                    );
                    let Some(ItemRef::Mask(id)) = node.mask else { panic!("own mask") };
                    let mask = x.filter_masks.iter().find(|m| m.id == id && m.owner == node.id).unwrap();
                    assert_eq!(mask.bounds, d.bounds());
                    assert!(mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
                    assert_eq!(mask.surface.try_read_interleaved(mask.bounds).unwrap(), effect.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes);
                    assert_eq!(node.enabled, effect.state.enabled.unwrap());
                }
            }
            let LayerContent::BackedRaster(native) = &layer.content else { panic!("actual original native provider") };
            let pin = native.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &c).unwrap();
            let mut captured = Vec::new();
            let mut pixel = vec![0; if case.precision == 650 { 16 } else { 32 }];
            for yy in 0..f.height {
                for xx in 0..f.width {
                    pin.read_pixel_native_into(xx as i32, yy as i32, &mut pixel, &c).unwrap();
                    captured.extend_from_slice(&pixel);
                }
            }
            assert_eq!(captured, raw.plane.as_ref().unwrap().bytes);
        }
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}

fn decoded(index: usize) -> XcfFile {
    let binding = inputs::CASES[index].source;
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(binding.path)).unwrap();
    photocraft_xcf::decode(&bytes, &Default::default()).unwrap()
}
#[test]
#[ignore = "UNRUN: later-node version/args/F32 endpoints/raw/mask/source-word failures remain final even inactive; explicit mixed activation only; old singleton route exact"]
fn independent_exposure_authority_and_old_identity() {
    use crate::xcf_filters::mixed_exposure_source::source_geometry;
    for (width, height) in [(0, 65), (130, 0), (262145, 1), (1, 262145), (262144, 4097)] {
        assert!(!source_geometry(width, height));
    }
    assert!(source_geometry(130, 65));
    assert!(source_geometry(262144, 4096)); // metadata bound only, no allocation/source claim
    for kind in 0..15 {
        let mut f = decoded(0);
        f.layers[1].visible = false;
        f.layers[1].opacity = 0.;
        for e in &mut f.layers[1].effects {
            e.state.enabled = Some(false);
        }
        let e = &mut f.layers[1].effects[3];
        assert_eq!(e.operation, "gegl:exposure");
        match kind {
            0 => e.version = Some("0:0".into()),
            1 => e.arguments[1].name = e.arguments[0].name.clone(),
            2 => {
                e.arguments.pop();
            }
            3 => e.arguments[0].value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
            4 => e.arguments.iter_mut().find(|a| a.name == "black-level").unwrap().value = ArgumentValue::Float32Bits(0.1f32.to_bits()),
            5 => e.arguments.iter_mut().find(|a| a.name == "black-level").unwrap().value = ArgumentValue::Float32Bits((-0.1f32).to_bits()),
            6 => e.state.enabled = None,
            7 => e.state.opacity_bits = Some(0.5f32.to_bits()),
            8 => e.state.composite_space = None,
            9 => e.mask = None,
            10 => e.mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&1f32.to_ne_bytes()),
            11 => f.layers[1].plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&f32::INFINITY.to_ne_bytes()),
            12 => e.arguments[0].value = ArgumentValue::Float64Bits(0.5f64.to_bits()),
            13 => e.state.composite_mode = None,
            _ => e.operation = "gegl:unknown".into(),
        }
        assert!(hint(&f, &Interrupt::NONE, &budget()).unwrap());
        assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    }
    let mut positive = decoded(0);
    positive.layers[0].visible = false;
    positive.layers[0].opacity = 0.375;
    for e in &mut positive.layers[0].effects {
        e.state.enabled = Some(false);
    }
    qualify(&positive, &Interrupt::NONE, &budget()).unwrap();
    let e = &positive.layers[0].effects[0];
    assert!(
        crate::xcf_filters::qualified_stack(
            std::slice::from_ref(e),
            &[None],
            photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
            1,
            22,
            crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::MixedExposure)
        )
        .is_err()
    );
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/exposure-black-level-precision-evidence/rgb-float_non_linear.source.xcf");
    let oldbytes = std::fs::read(path).unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(format!("{:x}", Sha256::digest(&oldbytes)), "11f0242cedd1e70854903f0d85868ae8d173666f6e6192d17616790d0b97bfa0");
    let old = photocraft_xcf::decode(&oldbytes, &Default::default()).unwrap();
    assert!(!hint(&old, &Interrupt::NONE, &budget()).unwrap());
    assert!(matches!(
        crate::xcf_point_replace_source::classify(&old, &Interrupt::NONE, None).unwrap(),
        Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::Exposure))
    ));
}
#[test]
#[ignore = "UNRUN: original shared work/cancel remains cumulative for every inactive native and mask; logical qualifier not full preparation substitute"]
fn inactive_work_and_cancellation_are_effective() {
    let mut f = decoded(1);
    for root in &mut f.layers {
        root.visible = false;
        root.opacity = 0.;
        for e in &mut root.effects {
            e.state.enabled = Some(false);
        }
    }
    let b = budget();
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    let one = b.used(photocraft_xcf::BudgetKind::Work);
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 2 * one);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, &photocraft_xcf::Budget::new(limits, None)).is_err());
    let cancel = Interrupt::cancel_only(&|| true);
    assert!(matches!(qualify(&f, &cancel, &budget()), Err(IoError::Cancelled)));
    f.layers.last_mut().unwrap().plane.as_mut().unwrap().bytes.pop();
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
}
