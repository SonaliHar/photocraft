//! Channel remapping keeps one canonical identity and never copies payloads.
use photocraft_doc::{
    LayerId, Rect, XcfState,
    item_identity::{ChannelId, FilterId, ItemRef, ItemSetId, MaskId, PathId},
    xcf_editing::{EditingSelection, FloatingAttachment, ItemFamily, ItemSet, ItemSetRule},
    xcf_filters::{FilterNode, FilterStack, OperationIdentity},
    xcf_items::{EditableSource, SourceItem},
    xcf_provenance::OpaqueRecord,
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use std::collections::BTreeMap;

fn item(reference: ItemRef) -> ItemState {
    ItemState {
        item: reference,
        name: "Own channel".into(),
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
fn two_channels() -> XcfState {
    XcfState { items: vec![item(ItemRef::Channel(ChannelId(1))), item(ItemRef::Channel(ChannelId(2)))], ..Default::default() }
}

#[test]
fn fresh_target_callback_runs_once_for_one_canonical_channel() {
    let old = ItemRef::Channel(ChannelId(1));
    let mut state =
        XcfState { items: vec![item(old)], selection: EditingSelection { selected: vec![old], active: Some(old), ..Default::default() }, ..Default::default() };
    let mut calls = 0;
    state
        .remap_channels(|_| {
            calls += 1;
            Some(ChannelId(10 + calls))
        })
        .unwrap();
    let new = ItemRef::Channel(ChannelId(11));
    assert_eq!(state.items[0].item, new);
    assert_eq!(state.selection.selected, vec![new]);
    assert_eq!(state.selection.active, Some(new));
    assert_eq!(calls, 1);
}

fn payload_state() -> XcfState {
    let mut state = two_channels();
    let bytes = vec![41; 128 * 1024];
    state.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Channel(ChannelId(2))),
        namespace: "own".into(),
        identity: "own".into(),
        source_version: 22,
        order: 0,
        flags: 3,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    state
}

#[test]
fn identity_remap_preserves_owned_payload_pointer() {
    let mut state = payload_state();
    let original = state.clone();
    let bytes = state.opaque[0].bytes.as_ptr();
    state.remap_channels(Some).unwrap();
    assert!(state == original, "identity changed canonical state");
    assert_eq!(state.opaque[0].bytes.as_ptr(), bytes);
}

#[test]
fn late_none_zero_and_collisions_preserve_exact_state_and_payload() {
    for case in 0..3 {
        let mut state = payload_state();
        let original = state.clone();
        let bytes = state.opaque[0].bytes.as_ptr();
        let result = state.remap_channels(|id| match case {
            0 => (id != ChannelId(2)).then_some(ChannelId(11)),
            1 => Some(ChannelId(0)),
            _ => Some(ChannelId(11)),
        });
        assert!(result.is_err());
        assert!(state == original, "refusal changed state");
        assert_eq!(state.opaque[0].bytes.as_ptr(), bytes);
    }
}

#[test]
fn all_channel_references_share_cached_targets_and_preserve_namespaces() {
    // A visitor census fixture, not an independent imported/evaluated XCF claim.
    let one = ItemRef::Channel(ChannelId(1));
    let two = ItemRef::Channel(ChannelId(2));
    let mask_ref = ItemRef::Mask(MaskId(1));
    let path_ref = ItemRef::Path(PathId(1));
    let mut state = payload_state();
    state.items[1].parent = Some(one);
    let mut mask = item(mask_ref);
    mask.mask = Some(MaskState {
        owner: two,
        bounds: Rect::new(0, 0, 1, 1),
        offset: [0, 0],
        outside_f64_bits: 0.0f64.to_bits(),
        enabled: true,
        shown: false,
        editing: false,
        linked: true,
    });
    state.items.extend([mask, item(path_ref), item(ItemRef::Layer(LayerId(1)))]);
    state.source_items.push(SourceItem {
        owner: two,
        source: EditableSource::Vector { path: PathId(1), source_modified: false, configuration: vec![] },
        transform: None,
        cache: None,
    });
    state.filter_stacks.push(FilterStack {
        owner: one,
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
            mask: Some(mask_ref),
            auxiliary: vec![("aux".into(), two)],
            composition: None,
        }],
    });
    state.selection = EditingSelection { selected: vec![two, one], active: Some(two), selected_paths: vec![path_ref], mask_target: Some(mask_ref) };
    state.item_sets.push(ItemSet { id: ItemSetId(1), name: "Own set".into(), family: ItemFamily::Channel, rule: ItemSetRule::Named, members: vec![two, one] });
    state.floating = Some(FloatingAttachment { layer: LayerId(1), target: two });
    let mut expected = state.clone();
    let new_one = ItemRef::Channel(ChannelId(11));
    let new_two = ItemRef::Channel(ChannelId(12));
    expected.items[0].item = new_one;
    expected.items[1].item = new_two;
    expected.items[1].parent = Some(new_one);
    expected.items[2].mask.as_mut().unwrap().owner = new_two;
    expected.source_items[0].owner = new_two;
    expected.filter_stacks[0].owner = new_one;
    expected.filter_stacks[0].nodes[0].auxiliary[0].1 = new_two;
    expected.selection.selected = vec![new_two, new_one];
    expected.selection.active = Some(new_two);
    expected.item_sets[0].members = vec![new_two, new_one];
    expected.opaque[0].owner = Some(new_two);
    expected.floating.as_mut().unwrap().target = new_two;
    let bytes = state.opaque[0].bytes.as_ptr();
    let mut calls = BTreeMap::new();
    state
        .remap_channels(|id| {
            *calls.entry(id).or_insert(0) += 1;
            Some(ChannelId(id.0 + 10))
        })
        .unwrap();
    assert!(state == expected, "reference mapping differs");
    assert_eq!(calls, BTreeMap::from([(ChannelId(1), 1), (ChannelId(2), 1)]));
    assert_eq!(state.opaque[0].bytes.as_ptr(), bytes);
}

#[test]
fn late_opaque_reference_failure_is_atomic() {
    let mut state = payload_state();
    state.opaque[0].owner = Some(ItemRef::Channel(ChannelId(3)));
    let original = state.clone();
    let bytes = state.opaque[0].bytes.as_ptr();
    assert!(state.remap_channels(|id| (id != ChannelId(3)).then_some(ChannelId(id.0 + 10))).is_err());
    assert!(state == original, "late reference failure changed state");
    assert_eq!(state.opaque[0].bytes.as_ptr(), bytes);
}

#[test]
fn existing_drawable_ceiling_accepts_channel_permutations() {
    let mut state = XcfState { items: (1..=10000).map(|id| item(ItemRef::Channel(ChannelId(id)))).collect(), ..Default::default() };
    state.remap_channels(|id| Some(ChannelId(10001 - id.0))).unwrap();
    assert_eq!(state.items[0].item, ItemRef::Channel(ChannelId(10000)));
    assert_eq!(state.items.last().unwrap().item, ItemRef::Channel(ChannelId(1)));
    state.items.push(item(ItemRef::Channel(ChannelId(30000))));
    let original = state.clone();
    assert!(state.remap_channels(Some).is_err());
    assert!(state == original, "oversized input changed");
}
