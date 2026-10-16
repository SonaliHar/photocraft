//! Closed current-model adapter for the existing filter wire commands.
//! A runtime storage class and a fresh complete operative recipe are mandatory;
//! neither persisted operation names nor this request grant source authority.
use super::*;
use crate::{DocumentSnapshot, accounted_view::AccountedView, channel_cmds::ChannelView, document_history::DocumentHistory, source_policy::SourceIdentity};
use photocraft_doc::{
    accounted_document::{AccountedDocument, PointArgumentPatch, PointArgumentValue, PointFilterPatch, PointFilterMovePatch, PointFilterRemovePatch},
    item_identity::{FilterId, ItemRef},
};

pub(super) const REPLY_ENVELOPE: u64 = 8192;
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
    opacity:Option<u64>,
}

fn flat_value_with_literal(v: &serde_json::Map<String, Value>, literal: bool) -> Option<(PointArgumentValue, Option<&str>)> {
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
        let encoding = match color.get("encoding")?.as_str()? {
            "nonlinear_profile" => photocraft_doc::SampleEncoding::NonlinearProfile,
            "linear" if literal => photocraft_doc::SampleEncoding::Linear,
            _ => return None,
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
        (if literal { PointArgumentValue::LiteralColor4F32 { components: words, encoding } } else { PointArgumentValue::Color4F32(words) }, None)
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

// Resident bits64 grammar is private; retain its bounded canonical form here.
fn opacity_bits(value:&Value)->Option<u64>{
    let text=value.as_str().filter(|s|s.len()==18&&s.starts_with("0x"))?;
    let digits=text.get(2..)?;
    if !digits.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase()){return None}
    let bits=u64::from_str_radix(digits,16).ok()?;let value=f64::from_bits(bits);
    (value.is_finite()&&(0.0..=1.0).contains(&value)).then_some(bits)
}

// This is a bounded borrowed wire check, not operation or source eligibility.
fn wire_with_literal<'a>(id: &str, params: &'a Value, literal: bool) -> Option<Fields<'a>> {
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
    let opacity=match p.get("opacityF64Bits"){Some(value)=>Some(opacity_bits(value)?),None=>None};
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
            let (value, domain) = flat_value_with_literal(a.get("value")?.as_object()?, literal)?;
            *arguments.get_mut(count)? = Some(Argument { name, value, domain });
            count += 1;
        }
    }
    if count == 0 && enabled.is_none() && !p.contains_key("opacityF64Bits") {
        return None;
    }
    Some(Fields { filter, arguments, count, enabled, opacity })
}

