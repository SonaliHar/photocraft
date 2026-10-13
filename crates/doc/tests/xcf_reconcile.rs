use photocraft_doc::*;
use photocraft_doc::{item_identity::*, xcf_editing::*, xcf_filters::*, xcf_items::*, xcf_provenance::*, xcf_resources::*, xcf_state::*};
use std::{collections::BTreeMap, sync::Arc};
fn setup() -> (Document, XcfState, LayerId) {
    let mut d = Document::with_background("own", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let owner = d.layers[0].id;
    d.layers[0].mask = Some(LayerMask { enabled: false, ..LayerMask::reveal_all() });
    let mask = ItemState {
        item: ItemRef::Mask(MaskId(1)),
        name: "mask".into(),
        custom_name: true,
        visible: true,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: None,
        composition: None,
        mask: Some(MaskState {
            owner: ItemRef::Layer(owner),
            bounds: Rect::new(0, 0, 2, 2),
            offset: [0, 0],
            outside_f64_bits: 0.0f64.to_bits(),
            enabled: false,
            shown: false,
            editing: false,
            linked: true,
        }),
    };
    let mut s = XcfState::default();
    s.reconcile_layers(&d).unwrap();
    s.items.push(mask);
    s.editable_indexed.push(EditableIndexedLayer {
        owner,
        raster: EditableIndexedRaster::try_new(Rect::new(0, 0, 2, 2), Arc::new(vec![0, 1, 0, 1]), None, vec![[1, 2, 3], [1, 2, 3]]).unwrap(),
    });
    s.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own".into(),
        collection: None,
        internal: false,
        locator: Some("inert/source.png".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    s.source_items.push(SourceItem {
        owner: ItemRef::Layer(owner),
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: None,
    });
    s.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(owner),
        revision: 4,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "own:filter".into(), stored_version: "1".into(), schema_epoch: "own".into() },
            arguments: vec![],
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: Some(ItemRef::Mask(MaskId(1))),
            auxiliary: vec![("input".into(), ItemRef::Layer(owner))],
            composition: None,
        }],
    });
    let bytes = vec![4, 5];
    s.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Layer(owner)),
        namespace: "own".into(),
        identity: "note".into(),
        source_version: 1,
        order: 0,
        flags: 0,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    s.validate(&d).unwrap();
    (d, s, owner)
}
#[test]
fn deletion_prunes_owned_state_and_editing_membership() {
    let (mut d, mut s, owner) = setup();
    s.selection.selected = vec![ItemRef::Layer(owner), ItemRef::Mask(MaskId(1))];
    s.selection.active = Some(ItemRef::Layer(owner));
    s.selection.mask_target = Some(ItemRef::Mask(MaskId(1)));
    s.item_sets.push(ItemSet {
        id: ItemSetId(1),
        name: "own".into(),
        family: ItemFamily::Layer,
        rule: ItemSetRule::Named,
        members: vec![ItemRef::Layer(owner)],
    });
    d.layers.clear();
    s.reconcile_layers(&d).unwrap();
    assert!(s.items.is_empty() && s.editable_indexed.is_empty() && s.source_items.is_empty() && s.filter_stacks.is_empty() && s.opaque.is_empty());
    assert!(s.selection.selected.is_empty() && s.selection.active.is_none() && s.selection.mask_target.is_none());
    assert!(s.item_sets[0].members.is_empty());
    assert_eq!(s.resources.len(), 1);
}
#[test]
fn dependent_floating_and_aux_targets_fail_transactionally() {
    let (mut d, mut s, owner) = setup();
    let other = d.layers[0].duplicate();
    let other_id = other.id;
    d.layers.push(other);
    s.reconcile_layers(&d).unwrap();
    s.floating = Some(FloatingAttachment { layer: other_id, target: ItemRef::Layer(owner) });
    d.layers.remove(0);
    let before = s.clone();
    assert!(s.reconcile_layers(&d).is_err());
    assert_eq!(s, before);
    s.floating = None;
    let mut stack = s.filter_stacks[0].clone();
    stack.owner = ItemRef::Layer(other_id);
    stack.nodes[0].id = FilterId(2);
    stack.nodes[0].mask = None;
    s.filter_stacks.push(stack);
    let before = s.clone();
    assert!(s.reconcile_layers(&d).is_err());
    assert_eq!(s, before);
}
#[test]
fn rename_move_new_group_and_visibility_reconcile_without_losing_locks() {
    let (mut d, mut s, owner) = setup();
    s.items[0].locks.visibility = true;
    let mut child = d.layers.remove(0);
    child.name = "renamed".into();
    child.visible = false;
    let group = Layer::group("new group", vec![child]);
    let group_id = group.id;
    d.layers.push(group);
    s.reconcile_layers(&d).unwrap();
    let item = s.items.iter().find(|i| i.item == ItemRef::Layer(owner)).unwrap();
    assert_eq!(item.name, "renamed");
    assert!(item.custom_name && item.locks.visibility && !item.visible);
    assert_eq!(item.parent, Some(ItemRef::Layer(group_id)));
    assert!(s.items.iter().any(|i| i.item == ItemRef::Layer(group_id)));
}
#[test]
fn clone_requires_unique_typed_ids_and_redirects_owned_dependencies() {
    let (mut d, mut s, owner) = setup();
    let copy = d.layers[0].duplicate();
    let target = copy.id;
    d.layers.push(copy);
    let before = s.clone();
    assert!(s.clone_layer_metadata(owner, target, &d, &BTreeMap::new(), &BTreeMap::new()).is_err());
    assert_eq!(s, before);
    assert!(s.clone_layer_metadata(owner, target, &d, &BTreeMap::from([(MaskId(1), MaskId(1))]), &BTreeMap::from([(FilterId(1), FilterId(2))])).is_err());
    assert_eq!(s, before);
    let mask_ids = BTreeMap::from([(MaskId(1), MaskId(2))]);
    let filter_ids = BTreeMap::from([(FilterId(1), FilterId(2))]);
    s.clone_layer_metadata(owner, target, &d, &mask_ids, &filter_ids).unwrap();
    let stack = s.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(target)).unwrap();
    assert_eq!(stack.nodes[0].id, FilterId(2));
    assert_eq!(stack.nodes[0].mask, Some(ItemRef::Mask(MaskId(2))));
    assert_eq!(stack.nodes[0].auxiliary[0].1, ItemRef::Layer(target));
    let copy = s.editable_indexed.iter_mut().find(|i| i.owner == target).unwrap();
    copy.raster.try_set_pixel(0, 0, 1, 128).unwrap();
    let original = s.editable_indexed.iter().find(|i| i.owner == owner).unwrap();
    assert!(original.raster.alpha().is_none());
    assert_eq!(s.source_items.len(), 2);
    assert_eq!(s.opaque.len(), 2);
}
#[test]
fn hostile_config_is_bounded_before_transaction_clone() {
    let (d, mut s, _) = setup();
    let mut value = FilterValue::Bool(true);
    for _ in 0..34 {
        value = FilterValue::Config(vec![FilterArgument { name: "own".into(), value }]);
    }
    s.filter_stacks[0].nodes[0].arguments = vec![FilterArgument { name: "own".into(), value }];
    assert_eq!(s.reconcile_layers(&d).unwrap_err().code, "config_limit");
}

