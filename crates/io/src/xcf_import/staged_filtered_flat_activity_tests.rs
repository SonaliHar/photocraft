//! UNRUN source activity: authentic GIMP transport plus explicitly external
//! decoded adversarial fixtures. Producer projection/profile variability is HOLD.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_source_activity_fixture.rs"]
mod inputs;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use sha2::{Digest, Sha256};
fn bytes(binding: inputs::Binding) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(binding.path);
    let b = std::fs::read(path).expect("explicit independent activity research input");
    assert_eq!(b.len(), binding.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), binding.sha256);
    b
}
fn file(case: &inputs::Case) -> XcfFile {
    photocraft_xcf::decode(&bytes(case.source), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
#[ignore = "UNRUN: twelve independently authored mixed originals retain source activity/native/raw/profile/mask custody; no numerical oracle"]
fn genuine_sources_preserve_effective_layer_and_explicit_filter_activity() {
    for case in &inputs::CASES {
        let source_bytes = bytes(case.source);
        let parsed = file(case);
        qualify(&parsed, &Interrupt::NONE, &budget()).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
        let source = crate::import_owned_xcf_from_reader_in_quota(
            case.id,
            &mut std::io::Cursor::new(&source_bytes),
            source_bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &control(),
        )
        .unwrap();
        let crate::OwnedXcfImport::FilteredFlat(source) = source else { panic!("mixed-only carrier") };
        let owner = source.document();
        owner.validate_readonly_filtered_flat_point_storage(&control()).unwrap();
        let d = owner.document();
        let x = d.xcf_state.as_ref().unwrap();
        assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert!(d.icc_profile.is_some());
        assert_eq!(d.layers.len(), 4);
        assert_eq!(x.filter_stacks.len(), 3);
        assert_eq!(x.filter_masks.len(), 3);
        for ((layer, original), expected) in d.layers.iter().zip(parsed.layers.iter().rev()).zip(&case.roots) {
            assert_eq!(layer.name, expected.name);
            assert_eq!(layer.visible, expected.visible);
            assert_eq!(layer.opacity.to_bits(), expected.opacity_bits);
            for property in &expected.layer_properties {
                let at = property.offset as usize;
                assert_eq!(&source_bytes[at..at + 4], &property.bytes);
            }
            let item = x.items.iter().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(layer.id)).unwrap();
            assert_eq!(item.visible, layer.visible);
            for property in &original.raw_properties {
                let identity = format!("{}@{}:{}", property.tag, property.offset, property.declared_bytes);
                let raw = x.opaque.iter().find(|r| r.owner == Some(item.item) && r.identity == identity).unwrap();
                assert_eq!(raw.bytes.as_slice(), property.payload.as_slice());
            }
            let LayerContent::BackedRaster(native) = &layer.content else { panic!("actual native provider") };
            let c = control();
            let pin = native.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &c).unwrap();
            let mut captured = Vec::new();
            let mut pixel = vec![0; if case.precision == 750 { 32 } else { 16 }];
            for y in 0..6 {
                for xx in 0..8 {
                    pin.read_pixel_native_into(xx, y, &mut pixel, &c).unwrap();
                    captured.extend_from_slice(&pixel);
                }
            }
            assert_eq!(captured, bytes(expected.native));
            let stack = x.filter_stacks.iter().find(|s| s.owner == item.item);
            if let Some(operation) = expected.operation {
                let node = &stack.unwrap().nodes[0];
                assert_eq!(node.operation.name, operation);
                assert_eq!(Some(node.enabled), expected.enabled);
                assert_eq!(node.opacity_f64_bits, 1f64.to_bits());
                let composition = node.composition.as_ref().unwrap();
                assert!(composition.resolved.is_none());
                let raw = expected.raw3.unwrap();
                assert_eq!([composition.stored_blend_space, composition.stored_composite_space, composition.stored_operator], raw);
                let parsed_effect = &original.effects[0];
                assert_eq!(parsed_effect.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes, bytes(expected.mask_native.unwrap()));
                for property in &parsed_effect.properties {
                    let identity = format!("{}@{}:{}", property.tag, property.offset, property.declared_bytes);
                    let kept = x.opaque.iter().find(|r| r.owner == Some(item.item) && r.identity == identity).unwrap();
                    assert_eq!(kept.bytes.as_slice(), property.payload.as_slice());
                }
            } else {
                assert!(stack.is_none());
            }
        }
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
#[test]
#[ignore = "UNRUN: inactive original sources retain all logical native/argument/mask work, same Budget and late cancellation; physical proof remains mandatory L3"]
fn inactive_roots_cannot_hide_invalid_source_or_refund_work() {
    for case in inputs::CASES.iter().filter(|c| c.id.ends_with("all-inactive")) {
        let f = file(case);
        qualify(&f, &Interrupt::NONE, &budget()).unwrap();
        for root in 0..4 {
            let mut bad = f.clone();
            let plane = bad.layers[root].plane.as_mut().unwrap();
            let word = if bad.sample == Sample::F32 { f32::NAN.to_ne_bytes().to_vec() } else { f64::NAN.to_ne_bytes().to_vec() };
            let at = plane.bytes.len() - word.len();
            plane.bytes[at..].copy_from_slice(&word);
            assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
        }
        for root in 0..3 {
            for change in [
                (|e: &mut photocraft_xcf::Effect| e.state.enabled = None) as fn(&mut photocraft_xcf::Effect),
                |e| e.state.opacity_bits = Some(0.5f32.to_bits()),
                |e| e.state.composite_space = None,
                |e| e.arguments.clear(),
            ] {
                let mut bad = f.clone();
                change(&mut bad.layers[root].effects[0]);
                assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
            }
            let mut bad = f.clone();
            let p = bad.layers[root].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
            let word = if bad.sample == Sample::F32 { 1f32.to_ne_bytes().to_vec() } else { 1f64.to_ne_bytes().to_vec() };
            let at = p.bytes.len() - word.len();
            p.bytes[at..].copy_from_slice(&word);
            assert!(qualify(&bad, &Interrupt::NONE, &budget()).is_err());
        }
        let b = budget();
        qualify(&f, &Interrupt::NONE, &b).unwrap();
        let used = b.used(photocraft_xcf::BudgetKind::Work);
        let mut active = f.clone();
        for d in &mut active.layers {
            d.visible = true;
            d.opacity = 1.;
            for e in &mut d.effects {
                e.state.enabled = Some(true);
            }
        }
        let active_budget = budget();
        qualify(&active, &Interrupt::NONE, &active_budget).unwrap();
        assert_eq!(active_budget.used(photocraft_xcf::BudgetKind::Work), used);
        qualify(&f, &Interrupt::NONE, &b).unwrap();
        assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), used * 2);
        let low = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", used - 1)]).unwrap();
        assert!(qualify(&f, &Interrupt::NONE, &photocraft_xcf::Budget::new(low, None)).is_err());
        let seen = std::cell::Cell::new(0u64);
        let observe = |units| {
            let next = seen.get() + units;
            seen.set(next);
            if next >= used { Err(photocraft_xcf::XcfError::new(ErrorKind::Cancelled, 0, "late inactive source test cancellation")) } else { Ok(()) }
        };
        let cancelled =
            photocraft_xcf::Budget::new_persistent_with_work(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None, Some(&observe))
                .unwrap();
        assert!(matches!(qualify(&f, &Interrupt::NONE, &cancelled), Err(IoError::Cancelled)));
    }
}
#[test]
#[ignore = "UNRUN: mixed-only disabled envelope isolation, unchanged singleton gates and authoritative F32 opacity raw custody"]
fn singleton_sources_stay_strict_and_layer_opacity_is_not_filter_opacity() {
    use crate::xcf_point_replace_source::{hue_chroma_source as h, semi_flatten_source as s, vibrance_source as v};
    for case in &inputs::CASES {
        let f = file(case);
        for root in &f.layers {
            if root.effects.is_empty() {
                continue;
            }
            let e = &root.effects[0];
            if !root.visible || root.opacity.to_bits() != 1f32.to_bits() || e.state.enabled != Some(true) {
                let mut single = f.clone();
                single.layers = vec![root.clone()];
                let old = match e.operation.as_str() {
                    "gimp:semi-flatten" => s::qualify(&single, &Interrupt::NONE, Some(&budget())),
                    "gegl:hue-chroma" => h::qualify(&single, &Interrupt::NONE, Some(&budget())),
                    _ => v::qualify(&single, &Interrupt::NONE, Some(&budget())),
                };
                assert!(!old.unwrap());
            }
            if e.state.enabled == Some(false) {
                let domain = root_domain(root, &Interrupt::NONE, &budget()).unwrap().unwrap();
                let owner = photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1));
                let masks = [Some(photocraft_doc::item_identity::ItemRef::Mask(photocraft_doc::item_identity::MaskId(2)))];
                assert!(crate::xcf_filters::qualified_stack(&root.effects, &masks, owner, 1, 22, domain).is_err());
                let mixed = crate::xcf_filters::qualified_filtered_flat_source_stack(&root.effects, &masks, owner, 1, 22, domain).unwrap();
                assert!(!mixed.nodes[0].enabled);
                assert_eq!(mixed.nodes[0].opacity_f64_bits, 1f64.to_bits());
                assert!(
                    crate::xcf_filters::qualified_filtered_flat_source_stack(&root.effects, &masks, owner, 1, 22, NeighborhoodDomain::BoxGeometry).is_err()
                );
            }
        }
    }
    use crate::xcf_fixture::{FixtureOptions, image_custom};
    for order in [false, true] {
        let mut properties = vec![(6, 95u32.to_be_bytes().to_vec()), (33, 0.375f32.to_be_bytes().to_vec())];
        if order {
            properties.reverse();
        }
        let packet = image_custom(
            22,
            650,
            0,
            1,
            1,
            &[0.5f32.to_be_bytes(); 4].concat(),
            &FixtureOptions { layer_type: Some(1), layer_properties: properties, ..Default::default() },
        );
        let parsed = photocraft_xcf::decode(&packet, &Default::default()).unwrap();
        assert_eq!(parsed.layers[0].opacity.to_bits(), 0.375f32.to_bits());
        assert!(parsed.layers[0].raw_properties.iter().any(|p| p.tag == 6 && p.payload.as_slice() == 95u32.to_be_bytes().as_slice()));
        assert!(parsed.layers[0].raw_properties.iter().any(|p| p.tag == 33 && p.payload.as_slice() == 0.375f32.to_be_bytes().as_slice()));
    }
    let f = file(&inputs::CASES[0]);
    for value in [0f32, -0., 0.375, 1.] {
        let mut changed = f.clone();
        for root in &mut changed.layers {
            root.opacity = value;
        }
        qualify(&changed, &Interrupt::NONE, &budget()).unwrap();
        assert!(changed.layers.iter().all(|r| r.opacity.to_bits() == value.to_bits()));
    }
    for value in [f32::NAN, f32::INFINITY, -0.001, 1.001] {
        let mut changed = f.clone();
        changed.layers[3].opacity = value;
        assert!(qualify(&changed, &Interrupt::NONE, &budget()).is_err());
    }
}
