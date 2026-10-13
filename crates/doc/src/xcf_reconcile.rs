//! Checked reconciliation after canonical layer-tree edits.
use crate::{Document, Layer, LayerId, XcfState, XcfValidationError, item_identity::*, xcf_state::*};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, XcfValidationError>;
#[path = "xcf_channel_clone.rs"]
mod channel_clone;
#[path = "xcf_reconcile_controlled.rs"]
mod controlled;
#[path = "xcf_path_clone_controlled.rs"]
mod path_clone;
#[path = "xcf_layer_clone_controlled.rs"]
mod layer_clone;
fn invalid(code: &'static str) -> XcfValidationError {
    XcfValidationError::new(code, "cannot reconcile imported layer state")
}

fn layer_tree(doc: &Document) -> Result<BTreeMap<LayerId, (&Layer, Option<LayerId>)>> {
    let mut result = BTreeMap::new();
    let mut pending = vec![(doc.layers.as_slice(), None, 0usize)];
    while let Some((siblings, parent, depth)) = pending.pop() {
        if depth > crate::MAX_GROUP_DEPTH {
            return Err(invalid("hierarchy_limit"));
        }
        for layer in siblings {
            if layer.id.0 == 0 || result.insert(layer.id, (layer, parent)).is_some() || result.len() > 10000 {
                return Err(invalid("layer_identity"));
            }
            if let crate::LayerContent::Group(group) = &layer.content {
                pending.push((group.children.as_slice(), Some(layer.id), depth + 1));
            }
        }
    }
    Ok(result)
}

