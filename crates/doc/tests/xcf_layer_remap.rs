//! Layer identity remapping is atomic and never copies payload-bearing state.
use photocraft_doc::{
    EditableIndexedLayer, EditableIndexedRaster, LayerId, Rect, XcfState,
    item_identity::{ChannelId, FilterId, ItemRef, ItemSetId, MaskId, PathId},
    xcf_editing::{EditingSelection, FloatingAttachment, ItemFamily, ItemSet, ItemSetRule},
    xcf_filters::{FilterNode, FilterStack, OperationIdentity},
    xcf_items::{EditableSource, SourceItem},
    xcf_provenance::OpaqueRecord,
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use std::{collections::BTreeMap, sync::Arc};

fn item(reference: ItemRef) -> ItemState {
    ItemState {
        item: reference,
        name: "Own identity".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(17),
        source_flags: 29,
        raster_bounds: None,
        mask: None,
        composition: None,
    }
}
fn two_layers() -> XcfState {
    XcfState { items: vec![item(ItemRef::Layer(LayerId(1))), item(ItemRef::Layer(LayerId(2)))], ..Default::default() }
}

#[test]
fn late_missing_mapping_is_atomic() {
    let mut state = two_layers();
    let original = state.clone();
    assert!(state.remap_layers(|id| (id == LayerId(1)).then_some(LayerId(11))).is_err());
    assert!(state == original, "late failure changed state");
}

#[test]
fn zero_target_is_atomic() {
    let mut state = two_layers();
    let original = state.clone();
    assert!(state.remap_layers(|_| Some(LayerId(0))).is_err());
    assert!(state == original, "zero target changed state");
}

#[test]
fn colliding_targets_are_atomic() {
    let mut state = two_layers();
    let original = state.clone();
    assert!(state.remap_layers(|_| Some(LayerId(19))).is_err());
    assert!(state == original, "collision changed state");
}

fn references() -> XcfState {
    let layer = ItemRef::Layer(LayerId(1));
    let other = ItemRef::Layer(LayerId(2));
    let path = ItemRef::Path(PathId(1));
    let mut state = two_layers();
    state.items[1].parent = Some(layer);
    let mut mask = item(ItemRef::Mask(MaskId(1)));
    mask.mask = Some(MaskState {
        owner: other,
        bounds: Rect::new(-1, 2, 3, 5),
        offset: [-1, 2],
        outside_f64_bits: 0.0f64.to_bits(),
        enabled: true,
        shown: false,
        editing: false,
        linked: true,
    });
    state.items.extend([mask, item(path), item(ItemRef::Channel(ChannelId(1)))]);
    state.editable_indexed.push(EditableIndexedLayer {
        owner: LayerId(2),
        raster: EditableIndexedRaster::try_new(Rect::new(0, 0, 1, 1), Arc::new(vec![0]), None, vec![[1, 2, 3]]).unwrap(),
    });
    state.source_items.push(SourceItem {
        owner: other,
        source: EditableSource::Vector { path: PathId(1), source_modified: false, configuration: vec![] },
        transform: None,
        cache: None,
    });
    state.filter_stacks.push(FilterStack {
        owner: layer,
        revision: 7,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "own:inert".into(), stored_version: "0:0".into(), schema_epoch: "own".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: false,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: Some(other),
            auxiliary: vec![("aux".into(), layer)],
            composition: None,
        }],
    });
    state.selection =
        EditingSelection { selected: vec![layer, other], active: Some(layer), selected_paths: vec![path], mask_target: Some(ItemRef::Mask(MaskId(1))) };
    state.item_sets.push(ItemSet {
        id: ItemSetId(1),
        name: "Own set".into(),
        family: ItemFamily::Layer,
        rule: ItemSetRule::Named,
        members: vec![other, layer],
    });
    let bytes = vec![41; 128 * 1024];
    state.opaque.push(OpaqueRecord {
        owner: Some(other),
        namespace: "own".into(),
        identity: "own".into(),
        source_version: 22,
        order: 0,
        flags: 3,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    state.floating = Some(FloatingAttachment { layer: LayerId(2), target: layer });
    state
}

#[test]
fn every_reference_remaps_once_per_old_id_without_payload_copies() {
    let mut state = references();
    let mut expected = state.clone();
    let one = ItemRef::Layer(LayerId(11));
    let two = ItemRef::Layer(LayerId(12));
    expected.items[0].item = one;
    expected.items[1].item = two;
    expected.items[1].parent = Some(one);
    expected.items[2].mask.as_mut().unwrap().owner = two;
    expected.editable_indexed[0].owner = LayerId(12);
    expected.source_items[0].owner = two;
    expected.filter_stacks[0].owner = one;
    expected.filter_stacks[0].nodes[0].mask = Some(two);
    expected.filter_stacks[0].nodes[0].auxiliary[0].1 = one;
    expected.selection.selected = vec![one, two];
    expected.selection.active = Some(one);
    expected.item_sets[0].members = vec![two, one];
    expected.opaque[0].owner = Some(two);
    expected.floating = Some(FloatingAttachment { layer: LayerId(12), target: one });
    let opaque_ptr = state.opaque[0].bytes.as_ptr();
    let indices = state.editable_indexed[0].raster.indices().as_ptr();
    let mut calls = BTreeMap::new();
    state
        .remap_layers(|id| {
            *calls.entry(id).or_insert(0) += 1;
            Some(LayerId(id.0 + 10))
        })
        .unwrap();
    assert!(state == expected, "reference remap differs");
    assert_eq!(calls, BTreeMap::from([(LayerId(1), 1), (LayerId(2), 1)]));
    assert_eq!(state.opaque[0].bytes.as_ptr(), opaque_ptr);
    assert_eq!(state.editable_indexed[0].raster.indices().as_ptr(), indices);
}

#[test]
fn missing_late_reference_is_atomic_and_payload_preserved() {
    let mut state = references();
    state.floating.as_mut().unwrap().target = ItemRef::Layer(LayerId(3));
    let original = state.clone();
    let opaque_ptr = state.opaque[0].bytes.as_ptr();
    assert!(state.remap_layers(|id| (id != LayerId(3)).then_some(LayerId(id.0 + 10))).is_err());
    assert!(state == original, "late reference failure changed state");
    assert_eq!(state.opaque[0].bytes.as_ptr(), opaque_ptr);
}

#[test]
fn layer_id_permutations_preserve_typed_nonlayer_namespaces() {
    let mut state = two_layers();
    state.items.push(item(ItemRef::Path(PathId(1))));
    state.remap_layers(|id| Some(LayerId(if id == LayerId(1) { 2 } else { 1 }))).unwrap();
    assert_eq!(state.items[0].item, ItemRef::Layer(LayerId(2)));
    assert_eq!(state.items[1].item, ItemRef::Layer(LayerId(1)));
    assert_eq!(state.items[2].item, ItemRef::Path(PathId(1)));
}

#[test]
fn existing_drawable_ceiling_is_accepted_and_excess_refused_atomically() {
    let mut state = XcfState { items: (1..=10000).map(|id| item(ItemRef::Layer(LayerId(id)))).collect(), ..Default::default() };
    let mut calls = 0;
    state
        .remap_layers(|id| {
            calls += 1;
            Some(LayerId(id.0 + 10000))
        })
        .unwrap();
    assert_eq!(calls, 10000);
    assert_eq!(state.items.last().unwrap().item, ItemRef::Layer(LayerId(20000)));
    state.items.push(item(ItemRef::Layer(LayerId(30000))));
    let original = state.clone();
    assert!(state.remap_layers(Some).is_err());
    assert!(state == original, "oversized state changed");
}

#[test]
fn duplicate_source_items_and_zero_source_identity_are_atomic() {
    for mut state in [
        XcfState { items: vec![item(ItemRef::Layer(LayerId(1))), item(ItemRef::Layer(LayerId(1)))], ..Default::default() },
        XcfState { items: vec![item(ItemRef::Layer(LayerId(0)))], ..Default::default() },
    ] {
        let original = state.clone();
        assert!(state.remap_layers(|_| Some(LayerId(9))).is_err());
        assert!(state == original, "invalid source identity changed state");
    }
}
