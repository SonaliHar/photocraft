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
        mask: Some(mask),
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
#[test]
#[ignore = "UNRUN: synthetic mixed actual roots preserve leaf policy/raw/native order; not producer proof"]
fn mixed_three_filters_plus_plain_keep_exact_root_local_domains() {
    for sample in [Sample::F32, Sample::F64] {
        for size in [(1, 1), (65, 1), (257, 259)] {
            let f = fixture(sample, size);
            let b = budget();
            assert!(hint(&f, &Interrupt::NONE, &b).unwrap());
            qualify(&f, &Interrupt::NONE, &b).unwrap();
            let domains = f.layers.iter().map(|d| root_domain(d, &Interrupt::NONE, &b).unwrap()).collect::<Vec<_>>();
            assert_eq!(
                domains,
                vec![
                    None,
                    Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::SemiFlatten)),
                    Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::HueChroma)),
                    Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::Vibrance))
                ]
            );
            for root in f.layers.iter().skip(1) {
                let e = &root.effects[0];
                assert_eq!((e.state.blend_space, e.state.composite_space, e.state.composite_mode), (Some(i32::MIN), Some(0), Some(i32::MAX)));
            }
            let mut selected = f.clone();
            selected.layers[2].active = true;
            qualify(&selected, &Interrupt::NONE, &b).unwrap(); // root editing selection is data, not pixel selection.
        }
    }
}
#[test]
#[ignore = "UNRUN: strict source activity/roles/header and later-root refusal; CURRENT grants separate"]
fn malformed_later_root_never_falls_back_to_plain_or_singleton() {
    let base = fixture(Sample::F32, (65, 1));
    for change in [
        (|f: &mut XcfFile| f.layers[3].visible = false) as fn(&mut XcfFile),
        |f| f.layers[3].opacity = 0.375,
        |f| f.layers[3].effects[0].state.enabled = Some(false),
        |f| f.layers[3].effects[0].state.composite_space = None,
        |f| f.layers[3].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
        |f| f.layers[3].effects[0].version = Some("1:0".into()),
        |f| f.layers[3].offset = (1, 0),
        |f| f.layers[3].legacy_linked = true,
        |f| f.layers[3].selection = true,
        |f| f.layers[3].mode = 30,
        |f| f.layers[3].effects[0].mask = None,
        |f| f.layers[3].effects[0].mask.as_mut().unwrap().width -= 1,
        |f| f.icc.push(1),
        |f| f.precision = 600,
        |f| f.layers[3].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.25f32.to_ne_bytes()),
    ] {
        let mut f = base.clone();
        change(&mut f);
        let b = budget();
        assert!(hint(&f, &Interrupt::NONE, &b).unwrap());
        assert!(qualify(&f, &Interrupt::NONE, &b).is_err());
    }
    let mut plain = base.clone();
    for d in &mut plain.layers {
        d.effects.clear();
    }
    assert!(!hint(&plain, &Interrupt::NONE, &budget()).unwrap());
    assert!(qualify(&plain, &Interrupt::NONE, &budget()).is_err());
    let mut singleton = base.clone();
    singleton.layers.truncate(1);
    assert!(!hint(&singleton, &Interrupt::NONE, &budget()).unwrap());
}
#[test]
#[ignore = "UNRUN: all-root logical native length/unit and zero-mask bytes; physical proof remains L3"]
fn every_native_and_mask_is_validated_and_signed_zero_retained() {
    for sample in [Sample::F32, Sample::F64] {
        let mut f = fixture(sample, (65, 1));
        let mask = f.layers[3].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
        let word = if sample == Sample::F32 { (-0f32).to_ne_bytes().to_vec() } else { (-0f64).to_ne_bytes().to_vec() };
        mask.bytes[..word.len()].copy_from_slice(&word);
        let before = mask.bytes.clone();
        qualify(&f, &Interrupt::NONE, &budget()).unwrap();
        assert_eq!(f.layers[3].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes, before);
        let mut truncated = f.clone();
        truncated.layers[0].plane.as_mut().unwrap().bytes.pop();
        assert!(qualify(&truncated, &Interrupt::NONE, &budget()).is_err());
        let mut bad = f.clone();
        let p = bad.layers[3].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
        let word = if sample == Sample::F32 { 1f32.to_ne_bytes().to_vec() } else { 1f64.to_ne_bytes().to_vec() };
        p.bytes[..word.len()].copy_from_slice(&word);
        assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
        let mut bad = f.clone();
        bad.layers[2].plane.as_mut().unwrap().channels = 2;
        assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: real shared qualifier work/cancel, external decoded source buffers not receipts"]
fn shared_control_is_cumulative_and_late_budget_refusal_is_final() {
    let f = fixture(Sample::F32, (65, 1));
    let b = budget();
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    let used = b.used(photocraft_xcf::BudgetKind::Work);
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), used * 2);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", used - 1)]).unwrap();
    let low = photocraft_xcf::Budget::new(limits, None);
    assert!(qualify(&f, &Interrupt::NONE, &low).is_err());
    let flag = AtomicBool::new(true);
    let stop = || flag.load(Ordering::SeqCst);
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), &b), Err(IoError::Cancelled)));
    assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), used * 2);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "UNRUN: independently authored four-root original transport with genuine reader/quota promotion; not numerical acceptance"]