#[test]
fn recursive_clone_requires_complete_hierarchy_and_preserves_child_state() {
    let (mut d, mut s, child) = setup();
    let group = Layer::group("own group", vec![d.layers.remove(0)]);
    let source = group.id;
    d.layers.push(group);
    s.reconcile_layers(&d).unwrap();
    let copy = d.layers[0].duplicate();
    let target = copy.id;
    let target_child = match &copy.content {
        LayerContent::Group(g) => g.children[0].id,
        _ => unreachable!(),
    };
    d.layers.push(copy);
    let before = s.clone();
    assert!(s.clone_layers_metadata(&BTreeMap::from([(source, target)]), &d, &BTreeMap::new(), &BTreeMap::new()).is_err());
    assert_eq!(s, before);
    let ids = BTreeMap::from([(source, target), (child, target_child)]);
    s.clone_layers_metadata(&ids, &d, &BTreeMap::from([(MaskId(1), MaskId(2))]), &BTreeMap::from([(FilterId(1), FilterId(2))])).unwrap();
    let item = s.items.iter().find(|i| i.item == ItemRef::Layer(target_child)).unwrap();
    assert_eq!(item.parent, Some(ItemRef::Layer(target)));
    assert!(s.editable_indexed.iter().any(|i| i.owner == target_child));
    assert!(s.source_items.iter().any(|i| i.owner == ItemRef::Layer(target_child)));
    let stack = s.filter_stacks.iter().find(|i| i.owner == ItemRef::Layer(target_child)).unwrap();
    assert_eq!(stack.nodes[0].auxiliary[0].1, ItemRef::Layer(target_child));
    assert_eq!(stack.nodes[0].mask, Some(ItemRef::Mask(MaskId(2))));
    s.validate(&d).unwrap();
}

