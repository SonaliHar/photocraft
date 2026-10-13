use photocraft_doc::*;
use photocraft_doc::{item_identity::*, xcf_editing::*, xcf_filters::*, xcf_items::*, xcf_provenance::*, xcf_resources::*, xcf_state::*};
fn document() -> Document {
    let mut doc = Document::with_background("foundation", Size::new(8, 6), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut channel = AlphaChannel::new("Own channel", Surface::new(PixelFormat::GRAY8));
    channel.id = ChannelId(2);
    doc.channels.push(channel);
    let mut path = NamedPath::new("Own path", Path::default());
    path.id = PathId(3);
    doc.paths.push(path);
    doc
}
fn item(item: ItemRef) -> ItemState {
    ItemState {
        item,
        name: "Own item".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: None,
        mask: None,
        composition: None,
    }
}
fn state(d: &Document) -> XcfState {
    XcfState { items: vec![item(ItemRef::Layer(d.layers[0].id)), item(ItemRef::Channel(ChannelId(2))), item(ItemRef::Path(PathId(3)))], ..Default::default() }
}
fn op() -> OperationIdentity {
    OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: "observed-own".into() }
}
fn cancel_filter_validation_after(polls: usize) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let d = document();
    let mut s = state(&d);
    let nodes = (1..=128)
        .map(|id| FilterNode {
            id: FilterId(id),
            operation: op(),
            arguments: vec![FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0)) }],
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        })
        .collect();
    s.filter_stacks.push(FilterStack { owner: ItemRef::Layer(d.layers[0].id), revision: 0, nodes });
    s.validate(&d).unwrap();
    let before = s.clone();
    let observed = AtomicUsize::new(0);
    let cancel = || observed.fetch_add(1, Ordering::Relaxed) >= polls;
    let control = photocraft_raster::Interrupt::cancel_only(&cancel);
    assert_eq!(s.validate_with(&d, &control).unwrap_err().code, "cancelled");
    assert_eq!(s, before);
    assert_eq!(observed.load(Ordering::Relaxed), polls + 1);
}
#[test]
fn scalar_filter_clone_bounds_observes_mid_validation_cancel() {
    cancel_filter_validation_after(32);
}
#[test]
fn scalar_filter_identity_validation_observes_later_cancel() {
    cancel_filter_validation_after(256);
}
fn cancel_nested_argument(value: FilterValue) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let d = document();
    let mut s = state(&d);
    s.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(d.layers[0].id),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: op(),
            arguments: vec![FilterArgument { name: "config".into(), value }],
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    s.validate(&d).unwrap();
    let before = s.clone();
    let observed = AtomicUsize::new(0);
    let cancel = || observed.fetch_add(1, Ordering::Relaxed) >= 32;
    assert_eq!(s.validate_with(&d, &photocraft_raster::Interrupt::cancel_only(&cancel)).unwrap_err().code, "cancelled");
    assert_eq!(s, before);
    assert_eq!(observed.load(Ordering::Relaxed), 33);
}
#[test]
fn nested_config_validation_observes_mid_argument_cancel() {
    let args = (0..128).map(|i| FilterArgument { name: format!("arg{i}"), value: FilterValue::Bool(true) }).collect();
    cancel_nested_argument(FilterValue::Config(args));
}
#[test]
fn nested_graph_validation_observes_mid_node_cancel() {
    let nodes = (1..=128).map(|id| GraphNode { id: GraphNodeId(id), operation: op(), version_origin: GraphVersionOrigin::Stored, arguments: vec![] }).collect();
    cancel_nested_argument(FilterValue::Graph(FilterGraph { input: None, nodes, edges: vec![], output: GraphNodeId(128), original_pipeline: None }));
}
#[test]
fn large_curve_validation_observes_mid_sample_cancel() {
    cancel_nested_argument(FilterValue::Curve { free: true, points_f64_bits: vec![], point_types: vec![], samples_f64_bits: vec![0_f64.to_bits(); 100000] });
}
#[test]
fn dpi_axes_and_custom_unit_preserve_bits() {
    let mut d = document();
    let r = PhysicalResolution {
        x_dpi: 137.5,
        y_dpi: 92.25,
        unit: PhysicalUnit::Custom(CustomUnit {
            name: "Own unit".into(),
            factor: 2.5399999618530273,
            digits: 3,
            symbol: "ou".into(),
            abbreviation: "own".into(),
        }),
    };
    d.set_physical_resolution(r.clone()).unwrap();
    assert_eq!(d.effective_resolution(), r);
    assert_eq!(d.effective_resolution().inches(275, 369).unwrap(), (2.0, 4.0));
    assert_eq!(d.resolution_dpi, 137.5);
}
#[test]
fn dpi_and_unit_invalid_numbers_fail() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(PhysicalResolution::square(bad).validate().is_err());
    }
    let mut r = PhysicalResolution::square(72.0);
    r.unit = PhysicalUnit::Custom(CustomUnit { name: "own".into(), factor: 0.0, digits: 2, symbol: "o".into(), abbreviation: "o".into() });
    assert!(r.validate().is_err());
}
#[test]
fn legacy_scalar_fallback_equal_axes() {
    let mut d = document();
    d.resolution_dpi = 144.0;
    assert_eq!(d.effective_resolution(), PhysicalResolution::square(144.0));
}
#[test]
fn duplicate_and_dangling_ids_fail() {
    let d = document();
    let mut s = state(&d);
    s.items.push(s.items[0].clone());
    assert!(s.validate(&d).is_err());
    let mut s = state(&d);
    s.items[0].item = ItemRef::Layer(LayerId(u64::MAX));
    assert!(s.validate(&d).is_err());
}
#[test]
fn inheritance_and_cycles_are_checked() {
    let mut d = document();
    let child = d.layers.remove(0);
    let id = child.id;
    let group = Layer::group("parent", vec![child]);
    let gid = group.id;
    d.layers.push(group);
    let mut a = item(ItemRef::Layer(id));
    a.parent = Some(ItemRef::Layer(gid));
    let mut b = item(ItemRef::Layer(gid));
    b.locks.visibility = true;
    b.locks.content = true;
    let mut s = XcfState { items: vec![a, b], ..Default::default() };
    s.validate(&d).unwrap();
    assert!(s.effective_locks(ItemRef::Layer(id)).unwrap().visibility);
    s.items[1].parent = Some(ItemRef::Layer(id));
    assert!(s.validate(&d).is_err());
}
#[test]
fn floating_self_and_path_targets_fail() {
    let d = document();
    let mut s = state(&d);
    s.floating = Some(FloatingAttachment { layer: d.layers[0].id, target: ItemRef::Layer(d.layers[0].id) });
    assert!(s.validate(&d).is_err());
    s.floating.as_mut().unwrap().target = ItemRef::Path(PathId(3));
    assert!(s.validate(&d).is_err());
}
#[test]
fn active_selection_and_set_types_fail() {
    let d = document();
    let mut s = state(&d);
    s.selection.active = Some(ItemRef::Layer(d.layers[0].id));
    assert!(s.validate(&d).is_err());
    s.selection.active = None;
    s.item_sets.push(ItemSet {
        id: ItemSetId(1),
        name: "own".into(),
        family: ItemFamily::Channel,
        rule: ItemSetRule::Named,
        members: vec![ItemRef::Path(PathId(3))],
    });
    assert!(s.validate(&d).is_err());
}
#[test]
fn fonts_cannot_embed_authority_or_bytes() {
    let d = document();
    let mut s = state(&d);
    s.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::Font,
        name: "external font".into(),
        collection: None,
        internal: false,
        locator: Some("inert/font".into()),
        content_hash: Some(blake3::hash(&[1]).to_hex().to_string()),
        owned_bytes: Some(vec![1].into()),
        resolution: None,
    });
    assert_eq!(s.validate(&d).unwrap_err().code, "embedded_font");
    s.resources[0].owned_bytes = None;
    s.validate(&d).unwrap();
}
#[test]
fn typed_numbers_keep_bits_and_validate_width() {
    let d = document();
    let mut s = state(&d);
    s.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(d.layers[0].id),
        revision: 1,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: op(),
            arguments: vec![FilterArgument { name: "exact".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0x7ff8_0000_0000_0042)) }],
            enabled: false,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    s.validate(&d).unwrap();
    let encoded = serde_json::to_vec(&s).unwrap();
    assert_eq!(serde_json::from_slice::<XcfState>(&encoded).unwrap(), s);
    s.filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Signed { width: 8, value: 128 });
    assert!(s.validate(&d).is_err());
}
#[test]
fn graph_cycles_fail() {
    let d = document();
    let mut s = state(&d);
    let g = FilterGraph {
        input: None,
        nodes: vec![GraphNode { id: GraphNodeId(1), operation: op(), version_origin: GraphVersionOrigin::Stored, arguments: vec![] }],
        edges: vec![GraphEdge { from: GraphNodeId(1), to: GraphNodeId(1), output_port: "out".into(), input_port: "in".into() }],
        output: GraphNodeId(1),
        original_pipeline: None,
    };
    s.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(d.layers[0].id),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: op(),
            arguments: vec![FilterArgument { name: "graph".into(), value: FilterValue::Graph(g) }],
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    assert_eq!(s.validate(&d).unwrap_err().code, "graph_cycle");
}
#[test]
fn config_depth_is_bounded() {
    let d = document();
    let mut s = state(&d);
    let mut v = FilterValue::Bool(true);
    for _ in 0..34 {
        v = FilterValue::Config(vec![FilterArgument { name: "own".into(), value: v }]);
    }
    s.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(d.layers[0].id),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: op(),
            arguments: vec![FilterArgument { name: "config".into(), value: v }],
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    assert_eq!(s.validate(&d).unwrap_err().code, "config_limit");
}
#[test]
fn opaque_hash_and_required_rendering_state_fail() {
    let d = document();
    let mut s = state(&d);
    s.opaque.push(OpaqueRecord {
        owner: None,
        namespace: "own".into(),
        identity: "note".into(),
        source_version: 26,
        order: 0,
        flags: 3,
        bytes: vec![1, 2, 3].into(),
        blake3: blake3::hash(&[1, 2, 3]).to_hex().to_string(),
        rendering_required: false,
    });
    s.validate(&d).unwrap();
    std::sync::Arc::make_mut(&mut s.opaque[0].bytes)[0] = 4;
    assert!(s.validate(&d).is_err());
    s.opaque[0].rendering_required = true;
    assert_eq!(s.validate(&d).unwrap_err().code, "opaque_rendering_state");
}
#[test]
fn source_cache_hash_and_bounds_are_authoritative() {
    let d = document();
    let mut s = state(&d);
    s.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "own".into(),
        collection: None,
        internal: false,
        locator: Some("inert/own.png".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    let bytes = vec![0; 4];
    s.source_items.push(SourceItem {
        owner: ItemRef::Layer(d.layers[0].id),
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: Some(SourceCache { bytes: bytes.clone().into(), blake3: blake3::hash(&bytes).to_hex().to_string(), width: 1, height: 1, format: PixelFormat::RGBA8 }),
    });
    s.validate(&d).unwrap();
    s.source_items[0].cache.as_mut().unwrap().width = 2;
    assert_eq!(s.validate(&d).unwrap_err().code, "cache_bounds");
}

#[test]
fn editable_indices_alpha_palette_and_cow_are_exact() {
    use std::sync::Arc;
    let mut raster =
        EditableIndexedRaster::try_new(Rect::new(-1, 2, 1, 3), Arc::new(vec![0, 1]), None, vec![[10, 20, 30], [10, 20, 30], [90, 80, 70]]).unwrap();
    let old = raster.clone();
    raster.try_set_pixel(0, 2, 2, 128).unwrap();
    assert_eq!(old.indices().as_slice(), &[0, 1]);
    assert!(old.alpha().is_none());
    assert_eq!(raster.indices().as_slice(), &[0, 2]);
    assert_eq!(raster.alpha().unwrap().as_slice(), &[255, 128]);
    assert_eq!(raster.expanded_pixel(0, 2).unwrap(), [90, 80, 70, 128]);
    let before = raster.clone();
    assert!(raster.try_set_pixel(1, 2, 0, 255).is_err());
    assert!(raster.try_set_pixel(0, 2, 3, 255).is_err());
    assert!(raster.try_set_palette(vec![[1, 2, 3]]).is_err());
    assert_eq!(before, raster);
    raster.try_set_palette(vec![[10, 20, 30], [10, 20, 30], [1, 2, 3]]).unwrap();
    assert_eq!(raster.expanded_pixel(0, 2).unwrap(), [1, 2, 3, 128]);
    assert_eq!(raster.palette()[0], raster.palette()[1]);
}
#[test]
fn indexed_deserialization_and_relationships_cannot_bypass_validation() {
    use std::sync::Arc;
    let raster = EditableIndexedRaster::try_new(Rect::new(0, 0, 1, 1), Arc::new(vec![0]), None, vec![[1, 2, 3]]).unwrap();
    let mut value = serde_json::to_value(&raster).unwrap();
    value["indices"] = serde_json::json!([1]);
    assert!(serde_json::from_value::<EditableIndexedRaster>(value).is_err());
    assert!(EditableIndexedRaster::try_new(Rect::new(i32::MIN, 0, i32::MAX, 1), Arc::new(vec![]), None, vec![[1, 2, 3]]).is_err());
    let d = document();
    let mut s = state(&d);
    s.editable_indexed.push(EditableIndexedLayer { owner: d.layers[0].id, raster });
    assert!(s.validate(&d).is_ok());
    s.editable_indexed.push(s.editable_indexed[0].clone());
    assert!(s.validate(&d).is_err());
    s.editable_indexed.pop();
    s.editable_indexed[0].owner = LayerId(u64::MAX);
    assert!(s.validate(&d).is_err());
}

#[test]
fn independent_mask_bounds_are_preserved_and_hostile_extents_rejected() {
    let d = document();
    let mut s = state(&d);
    let mut mask = item(ItemRef::Mask(MaskId(1)));
    mask.mask = Some(MaskState {
        owner: ItemRef::Layer(d.layers[0].id),
        bounds: Rect::new(-5, 2, 17, 5),
        offset: [-5, 2],
        outside_f64_bits: 0.0f64.to_bits(),
        enabled: false,
        shown: true,
        editing: true,
        linked: false,
    });
    s.items.push(mask);
    s.validate(&d).unwrap();
    let back: XcfState = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    s.items.last_mut().unwrap().mask.as_mut().unwrap().bounds = Rect::new(i32::MIN, 0, i32::MAX, 1);
    assert_eq!(s.validate(&d).unwrap_err().code, "mask_bounds");
}

#[test]
fn indexed_translation_preserves_plane_bytes_and_fails_atomically() {
    use std::sync::Arc;
    let mut raster =
        EditableIndexedRaster::try_new(Rect::new(0, 0, 2, 1), Arc::new(vec![0, 1]), Some(Arc::new(vec![255, 128])), vec![[1, 2, 3], [1, 2, 3]]).unwrap();
    let original = raster.clone();
    raster.translate_bounds(-10, 20).unwrap();
    assert_eq!(raster.bounds(), Rect::new(-10, 20, -8, 21));
    assert!(Arc::ptr_eq(raster.indices(), original.indices()));
    assert!(Arc::ptr_eq(raster.alpha().unwrap(), original.alpha().unwrap()));
    assert_eq!(raster.expanded_pixel(-9, 20).unwrap(), [1, 2, 3, 128]);
    let before = raster.clone();
    assert!(raster.translate_bounds(0, i32::MAX).is_err());
    assert_eq!(raster, before);
}

#[test]
fn channel_remap_covers_references_and_rejects_missing_or_colliding_ids_atomically() {
    let d = document();
    let mut s = state(&d);
    s.selection.active = Some(ItemRef::Channel(ChannelId(2)));
    s.selection.selected = vec![ItemRef::Channel(ChannelId(2))];
    let before = s.clone();
    assert!(s.remap_channels(|_| None).is_err());
    assert_eq!(s, before);
    assert!(s.remap_channels(|_| Some(ChannelId(0))).is_err());
    assert_eq!(s, before);
    s.remap_channels(|id| Some(ChannelId(id.0 + 10))).unwrap();
    assert_eq!(s.selection.active, Some(ItemRef::Channel(ChannelId(12))));
    assert_eq!(s.selection.selected, vec![ItemRef::Channel(ChannelId(12))]);
    assert_eq!(s.items[1].item, ItemRef::Channel(ChannelId(12)));
    let mut collision = before.clone();
    collision.items.push(item(ItemRef::Channel(ChannelId(3))));
    let original = collision.clone();
    assert!(collision.remap_channels(|_| Some(ChannelId(12))).is_err());
    assert_eq!(collision, original);
    let mut d = document();
    d.channels.push(d.channels[0].clone());
    assert!(d.validate_xcf_state().is_err());
}

#[test]
fn declared_raster_extent_is_exact_optional_and_checked() {
    let d = document();
    let mut s = state(&d);
    s.items[0].raster_bounds = Some(Rect::new(-19, 3, 5, 9));
    s.validate(&d).unwrap();
    let value = serde_json::to_value(&s.items[0]).unwrap();
    let back: ItemState = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(back.raster_bounds, Some(Rect::new(-19, 3, 5, 9)));
    let mut old = value;
    old.as_object_mut().unwrap().remove("raster_bounds");
    assert_eq!(serde_json::from_value::<ItemState>(old).unwrap().raster_bounds, None);
    for bounds in [Rect::new(0, 0, 0, 1), Rect::new(2, 1, 1, 2), Rect::new(i32::MIN, 0, i32::MAX, 1)] {
        s.items[0].raster_bounds = Some(bounds);
        assert!(s.validate(&d).is_err());
    }
}
fn linked_current_fixture() -> (Document, XcfState) {
    let d = document();
    let mut s = state(&d);
    let material = vec![17, 29, 43];
    let hash = blake3::hash(&material).to_hex().to_string();
    s.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "own current material".into(),
        collection: None,
        internal: false,
        locator: Some("inert/original-image".into()),
        content_hash: None,
        owned_bytes: Some(material.into()),
        resolution: Some(ResourceResolution { fidelity: ResourceFidelity::Substituted, payload_hash: hash }),
    });
    let bounds = Rect::new(-2, 1, 0, 2);
    s.items[0].raster_bounds = Some(bounds);
    s.source_items.push(SourceItem {
        owner: ItemRef::Layer(d.layers[0].id),
        source: EditableSource::Linked {
            resource: ResourceId(1),
            last_verified_hash: None,
            configuration: vec![],
            current: Some(LinkedCurrentState {
                transform: Some(TransformState {
                    matrix_f64_bits: [1., -0., 0.25, 0., 1., 0.375, 0., 0., 1.].map(f64::to_bits),
                    interpolation: 3,
                    pre_offset: [-2, 1],
                }),
                bounds,
            }),
        },
        transform: Some(TransformState { matrix_f64_bits: [1., -0., 0., 0., 1., 0., 0., 0., 1.].map(f64::to_bits), interpolation: 0, pre_offset: [0, 0] }),
        cache: None,
    });
    (d, s)
}
#[test]
fn linked_current_requires_resolved_material_exact_bounds_and_finite_transform() {
    let (d, valid) = linked_current_fixture();
    valid.validate(&d).unwrap();
    let retained = valid.source_items[0].transform.clone();
    for variant in 0..5 {
        let mut s = valid.clone();
        let EditableSource::Linked { current, .. } = &mut s.source_items[0].source else { panic!("own linked fixture") };
        let current = current.as_mut().unwrap();
        let expected = match variant {
            0 => {
                current.bounds = Rect::new(-1, 1, 1, 2);
                "linked_current_bounds"
            }
            1 => {
                current.bounds = Rect::new(0, 0, 0, 0);
                "linked_current_bounds"
            }
            2 => {
                current.transform.as_mut().unwrap().matrix_f64_bits[0] = f64::NAN.to_bits();
                "linked_current_transform"
            }
            3 => {
                current.transform.as_mut().unwrap().interpolation = 4;
                "linked_current_transform"
            }
            _ => {
                s.resources[0].owned_bytes = None;
                s.resources[0].resolution = None;
                "linked_current_resource"
            }
        };
        assert_eq!(s.validate(&d).unwrap_err().code, expected);
        assert_eq!(s.source_items[0].transform, retained);
    }
    let mut untransformed = valid;
    let EditableSource::Linked { current, .. } = &mut untransformed.source_items[0].source else { panic!("own linked fixture") };
    current.as_mut().unwrap().transform = None;
    untransformed.validate(&d).unwrap();
}
#[test]
fn linked_current_is_additive_and_absent_in_legacy_serialized_shape() {
    let (_, s) = linked_current_fixture();
    let mut wire = serde_json::to_value(&s.source_items[0].source).unwrap();
    assert!(wire["Linked"].get("current").is_some());
    wire["Linked"].as_object_mut().unwrap().remove("current");
    let legacy: EditableSource = serde_json::from_value(wire).unwrap();
    assert!(matches!(&legacy, EditableSource::Linked { current: None, .. }));
    assert!(serde_json::to_value(legacy).unwrap()["Linked"].get("current").is_none());
}

