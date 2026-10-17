//! Explicit layer-set cloning into the single private reconciliation stage.
use super::{Result, channel_clone as copy, controlled, invalid};
use crate::{Document, LayerId, XcfState, item_identity::*, xcf_filters::*, xcf_items::*, xcf_provenance::OpaqueRecord};
use photocraft_raster::Interrupt;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    mem::size_of,
    sync::Arc,
};
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| invalid("cancelled"))
}
fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<()> {
    let count = values.len().checked_add(additional).ok_or_else(|| invalid("clone_layer_limit"))?;
    let bytes = count.checked_mul(size_of::<T>()).filter(|n| *n <= 256 << 20).ok_or_else(|| invalid("clone_layer_limit"))?;
    values.try_reserve_exact(additional).map_err(|_| invalid("clone_layer_limit"))?;
    if values.capacity().checked_mul(size_of::<T>()).is_none_or(|n| n > bytes) {
        return Err(invalid("clone_layer_limit"));
    }
    Ok(())
}
fn source(value: &SourceItem, control: &Interrupt<'_>) -> Result<SourceItem> {
    let source = match &value.source {
        EditableSource::Text { content, markup, font, direction, source_modified, configuration, current_origin } => EditableSource::Text {
            content: copy::string(content, control)?,
            markup: copy::optional(markup.as_deref(), control)?,
            font: *font,
            direction: *direction,
            source_modified: *source_modified,
            configuration: copy::arguments(configuration, 0, control)?,
            current_origin: *current_origin,
        },
        EditableSource::Vector { path, source_modified, configuration } => {
            EditableSource::Vector { path: *path, source_modified: *source_modified, configuration: copy::arguments(configuration, 0, control)? }
        }
        EditableSource::Linked { resource, last_verified_hash, configuration, current } => EditableSource::Linked {
            resource: *resource,
            last_verified_hash: copy::optional(last_verified_hash.as_deref(), control)?,
            configuration: copy::arguments(configuration, 0, control)?,
            current: current.clone(),
        },
    };
    Ok(SourceItem {
        owner: value.owner,
        source,
        transform: value.transform.clone(),
        cache: value
            .cache
            .as_ref()
            .map(|cache| {
                Ok::<_, crate::XcfValidationError>(SourceCache {
                    bytes: Arc::clone(&cache.bytes),
                    blake3: copy::string(&cache.blake3, control)?,
                    width: cache.width,
                    height: cache.height,
                    format: cache.format,
                })
            })
            .transpose()?,
    })
}

