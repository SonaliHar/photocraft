//! Closed current-model adapter for the existing filter wire commands.
//! A runtime storage class and a fresh complete operative recipe are mandatory;
//! neither persisted operation names nor this request grant source authority.
use super::*;
use crate::{DocumentSnapshot, accounted_view::AccountedView, channel_cmds::ChannelView, document_history::DocumentHistory, source_policy::SourceIdentity};
use photocraft_doc::{
    accounted_document::{AccountedDocument, PointArgumentPatch, PointArgumentValue, PointFilterPatch, accounted_layer_access},
    item_identity::{FilterId, ItemRef},
};

pub(super) const REPLY_ENVELOPE: u64 = 8192;
const STACK_EDITS_EPOCH: &str = "xcf-22-stock-point-stack-edits-v1";
#[path = "point_structured.rs"]
mod structured;
#[derive(Clone, Copy)]
struct Argument<'a> {
    name: &'a str,
    value: PointArgumentValue,
    domain: Option<&'a str>,
}
#[derive(Clone, Copy)]
struct Fields<'a> {
    filter: FilterId,
    arguments: [Option<Argument<'a>>; 10],
    count: usize,
    enabled: Option<bool>,
}

fn flat_value(v: &serde_json::Map<String, Value>) -> Option<(PointArgumentValue, Option<&str>)> {
    flat_value_with_literal(v,false)
}
fn flat_value_with_literal(v: &serde_json::Map<String,Value>,literal:bool)->Option<(PointArgumentValue,Option<&str>)>{
    if v.len() != 1 {
        return None;
    }
    let result = if let Some(n) = v.get("Number") {
        let n = n.as_object()?;
        if n.len() != 1 {
            return None;
        }
        if let Some(signed) = n.get("Signed") {
            let signed = signed.as_object()?;
            if signed.len() != 2 || signed.get("width")?.as_u64()? != 32 {
                return None;
            }
            (PointArgumentValue::Signed32(i32::try_from(signed.get("value")?.as_i64()?).ok()?), None)
        } else {
            let bits = u32::try_from(n.get("Float32Bits")?.as_u64()?).ok()?;
            if !f32::from_bits(bits).is_finite() {
                return None;
            }
            (PointArgumentValue::Float32Bits(bits), None)
        }
    } else if let Some(color) = v.get("Color") {
        let color = color.as_object()?;
        let encoding=match color.get("encoding")?.as_str()?{
            "nonlinear_profile"=>photocraft_doc::SampleEncoding::NonlinearProfile,
            "linear" if literal=>photocraft_doc::SampleEncoding::Linear,
            _=>return None,
        };
        if color.len() != 3 || !color.get("profile")?.is_null() {
            return None;
        }
        let components = color.get("components")?.as_array()?;
        if components.len() != 4 {
            return None;
        }
        let mut words = [0; 4];
        for (word, component) in words.iter_mut().zip(components) {
            let component = component.as_object()?;
            if component.len() != 1 {
                return None;
            }
            let bits = u32::try_from(component.get("Float32Bits")?.as_u64()?).ok()?;
            let value = f32::from_bits(bits);
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return None;
            }
            *word = bits;
        }
        (if literal{PointArgumentValue::LiteralColor4F32{components:words,encoding}}else{PointArgumentValue::Color4F32(words)}, None)
    } else if let Some(v) = v.get("Bool") {
        (PointArgumentValue::Bool(v.as_bool()?), None)
    } else {
        let e = v.get("Enum")?.as_object()?;
        if e.len() != 2 {
            return None;
        }
        let domain = e.get("domain")?.as_str()?;
        if domain.is_empty() || domain.len() > 256 || domain.as_bytes().contains(&0) {
            return None;
        }
        (PointArgumentValue::Enum(i32::try_from(e.get("value")?.as_i64()?).ok()?), Some(domain))
    };
    Some(result)
}