#[test]
fn mask_toggle_and_deletion_preserve_bounds_or_reject_dependencies() {
    let (mut d, mut s, owner) = setup();
    d.layer_mut(owner).unwrap().mask.as_mut().unwrap().enabled = true;
    d.layer_mut(owner).unwrap().mask.as_mut().unwrap().linked = false;
    s.reconcile_layers(&d).unwrap();
    let mask = s.items.iter().find_map(|i| i.mask.as_ref()).unwrap();
    assert!(mask.enabled && !mask.linked);
    assert_eq!(mask.bounds, Rect::new(0, 0, 2, 2));
    d.layer_mut(owner).unwrap().mask = None;
    let before = s.clone();
    assert!(s.reconcile_layers(&d).is_err());
    assert_eq!(s, before);
    s.filter_stacks.clear();
    s.reconcile_layers(&d).unwrap();
    assert!(!s.items.iter().any(|i| i.item == ItemRef::Mask(MaskId(1))));
}

#[test]
fn channel_clone_preserves_owned_provenance_filters_and_remaps_self_input_only() {
    let (mut d, mut s, owner) = setup();
    let source = AlphaChannel::new("Own source", Surface::new(PixelFormat::GRAY8));
    let source_id = source.id;
    d.channels.push(source);
    s.reconcile_layers(&d).unwrap();
    s.items.iter_mut().find(|item| item.item == ItemRef::Channel(source_id)).unwrap().source_tattoo = Some(42);
    let mut stack = s.filter_stacks[0].clone();
    stack.owner = ItemRef::Channel(source_id);
    stack.nodes[0].id = FilterId(2);
    stack.nodes[0].mask = None;
    stack.nodes[0].auxiliary = vec![("self".into(), ItemRef::Channel(source_id)), ("layer".into(), ItemRef::Layer(owner))];
    s.filter_stacks.push(stack);
    s.selection.active = Some(ItemRef::Channel(source_id));
    s.selection.selected = vec![ItemRef::Channel(source_id)];
    let target = AlphaChannel::new("Own copy", Surface::new(PixelFormat::GRAY8));
    let target_id = target.id;
    d.channels.push(target);
    let original = s.clone();
    assert!(s.clone_channel_metadata(source_id, target_id, &d, &BTreeMap::new()).is_err());
    assert_eq!(s, original);
    s.clone_channel_metadata(source_id, target_id, &d, &BTreeMap::from([(FilterId(2), FilterId(3))])).unwrap();
    assert_eq!(s.selection.active, Some(ItemRef::Channel(source_id)));
    let copy = s.filter_stacks.iter().find(|stack| stack.owner == ItemRef::Channel(target_id)).unwrap();
    assert_eq!(copy.nodes[0].id, FilterId(3));
    assert_eq!(copy.nodes[0].auxiliary, vec![("self".into(), ItemRef::Channel(target_id)), ("layer".into(), ItemRef::Layer(owner))]);
    assert_eq!(s.items.iter().find(|item| item.item == ItemRef::Channel(target_id)).unwrap().source_tattoo, Some(42));
    s.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Channel(target_id)).unwrap().nodes[0]
        .auxiliary
        .push(("source".into(), ItemRef::Channel(source_id)));
    d.channels.retain(|channel| channel.id != source_id);
    let before = s.clone();
    assert!(s.reconcile_layers(&d).is_err());
    assert_eq!(s, before);
}
