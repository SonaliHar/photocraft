//! Current identity remapping, without copying retained source or raster payloads.
use crate::{
    XcfState, XcfValidationError,
    item_identity::{FilterId, ItemRef, ItemSetId, MaskId},
};

type Result<T> = std::result::Result<T, XcfValidationError>;
const MAX_IDENTITIES: usize = 10_000;
const MAX_VISITS: usize = 100_000;
fn invalid(code: &'static str) -> XcfValidationError {
    XcfValidationError::new(code, "invalid current identity remapping")
}

// Flat typed identity fields cannot contain recursive configurations. Check vector
// lengths before iterating them; no raw/configuration payload is traversed or cloned.
fn bounds(state: &XcfState) -> Result<usize> {
    let mut count = 0usize;
    let mut add = |n: usize| -> Result<()> {
        count = count.checked_add(n).filter(|v| *v <= MAX_VISITS).ok_or_else(|| invalid("remap_work_limit"))?;
        Ok(())
    };
    for n in [state.items.len(), state.source_items.len(), state.filter_stacks.len(), state.filter_masks.len(), state.item_sets.len()] {
        if n > MAX_IDENTITIES {
            return Err(invalid("item_limit"));
        }
        add(n)?;
    }
    add(state.opaque.len())?;
    add(state.selection.selected.len())?;
    add(state.selection.selected_paths.len())?;
    add(usize::from(state.selection.active.is_some()))?;
    add(usize::from(state.selection.mask_target.is_some()))?;
    add(usize::from(state.floating.is_some()))?;
    for item in &state.items {
        add(usize::from(item.parent.is_some()))?;
        add(usize::from(item.mask.is_some()))?;
    }
    for stack in &state.filter_stacks {
        add(stack.nodes.len())?;
        for node in &stack.nodes {
            add(usize::from(node.mask.is_some()))?;
            add(node.auxiliary.len())?;
        }
    }
    for set in &state.item_sets {
        add(set.members.len())?;
    }
    // A filter mask holds two identities, whereas its vector counted one above.
    add(state.filter_masks.len())?;
    Ok(count)
}

struct Plan(Vec<(u64, u64)>);
impl Plan {
    fn new(mut sources: Vec<u64>, mut map: impl FnMut(u64) -> Option<u64>) -> Result<Self> {
        if sources.len() > MAX_IDENTITIES {
            return Err(invalid("item_limit"));
        }
        sources.sort_unstable();
        let mut previous = None;
        for &source in &sources {
            if source == 0 || previous == Some(source) {
                return Err(invalid("remap_identity"));
            }
            previous = Some(source);
        }
        let mut entries = Vec::new();
        let mut targets = Vec::new();
        entries.try_reserve_exact(sources.len()).map_err(|_| invalid("remap_allocation"))?;
        targets.try_reserve_exact(sources.len()).map_err(|_| invalid("remap_allocation"))?;
        for source in sources {
            let target = map(source).filter(|v| *v != 0).ok_or_else(|| invalid("remap_reference"))?;
            entries.push((source, target));
            targets.push(target);
        }
        targets.sort_unstable();
        let mut previous = None;
        for target in targets {
            if previous == Some(target) {
                return Err(invalid("remap_collision"));
            }
            previous = Some(target);
        }
        Ok(Self(entries))
    }
    fn target(&self, source: u64) -> Result<u64> {
        let index = self.0.binary_search_by_key(&source, |entry| entry.0).map_err(|_| invalid("remap_reference"))?;
        self.0.get(index).map(|entry| entry.1).ok_or_else(|| invalid("remap_reference"))
    }
}
fn sources(capacity: usize) -> Result<Vec<u64>> {
    let mut result = Vec::new();
    result.try_reserve_exact(capacity).map_err(|_| invalid("remap_allocation"))?;
    Ok(result)
}

// Each retained current mask surface has one canonical identity; duplicate
// payload records cannot acquire authority from a unique ItemState declaration.
fn mask_payload_identities(state: &XcfState) -> Result<()> {
    let mut ids = sources(state.filter_masks.len())?;
    ids.extend(state.filter_masks.iter().map(|mask| mask.id.0));
    ids.sort_unstable();
    let mut previous = None;
    for id in ids {
        if id == 0 || previous == Some(id) {
            return Err(invalid("remap_identity"));
        }
        previous = Some(id);
    }
    Ok(())
}

