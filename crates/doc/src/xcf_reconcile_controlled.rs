//! Cancellable, private reconciliation staging. No source payload is rewritten.
use super::{Result, channel_clone as copy, invalid};
use crate::{
    Document, Layer, LayerId, XcfState, item_identity::*, xcf_editing::*, xcf_filters::*, xcf_items::*, xcf_provenance::*, xcf_resources::*, xcf_state::*,
};
use photocraft_raster::Interrupt;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    mem::size_of,
    sync::Arc,
};

fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| invalid("cancelled"))
}
fn count(total: &mut usize, amount: usize) -> Result<()> {
    *total = total.checked_add(amount).filter(|n| *n <= 100_000).ok_or_else(|| invalid("reconcile_limit"))?;
    Ok(())
}
fn retain<T>(values: &mut Vec<T>, control: &Interrupt<'_>, mut keep: impl FnMut(&T) -> bool) -> Result<()> {
    let mut cancelled = false;
    values.retain(|value| {
        cancelled |= control.cancelled();
        !cancelled && keep(value)
    });
    poll(control)
}

pub(super) fn layer_tree<'a>(doc: &'a Document, control: &Interrupt<'_>) -> Result<BTreeMap<LayerId, (&'a Layer, Option<LayerId>)>> {
    // Allocation-free depth/count/name admission precedes all map nodes.
    fn census(layers: &[Layer], depth: usize, total: &mut usize, bytes: &mut u64, control: &Interrupt<'_>) -> Result<()> {
        if depth > crate::MAX_GROUP_DEPTH {
            return Err(invalid("hierarchy_limit"));
        }
        for layer in layers {
            poll(control)?;
            *total = total.checked_add(1).filter(|n| *n <= 10_000).ok_or_else(|| invalid("layer_identity"))?;
            if layer.name.len() > 1 << 20 {
                return Err(invalid("string_limit"));
            }
            *bytes = bytes.checked_add(layer.name.len() as u64).filter(|n| *n <= 64 << 20).ok_or_else(|| invalid("reconcile_limit"))?;
            for chunk in layer.name.as_bytes().chunks(32 << 10) {
                poll(control)?;
                if chunk.contains(&0) {
                    return Err(invalid("string_limit"));
                }
            }
            if let crate::LayerContent::Group(group) = &layer.content {
                census(&group.children, depth + 1, total, bytes, control)?;
            }
        }
        Ok(())
    }
    census(&doc.layers, 0, &mut 0, &mut 0, control)?;
    let mut result = BTreeMap::new();
    collect_tree(&doc.layers, None, 0, &mut result, control)?;
    Ok(result)
}
fn collect_tree<'a>(
    layers: &'a [Layer],
    parent: Option<LayerId>,
    depth: usize,
    result: &mut BTreeMap<LayerId, (&'a Layer, Option<LayerId>)>,
    control: &Interrupt<'_>,
) -> Result<()> {
    if depth > crate::MAX_GROUP_DEPTH {
        return Err(invalid("hierarchy_limit"));
    }
    for layer in layers {
        poll(control)?;
        if layer.id.0 == 0 || result.insert(layer.id, (layer, parent)).is_some() {
            return Err(invalid("layer_identity"));
        }
        if let crate::LayerContent::Group(group) = &layer.content {
            collect_tree(&group.children, Some(layer.id), depth + 1, result, control)?;
        }
    }
    Ok(())
}

