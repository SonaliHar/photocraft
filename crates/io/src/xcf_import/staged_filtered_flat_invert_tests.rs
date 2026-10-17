//! UNRUN source transport and old-priority regression drafts. External decoded
//! buffers are harness data; only actual imports establish paid source custody.
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
    pub const OPERATIONS: [&[&str]; 8] = [
        &[],
        &["gegl:invert-linear"],
        &["gegl:invert-gamma"],
        &["gegl:exposure", "gegl:invert-linear"],
        &["gegl:invert-linear", "gegl:exposure"],
        &["gegl:exposure", "gegl:invert-gamma"],
        &["gegl:invert-gamma", "gegl:exposure"],
        &["gegl:vibrance", "gegl:hue-chroma", "gimp:semi-flatten", "gegl:invert-gamma", "gegl:invert-linear"],
    ];
    pub const CASES: [Case; 4] = [
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-8x6-mixed-inverts.xcf",
                bytes: 13247,
                sha256: "8948b71cbb257c700ce05145e1695d4cca599f291a7277ac101a232ad54acb46",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-8x6-mixed-inverts.case.json",
                bytes: 158805,
                sha256: "4cffc8140cb35720cafda6c71cb2866d53422903b5ee426169d6af010c17f1c2",
            },
            precision: 650,
        },
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-130x65-mixed-inverts.xcf",
                bytes: 371994,
                sha256: "cb9e5bb196a17f04acbc755e2c21df26e4478a791a22f469e8b799b744ec7aa2",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-130x65-mixed-inverts.case.json",
                bytes: 160197,
                sha256: "1b9331db984dc4ddbae54b156a5f8ff804958d024564437fcc3747d3318615ff",
            },
            precision: 650,
        },
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-8x6-mixed-inverts.xcf",
                bytes: 13319,
                sha256: "54aff2884dea1e361b1d5d6765fb51a9366a4c39b02ac842b2c71ed46ec68c08",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-8x6-mixed-inverts.case.json",
                bytes: 158969,
                sha256: "63943e30e92f0faf1672104fe2b19162536ee40853589c24e515223e222032e1",
            },
            precision: 750,
        },
        Case {
            source: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-130x65-mixed-inverts.xcf",
                bytes: 328098,
                sha256: "e6c830205356fb140e6a0b8df68dd5b6602fc669391c4c43c7156e0b46c7d8b3",
            },
            capture: Binding {
                path: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-130x65-mixed-inverts.case.json",
                bytes: 160301,
                sha256: "6c38fa63da38132675537f568ddec958c2b80dad60bd09bc20fe07f14f17d324",
            },
            precision: 750,
        },
    ];
}
#[test]
#[ignore = "UNRUN: genuine8x6/130x65 both-depth invert originals preserve canonical raw/public list, native planes and every own mask; not numerical order acceptance"]
fn genuine_invert_originals_preserve_each_node_and_all_native_custody() {
    use photocraft_doc::{LayerContent, item_identity::ItemRef};
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    use sha2::{Digest, Sha256};
    for case in inputs::CASES {
        let read = |binding: inputs::Binding| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(binding.path);
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(bytes.len(), binding.bytes);
            assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), binding.sha256);
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
        assert_eq!((d.layers.len(), x.filter_stacks.len(), x.filter_masks.len(), x.items.len()), (8, 7, 15, 23));
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
        for (index, (layer, raw)) in d.layers.iter().zip(f.layers.iter().rev()).enumerate() {
            assert_eq!(layer.name, raw.name);
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
                    if matches!(effect.operation.as_str(), "gegl:invert-linear" | "gegl:invert-gamma") {
                        assert!(node.arguments.is_empty());
                    }
                    let epoch = match effect.operation.as_str() {
                        "gimp:semi-flatten" => photocraft_compose::SEMI_FLATTEN_EPOCH,
                        "gegl:hue-chroma" => photocraft_compose::HUE_CHROMA_EPOCH,
                        "gegl:exposure" => photocraft_compose::EXPOSURE_MIXED_EPOCH,
                        "gegl:invert-linear" => photocraft_compose::INVERT_LINEAR_MIXED_EPOCH,
                        "gegl:invert-gamma" => photocraft_compose::INVERT_GAMMA_MIXED_EPOCH,
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
        assert!(q.usage().unwrap().resident > 0);
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}

fn decoded(index: usize) -> XcfFile {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(inputs::CASES[index].source.path);
    photocraft_xcf::decode(&std::fs::read(path).unwrap(), &Default::default()).unwrap()
}
#[test]
#[ignore = "UNRUN: later own-kind/version/zeroargs/raw/physical-logical mask and inactive native authority; no singleton epoch alias"]
fn each_invert_identity_is_sealed_and_later_failure_is_final() {
    use crate::xcf_filters::{
        NeighborhoodDomain as D, PointReplaceFamily as F,
        mixed_invert_source::{Kind, kind},
    };
    for which in [Kind::Linear, Kind::Gamma] {
        let mut f = decoded(0);
        let ordinal = f.layers[0].effects.iter().position(|e| kind(e) == Some(which)).unwrap();
        let e = &f.layers[0].effects[ordinal];
        let family = if which == Kind::Linear { F::MixedInvertLinear } else { F::MixedInvertGamma };
        assert!(
            crate::xcf_filters::qualified_stack(
                std::slice::from_ref(e),
                &[None],
                photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
                1,
                22,
                D::PointReplace(family)
            )
            .is_err()
        );
        // Wrong paired kind cannot map even with the exact complete Replace envelope.
        let wrong = if which == Kind::Linear { F::MixedInvertGamma } else { F::MixedInvertLinear };
        assert!(
            crate::xcf_filters::qualified_filtered_flat_source_stack(
                std::slice::from_ref(e),
                &[None],
                photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
                1,
                22,
                D::PointReplace(wrong)
            )
            .is_err()
        );
        f.layers[0].visible = false;
        f.layers[0].opacity = 0.;
        for e in &mut f.layers[0].effects {
            e.state.enabled = Some(false);
        }
        qualify(&f, &Interrupt::NONE, &budget()).unwrap();
        for control in 0..12 {
            let mut bad = f.clone();
            let e = &mut bad.layers[0].effects[ordinal];
            match control {
                0 => e.version = Some("1:0".into()),
                1 => e.arguments.push(photocraft_xcf::Argument {
                    name: "unexpected".into(),
                    property_index: 0,
                    value: ArgumentValue::Float32Bits(0),
                    configuration: None,
                    graph: None,
                }),
                2 => e.state.enabled = None,
                3 => e.state.opacity_bits = Some(0.5f32.to_bits()),
                4 => e.state.blend_space = None,
                5 => e.state.composite_space = None,
                6 => e.state.composite_mode = None,
                7 => e.mask = None,
                8 => e.mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&1f32.to_ne_bytes()),
                9 => bad.layers[0].plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&f32::INFINITY.to_ne_bytes()),
                10 => bad.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.1f32.to_ne_bytes()),
                _ => e.operation = "gegl:unknown".into(),
            }
            assert!(hint(&bad, &Interrupt::NONE, &budget()).unwrap());
            assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
        }
    }
    // Source unitRGB remains strict despite derivedCURRENT finiteRGB support.
    let mut hdr = decoded(2);
    hdr.layers[0].plane.as_mut().unwrap().bytes[0..8].copy_from_slice(&1.01f64.to_ne_bytes());
    assert!(qualify(&hdr, &Interrupt::NONE, &budget()).is_err());
}
#[test]
#[ignore = "UNRUN: same original Work ledger and cancellation apply to every inactive invert native/mask; no logical-only proof replaces actual mapper preparation"]
fn inactive_work_is_cumulative_and_cancellation_effective() {
    let mut f = decoded(3);
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
#[test]
#[ignore = "UNRUN: genuine original PointStack selects old positive carrier; fixed rawidentity exclusion remains final on malformed native; no U8/acceptance fallback"]
fn original_point_stack_priority_is_preserved() {
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    use sha2::{Digest, Sha256};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/filter-stack-edits-evidence/stack.source.xcf");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len(), 2348);
    assert_eq!(
        Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "414a7c66f0cbb78d772b2766592472353acb2e520bec71c9f46daee3070c12f8"
    );
    let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    assert!(crate::xcf_import::qualified_point_stack_edits(&f));
    let b = budget();
    assert!(!hint(&f, &Interrupt::NONE, &b).unwrap());
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 8);
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 });
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    // Preserve the original real refusal: captured input remains charged while
    // a complete preparation envelope equal to q.resident cannot fit.
    let before = q.usage().unwrap();
    let denied = crate::import_owned_xcf_from_reader_in_quota(
        "old original",
        &mut std::io::Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &c,
    );
    assert!(denied.is_err());
    assert_eq!(q.usage().unwrap(), before);
    // Lower the preparation envelope; never increase the parent quota.
    let limits = limits.lowered(&[("residentBytes", 64 << 20), ("indexBytes", 16 << 20)]).unwrap();
    let imported = crate::import_owned_xcf_from_reader_in_quota(
        "old original",
        &mut std::io::Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &c,
    )
    .unwrap();
    assert!(matches!(imported, crate::OwnedXcfImport::Point(_)));
    let mut bad = f.clone();
    bad.layers[0].plane.as_mut().unwrap().bytes.pop();
    // Metadata identity remains old despite unchanged header and broken native.
    assert!(!hint(&bad, &Interrupt::NONE, &budget()).unwrap());
    assert!(!crate::xcf_import::qualified_point_stack_edits(&bad));
    assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
    let mut changed = f.clone();
    changed.layers[0].effects.swap(0, 1);
    assert!(!hint(&changed, &Interrupt::NONE, &budget()).unwrap());
    changed.layers[0].blend_space = 0;
    changed.layers[0].composite_space = -1;
    changed.layers[0].composite_mode = -1;
    assert!(hint(&changed, &Interrupt::NONE, &budget()).unwrap());
    assert!(qualify(&changed, &Interrupt::NONE, &budget()).is_err());
}
