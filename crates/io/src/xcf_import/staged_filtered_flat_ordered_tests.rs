//! UNRUN external decoded synthetic sources, not independent producer evidence.
//! Logical mask visitor controls do not substitute for mandatory full physical
//! defaults/padding proof in whole-document paid compose preparation.
use super::*;
use photocraft_xcf::{Argument, Compression, Effect, EffectState, Unit};
use std::sync::atomic::{AtomicBool, Ordering};
fn base_file<P>(plane: P, model: Model, sample: Sample, precision: u32, encoding: Encoding, geometry: bool, operation: &str) -> XcfFile<P> {
    let (width, height, owner_width, owner_height, offset) = if geometry { (295, 160, 257, 129, (-13, 13)) } else { (8, 6, 8, 6, (0, 0)) };
    XcfFile {
        paths: Vec::new(),
        tattoo: None,
        item_sets: Vec::new(),
        version: 22,
        width,
        height,
        model,
        precision,
        sample,
        encoding,
        compression: Compression::None,
        icc: Vec::new(),
        dpi: (72., 72.),
        unit: Unit::Inch,
        palette: Vec::new(),
        layers: vec![Drawable {
            width: owner_width,
            height: owner_height,
            model,
            alpha: true,
            offset,
            mode: 28,
            blend_space: 0,
            composite_space: -1,
            composite_mode: -1,
            plane: Some(plane),
            effects: vec![Effect {
                name: "predicate fixture".into(),
                icon: String::new(),
                operation: operation.into(),
                version: Some("0:0".into()),
                properties: Vec::new(),
                arguments: Vec::new(),
                mask: None,
                state: EffectState::default(),
            }],
            ..Default::default()
        }],
        channels: Vec::new(),
        guides: Vec::new(),
        diagnostics: Vec::new(),
        charged_bytes: 0,
        raw_properties: Vec::new(),
    }
}

fn effect(sample: Sample, size: (u32, u32), kind: u8) -> Effect<Plane> {
    let bytes = if sample == Sample::F32 { 0f32.to_ne_bytes().to_vec() } else { 0f64.to_ne_bytes().to_vec() };
    let mask = Drawable {
        width: size.0,
        height: size.1,
        model: Model::Gray,
        alpha: false,
        visible: true,
        opacity: 0.5,
        mode: 0,
        blend_space: 2,
        composite_space: 2,
        composite_mode: 1,
        mask_enabled: true,
        plane: Some(Plane { width: size.0, height: size.1, channels: 1, sample, bytes: bytes.repeat(size.0 as usize * size.1 as usize) }),
        ..Default::default()
    };
    let (op, arguments) = match kind {
        0 => (
            "gimp:semi-flatten",
            vec![Argument {
                name: "color".into(),
                property_index: 0,
                value: ArgumentValue::Color {
                    encoding: "RGBA float".into(),
                    samples: [0.25f32, 0.5, 0.75, 1.].into_iter().flat_map(|v| v.to_le_bytes()).collect(),
                    icc: Vec::new(),
                },
                configuration: None,
                graph: None,
            }],
        ),
        1 => (
            "gegl:hue-chroma",
            ["hue", "chroma", "lightness"]
                .into_iter()
                .zip([37.5f32, -12.5, 8.25])
                .map(|(name, v)| Argument {
                    name: name.into(),
                    property_index: 0,
                    value: ArgumentValue::Float32Bits(v.to_bits()),
                    configuration: None,
                    graph: None,
                })
                .collect(),
        ),
        _ => (
            "gegl:vibrance",
            ["vibrance", "saturation"]
                .into_iter()
                .zip([25f32, 1.5])
                .map(|(name, v)| Argument {
                    name: name.into(),
                    property_index: 0,
                    value: ArgumentValue::Float32Bits(v.to_bits()),
                    configuration: None,
                    graph: None,
                })
                .collect(),
        ),
    };
    Effect {
        name: "synthetic transport".into(),
        icon: String::new(),
        operation: op.into(),
        version: Some("0:0".into()),
        arguments,
        properties: Vec::new(),
        mask: Some(Box::new(mask)),
        state: EffectState {
            enabled: Some(true),
            mode: Some(62),
            opacity_bits: Some(1f32.to_bits()),
            blend_space: Some(i32::MIN),
            composite_space: Some(0),
            composite_mode: Some(i32::MAX),
            clipping: Some(0),
            region: Some(0),
            ..Default::default()
        },
    }
}
fn fixture(sample: Sample, size: (u32, u32)) -> XcfFile {
    let word = if sample == Sample::F32 { 0.5f32.to_ne_bytes().to_vec() } else { 0.5f64.to_ne_bytes().to_vec() };
    let p = Plane { width: size.0, height: size.1, sample, channels: 4, bytes: word.repeat(size.0 as usize * size.1 as usize * 4) };
    let mut f = base_file(p, Model::Rgb, sample, if sample == Sample::F32 { 650 } else { 750 }, Encoding::NonlinearProfile, false, "unused");
    f.width = size.0;
    f.height = size.1;
    f.layers[0].width = size.0;
    f.layers[0].height = size.1;
    f.layers[0].effects.clear();
    let plain = f.layers[0].clone();
    for kind in 0..3 {
        let mut root = plain.clone();
        root.effects = vec![effect(sample, size, kind)];
        f.layers.push(root);
    }
    f
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}