pub(super) fn admit(state: &XcfState, doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    admit_extra(state, doc, control, 0, 0)
}
pub(super) fn admit_extra(state: &XcfState, doc: &Document, control: &Interrupt<'_>, extra_resident: u64, extra_work: u64) -> Result<()> {
    poll(control)?;
    if state.items.len() > 10_000
        || state.resources.len() > 10_000
        || state.source_items.len() > 10_000
        || state.editable_indexed.len() > 10_000
        || state.filter_masks.len() > 10_000
        || state.filter_stacks.len() > 10_000
        || state.item_sets.len() > 10_000
        || state.opaque.len() > 100_000
        || doc.channels.len() > 10_000
        || doc.paths.len() > 10_000
    {
        return Err(invalid("item_limit"));
    }
    let mut visits = 0;
    for length in [
        state.items.len(),
        state.resources.len(),
        state.source_items.len(),
        state.editable_indexed.len(),
        state.filter_masks.len(),
        state.filter_stacks.len(),
        state.item_sets.len(),
        state.opaque.len(),
        state.selection.selected.len(),
        state.selection.selected_paths.len(),
        state.provenance.protected_source_fingerprints.len(),
    ] {
        count(&mut visits, length)?;
    }
    for stack in &state.filter_stacks {
        poll(control)?;
        count(&mut visits, stack.nodes.len())?;
        for node in &stack.nodes {
            poll(control)?;
            count(&mut visits, node.auxiliary.len())?;
        }
    }
    for set in &state.item_sets {
        poll(control)?;
        count(&mut visits, set.members.len())?;
    }
    let mut arc_count = state
        .resources
        .len()
        .checked_add(state.source_items.len())
        .and_then(|n| n.checked_add(state.opaque.len()))
        .and_then(|n| n.checked_add(state.editable_indexed.len().checked_mul(2)?))
        .ok_or_else(|| invalid("reconcile_limit"))?;
    for mask in &state.filter_masks {
        poll(control)?;
        arc_count = arc_count.checked_add(mask.surface.tile_count()).ok_or_else(|| invalid("reconcile_limit"))?;
    }
    if (arc_count as u64).checked_mul(96).is_none_or(|n| n > 16 << 20) {
        return Err(invalid("reconcile_limit"));
    }
    let mut seen = HashSet::new();
    seen.try_reserve(arc_count).map_err(|_| invalid("reconcile_limit"))?;
    let index = (seen.capacity() as u64).checked_mul(48).filter(|n| *n <= 16 << 20).ok_or_else(|| invalid("reconcile_limit"))?;
    let mut c = copy::Census { control, model: index, payload: 0, work: 0, visits, seen };
    c.bytes(size_of::<XcfState>() as u64)?;
    c.array(&state.items)?;
    for value in &state.items {
        c.item(value)?;
    }
    c.array(&state.resources)?;
    for value in &state.resources {
        c.text(&value.name)?;
        for text in [&value.collection, &value.locator, &value.content_hash].into_iter().flatten() {
            c.text(text)?;
        }
        if let Some(resolution) = &value.resolution {
            c.text(&resolution.payload_hash)?;
        }
        if let Some(bytes) = &value.owned_bytes {
            c.arc(bytes)?;
        }
    }
    c.array(&state.source_items)?;
    for value in &state.source_items {
        poll(control)?;
        match &value.source {
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
        if let Some(cache) = &value.cache {
            c.text(&cache.blake3)?;
            c.arc(&cache.bytes)?;
        }
    }
    c.array(&state.filter_stacks)?;
    for value in &state.filter_stacks {
        c.stack(value)?;
    }
    c.array(&state.filter_masks)?;
    for value in &state.filter_masks {
        c.surface(&value.surface)?;
        let tiles = value.surface.tile_count() as u64;
        c.work = c
            .work
            .checked_add(tiles.checked_mul(tiles.saturating_add(1)).and_then(|n| n.checked_mul(8)).ok_or_else(|| invalid("reconcile_limit"))?)
            .ok_or_else(|| invalid("reconcile_limit"))?;
    }
    c.array(&state.editable_indexed)?;
    for value in &state.editable_indexed {
        c.arc(value.raster.indices())?;
        if let Some(alpha) = value.raster.alpha() {
            c.arc(alpha)?;
        }
        c.bytes(value.raster.palette_capacity_bytes() as u64)?;
    }
    c.array(&state.selection.selected)?;
    c.array(&state.selection.selected_paths)?;
    c.array(&state.item_sets)?;
    for value in &state.item_sets {
        c.text(&value.name)?;
        if let ItemSetRule::Text(text) | ItemSetRule::Regex(text) | ItemSetRule::Glob(text) = &value.rule {
            c.text(text)?;
        }
        c.array(&value.members)?;
    }
    c.array(&state.opaque)?;
    for value in &state.opaque {
        c.opaque(value)?;
    }
    for value in [&state.provenance.format, &state.provenance.container, &state.provenance.source_label] {
        c.text(value)?;
    }
    for value in
        [&state.provenance.producer, &state.provenance.release_id, &state.provenance.source_hash, &state.provenance.original_locator].into_iter().flatten()
    {
        c.text(value)?;
    }
    c.array(&state.provenance.protected_source_fingerprints)?;
    for value in &state.provenance.protected_source_fingerprints {
        c.text(value)?;
    }
    // Current model + independent staged model, map/index nodes and worst-case
    // inserted/renamed canonical item strings. Shared payload is charged once.
    let mut names = 0u64;
    for name in doc.channels.iter().map(|value| &value.name).chain(doc.paths.iter().map(|value| &value.name)) {
        poll(control)?;
        names = names.checked_add(name.len() as u64).ok_or_else(|| invalid("reconcile_limit"))?;
        if name.len() > 1 << 20 {
            return Err(invalid("string_limit"));
        }
        for chunk in name.as_bytes().chunks(32 << 10) {
            poll(control)?;
            if chunk.contains(&0) {
                return Err(invalid("string_limit"));
            }
        }
    }
    let overhead =
        (100_000u64).checked_mul(256).and_then(|n| n.checked_add(64 << 20)).and_then(|n| n.checked_add(names)).ok_or_else(|| invalid("reconcile_limit"))?;
    let resident = c
        .model
        .checked_mul(2)
        .and_then(|n| n.checked_add(c.payload))
        .and_then(|n| n.checked_add(overhead))
        .and_then(|n| n.checked_add(extra_resident))
        .ok_or_else(|| invalid("reconcile_limit"))?;
    let work =
        c.work.checked_mul(3).and_then(|n| n.checked_add(400_000_000)).and_then(|n| n.checked_add(extra_work)).ok_or_else(|| invalid("reconcile_limit"))?;
    if resident > photocraft_raster::MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(invalid("reconcile_limit"));
    }
    state.check_clone_bounds_with(control)?;
    poll(control)
}

