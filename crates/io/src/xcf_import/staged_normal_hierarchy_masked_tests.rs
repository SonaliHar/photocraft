//! UNRUN declared compatibility contracts. Synthetic decoded trees are external
//! harness data, not producer-authored nested-mask files or incoming receipts.
use super::*;
use crate::xcf_import::plane::ParsedPlaneDescriptor;
use photocraft_doc::LayerId;
use photocraft_xcf::{BudgetKind, Effect, PlaneDescriptor, PlaneRole};
use sha2::{Digest, Sha256};
#[derive(Clone)]
struct Stamped {
    native: Plane,
    coverage: bool,
}
impl ExactParsedPlane for Stamped {
    fn allows_normal_hierarchy_source(&self) -> bool {
        true
    }
    fn allows_normal_hierarchy_coverage(&self) -> bool {
        self.coverage
    }
    fn descriptor(&self) -> ParsedPlaneDescriptor {
        self.native.descriptor()
    }
    fn native_byte_len(&self) -> Option<u64> {
        self.native.native_byte_len()
    }
    fn visit_native(&self, i: &Interrupt<'_>, v: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        self.native.visit_native(i, v)
    }
}
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn table(path: &str, hash: &str) -> serde_json::Value {
    let b = std::fs::read(root().join(path)).unwrap();
    assert_eq!(Sha256::digest(&b).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), hash);
    serde_json::from_slice(&b).unwrap()
}
fn bytes(v: &serde_json::Value) -> Vec<u8> {
    let raw = v["path"].as_str().unwrap().replace('\\', "/");
    let p = raw.split_once("/temp/").unwrap().1;
    let b = std::fs::read(root().join("temp").join(p)).unwrap();
    assert_eq!(b.len() as u64, v["bytes"].as_u64().unwrap());
    assert_eq!(Sha256::digest(&b).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), v["sha256"].as_str().unwrap());
    b
}
fn hierarchy_table() -> serde_json::Value {
    table("temp/sol1-normal-hierarchy-gimp-readiness-20261016/literal-index-v1.json", "852e72bec7dcef9714c4a1fc84f3f2708b7edec598fb170e8ebcc0aa226bb2e9")
}
fn mixed_table() -> serde_json::Value {
    table("temp/root-mixed-mask-research-20261016/literal-index-v1.json", "4d64c41a6460a57e96673e04fdfd79d62183f0808d9ffe1e7db0a34ec442d43e")
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn remap_effect<P, Q>(e: Effect<P>, map: &impl Fn(P) -> Q) -> Effect<Q> {
    Effect {
        name: e.name,
        icon: e.icon,
        operation: e.operation,
        version: e.version,
        properties: e.properties,
        arguments: e.arguments,
        state: e.state,
        mask: e.mask.map(|m| Box::new(remap(*m, map))),
    }
}
fn remap<P, Q>(d: Drawable<P>, map: &impl Fn(P) -> Q) -> Drawable<Q> {
    Drawable {
        record_offset: d.record_offset,
        tattoo: d.tattoo,
        active: d.active,
        legacy_linked: d.legacy_linked,
        group_flags: d.group_flags,
        color_tag: d.color_tag,
        mask_editing: d.mask_editing,
        mask_shown: d.mask_shown,
        show_masked: d.show_masked,
        channel_color: d.channel_color,
        floating_attachment: d.floating_attachment,
        item_sets: d.item_sets,
        name: d.name,
        width: d.width,
        height: d.height,
        model: d.model,
        alpha: d.alpha,
        offset: d.offset,
        opacity: d.opacity,
        visible: d.visible,
        lock_alpha: d.lock_alpha,
        lock_content: d.lock_content,
        lock_position: d.lock_position,
        lock_visibility: d.lock_visibility,
        group: d.group,
        path: d.path,
        mode: d.mode,
        composite_mode: d.composite_mode,
        composite_space: d.composite_space,
        blend_space: d.blend_space,
        mask_enabled: d.mask_enabled,
        mask: d.mask.map(|m| Box::new(remap(*m, map))),
        plane: d.plane.map(map),
        selection: d.selection,
        unsupported: d.unsupported,
        diagnostics: d.diagnostics,
        raw_properties: d.raw_properties,
        effects: d.effects.into_iter().map(|e| remap_effect(e, map)).collect(),
        link: d.link,
        transform: d.transform,
        text: d.text,
        text_flags: d.text_flags,
        vector: d.vector,
    }
}
fn remap_file<P, Q>(f: XcfFile<P>, map: &impl Fn(P) -> Q) -> XcfFile<Q> {
    XcfFile {
        paths: f.paths,
        tattoo: f.tattoo,
        item_sets: f.item_sets,
        version: f.version,
        width: f.width,
        height: f.height,
        model: f.model,
        precision: f.precision,
        sample: f.sample,
        encoding: f.encoding,
        compression: f.compression,
        icc: f.icc,
        dpi: f.dpi,
        unit: f.unit,
        palette: f.palette,
        layers: f.layers.into_iter().map(|d| remap(d, map)).collect(),
        channels: f.channels.into_iter().map(|d| remap(d, map)).collect(),
        guides: f.guides,
        diagnostics: f.diagnostics,
        charged_bytes: f.charged_bytes,
        raw_properties: f.raw_properties,
    }
}
fn source_effect(op: &str) -> Effect<Plane> {
    let table = if op == "gegl:exposure" {
        mixed_table()
    } else {
        table("temp/sol1-four-masked-point-literal-bindings-20261016/literal-index-v1.json", "f21b945a5957539a5d645fa87a87a8012bf00de2a32cdb12a3d0b3520716c017")
    };
    let cases = if op == "gegl:exposure" { table["cases"].as_array().unwrap() } else { table["nonempty_cases"].as_array().unwrap() };
    for c in cases.iter().filter(|c| c["source_precision"] == 650) {
        let f = photocraft_xcf::decode(&bytes(&c["source"]), &Default::default()).unwrap();
        for d in f.layers {
            for e in d.effects {
                if e.operation == op {
                    return e;
                }
            }
        }
    }
    panic!("genuine F32 scalar template")
}
fn fixture(op: &str, double: bool) -> XcfFile<Stamped> {
    let t = hierarchy_table();
    let c = t["cases"].as_array().unwrap().iter().find(|c| c["id"] == "f32-nested2").unwrap();
    let mut f = photocraft_xcf::decode(&bytes(&c["source"]), &Default::default()).unwrap();
    let e = source_effect(op);
    for d in &mut f.layers {
        if !d.group {
            d.effects = vec![e.clone(), e.clone()];
            let p = d.effects[1].mask.as_mut().unwrap().plane.as_mut().unwrap();
            p.bytes.fill(0);
        }
    }
    let mut f = remap_file(f, &|mut native: Plane| {
        if double {
            native.bytes = native.bytes.as_chunks::<4>().0.iter().flat_map(|v| (f32::from_ne_bytes(*v) as f64).to_ne_bytes()).collect();
            native.sample = Sample::F64;
        }
        Stamped { native, coverage: true }
    });
    if double {
        f.sample = Sample::F64;
        f.precision = 750;
    }
    f
}
fn mask_mut(f: &mut XcfFile<Stamped>) -> &mut Plane {
    &mut f.layers.iter_mut().find(|d| !d.group).unwrap().effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().native
}
#[test]
#[ignore = "UNRUN: declared nested5 x two-depth decoded compatibility, own ordered domains/paired epochs/raw words; no producer nested-mask or Accounted receipt claim"]
fn declared_nested_source_owns_each_node_domain() {
    use photocraft_doc::item_identity::{ItemRef, MaskId};
    for op in ["gegl:exposure", "gegl:invert-linear", "gegl:invert-gamma", "gegl:brightness-contrast", "gegl:levels"] {
        for double in [false, true] {
            let f = fixture(op, double);
            let b = budget();
            assert_eq!(select(&f, &Interrupt::NONE, &b).unwrap(), SelectedStagedFamily::MaskedNormalHierarchy);
            qualify(&f, &Interrupt::NONE, &b).unwrap();
            assert!(super::super::staged_normal_hierarchy::qualify(&f, &Interrupt::NONE, &b).is_err());
            for d in f.layers.iter().filter(|d| !d.group) {
                let domains = super::super::staged_masked_filtered_flat::domains(d, f.sample, f.width, f.height, &Interrupt::NONE, &b).unwrap();
                assert!(domains.values[0] != domains.values[1]);
                let masks = vec![Some(ItemRef::Mask(MaskId(1))), Some(ItemRef::Mask(MaskId(2)))];
                let stack = crate::xcf_filters::qualified_ordered_masked_normal_hierarchy_source_stack(
                    &d.effects,
                    &masks,
                    &domains.values,
                    ItemRef::Layer(LayerId(1)),
                    1,
                    22,
                )
                .unwrap();
                assert!(stack.nodes[0].operation.schema_epoch.contains("-masked-mixed-wide-v1"));
                assert!(!stack.nodes[1].operation.schema_epoch.contains("-masked-mixed-wide-v1"));
                assert_eq!(stack.nodes[0].operation.name, op);
                assert_eq!(stack.nodes[0].mask, masks[0]);
                assert_eq!(stack.nodes[1].mask, masks[1]);
                let mut wrong = domains.values.clone();
                wrong[0] = crate::xcf_filters::NeighborhoodDomain::PointReplace(if op == "gegl:invert-linear" {
                    crate::xcf_filters::PointReplaceFamily::MaskedMixedInvertGamma
                } else {
                    crate::xcf_filters::PointReplaceFamily::MaskedMixedInvertLinear
                });
                assert!(
                    crate::xcf_filters::qualified_ordered_masked_normal_hierarchy_source_stack(&d.effects, &masks, &wrong, ItemRef::Layer(LayerId(1)), 1, 22)
                        .is_err()
                );
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: raw preimage/tiny positive before Float, signedzero thirdstate/invalid inactive/SourceOpacity/version/native refusals and oldzero priority; external decoded harness"]
fn raw_coverage_and_source_boundaries_are_strict() {
    for double in [false, true] {
        let mut f = fixture("gegl:invert-gamma", double);
        let b = budget();
        for d in &mut f.layers {
            for e in &mut d.effects {
                e.mask.as_mut().unwrap().plane.as_mut().unwrap().native.bytes.fill(0);
            }
        }
        assert_eq!(select(&f, &Interrupt::NONE, &b).unwrap(), SelectedStagedFamily::NormalHierarchy);
        super::super::staged_normal_hierarchy::qualify(&f, &Interrupt::NONE, &b).unwrap();
        assert!(qualify(&f, &Interrupt::NONE, &b).is_err());
        let p = mask_mut(&mut f);
        p.bytes = if double { (-0f64).to_ne_bytes().repeat(48) } else { (-0f32).to_ne_bytes().repeat(48) };
        assert!(select(&f, &Interrupt::NONE, &b).is_err());
    }
    let mut f = fixture("gegl:invert-linear", true);
    let p = mask_mut(&mut f);
    let high = f64::from_bits(0x3ff0000009b25010);
    assert!(high > 1. && high as f32 == 1.);
    p.bytes = high.to_ne_bytes().repeat(48);
    qualify(&f, &Interrupt::NONE, &budget()).unwrap();
    assert_eq!(mask_mut(&mut f).bytes, high.to_ne_bytes().repeat(48));
    mask_mut(&mut f).bytes = f64::MIN_POSITIVE.to_ne_bytes().repeat(48);
    assert_eq!(f64::MIN_POSITIVE as f32, 0.);
    assert_eq!(select(&f, &Interrupt::NONE, &budget()).unwrap(), SelectedStagedFamily::MaskedNormalHierarchy);
    for invalid in [-0.25, f64::NAN, f64::INFINITY, 2.0] {
        mask_mut(&mut f).bytes = invalid.to_ne_bytes().repeat(48);
        assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    }
    let mut f = fixture("gegl:levels", false);
    f.version = 12;
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    let mut f = fixture("gegl:levels", false);
    let d = f.layers.iter_mut().find(|d| !d.group).unwrap();
    d.effects[0].state.enabled = Some(false);
    d.effects[0].arguments.clear();
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    let mut f = fixture("gegl:exposure", false);
    f.layers.iter_mut().find(|d| !d.group).unwrap().effects[0].state.opacity_bits = Some(0.375f32.to_bits());
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    let mut f = fixture("gegl:invert-gamma", false);
    f.layers.iter_mut().find(|d| !d.group).unwrap().plane.as_mut().unwrap().native.bytes.pop();
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    let mut f = fixture("gegl:invert-gamma", false);
    for d in &mut f.layers {
        if let Some(p) = &mut d.plane {
            p.coverage = false;
        }
    }
    assert_eq!(select(&f, &Interrupt::NONE, &budget()).unwrap(), SelectedStagedFamily::NormalHierarchy);
    assert!(super::super::staged_normal_hierarchy::qualify(&f, &Interrupt::NONE, &budget()).is_err());
}
#[test]
#[ignore = "UNRUN: pre-Surface actual descriptor permission/order/count/capmiss/typed Work+cancel and cumulative scans, original Index lease; no forged model custody"]
fn preallocation_and_original_controls_are_effective() {
    let f = fixture("gegl:invert-gamma", false);
    let meta = remap_file(f.clone(), &|p: Stamped| {
        let d = p.native.descriptor();
        PlaneDescriptor {
            width: d.width,
            height: d.height,
            channels: d.channels,
            sample: d.sample,
            source_sample: d.sample,
            model: if d.channels == 1 { Model::Gray } else { Model::Rgb },
            alpha: d.channels == 4,
            role: if d.channels == 1 { PlaneRole::Mask } else { PlaneRole::Raster },
            record_offset: 0,
            offset: (0, 0),
        }
    });
    let b = budget();
    let baseline = b.used(BudgetKind::Resident);
    let mut plan = super::super::staged_normal_hierarchy::descriptors(&meta, &Interrupt::NONE, &b).unwrap().unwrap();
    assert!(plan.hierarchy_upgrade() && plan.hierarchy_coverage());
    assert!(plan.complete().is_err());
    let mut wrong = meta.layers.iter().find_map(|d| d.plane.clone()).unwrap();
    wrong.record_offset = 999;
    assert!(plan.verify(&wrong, &b).is_err());
    for d in &meta.layers {
        for e in &d.effects {
            plan.verify(e.mask.as_ref().unwrap().plane.as_ref().unwrap(), &b).unwrap();
        }
        if let Some(p) = &d.plane {
            plan.verify(p, &b).unwrap();
        }
    }
    plan.complete().unwrap();
    drop(plan);
    assert_eq!(b.used(BudgetKind::Resident), baseline);
    // All geometry agrees; refusal isolates physical mask admission, rather
    // than a stale 8x6 descriptor/header mismatch. No pixel plane is allocated.
    let mut cap = meta.clone();
    cap.width = 4097;
    cap.height = 4097;
    for d in &mut cap.layers {
        d.width = cap.width;
        d.height = cap.height;
        if let Some(p) = &mut d.plane {
            p.width = cap.width;
            p.height = cap.height;
        }
        for e in &mut d.effects {
            let m = e.mask.as_mut().unwrap();
            m.width = cap.width;
            m.height = cap.height;
            let p = m.plane.as_mut().unwrap();
            p.width = cap.width;
            p.height = cap.height;
        }
    }
    assert!(super::super::staged_normal_hierarchy::descriptors(&cap, &Interrupt::NONE, &b).unwrap().is_none(), "one mask exceeds16MiB");
    let mut aggregate = meta.clone();
    let leaf = aggregate.layers.iter_mut().find(|d| !d.group).unwrap();
    let e = leaf.effects[0].clone();
    leaf.effects = vec![e; 257];
    // Each 8x6 mask admits one256x256 F32 tile plus default (<16MiB),
    // but 257 such masks exceed64MiB before the descriptor Vec/Surface phase.
    assert!(super::super::staged_normal_hierarchy::descriptors(&aggregate, &Interrupt::NONE, &b).unwrap().is_none(), "aggregate exceeds64MiB");
    let used = b.used(BudgetKind::Work);
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    let one = b.used(BudgetKind::Work) - used;
    qualify(&f, &Interrupt::NONE, &b).unwrap();
    assert_eq!(b.used(BudgetKind::Work) - used, 2 * one);
    let lowered = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 1)]).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, &photocraft_xcf::Budget::new(lowered, None)).is_err());
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&|| true), &b), Err(IoError::Cancelled)));
    let fee = b.reserve(BudgetKind::Index, 256, "external unique proof receipt contract").unwrap();
    let mut lease = super::super::staged_masked_filtered_flat::CoverageLease(fee);
    assert!(photocraft_doc::xcf_filter_masks::MaskCoverageSetLease::reconcile_capacity(&mut lease, 257).is_err());
    drop(lease);
    assert_eq!(b.used(BudgetKind::Index), 0);
}
#[test]
#[ignore = "UNRUN: genuine separate hierarchy12/maskedmixed8 reader Source custody remains original class and same-q; declared crossproduct is not producer nested-mask evidence"]
fn genuine_baselines_keep_source_custody_and_priority() {
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    for (t, hierarchy) in [(hierarchy_table(), true), (mixed_table(), false)] {
        for c in t["cases"].as_array().unwrap() {
            let original = bytes(&c["source"]);
            let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
            let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
            let owned = crate::import_owned_xcf_from_reader_in_quota(
                "genuine baseline",
                &mut std::io::Cursor::new(&original),
                original.len() as u64,
                &Default::default(),
                &photocraft_xcf::ResourceLimits::for_policy(Default::default())
                    .unwrap()
                    .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
                    .unwrap(),
                q.clone(),
                &ctl,
            )
            .unwrap();
            if hierarchy {
                assert!(matches!(&owned, crate::OwnedXcfImport::NormalHierarchy(_)));
                owned.document().checked_readonly_normal_hierarchy_index(&ctl).unwrap();
            } else {
                assert!(matches!(&owned, crate::OwnedXcfImport::MaskedFilteredFlat(_)));
            }
            assert!(owned.document().document().source_color_context.as_ref().unwrap().original_icc.is_none());
            assert_eq!(bytes(&c["source"]), original);
            drop(owned);
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().index, 0);
        }
    }
}