fn authored_four_root_sources_keep_native_filter_mask_and_profile_custody() {
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    };
    use sha2::{Digest, Sha256};
    for (name, hash) in [
        ("f32-plain-semi-hue-vibrance.xcf", "25b2e44cff477af09f4677db4777416f324af0432aceb69341155782619667da"),
        ("f64-plain-semi-hue-vibrance.xcf", "14f2563bbb37ecd6fdaabb5d6e00bf5dd6863f5456ab485913c54fae2e330c66"),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/root-filtered-flat-gimp-20261016/author-v1/outputs").join(name);
        let bytes = std::fs::read(path).expect("explicit independent GIMP research input");
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(f.layers.len(), 4);
        qualify(&f, &Interrupt::NONE, &budget()).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
        let owned = crate::import_owned_xcf_from_reader_in_quota(
            name,
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::FilteredFlat(source) = owned else { panic!("strict filtered-flat carrier") };
        let owner = source.document();
        owner.validate_readonly_filtered_flat_point_storage(&c).unwrap();
        assert!(!owner.is_readonly_point_storage());
        assert!(!owner.is_readonly_flat_composite_storage());
        let d = owner.document();
        assert_eq!(d.layers.len(), 4);
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        let x = d.xcf_state.as_ref().unwrap();
        assert_eq!((x.filter_stacks.len(), x.filter_masks.len(), x.items.len()), (3, 3, 7));
        for (parsed, current) in f.layers.iter().zip(&d.layers) {
            let LayerContent::BackedRaster(native) = &current.content else { panic!("real native backing") };
            let mut captured = Vec::new();
            let pin = native.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &c).unwrap();
            let bpp = if f.sample == Sample::F64 { 32 } else { 16 };
            let mut pixel = vec![0; bpp];
            for y in 0..6 {
                for xx in 0..8 {
                    pin.read_pixel_native_into(xx, y, &mut pixel, &c).unwrap();
                    captured.extend_from_slice(&pixel);
                }
            }
            assert_eq!(captured, parsed.plane.as_ref().unwrap().bytes);
            if let Some(effect) = parsed.effects.first() {
                let stack = x.filter_stacks.iter().find(|s| s.owner == photocraft_doc::item_identity::ItemRef::Layer(current.id)).unwrap();
                let node = &stack.nodes[0];
                assert_eq!(node.operation.name, effect.operation);
                assert!(node.enabled);
                let composition = node.composition.as_ref().unwrap();
                assert_eq!(
                    (composition.stored_blend_space, composition.stored_composite_space, composition.stored_operator),
                    (effect.state.blend_space.unwrap(), effect.state.composite_space.unwrap(), effect.state.composite_mode.unwrap())
                );
                assert!(composition.resolved.is_none());
            }
        }
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