// This is a bounded borrowed wire check, not operation or source eligibility.
fn wire<'a>(id: &str, params: &'a Value) -> Option<Fields<'a>> {
    wire_with_literal(id,params,false)
}
fn wire_with_literal<'a>(id:&str,params:&'a Value,literal:bool)->Option<Fields<'a>>{
    let enable = match id {
        "xcf.filter.set" => false,
        "xcf.filter.enable" => true,
        _ => return None,
    };
    let p = params.as_object()?;
    if p.len() > 4 || p.keys().any(|k| !matches!(k.as_str(), "filterId" | "enabled") && (enable || !matches!(k.as_str(), "arguments" | "opacityF64Bits"))) {
        return None;
    }
    let text = p.get("filterId")?.as_str()?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let filter = FilterId(text.parse().ok()?);
    let enabled = match p.get("enabled") {
        Some(v) => Some(v.as_bool()?),
        None if enable => return None,
        None => None,
    };
    if p.get("opacityF64Bits").is_some_and(|v| v.as_str() != Some("0x3ff0000000000000")) {
        return None;
    }
    let mut arguments = [None; 10];
    let mut count = 0;
    if let Some(values) = p.get("arguments") {
        let values = values.as_array()?;
        if values.len() > 10 {
            return None;
        }
        for value in values {
            let a = value.as_object()?;
            if a.len() != 2 {
                return None;
            }
            let name = a.get("name")?.as_str()?;
            if name.is_empty() || name.len() > 128 || name.as_bytes().contains(&0) || arguments.iter().flatten().any(|v: &Argument<'_>| v.name == name) {
                return None;
            }
            let (value, domain) = flat_value_with_literal(a.get("value")?.as_object()?,literal)?;
            *arguments.get_mut(count)? = Some(Argument { name, value, domain });
            count += 1;
        }
    }
    if count == 0 && enabled.is_none() && !p.contains_key("opacityF64Bits") {
        return None;
    }
    Some(Fields { filter, arguments, count, enabled })
}
pub(super) fn flat_journal_depth(id: &str, params: &Value) -> usize {
    if id != "xcf.filter.set" {
        return 4;
    }
    let Some(fields) = wire_with_literal(id, params, true) else {
        return 4;
    };
    // root0 -> arguments1 -> entry2 -> value3 -> Color4 -> components5
    // -> component object6 -> Float32Bits scalar7. Only this exact closed
    // literal wire gains depth7. Signed32 adds Number4 -> Signed5 ->
    // width/value6; old flat set shapes remain depth5.
    if fields.arguments.iter().flatten().any(|a| matches!(a.value, PointArgumentValue::Color4F32(_) | PointArgumentValue::LiteralColor4F32{..})) {
        7
    } else if fields.arguments.iter().flatten().any(|a| matches!(a.value, PointArgumentValue::Signed32(_))) {
        6
    } else {
        5
    }
}

#[cfg(test)]
pub(super) fn journal_depth(id: &str, params: &Value) -> usize {
    journal_depth_controlled(id, params, &crate::read_control_none()).unwrap_or(4)
}
pub(super) fn journal_depth_controlled(id: &str, params: &Value, control: &ReadControl<'_>) -> Result<usize> {
    let flat = flat_journal_depth(id, params);
    if flat > 4 || id != "xcf.filter.set" {
        return Ok(flat);
    }
    Ok(structured::preflight(id, params, control)?.map_or(4, |p| p.depth))
}

fn move_wire(params: &Value) -> Option<(Fields<'_>, usize)> {
    let p = params.as_object()?;
    if p.len() != 2 || !p.contains_key("filterId") || !p.contains_key("index") {
        return None;
    }
    let text = p.get("filterId")?.as_str()?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let filter = FilterId(text.parse().ok()?);
    let index = usize::try_from(p.get("index")?.as_u64().filter(|v| *v < 1024)?).ok()?;
    Some((Fields { filter, arguments: [None; 10], count: 0, enabled: None }, index))
}
fn remove_wire(params: &Value) -> Option<Fields<'_>> {
    let p = params.as_object()?;
    if p.len() != 1 {
        return None;
    }
    let text = p.get("filterId")?.as_str()?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(Fields { filter: FilterId(text.parse().ok()?), arguments: [None; 10], count: 0, enabled: None })
}
fn pre_dispatch(session: &Session, id: &str, params: &Value) -> Result<()> {
    if id == "xcf.filter.move" {
        crate::xcf_filter_cmds::pre_dispatch_point_move(session, id, params, false)
    } else if id == "xcf.filter.remove" {
        crate::xcf_filter_cmds::pre_dispatch_point_remove(session, id, params, false)
    } else {
        crate::xcf_filter_cmds::pre_dispatch_set_change(session, id, params, false)
    }
}

