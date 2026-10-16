//! Paid metadata-only inspection of actual readonly Threshold and Point storage owners.
//! No source, profile, renderer or editing authority follows. This additive API
//! does not enroll generic Session commands: a typed caller MUST retain the
//! returned carrier through serialization and the complete output operation.
use crate::{EngineError, Result, Session};
use photocraft_doc::xcf_filters::{FilterNode, FilterValue};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use serde_json::Value;
mod wire;
pub use wire::{OwnedFilterInspectWire, encode_cli, encode_rpc, validate_rpc_id};
const COMMAND: &str = "xcf.filter.inspect";
const MAX_TEXT: usize = 4096;
// Closed Alpha/Histogram recipes have <=3 scalar/enum arguments, no auxiliary
// entries, and no recursively serialized values. These finite structural bounds
// cover the complete shared response, including optional composition/identity.
const MAX_NODES: usize = 256;
const MAX_OBJECTS: usize = 32;
const MAX_MEMBERS: usize = 256;
const MAX_DEPTH: usize = 8;
const FIXED_TEXT: usize = 8192;
// Same established serde/BTreeMap logical envelope as accounted journal code;
// allocator nodes have no stable fallible capacity API. This is not an RSS census.
const MAP_HEADER: usize = 4096;
const MAP_ENTRY: usize = 256;

#[derive(Clone, Copy)]
pub(crate) enum ReplyKind {
    Filter,
    Project,
}
#[derive(Clone, Copy)]
pub(crate) struct ReplyBounds {
    pub(crate) nodes: usize,
    pub(crate) objects: usize,
    pub(crate) members: usize,
    pub(crate) depth: usize,
    pub(crate) max_string: usize,
}
const FILTER_BOUNDS: ReplyBounds = ReplyBounds { nodes: MAX_NODES, objects: MAX_OBJECTS, members: MAX_MEMBERS, depth: MAX_DEPTH, max_string: MAX_TEXT };
// Separate Point-only envelope: <=10 FLAT scalar/bool/enum arguments. Existing
// Alpha/Histogram constants and envelope remain unchanged. These cover each
// typed argument's serde enum/state/row maps plus complete original metadata.
const POINT_FILTER_BOUNDS: ReplyBounds = ReplyBounds { nodes: 512, objects: 128, members: 512, depth: 8, max_string: MAX_TEXT };

