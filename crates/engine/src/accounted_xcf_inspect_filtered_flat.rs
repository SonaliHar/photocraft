//! Additive paginated metadata for the distinct readonly filtered-flat class.
use super::*;
pub(super) fn prepare(
    owner: &photocraft_doc::accounted_document::AccountedDocument,
    offset: usize,
    limit: usize,
    control: &ReadControl<'_>,
) -> Result<OwnedFilterInspectReply> {
    owner.validate_readonly_filtered_flat_point_storage(control)?;
    let doc = owner.document();
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    if !(2..=4096).contains(&doc.layers.len())
        || state.filter_stacks.len() > 4096
        || state.filter_masks.len() > 4096
        || state.items.len() > 8192
        || state.selection.selected.len() > 4096
        || !state.selection.selected_paths.is_empty()
        || !state.resources.is_empty()
        || !state.source_items.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
    {
        return Err(BackingError::Unsupported.into());
    }
    let mut filters_in_owner = 0usize;
    for stack in &state.filter_stacks {
        control.check(1)?;
        if stack.nodes.len() != 1 {
            return Err(BackingError::Integrity.into());
        }
        filters_in_owner = filters_in_owner.checked_add(1).ok_or(BackingError::Length)?;
    }
    if filters_in_owner == 0 || filters_in_owner != state.filter_masks.len() {
        return Err(BackingError::Integrity.into());
    }
    // Borrow the actual unit strings; effective_resolution() would clone them.
    let fallback = photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
    let resolution = doc.physical_resolution.as_ref().unwrap_or(&fallback);
    let items = count(state.items.len(), offset, limit);
    let stacks = count(state.filter_stacks.len(), offset, limit);
    let filters = count(filters_in_owner, offset, limit);
    let selected = count(state.selection.selected.len(), offset, limit);
    // Full byte lengths are read without traversing text. The shared builder's
    // truncation flags use chars().count(), which still visits the COMPLETE
    // original name/epoch even when only 256 code points are returned. Prepay
    // all such full UTF8 scans below, including skipped records conservatively.
    let mut scanned = state.provenance.container.len();
    if let photocraft_doc::PhysicalUnit::Custom(unit) = &resolution.unit {
        for text in [&unit.name, &unit.symbol, &unit.abbreviation] {
            add(&mut scanned, text.len())?;
        }
    }
    for item in &state.items {
        control.check(1)?;
        add(&mut scanned, item.name.len())?;
        if let Some(policy) = &item.composition {
            add(&mut scanned, policy.compatibility_epoch.len())?;
        }
    }
    for stack in &state.filter_stacks {
        control.check(1)?;
        for node in &stack.nodes {
            control.check(1)?;
            for text in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
                add(&mut scanned, text.len())?;
            }
        }
    }
    // Exact closed response row counts determine conservative structural bounds.
    // Empty page/selection objects are included in the fixed base; item rows
    // include optional composition/MaskState/locks and references. No arbitrary
    // caller tree or source bytes are serialized by this branch.
    let nodes = 128usize
        .checked_add(items.checked_mul(96).ok_or(BackingError::Length)?)
        .and_then(|n| n.checked_add(stacks.checked_mul(16)?))
        .and_then(|n| n.checked_add(filters.checked_mul(32)?))
        .and_then(|n| n.checked_add(selected.checked_mul(8)?))
        .ok_or(BackingError::Length)?;
    let objects = 24usize
        .checked_add(items.checked_mul(12).ok_or(BackingError::Length)?)
        .and_then(|n| n.checked_add(stacks.checked_mul(3)?))
        .and_then(|n| n.checked_add(filters.checked_mul(3)?))
        .and_then(|n| n.checked_add(selected.checked_mul(2)?))
        .ok_or(BackingError::Length)?;
    let members = 96usize
        .checked_add(items.checked_mul(64).ok_or(BackingError::Length)?)
        .and_then(|n| n.checked_add(stacks.checked_mul(8)?))
        .and_then(|n| n.checked_add(filters.checked_mul(16)?))
        .and_then(|n| n.checked_add(selected.checked_mul(4)?))
        .ok_or(BackingError::Length)?;
    let bounds = ReplyBounds { nodes, objects, members, depth: 8, max_string: 4096 };
    // At most 256 UTF8 code points (1024 bytes) for every shortened string;
    // fixed text covers all schema keys, literal values and decimal identities.
    let short_fields = 1usize
        .checked_add(items.checked_mul(2).ok_or(BackingError::Length)?)
        .and_then(|n| n.checked_add(filters.checked_mul(3)?))
        .ok_or(BackingError::Length)?;
    let text = short_fields.checked_mul(1024).and_then(|n| n.checked_add(8192)).ok_or(BackingError::Length)?;
    let planned = nodes
        .checked_mul(std::mem::size_of::<Value>())
        .and_then(|n| n.checked_add(objects.checked_mul(4096)?))
        .and_then(|n| n.checked_add(members.checked_mul(256)?))
        .and_then(|n| n.checked_add(text.checked_mul(2)?))
        .and_then(|n| n.checked_add(std::mem::size_of::<OwnedFilterInspectReply>() + 128))
        .ok_or(BackingError::Length)?;
    let planned = u64::try_from(planned).map_err(|_| BackingError::Length)?;
    paid_reply(owner.quota(), ReplyKind::Project, bounds, planned, scanned, control, || {
        validate_resolution(resolution, control)?;
        let facts_count = count(state.items.len(), offset, limit);
        let fee_bytes = facts_count
            .checked_mul(2)
            .and_then(|n| n.checked_mul(std::mem::size_of::<Option<crate::xcf_inspect_cmds::ItemTextFacts<'_>>>()))
            .and_then(|n| n.checked_add(128))
            .ok_or(BackingError::Length)?;
        let facts_fee = owner.quota().reserve(photocraft_raster::backing::quota::QuotaKind::Index, fee_bytes as u64).map_err(|_| BackingError::Quota)?;
        let mut facts = Vec::new();
        facts.try_reserve_exact(facts_count).map_err(|_| BackingError::Allocation)?;
        if facts.capacity() > facts_count.checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation.into());
        }
        for item in state.items.iter().skip(offset).take(limit) {
            control.check(1)?;
            facts.push(Some(crate::xcf_inspect_cmds::ItemTextFacts {
                item,
                name_truncated: over_256(&item.name, control)?,
                epoch_truncated: item.composition.as_ref().map(|p| over_256(&p.compatibility_epoch, control)).transpose()?.unwrap_or(false),
            }));
        }
        let result = crate::xcf_inspect_cmds::build_inspect_reply(doc, state, resolution, offset, limit, Some(&facts));
        drop(facts);
        drop(facts_fee);
        result
    })
}