#[path = "../../../engine/src/accounted_filtered_flat_ordered_fixture.rs"]
mod inputs;
fn ordered(sample: Sample, count: usize) -> XcfFile {
    let mut f = fixture(sample, (65, 1));
    f.layers.truncate(1);
    f.layers[0].effects = (0..count).map(|n| effect(sample, (65, 1), (n % 3) as u8)).collect();
    f
}
#[test]
#[ignore = "UNRUN: genuine two-depth ordered originals preserve canonical raw/public list, native planes and every own mask; not numerical order acceptance"]
fn genuine_ordered_originals_retain_actual_global_transport() {
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
                    let epoch = match effect.operation.as_str() {
                        "gimp:semi-flatten" => photocraft_compose::SEMI_FLATTEN_EPOCH,
                        "gegl:hue-chroma" => photocraft_compose::HUE_CHROMA_EPOCH,
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
            for yy in 0..6 {
                for xx in 0..8 {
                    pin.read_pixel_native_into(xx, yy, &mut pixel, &c).unwrap();
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
#[test]
#[ignore = "UNRUN: synthetic single-root2/4/1024 success and1025 refusal, global4096 across4stacks; external decoded harness is not producer proof"]
fn topology_is_global_bounded_not_three_nodes_per_root() {
    use photocraft_doc::item_identity::ItemRef;
    for sample in [Sample::F32, Sample::F64] {
        for count in [2, 4, 1024] {
            let f = ordered(sample, count);
            let b = budget();
            assert!(hint(&f, &Interrupt::NONE, &b).unwrap());
            qualify(&f, &Interrupt::NONE, &b).unwrap();
            let masks = (0..count).map(|n| Some(ItemRef::Mask(photocraft_doc::item_identity::MaskId(n as u64 + 1)))).collect::<Vec<_>>();
            let stack = crate::xcf_filters::qualified_ordered_filtered_flat_source_stack(
                &f.layers[0].effects,
                &masks,
                ItemRef::Layer(photocraft_doc::LayerId(1)),
                100,
                22,
            )
            .unwrap();
            assert_eq!(stack.nodes.len(), count);
            for (n, node) in stack.nodes.iter().enumerate() {
                assert_eq!(node.id.0, 100 + n as u64);
                assert_eq!(node.operation.name, f.layers[0].effects[n].operation);
                assert_eq!(node.mask, masks[n]);
            }
        }
        let over_stack = ordered(sample, 1025);
        // Hint stays coarse so this matching overbound source is selected for
        // strict final refusal rather than falling back to another family.
        assert!(hint(&over_stack, &Interrupt::NONE, &budget()).unwrap());
        assert!(root_domain(&over_stack.layers[0], &Interrupt::NONE, &budget()).is_err());
        assert!(qualify(&over_stack, &Interrupt::NONE, &budget()).is_err());
        let mut global = ordered(sample, 1024);
        let root = global.layers[0].clone();
        for _ in 0..3 {
            global.layers.push(root.clone());
        }
        qualify(&global, &Interrupt::NONE, &budget()).unwrap();
        assert_eq!(global.layers.iter().map(|d| d.effects.len()).sum::<usize>(), 4096);
        global.layers.push(ordered(sample, 1).layers.remove(0));
        assert!(global.layers.iter().all(|d| d.effects.len() <= 1024));
        assert!(hint(&global, &Interrupt::NONE, &budget()).is_err());
        assert!(qualify(&global, &Interrupt::NONE, &budget()).is_err());
    }
    let mut old = ordered(Sample::F32, 1);
    assert!(!hint(&old, &Interrupt::NONE, &budget()).unwrap());
    assert!(qualify(&old, &Interrupt::NONE, &budget()).is_err());
    old.layers[0].effects.clear();
    assert!(!hint(&old, &Interrupt::NONE, &budget()).unwrap());
    let too_many = ordered(Sample::F32, 4097);
    assert!(hint(&too_many, &Interrupt::NONE, &budget()).is_err());
    assert!(qualify(&too_many, &Interrupt::NONE, &budget()).is_err());
}
#[test]
#[ignore = "UNRUN: malformed later ordered node cannot inherit first domain or hide in inactive source; old singleton classifier remains final"]
fn later_nodes_keep_independent_strict_envelopes() {
    for change in [
        (|e: &mut Effect| e.operation = "gegl:unknown".into()) as fn(&mut Effect),
        |e| e.version = Some("1:0".into()),
        |e| e.state.enabled = None,
        |e| e.state.opacity_bits = Some(0.5f32.to_bits()),
        |e| e.state.composite_space = None,
        |e| e.mask = None,
        |e| e.arguments.clear(),
        |e| e.mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&1f32.to_ne_bytes()),
    ] {
        let mut f = ordered(Sample::F32, 4);
        f.layers[0].visible = false;
        f.layers[0].opacity = 0.;
        for e in &mut f.layers[0].effects {
            e.state.enabled = Some(false)
        }
        change(&mut f.layers[0].effects[3]);
        assert!(hint(&f, &Interrupt::NONE, &budget()).unwrap());
        assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    }
    let f = ordered(Sample::F32, 1);
    assert!(!hint(&f, &Interrupt::NONE, &budget()).unwrap());
    let mut f = ordered(Sample::F32, 2);
    f.layers[0].effects[0].operation = "gegl:unknown".into();
    assert!(hint(&f, &Interrupt::NONE, &budget()).unwrap());
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
}
#[test]
#[ignore = "UNRUN: ordered source all-node scans consume one cumulative work ledger, with cancel and work refusal; full physical admission separately mandatory"]
fn every_node_uses_original_work_and_interrupt() {
    let f = ordered(Sample::F64, 4);
    let b = budget();
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    let first = b.used(photocraft_xcf::BudgetKind::Work);
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), first * 2);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", first - 1)]).unwrap();
    let limited = photocraft_xcf::Budget::new(limits, None);
    assert!(qualify(&f, &Interrupt::NONE, &limited).is_err());
    let stop = AtomicBool::new(true);
    let poll = || stop.load(Ordering::Relaxed);
    let i = Interrupt::cancel_only(&poll);
    assert!(matches!(qualify(&f, &i, &budget()), Err(IoError::Cancelled)));
    let mut bad = f.clone();
    let last = bad.layers[0].effects.last_mut().unwrap().mask.as_mut().unwrap().plane.as_mut().unwrap();
    last.bytes.pop();
    assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
}
