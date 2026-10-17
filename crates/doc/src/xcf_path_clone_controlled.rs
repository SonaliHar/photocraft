//! Selective path-owned staging with a reversible append journal. The existing
//! controlled reconciliation supplies the sole complete canonical state stage.
use super::{Result, channel_clone as copy, controlled, invalid};
use crate::{Document, XcfState, item_identity::*, xcf_provenance::OpaqueRecord, xcf_state::ItemState};
use photocraft_raster::Interrupt;
use std::{collections::HashSet, mem::size_of, sync::Arc};
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| invalid("cancelled"))
}

pub(super) fn clone_path(state: &mut XcfState, source: PathId, target: PathId, doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    // This is a metadata operation, not a source/cache regeneration. It retains
    // the legacy reconciliation of unrelated deleted/renamed current items.
    controlled::admit(state, doc, control)?;
    let from = ItemRef::Path(source);
    let to = ItemRef::Path(target);
    if source == target || source.0 == 0 || target.0 == 0 {
        return Err(invalid("clone_path_id"));
    }
    let mut found_source = false;
    let mut found_target = false;
    for path in &doc.paths {
        poll(control)?;
        found_source |= path.id == source;
        found_target |= path.id == target;
    }
    if !found_source || !found_target {
        return Err(invalid("clone_path_id"));
    }
    let mut original = None;
    for value in &state.items {
        poll(control)?;
        if value.item == to {
            return Err(invalid("clone_path_id"));
        }
        if value.item == from && original.is_none() {
            original = Some(value);
        }
    }
    let mut extra = 0usize;
    let mut logical = 0u64;
    for value in &state.opaque {
        poll(control)?;
        logical = logical.checked_add(value.bytes.len() as u64).ok_or_else(|| invalid("opaque_limit"))?;
        if value.owner == Some(from) {
            extra = extra.checked_add(1).ok_or_else(|| invalid("opaque_limit"))?;
            logical = logical.checked_add(value.bytes.len() as u64).ok_or_else(|| invalid("opaque_limit"))?;
        }
    }
    let item_count = state.items.len().checked_add(usize::from(original.is_some())).filter(|n| *n <= 10_000).ok_or_else(|| invalid("item_limit"))?;
    let opaque_count = state.opaque.len().checked_add(extra).filter(|n| *n <= 100_000).ok_or_else(|| invalid("opaque_limit"))?;
    if logical > 256 << 20 {
        return Err(invalid("opaque_limit"));
    }
    let mut seen = HashSet::new();
    seen.try_reserve(extra).map_err(|_| invalid("clone_path_limit"))?;
    let index = (seen.capacity() as u64).checked_mul(48).filter(|n| *n <= 16 << 20).ok_or_else(|| invalid("clone_path_limit"))?;
    let mut census = copy::Census { control, model: index, payload: 0, work: 0, visits: 0, seen };
    if let Some(original) = original {
        census.item(original)?;
    }
    for value in &state.opaque {
        poll(control)?;
        if value.owner == Some(from) {
            census.opaque(value)?;
        }
    }
    let replacements = (item_count as u64)
        .checked_mul(size_of::<ItemState>() as u64)
        .and_then(|n| n.checked_add((opaque_count as u64).checked_mul(size_of::<OpaqueRecord>() as u64)?))
        .ok_or_else(|| invalid("clone_path_limit"))?;
    // Replacement buffers, pending records and the later reconciler's copied
    // records are charged before any selected typed clone. Payload stays shared.
    census.bytes(replacements.checked_mul(3).ok_or_else(|| invalid("clone_path_limit"))?)?;
    if census.model.checked_add(census.payload).is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES) || census.work > 8_000_000_000 {
        return Err(invalid("clone_path_limit"));
    }
    // One combined headroom admission, not two independent maximum envelopes.
    // Selected payload is already included in the baseline shared-Arc census.
    controlled::admit_extra(state, doc, control, census.model, census.work)?;
    let mut pending_items = copy::reserved(usize::from(original.is_some()))?;
    let mut pending_opaque = copy::reserved(extra)?;
    let mut replacement_items = copy::reserved(item_count)?;
    let mut replacement_opaque = copy::reserved(opaque_count)?;
    if let Some(original) = original {
        let mut value = copy::item(original, control)?;
        value.item = to;
        pending_items.push(value);
    }
    for value in &state.opaque {
        poll(control)?;
        if value.owner == Some(from) {
            pending_opaque.push(OpaqueRecord {
                owner: Some(to),
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
    }
    poll(control)?;
    let original_items = state.items.len();
    let original_opaque = state.opaque.len();
    let mut item_allocation = std::mem::take(&mut state.items);
    let mut opaque_allocation = std::mem::take(&mut state.opaque);
    replacement_items.append(&mut item_allocation);
    replacement_items.append(&mut pending_items);
    replacement_opaque.append(&mut opaque_allocation);
    replacement_opaque.append(&mut pending_opaque);
    state.items = replacement_items;
    state.opaque = replacement_opaque;
    // Reconciliation itself is private and atomic. A failed/cancelled attempt
    // leaves these append buffers intact, permitting allocation-free rollback
    // into the exact original capacities. No cancellation is polled mid-rollback.
    match state.reconcile_layers_with_interrupt(doc, control) {
        Ok(()) => Ok(()),
        Err(error) => {
            state.items.truncate(original_items);
            state.opaque.truncate(original_opaque);
            item_allocation.append(&mut state.items);
            opaque_allocation.append(&mut state.opaque);
            state.items = item_allocation;
            state.opaque = opaque_allocation;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorMode, NamedPath, Path, SampleType, Size};
    fn fixture() -> (Document, XcfState, PathId, PathId) {
        let mut doc = Document::new("Own path clone", Size::new(2, 2), ColorMode::Rgb, SampleType::F32);
        let source = NamedPath::new("Source path", Path::default());
        let from = source.id;
        doc.paths.push(source);
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        let metadata = state.items.iter_mut().find(|item| item.item == ItemRef::Path(from)).unwrap();
        metadata.source_tattoo = Some(42);
        metadata.source_flags = 17;
        state.selection.selected_paths.push(ItemRef::Path(from));
        let bytes = Arc::new(vec![0, 255, 2]);
        state.opaque.push(OpaqueRecord {
            owner: Some(ItemRef::Path(from)),
            namespace: "own".into(),
            identity: "path data".into(),
            source_version: 1,
            order: 2,
            flags: 7,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
        let target = NamedPath::new("Target current name", Path::default());
        let to = target.id;
        doc.paths.push(target);
        (doc, state, from, to)
    }
    #[test]
    fn controlled_path_clone_matches_legacy_and_retains_global_source_and_selection() {
        let (doc, state, from, to) = fixture();
        let mut expected = state.clone();
        expected.clone_path_metadata(from, to, &doc).unwrap();
        let mut actual = state.clone();
        actual.clone_path_metadata_with_interrupt(from, to, &doc, &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.selection, state.selection);
        assert_eq!(actual.source_items, state.source_items);
        assert_eq!(actual.provenance, state.provenance);
        assert!(Arc::ptr_eq(&actual.opaque[0].bytes, &actual.opaque[1].bytes));
        let target = actual.items.iter().find(|item| item.item == ItemRef::Path(to)).unwrap();
        assert_eq!(target.name, "Target current name");
        assert_eq!(target.source_tattoo, Some(42));
        assert_eq!(target.source_flags, 17);
    }
    #[test]
    fn cancellation_after_append_and_invalid_target_restore_exact_allocations() {
        let (doc, mut state, from, to) = fixture();
        let before = state.clone();
        let capacities = (state.items.capacity(), state.opaque.capacity());
        // Ordinarily cloning a selected opaque Arc adds one owner. The third
        // owner appears only in reconciliation's private canonical stage.
        let bytes = Arc::clone(&state.opaque[0].bytes);
        let initial = Arc::strong_count(&bytes);
        let cancelled = || Arc::strong_count(&bytes) > initial + 1;
        assert_eq!(state.clone_path_metadata_with_interrupt(from, to, &doc, &Interrupt::cancel_only(&cancelled)).unwrap_err().code, "cancelled");
        assert_eq!(state, before);
        assert_eq!((state.items.capacity(), state.opaque.capacity()), capacities);
        assert!(state.clone_path_metadata_with_interrupt(from, from, &doc, &Interrupt::NONE).is_err());
        assert_eq!(state, before);
    }
}
