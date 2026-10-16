//! Paid discovery of canonical metadata and stable filter IDs for genuine
//! readonly Alpha/Histogram/Point storage. No source, render or editing grant follows.
//! The returned carrier MUST remain held through encoding and transport output.
use crate::{
    EngineError, Result, Session,
    accounted_filter_inspect::{OwnedFilterInspectReply, ReplyBounds, ReplyKind, paid_reply},
};
use photocraft_raster::backing::{BackingError, ReadControl};
use serde_json::Value;
const COMMAND: &str = "xcf.inspect";
fn bad(message: &'static str) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}
pub(crate) fn pagination(params: &Value, control: &ReadControl<'_>) -> Result<(usize, usize)> {
    control.check(4)?;
    // Registered resident dispatch treats null as {}, before its old parser.
    let fields = if params.is_null() {
        None
    } else {
        Some(
            params
                .as_object()
                .filter(|v| v.len() <= 2 && v.keys().all(|key| matches!(key.as_str(), "offset" | "limit")))
                .ok_or_else(|| bad("expected offset and limit only"))?,
        )
    };
    let integer = |key: &str, default: u64, max: u64| -> Result<usize> {
        let value = match fields.and_then(|v| v.get(key)) {
            None => default,
            Some(value) => value.as_u64().ok_or_else(|| bad("pagination must be unsigned integer"))?,
        };
        if value > max {
            return Err(bad("pagination exceeds inspection bound"));
        }
        usize::try_from(value).map_err(|_| bad("pagination overflow"))
    };
    let offset = integer("offset", 0, 100000)?;
    let limit = integer("limit", 64, 128)?;
    if limit == 0 {
        return Err(bad("limit must be positive"));
    }
    Ok((offset, limit))
}
fn count(length: usize, offset: usize, limit: usize) -> usize {
    length.saturating_sub(offset).min(limit)
}
fn add(total: &mut usize, n: usize) -> Result<()> {
    *total = total.checked_add(n).ok_or(BackingError::Length)?;
    Ok(())
}
// A str is valid UTF8; counting non-continuation bytes gives its code-point
// count even when a bounded byte chunk ends inside a multi-byte character.
pub(crate) fn over_256(text: &str, control: &ReadControl<'_>) -> Result<bool> {
    let mut characters = 0usize;
    for chunk in text.as_bytes().chunks(32 * 1024) {
        control.check(chunk.len() as u64 + 1)?;
        characters = characters.checked_add(chunk.iter().filter(|byte| **byte & 0xc0 != 0x80).count()).ok_or(BackingError::Length)?;
    }
    Ok(characters > 256)
}
pub(crate) fn validate_resolution(resolution: &photocraft_doc::PhysicalResolution, control: &ReadControl<'_>) -> Result<()> {
    control.check(1)?;
    if !resolution.x_dpi.is_finite() || !resolution.y_dpi.is_finite() || resolution.x_dpi <= 0. || resolution.y_dpi <= 0. {
        return Err(bad("invalid physical resolution"));
    }
    // Same public PhysicalResolution::validate conditions, without its unpolled
    // whole-string contains() scans or a CustomUnit clone.
    if let photocraft_doc::PhysicalUnit::Custom(unit) = &resolution.unit {
        if !unit.factor.is_finite() || unit.factor <= 0. || unit.name.is_empty() {
            return Err(bad("invalid physical resolution"));
        }
        for text in [&unit.name, &unit.symbol, &unit.abbreviation] {
            if text.len() > 1 << 20 {
                return Err(bad("invalid physical resolution"));
            }
            for chunk in text.as_bytes().chunks(32 * 1024) {
                control.check(chunk.len() as u64 + 1)?;
                if chunk.contains(&0) {
                    return Err(bad("invalid physical resolution"));
                }
            }
        }
    }
    Ok(())
}
/// This is a storage metadata query, not authorization. Typed frontends apply
/// their existing command/workspace authorizers to the exact original request.
/// An immutable Session borrow prevents intervening publication; installed model,
/// source, view, history, redo, gesture state and journal remain unchanged.
pub fn prepare(session: &Session, params: &Value, control: &ReadControl<'_>) -> Result<OwnedFilterInspectReply> {
    let (offset, limit) = pagination(params, control)?;
    let owner = session.active().ok_or(EngineError::NoDocument)?.doc.accounted().ok_or(BackingError::Unsupported)?;
    if owner.is_readonly_filtered_flat_point_storage() || owner.is_readonly_masked_filtered_flat_point_storage() {
        return filtered_flat::prepare(owner,offset,limit,control);
    } else if owner.is_readonly_flat_composite_storage() {
        return crate::accounted_plain_xcf_inspect::prepare_flat_composite(owner,offset,limit,control);
    } else if owner.is_readonly_threshold_alpha_storage() {
        owner.validate_readonly_threshold_alpha_storage(control)?;
    } else if owner.is_readonly_threshold_storage() {
        owner.validate_readonly_threshold_storage(control)?;
    } else if owner.is_readonly_gaussian_group_storage() {
        owner.validate_gaussian_group_storage(control)?;
    } else if owner.is_readonly_assigned_point_storage() {
        owner.validate_readonly_assigned_point_storage(control)?;
    } else if owner.is_readonly_point_storage() {
        owner.validate_readonly_point_storage(control)?;
    } else {
        return Err(BackingError::Unsupported.into());
    }
    let doc = owner.document();
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let point = owner.is_readonly_point_storage() || owner.is_readonly_assigned_point_storage();
    let gaussian_group=owner.is_readonly_gaussian_group_storage();
    // Actual class/census already enforces these; repeat the small shape before
    // deriving response counts so future class expansion cannot widen this API.
    // An empty CURRENT Point retains its stack/owner/revision, with no masks
    // and exactly one layer item. This is metadata, not a source/render grant.
    let filters_in_owner = if gaussian_group {
        if state.items.len()!=6 || state.filter_masks.len()!=1 || state.filter_stacks.len()!=1
            || state.filter_stacks.first().is_none_or(|s|s.nodes.len()!=1) {return Err(BackingError::Unsupported.into())}
        1
    } else if point {
        let [stack] = state.filter_stacks.as_slice() else { return Err(BackingError::Unsupported.into()) };
        if stack.nodes.len() > 2
            || state.filter_masks.len() > stack.nodes.len()
            || state.items.len() != state.filter_masks.len().checked_add(1).ok_or(BackingError::Length)?
        {
            return Err(BackingError::Unsupported.into());
        }
        stack.nodes.len()
    } else {
        if state.items.len() > 2 || state.filter_stacks.len() != 1 || state.filter_stacks.first().is_none_or(|s| s.nodes.len() != 1) {
            return Err(BackingError::Unsupported.into());
        }
        1
    };
    if !doc.paths.is_empty()
        || !state.source_items.is_empty()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
        || state.selection.selected.len() > 1
        || !state.selection.selected_paths.is_empty()
    {
        return Err(BackingError::Unsupported.into());
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
        // Point has at most one layer plus two optional matched masks. The
        // other readonly classes retain their original two-item shape above.
        // Six fixed slots admit only the explicit checked Gaussian-group branch;
        // old class shapes and their computed bounds are unchanged.
        let mut facts = [None, None, None, None, None, None];
        for (index, item) in state.items.iter().enumerate() {
            let fact = crate::xcf_inspect_cmds::ItemTextFacts {
                item,
                name_truncated: over_256(&item.name, control)?,
                epoch_truncated: item.composition.as_ref().map(|p| over_256(&p.compatibility_epoch, control)).transpose()?.unwrap_or(false),
            };
            *facts.get_mut(index).ok_or(BackingError::Length)? = Some(fact);
        }
        let facts = facts.get(..state.items.len()).ok_or(BackingError::Length)?;
        crate::xcf_inspect_cmds::build_inspect_reply(doc, state, resolution, offset, limit, Some(facts))
    })
}
#[cfg(test)]
#[path = "accounted_xcf_inspect_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus"))]
#[path = "accounted_point_stack_inspect_tests.rs"]
mod point_stack_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_empty_point_inspect_tests.rs"]
mod empty_point_tests;

#[path="accounted_xcf_inspect_filtered_flat.rs"]
mod filtered_flat;