/// The copied Value drops before its actual quota reservation. There is no raw
/// Value extractor: callers retain this owner until their output is complete.
pub struct OwnedFilterInspectReply {
    value: Value,
    kind: ReplyKind,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl OwnedFilterInspectReply {
    pub fn command(&self) -> &'static str {
        match self.kind {
            ReplyKind::Filter => "xcf.filter.inspect",
            ReplyKind::Project => "xcf.inspect",
        }
    }
    pub fn value(&self) -> &Value {
        &self.value
    }
    /// The real catalog ledger, for a caller's separately paid serialization or
    /// transport envelope. This is not proof for caller-created allocations.
    pub fn quota(&self) -> &BackingQuota {
        &self.quota
    }
    pub fn charged_index_bytes(&self) -> u64 {
        self._lease.bytes()
    }
}
fn bad(message: &'static str) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}
fn checked_add(total: &mut usize, bytes: usize) -> Result<()> {
    *total = total.checked_add(bytes).ok_or(BackingError::Length)?;
    Ok(())
}
fn text(total: &mut usize, value: &str, maximum: usize, control: &ReadControl<'_>) -> Result<()> {
    if value.len() > maximum {
        return Err(bad("filter metadata exceeds bounded inspection reply"));
    }
    // We only count lengths here; all later byte reads/copies are prepaid below.
    control.check(1)?;
    checked_add(total, value.len())
}
fn metadata(node: &FilterNode, control: &ReadControl<'_>) -> Result<usize> {
    let mut total = 0;
    for value in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
        text(&mut total, value, MAX_TEXT, control)?;
    }
    if let Some(policy) = &node.composition {
        text(&mut total, &policy.compatibility_epoch, MAX_TEXT, control)?;
    }
    for value in [&node.source_display_name, &node.source_icon].into_iter().flatten() {
        text(&mut total, value, MAX_TEXT, control)?;
    }
    if node.arguments.len() > 3 || !node.auxiliary.is_empty() {
        return Err(BackingError::Unsupported.into());
    }
    for argument in &node.arguments {
        text(&mut total, &argument.name, 128, control)?;
        match &argument.value {
            FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_)) => {}
            FilterValue::Enum { domain, .. } => text(&mut total, domain, 256, control)?,
            _ => return Err(BackingError::Unsupported.into()),
        }
    }
    Ok(total)
}
fn point_header_metadata(node: &FilterNode, control: &ReadControl<'_>) -> Result<usize> {
    let mut total = 0;
    for value in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
        text(&mut total, value, MAX_TEXT, control)?;
    }
    if let Some(policy) = &node.composition {
        text(&mut total, &policy.compatibility_epoch, MAX_TEXT, control)?;
    }
    for value in [&node.source_display_name, &node.source_icon].into_iter().flatten() {
        text(&mut total, value, MAX_TEXT, control)?;
    }
    if node.arguments.len() > 10 || !node.auxiliary.is_empty() {
        return Err(BackingError::Unsupported.into());
    }
    Ok(total)
}
fn point_metadata(node: &FilterNode, control: &ReadControl<'_>) -> Result<usize> {
    let mut total = point_header_metadata(node, control)?;
    for argument in &node.arguments {
        text(&mut total, &argument.name, 128, control)?;
        match &argument.value {
            FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_)) | FilterValue::Bool(_) => {}
            FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed { width: 32, value }) if i32::try_from(*value).is_ok() => {}
            FilterValue::Enum { domain, .. } => text(&mut total, domain, 256, control)?,
            FilterValue::Color {components,encoding,profile:None} if components.len()==4 && (*encoding==photocraft_doc::SampleEncoding::NonlinearProfile || *encoding==photocraft_doc::SampleEncoding::Linear && crate::xcf_filter_cmds::semi_flatten_literal_node(node)) => {
                // Exact bounded storage metadata only. Preserve raw words,
                // including nonfinite values; this grants no evaluation/edit.
                control.check(4)?;
                if components.iter().any(|value| !matches!(value, photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))) {
                    return Err(BackingError::Unsupported.into());
                }
            }
            _ => return Err(BackingError::Unsupported.into()),
        }
    }
    Ok(total)
}
fn structured_point_plan(node: &FilterNode, control: &ReadControl<'_>) -> Result<(usize, ReplyBounds, u64)> {
    let mut scanned = point_header_metadata(node, control)?;
    let shape = photocraft_doc::accounted_document::point_argument_shape(&node.arguments, control)?;
    // The same bounded raw shape precedes the existing canonical config wire
    // builder. Count nested names and prospective hexadecimal word strings;
    // the returned facts grant no source, rendering or quota authority.
    for argument in &node.arguments {
        text(&mut scanned, &argument.name, 128, control)?;
    }
    let nested_text = shape
        .utf8_bytes
        .checked_add(shape.f64_words.checked_mul(32).ok_or(BackingError::Length)?)
        .and_then(|n| n.checked_add(shape.config_entries.checked_mul(64)?))
        .and_then(|n| n.checked_add(shape.curve_arrays.checked_mul(128)?))
        .ok_or(BackingError::Length)?;
    checked_add(&mut scanned, usize::try_from(nested_text).map_err(|_| BackingError::Length)?)?;
    let count = |base: u64, words: u64, entries: u64, arrays: u64| -> Result<usize> {
        let n = base
            .checked_add(shape.f64_words.checked_mul(words).ok_or(BackingError::Length)?)
            .and_then(|n| n.checked_add(shape.config_entries.checked_mul(entries)?))
            .and_then(|n| n.checked_add(shape.curve_arrays.checked_mul(arrays)?))
            .ok_or(BackingError::Length)?;
        usize::try_from(n).map_err(|_| BackingError::Length.into())
    };
    let bounds = ReplyBounds { nodes: count(512, 4, 16, 32)?, objects: count(128, 0, 4, 4)?, members: count(512, 0, 16, 32)?, depth: 16, max_string: MAX_TEXT };
    let planned = bounds
        .nodes
        .checked_mul(std::mem::size_of::<Value>())
        .and_then(|n| n.checked_add(bounds.objects.checked_mul(MAP_HEADER)?))
        .and_then(|n| n.checked_add(bounds.members.checked_mul(MAP_ENTRY)?))
        .and_then(|n| n.checked_add(scanned.checked_add(FIXED_TEXT)?.checked_mul(2)?))
        .and_then(|n| n.checked_add(std::mem::size_of::<OwnedFilterInspectReply>() + 128))
        .ok_or(BackingError::Length)?;
    Ok((scanned, bounds, u64::try_from(planned).map_err(|_| BackingError::Length)?))
}
fn point_envelope(text: usize) -> Result<u64> {
    let bytes = POINT_FILTER_BOUNDS
        .nodes
        .checked_mul(std::mem::size_of::<Value>())
        .and_then(|n| n.checked_add(POINT_FILTER_BOUNDS.objects.checked_mul(MAP_HEADER)?))
        .and_then(|n| n.checked_add(POINT_FILTER_BOUNDS.members.checked_mul(MAP_ENTRY)?))
        .and_then(|n| n.checked_add(text.checked_add(FIXED_TEXT)?.checked_mul(2)?))
        .and_then(|n| n.checked_add(std::mem::size_of::<OwnedFilterInspectReply>() + 128))
        .ok_or(BackingError::Length)?;
    u64::try_from(bytes).map_err(|_| BackingError::Length.into())
}
fn envelope(text: usize) -> Result<u64> {
    let bytes = MAX_NODES
        .checked_mul(std::mem::size_of::<Value>())
        .and_then(|n| n.checked_add(MAX_OBJECTS.checked_mul(MAP_HEADER)?))
        .and_then(|n| n.checked_add(MAX_MEMBERS.checked_mul(MAP_ENTRY)?))
        .and_then(|n| n.checked_add(text.checked_add(FIXED_TEXT)?.checked_mul(2)?))
        .and_then(|n| n.checked_add(std::mem::size_of::<OwnedFilterInspectReply>() + 128))
        .ok_or(BackingError::Length)?;
    u64::try_from(bytes).map_err(|_| BackingError::Length.into())
}
#[derive(Default)]
struct Census {
    bytes: usize,
    nodes: usize,
    objects: usize,
    members: usize,
}
impl Census {
    fn string(&mut self, value: &String, bounds: ReplyBounds, control: &ReadControl<'_>) -> Result<()> {
        if value.len() > bounds.max_string || value.capacity() > bounds.max_string.checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Length.into());
        }
        control.check(value.len() as u64 + 1)?;
        checked_add(&mut self.bytes, value.capacity())
    }
    fn value(&mut self, value: &Value, depth: usize, bounds: ReplyBounds, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        self.nodes = self.nodes.checked_add(1).ok_or(BackingError::Length)?;
        if self.nodes > bounds.nodes || depth > bounds.depth {
            return Err(BackingError::Length.into());
        }
        checked_add(&mut self.bytes, std::mem::size_of::<Value>())?;
        match value {
            Value::String(value) => self.string(value, bounds, control)?,
            Value::Array(values) => {
                if values.capacity() > bounds.nodes.checked_mul(2).ok_or(BackingError::Length)? {
                    return Err(BackingError::Length.into());
                }
                checked_add(&mut self.bytes, values.capacity().checked_mul(std::mem::size_of::<Value>()).ok_or(BackingError::Length)?)?;
                for value in values {
                    self.value(value, depth + 1, bounds, control)?;
                }
            }
            Value::Object(values) => {
                self.objects = self.objects.checked_add(1).ok_or(BackingError::Length)?;
                self.members = self.members.checked_add(values.len()).ok_or(BackingError::Length)?;
                if self.objects > bounds.objects || self.members > bounds.members {
                    return Err(BackingError::Length.into());
                }
                checked_add(
                    &mut self.bytes,
                    values
                        .len()
                        .checked_mul(MAP_ENTRY)
                        .and_then(|n| n.checked_add(if values.is_empty() { 0 } else { MAP_HEADER }))
                        .ok_or(BackingError::Length)?,
                )?;
                for (key, value) in values {
                    self.string(key, bounds, control)?;
                    self.value(value, depth + 1, bounds, control)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
/// Inspect exactly one canonical nonzero decimal filterId, with the unchanged
/// registered resident response fields. Only genuine Alpha/Histogram readonly
/// runtime classes plus the explicit Point class are admitted; neither a stored
/// name nor a schema grants it.
/// The immutable Session borrow precludes intervening publication. No model,
/// view, history, redo, coalescing marker, source identity or journal is mutated.
/// This is not an authorization bypass: typed frontends must apply their existing
/// command/workspace authorization to the original command and parameters first.
pub fn prepare(session: &Session, params: &Value, control: &ReadControl<'_>) -> Result<OwnedFilterInspectReply> {
    control.check(1)?;
    params.as_object().filter(|fields| fields.len() == 1 && fields.contains_key("filterId")).ok_or_else(|| bad("expected exactly filterId"))?;
    // Canonical parsing is bounded to twenty bytes by the existing helper.
    control.check(64)?;
    let id = crate::xcf_filter_cmds::id(COMMAND, params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let owner = active.doc.accounted().ok_or(BackingError::Unsupported)?;
    if owner.is_readonly_filtered_flat_point_storage(){
        owner.validate_readonly_filtered_flat_point_storage(control)?;
        let state=owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        if state.filter_stacks.len()>4096{return Err(BackingError::Length.into())}
        let mut found=None;
        let mut nodes=0usize;
        for stack in &state.filter_stacks{control.check(1)?;
            nodes=nodes.checked_add(stack.nodes.len()).filter(|n|*n<=4096).ok_or(BackingError::Length)?;
            if stack.nodes.len()>1024{return Err(BackingError::Length.into())}
            for node in &stack.nodes {control.check(1)?;if node.id==id{if found.is_some(){return Err(BackingError::Integrity.into())}found=Some((stack,node));}}
        }
        let (stack,node)=found.ok_or_else(||bad("unknown filterId"))?;
        control.check(owner.document().layers.len().checked_add(state.filter_stacks.len()).and_then(|n|n.checked_add(nodes)).ok_or(BackingError::Length)? as u64)?;
        let text=point_metadata(node,control)?;let planned=point_envelope(text)?;
        return paid_reply(owner.quota(),ReplyKind::Filter,POINT_FILTER_BOUNDS,planned,text,control,||{
            let mut reply=crate::xcf_filter_cmds::build_point_storage_inspect_reply(id,stack,node)?;
            *reply.get_mut("scalarEditCandidate").ok_or(BackingError::Integrity)?=Value::Bool(match stack.owner {photocraft_doc::item_identity::ItemRef::Layer(layer)=>crate::filtered_flat_filter_edit_candidate(owner,layer,node),_=>false});
            Ok(reply)
        });
    }
    let gaussian_group=owner.is_readonly_gaussian_group_storage();
    let point = if owner.is_readonly_threshold_alpha_storage() {
        owner.validate_readonly_threshold_alpha_storage(control)?;
        false
    } else if owner.is_readonly_threshold_storage() {
        owner.validate_readonly_threshold_storage(control)?;
        false
    } else if owner.is_readonly_gaussian_group_storage() {
        owner.validate_gaussian_group_storage(control)?;
        false
    } else if owner.is_readonly_assigned_point_storage() {
        owner.validate_readonly_assigned_point_storage(control)?;
        true
    } else if owner.is_readonly_point_storage() {
        owner.validate_readonly_point_storage(control)?;
        true
    } else {
        return Err(BackingError::Unsupported.into());
    };
    let state = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let [stack] = state.filter_stacks.as_slice() else {
        return Err(BackingError::Integrity.into());
    };
    let node = if point {
        if stack.nodes.len() > 2 {
            return Err(BackingError::Integrity.into());
        }
        let mut found = None;
        for node in &stack.nodes {
            control.check(1)?;
            if node.id == id {
                if found.is_some() {
                    return Err(BackingError::Integrity.into());
                }
                found = Some(node);
            }
        }
        found.ok_or_else(|| bad("unknown filterId"))?
    } else {
        let [node] = stack.nodes.as_slice() else {
            return Err(BackingError::Integrity.into());
        };
        if node.id != id {
            return Err(bad("unknown filterId"));
        }
        node
    };
    if gaussian_group {
        // The separate actual class fixes five flat F32/Enum/Bool arguments.
        // Use the already bounded complete flat reply envelope; old class
        // selection, thresholds and formulas remain unchanged.
        if node.arguments.len()!=5 {return Err(BackingError::Integrity.into())}
        let text=point_metadata(node,control)?;
        let planned=point_envelope(text)?;
        return paid_reply(owner.quota(),ReplyKind::Filter,POINT_FILTER_BOUNDS,planned,text,control,||{
            crate::xcf_filter_cmds::build_point_storage_inspect_reply(id,stack,node)
        });
    }
    if point {
        control.check(node.arguments.len() as u64 + 1)?;
        let structured = node.arguments.iter().any(|argument| {
            matches!(
                argument.value,
                FilterValue::Config(_) | FilterValue::Curve { .. } | FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float64Bits(_))
            )
        });
        if structured {
            let (text, bounds, planned) = structured_point_plan(node, control)?;
            return paid_reply(owner.quota(), ReplyKind::Filter, bounds, planned, text, control, || {
                crate::xcf_filter_cmds::build_point_storage_inspect_reply(id, stack, node)
            });
        }
        let text = point_metadata(node, control)?;
        let planned = point_envelope(text)?;
        return paid_reply(owner.quota(), ReplyKind::Filter, POINT_FILTER_BOUNDS, planned, text, control, || {
            crate::xcf_filter_cmds::build_point_storage_inspect_reply(id, stack, node)
        });
    }
    let text = metadata(node, control)?;
    let planned = envelope(text)?;
    paid_reply(owner.quota(), ReplyKind::Filter, FILTER_BOUNDS, planned, text, control, || crate::xcf_filter_cmds::build_inspect_reply(id, stack, node))
}
/// Engine-private construction: only the two checked query factories call this.
/// Every returned tree and serde temporary is backed by real same-ledger leases;
/// this exposes neither a public numeric receipt nor a raw Value extractor.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paid_reply(
    quota: &BackingQuota,
    kind: ReplyKind,
    bounds: ReplyBounds,
    planned: u64,
    scanned_text: usize,
    control: &ReadControl<'_>,
    build: impl FnOnce() -> Result<Value>,
) -> Result<OwnedFilterInspectReply> {
    // Actual serde constructs multiple temporary maps/argument vectors before
    // returning its tree. Hold a second complete envelope throughout that stage
    // and final capacity reconciliation; standard serde allocation is the same
    // explicitly bounded provider boundary used by other paid replies.
    let lease = quota.reserve(QuotaKind::Index, planned).map_err(|_| BackingError::Quota)?;
    let mut reply = OwnedFilterInspectReply { value: Value::Null, kind, quota: quota.clone(), _lease: lease };
    let temporary = quota.reserve(QuotaKind::Index, planned).map_err(|_| BackingError::Quota)?;
    let work = planned.checked_mul(2).and_then(|n| n.checked_add((scanned_text as u64).checked_mul(4)?)).ok_or(BackingError::Length)?;
    control.check(work)?;
    reply.value = build()?;
    let mut actual = Census::default();
    actual.value(&reply.value, 0, bounds, control)?;
    checked_add(&mut actual.bytes, std::mem::size_of::<OwnedFilterInspectReply>() + 128)?;
    if actual.bytes as u64 > planned {
        return Err(BackingError::Allocation.into());
    }
    reply._lease.reconcile_capacity(actual.bytes as u64).map_err(|_| BackingError::Quota)?;
    control.check(1)?;
    drop(temporary);
    Ok(reply)
}
#[cfg(test)]
#[path = "accounted_filter_inspect_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "accounted_point_inspect_tests.rs"]
mod point_tests;

#[cfg(test)]
#[path = "accounted_configured_point_inspect_tests.rs"]
mod configured_point_tests;

#[cfg(test)]
#[path = "accounted_point_stack_filter_inspect_tests.rs"]
mod point_stack_tests;