fn cloned(state: &XcfState, extra_items: usize, control: &Interrupt<'_>) -> Result<XcfState> {
    let p = &state.provenance;
    let mut fingerprints = copy::reserved(p.protected_source_fingerprints.len())?;
    for value in &p.protected_source_fingerprints {
        poll(control)?;
        fingerprints.push(copy::string(value, control)?);
    }
    let provenance = ProjectProvenance {
        format: copy::string(&p.format, control)?,
        format_version: p.format_version,
        container: copy::string(&p.container, control)?,
        producer: copy::optional(p.producer.as_deref(), control)?,
        release_id: copy::optional(p.release_id.as_deref(), control)?,
        source_label: copy::string(&p.source_label, control)?,
        source_hash: copy::optional(p.source_hash.as_deref(), control)?,
        original_locator: copy::optional(p.original_locator.as_deref(), control)?,
        protected_source_fingerprints: fingerprints,
    };
    let item_count = state.items.len().checked_add(extra_items).filter(|n| *n <= 40_001).ok_or_else(|| invalid("item_limit"))?;
    let mut items = copy::reserved(item_count)?;
    for value in &state.items {
        poll(control)?;
        items.push(copy::item(value, control)?);
    }
    let mut resources = copy::reserved(state.resources.len())?;
    for value in &state.resources {
        poll(control)?;
        resources.push(ResourceReference {
            id: value.id,
            kind: value.kind.clone(),
            name: copy::string(&value.name, control)?,
            collection: copy::optional(value.collection.as_deref(), control)?,
            internal: value.internal,
            locator: copy::optional(value.locator.as_deref(), control)?,
            content_hash: copy::optional(value.content_hash.as_deref(), control)?,
            owned_bytes: value.owned_bytes.clone(),
            resolution: value
                .resolution
                .as_ref()
                .map(|r| Ok::<_, crate::XcfValidationError>(ResourceResolution { fidelity: r.fidelity, payload_hash: copy::string(&r.payload_hash, control)? }))
                .transpose()?,
        });
    }
    let mut source_items = copy::reserved(state.source_items.len())?;
    for value in &state.source_items {
        poll(control)?;
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
        source_items.push(SourceItem {
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
        });
    }
    let mut editable_indexed = copy::reserved(state.editable_indexed.len())?;
    for value in &state.editable_indexed {
        poll(control)?;
        editable_indexed.push(crate::EditableIndexedLayer {
            owner: value.owner,
            raster: crate::EditableIndexedRaster::try_new_with_interrupt(
                value.raster.bounds(),
                Arc::clone(value.raster.indices()),
                value.raster.alpha().map(Arc::clone),
                copy::copied(value.raster.palette(), control)?,
                control,
            )?,
        });
    }
    let mut filter_stacks = copy::reserved(state.filter_stacks.len())?;
    for value in &state.filter_stacks {
        let mut nodes = copy::reserved(value.nodes.len())?;
        for value in &value.nodes {
            poll(control)?;
            nodes.push(copy::node(value, control)?);
        }
        filter_stacks.push(FilterStack { owner: value.owner, nodes, revision: value.revision });
    }
    let mut filter_masks = copy::reserved(state.filter_masks.len())?;
    for value in &state.filter_masks {
        poll(control)?;
        filter_masks.push(crate::FilterMask { id: value.id, owner: value.owner, surface: copy::surface(&value.surface, control)?, bounds: value.bounds });
    }
    let selection = EditingSelection {
        selected: copy::copied(&state.selection.selected, control)?,
        active: state.selection.active,
        selected_paths: copy::copied(&state.selection.selected_paths, control)?,
        mask_target: state.selection.mask_target,
    };
    let mut item_sets = copy::reserved(state.item_sets.len())?;
    for value in &state.item_sets {
        poll(control)?;
        let rule = match &value.rule {
            ItemSetRule::Named => ItemSetRule::Named,
            ItemSetRule::Text(text) => ItemSetRule::Text(copy::string(text, control)?),
            ItemSetRule::Regex(text) => ItemSetRule::Regex(copy::string(text, control)?),
            ItemSetRule::Glob(text) => ItemSetRule::Glob(copy::string(text, control)?),
        };
        item_sets.push(ItemSet {
            id: value.id,
            name: copy::string(&value.name, control)?,
            family: value.family,
            rule,
            members: copy::copied(&value.members, control)?,
        });
    }
    let mut opaque = copy::reserved(state.opaque.len())?;
    for value in &state.opaque {
        poll(control)?;
        opaque.push(OpaqueRecord {
            owner: value.owner,
            namespace: copy::string(&value.namespace, control)?,
            identity: copy::string(&value.identity, control)?,
            source_version: value.source_version,
            order: value.order,
            flags: value.flags,
            bytes: Arc::clone(&value.bytes),
            blake3: copy::string(&value.blake3, control)?,
            rendering_required: value.rendering_required,
        });
    }
    Ok(XcfState {
        provenance,
        items,
        resources,
        source_items,
        editable_indexed,
        filter_stacks,
        current_filter_output_precision: state.current_filter_output_precision,
        filter_masks,
        floating: state.floating.clone(),
        selection,
        item_sets,
        opaque,
    })
}

