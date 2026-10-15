//! Paid paginated CURRENT metadata over genuine accounted plain XCF storage.
//! Provenance/resources remain inert; no original lineage, renderer, profile,
//! edit permission, locator resolution or payload access follows from inspection.
use crate::{
    EngineError, Result, Session,
    accounted_filter_inspect::{OwnedFilterInspectReply, ReplyBounds, ReplyKind, paid_reply},
    accounted_xcf_inspect::{over_256, pagination, validate_resolution},
    xcf_inspect_cmds::{ItemTextFacts, SetTextFacts},
};
use photocraft_doc::{XcfState, xcf_editing::ItemSetRule};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use serde_json::Value;
const COMMAND: &str = "xcf.inspect";
fn bad(message: &'static str) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}
fn add(n: &mut usize, value: usize) -> Result<()> {
    *n = n.checked_add(value).ok_or(BackingError::Length)?;
    Ok(())
}
fn rows(length: usize, offset: usize, limit: usize) -> usize {
    length.saturating_sub(offset).min(limit)
}
fn expression(rule: &ItemSetRule) -> Option<&str> {
    match rule {
        ItemSetRule::Named => None,
        ItemSetRule::Text(s) | ItemSetRule::Regex(s) | ItemSetRule::Glob(s) => Some(s),
    }
}
struct Plan {
    bounds: ReplyBounds,
    bytes: u64,
    scanned: usize,
    items: usize,
    sets: usize,
}
fn plan(state: &XcfState, resolution: &photocraft_doc::PhysicalResolution, offset: usize, limit: usize, control: &ReadControl<'_>) -> Result<Plan> {
    control.check(1)?;
    // The genuine plain gate already enforces these. Keep this query's row
    // vocabulary explicit even if that unrelated model class expands later.
    if state.items.len() > 4096
        || state.resources.len() > 4096
        || state.item_sets.len() > 4096
        || state.selection.selected.len() > 4096
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !state.selection.selected_paths.is_empty()
    {
        return Err(BackingError::Unsupported.into());
    }
    let items = rows(state.items.len(), offset, limit);
    let resources = rows(state.resources.len(), offset, limit);
    let sets = rows(state.item_sets.len(), offset, limit);
    let selected = rows(state.selection.selected.len(), offset, limit);
    let mut members = 0usize;
    let mut scanned = state.provenance.container.len();
    let mut hash_bytes = 0usize;
    let mut max_string = 4096usize;
    if let photocraft_doc::PhysicalUnit::Custom(unit) = &resolution.unit {
        for text in [&unit.name, &unit.symbol, &unit.abbreviation] {
            add(&mut scanned, text.len())?;
        }
    }
    for item in state.items.iter().skip(offset).take(limit) {
        control.check(1)?;
        add(&mut scanned, item.name.len())?;
        if let Some(policy) = &item.composition {
            add(&mut scanned, policy.compatibility_epoch.len())?;
        }
    }
    for resource in state.resources.iter().skip(offset).take(limit) {
        control.check(1)?;
        add(&mut scanned, resource.name.len())?;
        if let Some(hash) = &resource.content_hash {
            if hash.len() > 1 << 20 {
                return Err(BackingError::Length.into());
            }
            add(&mut scanned, hash.len())?;
            add(&mut hash_bytes, hash.len())?;
            max_string = max_string.max(hash.len());
        }
    }
    for set in state.item_sets.iter().skip(offset).take(limit) {
        control.check(1)?;
        add(&mut scanned, set.name.len())?;
        if set.members.len() > 4096 {
            return Err(BackingError::Length.into());
        }
        add(&mut members, rows(set.members.len(), offset, limit))?;
        if let Some(text) = expression(&set.rule) {
            add(&mut scanned, text.len())?;
        }
    }
    // Page objects are in the base. Each item includes composition/locks/bounds
    // and the existing mask object (owner reference, bounds, offset, flags and
    // fixed 18-byte outsideFloat64Bits). The 96/12/64 item envelope also covers
    // the closed MaskItem, which has no composition; no pixel payload is read.
    // each set includes its own page object, with independently paged members.
    // No limit multiplication is guessed: members is the actual sum above.
    let mut nodes = 128usize;
    let mut objects = 24usize;
    let mut fields = 96usize;
    for (count, n, o, m) in [(items, 96, 12, 64), (resources, 32, 3, 24), (sets, 48, 6, 32), (members, 8, 2, 4), (selected, 8, 2, 4)] {
        add(&mut nodes, count.checked_mul(n).ok_or(BackingError::Length)?)?;
        add(&mut objects, count.checked_mul(o).ok_or(BackingError::Length)?)?;
        add(&mut fields, count.checked_mul(m).ok_or(BackingError::Length)?)?;
    }
    let bounds = ReplyBounds { nodes, objects, members: fields, depth: 8, max_string };
    let mut shortened = 1usize;
    add(&mut shortened, items.checked_mul(2).ok_or(BackingError::Length)?)?;
    add(&mut shortened, resources)?;
    add(&mut shortened, sets.checked_mul(2).ok_or(BackingError::Length)?)?;
    let mut text = 8192usize;
    add(&mut text, shortened.checked_mul(1024).ok_or(BackingError::Length)?)?;
    // Every row/reference adds decimal IDs and fixed schema keys/literals.
    add(&mut text, fields.checked_mul(64).ok_or(BackingError::Length)?)?;
    add(&mut text, hash_bytes)?;
    let bytes = nodes
        .checked_mul(std::mem::size_of::<Value>())
        .and_then(|n| n.checked_add(objects.checked_mul(4096)?))
        .and_then(|n| n.checked_add(fields.checked_mul(256)?))
        .and_then(|n| n.checked_add(text.checked_mul(2)?))
        .and_then(|n| n.checked_add(std::mem::size_of::<OwnedFilterInspectReply>() + 128))
        .ok_or(BackingError::Length)?;
    Ok(Plan { bounds, bytes: u64::try_from(bytes).map_err(|_| BackingError::Length)?, scanned, items, sets })
}
// Fact payloads are destroyed before their actual same-ledger reservation.
struct Facts<'a> {
    items: Vec<Option<ItemTextFacts<'a>>>,
    sets: Vec<SetTextFacts<'a>>,
    _lease: QuotaLease,
}
impl<'a> Facts<'a> {
    fn new(state: &'a XcfState, plan: &Plan, offset: usize, limit: usize, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<Self> {
        let item_bytes = plan.items.checked_mul(2).and_then(|n| n.checked_mul(std::mem::size_of::<Option<ItemTextFacts<'a>>>())).ok_or(BackingError::Length)?;
        let set_bytes = plan.sets.checked_mul(2).and_then(|n| n.checked_mul(std::mem::size_of::<SetTextFacts<'a>>())).ok_or(BackingError::Length)?;
        let header = std::mem::size_of::<Self>() + 128;
        let bytes = item_bytes.checked_add(set_bytes).and_then(|n| n.checked_add(header)).ok_or(BackingError::Length)?;
        let lease = quota.reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
        let mut facts = Self { items: Vec::new(), sets: Vec::new(), _lease: lease };
        control.check(bytes as u64 + 1)?;
        facts.items.try_reserve_exact(plan.items).map_err(|_| BackingError::Allocation)?;
        facts.sets.try_reserve_exact(plan.sets).map_err(|_| BackingError::Allocation)?;
        if facts.items.capacity() > 2 * plan.items || facts.sets.capacity() > 2 * plan.sets {
            return Err(BackingError::Allocation.into());
        }
        for item in state.items.iter().skip(offset).take(limit) {
            control.check(1)?;
            facts.items.push(Some(ItemTextFacts {
                item,
                name_truncated: over_256(&item.name, control)?,
                epoch_truncated: item.composition.as_ref().map(|p| over_256(&p.compatibility_epoch, control)).transpose()?.unwrap_or(false),
            }));
        }
        for set in state.item_sets.iter().skip(offset).take(limit) {
            control.check(1)?;
            facts.sets.push(SetTextFacts { set, rule_truncated: expression(&set.rule).map(|text| over_256(text, control)).transpose()?.unwrap_or(false) });
        }
        let actual = facts
            .items
            .capacity()
            .checked_mul(std::mem::size_of::<Option<ItemTextFacts<'a>>>())
            .and_then(|n| n.checked_add(facts.sets.capacity().checked_mul(std::mem::size_of::<SetTextFacts<'a>>())?))
            .and_then(|n| n.checked_add(header))
            .ok_or(BackingError::Length)?;
        facts._lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        Ok(facts)
    }
}
/// Full paginated current plain metadata. Existing authorizers must run first;
/// retain the carrier through output. A generic genuine Legacy owner with
/// coherent current XcfState may pass; original-XCF lineage is never asserted.
pub fn prepare(session: &Session, params: &Value, control: &ReadControl<'_>) -> Result<OwnedFilterInspectReply> {
    let (offset, limit) = pagination(params, control)?;
    let owner = session.active().ok_or(EngineError::NoDocument)?.doc.accounted().ok_or(BackingError::Unsupported)?;
    if crate::accounted_native_open::is_current_analytic_geometry_candidate(owner.document(), control)? {
        // This metadata-only branch requires the actual paid storage gate. A
        // malformed matching geometry never retries the broader plain gate.
        owner.validate_masked_analytic_normal_storage(control)?;
    } else {
        owner.validate_plain_xcf_inspection(control)?;
    }
    let doc = owner.document();
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    if !doc.paths.is_empty() {
        return Err(bad("plain current metadata does not admit paths"));
    }
    let fallback = photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
    let resolution = doc.physical_resolution.as_ref().unwrap_or(&fallback);
    let plan = plan(state, resolution, offset, limit, control)?;
    paid_reply(owner.quota(), ReplyKind::Project, plan.bounds, plan.bytes, plan.scanned, control, || {
        validate_resolution(resolution, control)?;
        let facts = Facts::new(state, &plan, offset, limit, owner.quota(), control)?;
        let value = crate::xcf_inspect_cmds::build_plain_inspect_reply(doc, state, resolution, offset, limit, &facts.items, &facts.sets, control)?;
        // No facts/hash metadata clone survives without its real lease. The
        // returned tree is already covered by paid_reply's primary reservation.
        drop(facts);
        control.check(1)?;
        Ok(value)
    })
}
#[cfg(test)]
#[path = "accounted_plain_xcf_inspect_tests.rs"]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_masked_analytic_inspect_tests.rs"]
mod masked_analytic_tests;
