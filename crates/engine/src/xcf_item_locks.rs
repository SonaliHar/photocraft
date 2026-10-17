//! Checked native ancestry and imported item locks for command consumers.
use crate::{EngineError, Result};
use photocraft_doc::{
    Document, Layer, LayerContent, LayerId, Locks, MAX_GROUP_DEPTH,
    item_identity::ItemRef,
    xcf_state::{ItemLocks, ItemState},
};
use std::collections::HashMap;

const MAX_ITEMS: usize = 10_000;
fn bad(message: &str) -> EngineError {
    EngineError::Other(format!("xcf.item.locks: {message}"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectiveItemLocks {
    pub native: Locks,
    pub imported: ItemLocks,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Content,
    Position,
    Alpha,
    Visibility,
}
impl EffectiveItemLocks {
    pub fn require(self, edit: EditKind) -> Result<()> {
        let locked = match edit {
            EditKind::Content => self.native.all || self.native.pixels || self.imported.content,
            EditKind::Position => self.native.all || self.native.position || self.imported.position,
            EditKind::Alpha => self.native.all || self.native.transparency || self.imported.alpha,
            // Photoshop's Lock All does not forbid toggling visibility. Imported
            // visibility locks are an explicit distinct canonical permission.
            EditKind::Visibility => self.imported.visibility,
        };
        if locked { Err(bad("requested edit is locked")) } else { Ok(()) }
    }
    fn union(self, other: Self) -> Self {
        Self { native: self.native.union(other.native), imported: self.imported.union(other.imported) }
    }
}

fn imported_items(doc: &Document) -> Result<HashMap<ItemRef, &ItemState>> {
    let mut result = HashMap::new();
    let Some(state) = &doc.xcf_state else { return Ok(result) };
    if state.items.len() > MAX_ITEMS {
        return Err(bad("item limit"));
    }
    result.try_reserve(state.items.len()).map_err(|_| bad("allocation"))?;
    for item in &state.items {
        if item.item.raw() == 0 || result.insert(item.item, item).is_some() {
            return Err(bad("duplicate or zero item identity"));
        }
    }
    Ok(result)
}

fn imported_chain(items: &HashMap<ItemRef, &ItemState>, start: ItemRef) -> Result<ItemLocks> {
    let mut seen = Vec::new();
    seen.try_reserve_exact(MAX_GROUP_DEPTH + 1).map_err(|_| bad("allocation"))?;
    imported_chain_with_seen(items, start, &mut seen)
}
fn imported_chain_with_seen(items: &HashMap<ItemRef, &ItemState>, start: ItemRef, seen: &mut Vec<ItemRef>) -> Result<ItemLocks> {
    let mut result = ItemLocks::default();
    seen.clear();
    let mut next = Some(start);
    while let Some(id) = next {
        if id.raw() == 0 || seen.contains(&id) || seen.len() > MAX_GROUP_DEPTH {
            return Err(bad("invalid or cyclic imported ancestry"));
        }
        seen.push(id);
        let item = items.get(&id).ok_or_else(|| bad("missing imported item"))?;
        result = result.union(item.locks);
        if item.parent.is_some_and(|parent| !matches!(parent, ItemRef::Layer(_))) {
            return Err(bad("parent is not a layer"));
        }
        next = item.parent;
    }
    Ok(result)
}

// Build a bounded borrowed tree census; never recurse or inspect raster samples.
fn layer_ancestry(doc: &Document, target: LayerId) -> Result<Vec<&Layer>> {
    if target.0 == 0 || doc.layers.len() > MAX_ITEMS {
        return Err(bad("invalid layer identity or item limit"));
    }
    let mut pending = Vec::new();
    let mut layers = HashMap::new();
    pending.try_reserve_exact(MAX_ITEMS).map_err(|_| bad("allocation"))?;
    layers.try_reserve(MAX_ITEMS).map_err(|_| bad("allocation"))?;
    pending.extend(doc.layers.iter().map(|layer| (layer, None, 0usize)));
    while let Some((layer, parent, depth)) = pending.pop() {
        if depth > MAX_GROUP_DEPTH || layer.id.0 == 0 || layers.len() >= MAX_ITEMS {
            return Err(bad("layer hierarchy limit"));
        }
        if layers.insert(layer.id, (layer, parent)).is_some() {
            return Err(bad("duplicate layer identity"));
        }
        if let LayerContent::Group(group) = &layer.content {
            if pending.len().checked_add(layers.len()).and_then(|n| n.checked_add(group.children.len())).is_none_or(|n| n > MAX_ITEMS) {
                return Err(bad("layer hierarchy limit"));
            }
            let child_depth = depth.checked_add(1).ok_or_else(|| bad("layer hierarchy limit"))?;
            pending.extend(group.children.iter().map(|child| (child, Some(layer.id), child_depth)));
        }
    }
    let mut result = Vec::new();
    result.try_reserve_exact(MAX_GROUP_DEPTH + 1).map_err(|_| bad("allocation"))?;
    let mut next = Some(target);
    while let Some(id) = next {
        let (layer, parent) = layers.get(&id).ok_or(EngineError::NoLayer(id))?;
        result.push(*layer);
        next = *parent;
    }
    Ok(result)
}

/// Native locks and canonical imported flags inherited by the actual layer tree.
/// Newly created ordinary children still inherit imported locks from their groups.
pub fn effective_layer_locks(doc: &Document, target: LayerId) -> Result<EffectiveItemLocks> {
    let items = imported_items(doc)?;
    layer_locks(doc, target, &items)
}
fn layer_locks(doc: &Document, target: LayerId, items: &HashMap<ItemRef, &ItemState>) -> Result<EffectiveItemLocks> {
    let mut result = EffectiveItemLocks::default();
    for layer in layer_ancestry(doc, target)? {
        result.native = result.native.union(layer.locks);
        let item = ItemRef::Layer(layer.id);
        if items.contains_key(&item) {
            result.imported = result.imported.union(imported_chain(items, item)?);
        }
    }
    Ok(result)
}

/// Exact typed item query; masks additionally inherit their declared owner's locks.
/// Missing canonical targets are errors, never an unlocked name/index fallback.
pub fn effective_item_locks(doc: &Document, target: ItemRef) -> Result<EffectiveItemLocks> {
    if let ItemRef::Layer(id) = target {
        return effective_layer_locks(doc, id);
    }
    let items = imported_items(doc)?;
    let item = items.get(&target).ok_or_else(|| bad("missing imported item"))?;
    let mut result = EffectiveItemLocks { native: Locks::default(), imported: imported_chain(&items, target)? };
    if let Some(ItemRef::Layer(parent)) = item.parent {
        result = result.union(layer_locks(doc, parent, &items)?);
    }
    if matches!(target, ItemRef::Mask(_)) {
        // Filter masks have a FilterId owner in their raster payload, whereas
        // layer/channel masks carry the owner here. No owner is guessed by name.
        if let Some(mask) = &item.mask {
            match mask.owner {
                ItemRef::Layer(owner) => result = result.union(layer_locks(doc, owner, &items)?),
                ItemRef::Channel(owner) => {
                    let owner_ref = ItemRef::Channel(owner);
                    result.imported = result.imported.union(imported_chain(&items, owner_ref)?);
                    if let Some(ItemRef::Layer(parent)) = items.get(&owner_ref).and_then(|item| item.parent) {
                        result = result.union(layer_locks(doc, parent, &items)?);
                    }
                }
                _ => return Err(bad("invalid mask owner")),
            }
        } else {
            let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing imported state"))?;
            if state.filter_masks.len() > MAX_ITEMS || state.filter_stacks.len() > MAX_ITEMS {
                return Err(bad("filter mask item limit"));
            }
            let mut payload = None;
            for mask in &state.filter_masks {
                if ItemRef::Mask(mask.id) == target {
                    if payload.is_some() {
                        return Err(bad("duplicate filter mask identity"));
                    }
                    payload = Some(mask);
                }
            }
            let payload = payload.ok_or_else(|| bad("missing mask owner"))?;
            if payload.owner.0 == 0 {
                return Err(bad("zero filter owner"));
            }
            let mut visits = 0usize;
            for stack in &state.filter_stacks {
                visits = visits.checked_add(stack.nodes.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("filter owner work limit"))?;
            }
            let mut owner = None;
            for stack in &state.filter_stacks {
                for node in &stack.nodes {
                    if node.id == payload.owner {
                        if owner.is_some() || node.mask != Some(target) {
                            return Err(bad("invalid filter mask association"));
                        }
                        owner = Some(stack.owner);
                    }
                }
            }
            match owner.ok_or_else(|| bad("missing filter owner"))? {
                ItemRef::Layer(id) => result = result.union(layer_locks(doc, id, &items)?),
                ItemRef::Channel(id) => {
                    let owner_ref = ItemRef::Channel(id);
                    result.imported = result.imported.union(imported_chain(&items, owner_ref)?);
                    if let Some(ItemRef::Layer(parent)) = items.get(&owner_ref).and_then(|item| item.parent) {
                        result = result.union(layer_locks(doc, parent, &items)?);
                    }
                }
                _ => return Err(bad("invalid filter owner type")),
            }
        }
    }
    Ok(result)
}

/// Preflight inherited imported permissions over a whole layer subtree before mutation.
/// Native permissions stay with existing transform kernels, including their explicit
/// Background conversion policy. One borrowed census and one scratch set serve the walk.
pub fn require_layer_subtree(doc: &Document, target: LayerId, edit: EditKind) -> Result<()> {
    let ancestry = layer_ancestry(doc, target)?;
    let root = ancestry.first().copied().ok_or(EngineError::NoLayer(target))?;
    let items = imported_items(doc)?;
    let mut seen = Vec::new();
    seen.try_reserve_exact(MAX_GROUP_DEPTH + 1).map_err(|_| bad("allocation"))?;
    let mut inherited = ItemLocks::default();
    for ancestor in &ancestry {
        let id = ItemRef::Layer(ancestor.id);
        if items.contains_key(&id) {
            inherited = inherited.union(imported_chain_with_seen(&items, id, &mut seen)?);
        }
    }
    let mut pending = Vec::new();
    pending.try_reserve_exact(MAX_ITEMS).map_err(|_| bad("allocation"))?;
    pending.push((root, inherited));
    let mut visits = 0usize;
    while let Some((layer, mut inherited)) = pending.pop() {
        visits = visits.checked_add(1).filter(|v| *v <= MAX_ITEMS).ok_or_else(|| bad("layer hierarchy limit"))?;
        let id = ItemRef::Layer(layer.id);
        if items.contains_key(&id) {
            inherited = inherited.union(imported_chain_with_seen(&items, id, &mut seen)?);
        }
        EffectiveItemLocks { native: Locks::default(), imported: inherited }.require(edit)?;
        if let LayerContent::Group(group) = &layer.content {
            if pending.len().checked_add(visits).and_then(|n| n.checked_add(group.children.len())).is_none_or(|n| n > MAX_ITEMS) {
                return Err(bad("layer hierarchy limit"));
            }
            pending.extend(group.children.iter().map(|child| (child, inherited)));
        }
    }
    Ok(())
}

/// Layer-mask painting respects Lock All and imported content permissions, while
/// a native pixel-only lock continues to permit editing the separate mask surface.
pub fn require_layer_mask_content(doc: &Document, owner: LayerId) -> Result<()> {
    // Ordinary documents retain their existing tree lookup and avoid allocating
    // an imported-item census for every mask painting request.
    if doc.xcf_state.is_none() {
        doc.layer(owner).ok_or(EngineError::NoLayer(owner))?;
        return if doc.effective_locks(owner).all { Err(bad("layer mask content is locked")) } else { Ok(()) };
    }
    let items = imported_items(doc)?;
    let inherited = layer_locks(doc, owner, &items)?;
    if inherited.native.all || inherited.imported.content {
        return Err(bad("layer mask content is locked"));
    }
    let Some(state) = &doc.xcf_state else { return Ok(()) };
    if state.filter_masks.len() > MAX_ITEMS {
        return Err(bad("filter mask item limit"));
    }
    let mut filter_masks = std::collections::HashSet::new();
    filter_masks.try_reserve(state.filter_masks.len()).map_err(|_| bad("allocation"))?;
    for mask in &state.filter_masks {
        if mask.id.0 == 0 || !filter_masks.insert(mask.id) {
            return Err(bad("invalid filter mask identity"));
        }
    }
    let mut target = None;
    for item in items.values() {
        if let ItemRef::Mask(id) = item.item
            && !filter_masks.contains(&id)
            && item.mask.as_ref().is_some_and(|mask| mask.owner == ItemRef::Layer(owner))
        {
            if target.is_some() {
                return Err(bad("ambiguous layer mask association"));
            }
            target = Some(item.item);
        }
    }
    let target = target.ok_or_else(|| bad("unsupported layer mask without typed association"))?;
    if imported_chain(&items, target)?.content {
        return Err(bad("layer mask content is locked"));
    }
    Ok(())
}

/// Batch imported visibility preflight. Each operation builds one borrowed native
/// census and one imported map; shared imported ancestor chains are memoized.
/// Native pixel/all locks do not become visibility permissions.
pub fn require_visibility_changes(doc: &Document, changed: &[LayerId], control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    if changed.len() > MAX_ITEMS || doc.layers.len() > MAX_ITEMS {
        return Err(bad("item limit"));
    }
    if changed.is_empty() {
        return Ok(());
    }
    let items = imported_items(doc)?;
    let mut pending = Vec::new();
    let mut layers = HashMap::new();
    pending.try_reserve_exact(MAX_ITEMS).map_err(|_| bad("allocation"))?;
    layers.try_reserve(MAX_ITEMS).map_err(|_| bad("allocation"))?;
    pending.extend(doc.layers.iter().map(|layer| (layer, None, 0usize)));
    while let Some((layer, parent, depth)) = pending.pop() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if depth > MAX_GROUP_DEPTH || layer.id.0 == 0 || layers.len() >= MAX_ITEMS {
            return Err(bad("layer hierarchy limit"));
        }
        if layers.insert(layer.id, (layer, parent)).is_some() {
            return Err(bad("duplicate layer identity"));
        }
        if let LayerContent::Group(group) = &layer.content {
            if pending.len().checked_add(layers.len()).and_then(|n| n.checked_add(group.children.len())).is_none_or(|n| n > MAX_ITEMS) {
                return Err(bad("layer hierarchy limit"));
            }
            let depth = depth.checked_add(1).ok_or_else(|| bad("layer hierarchy limit"))?;
            pending.extend(group.children.iter().map(|child| (child, Some(layer.id), depth)));
        }
    }
    let mut memo = HashMap::<ItemRef, (bool, usize)>::new();
    memo.try_reserve(items.len()).map_err(|_| bad("allocation"))?;
    let mut scratch = Vec::new();
    scratch.try_reserve_exact(MAX_GROUP_DEPTH + 1).map_err(|_| bad("allocation"))?;
    let mut targets = std::collections::HashSet::new();
    targets.try_reserve(changed.len()).map_err(|_| bad("allocation"))?;
    for &target in changed {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if target.0 == 0 || !targets.insert(target) {
            return Err(bad("invalid changed layer identity"));
        }
        let layer = layers.get(&target).ok_or(EngineError::NoLayer(target))?.0;
        let mut next = Some(target);
        let mut visits = 0usize;
        while let Some(id) = next {
            control.check().map_err(|_| EngineError::Cancelled)?;
            visits = visits.checked_add(1).filter(|n| *n <= MAX_GROUP_DEPTH + 1).ok_or_else(|| bad("layer hierarchy limit"))?;
            let (_, parent) = layers.get(&id).ok_or(EngineError::NoLayer(id))?;
            let item = ItemRef::Layer(id);
            if items.contains_key(&item) && imported_visibility(&items, item, &mut memo, &mut scratch, control)? {
                return Err(EngineError::Other(format!("the visibility of layer {:?} is locked", layer.name)));
            }
            next = *parent;
        }
    }
    Ok(())
}
fn imported_visibility(
    items: &HashMap<ItemRef, &ItemState>,
    start: ItemRef,
    memo: &mut HashMap<ItemRef, (bool, usize)>,
    scratch: &mut Vec<ItemRef>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<bool> {
    scratch.clear();
    let mut next = Some(start);
    let mut inherited = false;
    let mut tail_depth = 0usize;
    while let Some(id) = next {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if let Some(&(locked, depth)) = memo.get(&id) {
            if scratch.len().checked_add(depth).is_none_or(|n| n > MAX_GROUP_DEPTH + 1) {
                return Err(bad("imported hierarchy limit"));
            }
            inherited = locked;
            tail_depth = depth;
            break;
        }
        if id.raw() == 0 || scratch.contains(&id) || scratch.len() > MAX_GROUP_DEPTH {
            return Err(bad("invalid or cyclic imported ancestry"));
        }
        scratch.push(id);
        let item = items.get(&id).ok_or_else(|| bad("missing imported item"))?;
        if item.parent.is_some_and(|parent| !matches!(parent, ItemRef::Layer(_))) {
            return Err(bad("parent is not a layer"));
        }
        next = item.parent;
    }
    for &id in scratch.iter().rev() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let item = items.get(&id).ok_or_else(|| bad("missing imported item"))?;
        inherited |= item.locks.visibility;
        tail_depth = tail_depth.checked_add(1).filter(|n| *n <= MAX_GROUP_DEPTH + 1).ok_or_else(|| bad("imported hierarchy limit"))?;
        memo.insert(id, (inherited, tail_depth));
    }
    Ok(inherited)
}

/// Existing ordinary transparency policy plus independent imported alpha locks.
/// Consumers preserve alpha while permitting color edits; this is not a paint refusal.
pub fn transparency_locked(doc: &Document, id: LayerId) -> Result<bool> {
    if doc.xcf_state.is_none() {
        return Ok(doc.effective_locks(id).transparency);
    }
    let locks = effective_layer_locks(doc, id)?;
    Ok(locks.native.transparency || locks.imported.alpha)
}

/// Content preflight for an existing canonical alpha channel. Legacy index inputs
/// must first resolve to this typed ID; names never identify the target. A live
/// newly-created ordinary channel without imported metadata remains editable.
pub fn require_channel_content(doc: &Document, target: photocraft_doc::item_identity::ChannelId) -> Result<()> {
    if target.0 == 0 || doc.channels.len() > MAX_ITEMS {
        return Err(bad("invalid channel identity or item limit"));
    }
    let mut identities = std::collections::HashSet::new();
    identities.try_reserve(doc.channels.len()).map_err(|_| bad("allocation"))?;
    let mut found = false;
    for channel in &doc.channels {
        if channel.id.0 == 0 || !identities.insert(channel.id) {
            return Err(bad("duplicate or zero channel identity"));
        }
        found |= channel.id == target;
    }
    if !found {
        return Err(bad("missing canonical channel"));
    }
    let items = imported_items(doc)?;
    let reference = ItemRef::Channel(target);
    if items.contains_key(&reference) {
        effective_item_locks(doc, reference)?.require(EditKind::Content)?;
    }
    Ok(())
}

#[cfg(test)]
mod channel_lock_tests {
    use super::*;
    use photocraft_doc::{AlphaChannel, ColorMode, SampleType, Size, XcfState};
    #[test]
    fn channel_content_is_typed_and_preserves_ordinary_unbound_channels() {
        let mut document = Document::new("Own channel lock draft", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        document.channels.push(AlphaChannel::new("Duplicate-safe name", photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8)));
        let id = document.channels.first().unwrap().id;
        require_channel_content(&document, id).unwrap();
        document.xcf_state = Some(XcfState::default());
        require_channel_content(&document, id).unwrap();
        document.xcf_state.as_mut().unwrap().items.push(ItemState {
            item: ItemRef::Channel(id),
            name: "Retained source name".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks { content: true, ..Default::default() },
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: None,
            mask: None,
            composition: None,
        });
        let before = document.clone();
        assert!(require_channel_content(&document, id).is_err());
        assert_eq!(document, before);
        assert!(require_channel_content(&document, photocraft_doc::item_identity::ChannelId(0)).is_err());
        document.channels.push(document.channels.first().unwrap().clone());
        assert!(require_channel_content(&document, id).is_err());
    }
}