pub(super) fn clone_layers(
    state: &mut XcfState,
    layer_ids: &BTreeMap<LayerId, LayerId>,
    doc: &Document,
    mask_ids: &BTreeMap<MaskId, MaskId>,
    filter_ids: &BTreeMap<FilterId, FilterId>,
    control: &Interrupt<'_>,
) -> Result<()> {
    poll(control)?;
    if layer_ids.is_empty() || layer_ids.len() > 10_000 || mask_ids.len() > 100_000 || filter_ids.len() > 100_000 {
        return Err(invalid("clone_layer"));
    }
    let planning = (layer_ids.len() as u64)
        .checked_add(mask_ids.len() as u64)
        .and_then(|n| n.checked_add(filter_ids.len() as u64))
        .and_then(|n| n.checked_mul(256))
        .ok_or_else(|| invalid("clone_layer_limit"))?;
    controlled::admit_extra(state, doc, control, planning, 400_000_000)?;
    let mut all_nodes = 0u64;
    for stack in &state.filter_stacks {
        poll(control)?;
        all_nodes = all_nodes.checked_add(stack.nodes.len() as u64).filter(|n| *n <= 100_000).ok_or_else(|| invalid("clone_layer_limit"))?;
    }
    let planning_work = all_nodes
        .checked_mul(all_nodes)
        .and_then(|n| n.checked_add((state.items.len() as u64).checked_mul((state.items.len() + state.filter_masks.len()) as u64)?))
        .and_then(|n| n.checked_add((layer_ids.len() as u64).checked_mul(state.items.len() as u64)?))
        .and_then(|n| n.checked_add(400_000_000))
        .ok_or_else(|| invalid("clone_layer_limit"))?;
    // Charge worst-case repeated ID lookups before building owned-ID sets or
    // entering their nested validation scans.
    controlled::admit_extra(state, doc, control, planning, planning_work)?;
    let layers = controlled::layer_tree(doc, control)?;
    let mut targets = BTreeSet::new();
    for (&from, &to) in layer_ids {
        poll(control)?;
        let source = layers.get(&from).ok_or_else(|| invalid("clone_layer"))?.0;
        let target = layers.get(&to).ok_or_else(|| invalid("clone_layer"))?.0;
        if from == to || layer_ids.contains_key(&to) || !targets.insert(to) || source.is_group() != target.is_group() {
            return Err(invalid("clone_layer"));
        }
        let mut has_source = false;
        for item in &state.items {
            poll(control)?;
            has_source |= item.item == ItemRef::Layer(from);
            if item.item == ItemRef::Layer(to) {
                return Err(invalid("clone_layer"));
            }
        }
        if !has_source {
            return Err(invalid("clone_layer"));
        }
        if let Some(parent) = layers.get(&from).and_then(|entry| entry.1).and_then(|id| layer_ids.get(&id)).copied()
            && layers.get(&to).and_then(|entry| entry.1) != Some(parent)
        {
            return Err(invalid("clone_hierarchy"));
        }
        if let (crate::LayerContent::Group(source), crate::LayerContent::Group(target)) = (&source.content, &target.content) {
            let mut expected = BTreeSet::new();
            let mut actual = BTreeSet::new();
            for child in &source.children {
                poll(control)?;
                expected.insert(layer_ids.get(&child.id).copied().ok_or_else(|| invalid("clone_descendant"))?);
            }
            for child in &target.children {
                poll(control)?;
                actual.insert(child.id);
            }
            if expected != actual {
                return Err(invalid("clone_descendant"));
            }
        }
    }
    let mut owned_masks = BTreeSet::new();
    let mut owned_filters = BTreeSet::new();
    for item in &state.items {
        poll(control)?;
        if let ItemRef::Mask(id) = item.item
            && item.mask.as_ref().is_some_and(|mask| matches!(mask.owner,ItemRef::Layer(owner) if layer_ids.contains_key(&owner)))
        {
            owned_masks.insert(id);
        }
    }
    for stack in &state.filter_stacks {
        poll(control)?;
        if matches!(stack.owner,ItemRef::Layer(id) if layer_ids.contains_key(&id)) {
            for node in &stack.nodes {
                poll(control)?;
                owned_filters.insert(node.id);
            }
        }
    }
    for mask in &state.filter_masks {
        poll(control)?;
        if owned_filters.contains(&mask.owner) {
            owned_masks.insert(mask.id);
        }
    }
    let owns =
        |item: ItemRef| matches!(item,ItemRef::Layer(id) if layer_ids.contains_key(&id)) || matches!(item,ItemRef::Mask(id) if owned_masks.contains(&id));
    let remap = |item: ItemRef| -> Result<ItemRef> {
        match item {
            ItemRef::Layer(id) => Ok(ItemRef::Layer(layer_ids.get(&id).copied().unwrap_or(id))),
            ItemRef::Mask(id) if owned_masks.contains(&id) => mask_ids.get(&id).copied().map(ItemRef::Mask).ok_or_else(|| invalid("clone_mask_id")),
            _ => Ok(item),
        }
    };
    // Reject missing, exhausted, duplicate or already-held destination typed IDs
    // before copying any selected record. Unused caller-map entries are inert.
    let mut fresh_masks = BTreeSet::new();
    for id in &owned_masks {
        poll(control)?;
        let fresh = mask_ids.get(id).copied().filter(|id| id.0 != 0).ok_or_else(|| invalid("clone_mask_id"))?;
        if !fresh_masks.insert(fresh) {
            return Err(invalid("clone_mask_id"));
        }
        for item in &state.items {
            poll(control)?;
            if item.item == ItemRef::Mask(fresh) {
                return Err(invalid("clone_mask_id"));
            }
        }
        for mask in &state.filter_masks {
            poll(control)?;
            if mask.id == fresh {
                return Err(invalid("clone_mask_id"));
            }
        }
    }
    let mut fresh_filters = BTreeSet::new();
    for stack in &state.filter_stacks {
        poll(control)?;
        if owns(stack.owner) {
            for node in &stack.nodes {
                poll(control)?;
                let fresh = filter_ids.get(&node.id).copied().filter(|id| id.0 != 0).ok_or_else(|| invalid("clone_filter_id"))?;
                if !fresh_filters.insert(fresh) {
                    return Err(invalid("clone_filter_id"));
                }
                for existing in &state.filter_stacks {
                    poll(control)?;
                    for held in &existing.nodes {
                        poll(control)?;
                        if held.id == fresh {
                            return Err(invalid("clone_filter_id"));
                        }
                    }
                }
            }
        }
    }
    let mut count = [0usize; 6];
    let mut logical_opaque = 0u64;
    let mut seen = HashSet::new();
    let mut arc_count = state
        .source_items
        .len()
        .checked_add(state.opaque.len())
        .and_then(|n| n.checked_add(state.editable_indexed.len().checked_mul(2)?))
        .ok_or_else(|| invalid("clone_layer_limit"))?;
    for mask in &state.filter_masks {
        poll(control)?;
        arc_count = arc_count.checked_add(mask.surface.tile_count()).ok_or_else(|| invalid("clone_layer_limit"))?;
    }
    if (arc_count as u64).checked_mul(96).is_none_or(|n| n > 16 << 20) {
        return Err(invalid("clone_layer_limit"));
    }
    seen.try_reserve(arc_count).map_err(|_| invalid("clone_layer_limit"))?;
    let index = (seen.capacity() as u64).checked_mul(48).filter(|n| *n <= 16 << 20).ok_or_else(|| invalid("clone_layer_limit"))?;
    let mut c = copy::Census { control, model: index, payload: 0, work: 0, visits: 0, seen };
    for item in &state.items {
        poll(control)?;
        if owns(item.item) {
            count[0] += 1;
            c.item(item)?;
        }
    }
    for item in &state.source_items {
        poll(control)?;
        if owns(item.owner) {
            count[1] += 1;
            match &item.source {
                EditableSource::Text { content, markup, configuration, .. } => {
                    c.text(content)?;
                    if let Some(markup) = markup {
                        c.text(markup)?;
                    }
                    c.arguments(configuration, 0)?;
                }
                EditableSource::Vector { configuration, .. } => c.arguments(configuration, 0)?,
                EditableSource::Linked { last_verified_hash, configuration, .. } => {
                    if let Some(hash) = last_verified_hash {
                        c.text(hash)?;
                    }
                    c.arguments(configuration, 0)?;
                }
            }
            if let Some(cache) = &item.cache {
                c.text(&cache.blake3)?;
                c.arc(&cache.bytes)?;
            }
        }
    }
    for item in &state.editable_indexed {
        poll(control)?;
        if layer_ids.contains_key(&item.owner) {
            count[2] += 1;
            c.bytes(item.raster.palette_capacity_bytes() as u64)?;
            c.arc(item.raster.indices())?;
            if let Some(alpha) = item.raster.alpha() {
                c.arc(alpha)?;
            }
        }
    }
    for stack in &state.filter_stacks {
        poll(control)?;
        if owns(stack.owner) {
            count[3] += 1;
            c.stack(stack)?;
        }
    }
    for mask in &state.filter_masks {
        poll(control)?;
        if owned_filters.contains(&mask.owner) {
            count[4] += 1;
            c.surface(&mask.surface)?;
            let tiles = mask.surface.tile_count() as u64;
            c.work = c
                .work
                .checked_add(tiles.checked_mul(tiles.saturating_add(1)).and_then(|n| n.checked_mul(8)).ok_or_else(|| invalid("clone_layer_limit"))?)
                .ok_or_else(|| invalid("clone_layer_limit"))?;
        }
    }
    for value in &state.opaque {
        poll(control)?;
        logical_opaque = logical_opaque.checked_add(value.bytes.len() as u64).ok_or_else(|| invalid("opaque_limit"))?;
        if value.owner.is_some_and(owns) {
            count[5] += 1;
            c.opaque(value)?;
            logical_opaque = logical_opaque.checked_add(value.bytes.len() as u64).ok_or_else(|| invalid("opaque_limit"))?;
        }
    }
    for (old, extra, limit) in [
        (state.items.len(), count[0], 10_000),
        (state.source_items.len(), count[1], 10_000),
        (state.editable_indexed.len(), count[2], 10_000),
        (state.filter_stacks.len(), count[3], 10_000),
        (state.filter_masks.len(), count[4], 10_000),
        (state.opaque.len(), count[5], 100_000),
    ] {
        if old.checked_add(extra).is_none_or(|n| n > limit) {
            return Err(invalid("item_limit"));
        }
    }
    if logical_opaque > 256 << 20 {
        return Err(invalid("opaque_limit"));
    }
    let headers = (count[0] as u64) * size_of::<crate::xcf_state::ItemState>() as u64
        + (count[1] as u64) * size_of::<SourceItem>() as u64
        + (count[2] as u64) * size_of::<crate::EditableIndexedLayer>() as u64
        + (count[3] as u64) * size_of::<FilterStack>() as u64
        + (count[4] as u64) * size_of::<crate::FilterMask>() as u64
        + (count[5] as u64) * size_of::<OpaqueRecord>() as u64;
    // Vec growth may transiently retain the original stage allocation during
    // reallocation. Precharge the complete old header capacities, not only the
    // selected tail, before the full stage exists.
    let replacement_headers = (state.source_items.capacity() as u64)
        .checked_mul(size_of::<SourceItem>() as u64)
        .and_then(|n| n.checked_add((state.editable_indexed.capacity() as u64).checked_mul(size_of::<crate::EditableIndexedLayer>() as u64)?))
        .and_then(|n| n.checked_add((state.filter_stacks.capacity() as u64).checked_mul(size_of::<FilterStack>() as u64)?))
        .and_then(|n| n.checked_add((state.filter_masks.capacity() as u64).checked_mul(size_of::<crate::FilterMask>() as u64)?))
        .and_then(|n| n.checked_add((state.opaque.capacity() as u64).checked_mul(size_of::<OpaqueRecord>() as u64)?))
        .ok_or_else(|| invalid("clone_layer_limit"))?;
    c.bytes(
        headers
            .checked_mul(4)
            .and_then(|n| n.checked_add(planning))
            .and_then(|n| n.checked_add(replacement_headers))
            .ok_or_else(|| invalid("clone_layer_limit"))?,
    )?;
    let extra_resident = c.model.checked_mul(2).ok_or_else(|| invalid("clone_layer_limit"))?;
    let extra_work = c.work.checked_mul(3).and_then(|n| n.checked_add(planning_work)).ok_or_else(|| invalid("clone_layer_limit"))?;
    // Baseline canonical validation happens after reconciliation, just as the
    // legacy method: valid deletion/rename reconciliation is not pre-refused.
    controlled::reconcile_with_additions(state, doc, control, extra_resident, extra_work, count[0], |next| {
        reserve(&mut next.source_items, count[1])?;
        reserve(&mut next.editable_indexed, count[2])?;
        reserve(&mut next.filter_stacks, count[3])?;
        reserve(&mut next.filter_masks, count[4])?;
        reserve(&mut next.opaque, count[5])?;
        // Borrow originals from the private stage only until each copy is made;
        // take fixed original lengths so appended records are never duplicated.
        let old = next.items.len();
        for index in 0..old {
            poll(control)?;
            let original = next.items.get(index).ok_or_else(|| invalid("clone_layer"))?;
            if owns(original.item) {
                let mut value = copy::item(original, control)?;
                value.item = remap(value.item)?;
                if let Some(mask) = &mut value.mask {
                    mask.owner = remap(mask.owner)?;
                }
                if let Some(parent) = &mut value.parent {
                    *parent = remap(*parent)?;
                }
                next.items.push(value);
            }
        }
        let old = next.source_items.len();
        for index in 0..old {
            poll(control)?;
            let original = next.source_items.get(index).ok_or_else(|| invalid("clone_layer"))?;
            if owns(original.owner) {
                let mut value = source(original, control)?;
                value.owner = remap(value.owner)?;
                next.source_items.push(value);
            }
        }
        let old = next.editable_indexed.len();
        for index in 0..old {
            poll(control)?;
            let original = next.editable_indexed.get(index).ok_or_else(|| invalid("clone_layer"))?;
            if let Some(owner) = layer_ids.get(&original.owner) {
                let value = crate::EditableIndexedLayer {
                    owner: *owner,
                    raster: crate::EditableIndexedRaster::try_new_with_interrupt(
                        original.raster.bounds(),
                        Arc::clone(original.raster.indices()),
                        original.raster.alpha().map(Arc::clone),
                        copy::copied(original.raster.palette(), control)?,
                        control,
                    )?,
                };
                next.editable_indexed.push(value);
            }
        }
        let old = next.filter_stacks.len();
        for index in 0..old {
            poll(control)?;
            let original = next.filter_stacks.get(index).ok_or_else(|| invalid("clone_layer"))?;
            if owns(original.owner) {
                let mut nodes = copy::reserved(original.nodes.len())?;
                for original in &original.nodes {
                    poll(control)?;
                    let mut value = copy::node(original, control)?;
                    value.id = filter_ids.get(&value.id).copied().ok_or_else(|| invalid("clone_filter_id"))?;
                    if let Some(mask) = &mut value.mask {
                        *mask = remap(*mask)?;
                    }
                    for (_, input) in &mut value.auxiliary {
                        poll(control)?;
                        *input = remap(*input)?;
                    }
                    nodes.push(value);
                }
                let value = FilterStack { owner: remap(original.owner)?, nodes, revision: original.revision };
                next.filter_stacks.push(value);
            }
        }
        let old = next.filter_masks.len();
        for index in 0..old {
            poll(control)?;
            let original = next.filter_masks.get(index).ok_or_else(|| invalid("clone_layer"))?;
            if owned_filters.contains(&original.owner) {
                let value = crate::FilterMask {
                    id: mask_ids.get(&original.id).copied().ok_or_else(|| invalid("clone_mask_id"))?,
                    owner: filter_ids.get(&original.owner).copied().ok_or_else(|| invalid("clone_filter_id"))?,
                    surface: copy::surface(&original.surface, control)?,
                    bounds: original.bounds,
                };
                next.filter_masks.push(value);
            }
        }
        let old = next.opaque.len();
        for index in 0..old {
            poll(control)?;
            let original = next.opaque.get(index).ok_or_else(|| invalid("clone_layer"))?;
            if original.owner.is_some_and(owns) {
                let value = OpaqueRecord {
                    owner: original.owner.map(remap).transpose()?,
                    namespace: copy::string(&original.namespace, control)?,
                    identity: copy::string(&original.identity, control)?,
                    source_version: original.source_version,
                    order: original.order,
                    flags: original.flags,
                    bytes: Arc::clone(&original.bytes),
                    blake3: copy::string(&original.blake3, control)?,
                    rendering_required: original.rendering_required,
                };
                next.opaque.push(value);
            }
        }
        poll(control)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use crate::{xcf_resources::*, xcf_state::*};
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
    fn controlled_recursive_clone_matches_legacy_exact_current_and_original_metadata() {
        let (mut doc, state, child) = setup();
        let mut state = state;
        state.filter_stacks[0].nodes[0]
            .arguments
            .push(FilterArgument { name: "raw-negative-zero".into(), value: FilterValue::Number(ExactNumber::Float64Bits((-0.0_f64).to_bits())) });
        let layer = doc.layers.remove(0);
        let group = Layer::group("Own parent", vec![layer]);
        let from = group.id;
        doc.layers.push(group);
        state.reconcile_layers(&doc).unwrap();
        let duplicate = doc.layers[0].duplicate();
        let to = duplicate.id;
        let LayerContent::Group(target) = &duplicate.content else {
            panic!("group fixture");
        };
        let target_child = target.children[0].id;
        doc.layers.push(duplicate);
        let pairs = BTreeMap::from([(from, to), (child, target_child)]);
        let masks = BTreeMap::from([(MaskId(1), MaskId(2))]);
        let filters = BTreeMap::from([(FilterId(1), FilterId(2))]);
        let mut expected = state.clone();
        expected.clone_layers_metadata(&pairs, &doc, &masks, &filters).unwrap();
        let mut actual = state.clone();
        actual.clone_layers_metadata_with_interrupt(&pairs, &doc, &masks, &filters, &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
        actual.validate(&doc).unwrap();
        assert!(Arc::ptr_eq(actual.editable_indexed[0].raster.indices(), actual.editable_indexed[1].raster.indices()));
        assert!(Arc::ptr_eq(&actual.opaque[0].bytes, &actual.opaque[1].bytes));
        assert_eq!(actual.resources, state.resources);
        assert_eq!(actual.provenance, state.provenance);
        assert_eq!(actual.selection, state.selection);
    }
    #[test]
    fn late_addition_cancel_and_incomplete_typed_maps_preserve_state() {
        let (mut doc, mut state, from) = setup();
        let duplicate = doc.layers[0].duplicate();
        let to = duplicate.id;
        doc.layers.push(duplicate);
        let pairs = BTreeMap::from([(from, to)]);
        let masks = BTreeMap::from([(MaskId(1), MaskId(2))]);
        let filters = BTreeMap::from([(FilterId(1), FilterId(2))]);
        let before = state.clone();
        assert!(state.clone_layers_metadata_with_interrupt(&pairs, &doc, &BTreeMap::new(), &filters, &Interrupt::NONE).is_err());
        assert_eq!(state, before);
        let bytes = Arc::clone(&state.opaque[0].bytes);
        let initial = Arc::strong_count(&bytes);
        let cancel = || Arc::strong_count(&bytes) > initial + 1;
        assert_eq!(state.clone_layers_metadata_with_interrupt(&pairs, &doc, &masks, &filters, &Interrupt::cancel_only(&cancel)).unwrap_err().code, "cancelled");
        assert_eq!(state, before);
    }
}