impl XcfState {
    /// Transactionally synchronize layer names, visibility and hierarchy with `doc`.
    /// Mask enabled/link flags follow canonical masks while declared bounds stay exact.
    /// Deleted canonical masks and layer-owned items, caches, indexed planes, filters and opaque metadata
    /// are removed. Editing selections and named-set memberships drop deleted items.
    /// A deleted floating layer removes its attachment; a surviving attachment or filter
    /// referring to a deleted target is an error. Resource/path/channel semantics are retained.
    /// Newly created canonical layers receive ordinary item metadata, never imported semantics.
    /// Existing XCF locks/composition stay explicit and must be edited through their own APIs.
    pub fn reconcile_layers(&mut self, doc: &Document) -> Result<()> {
        let layers = layer_tree(doc)?;
        let channels: BTreeMap<_, _> = doc.channels.iter().map(|channel| (channel.id, channel)).collect();
        if channels.len() != doc.channels.len() || channels.keys().any(|id| id.0 == 0) {
            return Err(invalid("channel_identity"));
        }
        let mut channel_ids: BTreeSet<_> = channels.keys().copied().collect();
        if let Some(id) = doc.selection_channel_id
            && (id.0 == 0 || !channel_ids.insert(id))
        {
            return Err(invalid("channel_identity"));
        }
        let paths: BTreeMap<_, _> = doc.paths.iter().map(|path| (path.id, path)).collect();
        if paths.len() != doc.paths.len() || paths.keys().any(|id| id.0 == 0) {
            return Err(invalid("path_identity"));
        }
        self.check_clone_bounds()?;
        let mut next = self.clone();
        let mut removed = BTreeSet::new();
        let filter_mask_items: BTreeSet<_> = next.filter_masks.iter().map(|mask| ItemRef::Mask(mask.id)).collect();
        for item in &next.items {
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
        next.items.retain(|item| !removed.contains(&item.item));
        for item in &mut next.items {
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
            if let Some(item) = next.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)) {
                if item.name != layer.name {
                    item.name.clone_from(&layer.name);
                    item.custom_name = true;
                }
                item.visible = layer.visible;
                item.parent = parent.map(ItemRef::Layer);
            } else {
                next.items.push(ItemState {
                    item: ItemRef::Layer(id),
                    name: layer.name.clone(),
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
            if let Some(item) = next.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)) {
                if item.name != channel.name {
                    item.name.clone_from(&channel.name);
                    item.custom_name = true;
                }
            } else {
                next.items.push(ItemState {
                    item: ItemRef::Channel(id),
                    name: channel.name.clone(),
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
            && !next.items.iter().any(|item| item.item == ItemRef::Channel(id))
        {
            next.items.push(ItemState {
                item: ItemRef::Channel(id),
                name: "Selection".into(),
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
            if let Some(item) = next.items.iter_mut().find(|item| item.item == ItemRef::Path(id)) {
                if item.name != path.name {
                    item.name.clone_from(&path.name);
                    item.custom_name = true;
                }
            } else {
                next.items.push(ItemState {
                    item: ItemRef::Path(id),
                    name: path.name.clone(),
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
        next.filter_stacks.retain(|stack| !removed.contains(&stack.owner));
        let retained_filters: BTreeSet<_> = next.filter_stacks.iter().flat_map(|stack| stack.nodes.iter().map(|node| node.id)).collect();
        next.filter_masks.retain(|mask| {
            let keep = retained_filters.contains(&mask.owner);
            if !keep {
                removed.insert(ItemRef::Mask(mask.id));
            }
            keep
        });
        next.items.retain(|item| !removed.contains(&item.item));
        let owner_survives = |owner: ItemRef| !matches!(owner, ItemRef::Layer(id) if !layers.contains_key(&id)) && !removed.contains(&owner);
        next.source_items.retain(|item| owner_survives(item.owner));
        next.editable_indexed.retain(|item| layers.contains_key(&item.owner));
        next.filter_stacks.retain(|stack| owner_survives(stack.owner));
        next.opaque.retain(|record| record.owner.is_none_or(owner_survives));
        next.selection.selected.retain(|item| owner_survives(*item));
        next.selection.selected_paths.retain(|item| owner_survives(*item));
        if next.selection.active.is_some_and(|item| !owner_survives(item)) {
            next.selection.active = None;
        }
        if next.selection.mask_target.is_some_and(|item| !owner_survives(item)) {
            next.selection.mask_target = None;
        }
        for set in &mut next.item_sets {
            set.members.retain(|item| owner_survives(*item));
        }
        if next.floating.as_ref().is_some_and(|floating| !layers.contains_key(&floating.layer)) {
            next.floating = None;
        }
        next.reconcile_filter_output_precision();
        next.validate(doc)?;
        *self = next;
        Ok(())
    }

    /// Reconcile on a fallible private metadata stage, polling collection/config
    /// traversal and validation. Cancellation never publishes a partial state.
    pub fn reconcile_layers_with_interrupt(&mut self, doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        controlled::reconcile(self, doc, control)
    }

    /// Clone a saved path's owned registry/provenance with an explicit fresh canonical ID.
    /// Global editing selections and saved sets are not duplicated; vector-source references
    /// continue to point at their original paths until explicitly edited.
    pub fn clone_path_metadata(&mut self, source: PathId, target: PathId, doc: &Document) -> Result<()> {
        self.check_clone_bounds()?;
        let from = ItemRef::Path(source);
        let to = ItemRef::Path(target);
        if source == target
            || source.0 == 0
            || target.0 == 0
            || !doc.paths.iter().any(|path| path.id == source)
            || !doc.paths.iter().any(|path| path.id == target)
            || self.items.iter().any(|item| item.item == to)
        {
            return Err(invalid("clone_path_id"));
        }
        let mut next = self.clone();
        if let Some(item) = self.items.iter().find(|item| item.item == from) {
            let mut copy = item.clone();
            copy.item = to;
            next.items.push(copy);
        }
        for record in self.opaque.iter().filter(|record| record.owner == Some(from)) {
            let mut copy = record.clone();
            copy.owner = Some(to);
            next.opaque.push(copy);
        }
        next.reconcile_layers(doc)?;
        *self = next;
        Ok(())
    }

    /// Controlled path-owned cloning; failed reconciliation restores original records and capacities.
    pub fn clone_path_metadata_with_interrupt(&mut self, source: PathId, target: PathId, doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        path_clone::clone_path(self, source, target, doc, control)
    }

    /// Clone a canonical channel's owned metadata without duplicating global selections or sets.
    /// Self auxiliary references follow the copy; unrelated dependencies and resources stay shared.
    pub fn clone_channel_metadata(&mut self, source: ChannelId, target: ChannelId, doc: &Document, filter_ids: &BTreeMap<FilterId, FilterId>) -> Result<()> {
        self.clone_channel_metadata_with_interrupt(source, target, doc, filter_ids, &photocraft_raster::Interrupt::NONE)
    }
    pub fn clone_channel_metadata_with_interrupt(&mut self, source: ChannelId, target: ChannelId, doc: &Document, filter_ids: &BTreeMap<FilterId, FilterId>, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        channel_clone::clone_channel(self, source, target, doc, filter_ids, control)
    }

    /// Clone one layer's imported metadata with caller-allocated typed mask/filter IDs.
    /// Global resources and saved paths remain shared references. Internal owner/auxiliary
    /// references are redirected to the clone; unrelated dependencies remain exact.
    /// Floating attachments and global selection/set membership are not duplicated.
    pub fn clone_layer_metadata(
        &mut self,
        source: LayerId,
        target: LayerId,
        doc: &Document,
        mask_ids: &BTreeMap<MaskId, MaskId>,
        filter_ids: &BTreeMap<FilterId, FilterId>,
    ) -> Result<()> {
        self.clone_layers_metadata(&BTreeMap::from([(source, target)]), doc, mask_ids, filter_ids)
    }

    pub fn clone_layer_metadata_with_interrupt(&mut self, source: LayerId, target: LayerId, doc: &Document, mask_ids: &BTreeMap<MaskId, MaskId>, filter_ids: &BTreeMap<FilterId, FilterId>, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        layer_clone::clone_layers(self, &BTreeMap::from([(source, target)]), doc, mask_ids, filter_ids, control)
    }

    /// Controlled complete layer-set cloning; source payloads stay shared and
    /// cancellation cannot publish partially remapped metadata.
    pub fn clone_layers_metadata_with_interrupt(&mut self, layer_ids: &BTreeMap<LayerId, LayerId>, doc: &Document, mask_ids: &BTreeMap<MaskId, MaskId>, filter_ids: &BTreeMap<FilterId, FilterId>, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        layer_clone::clone_layers(self, layer_ids, doc, mask_ids, filter_ids, control)
    }

    /// Clone a complete recursive layer set in one transaction. Every duplicated descendant
    /// must be included explicitly. Masks/filters receive caller-allocated fresh typed IDs.
    /// References between copied layers are redirected together; external dependencies remain.
    pub fn clone_layers_metadata(
        &mut self,
        layer_ids: &BTreeMap<LayerId, LayerId>,
        doc: &Document,
        mask_ids: &BTreeMap<MaskId, MaskId>,
        filter_ids: &BTreeMap<FilterId, FilterId>,
    ) -> Result<()> {
        let layers = layer_tree(doc)?;
        self.check_clone_bounds()?;
        if layer_ids.is_empty() || layer_ids.len() > 10000 {
            return Err(invalid("clone_layer"));
        }
        let targets: BTreeSet<_> = layer_ids.values().copied().collect();
        if targets.len() != layer_ids.len() {
            return Err(invalid("clone_layer"));
        }
        for (&source, &target) in layer_ids {
            let source_layer = layers.get(&source).ok_or_else(|| invalid("clone_layer"))?.0;
            let target_layer = layers.get(&target).ok_or_else(|| invalid("clone_layer"))?.0;
            if source == target
                || layer_ids.contains_key(&target)
                || source_layer.is_group() != target_layer.is_group()
                || !self.items.iter().any(|item| item.item == ItemRef::Layer(source))
                || self.items.iter().any(|item| item.item == ItemRef::Layer(target))
            {
                return Err(invalid("clone_layer"));
            }
            if let Some(parent) = layers.get(&source).and_then(|entry| entry.1).and_then(|id| layer_ids.get(&id)).copied()
                && layers.get(&target).and_then(|entry| entry.1) != Some(parent)
            {
                return Err(invalid("clone_hierarchy"));
            }
            if let (crate::LayerContent::Group(source_group), crate::LayerContent::Group(target_group)) = (&source_layer.content, &target_layer.content) {
                let expected: Option<BTreeSet<_>> = source_group.children.iter().map(|child| layer_ids.get(&child.id).copied()).collect();
                let actual: BTreeSet<_> = target_group.children.iter().map(|child| child.id).collect();
                if expected.as_ref() != Some(&actual) {
                    return Err(invalid("clone_descendant"));
                }
            }
        }
        let mut owned_masks: BTreeSet<MaskId> = self
            .items
            .iter()
            .filter_map(|item| match item.item {
                ItemRef::Mask(id) if item.mask.as_ref().is_some_and(|mask| matches!(mask.owner, ItemRef::Layer(owner) if layer_ids.contains_key(&owner))) => {
                    Some(id)
                }
                _ => None,
            })
            .collect();
        let owned_filters: BTreeSet<_> = self
            .filter_stacks
            .iter()
            .filter(|stack| matches!(stack.owner, ItemRef::Layer(id) if layer_ids.contains_key(&id)))
            .flat_map(|stack| stack.nodes.iter().map(|node| node.id))
            .collect();
        owned_masks.extend(self.filter_masks.iter().filter(|mask| owned_filters.contains(&mask.owner)).map(|mask| mask.id));
        let remap = |item: ItemRef| -> Result<ItemRef> {
            match item {
                ItemRef::Layer(id) => Ok(ItemRef::Layer(layer_ids.get(&id).copied().unwrap_or(id))),
                ItemRef::Mask(id) if owned_masks.contains(&id) => mask_ids.get(&id).copied().map(ItemRef::Mask).ok_or_else(|| invalid("clone_mask_id")),
                _ => Ok(item),
            }
        };
        let owns =
            |item: ItemRef| matches!(item,ItemRef::Layer(id) if layer_ids.contains_key(&id)) || matches!(item,ItemRef::Mask(id) if owned_masks.contains(&id));
        let mut next = self.clone();
        for item in self.items.iter().filter(|item| owns(item.item)) {
            let mut copied = item.clone();
            copied.item = remap(copied.item)?;
            if let Some(mask) = &mut copied.mask {
                mask.owner = remap(mask.owner)?;
            }
            if let Some(parent) = &mut copied.parent {
                *parent = remap(*parent)?;
            }
            next.items.push(copied);
        }
        for item in self.source_items.iter().filter(|item| owns(item.owner)) {
            let mut copied = item.clone();
            copied.owner = remap(copied.owner)?;
            next.source_items.push(copied);
        }
        for item in self.editable_indexed.iter().filter(|item| layer_ids.contains_key(&item.owner)) {
            let mut copied = item.clone();
            copied.owner = layer_ids.get(&item.owner).copied().ok_or_else(|| invalid("clone_layer"))?;
            next.editable_indexed.push(copied);
        }
        for stack in self.filter_stacks.iter().filter(|stack| owns(stack.owner)) {
            let mut copied = stack.clone();
            copied.owner = remap(copied.owner)?;
            for node in &mut copied.nodes {
                node.id = filter_ids.get(&node.id).copied().ok_or_else(|| invalid("clone_filter_id"))?;
                if let Some(mask) = &mut node.mask {
                    *mask = remap(*mask)?;
                }
                for (_, input) in &mut node.auxiliary {
                    *input = remap(*input)?;
                }
            }
            next.filter_stacks.push(copied);
        }
        for mask in self.filter_masks.iter().filter(|mask| owned_filters.contains(&mask.owner)) {
            let mut copy = mask.clone();
            copy.id = mask_ids.get(&mask.id).copied().ok_or_else(|| invalid("clone_mask_id"))?;
            copy.owner = filter_ids.get(&mask.owner).copied().ok_or_else(|| invalid("clone_filter_id"))?;
            next.filter_masks.push(copy);
        }
        for record in self.opaque.iter().filter(|record| record.owner.is_some_and(owns)) {
            let mut copied = record.clone();
            copied.owner = copied.owner.map(remap).transpose()?;
            next.opaque.push(copied);
        }
        next.reconcile_layers(doc)?;
        *self = next;
        Ok(())
    }
}
