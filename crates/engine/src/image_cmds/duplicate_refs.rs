//! Bounded current-layer identity planning for Image Duplicate.
use crate::{EngineError, Result};
use photocraft_doc::{Document, LayerContent, LayerId, MAX_GROUP_DEPTH};
use std::collections::{HashMap, HashSet};
fn poll(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
const MAX_LAYERS: usize = 10_000;
const MAX_REFERENCES: usize = 100_000;
fn error(reason: &str) -> EngineError {
    EngineError::Other(format!("image.duplicate: {reason}"))
}
fn visit(total: &mut usize, count: usize) -> Result<()> {
    *total = total.checked_add(count).ok_or_else(|| error("reference limit"))?;
    if *total > MAX_REFERENCES {
        return Err(error("reference limit"));
    }
    Ok(())
}
fn typed_visits(doc: &Document, total: &mut usize, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let Some(state) = &doc.xcf_state else { return Ok(()) };
    // Length checks precede each traversal, including repeat references.
    visit(total, state.items.len())?;
    for item in &state.items {
        poll(control)?;
        visit(total, usize::from(item.parent.is_some()) + usize::from(item.mask.is_some()))?;
    }
    visit(total, state.editable_indexed.len())?;
    visit(total, state.source_items.len())?;
    visit(total, state.filter_stacks.len())?;
    for stack in &state.filter_stacks {
        poll(control)?;
        visit(total, stack.nodes.len())?;
        for node in &stack.nodes {
            poll(control)?;
            visit(total, node.auxiliary.len())?;
            visit(total, usize::from(node.mask.is_some()))?;
        }
    }
    visit(total, state.selection.selected.len())?;
    visit(total, state.selection.selected_paths.len())?;
    visit(total, usize::from(state.selection.active.is_some()) + usize::from(state.selection.mask_target.is_some()))?;
    visit(total, state.item_sets.len())?;
    for set in &state.item_sets {
        poll(control)?;
        visit(total, set.members.len())?;
    }
    visit(total, state.opaque.len())?;
    visit(total, if state.floating.is_some() { 2 } else { 0 })?;
    Ok(())
}
pub(super) fn has_associations_with_interrupt(doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<bool> {
    poll(control)?;
    if doc.layer_comps.len() > MAX_REFERENCES || doc.slices.list.len() > MAX_REFERENCES || doc.variables.defs.len() > MAX_REFERENCES {
        return Err(error("reference container limit"));
    }
    let mut references = 0usize;
    for comp in doc.layer_comps.iter().chain(doc.last_document_state.iter()) {
        poll(control)?;
        visit(&mut references, comp.states.len())?;
    }
    if !doc.variables.defs.is_empty() {
        return Ok(true);
    }
    for slice in &doc.slices.list {
        poll(control)?;
        if slice.layer.is_some() {
            return Ok(true);
        }
    }
    poll(control)?;
    Ok(false)
}
pub(super) struct Plan {
    ids: HashMap<LayerId, LayerId>,
    missing_comp_ids: HashSet<LayerId>,
    count: usize,
}
impl Plan {
    pub(super) fn new_with_interrupt(doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        Self::with_fresh_controlled(doc, LayerId::fresh, control)
    }
    #[cfg(test)]
    fn with_fresh(doc: &Document, fresh: impl FnMut() -> LayerId) -> Result<Self> {
        Self::with_fresh_controlled(doc, fresh, &photocraft_raster::Interrupt::NONE)
    }
    fn with_fresh_controlled(doc: &Document, mut fresh: impl FnMut() -> LayerId, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        poll(control)?;
        let mut live = HashSet::new();
        live.try_reserve(MAX_LAYERS).map_err(|_| error("layer identity allocation"))?;
        if live.capacity() > MAX_LAYERS * 4 {
            return Err(error("layer identity capacity"));
        }
        poll(control)?;
        let mut pending = Vec::new();
        pending.try_reserve_exact(MAX_LAYERS).map_err(|_| error("layer census allocation"))?;
        if pending.capacity() > MAX_LAYERS * 4 {
            return Err(error("layer census capacity"));
        }
        poll(control)?;
        if doc.layers.len() > MAX_LAYERS {
            return Err(error("layer limit"));
        }
        pending.extend(doc.layers.iter().map(|l| (l, 0usize)));
        while let Some((layer, depth)) = pending.pop() {
            poll(control)?;
            if depth > MAX_GROUP_DEPTH || live.len() >= MAX_LAYERS {
                return Err(error("layer/depth limit"));
            }
            if layer.id.0 == 0 || !live.insert(layer.id) {
                return Err(error("zero or duplicate layer identity"));
            }
            if let LayerContent::Group(group) = &layer.content {
                if group.children.len() > MAX_LAYERS.saturating_sub(live.len()).saturating_sub(pending.len()) {
                    return Err(error("layer limit"));
                }
                pending.extend(group.children.iter().map(|l| (l, depth + 1)));
            }
        }
        if doc.slices.list.len() > MAX_REFERENCES || doc.layer_comps.len() > MAX_REFERENCES || doc.variables.defs.len() > MAX_REFERENCES {
            return Err(error("reference container limit"));
        }
        // Missing comp records are retained warnings, not live layer identities.
        // Reserve them too, so fresh copies cannot accidentally satisfy a stale record.
        let mut missing_comp_ids = HashSet::new();
        missing_comp_ids.try_reserve(MAX_REFERENCES).map_err(|_| error("missing comp identity allocation"))?;
        if missing_comp_ids.capacity() > MAX_REFERENCES * 4 {
            return Err(error("missing comp identity capacity"));
        }
        let mut visits = 0usize;
        let mut check = |id: LayerId, allow_missing_comp: bool| -> Result<()> {
            poll(control)?;
            visits = visits.checked_add(1).ok_or_else(|| error("reference limit"))?;
            if visits > MAX_REFERENCES {
                return Err(error("reference limit"));
            }
            if !live.contains(&id) {
                if allow_missing_comp {
                    missing_comp_ids.insert(id);
                } else {
                    return Err(error("unsupported unresolved current layer association"));
                }
            }
            Ok(())
        };
        for comp in doc.layer_comps.iter().chain(doc.last_document_state.iter()) {
            poll(control)?;
            for state in &comp.states {
                poll(control)?;
                check(state.layer, true)?;
            }
        }
        for v in &doc.variables.defs {
            poll(control)?;
            check(v.layer, false)?;
        }
        for slice in &doc.slices.list {
            poll(control)?;
            if let Some(id) = slice.layer {
                check(id, false)?;
            }
        }
        typed_visits(doc, &mut visits, control)?;
        doc.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(&e.to_string()) })?;
        let mut ids = HashMap::new();
        ids.try_reserve(live.len()).map_err(|_| error("layer map allocation"))?;
        if ids.capacity() > MAX_LAYERS * 4 {
            return Err(error("layer map capacity"));
        }
        poll(control)?;
        let mut assigned = HashSet::new();
        assigned.try_reserve(live.len()).map_err(|_| error("fresh identity allocation"))?;
        if assigned.capacity() > MAX_LAYERS * 4 {
            return Err(error("fresh identity capacity"));
        }
        poll(control)?;
        let mut attempts = 0usize;
        for old in &live {
            poll(control)?;
            let next = loop {
                poll(control)?;
                if attempts >= MAX_REFERENCES {
                    return Err(error("fresh layer identity exhaustion"));
                }
                attempts += 1;
                let id = fresh();
                if id.0 != 0 && !live.contains(&id) && !missing_comp_ids.contains(&id) && assigned.insert(id) {
                    break id;
                }
            };
            ids.insert(*old, next);
        }
        poll(control)?;
        Ok(Self { ids, missing_comp_ids, count: live.len() })
    }
    #[cfg(test)]
    pub(super) fn apply(self, doc: &mut Document) -> Result<()> {
        self.apply_with_interrupt(doc, &photocraft_raster::Interrupt::NONE)
    }
    pub(super) fn apply_with_interrupt(self, doc: &mut Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        poll(control)?;
        let mut pending = Vec::new();
        pending.try_reserve_exact(self.count).map_err(|_| error("layer remap allocation"))?;
        if pending.capacity() > MAX_LAYERS * 4 {
            return Err(error("layer remap capacity"));
        }
        poll(control)?;
        pending.extend(doc.layers.iter_mut());
        while let Some(layer) = pending.pop() {
            poll(control)?;
            layer.id = *self.ids.get(&layer.id).ok_or_else(|| error("missing layer identity mapping"))?;
            if let LayerContent::Group(group) = &mut layer.content {
                pending.extend(group.children.iter_mut());
            }
        }
        if let Some(state) = &mut doc.xcf_state {
            poll(control)?;
            state
                .remap_layers_with_interrupt(|id| self.ids.get(&id).copied(), control)
                .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(&e.to_string()) })?;
            poll(control)?;
        }
        let map = |id: &mut LayerId| -> Result<()> {
            poll(control)?;
            *id = *self.ids.get(id).ok_or_else(|| error("missing current association mapping"))?;
            Ok(())
        };
        for comp in doc.layer_comps.iter_mut().chain(doc.last_document_state.iter_mut()) {
            poll(control)?;
            for state in &mut comp.states {
                poll(control)?;
                if let Some(mapped) = self.ids.get(&state.layer) {
                    state.layer = *mapped;
                } else if !self.missing_comp_ids.contains(&state.layer) {
                    return Err(error("missing comp association mapping"));
                }
            }
        }
        for v in &mut doc.variables.defs {
            poll(control)?;
            map(&mut v.layer)?;
        }
        for slice in &mut doc.slices.list {
            poll(control)?;
            if let Some(id) = &mut slice.layer {
                map(id)?;
            }
        }
        doc.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(&e.to_string()) })?;
        poll(control)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, SampleType, Size};
    fn doc() -> Document {
        let mut d = Document::new("Own identity", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mut l = Layer::raster("Own layer", d.pixel_format());
        l.id = LayerId(9);
        d.layers.push(l);
        d
    }
    #[test]
    fn fresh_allocator_skips_zero_source_and_destination_collisions() {
        let mut d = doc();
        let mut second = d.layers[0].clone();
        second.id = LayerId(10);
        d.layers.push(second);
        let mut values = [0, 9, 10, 11, 11, 12].into_iter();
        let plan = Plan::with_fresh(&d, || LayerId(values.next().unwrap())).unwrap();
        plan.apply(&mut d).unwrap();
        let mut ids = d.layers.iter().map(|l| l.id.0).collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, vec![11, 12]);
    }
    #[test]
    fn missing_comp_identity_cannot_be_allocated_to_a_live_copy() {
        let mut d = doc();
        d.layer_comps.push(photocraft_doc::LayerComp {
            id: 1,
            name: "Missing warning".into(),
            comment: String::new(),
            apply_visibility: true,
            apply_position: false,
            apply_appearance: false,
            states: vec![photocraft_doc::CompLayerState { layer: LayerId(99), visible: Some(false), position: None, appearance: None }],
        });
        let before = d.clone();
        let mut candidates = [99, 9, 100].into_iter();
        let plan = Plan::with_fresh(&d, || LayerId(candidates.next().unwrap())).unwrap();
        plan.apply(&mut d).unwrap();
        assert_eq!(d.layers[0].id, LayerId(100));
        assert_eq!(d.layer_comps, before.layer_comps);
        assert_eq!(d.layer_comps[0].missing_layers(&d), vec![LayerId(99)]);
    }
    #[test]
    fn exhausted_allocator_and_invalid_identity_leave_source_exact() {
        let d = doc();
        let before = d.clone();
        assert!(Plan::with_fresh(&d, || LayerId(9)).is_err());
        assert_eq!(d, before);
        for id in [0, 9] {
            let mut bad = d.clone();
            let mut l = bad.layers[0].clone();
            l.id = LayerId(id);
            bad.layers.push(l);
            assert!(Plan::with_fresh(&bad, || LayerId(99)).is_err());
        }
    }
    #[test]
    fn layer_depth_and_reference_limits_are_checked_before_id_allocation() {
        let mut d = doc();
        for _ in 0..MAX_GROUP_DEPTH + 1 {
            d.layers = vec![Layer::group("Own group", std::mem::take(&mut d.layers))];
        }
        assert!(Plan::with_fresh(&d, || panic!("must not allocate IDs")).is_err());
        let mut d = doc();
        d.layers.resize(MAX_LAYERS + 1, d.layers[0].clone());
        assert!(Plan::with_fresh(&d, || panic!("must not allocate IDs")).is_err());
        let mut d = doc();
        d.variables.defs.resize(
            MAX_REFERENCES + 1,
            photocraft_doc::variables::VariableDef { name: "Own".into(), layer: LayerId(9), kind: photocraft_doc::variables::VarKind::Visibility },
        );
        assert!(Plan::with_fresh(&d, || panic!("must not allocate IDs")).is_err());
    }
    #[test]
    fn typed_repeated_and_combined_reference_work_is_bounded_before_validation() {
        let mut d = doc();
        let mut state = photocraft_doc::XcfState::default();
        state.selection.selected = vec![photocraft_doc::item_identity::ItemRef::Layer(LayerId(9)); MAX_REFERENCES + 1];
        d.xcf_state = Some(state);
        let error = Plan::with_fresh(&d, || panic!("must not allocate IDs")).err().unwrap();
        assert!(error.to_string().contains("reference limit"));
        d.xcf_state.as_mut().unwrap().selection.selected.truncate(50_000);
        d.variables.defs.resize(
            50_001,
            photocraft_doc::variables::VariableDef { name: "Own".into(), layer: LayerId(9), kind: photocraft_doc::variables::VarKind::Visibility },
        );
        let error = Plan::with_fresh(&d, || panic!("must not allocate IDs")).err().unwrap();
        assert!(error.to_string().contains("reference limit"));
    }
}