fn find_item<'a>(items: &'a mut [ItemState], id: ItemRef, control: &Interrupt<'_>) -> Result<Option<&'a mut ItemState>> {
    for item in items {
        poll(control)?;
        if item.item == id {
            return Ok(Some(item));
        }
    }
    Ok(None)
}

pub(super) fn reconcile(state: &mut XcfState, doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    reconcile_with_additions(state, doc, control, 0, 0, 0, |_| Ok(()))
}
pub(super) fn reconcile_with_additions(
    state: &mut XcfState,
    doc: &Document,
    control: &Interrupt<'_>,
    extra_resident: u64,
    extra_work: u64,
    additional_items: usize,
    additions: impl FnOnce(&mut XcfState) -> Result<()>,
) -> Result<()> {
    admit_extra(state, doc, control, extra_resident, extra_work)?;
    let layers = layer_tree(doc, control)?;
    let mut channels = BTreeMap::new();
    for channel in &doc.channels {
        poll(control)?;
        channels.insert(channel.id, channel);
    }
    if channels.len() != doc.channels.len() || channels.keys().any(|id| id.0 == 0) {
        return Err(invalid("channel_identity"));
    }
    let mut channel_ids = BTreeSet::new();
    for id in channels.keys() {
        poll(control)?;
        channel_ids.insert(*id);
    }
    if let Some(id) = doc.selection_channel_id
        && (id.0 == 0 || !channel_ids.insert(id))
    {
        return Err(invalid("channel_identity"));
    }
    let mut paths = BTreeMap::new();
    for path in &doc.paths {
        poll(control)?;
        paths.insert(path.id, path);
    }
    if paths.len() != doc.paths.len() || paths.keys().any(|id| id.0 == 0) {
        return Err(invalid("path_identity"));
    }
    let extra = layers
        .len()
        .checked_add(channels.len())
        .and_then(|n| n.checked_add(paths.len()))
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_add(additional_items))
        .ok_or_else(|| invalid("item_limit"))?;
    let mut next = cloned(state, extra, control)?;
    additions(&mut next)?;
    next.check_clone_bounds_with(control)?;
    let mut removed = BTreeSet::new();
    let mut filter_mask_items = BTreeSet::new();
    for mask in &next.filter_masks {
        poll(control)?;
        filter_mask_items.insert(ItemRef::Mask(mask.id));
    }
    for item in &next.items {
        poll(control)?;
        if matches!(item.item, ItemRef::Layer(id) if !layers.contains_key(&id))
            || matches!(item.item, ItemRef::Channel(id) if !channel_ids.contains(&id))
            || matches!(item.item, ItemRef::Path(id) if !paths.contains_key(&id))
            || (!filter_mask_items.contains(&item.item)
                && item
                    .mask
                    .as_ref()
                    .is_some_and(|mask| matches!(mask.owner, ItemRef::Layer(id) if layers.get(&id).is_none_or(|(layer, _)| layer.mask.is_none()))))
        {
            removed.insert(item.item);
        }
    }
    retain(&mut next.items, control, |item| !removed.contains(&item.item))?;
    for item in &mut next.items {
        poll(control)?;
        if !filter_mask_items.contains(&item.item)
            && let Some(mask) = &mut item.mask
            && let ItemRef::Layer(owner) = mask.owner
            && let Some(canonical) = layers.get(&owner).and_then(|(layer, _)| layer.mask.as_ref())
        {
            mask.enabled = canonical.enabled;
            mask.linked = canonical.linked;
        }
    }
    for (&id, (layer, parent)) in &layers {
        poll(control)?;
        if let Some(item) = find_item(&mut next.items, ItemRef::Layer(id), control)? {
            if item.name != layer.name {
                item.name = copy::string(&layer.name, control)?;
                item.custom_name = true;
            }
            item.visible = layer.visible;
            item.parent = parent.map(ItemRef::Layer);
        } else {
            next.items.push(ItemState {
                item: ItemRef::Layer(id),
                name: copy::string(&layer.name, control)?,
                custom_name: true,
                visible: layer.visible,
                locks: ItemLocks {
                    content: layer.locks.pixels || layer.locks.all,
                    position: layer.locks.position || layer.locks.all,
                    alpha: layer.locks.transparency || layer.locks.all,
                    visibility: false,
                },
                parent: parent.map(ItemRef::Layer),
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: None,
                mask: None,
                composition: None,
            });
        }
    }
    for (&id, channel) in &channels {
        poll(control)?;
        if let Some(item) = find_item(&mut next.items, ItemRef::Channel(id), control)? {
            if item.name != channel.name {
                item.name = copy::string(&channel.name, control)?;
                item.custom_name = true;
            }
        } else {
            next.items.push(ItemState {
                item: ItemRef::Channel(id),
                name: copy::string(&channel.name, control)?,
                custom_name: true,
                visible: false,
                locks: Default::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: None,
                mask: None,
                composition: None,
            });
        }
    }
    if let Some(id) = doc.selection_channel_id
        && find_item(&mut next.items, ItemRef::Channel(id), control)?.is_none()
    {
        next.items.push(ItemState {
            item: ItemRef::Channel(id),
            name: copy::string("Selection", control)?,
            custom_name: false,
            visible: false,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: None,
            mask: None,
            composition: None,
        });
    }
    for (&id, path) in &paths {
        poll(control)?;
        if let Some(item) = find_item(&mut next.items, ItemRef::Path(id), control)? {
            if item.name != path.name {
                item.name = copy::string(&path.name, control)?;
                item.custom_name = true;
            }
        } else {
            next.items.push(ItemState {
                item: ItemRef::Path(id),
                name: copy::string(&path.name, control)?,
                custom_name: true,
                visible: false,
                locks: Default::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: None,
                mask: None,
                composition: None,
            });
        }
    }
    retain(&mut next.filter_stacks, control, |stack| !removed.contains(&stack.owner))?;
    let mut retained_filters = BTreeSet::new();
    for stack in &next.filter_stacks {
        poll(control)?;
        for node in &stack.nodes {
            poll(control)?;
            retained_filters.insert(node.id);
        }
    }
    retain(&mut next.filter_masks, control, |mask| {
        let keep = retained_filters.contains(&mask.owner);
        if !keep {
            removed.insert(ItemRef::Mask(mask.id));
        }
        keep
    })?;
    retain(&mut next.items, control, |item| !removed.contains(&item.item))?;
    let owner_survives = |owner: ItemRef| !matches!(owner, ItemRef::Layer(id) if !layers.contains_key(&id)) && !removed.contains(&owner);
    retain(&mut next.source_items, control, |item| owner_survives(item.owner))?;
    retain(&mut next.editable_indexed, control, |item| layers.contains_key(&item.owner))?;
    retain(&mut next.filter_stacks, control, |stack| owner_survives(stack.owner))?;
    retain(&mut next.opaque, control, |record| record.owner.is_none_or(owner_survives))?;
    retain(&mut next.selection.selected, control, |item| owner_survives(*item))?;
    retain(&mut next.selection.selected_paths, control, |item| owner_survives(*item))?;
    if next.selection.active.is_some_and(|item| !owner_survives(item)) {
        next.selection.active = None;
    }
    if next.selection.mask_target.is_some_and(|item| !owner_survives(item)) {
        next.selection.mask_target = None;
    }
    for set in &mut next.item_sets {
        poll(control)?;
        retain(&mut set.members, control, |item| owner_survives(*item))?;
    }
    if next.floating.as_ref().is_some_and(|floating| !layers.contains_key(&floating.layer)) {
        next.floating = None;
    }
    let mut empty = true;
    for stack in &next.filter_stacks {
        poll(control)?;
        empty &= stack.nodes.is_empty();
    }
    if empty {
        next.current_filter_output_precision = crate::FilterOutputPrecision::Canonical;
    }
    poll(control)?;
    next.validate_with(doc, control)?;
    poll(control)?;
    *state = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Document, XcfState, LayerId, LayerId) {
        let mut doc = Document::new("Own reconcile", crate::Size::new(2, 2), crate::ColorMode::Rgb, crate::SampleType::F32);
        doc.layers.clear();
        let kept = Layer::raster("Kept", doc.pixel_format());
        let keep = kept.id;
        let deleted = Layer::raster("Deleted", doc.pixel_format());
        let delete = deleted.id;
        doc.layers = vec![kept, deleted];
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items.iter_mut().find(|item| item.item == ItemRef::Layer(keep)).unwrap().source_tattoo = Some(42);
        state.selection.selected = vec![ItemRef::Layer(keep), ItemRef::Layer(delete)];
        state.selection.active = Some(ItemRef::Layer(delete));
        state.item_sets.push(ItemSet {
            id: ItemSetId(7),
            name: "Own set".into(),
            family: ItemFamily::Layer,
            rule: ItemSetRule::Named,
            members: state.selection.selected.clone(),
        });
        let bytes = Arc::new(vec![0, 255, 2]);
        state.opaque.push(OpaqueRecord {
            owner: None,
            namespace: "own".into(),
            identity: "original".into(),
            source_version: 1,
            order: 2,
            flags: 9,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
        state.filter_stacks.push(FilterStack {
            owner: ItemRef::Layer(delete),
            revision: 3,
            nodes: vec![FilterNode {
                id: FilterId(8),
                operation: OperationIdentity { name: "own:reconcile".into(), stored_version: "0:0".into(), schema_epoch: "own".into() },
                source_display_name: None,
                source_icon: None,
                arguments: vec![FilterArgument { name: "raw".into(), value: FilterValue::Number(ExactNumber::Float64Bits((-0.0_f64).to_bits())) }],
                enabled: false,
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: None,
                source_region: None,
                mask: None,
                auxiliary: vec![],
                composition: None,
            }],
        });
        (doc, state, keep, delete)
    }
    #[test]
    fn controlled_reconcile_matches_legacy_deletion_hierarchy_names_and_original_payload() {
        let (mut doc, state, keep, delete) = fixture();
        doc.layers.retain(|layer| layer.id != delete);
        let mut kept = doc.layers.remove(0);
        kept.name = "Explicit current name".into();
        kept.visible = false;
        doc.layers.push(Layer::group("New parent", vec![kept]));
        let mut legacy = state.clone();
        legacy.reconcile_layers(&doc).unwrap();
        let mut controlled = state.clone();
        controlled.reconcile_layers_with_interrupt(&doc, &Interrupt::NONE).unwrap();
        assert_eq!(controlled, legacy);
        controlled.validate(&doc).unwrap();
        assert!(Arc::ptr_eq(&controlled.opaque[0].bytes, &state.opaque[0].bytes));
        assert_eq!(controlled.items.iter().find(|item| item.item == ItemRef::Layer(keep)).unwrap().source_tattoo, Some(42));
    }
    #[test]
    fn staged_cancellation_and_surviving_dangling_dependency_never_publish() {
        let (mut doc, mut state, keep, delete) = fixture();
        let before = state.clone();
        let bytes = Arc::clone(&state.opaque[0].bytes);
        let initial = Arc::strong_count(&bytes);
        let cancelled = || Arc::strong_count(&bytes) > initial;
        assert_eq!(state.reconcile_layers_with_interrupt(&doc, &Interrupt::cancel_only(&cancelled)).unwrap_err().code, "cancelled");
        assert_eq!(state, before);
        state.filter_stacks[0].owner = ItemRef::Layer(keep);
        state.filter_stacks[0].nodes[0].auxiliary.push(("missing".into(), ItemRef::Layer(delete)));
        doc.layers.retain(|layer| layer.id != delete);
        let before = state.clone();
        let mut legacy = state.clone();
        assert!(legacy.reconcile_layers(&doc).is_err());
        assert!(state.reconcile_layers_with_interrupt(&doc, &Interrupt::NONE).is_err());
        assert_eq!(state, before);
    }
}