#[test]
fn owned_resource_digest_hits_preserve_hash_resolution_cow_and_cancel_checks() {
    use std::sync::Arc;
    let d=document();let mut s=state(&d);
    let mut bytes=Vec::with_capacity(32768);bytes.extend_from_slice(&[17,29,43]);
    let digest=blake3::hash(&bytes).to_hex().to_string();
    s.resources.push(ResourceReference{id:ResourceId(1),kind:ResourceKind::LinkedImage,name:"Own bytes".into(),collection:None,internal:false,locator:Some("inert/not-a-grant".into()),content_hash:None,owned_bytes:Some(Arc::new(bytes)),resolution:Some(ResourceResolution{fidelity:ResourceFidelity::Substituted,payload_hash:digest.clone()})});
    s.validate(&d).unwrap();s.validate(&d).unwrap();
    let original=s.clone();
    let payload=s.resources[0].owned_bytes.as_ref().unwrap();
    assert_eq!(payload.len(),3);assert!(payload.capacity()>=32768);
    assert!(Arc::ptr_eq(payload,original.resources[0].owned_bytes.as_ref().unwrap()));
    let mut changed=s.clone();Arc::make_mut(changed.resources[0].owned_bytes.as_mut().unwrap())[0]=99;
    assert_eq!(changed.validate(&d).unwrap_err().code,"resource_hash");
    assert_eq!(original.resources[0].owned_bytes.as_ref().unwrap().as_slice(),[17,29,43]);original.validate(&d).unwrap();
    let mut forged=s.clone();forged.resources[0].resolution.as_mut().unwrap().payload_hash="0".repeat(64);
    assert_eq!(forged.validate(&d).unwrap_err().code,"resource_hash");
    forged.resources[0].resolution.as_mut().unwrap().payload_hash="bad".into();assert!(forged.validate(&d).is_err());
    let mut exact=s.clone();exact.resources[0].resolution.as_mut().unwrap().fidelity=ResourceFidelity::Exact;
    exact.resources[0].content_hash=Some("0".repeat(64));assert_eq!(exact.validate(&d).unwrap_err().code,"resource_exact_identity");
    exact.resources[0].content_hash=Some(digest);exact.validate(&d).unwrap();
    let mut font=s.clone();font.resources[0].kind=ResourceKind::Font;assert_eq!(font.validate(&d).unwrap_err().code,"embedded_font");
    let stop=||true;assert_eq!(s.validate_with(&d,&photocraft_raster::Interrupt::cancel_only(&stop)).unwrap_err().code,"cancelled");
    assert_eq!(s,original);
}