#[derive(Clone, Copy)]
enum EditStorage {
    ImplicitPoint,
    AssignedBox,
    ObservedBox,
}
/// Cheap CURRENT routing hint only; complete recipe proof remains mandatory.
fn semi_flatten_edit_candidate(owner:&AccountedDocument)->bool{crate::xcf_filter_cmds::semi_flatten_literal_owner(owner)}
pub(super) fn observed_box_edit_candidate(owner: &AccountedDocument) -> bool {
    (owner.is_readonly_point_storage() || owner.is_readonly_assigned_point_storage())
        && owner.document().xcf_state.as_ref().is_some_and(|state| {
            matches!(state.filter_stacks.as_slice(), [stack] if matches!(stack.nodes.as_slice(), [node]
                if node.operation.name == "gegl:box-blur"
                    && node.operation.stored_version == "0:0"
                    && node.operation.schema_epoch == photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH
                    && node.composition.as_ref().is_some_and(|p| p.compatibility_epoch == photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH)))
        })
}
fn observed_box_fields<'a>(id: &str, params: &'a Value) -> Option<Fields<'a>> {
    let object = params.as_object()?;
    if object.keys().any(|key| !matches!(key.as_str(), "filterId" | "arguments" | "enabled")) {
        return None;
    }
    let fields = wire(id, params)?;
    if object.contains_key("arguments") {
        if id != "xcf.filter.set" || fields.count != 1 {
            return None;
        }
        let argument = fields.arguments.first()?.as_ref()?;
        if argument.name != "radius" || !matches!(argument.value, PointArgumentValue::Signed32(0..=1000)) || argument.domain.is_some() {
            return None;
        }
    } else if fields.enabled.is_none() {
        return None;
    }
    Some(fields)
}
pub(super) fn observed_box_wire_valid(id: &str, params: &Value) -> bool {
    observed_box_fields(id, params).is_some()
}
/// Routing hint only. Full assigned storage/profile/native qualification still
/// runs before either equal or changed activation can be accepted.
pub(super) fn assigned_enabled_candidate(owner: &AccountedDocument) -> bool {
    owner.is_readonly_assigned_point_storage()
        && owner.document().xcf_state.as_ref().is_some_and(|state| {
            matches!(state.filter_stacks.as_slice(), [stack] if matches!(stack.nodes.as_slice(), [node] if node.operation.name == "gegl:box-blur"))
        })
}
/// The observational source geometry epoch has rendering/native continuation
/// only. It must not inherit the generic implicit Point mutation wire.
pub(super) fn observed_box_readonly(owner: &AccountedDocument) -> bool {
    owner.document().xcf_state.as_ref().is_some_and(|state| {
        matches!(state.filter_stacks.as_slice(), [stack] if matches!(stack.nodes.as_slice(), [node]
            if node.operation.schema_epoch == photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH
                || node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH)))
    })
}
fn assigned_enabled_fields<'a>(id: &str, params: &'a Value) -> Option<Fields<'a>> {
    if !matches!(id,"xcf.filter.set"|"xcf.filter.enable") { return None; }
    let object=params.as_object()?;
    if object.len()!=2 || object.keys().any(|key| !matches!(key.as_str(),"filterId"|"enabled")) { return None; }
    wire(id,params).filter(|fields|fields.enabled.is_some())
}
/// Cheap borrowed eligibility; never invokes upstream helpers with an unpaid
/// work control. Actual parse and preparation repeat it under the real ledger.
pub(super) fn assigned_enabled_wire_valid(id: &str, params: &Value) -> bool {
    assigned_enabled_fields(id,params).is_some()
}
pub(super) struct Request<'a> {
    id: &'static str,
    params: &'a Value,
    fields: Fields<'a>,
    structured: Option<structured::Plan>,
    move_to: Option<usize>,
    storage: EditStorage,
}
impl<'a> Request<'a> {
    pub(super) fn parse_observed_box(session: &Session, id: &str, params: &'a Value, ctl: &ReadControl<'_>) -> Result<Self> {
        ctl.check(32768)?;
        let id = match id {
            "xcf.filter.set" => "xcf.filter.set",
            "xcf.filter.enable" => "xcf.filter.enable",
            _ => return Err(BackingError::Unsupported.into()),
        };
        let fields = observed_box_fields(id, params).ok_or_else(|| bad(id, "observed Box requires radius Signed32 0..1000 and/or boolean enabled"))?;
        let owner = session.active().and_then(|state| state.doc.accounted()).ok_or(BackingError::Unsupported)?;
        if !observed_box_edit_candidate(owner) { return Err(BackingError::Unsupported.into()); }
        ctl.check(32768)?;
        pre_dispatch(session, id, params)?;
        if crate::xcf_filter_cmds::id(id, params)? != fields.filter { return Err(BackingError::Integrity.into()); }
        Ok(Self { id, params, fields, structured: None, move_to: None, storage: EditStorage::ObservedBox })
    }
    /// Assigned storage does not inherit the implicit Point patch parser.
    /// Only the existing two-field activation wire is admitted here.
    pub(super) fn parse_assigned_enabled(session: &Session, id: &str, params: &'a Value, ctl: &ReadControl<'_>) -> Result<Self> {
        ctl.check(32768)?;
        let id = match id {
            "xcf.filter.set" => "xcf.filter.set",
            "xcf.filter.enable" => "xcf.filter.enable",
            _ => return Err(BackingError::Unsupported.into()),
        };
        let fields=assigned_enabled_fields(id,params).ok_or_else(||bad(id,"assigned activation requires only canonical filterId and boolean enabled"))?;
        let owner=session.active().and_then(|state|state.doc.accounted()).ok_or(BackingError::Unsupported)?;
        if !assigned_enabled_candidate(owner) { return Err(BackingError::Unsupported.into()); }
        ctl.check(32768)?;
        pre_dispatch(session,id,params)?;
        if crate::xcf_filter_cmds::id(id,params)? != fields.filter { return Err(BackingError::Integrity.into()); }
        Ok(Self { id,params,fields,structured:None,move_to:None,storage:EditStorage::AssignedBox })
    }
    pub(super) fn parse(session: &Session, id: &str, params: &'a Value, ctl: &ReadControl<'_>) -> Result<Self> {
        ctl.check(32768)?;
        if session.active().and_then(|state| state.doc.accounted()).is_some_and(observed_box_readonly) {
            return Err(BackingError::Unsupported.into());
        }
        let id = match id {
            "xcf.filter.set" => "xcf.filter.set",
            "xcf.filter.enable" => "xcf.filter.enable",
            "xcf.filter.move" => "xcf.filter.move",
            "xcf.filter.remove" => "xcf.filter.remove",
            _ => return Err(BackingError::Unsupported.into()),
        };
        let literal=session.active().and_then(|state|state.doc.accounted()).is_some_and(semi_flatten_edit_candidate);
        let (fields, structured, move_to) = if id == "xcf.filter.move" {
            let (fields, index) = move_wire(params).ok_or_else(|| bad(id, "canonical filterId and bounded owner-local index required"))?;
            (fields, None, Some(index))
        } else if id == "xcf.filter.remove" {
            (remove_wire(params).ok_or_else(|| bad(id, "canonical filterId required"))?, None, None)
        } else if let Some(fields) = wire_with_literal(id, params, literal) {
            (fields, None, None)
        } else {
            let plan = structured::preflight(id, params, ctl)?.ok_or_else(|| bad(id, "unsupported bounded Point patch wire"))?;
            (Fields { filter: plan.filter, arguments: [None; 10], count: 0, enabled: plan.enabled }, Some(plan), None)
        };
        // The closed shape has at most four fixed keys and ten bounded typed patches,
        // bounded identity digits and opacity characters. Price its
        // repeated upstream wire/identity checks before invoking those helpers.
        ctl.check(structured.map_or(Ok(32768), |plan| plan.helper_work())?)?;
        pre_dispatch(session, id, params)?;
        // Preserve the upstream canonical identity parser as the command authority.
        if crate::xcf_filter_cmds::id(id, params)? != fields.filter {
            return Err(BackingError::Integrity.into());
        }
        Ok(Self { id, params, fields, structured, move_to, storage:EditStorage::ImplicitPoint })
    }
    pub(super) fn prepare(self, session: &Session, ctl: &ReadControl<'_>) -> Result<Prepared<'a>> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        if observed_box_readonly(owner) && !matches!(self.storage, EditStorage::ObservedBox) {
            return Err(BackingError::Unsupported.into());
        }
        match self.storage {
            EditStorage::ObservedBox => {
                if !observed_box_edit_candidate(owner) { return Err(BackingError::Unsupported.into()); }
                if owner.is_readonly_assigned_point_storage() {
                    owner.validate_readonly_assigned_point_storage(ctl)?;
                } else {
                    owner.validate_readonly_point_storage(ctl)?;
                }
                ctl.check(1)?;
                if !state.history.current()?.shares_state(&state.doc) { return Err(BackingError::Integrity.into()); }
            }
            EditStorage::ImplicitPoint => {owner.validate_readonly_point_storage(ctl)?;if (semi_flatten_edit_candidate(owner) || crate::xcf_filter_cmds::hue_chroma_owner(owner) || crate::xcf_filter_cmds::vibrance_owner(owner) || crate::xcf_filter_cmds::loaded_exposure_point_owner(owner) || crate::xcf_filter_cmds::masked_exposure_point_owner(owner)) && !state.history.current()?.shares_state(&state.doc){return Err(BackingError::Integrity.into());}},
            EditStorage::AssignedBox => {
                owner.validate_readonly_assigned_point_storage(ctl)?;
                ctl.check(1)?;
                if !state.history.current()?.shares_state(&state.doc) { return Err(BackingError::Integrity.into()); }
                if !assigned_enabled_candidate(owner) { return Err(BackingError::Unsupported.into()); }
            }
        }
        let xcf = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let stack = xcf.filter_stacks.first().ok_or(BackingError::Integrity)?;
        if stack.nodes.is_empty() || stack.nodes.len() > 2 {
            return Err(BackingError::Unsupported.into());
        }
        let mut target = None;
        for candidate in &stack.nodes {
            ctl.check(1)?;
            if candidate.id == self.fields.filter {
                if target.is_some() {
                    return Err(BackingError::Integrity.into());
                }
                target = Some(candidate);
            }
        }
        let node = target.ok_or_else(|| bad(self.id, "filterId does not identify a current Point filter"))?;
        if self.move_to.is_some() || self.id == "xcf.filter.remove" {
            if self.move_to.is_some_and(|index| index >= stack.nodes.len()) {
                return Err(bad(self.id, "index is outside the current owner stack"));
            }
            // Preserve upstream stack-change revision headroom, including a
            // same-position move. Other typed set/no-op policies stay unchanged.
            if stack.revision == u64::MAX {
                return Err(BackingError::Length.into());
            }
            for candidate in &stack.nodes {
                ctl.check(8192)?;
                if !(semi_flatten_edit_candidate(owner) || crate::xcf_filter_cmds::hue_chroma_owner(owner) || crate::xcf_filter_cmds::vibrance_owner(owner) || crate::xcf_filter_cmds::loaded_exposure_point_owner(owner) || crate::xcf_filter_cmds::masked_exposure_point_owner(owner)) && (candidate.operation.schema_epoch != STACK_EDITS_EPOCH
                    || candidate.composition.as_ref().is_none_or(|p| p.compatibility_epoch != STACK_EDITS_EPOCH))
                {
                    return Err(BackingError::Unsupported.into());
                }
            }
        }

        let ItemRef::Layer(layer) = stack.owner else { return Err(BackingError::Unsupported.into()) };
        let visibility_changed = self.fields.enabled.is_some_and(|v| v != node.enabled);
        locks(owner, layer, visibility_changed, ctl)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        channel_shape(&state.channel_view)?;
        if matches!(state.channel_view.target,crate::channel_cmds::ChannelTarget::Color(i) if i>=state.doc.mode.color_channels()) {
            return Err(BackingError::Bounds.into());
        }
        if state.path.as_ref().is_some_and(|v| v.len() > 4096)
            || state.source.as_ref().is_some_and(|s| s.label.len() > 4096 || s.path.as_ref().is_some_and(|v| v.len() > 4096))
        {
            return Err(BackingError::Length.into());
        }
        let strings = state
            .path
            .as_ref()
            .map_or(0, String::len)
            .checked_add(state.source.as_ref().map_or(0, |s| s.label.len() + s.path.as_ref().map_or(0, String::len)))
            .ok_or(BackingError::Length)?;
        let count = state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;
        let envelope = strings
            .checked_add(count)
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(std::mem::size_of::<Prepared<'_>>() + 512))
            .ok_or(BackingError::Length)?;
        let mut lease = owner.quota().reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
        ctl.check(strings.checked_add(count).ok_or(BackingError::Length)? as u64)?;
        let path = state.path.as_deref().map(copy).transpose()?;
        let source = state
            .source
            .as_ref()
            .map(|s| Ok::<_, EngineError>(SourceIdentity { label: copy(&s.label)?, path: s.path.as_deref().map(copy).transpose()?, origin: s.origin }))
            .transpose()?;
        let channel = ChannelView {
            target: state.channel_view.target,
            color_hidden: flags(&state.channel_view.color_hidden, ctl)?,
            alpha_visible: flags(&state.channel_view.alpha_visible, ctl)?,
            quick_mask_hidden: state.channel_view.quick_mask_hidden,
            layer_mask: None,
            editing_mask: None,
        };
        let actual = path
            .as_ref()
            .map_or(0, String::capacity)
            .checked_add(source.as_ref().map_or(0, |s| s.label.capacity() + s.path.as_ref().map_or(0, String::capacity)))
            .and_then(|v| v.checked_add(channel.color_hidden.capacity() + channel.alpha_visible.capacity()))
            .and_then(|v| v.checked_add(std::mem::size_of::<Prepared<'_>>() + 512))
            .ok_or(BackingError::Length)?;
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        validate_recipe(owner, ctl)?;
        // Fixed stack storage retains the original argument names and enum domains.
        // No parser proxy or per-operation arithmetic is reconstructed here.
        let staged = if matches!(self.storage,EditStorage::AssignedBox) {
            owner.try_with_readonly_assigned_point_enabled(
                photocraft_doc::accounted_document::AssignedPointEnabledPatch {
                    layer,filter:self.fields.filter,expected_stack_revision:stack.revision,
                    enabled:self.fields.enabled.ok_or(BackingError::Integrity)?,
                },ctl,
            )?
        } else if let Some(index) = self.move_to {
            owner.try_move_readonly_point_filter(
                photocraft_doc::accounted_document::PointFilterMovePatch { layer, filter: self.fields.filter, expected_stack_revision: stack.revision, index },
                ctl,
            )?
        } else if self.id == "xcf.filter.remove" {
            owner.try_remove_readonly_point_filter(
                photocraft_doc::accounted_document::PointFilterRemovePatch { layer, filter: self.fields.filter, expected_stack_revision: stack.revision },
                ctl,
            )?
        } else if let Some(plan) = self.structured {
            let temporary = structured::Temporary::parse(self.params, &node.operation.name, plan, owner.quota(), ctl)?;
            let sentinel = photocraft_doc::xcf_filters::FilterValue::Bool(false);
            let mut patches = [photocraft_doc::accounted_document::PointStructuredArgumentPatch { name: "", value: &sentinel }; 10];
            for (slot, argument) in patches.iter_mut().zip(temporary.arguments()) {
                *slot = photocraft_doc::accounted_document::PointStructuredArgumentPatch { name: &argument.name, value: &argument.value };
            }
            let staged = owner.try_with_readonly_point_structured_filter(
                photocraft_doc::accounted_document::PointStructuredFilterPatch {
                    layer,
                    filter: self.fields.filter,
                    expected_stack_revision: stack.revision,
                    arguments: patches.get(..temporary.arguments().len()).ok_or(BackingError::Integrity)?,
                    enabled: self.fields.enabled,
                },
                ctl,
            )?;
            validate_recipe(&staged, ctl)?;
            drop(temporary);
            staged
        } else {
            let mut patches = [PointArgumentPatch { name: "", value: PointArgumentValue::Bool(false) }; 10];
            for (index, argument) in self.fields.arguments.iter().take(self.fields.count).enumerate() {
                let argument = argument.as_ref().ok_or(BackingError::Integrity)?;
                ctl.check(4096)?;
                // New incoming black words remain STRICT even when the loaded
                // stored endpoint has V2 effective-zero interpretation.
                if crate::xcf_filter_cmds::loaded_exposure_point_owner(owner) || crate::xcf_filter_cmds::masked_exposure_point_owner(owner) {
                    let PointArgumentValue::Float32Bits(bits)=argument.value else{return Err(BackingError::Unsupported.into())};
                    let value=f64::from(f32::from_bits(bits));
                    if !value.is_finite() || !matches!(argument.name,"exposure"|"black-level") || argument.name=="black-level" && !(-0.1..=0.1).contains(&value){return Err(BackingError::Unsupported.into())}
                }
                let stored = node.arguments.iter().find(|v| v.name == argument.name).ok_or(BackingError::Unsupported)?;
                use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
                match (&stored.value, argument.value, argument.domain) {
                    (FilterValue::Number(ExactNumber::Signed { width: 32, value }), PointArgumentValue::Signed32(_), None) if i32::try_from(*value).is_ok() => {
                    }
                    (FilterValue::Number(ExactNumber::Float32Bits(_)), PointArgumentValue::Float32Bits(_), None)
                    | (FilterValue::Bool(_), PointArgumentValue::Bool(_), None) => {}
                    (FilterValue::Enum { domain, .. }, PointArgumentValue::Enum(_), Some(requested)) if domain == requested => {}
                    (
                        FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None },
                        PointArgumentValue::Color4F32(_),
                        None,
                    ) if components.len() == 4 && components.iter().all(|c| matches!(c, ExactNumber::Float32Bits(_))) => {}
                    (FilterValue::Color{components,encoding,profile:None},PointArgumentValue::LiteralColor4F32{encoding:requested,..},None)
                        if semi_flatten_edit_candidate(owner) && matches!(encoding,photocraft_doc::SampleEncoding::Linear|photocraft_doc::SampleEncoding::NonlinearProfile) && matches!(requested,photocraft_doc::SampleEncoding::Linear|photocraft_doc::SampleEncoding::NonlinearProfile) && components.len()==4 && components.iter().all(|v|matches!(v,ExactNumber::Float32Bits(_)))=>{}
                    _ => return Err(BackingError::Unsupported.into()),
                }
                *patches.get_mut(index).ok_or(BackingError::Integrity)? = PointArgumentPatch { name: argument.name, value: argument.value };
            }
            let patch = PointFilterPatch {
                    layer,
                    filter: self.fields.filter,
                    expected_stack_revision: stack.revision,
                    arguments: patches.get(..self.fields.count).ok_or(BackingError::Integrity)?,
                    enabled: self.fields.enabled,
                };
            let staged = if matches!(self.storage, EditStorage::ObservedBox) && owner.is_readonly_assigned_point_storage() {
                owner.try_with_readonly_assigned_point_filter(patch, ctl)?
            } else {
                owner.try_with_readonly_point_filter(patch, ctl)?
            };
            staged
        };
        if self.structured.is_none() {
            validate_recipe(&staged, ctl)?;
        }
        let updated = !staged.shares_model(owner);
        let mut history = state.history.clone();
        let snapshot = if updated {
            history.max_states = session.prefs().performance.history_states.max(1) as usize;
            history.max_bytes = session.prefs().performance.history_budget_bytes();
            history.set_current_layers_borrowed(view.target(), ctl)?;
            history.try_commit_prepared_accounted(
                owner,
                staged,
                if self.move_to.is_some() {
                    "Move GIMP Stock Filter"
                } else if self.id == "xcf.filter.remove" {
                    "Remove GIMP Stock Filter"
                } else {
                    "Edit GIMP Stock Filter"
                },
                view.target(),
                ctl,
            )?
        } else {
            state.doc.clone()
        };
        let next_view = view.try_replace(snapshot.accounted().ok_or(BackingError::Integrity)?, state.active_layer, view.selected(), state.layer_anchor, ctl)?;
        let reply_lease = owner.quota().reserve(QuotaKind::Index, REPLY_ENVELOPE).map_err(|_| BackingError::Quota)?;
        ctl.check(REPLY_ENVELOPE)?;
        // Canonical wire identity is at most twenty bytes. Retain that exact
        // bounded string; JSON contains only two fixed keys and scalar cells.
        let identity = copy(self.params.get("filterId").and_then(Value::as_str).ok_or(BackingError::Integrity)?)?;
        let reply = serde_json::json!({"filterId":identity,"updated":updated});
        ctl.check(1)?;
        Ok(Prepared {
            request: self,
            captured: state.doc.clone(),
            snapshot,
            history,
            old_history: state.history.clone(),
            view: view.clone(),
            next_view,
            revision: state.revision,
            view_revision: state.view_revision,
            active: state.active_layer,
            anchor: state.layer_anchor,
            channel,
            path,
            source,
            read_only: state.source_read_only,
            history_states: session.prefs().performance.history_states,
            history_bytes: session.prefs().performance.history_budget_bytes(),
            layer,
            visibility_changed,
            updated,
            reply,
            _reply_lease: reply_lease,
            _lease: lease,
        })
    }
}

fn copy(v: &str) -> Result<String> {
    let mut s = String::new();
    s.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if s.capacity() > v.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    s.push_str(v);
    Ok(s)
}
fn flags(v: &[bool], ctl: &ReadControl<'_>) -> Result<Vec<bool>> {
    let mut out = Vec::new();
    out.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if out.capacity() > v.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for b in v {
        ctl.check(1)?;
        out.push(*b);
    }
    Ok(out)
}
fn channel_shape(v: &ChannelView) -> Result<()> {
    if v.color_hidden.len() > 32 || v.alpha_visible.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    if v.layer_mask.is_some()
        || v.editing_mask.is_some()
        || !matches!(v.target, crate::channel_cmds::ChannelTarget::Composite | crate::channel_cmds::ChannelTarget::Color(_))
    {
        return Err(BackingError::Unsupported.into());
    }
    Ok(())
}
fn locks(owner: &AccountedDocument, layer: LayerId, visibility: bool, ctl: &ReadControl<'_>) -> Result<()> {
    let a = accounted_layer_access(owner.document(), layer, ctl)?.ok_or(EngineError::NoLayer(layer))?;
    if a.locks.all || a.locks.pixels || a.imported_locks.content || visibility && a.imported_locks.visibility {
        return Err(EngineError::Other("the filter owner is locked".into()));
    }
    Ok(())
}
fn validate_recipe(owner: &AccountedDocument, ctl: &ReadControl<'_>) -> Result<()> {
    crate::accounted_native_open::validate_current_stock_point_storage(owner, ctl)
}

pub(super) struct Prepared<'a> {
    request: Request<'a>,
    captured: DocumentSnapshot,
    snapshot: DocumentSnapshot,
    history: DocumentHistory,
    old_history: DocumentHistory,
    view: AccountedView,
    next_view: AccountedView,
    revision: u64,
    view_revision: u64,
    active: Option<LayerId>,
    anchor: Option<LayerId>,
    channel: ChannelView,
    path: Option<String>,
    source: Option<SourceIdentity>,
    read_only: bool,
    history_states: u32,
    history_bytes: usize,
    layer: LayerId,
    visibility_changed: bool,
    updated: bool,
    reply: Value,
    _reply_lease: QuotaLease,
    _lease: QuotaLease,
}
impl Prepared<'_> {
    pub(super) fn execute(self, session: &mut Session, ctl: &ReadControl<'_>) -> Result<Value> {
        ctl.check(self.request.structured.map_or(Ok(32768), |plan| plan.helper_work())?)?;
        if session.active().and_then(|state| state.doc.accounted()).is_some_and(observed_box_readonly)
            && !matches!(self.request.storage, EditStorage::ObservedBox) {
            return Err(BackingError::Unsupported.into());
        }
        pre_dispatch(session, self.request.id, self.request.params)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        channel_shape(&state.channel_view)?;
        ctl.check((self.channel.color_hidden.len() + self.channel.alpha_visible.len() + 1) as u64)?;
        for text in self.path.iter().chain(self.source.iter().flat_map(|s| std::iter::once(&s.label).chain(s.path.iter()))) {
            ctl.check(text.len() as u64)?;
        }
        if !state.doc.shares_state(&self.captured)
            || state.revision != self.revision
            || state.view_revision != self.view_revision
            || !view.shares_state(&self.view)
            || state.active_layer != self.active
            || state.layer_anchor != self.anchor
            || !state.selected_layers.is_empty()
            || state.channel_view != self.channel
            || state.path != self.path
            || state.source != self.source
            || state.source_read_only != self.read_only
            || !state.history.shares_accounted_history(&self.old_history)
            || state.history.max_states != self.old_history.max_states
            || state.history.max_bytes != self.old_history.max_bytes
            || session.prefs().performance.history_states != self.history_states
            || session.prefs().performance.history_budget_bytes() != self.history_bytes
        {
            return Err(EngineError::Other("the prepared Point edit targets older Session state".into()));
        }
        locks(state.doc.accounted().ok_or(BackingError::Integrity)?, self.layer, self.visibility_changed, ctl)?;
        let revision = state.revision.checked_add(u64::from(self.updated)).ok_or(BackingError::Length)?;
        ctl.check(1)?;
        if self.updated {
            let state = session.active_mut().ok_or(EngineError::NoDocument)?;
            state.doc = self.snapshot;
            state.history = self.history;
            state.accounted_view = Some(self.next_view);
            state.revision = revision;
            state.last_damage = None;
            state.coalesce = None;
            state._accounted_metadata_gesture = None;
        }
        Ok(self.reply)
    }
}