enum Slot<'a> {
    Item(&'a mut ItemRef),
    Mask(&'a mut MaskId),
    Filter(&'a mut FilterId),
    Set(&'a mut ItemSetId),
}
impl Slot<'_> {
    fn raw(&self) -> u64 {
        match self {
            Self::Item(id) => id.raw(),
            Self::Mask(id) => id.0,
            Self::Filter(id) => id.0,
            Self::Set(id) => id.0,
        }
    }
    fn publish(self, raw: u64) {
        match self {
            Self::Item(id) => *id = ItemRef::Mask(MaskId(raw)),
            Self::Mask(id) => *id = MaskId(raw),
            Self::Filter(id) => *id = FilterId(raw),
            Self::Set(id) => *id = ItemSetId(raw),
        }
    }
}
fn publish(slots: Vec<Slot<'_>>, plan: &Plan) -> Result<()> {
    let mut targets = Vec::new();
    targets.try_reserve_exact(slots.len()).map_err(|_| invalid("remap_allocation"))?;
    for slot in &slots {
        targets.push(plan.target(slot.raw())?);
    }
    // Every lookup and allocation has completed; no callback or fallible work follows.
    for (slot, target) in slots.into_iter().zip(targets) {
        slot.publish(target);
    }
    Ok(())
}
fn mask<'a>(slots: &mut Vec<Slot<'a>>, item: &'a mut ItemRef) {
    if matches!(item, ItemRef::Mask(_)) {
        slots.push(Slot::Item(item));
    }
}

impl XcfState {
    /// Remap canonical current masks and every typed current mask association.
    /// Source tattoos, source/cache bytes, original pipeline and opaque bytes are retained.
    pub fn remap_masks(&mut self, mut map: impl FnMut(MaskId) -> Option<MaskId>) -> Result<()> {
        let visits = bounds(self)?;
        mask_payload_identities(self)?;
        let mut ids = sources(self.items.len())?;
        for item in &self.items {
            if let ItemRef::Mask(id) = item.item {
                ids.push(id.0);
            }
        }
        let plan = Plan::new(ids, |id| map(MaskId(id)).map(|target| target.0))?;
        let mut slots = Vec::new();
        slots.try_reserve_exact(visits).map_err(|_| invalid("remap_allocation"))?;
        for item in &mut self.items {
            mask(&mut slots, &mut item.item);
            if let Some(parent) = &mut item.parent {
                mask(&mut slots, parent);
            }
            if let Some(state) = &mut item.mask {
                mask(&mut slots, &mut state.owner);
            }
        }
        for source in &mut self.source_items {
            mask(&mut slots, &mut source.owner);
        }
        for stack in &mut self.filter_stacks {
            mask(&mut slots, &mut stack.owner);
            for node in &mut stack.nodes {
                if let Some(target) = &mut node.mask {
                    mask(&mut slots, target);
                }
                for (_, target) in &mut node.auxiliary {
                    mask(&mut slots, target);
                }
            }
        }
        for target in &mut self.selection.selected {
            mask(&mut slots, target);
        }
        for target in &mut self.selection.selected_paths {
            mask(&mut slots, target);
        }
        if let Some(target) = &mut self.selection.active {
            mask(&mut slots, target);
        }
        if let Some(target) = &mut self.selection.mask_target {
            mask(&mut slots, target);
        }
        for set in &mut self.item_sets {
            for target in &mut set.members {
                mask(&mut slots, target);
            }
        }
        for record in &mut self.opaque {
            if let Some(owner) = &mut record.owner {
                mask(&mut slots, owner);
            }
        }
        if let Some(attachment) = &mut self.floating {
            mask(&mut slots, &mut attachment.target);
        }
        for filter_mask in &mut self.filter_masks {
            slots.push(Slot::Mask(&mut filter_mask.id));
        }
        publish(slots, &plan)
    }

    /// Remap current FilterNode identities and FilterMask ownership, not graph/source IDs.
    pub fn remap_filters(&mut self, mut map: impl FnMut(FilterId) -> Option<FilterId>) -> Result<()> {
        let visits = bounds(self)?;
        mask_payload_identities(self)?;
        let mut ids = sources(visits.min(MAX_IDENTITIES))?;
        for stack in &self.filter_stacks {
            for node in &stack.nodes {
                if ids.len() >= MAX_IDENTITIES {
                    return Err(invalid("item_limit"));
                }
                ids.push(node.id.0);
            }
        }
        let plan = Plan::new(ids, |id| map(FilterId(id)).map(|target| target.0))?;
        let mut slots = Vec::new();
        slots.try_reserve_exact(visits).map_err(|_| invalid("remap_allocation"))?;
        for stack in &mut self.filter_stacks {
            for node in &mut stack.nodes {
                slots.push(Slot::Filter(&mut node.id));
            }
        }
        for filter_mask in &mut self.filter_masks {
            slots.push(Slot::Filter(&mut filter_mask.owner));
        }
        publish(slots, &plan)
    }

    /// Remap current set identities. Set members retain their separately typed item IDs.
    /// The model has no stored ItemSetId selector elsewhere; no name-based link is inferred.
    pub fn remap_item_sets(&mut self, mut map: impl FnMut(ItemSetId) -> Option<ItemSetId>) -> Result<()> {
        let visits = bounds(self)?;
        let mut ids = sources(self.item_sets.len())?;
        ids.extend(self.item_sets.iter().map(|set| set.id.0));
        let plan = Plan::new(ids, |id| map(ItemSetId(id)).map(|target| target.0))?;
        let mut slots = Vec::new();
        slots.try_reserve_exact(visits).map_err(|_| invalid("remap_allocation"))?;
        for set in &mut self.item_sets {
            slots.push(Slot::Set(&mut set.id));
        }
        publish(slots, &plan)
    }
}
