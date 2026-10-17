//! UNRUN: genuine reader custody group is separate from external metadata/tree
//! helper fixtures; no projection/default/padding or numerical acceptance claim.
use super::*;
use photocraft_xcf::{BudgetKind, Compression, Effect, EffectState, PlaneRole, Unit};
use sha2::{Digest, Sha256};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn binding(v: &serde_json::Value) -> Vec<u8> {
    let p = v["path"].as_str().unwrap().replace('\\', "/");
    let p = p.split_once("/temp/").unwrap().1;
    let bytes = std::fs::read(root().join("temp").join(p)).unwrap();
    assert_eq!(bytes.len() as u64, v["bytes"].as_u64().unwrap());
    assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), v["sha256"].as_str().unwrap());
    bytes
}
fn genuine_sources() {
    let index = std::fs::read(root().join("temp/sol1-normal-hierarchy-gimp-readiness-20261016/literal-index-v1.json")).unwrap();
    assert_eq!(
        Sha256::digest(&index).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "852e72bec7dcef9714c4a1fc84f3f2708b7edec598fb170e8ebcc0aa226bb2e9"
    );
    let index: serde_json::Value = serde_json::from_slice(&index).unwrap();
    assert_eq!(index["cases"].as_array().unwrap().len(), 12);
    for case in index["cases"].as_array().unwrap() {
        // Encoded/capture/seed/packet buffers are external test input, never incoming receipts.
        let bytes = binding(&case["source"]);
        let _capture = binding(&case["capture"]);
        let _packet = binding(&case["packet"]);
        let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
            resident: 256 << 20,
            index: 64 << 20,
            scratch: 0,
        });
        let c = photocraft_raster::backing::ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let source = crate::import_owned_xcf_from_reader_in_quota(
            case["id"].as_str().unwrap(),
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits(),
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::NormalHierarchy(source) = source else { panic!("new actual hierarchy carrier") };
        let owner = source.document();
        assert!(owner.is_readonly_normal_hierarchy_storage());
        assert!(!owner.is_readonly_filtered_flat_point_storage());
        let d = owner.document();
        let state = d.xcf_state.as_ref().unwrap();
        assert_eq!(state.provenance.format_version, case["source_version"].as_u64().unwrap() as u32);
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        let rows = owner.checked_readonly_normal_hierarchy_index(&c).unwrap();
        let raw = case["expected_top_first"].as_array().unwrap();
        assert_eq!(rows.len(), raw.len());
        for r in raw {
            let name = r["name"].as_str().unwrap();
            let row = rows.rows().iter().find(|v| v.layer.name == name).unwrap();
            let canonical = &state.items[row.canonical_item_index];
            let path = r["path_top_first"].as_array().unwrap();
            if path.len() == 1 {
                assert!(row.parent.is_none())
            } else {
                let prefix = &path[..path.len() - 1];
                let parent = raw.iter().find(|p| p["path_top_first"].as_array().unwrap().as_slice() == prefix).unwrap();
                assert_eq!(rows.row(row.parent.unwrap()).unwrap().layer.name, parent["name"].as_str().unwrap());
            }
            if r["kind"] == "group" {
                assert!(matches!(&row.layer.content, LayerContent::Group(_)));
                assert!(canonical.raster_bounds.is_none());
                assert!(row.layer.surface().is_none());
            } else {
                let LayerContent::BackedRaster(native) = &row.layer.content else { panic!("real retained native leaf") };
                let expected = binding(&r["seed"]);
                let mut actual = Vec::new();
                let mut pixel = vec![0u8; if d.depth == photocraft_color::SampleType::F64 { 32 } else { 16 }];
                for y in 0..d.size.height {
                    for x in 0..d.size.width {
                        native.checked_view().read_pixel_native_into(x as i32, y as i32, &mut pixel, &c).unwrap();
                        actual.extend_from_slice(&pixel);
                    }
                }
                assert_eq!(actual, expected);
            }
            let expected_children = raw
                .iter()
                .filter(|p| {
                    let child = p["path_top_first"].as_array().unwrap();
                    child.len() == path.len() + 1 && child[..path.len()] == path[..]
                })
                .map(|p| p["name"].as_str().unwrap())
                .rev()
                .collect::<Vec<_>>();
            if let LayerContent::Group(group) = &row.layer.content {
                assert_eq!(group.children.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), expected_children);
            }
        }
        let roots = raw.iter().filter(|r| r["path_top_first"].as_array().unwrap().len() == 1).map(|r| r["name"].as_str().unwrap()).rev().collect::<Vec<_>>();
        assert_eq!(d.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), roots);
        drop(rows);
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
        assert_eq!(binding(&case["source"]), bytes);
    }
}
fn old_genuine_priority() {
    let (root, rows) = staged_accounted_group::tests::records();
    for row in rows {
        let bytes = staged_accounted_group::tests::read(&root, &row);
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let b = budget();
        assert!(!intrinsic_old_partition(&original, &Interrupt::NONE, &b).unwrap());
        assert_eq!(SourceSelectedPolicy::family_controlled(&original, &Interrupt::NONE, Some(&b)).unwrap(), SelectedStagedFamily::IsolatedNormalGroup);
        let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
            resident: 256 << 20,
            index: 64 << 20,
            scratch: 0,
        });
        let c = photocraft_raster::backing::ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let imported = crate::import_owned_xcf_from_reader_in_quota(
            "old actualgroup",
            &mut std::io::Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits(),
            q.clone(),
            &c,
        )
        .unwrap();
        assert!(matches!(imported, crate::OwnedXcfImport::IsolatedNormalGroup(_)));
        drop(imported);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
        let mut broken = original;
        broken.layers.iter_mut().find_map(|d| d.plane.as_mut()).unwrap().bytes.pop();
        assert!(!intrinsic_old_partition(&broken, &Interrupt::NONE, &b).unwrap());
        assert_eq!(
            SourceSelectedPolicy::family_controlled(&broken, &Interrupt::NONE, Some(&b)).unwrap(),
            SelectedStagedFamily::IsolatedNormalGroup,
            "old identity remains final even malformed native"
        );
    }
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(limits(), None)
}
fn descriptor(role: PlaneRole, id: u64) -> PlaneDescriptor {
    PlaneDescriptor {
        width: 8,
        height: 6,
        channels: if role == PlaneRole::Mask { 1 } else { 4 },
        sample: Sample::F32,
        source_sample: Sample::F32,
        model: if role == PlaneRole::Mask { Model::Gray } else { Model::Rgb },
        alpha: role == PlaneRole::Raster,
        role,
        record_offset: id,
        offset: (0, 0),
    }
}
fn effect(id: u64) -> Effect<PlaneDescriptor> {
    Effect {
        name: "external effect".into(),
        icon: String::new(),
        operation: "gegl:invert-gamma".into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: Some(Box::new(Drawable { width: 8, height: 6, plane: Some(descriptor(PlaneRole::Mask, id)), ..Default::default() })),
        state: EffectState::default(),
    }
}
fn tree() -> XcfFile<PlaneDescriptor> {
    let group = Drawable { group: true, path: Vec::new(), ..Default::default() };
    let leaf = |id, path| Drawable {
        width: 8,
        height: 6,
        model: Model::Rgb,
        alpha: true,
        mode: 28,
        blend_space: 0,
        composite_space: -1,
        composite_mode: -1,
        path,
        plane: Some(descriptor(PlaneRole::Raster, id)),
        ..Default::default()
    };
    XcfFile {
        version: 22,
        width: 8,
        height: 6,
        model: Model::Rgb,
        precision: 650,
        sample: Sample::F32,
        encoding: Encoding::NonlinearProfile,
        compression: Compression::None,
        icc: Vec::new(),
        dpi: (72., 72.),
        unit: Unit::Inch,
        palette: Vec::new(),
        layers: vec![group, leaf(1, vec![0, 0]), leaf(2, Vec::new()), leaf(3, Vec::new())],
        channels: Vec::new(),
        guides: Vec::new(),
        paths: Vec::new(),
        tattoo: None,
        item_sets: Vec::new(),
        diagnostics: Vec::new(),
        charged_bytes: 0,
        raw_properties: Vec::new(),
    }
}
#[test]
#[ignore = "UNRUN: twelve genuine hierarchy Source carrier/native/parent/order custody plus exact intrinsic old-priority controls; no numeric/padding theorem"]
fn old_priority_partition_requires_intrinsic_topology() {
    genuine_sources();
    old_genuine_priority();
    let mut f = tree();
    let b = budget();
    assert!(staged_accounted_group::hint(&f));
    assert!(!intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap());
    f.layers[1].effects.push(effect(80));
    f.layers[1].effects[0].state.enabled = Some(false);
    assert!(intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap());
    f.layers[1].path = vec![9, 0];
    assert!(!intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap());
    f.layers[1].path = vec![0, 0];
    f.layers[1].effects.clear();
    f.layers[1].group = true;
    f.layers[1].plane = None;
    f.layers[2].path = vec![0, 0, 0];
    assert!(intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap());
    f.layers[2].path = vec![7, 0, 0];
    assert!(!intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap());
    f.layers[1].path = vec![0, 9];
    assert!(!intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap());
}
#[test]
#[ignore = "UNRUN: external descriptor plan precedes builders, exact order/complete count/caps and refund; no Surface/native/profile receipt or producer padding claim"]
fn descriptor_preallocation_and_permanent_capmiss() {
    let mut f = tree();
    f.layers[1].effects.push(effect(80));
    let b = budget();
    let baseline = b.used(BudgetKind::Resident);
    let mut plan = descriptors(&f, &Interrupt::NONE, &b).unwrap().unwrap();
    assert!(plan.hierarchy_upgrade());
    assert!(!plan.mixed_upgrade());
    assert!(plan.complete().is_err());
    let wrong = descriptor(PlaneRole::Raster, 999);
    assert!(plan.verify(&wrong, &b).is_err());
    for d in &f.layers {
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
    f.layers[1].effects = (0..257).map(|n| effect(n + 80)).collect();
    assert!(descriptors(&f, &Interrupt::NONE, &b).unwrap().is_none());
    f.layers[1].effects.clear();
    assert!(descriptors(&f, &Interrupt::NONE, &b).unwrap().is_none(), "old fixed plain cohort remains old");
}
#[test]
#[ignore = "UNRUN: external model tree iterative visit covers arbitrary sibling/depth order and empty groups without fake native/group projection; not Accounted custody"]
fn iterative_actual_tree_is_bounded_and_ordered() {
    let mut d = Document::new("external", Size::new(8, 6), ColorMode::Rgb, photocraft_color::SampleType::F32);
    d.layers = vec![Layer::group("a", vec![Layer::group("b", vec![]), Layer::group("c", vec![Layer::group("d", vec![])])]), Layer::group("e", vec![])];
    let names = layers(&d).map(|l| l.unwrap().name.clone()).collect::<Vec<_>>();
    assert_eq!(names, ["a", "b", "c", "d", "e"]);
    let chain = |n| {
        let mut node = Layer::group("bottom", vec![]);
        for _ in 1..n {
            node = Layer::group("ancestor", vec![node]);
        }
        node
    };
    d.layers = vec![chain(32)];
    assert_eq!(layers(&d).collect::<Result<Vec<_>, _>>().unwrap().len(), 32);
    d.layers = vec![chain(33)];
    assert!(layers(&d).collect::<Result<Vec<_>, _>>().is_err());
}
#[test]
#[ignore = "UNRUN: fixed priority/planner work debits original cumulative Budget and typed cancellation; metadata permission never grants Source by itself; real Source release binds separate full proof"]
fn original_budget_and_source_permissions_remain_separate() {
    let mut f = tree();
    f.layers[1].effects.push(effect(80));
    let b = budget();
    intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap();
    let used = b.used(BudgetKind::Work);
    assert!(used >= 8192);
    intrinsic_old_partition(&f, &Interrupt::NONE, &b).unwrap();
    assert!(b.used(BudgetKind::Work) >= used + 8192);
    let low = photocraft_xcf::Budget::new(limits().lowered(&[("sampleWorkUnits", 8191)]).unwrap(), None);
    assert!(matches!(intrinsic_old_partition(&f,&Interrupt::NONE,&low),Err(IoError::Xcf(e)) if e.kind==ErrorKind::Limit));
    assert!(matches!(intrinsic_old_partition(&f, &Interrupt::cancel_only(&|| true), &b), Err(IoError::Cancelled)));
    let noresident = photocraft_xcf::Budget::new(limits().lowered(&[("residentBytes", 0)]).unwrap(), None);
    assert!(descriptors(&f, &Interrupt::NONE, &noresident).is_err());
    // No current/Source factory is bypassed by metadata permission or class names.
    f.model = Model::Gray;
    assert!(descriptors(&f, &Interrupt::NONE, &b).unwrap().is_none());
}