fn family(node: &photocraft_doc::xcf_filters::FilterNode) -> Option<u8> {
    let (kind, version, epoch) = match node.operation.name.as_str() {
        "gimp:semi-flatten" => (0, "0:0", photocraft_compose::SEMI_FLATTEN_EPOCH),
        "gegl:hue-chroma" => (1, "0:0", photocraft_compose::HUE_CHROMA_EPOCH),
        "gegl:vibrance" => (2, "0:0", photocraft_compose::VIBRANCE_EPOCH),
        "gegl:exposure" => (3, "1:0", if node.operation.schema_epoch==photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH {photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH}else{photocraft_compose::EXPOSURE_MIXED_EPOCH}),
        "gegl:invert-linear" => (4, "0:0", photocraft_compose::INVERT_LINEAR_MIXED_EPOCH),
        "gegl:invert-gamma" => (5, "0:0", photocraft_compose::INVERT_GAMMA_MIXED_EPOCH),
        "gegl:brightness-contrast" => (6, "0:0", photocraft_compose::BRIGHTNESS_CONTRAST_MIXED_EPOCH),
        "gegl:levels" => (7, "0:0", photocraft_compose::GEGL_LEVELS_MIXED_EPOCH),
        "gegl:color-temperature" => (8, "0:0", photocraft_compose::COLOR_TEMPERATURE_MIXED_EPOCH),
        "gegl:sepia" => (9, "0:0", photocraft_compose::SEPIA_MIXED_EPOCH),
        "gegl:alien-map" if node.operation.schema_epoch==photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH => (11, "0:0", photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH),
        "gegl:alien-map" => (10, "0:0", photocraft_compose::ALIEN_MAP_RGB_MIXED_EPOCH),
        _ => return None,
    };
    (node.operation.stored_version == version && node.operation.schema_epoch == epoch && node.composition.as_ref().is_some_and(|p| p.compatibility_epoch == epoch)).then_some(kind)
}
fn range(kind: u8, name: &str) -> Option<(f32, f32)> {
    match (kind, name) {
        (1, "hue") => Some((-180., 180.)),
        (1, "chroma" | "lightness") | (2, "vibrance") => Some((-100., 100.)),
        (2, "saturation") => Some((0., 10.)),
        (8, "original-temperature" | "intended-temperature") => Some((1000., 12000.)),
        (9, "scale") => Some((0., 1.)),
        (10|11, "cpn-1-frequency" | "cpn-2-frequency" | "cpn-3-frequency") => Some((0., 20.)),
        (10|11, "cpn-1-phaseshift" | "cpn-2-phaseshift" | "cpn-3-phaseshift") => Some((0., 360.)),
        (6, "brightness") => Some((-3., 3.)),
        (6, "contrast") => Some((-5., 5.)),
        _ => None,
    }
}
fn number_in_domain(kind:u8,name:&str,bits:u32)->bool {
    let value=f32::from_bits(bits);
    if !value.is_finite(){return false}
    if kind==7 {return matches!(name,"in-low"|"in-high"|"out-low"|"out-high")}
    if kind==3 {
        return match name {
            "exposure"=>true,
            // Preserve the existing promoted-f64 kernel domain. The f32 .1
            // word lies outside it and must not be silently normalized.
            "black-level"=>(-0.1f64..=0.1f64).contains(&f64::from(value)),
            _=>false,
        };
    }
    range(kind,name).is_some_and(|(lo,hi)|(lo..=hi).contains(&value))
}
fn valid_node(node: &photocraft_doc::xcf_filters::FilterNode) -> bool {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let Some(kind) = family(node) else { return false };
    if kind == 8 { return photocraft_compose::color_temperature_mixed_parameters(&node.arguments).is_ok(); }
    if kind == 9 { return photocraft_compose::sepia_mixed_parameters(&node.arguments).is_ok(); }
    if kind == 10 { return photocraft_compose::alien_map_rgb_mixed_parameters(&node.arguments).is_ok(); }
    if kind == 11 { return photocraft_compose::alien_map_hsl_mixed_parameters(&node.arguments).is_ok(); }
    if kind == 6 { return photocraft_compose::brightness_contrast_mixed_parameters(&node.arguments).is_ok(); }
    if kind == 7 { return photocraft_compose::gegl_levels_mixed_parameters(&node.arguments).is_ok(); }
    if matches!(kind,4|5) { return node.arguments.is_empty(); }
    if kind == 0 {
        return matches!(node.arguments.as_slice(),[a] if a.name=="color"&&matches!(&a.value,FilterValue::Color{components,encoding:photocraft_doc::SampleEncoding::Linear|photocraft_doc::SampleEncoding::NonlinearProfile,profile:None} if components.len()==4&&components.iter().all(|v|matches!(v,ExactNumber::Float32Bits(b) if f32::from_bits(*b).is_finite()&&(0.0..=1.0).contains(&f32::from_bits(*b))))));
    }
    if node.arguments.len() != if kind == 1 { 3 } else { 2 } {
        return false;
    }
    for (i, a) in node.arguments.iter().enumerate() {
        if node.arguments.iter().take(i).any(|old| old.name == a.name)
            || !matches!(&a.value,FilterValue::Number(ExactNumber::Float32Bits(b)) if number_in_domain(kind,&a.name,*b) || kind==3 && node.operation.schema_epoch==photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH && a.name=="black-level" && matches!(*b,0x3dcc_cccd|0xbdcc_cccd))
        {
            return false;
        }
    }
    true
}
/// Borrowed actual-class/retained-target hint, never a source/profile/render proof.
pub fn candidate(owner: &AccountedDocument, layer: LayerId, node: &photocraft_doc::xcf_filters::FilterNode) -> bool {
    if !owner.is_readonly_filtered_flat_point_storage() || node.id.0 == 0 || !valid_node(node) {
        return false;
    }
    let doc = owner.document();
    let Some(state) = doc.xcf_state.as_ref() else { return false };
    if doc.layers.len() > 4096 || state.filter_stacks.len() > 4096 {
        return false;
    }
    doc.layers.iter().any(|l| l.id == layer)
        && state.filter_stacks.iter().any(|s| s.owner == ItemRef::Layer(layer) && s.nodes.len()<=1024 && s.nodes.iter().any(|n|std::ptr::eq(n,node)))
}
fn context(session: &Session, id: &str, params: &Value) -> Result<()> {
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if !state.doc.accounted().is_some_and(|o| o.is_readonly_filtered_flat_point_storage()) {
        return Err(BackingError::Unsupported.into());
    }
    if !matches!(id, "xcf.filter.set" | "xcf.filter.enable" | "xcf.filter.move" | "xcf.filter.remove")
        || params.get("coalesce").is_some()
        || session.coalesce_request.is_some()
        || state.coalesce.is_some()
        || state.floating.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
        || session.color_restrict.is_some()
        || state.revision == u64::MAX
    {
        return Err(bad(id, "synchronous mixed filter context/revision required"));
    }
    Ok(())
}
fn target<'a>(
    owner: &'a AccountedDocument,
    id: FilterId,
    c: &ReadControl<'_>,
) -> Result<(&'a photocraft_doc::xcf_filters::FilterStack, &'a photocraft_doc::xcf_filters::FilterNode, LayerId,usize)> {
    let state = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    if state.filter_stacks.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    let mut found = None;let mut nodes=0usize;
    for stack in &state.filter_stacks {
        c.check(1)?;
        nodes=nodes.checked_add(stack.nodes.len()).filter(|n|*n<=4096).ok_or(BackingError::Length)?;
        if stack.nodes.len()>1024{return Err(BackingError::Length.into())}
        for (index,node) in stack.nodes.iter().enumerate(){c.check(1)?;
            if node.id == id {
                if found.is_some(){return Err(BackingError::Integrity.into())}
                let ItemRef::Layer(layer)=stack.owner else{return Err(BackingError::Integrity.into())};
                c.check(owner.document().layers.len().checked_add(state.filter_stacks.len()).and_then(|n|n.checked_add(nodes)).ok_or(BackingError::Length)? as u64)?;
                if !candidate(owner,layer,node){return Err(BackingError::Unsupported.into())}
                found=Some((stack,node,layer,index));
            }
        }
    }
    found.ok_or_else(|| bad("xcf.filter.set", "unknown mixed filterId"))
}
fn patch_limit(kind:u8)->usize {match kind {7=>4,10|11=>10,8|9=>2,_=>3}}
fn incoming(kind:u8,argument:&Argument<'_>)->bool {
    if matches!(kind,10|11) && argument.name=="color-model" {
        return argument.domain==Some("gegl:alien-map/color-model") && matches!(argument.value,PointArgumentValue::Enum(v) if v==if kind==10{0}else{1});
    }
    if argument.domain.is_some(){return false}
    if (kind==9 && argument.name=="srgb") || (matches!(kind,10|11) && matches!(argument.name,"cpn-1-keep"|"cpn-2-keep"|"cpn-3-keep")) {
        return matches!(argument.value,PointArgumentValue::Bool(_));
    }
    if kind==0 {return argument.name=="color" && matches!(argument.value,PointArgumentValue::LiteralColor4F32{components,encoding:photocraft_doc::SampleEncoding::Linear|photocraft_doc::SampleEncoding::NonlinearProfile} if components.iter().all(|b|f32::from_bits(*b).is_finite()&&(0.0..=1.0).contains(&f32::from_bits(*b))))}
    matches!(argument.value,PointArgumentValue::Float32Bits(b) if number_in_domain(kind,argument.name,b))
}
fn stage_patches<const N:usize>(owner:&AccountedDocument,fields:&Fields<'_>,kind:u8,layer:photocraft_doc::LayerId,revision:u64,stack_len:usize,id:&str,move_to:Option<usize>,ctl:&ReadControl<'_>)->Result<AccountedDocument>{
    if fields.count>N {return Err(BackingError::Length.into())}
        // This scratch is additional to OLD/history/STAGED and the reply fees.
        // Declare the fee before the real array and retain it through full proof.
        let patch_bytes = std::mem::size_of::<[PointArgumentPatch<'_>; N]>()
            .checked_add(std::mem::size_of::<photocraft_raster::backing::quota::QuotaLease>())
            .and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
        let _patch_fee = owner.quota().reserve(QuotaKind::Index, patch_bytes as u64).map_err(|_| BackingError::Quota)?;
        let mut patches = [PointArgumentPatch { name: "", value: PointArgumentValue::Bool(false) }; N];
        for (slot, argument) in patches.iter_mut().zip(fields.arguments.iter().take(fields.count)) {
            let argument = argument.as_ref().ok_or(BackingError::Integrity)?;
            ctl.check(4096)?;
            if !incoming(kind,argument) {return Err(BackingError::Unsupported.into());}
            *slot = PointArgumentPatch { name: argument.name, value: argument.value };
        }
        let staged = if id=="xcf.filter.remove" {
            owner.try_remove_readonly_filtered_flat_point_filter(PointFilterRemovePatch{layer,filter:fields.filter,expected_stack_revision:revision},ctl)?
        } else if let Some(index)=move_to {
            if index>=stack_len{return Err(bad(id,"index is outside the current owner stack"))}
            owner.try_move_readonly_filtered_flat_point_filter(PointFilterMovePatch{layer,filter:fields.filter,expected_stack_revision:revision,index},ctl)?
        } else { owner.try_with_readonly_filtered_flat_point_filter_with_opacity(
            PointFilterPatch {
                layer,
                filter: fields.filter,
                expected_stack_revision: revision,
                arguments: patches.get(..fields.count).ok_or(BackingError::Integrity)?,
                enabled: fields.enabled,
            },
            fields.opacity,
            ctl,
        )? };
        validate_recipe(&staged, ctl)?;
        Ok(staged)

}

pub(super) struct Request<'a> {
    id: &'static str,
    params: &'a Value,
    fields: Fields<'a>,
    move_to: Option<usize>,
}
impl<'a> Request<'a> {
    pub(super) fn parse(session: &Session, id: &str, params: &'a Value, c: &ReadControl<'_>) -> Result<Self> {
        c.check(32768)?;
        context(session, id, params)?;
        let id = match id {
            "xcf.filter.set" => "xcf.filter.set",
            "xcf.filter.enable" => "xcf.filter.enable",
            "xcf.filter.move" => "xcf.filter.move",
            "xcf.filter.remove" => "xcf.filter.remove",
            _ => return Err(BackingError::Unsupported.into()),
        };
        if id == "xcf.filter.remove" {
            let object=params.as_object().filter(|o|o.len()==1 && o.contains_key("filterId")).ok_or_else(||bad(id,"only filterId supported"))?;
            let text=object.get("filterId").and_then(Value::as_str).filter(|s|!s.is_empty()&&s.len()<=20&&!s.starts_with('0')&&s.bytes().all(|b|b.is_ascii_digit()))
                .ok_or_else(||bad(id,"canonical filterId required"))?;
            let filter=FilterId(text.parse().map_err(|_|bad(id,"filterId overflow"))?);
            return Ok(Self{id,params,fields:Fields{filter,arguments:[None;10],count:0,enabled:None,opacity:None},move_to:None});
        }
        if id == "xcf.filter.move" {
            let object=params.as_object().filter(|o|o.len()==2 && o.keys().all(|k|matches!(k.as_str(),"filterId"|"index")))
                .ok_or_else(||bad(id,"only filterId,index supported"))?;
            let text=object.get("filterId").and_then(Value::as_str).filter(|s|!s.is_empty()&&s.len()<=20&&!s.starts_with('0')&&s.bytes().all(|b|b.is_ascii_digit()))
                .ok_or_else(||bad(id,"canonical filterId required"))?;
            let filter=FilterId(text.parse().map_err(|_|bad(id,"filterId overflow"))?);
            let index=object.get("index").and_then(Value::as_u64).filter(|i|*i<1024).ok_or_else(||bad(id,"index must be a bounded owner-local integer"))?;
            let index=usize::try_from(index).map_err(|_|BackingError::Length)?;
            return Ok(Self{id,params,fields:Fields{filter,arguments:[None;10],count:0,enabled:None,opacity:None},move_to:Some(index)});
        }
        let object = params
            .as_object()
            .filter(|o| o.len() <= 4 && o.keys().all(|k| matches!(k.as_str(), "filterId" | "arguments" | "enabled" | "opacityF64Bits")))
            .ok_or_else(|| bad(id, "only filterId,arguments,enabled,opacityF64Bits supported"))?;
        if object.get("arguments").is_some_and(|v| v.as_array().is_none_or(|a| a.is_empty() || a.len() > 10)) {
            return Err(bad(id, "bounded nonempty argument patches required"));
        }
        let fields = wire_with_literal(id, params, true).ok_or_else(|| bad(id, "typed mixed filter patch required"))?;
        // Extra slots belong only to the exact retained target family;
        // this is not a generic ten-field command permission.
        if fields.count > 3 {
            let owner = session.active().and_then(|s| s.doc.accounted()).ok_or(BackingError::Unsupported)?;
            let (_, node, _, _) = target(owner, fields.filter, c)?;
            let kind=family(node).ok_or(BackingError::Unsupported)?;
            if fields.count>patch_limit(kind) {return Err(bad(id,"patch count exceeds the actual target family"));}
        }
        Ok(Self { id, params, fields, move_to:None })
    }
    pub(super) fn prepare(self, session: &Session, ctl: &ReadControl<'_>) -> Result<Prepared<'a>> {
        context(session, self.id, self.params)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        owner.validate_readonly_filtered_flat_point_storage(ctl)?;
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        validate_recipe(owner, ctl)?;
        let (stack, node, layer,node_index) = target(owner, self.fields.filter, ctl)?;
        let kind = family(node).ok_or(BackingError::Unsupported)?;
        if self.fields.count > patch_limit(kind) {
            return Err(bad(self.id, "argument count exceeds the target family"));
        }
        if (self.move_to.is_some() || self.id=="xcf.filter.remove") && stack.revision==u64::MAX {return Err(BackingError::Length.into())}
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

        let staged = if matches!(kind,10|11) {
            stage_patches::<10>(owner,&self.fields,kind,layer,stack.revision,stack.nodes.len(),self.id,self.move_to,ctl)?
        } else {
            stage_patches::<4>(owner,&self.fields,kind,layer,stack.revision,stack.nodes.len(),self.id,self.move_to,ctl)?
        };
        let updated = !staged.shares_model(owner);
        let mut history = state.history.clone();
        let snapshot = if updated {
            history.max_states = session.prefs().performance.history_states.max(1) as usize;
            history.max_bytes = session.prefs().performance.history_budget_bytes();
            history.set_current_layers_borrowed(view.target(), ctl)?;
            history.try_commit_prepared_accounted(owner, staged, if self.move_to.is_some(){"Move GIMP Stock Filter"}else if self.id=="xcf.filter.remove"{"Remove GIMP Stock Filter"}else{"Edit GIMP Stock Filter"}, view.target(), ctl)?
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
            stack_revision: stack.revision,
            node_index,
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
fn locks(owner: &AccountedDocument, layer: LayerId, visibility: bool, c: &ReadControl<'_>) -> Result<()> {
    let a = owner.accounted_layer_access(layer, c)?.ok_or(EngineError::NoLayer(layer))?;
    if a.locks.all || a.locks.pixels || a.imported_locks.content || visibility && a.imported_locks.visibility {
        return Err(EngineError::Other("the filter owner is locked".into()));
    }
    Ok(())
}
fn validate_recipe(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
    crate::accounted_xcf_open::filtered_flat::validate_current_filtered_flat_storage(owner, c)
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
    stack_revision: u64,
    node_index:usize,
    updated: bool,
    reply: Value,
    _reply_lease: QuotaLease,
    _lease: QuotaLease,
}
impl Prepared<'_> {
    pub(super) fn execute(self, session: &mut Session, ctl: &ReadControl<'_>) -> Result<Value> {
        ctl.check(32768)?;
        context(session, self.request.id, self.request.params)?;
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
        let owner = state.doc.accounted().ok_or(BackingError::Integrity)?;
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        let (stack, _, layer,node_index) = target(owner, self.request.fields.filter, ctl)?;
        if layer != self.layer || stack.revision != self.stack_revision || node_index!=self.node_index {
            return Err(BackingError::Integrity.into());
        }
        locks(owner, self.layer, self.visibility_changed, ctl)?;
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

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_tests.rs"]
mod tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_move_tests.rs"]
mod move_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_remove_tests.rs"]
mod remove_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_current_tests.rs"]
mod current_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_exposure_tests.rs"]
mod exposure_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_invert_tests.rs"]
mod invert_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_opacity_tests.rs"]
mod opacity_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_source_opacity_tests.rs"]
mod source_opacity_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "filtered_flat_affine_tests.rs"]
mod affine_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_stock_rgb_tests.rs"]
mod stock_rgb_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_hsl_tests.rs"]
mod hsl_tests;