#[cfg(test)]
#[path = "point_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "point_stack_tests.rs"]
mod stack_tests;

#[cfg(test)]
#[path = "point_move_tests.rs"]
mod move_tests;

#[cfg(test)]
#[path = "point_remove_tests.rs"]
mod remove_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "point_empty_lifecycle_tests.rs"]
mod empty_lifecycle_tests;

#[cfg(all(test, feature="corpus", not(target_arch="wasm32")))]
#[path="point_assigned_enabled_tests.rs"]
mod assigned_enabled_tests;

#[cfg(all(test, feature="corpus", not(target_arch="wasm32")))]
#[path="point_observed_edit_tests.rs"]
mod observed_edit_tests;

#[cfg(all(test,feature="corpus",not(target_arch="wasm32")))]
#[path="point_semi_flatten_tests.rs"]
mod semi_flatten_tests;

#[cfg(all(test,feature="corpus",not(target_arch="wasm32")))]
#[path="point_hue_chroma_tests.rs"]
mod hue_chroma_tests;

#[cfg(test)]
#[path="point_vibrance_tests.rs"]
mod vibrance_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "point_origin_geometry_tests.rs"]
mod origin_geometry_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "point_bundle_geometry_tests.rs"]
mod bundle_geometry_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "point_residual_geometry_tests.rs"]
mod residual_geometry_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="exposure_masked_tests.rs"]
mod exposure_masked_tests;
