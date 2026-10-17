//! Checked current saved-set targets; never evaluate retained search rules.
use crate::{EngineError, Result};
use photocraft_doc::{
    XcfState,
    item_identity::{ItemRef, ItemSetId},
    xcf_editing::{ItemFamily, ItemSetRule},
};
use std::collections::HashSet;

fn bad(cmd: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
pub(super) fn set_index(state: &XcfState, id: ItemSetId, cmd: &str) -> Result<usize> {
    if id.0 == 0 || state.items.len() > 10_000 || state.item_sets.len() > 10_000 {
        return Err(bad(cmd, "invalid or oversized canonical set target"));
    }
    // Check lengths before iteration or any candidate/snapshot preparation.
    let mut visits = state.items.len().checked_add(state.item_sets.len()).ok_or_else(|| bad(cmd, "saved membership work limit"))?;
    for set in &state.item_sets {
        visits = visits.checked_add(set.members.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad(cmd, "saved membership work limit"))?;
    }
    let mut sets = HashSet::new();
    sets.try_reserve(state.item_sets.len()).map_err(|_| bad(cmd, "set identity allocation limit"))?;
    let mut target = None;
    for (index, set) in state.item_sets.iter().enumerate() {
        if set.id.0 == 0 || !sets.insert(set.id) {
            return Err(bad(cmd, "zero or duplicate canonical set identity"));
        }
        if set.id == id {
            target = Some(index);
        }
    }
    let mut items = HashSet::new();
    items.try_reserve(state.items.len()).map_err(|_| bad(cmd, "item identity allocation limit"))?;
    for item in &state.items {
        if item.item.raw() == 0 || !items.insert(item.item) {
            return Err(bad(cmd, "zero or duplicate canonical item identity"));
        }
    }
    target.ok_or_else(|| bad(cmd, "unknown setId"))
}

pub(super) fn members(state: &XcfState, id: ItemSetId, requested: &[ItemRef], cmd: &str) -> Result<usize> {
    if requested.len() > 10_000 {
        return Err(bad(cmd, "membership limit"));
    }
    let index = set_index(state, id, cmd)?;
    let set = state.item_sets.get(index).ok_or_else(|| bad(cmd, "unknown setId"))?;
    if set.rule != ItemSetRule::Named {
        return Err(bad(cmd, "search rules cannot be replaced with named membership"));
    }
    let mut available = HashSet::new();
    available.try_reserve(state.items.len()).map_err(|_| bad(cmd, "item identity allocation limit"))?;
    available.extend(state.items.iter().map(|item| item.item));
    let mut selected = HashSet::new();
    selected.try_reserve(requested.len()).map_err(|_| bad(cmd, "membership allocation limit"))?;
    for &member in requested {
        if member.raw() == 0 || !selected.insert(member) {
            return Err(bad(cmd, "zero or duplicate member"));
        }
        if !available.contains(&member)
            || !matches!(
                (set.family, member),
                (ItemFamily::Layer, ItemRef::Layer(_)) | (ItemFamily::Channel, ItemRef::Channel(_)) | (ItemFamily::Path, ItemRef::Path(_))
            )
        {
            return Err(bad(cmd, "member is absent or belongs to a different family"));
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::xcf_editing::ItemSet;
    fn set(id: u64) -> ItemSet {
        ItemSet { id: ItemSetId(id), name: "Same retained name".into(), family: ItemFamily::Layer, rule: ItemSetRule::Named, members: Vec::new() }
    }

    #[test]
    fn duplicate_saved_ids_are_not_resolved_by_first_position() {
        let mut state = XcfState { item_sets: vec![set(7), set(7)], ..Default::default() };
        let before = state.clone();
        assert!(set_index(&state, ItemSetId(7), "test").is_err());
        assert_eq!(state, before);
        state.item_sets.get_mut(1).unwrap().id = ItemSetId(8);
        assert_eq!(set_index(&state, ItemSetId(8), "test").unwrap(), 1);
        assert!(set_index(&state, ItemSetId(9), "test").is_err());
    }

    #[test]
    fn missing_typed_members_and_search_rule_replacement_are_refused() {
        let mut state = XcfState::default();
        state.item_sets.push(set(7));
        assert!(members(&state, ItemSetId(7), &[ItemRef::Layer(photocraft_doc::LayerId(3))], "test").is_err());
        assert_eq!(members(&state, ItemSetId(7), &[], "test").unwrap(), 0);
        state.item_sets.get_mut(0).unwrap().rule = ItemSetRule::Text("Original source query".into());
        let before = state.clone();
        assert!(members(&state, ItemSetId(7), &[], "test").is_err());
        assert_eq!(state, before);
    }
}
