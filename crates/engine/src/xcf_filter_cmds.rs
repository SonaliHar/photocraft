//! Bounded current stock-filter edits. Stored source records remain immutable evidence.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerContent,
    item_identity::{FilterId, ItemRef},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

mod color_balance;
mod group_activation;

const INSPECT: &str = "xcf.filter.inspect";
const SET: &str = "xcf.filter.set";
const ENABLE: &str = "xcf.filter.enable";
const REMOVE: &str = "xcf.filter.remove";
const MOVE: &str = "xcf.filter.move";
fn bad(cmd: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn object<'a>(cmd: &str, value: &'a Value, keys: &[&str]) -> Result<&'a serde_json::Map<String, Value>> {
    let object = value.as_object().ok_or_else(|| bad(cmd, "expected an object"))?;
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(bad(cmd, "unknown filter parameter"));
    }
    Ok(object)
}
pub(crate) fn id(cmd: &str, value: &Value) -> Result<FilterId> {
    let text = value.get("filterId").and_then(Value::as_str).ok_or_else(|| bad(cmd, "filterId must be a nonzero decimal string"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad(cmd, "invalid decimal filterId"));
    }
    let raw = text.parse::<u64>().map_err(|_| bad(cmd, "filterId overflow"))?;
    // Digits-only parsing plus no leading zero is canonical without allocating
    // a temporary formatted String during borrowed dispatch validation.
    if raw == 0 || text.starts_with('0') {
        return Err(bad(cmd, "filterId must use canonical nonzero decimal notation"));
    }
    Ok(FilterId(raw))
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active()
        .and_then(|state| state.doc.xcf_state.as_ref())
        .filter(|state| !state.filter_stacks.is_empty())
        .map(|_| ())
        .ok_or_else(|| "no imported filter state".into())
}
/// Metadata-only edit candidate; checked staged rendering remains the admission authority.
/// Suitable for UI enablement without per-frame document validation or rendering.
pub fn scalar_edit_candidate(node: &FilterNode) -> bool {
    if node.operation.name == "gegl:color-rotate" {
        return node.operation.stored_version == "0:0" && node.operation.schema_epoch == "xcf-22-stock-color-rotate-perceptual-v1";
    }
    if node.operation.name == "gegl:video-degradation" {
        return node.operation.stored_version == "0:0"
            && photocraft_compose::gimp_video_degradation::typed_arguments(&node.arguments).is_ok_and(|operation| {
                photocraft_compose::gimp_video_degradation::current_epoch_parameters(&node.operation.schema_epoch, operation, node.enabled)
            });
    }
    if node.operation.name == "gegl:channel-mixer" {
        return node.operation.stored_version == "0:0" && node.operation.schema_epoch == "xcf-22-stock-channel-mixer-precision-v1";
    }
    if node.operation.name == "gegl:absolute" {
        return node.operation.stored_version == "0:0"
            && node.operation.schema_epoch == "xcf-22-stock-absolute-linear-precision-v1"
            && node.arguments.is_empty();
    }
    if node.operation.name == "gegl:unpremultiply" {
        return node.operation.stored_version == "0:0"
            && node.operation.schema_epoch == photocraft_compose::gimp_unpremultiply::UNPREMULTIPLY_EPOCH
            && node.arguments.is_empty();
    }
    if node.operation.name == "gegl:red-eye-removal" {
        return node.operation.stored_version == "0:0"
            && (node.operation.schema_epoch == "xcf-22-stock-red-eye-removal-precision-v1"
                || node.operation.schema_epoch == photocraft_compose::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH
                    && photocraft_compose::gimp_red_eye::current_epoch_parameters(&node.operation.schema_epoch, &node.arguments));
    }
    if node.operation.name == "gegl:alpha-clip" {
        return node.operation.stored_version == "0:0"
            && matches!(node.operation.schema_epoch.as_str(), "xcf22-stock-alpha-clip-precision-v1" | "xcf22-stock-alpha-clip-finite-precision-v1");
    }
    if node.operation.name == "gegl:component-extract" {
        return node.operation.stored_version == "0:0"
            && matches!(
                node.operation.schema_epoch.as_str(),
                "xcf-22-stock-component-extract-four-precision-v1"
                    | "xcf-22-stock-component-extract-cmyk-precision-v1"
                    | "xcf-22-stock-component-extract-ycbcr-precision-v1"
                    | "xcf-22-stock-component-extract-geometry-v1"
            );
    }
    if node.operation.name == "gegl:mono-mixer" && node.operation.schema_epoch == "xcf-22-stock-mono-mixer-signed-precision-v1" {
        return node.operation.stored_version == "0:0";
    }
    if node.operation.name == "gegl:color-overlay" {
        return node.operation.stored_version == "0:0" && node.operation.schema_epoch == "xcf-22-stock-color-overlay-six-precision-v1";
    }
    if node.operation.name == "gegl:color-to-alpha" {
        return node.operation.stored_version == "0:0" && node.operation.schema_epoch == "xcf-22-stock-color-to-alpha-precision-v1";
    }
    if node.operation.name == "gegl:color-exchange" {
        return node.operation.stored_version == "0:0" && node.operation.schema_epoch == "xcf-22-stock-color-exchange-precision-v1";
    }
    photocraft_algo::gimp::registry::POINT_MEMBERS
        .iter()
        .any(|member| member.operation == node.operation.name && member.version == node.operation.stored_version)
        || (matches!(
            node.operation.name.as_str(),
            "gegl:gaussian-blur"
                | "gegl:box-blur"
                | "gegl:median-blur"
                | "gegl:motion-blur-linear"
                | "gegl:pixelize"
                | "gimp:curves"
                | "gimp:levels"
                | "gimp:color-balance"
        ) && node.operation.stored_version == "0:0")
}
/// Bounded metadata hint for activation controls only. Group parameter editing
/// remains separate; the registered operation performs actual renderer admission.
pub fn activation_edit_candidate(document:&Document,node:&FilterNode)->bool {
    let Some(state)=document.xcf_state.as_ref()else{return false};
    if state.filter_stacks.len()>10_000||state.items.len()>10_000{return false;}
    let mut total=0usize;let mut found=None;
    for stack in &state.filter_stacks {
        let Some(next)=total.checked_add(stack.nodes.len()).filter(|count|*count<=10_000)else{return false};total=next;
        for current in &stack.nodes {
            if current.id==node.id {
                if found.is_some()||!std::ptr::eq(current,node){return false;}
                found=Some(stack);
            }
        }
    }
    let Some(stack)=found else{return false};
    let ItemRef::Layer(owner)=stack.owner else{return false};
    let Ok(locks)=crate::xcf_item_locks::effective_layer_locks(document,owner)else{return false};
    if locks.native.all||locks.native.pixels||locks.imported.content||locks.imported.visibility{return false;}
    let Some(layer)=document.layer(owner)else{return false};
    match &layer.content {
        LayerContent::Raster(_)=>scalar_edit_candidate(node),
        LayerContent::Group(_)=>stack.nodes.len()==1&&node.operation.name=="gegl:gaussian-blur"&&node.operation.stored_version=="0:0"&&node.operation.schema_epoch=="xcf22-isolated-small-auto-v1",
        _=>false,
    }
}
fn reference(item: ItemRef) -> Value {
    let kind = match item {
        ItemRef::Layer(_) => "layer",
        ItemRef::Channel(_) => "channel",
        ItemRef::Mask(_) => "mask",
        ItemRef::Path(_) => "path",
    };
    json!({"kind":kind,"id":item.raw().to_string()})
}
fn inspect(s: &mut Session, params: &Value) -> Result<Value> {
    object(INSPECT, params, &["filterId"])?;
    let id = id(INSPECT, params)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    doc.validate_xcf_state().map_err(|e| bad(INSPECT, e.to_string()))?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad(INSPECT, "missing imported filter state"))?;
    let stack = state.filter_stacks.iter().find(|stack| stack.nodes.iter().any(|node| node.id == id)).ok_or_else(|| bad(INSPECT, "unknown filterId"))?;
    let node = stack.nodes.iter().find(|node| node.id == id).ok_or_else(|| bad(INSPECT, "unknown filterId"))?;
    build_inspect_reply(id, stack, node)
}
/// Shared bounded metadata response only. The resident caller validates its
/// complete XCF state first; accounted callers must separately validate their
/// actual closed storage owner and prepay all response/copy allocations.
pub(crate) fn build_inspect_reply(
    id: FilterId,
    stack: &photocraft_doc::xcf_filters::FilterStack,
    node: &FilterNode,
) -> Result<Value> {
    build_inspect_reply_with_point_colors(id, stack, node, false)
}
/// Exact bounded Point storage metadata only. Its paid caller must validate the
/// real owner, ten-argument shape and response fees first. Color words convey
/// neither an evaluator qualification nor permission to replace an argument.
pub(crate) fn build_point_storage_inspect_reply(
    id: FilterId,
    stack: &photocraft_doc::xcf_filters::FilterStack,
    node: &FilterNode,
) -> Result<Value> {
    build_inspect_reply_with_point_colors(id, stack, node, true)
}
fn build_inspect_reply_with_point_colors(
    id: FilterId,
    stack: &photocraft_doc::xcf_filters::FilterStack,
    node: &FilterNode,
    point_colors: bool,
) -> Result<Value> {
    if [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch].iter().any(|text| text.len() > 4096)
        || node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch.len() > 4096)
        || node.arguments.iter().take(64).any(|argument| argument.name.len() > 128)
        || node.source_display_name.as_ref().is_some_and(|text| text.len() > 4096)
        || node.source_icon.as_ref().is_some_and(|text| text.len() > 4096)
        || node.auxiliary.len() > 64
        || node.auxiliary.iter().any(|(port, _)| port.len() > 128)
    {
        return Err(bad(INSPECT, "filter metadata exceeds bounded inspection reply"));
    }
    // Editable configurations have closed depth/count limits. Other forms remain inspectable
    // as metadata without recursively serializing hostile, arbitrarily large configuration trees.
    let mut stored_color_bytes = 0usize;
    for argument in node.arguments.iter().take(64) {
        if let FilterValue::StoredColor(storage) = &argument.value {
            let (layout, samples, opaque_tail) = (&storage.layout, &storage.samples, &storage.opaque_tail);
            let encoded = samples
                .len()
                .checked_add(opaque_tail.len())
                .and_then(|n| n.checked_mul(2))
                .and_then(|n| layout.len().checked_mul(6).and_then(|text| n.checked_add(text)))
                .and_then(|n| n.checked_add(256))
                .ok_or_else(|| bad(INSPECT, "StoredColor inspection size overflow"))?;
            stored_color_bytes =
                stored_color_bytes.checked_add(encoded).filter(|n| *n <= 16 * 1024).ok_or_else(|| bad(INSPECT, "StoredColor exact inspection exceeds16KiB"))?;
        }
    }
    let arguments: Vec<_> = node
        .arguments
        .iter()
        .take(64)
        .map(|argument| -> Result<Value> {
            let value = match &argument.value {
                FilterValue::StoredColor(storage) => json!({"value":crate::inspect_xcf_typed::stored_color_inspection(&storage.layout,&storage.samples,&storage.opaque_tail,&Interrupt::NONE)?,"editable":false}),
                FilterValue::Number(ExactNumber::Float64Bits(bits)) if point_colors => json!({"value":{"Number":{"Float64Bits":format!("0x{bits:016x}")}}}),
                FilterValue::Number(_) | FilterValue::Bool(_) => json!({"value":argument.value}),
                FilterValue::Color { components, encoding, profile: None }
                    if semi_flatten_literal_node(node) && matches!(encoding,photocraft_doc::SampleEncoding::Linear|photocraft_doc::SampleEncoding::NonlinearProfile) && components.len()==4 && components.iter().all(|v|matches!(v,ExactNumber::Float32Bits(_))) => json!({"value":argument.value}),
                FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None }
                    if point_colors && components.len() == 4
                        && components.iter().all(|component| matches!(component, ExactNumber::Float32Bits(_))) => json!({"value":argument.value}),
                FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None }
                    if matches!(node.operation.name.as_str(), "gegl:color-exchange" | "gegl:color-to-alpha") && components.len() == 4
                        && components.iter().all(|component| matches!(component, ExactNumber::Float32Bits(bits) if f32::from_bits(*bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(*bits)))) => json!({"value":argument.value}),
                FilterValue::Enum { domain, .. } if domain.len() <= 256 => json!({"value":argument.value}),
                FilterValue::String(text) if text.len() <= 4096 => json!({"value":argument.value}),
                FilterValue::Config(entries) => config_wire(&node.operation.name, &argument.value)
                    .map(|value| json!({"value":value,"structuredBits":"canonical hexadecimal64"}))
                    .unwrap_or_else(|| json!({"unresolvedStructuredValue":"Config","members":entries.len()})),
                FilterValue::Curve { free, points_f64_bits, samples_f64_bits, .. } => {
                    json!({"unresolvedStructuredValue":"Curve","free":free,"points":points_f64_bits.len(),"samples":samples_f64_bits.len()})
                }
                FilterValue::Graph(graph) => json!({"unresolvedStructuredValue":"Graph","nodes":graph.nodes.len(),"edges":graph.edges.len()}),
                FilterValue::Resource(resource) => json!({"resourceId":resource.0.to_string()}),
                _ => json!({"unresolvedStructuredValue":"outside bounded scalar inspection"}),
            };
            Ok(json!({"name":argument.name,"state":value}))
        })
        .collect::<Result<Vec<_>>>()?;
    let auxiliary: Vec<_> = node.auxiliary.iter().map(|(port, target)| json!({"port":port,"target":reference(*target)})).collect();
    Ok(json!({"filterId":id.0.to_string(),"owner":reference(stack.owner),"revision":stack.revision,
        "operation":node.operation,"enabled":node.enabled,"opacityF64Bits":node.opacity_f64_bits,"opacityF64Hex":format!("0x{:016x}",node.opacity_f64_bits),
        "sourceDisplayName":node.source_display_name,"sourceIcon":node.source_icon,"mask":node.mask.map(reference),"auxiliary":auxiliary,
        "clipping":node.source_clipping,"region":node.source_region,"composition":node.composition,
        "arguments":arguments,"argumentCount":node.arguments.len(),"argumentsTruncated":node.arguments.len()>64,"scalarEditCandidate":scalar_edit_candidate(node),
        "admission":"staged checked renderer required; metadata inspection does not qualify activation"}))
}
fn value(input: &Value) -> Result<FilterValue> {
    let outer = object(SET, input, &["Number", "Bool", "Enum", "Color"])?;
    if outer.len() != 1 {
        return Err(bad(SET, "one exact scalar typed value required"));
    }
    if let Some(color) = outer.get("Color") {
        let fields = object(SET, color, &["components", "encoding", "profile"])?;
        if fields.len() != 3
            || fields.get("encoding").and_then(Value::as_str) != Some("nonlinear_profile")
            || !fields.get("profile").is_some_and(Value::is_null)
        {
            return Err(bad(SET, "literal Color requires exact NonlinearProfile encoding and null profile"));
        }
        let components = fields
            .get("components")
            .and_then(Value::as_array)
            .filter(|values| values.len() == 4)
            .ok_or_else(|| bad(SET, "literal Color requires four exact Float32Bits components"))?;
        let mut output = Vec::new();
        output.try_reserve_exact(4).map_err(|_| bad(SET, "literal Color allocation failed"))?;
        for component in components {
            let fields = object(SET, component, &["Float32Bits"])?;
            let bits = fields
                .get("Float32Bits")
                .and_then(Value::as_u64)
                .and_then(|bits| u32::try_from(bits).ok())
                .filter(|bits| {
                    let value = f32::from_bits(*bits);
                    value.is_finite() && (0.0..=1.0).contains(&value)
                })
                .ok_or_else(|| bad(SET, "literal Color component requires finite unit Float32Bits"))?;
            output.push(ExactNumber::Float32Bits(bits));
        }
        return Ok(FilterValue::Color { components: output, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None });
    }
    if let Some(value) = outer.get("Number") {
        let inner = object(SET, value, &["Float32Bits", "Signed"])?;
        if inner.len() != 1 {
            return Err(bad(SET, "one exact stored numeric type required"));
        }
        if let Some(signed) = inner.get("Signed") {
            let signed = object(SET, signed, &["width", "value"])?;
            if signed.get("width").and_then(Value::as_u64) != Some(32) {
                return Err(bad(SET, "integer parameter requires stored signed32 width"));
            }
            let value = signed
                .get("value")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| bad(SET, "integer parameter requires a signed32 value"))?;
            return Ok(FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(value) }));
        }
        let bits =
            inner.get("Float32Bits").and_then(Value::as_u64).and_then(|bits| u32::try_from(bits).ok()).ok_or_else(|| bad(SET, "exact Float32Bits required"))?;
        if !f32::from_bits(bits).is_finite() {
            return Err(bad(SET, "nonfinite parameter bits"));
        }
        return Ok(FilterValue::Number(ExactNumber::Float32Bits(bits)));
    }
    if let Some(value) = outer.get("Bool") {
        return value.as_bool().map(FilterValue::Bool).ok_or_else(|| bad(SET, "Bool requires a boolean"));
    }
    let inner = object(SET, outer.get("Enum").ok_or_else(|| bad(SET, "exact scalar typed value required"))?, &["domain", "value"])?;
    let domain = inner
        .get("domain")
        .and_then(Value::as_str)
        .filter(|domain| !domain.is_empty() && domain.len() <= 256 && !domain.contains('\0'))
        .ok_or_else(|| bad(SET, "bounded exact enum domain required"))?;
    let value =
        inner.get("value").and_then(Value::as_i64).and_then(|value| i32::try_from(value).ok()).ok_or_else(|| bad(SET, "enum requires a signed32 value"))?;
    Ok(FilterValue::Enum { domain: domain.into(), value })
}
const CHANNELS: [&str; 5] = ["value", "red", "green", "blue", "alpha"];
const LEVEL_FIELDS: [&str; 5] = ["low-input", "high-input", "gamma", "low-output", "high-output"];
fn bits64(value: &Value) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|text| text.len() == 18 && text.starts_with("0x"))
        .ok_or_else(|| bad(SET, "Float64 bits require a fixed16 lowercase hexadecimal string"))?;
    let digits = text.get(2..).ok_or_else(|| bad(SET, "invalid hexadecimal bits"))?;
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
        return Err(bad(SET, "noncanonical hexadecimal bits"));
    }
    let bits = u64::from_str_radix(digits, 16).map_err(|_| bad(SET, "invalid hexadecimal bits"))?;
    if !f64::from_bits(bits).is_finite() {
        return Err(bad(SET, "nonfinite configuration bits"));
    }
    Ok(bits)
}
fn configuration(operation: &str, input: &Value) -> Result<FilterValue> {
    if operation == "gimp:color-balance" {
        return color_balance::decode(input);
    }
    let outer = object(SET, input, &["Config"])?;
    let channels = outer
        .get("Config")
        .and_then(Value::as_array)
        .filter(|channels| channels.len() == 5)
        .ok_or_else(|| bad(SET, "configuration requires five ordered selectors"))?;
    let mut decoded = Vec::new();
    decoded.try_reserve_exact(5).map_err(|_| bad(SET, "configuration allocation denied"))?;
    for (channel, expected) in channels.iter().zip(CHANNELS) {
        let channel = object(SET, channel, &["name", "value"])?;
        if channel.get("name").and_then(Value::as_str) != Some(expected) {
            return Err(bad(SET, "unknown, duplicate or unordered selector"));
        }
        let value = channel.get("value").ok_or_else(|| bad(SET, "selector value missing"))?;
        let value = match operation {
            "gimp:curves" => {
                let outer = object(SET, value, &["Curve"])?;
                let curve = object(
                    SET,
                    outer.get("Curve").ok_or_else(|| bad(SET, "FREE curve required"))?,
                    &["free", "points_f64_bits", "point_types", "samples_f64_bits"],
                )?;
                if curve.get("free").and_then(Value::as_bool) != Some(true)
                    || !curve.get("points_f64_bits").and_then(Value::as_array).is_some_and(Vec::is_empty)
                    || !curve.get("point_types").and_then(Value::as_array).is_some_and(Vec::is_empty)
                {
                    return Err(bad(SET, "only exact stored FREE256 curves are editable"));
                }
                let samples = curve
                    .get("samples_f64_bits")
                    .and_then(Value::as_array)
                    .filter(|samples| samples.len() == 256)
                    .ok_or_else(|| bad(SET, "FREE curve requires exactly256 samples"))?;
                let mut bits = Vec::new();
                bits.try_reserve_exact(256).map_err(|_| bad(SET, "curve allocation denied"))?;
                for sample in samples {
                    let sample = bits64(sample)?;
                    if !(0.0..=1.0).contains(&f64::from_bits(sample)) {
                        return Err(bad(SET, "FREE sample outside0..1"));
                    }
                    bits.push(sample);
                }
                FilterValue::Curve { free: true, points_f64_bits: Vec::new(), point_types: Vec::new(), samples_f64_bits: bits }
            }
            "gimp:levels" => {
                let outer = object(SET, value, &["Config"])?;
                let fields = outer
                    .get("Config")
                    .and_then(Value::as_array)
                    .filter(|fields| fields.len() == 5)
                    .ok_or_else(|| bad(SET, "Levels requires five ordered fields"))?;
                let mut out = Vec::new();
                out.try_reserve_exact(5).map_err(|_| bad(SET, "Levels allocation denied"))?;
                let mut numbers = [0.0; 5];
                for ((field, expected), decoded_number) in fields.iter().zip(LEVEL_FIELDS).zip(numbers.iter_mut()) {
                    let field = object(SET, field, &["name", "value"])?;
                    if field.get("name").and_then(Value::as_str) != Some(expected) {
                        return Err(bad(SET, "unknown, duplicate or unordered Levels field"));
                    }
                    let number = object(SET, field.get("value").ok_or_else(|| bad(SET, "Levels value missing"))?, &["Number"])?;
                    let number = object(SET, number.get("Number").ok_or_else(|| bad(SET, "Levels exact number required"))?, &["Float64Bits"])?;
                    let bits = bits64(number.get("Float64Bits").ok_or_else(|| bad(SET, "Levels Float64Bits missing"))?)?;
                    *decoded_number = f64::from_bits(bits);
                    out.push(FilterArgument { name: expected.into(), value: FilterValue::Number(ExactNumber::Float64Bits(bits)) });
                }
                let [low_input, high_input, gamma, low_output, high_output] = numbers;
                let channel = photocraft_algo::gimp::levels::LevelsChannel { low_input, high_input, gamma, low_output, high_output };
                photocraft_algo::gimp::levels::LevelsConfiguration { channels: [channel; 5], clamp_input: false, clamp_output: false }
                    .validate()
                    .map_err(|_| bad(SET, "Levels exact field domain"))?;
                FilterValue::Config(out)
            }
            _ => return Err(bad(SET, "structured operation is inspection-only")),
        };
        decoded.push(FilterArgument { name: expected.into(), value });
    }
    Ok(FilterValue::Config(decoded))
}
fn config_wire(operation: &str, input: &FilterValue) -> Option<Value> {
    if operation == "gimp:color-balance" {
        return color_balance::wire(input);
    }
    let FilterValue::Config(channels) = input else { return None };
    if channels.len() != 5 {
        return None;
    }
    let mut output = Vec::new();
    for (channel, expected) in channels.iter().zip(CHANNELS) {
        if channel.name != expected {
            return None;
        }
        let value = match (operation, &channel.value) {
            ("gimp:curves", FilterValue::Curve { free: true, points_f64_bits, point_types, samples_f64_bits })
                if points_f64_bits.is_empty()
                    && point_types.is_empty()
                    && samples_f64_bits.len() == 256
                    && samples_f64_bits.iter().all(|bits| f64::from_bits(*bits).is_finite() && (0.0..=1.0).contains(&f64::from_bits(*bits))) =>
            {
                json!({"Curve":{"free":true,"points_f64_bits":[],"point_types":[],"samples_f64_bits":samples_f64_bits.iter().map(|bits|format!("0x{bits:016x}")).collect::<Vec<_>>()}})
            }
            ("gimp:levels", FilterValue::Config(fields)) if fields.len() == 5 => {
                let mut output = Vec::new();
                for (field, expected) in fields.iter().zip(LEVEL_FIELDS) {
                    let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field.value else { return None };
                    if field.name != expected || !f64::from_bits(bits).is_finite() {
                        return None;
                    }
                    output.push(json!({"name":expected,"value":{"Number":{"Float64Bits":format!("0x{bits:016x}")}}}));
                }
                json!({"Config":output})
            }
            _ => return None,
        };
        output.push(json!({"name":expected,"value":value}));
    }
    Some(json!({"Config":output}))
}
/// Only the paid Point adapter may use this borrowed conversion seam. The
/// caller MUST first check its closed bounded wire and reserve a genuine same-
/// quota temporary envelope, then census/reconcile returned actual capacities
/// before model/renderer work. This helper itself grants no storage authority.
pub(crate) fn paid_point_patches(params:&Value,operation:&str)->Result<Vec<FilterArgument>> { patches(params,operation) }
fn patches(params: &Value, operation: &str) -> Result<Vec<FilterArgument>> {
    let Some(input) = params.get("arguments") else {
        return Ok(Vec::new());
    };
    let input =
        input.as_array().filter(|input| !input.is_empty() && input.len() <= 64).ok_or_else(|| bad(SET, "arguments requires1..64 named scalar patches"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(input.len()).map_err(|_| bad(SET, "argument allocation denied"))?;
    for input in input {
        let fields = object(SET, input, &["name", "value"])?;
        let name = fields
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty() && name.len() <= 128 && !name.contains('\0'))
            .ok_or_else(|| bad(SET, "bounded parameter name required"))?;
        if out.iter().any(|argument: &FilterArgument| argument.name == name) {
            return Err(bad(SET, "duplicate parameter patch"));
        }
        let input = fields.get("value").ok_or_else(|| bad(SET, "parameter value required"))?;
        let value = if ((operation == "gegl:mono-mixer" && matches!(name, "red" | "green" | "blue"))
            || (operation == "gegl:channel-mixer"
                && matches!(name, "rr-gain" | "rg-gain" | "rb-gain" | "gr-gain" | "gg-gain" | "gb-gain" | "br-gain" | "bg-gain" | "bb-gain")))
            && input.get("Number").is_some_and(|n| n.get("Float64Bits").is_some())
        {
            let outer = object(SET, input, &["Number"])?;
            let number = object(SET, outer.get("Number").ok_or_else(|| bad(SET, "Mono exact number missing"))?, &["Float64Bits"])?;
            FilterValue::Number(ExactNumber::Float64Bits(bits64(number.get("Float64Bits").ok_or_else(|| bad(SET, "Mono Double bits missing"))?)?))
        } else if name == "config" && matches!(operation, "gimp:curves" | "gimp:levels" | "gimp:color-balance") {
            configuration(operation, input)?
        } else {
            value(input)?
        };
        out.push(FilterArgument { name: name.into(), value });
    }
    Ok(out)
}
fn same_parameter_type(node: &FilterNode, name: &str, current: &FilterValue, replacement: &FilterValue) -> bool {
    let floating = |value: &FilterValue| matches!(value, FilterValue::Number(ExactNumber::Float32Bits(_) | ExactNumber::Float64Bits(_)));
    same_type(current, replacement)
        || (node.operation.name == "gegl:channel-mixer"
            && node.operation.stored_version == "0:0"
            && node.operation.schema_epoch == "xcf-22-stock-channel-mixer-precision-v1"
            && matches!(name, "rr-gain" | "rg-gain" | "rb-gain" | "gr-gain" | "gg-gain" | "gb-gain" | "br-gain" | "bg-gain" | "bb-gain")
            && floating(current)
            && floating(replacement))
        || (node.operation.name == "gegl:mono-mixer"
            && node.operation.stored_version == "0:0"
            && node.operation.schema_epoch == "xcf-22-stock-mono-mixer-signed-precision-v1"
            && matches!(name, "red" | "green" | "blue")
            && floating(current)
            && floating(replacement))
}
/// Complete scalar configuration binding for the narrow template-insert lane.
pub(crate) fn template_arguments_compatible(node: &FilterNode, arguments: &[FilterArgument]) -> bool {
    arguments.len() <= 64
        && arguments.len() == node.arguments.len()
        && arguments.iter().all(|replacement| {
            node.arguments
                .iter()
                .find(|current| current.name == replacement.name)
                .is_some_and(|current| same_parameter_type(node, &current.name, &current.value, &replacement.value))
        })
}
fn same_type(current: &FilterValue, replacement: &FilterValue) -> bool {
    match (current, replacement) {
        (FilterValue::Number(ExactNumber::Float32Bits(_)), FilterValue::Number(ExactNumber::Float32Bits(_))) | (FilterValue::Bool(_), FilterValue::Bool(_)) => {
            true
        }
        (FilterValue::Number(ExactNumber::Signed { width: 32, .. }), FilterValue::Number(ExactNumber::Signed { width: 32, .. })) => true,
        (FilterValue::Config(_), FilterValue::Config(_)) => true,
        (FilterValue::Enum { domain: a, .. }, FilterValue::Enum { domain: b, .. }) => a == b,
        (FilterValue::Color { components: a, encoding: ae, profile: ap }, FilterValue::Color { components: b, encoding: be, profile: bp }) => {
            ae == be && ap == bp && a.len() == 4 && b.len() == 4 && a.iter().chain(b).all(|component| matches!(component, ExactNumber::Float32Bits(_)))
        }
        _ => false,
    }
}
/// Immutable, cancellable validation seam for command staging and future worker dispatch.
pub fn validate_current_pipeline_with_interrupt(doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    validate_current_pipeline_font_input(doc, None, control)
}
/// Reuses an already admitted runtime font for qualification and every render
/// band. The original document/resource font authority gates remain unchanged.
/// This is borrowed runtime input, never a serialized grant or locator lookup.
pub fn validate_current_pipeline_with_authorized_font(
    doc: &Document,
    font: &photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont<'_>,
    control: &Interrupt<'_>,
) -> Result<()> {
    validate_current_pipeline_font_input(doc, Some(font), control)
}
fn validate_current_pipeline_font_input(
    doc: &Document,
    font: Option<&photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont<'_>>,
    control: &Interrupt<'_>,
) -> Result<()> {
    doc.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, e.to_string()) })?;
    let bounds = doc.bounds();
    let pixels = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(|| bad(SET, "canvas work overflow"))?;
    if pixels == 0 || pixels > 36_000_000 {
        return Err(bad(SET, "filter validation requires a nonempty canvas at most36MP"));
    }
    // Carry the admitted font receipt into this validation envelope. Reusing
    // the handle does not perform another hash or create a fresh font budget.
    let mut sampled = font.map(|font| font.requirements().work_units).unwrap_or(0);
    let mut resident = font.map(|font| font.requirements().peak_bytes).unwrap_or(0);
    if sampled > 36_000_000 * 512 || resident > 512 * 1024 * 1024 {
        return Err(bad(SET, "authorized font exceeds filter validation envelope"));
    }
    let prepared = match font {
        Some(font) => photocraft_compose::gimp_render::PreparedRenderer::new_with_authorized_font(doc, font, control),
        None => photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control),
    }
    .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, format!("unqualified filter pipeline: {e}")) })?;
    let mut poll = |work| {
        use photocraft_cms::WideWork;
        use photocraft_compose::gimp_composite::CompositionError;
        if control.cancelled() {
            return Err(photocraft_raster::RasterInputError::Cancelled.into());
        }
        match work {
            WideWork::SampleUnits(units) => {
                sampled = sampled.checked_add(units).ok_or(CompositionError::Invalid("filter work overflow"))?;
                if sampled > 36_000_000 * 512 {
                    return Err(CompositionError::Invalid("filter command work limit"));
                }
            }
            WideWork::ReserveBytes(bytes) => {
                resident = resident.checked_add(bytes).ok_or(CompositionError::Invalid("filter resident overflow"))?;
                if resident > 512 * 1024 * 1024 {
                    return Err(CompositionError::Invalid("filter command resident limit"));
                }
            }
            WideWork::ReleaseBytes(bytes) => {
                resident = resident.checked_sub(bytes).ok_or(CompositionError::Invalid("filter resident release exceeds reservation"))?
            }
        }
        Ok(())
    };
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let y1 = y.saturating_add(128).min(bounds.y1);
        let mut x = bounds.x0;
        while x < bounds.x1 {
            let x1 = x.saturating_add(128).min(bounds.x1);
            let rendered = prepared
                .render_with_interrupt(photocraft_doc::Rect::new(x, y, x1, y1), &mut poll, control)
                .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, format!("unqualified filter samples: {e}")) })?;
            let released =
                u64::try_from(rendered.px.len()).ok().and_then(|pixels| pixels.checked_mul(32)).ok_or_else(|| bad(SET, "output residency overflow"))?;
            drop(rendered);
            poll(photocraft_cms::WideWork::ReleaseBytes(released)).map_err(|e| bad(SET, e.to_string()))?;
            x = x1;
        }
        y = y1;
    }
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(())
}
// Borrowed closed wire census before dispatcher JSON/model copying. This is
// schema/size admission; operation-specific value domains remain staged below.
fn set_wire(value:&Value,depth:usize,nodes:&mut usize,bytes:&mut usize)->Result<()> {
    if depth>16 {return Err(bad(SET,"filter wire depth limit"));}
    *nodes=nodes.checked_add(1).filter(|n|*n<=8192).ok_or_else(||bad(SET,"filter wire node limit"))?;
    let amount=match value {Value::String(text)=>text.len().checked_mul(6).and_then(|n|n.checked_add(2)),Value::Object(object)=>{
        if object.len()>8 {return Err(bad(SET,"filter wire record limit"));}
        Some(2)
    },Value::Array(values)=>{if values.len()>256 {return Err(bad(SET,"filter wire array limit"));}Some(values.len()+2)},_=>Some(32)}.ok_or_else(||bad(SET,"filter wire size overflow"))?;
    *bytes=bytes.checked_add(amount).filter(|n|*n<=256<<10).ok_or_else(||bad(SET,"filter wire byte limit"))?;
    match value {
        Value::Array(values)=>for value in values {set_wire(value,depth+1,nodes,bytes)?;},
        Value::Object(object)=>for (key,value) in object {
            if key.len()>128 {return Err(bad(SET,"filter wire key limit"));}
            *bytes=bytes.checked_add(key.len().checked_mul(6).ok_or_else(||bad(SET,"filter wire size overflow"))?).filter(|n|*n<=256<<10).ok_or_else(||bad(SET,"filter wire byte limit"))?;
            set_wire(value,depth+1,nodes,bytes)?;
        },_=>{}
    }
    Ok(())
}
pub(crate) fn hue_chroma_node(node:&photocraft_doc::xcf_filters::FilterNode)->bool{
    node.operation.name=="gegl:hue-chroma" && node.operation.stored_version=="0:0" && node.operation.schema_epoch==photocraft_compose::HUE_CHROMA_EPOCH && node.composition.as_ref().is_some_and(|p|p.compatibility_epoch==photocraft_compose::HUE_CHROMA_EPOCH)
}
pub(crate) fn hue_chroma_owner(owner:&photocraft_doc::accounted_document::AccountedDocument)->bool{
    owner.is_readonly_point_storage() && owner.document().xcf_state.as_ref().is_some_and(|state|matches!(state.filter_stacks.as_slice(),[stack] if matches!(stack.nodes.as_slice(),[node] if hue_chroma_node(node))))
}
pub(crate) fn semi_flatten_literal_node(node:&photocraft_doc::xcf_filters::FilterNode)->bool{
    node.operation.name=="gimp:semi-flatten" && node.operation.stored_version=="0:0" && node.operation.schema_epoch==photocraft_compose::SEMI_FLATTEN_EPOCH && node.composition.as_ref().is_some_and(|p|p.compatibility_epoch==photocraft_compose::SEMI_FLATTEN_EPOCH)
}
pub(crate) fn semi_flatten_literal_owner(owner:&photocraft_doc::accounted_document::AccountedDocument)->bool{
    owner.is_readonly_point_storage() && owner.document().xcf_state.as_ref().is_some_and(|state|matches!(state.filter_stacks.as_slice(),[stack] if matches!(stack.nodes.as_slice(),[node] if semi_flatten_literal_node(node))))
}
fn borrowed_value(input:&Value,structured:bool,literal:bool)->Result<()> {
    let outer=object(SET,input,&["Number","Bool","Enum","Color","Config","Curve"])?;
    if outer.len()!=1 {return Err(bad(SET,"one exact typed value required"));}
    if let Some(config)=outer.get("Config") {
        if !structured {return Err(bad(SET,"structured operation is inspection-only"));}
        let entries=config.as_array().filter(|entries|entries.len()<=5).ok_or_else(||bad(SET,"closed configuration record limit"))?;
        for (index,entry) in entries.iter().enumerate() {
            let fields=object(SET,entry,&["name","value"])?;
            if fields.len()!=2 {return Err(bad(SET,"configuration entry requires name/value"));}
            let name=fields.get("name").and_then(Value::as_str).filter(|name|!name.is_empty() && name.len()<=128 && !name.contains('\0')).ok_or_else(||bad(SET,"bounded configuration name required"))?;
            if entries.iter().take(index).any(|entry|entry.get("name").and_then(Value::as_str)==Some(name)) {return Err(bad(SET,"duplicate configuration name"));}
            borrowed_value(fields.get("value").ok_or_else(||bad(SET,"configuration value missing"))?,true,false)?;
        }
        return Ok(());
    }
    if let Some(curve)=outer.get("Curve") {
        if !structured {return Err(bad(SET,"curve requires closed config"));}
        let fields=object(SET,curve,&["free","points_f64_bits","point_types","samples_f64_bits"])?;
        if fields.len()!=4 || fields.get("free").and_then(Value::as_bool)!=Some(true)
            || !fields.get("points_f64_bits").and_then(Value::as_array).is_some_and(Vec::is_empty)
            || !fields.get("point_types").and_then(Value::as_array).is_some_and(Vec::is_empty) {return Err(bad(SET,"only FREE256 curves editable"));}
        let samples=fields.get("samples_f64_bits").and_then(Value::as_array).filter(|samples|samples.len()==256).ok_or_else(||bad(SET,"FREE256 samples required"))?;
        for sample in samples {if !(0.0..=1.0).contains(&f64::from_bits(bits64(sample)?)) {return Err(bad(SET,"curve sample range"));}}
        return Ok(());
    }
    if let Some(number)=outer.get("Number") {
        let fields=object(SET,number,&["Float32Bits","Float64Bits","Signed"])?;
        if fields.len()!=1 {return Err(bad(SET,"one exact numeric type required"));}
        if let Some(value)=fields.get("Float64Bits") {bits64(value)?;return Ok(());}
        if let Some(value)=fields.get("Float32Bits") {
            let bits=value.as_u64().and_then(|bits|u32::try_from(bits).ok()).ok_or_else(||bad(SET,"exact Float32Bits required"))?;
            if !f32::from_bits(bits).is_finite() {return Err(bad(SET,"nonfinite parameter bits"));}return Ok(());
        }
        let fields=object(SET,fields.get("Signed").ok_or_else(||bad(SET,"numeric type required"))?,&["width","value"])?;
        if fields.len()!=2 || fields.get("width").and_then(Value::as_u64)!=Some(32)
            || fields.get("value").and_then(Value::as_i64).and_then(|value|i32::try_from(value).ok()).is_none() {return Err(bad(SET,"signed32 value required"));}
        return Ok(());
    }
    if let Some(value)=outer.get("Bool") {if !value.is_boolean() {return Err(bad(SET,"Bool requires boolean"));}return Ok(());}
    if let Some(value)=outer.get("Enum") {
        let fields=object(SET,value,&["domain","value"])?;
        if fields.len()!=2 || !fields.get("domain").and_then(Value::as_str).is_some_and(|text|!text.is_empty() && text.len()<=256 && !text.contains('\0'))
            || fields.get("value").and_then(Value::as_i64).and_then(|value|i32::try_from(value).ok()).is_none() {return Err(bad(SET,"exact bounded enum required"));}
        return Ok(());
    }
    let color=object(SET,outer.get("Color").ok_or_else(||bad(SET,"typed scalar required"))?,&["components","encoding","profile"])?;
    if color.len()!=3 || (!matches!(color.get("encoding").and_then(Value::as_str),Some("nonlinear_profile")) && !(literal && color.get("encoding").and_then(Value::as_str)==Some("linear"))) || !color.get("profile").is_some_and(Value::is_null) {return Err(bad(SET,"literal Color encoding/profile"));}
    let components=color.get("components").and_then(Value::as_array).filter(|values|values.len()==4).ok_or_else(||bad(SET,"four Color components required"))?;
    for value in components {
        let fields=object(SET,value,&["Float32Bits"])?;
        let bits=fields.get("Float32Bits").and_then(Value::as_u64).and_then(|bits|u32::try_from(bits).ok()).ok_or_else(||bad(SET,"Color Float32Bits required"))?;
        if fields.len()!=1 || !f32::from_bits(bits).is_finite() || !(0.0..=1.0).contains(&f32::from_bits(bits)) {return Err(bad(SET,"Color component range"));}
    }
    Ok(())
}
/// Root dispatch calls before normalization, any JSON clone or implicit float drop.
pub(crate) fn pre_dispatch_set_change(session:&Session,cmd:&str,params:&Value,background:bool)->Result<()> {
    if !matches!(cmd,SET|ENABLE) {return Err(bad(cmd,"not a SET/ENABLE lane"));}
    let active=session.active().ok_or(EngineError::NoDocument)?;
    if params.as_object().is_some_and(|object|object.contains_key("coalesce")) || session.coalesce_request.is_some() {return Err(bad(cmd,"coalescing refused"));}
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {return Err(bad(cmd,"floating preview/attachment refused"));}
    if background || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() || active.revision==u64::MAX {return Err(bad(cmd,"synchronous context/revision required"));}
    object(cmd,params,if cmd==ENABLE {&["filterId","enabled"]} else {&["filterId","enabled","arguments","opacityF64Bits"]})?;
    id(cmd,params)?;
    if let Some(enabled)=params.get("enabled") {if !enabled.is_boolean() {return Err(bad(cmd,"enabled must be boolean"));}}
    else if cmd==ENABLE {return Err(bad(cmd,"enabled required"));}
    if let Some(opacity)=params.get("opacityF64Bits") {if !(0.0..=1.0).contains(&f64::from_bits(bits64(opacity)?)) {return Err(bad(cmd,"opacity outside 0..1"));}}
    set_wire(params,0,&mut 0,&mut 0)?;
    if let Some(arguments)=params.get("arguments") {
        let arguments=arguments.as_array().filter(|values|values.len()<=64).ok_or_else(||bad(cmd,"at most64 patches required"))?;
        for (index,argument) in arguments.iter().enumerate() {
            let fields=object(cmd,argument,&["name","value"])?;
            let name=fields.get("name").and_then(Value::as_str).filter(|text|!text.is_empty() && text.len()<=128 && !text.contains('\0')).ok_or_else(||bad(cmd,"bounded argument name required"))?;
            if fields.len()!=2 || arguments.iter().take(index).any(|argument|argument.get("name").and_then(Value::as_str)==Some(name)) {return Err(bad(cmd,"duplicate/invalid patch"));}
            borrowed_value(fields.get("value").ok_or_else(||bad(cmd,"argument value required"))?,name=="config",name=="color" && active.doc.accounted().is_some_and(semi_flatten_literal_owner))?;
        }
    }
    if cmd==SET && params.get("enabled").is_none() && params.get("opacityF64Bits").is_none()
        && params.get("arguments").and_then(Value::as_array).is_none_or(Vec::is_empty) {return Err(bad(cmd,"an activation or patch required"));}
    Ok(())
}
fn set_clone_headroom(snapshot:&crate::DocumentSnapshot,control:&Interrupt<'_>)->Result<()> {
    // Existing native structural/payload limits count input without encoding a
    // document copy. Raster sparse-map clone cost remains a separate envelope.
    photocraft_io::retained_native_input_bytes(snapshot,photocraft_io::NativeSaveLimits::default().preparation,control)
        .map_err(|error|if control.cancelled() {EngineError::Cancelled} else {bad(SET,format!("filter retained input headroom: {error}"))})?;
    let footprint=snapshot.resident_raster_footprint_with(control)?;
    if footprint.clone_metadata_bytes.checked_mul(3).and_then(|n|n.checked_add(footprint.retained_bytes))
        .is_none_or(|n|n>photocraft_raster::MAX_CHECKED_RASTER_BYTES) {return Err(bad(SET,"filter raster COW headroom"));}
    control.check().map_err(|_|EngineError::Cancelled)
}
fn final_set_context(session:&Session,snapshot:&crate::DocumentSnapshot,revision:u64,params:&Value,control:&Interrupt<'_>)->Result<()> {
    control.check().map_err(|_|EngineError::Cancelled)?;
    pre_dispatch_set_change(session,SET,params,false)?;
    let active=session.active().ok_or(EngineError::NoDocument)?;
    if active.revision!=revision || !active.doc.shares_state(snapshot) {return Err(bad(SET,"filter target changed before commit"));}
    Ok(())
}
fn set(s: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(s, params, &Interrupt::NONE)
}
/// Checked staging and a final Session publication cancellation fence.
pub fn set_with_interrupt(s: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    pre_dispatch_set_change(s,SET,params,false)?;
    object(SET, params, &["filterId", "enabled", "arguments", "opacityF64Bits"])?;
    let id = id(SET, params)?;
    let enabled = params.get("enabled").map(|value| value.as_bool().ok_or_else(|| bad(SET, "enabled requires a boolean"))).transpose()?;
    let opacity = params.get("opacityF64Bits").map(bits64).transpose()?;
    if opacity.is_some_and(|bits| !(0.0..=1.0).contains(&f64::from_bits(bits))) {
        return Err(bad(SET, "opacity must be finite and within 0..1"));
    }
    // Session::edit deliberately records accepted transactions, even equal ones.
    // Keep an exact no-op outside that history boundary while retaining qualification.
    let snapshot = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let revision = s.active().ok_or(EngineError::NoDocument)?.revision;
    snapshot.validate_resident().map_err(|error| bad(SET, format!("filter edits require resident editable storage: {error}")))?;
    let document: &Document = &snapshot;
    document.validate_xcf_state_with(control).map_err(|e| if control.cancelled() {EngineError::Cancelled} else {bad(SET,e.to_string())})?;
    set_clone_headroom(&snapshot,control)?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad(SET, "missing imported filter state"))?;
    let stack = state.filter_stacks.iter().find(|stack| stack.nodes.iter().any(|node| node.id == id)).ok_or_else(|| bad(SET, "unknown filterId"))?;
    let node = stack.nodes.iter().find(|node| node.id == id).ok_or_else(|| bad(SET, "unknown filterId"))?;
    if let ItemRef::Layer(owner) = stack.owner {
        if document.layer(owner).is_some_and(|layer| matches!(layer.content, LayerContent::Group(_))) {
            return group_activation::set(s, params, &snapshot, revision, id, control);
        }
    }
    let patches = patches(params, &node.operation.name)?;
    if enabled.is_none() && opacity.is_none() && patches.is_empty() {
        return Err(bad(SET, "an activation or parameter patch is required"));
    }
    for patch in &patches {
        control.check().map_err(|_|EngineError::Cancelled)?;
        let original =
            node.arguments.iter().find(|argument| argument.name == patch.name).ok_or_else(|| bad(SET, format!("unknown stored parameter {}", patch.name)))?;
        if !same_parameter_type(node, &patch.name, &original.value, &patch.value) {
            return Err(bad(SET, "parameter stored type/domain must remain exact"));
        }
    }
    if enabled.is_none_or(|enabled| enabled == node.enabled)
        && opacity.is_none_or(|opacity| opacity == node.opacity_f64_bits)
        && patches.iter().all(|patch| node.arguments.iter().any(|argument| argument == patch))
    {
        if !scalar_edit_candidate(node) {
            return Err(bad(SET, "this filter is inspection-only"));
        }
        let ItemRef::Layer(owner) = stack.owner else {
            return Err(bad(SET, "only admitted raster-layer filter owners are editable"));
        };
        let effective = crate::xcf_item_locks::effective_layer_locks(document, owner).map_err(|e| bad(SET, e.to_string()))?;
        let locks = effective.imported;
        let ordinary = effective.native;
        if locks.content || ordinary.all || ordinary.pixels {
            return Err(bad(SET, "filter owner or inherited content is locked"));
        }
        if !document.layer(owner).is_some_and(|layer| matches!(layer.content, LayerContent::Raster(_))) {
            return Err(bad(SET, "filter editing requires an ordinary raster owner"));
        }
        if !node.enabled && node.operation.name == "gegl:unpremultiply" {
            // This closed consumer validates disabled native input and schema.
            // Its identity path must not execute division that was not requested.
            validate_current_pipeline_with_interrupt(document, control)?;
            final_set_context(s,&snapshot,revision,params,control)?;
            return Ok(json!({"filterId":id.0.to_string(),"updated":false}));
        }
        let mut active = document.clone();
        let active_node = active
            .xcf_state
            .as_mut()
            .and_then(|state| state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)))
            .and_then(|stack| stack.nodes.iter_mut().find(|node| node.id == id))
            .ok_or_else(|| bad(SET, "missing staged filter"))?;
        active_node.enabled = true;
        validate_current_pipeline_with_interrupt(&active, control)?;
        final_set_context(s,&snapshot,revision,params,control)?;
        return Ok(json!({"filterId":id.0.to_string(),"updated":false}));
    }
    final_set_context(s,&snapshot,revision,params,control)?;
    let mut updated = false;
    s.edit_with_interrupt("Edit GIMP Stock Filter", control, |doc, _| {
        doc.validate_xcf_state_with(control).map_err(|e|if control.cancelled() {EngineError::Cancelled} else {bad(SET,e.to_string())})?;
        let state = doc.xcf_state.as_ref().ok_or_else(|| bad(SET, "missing imported filter state"))?;
        let stack = state.filter_stacks.iter().find(|stack| stack.nodes.iter().any(|node| node.id == id)).ok_or_else(|| bad(SET, "unknown filterId"))?;
        let ItemRef::Layer(owner) = stack.owner else {
            return Err(bad(SET, "only admitted raster-layer filter owners are editable"));
        };
        let node = stack.nodes.iter().find(|node| node.id == id).ok_or_else(|| bad(SET, "unknown filterId"))?;
        if !scalar_edit_candidate(node) {
            return Err(bad(SET, "this filter is inspection-only; closed parameter and activation semantics remain unqualified"));
        }
        let effective = crate::xcf_item_locks::effective_layer_locks(doc, owner).map_err(|e| bad(SET, e.to_string()))?;
        let locks = effective.imported;
        let ordinary = effective.native;
        if locks.content || ordinary.all || ordinary.pixels || (enabled.is_some_and(|enabled| enabled != node.enabled) && locks.visibility) {
            return Err(bad(SET, "filter owner or inherited content/activation is locked"));
        }
        if !doc.layer(owner).is_some_and(|layer| matches!(layer.content, LayerContent::Raster(_))) {
            return Err(bad(SET, "filter editing requires an ordinary raster owner"));
        }
        let state = doc.xcf_state.as_mut().ok_or_else(|| bad(SET, "missing filter state"))?;
        let stack = state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)).ok_or_else(|| bad(SET, "missing filter owner"))?;
        let node = stack.nodes.iter_mut().find(|node| node.id == id).ok_or_else(|| bad(SET, "unknown filterId"))?;
        let previous = node.clone();
        for patch in &patches {
            control.check().map_err(|_|EngineError::Cancelled)?;
            let argument = node
                .arguments
                .iter_mut()
                .find(|argument| argument.name == patch.name)
                .ok_or_else(|| bad(SET, format!("unknown stored parameter {}", patch.name)))?;
            if !same_parameter_type(&previous, &patch.name, &argument.value, &patch.value) {
                return Err(bad(SET, "parameter stored type/domain must remain exact"));
            }
            argument.value = patch.value.clone();
        }
        if let Some(enabled) = enabled {
            node.enabled = enabled;
        }
        if let Some(opacity) = opacity {
            node.opacity_f64_bits = opacity;
        }
        if *node == previous {
            return Ok(());
        }
        updated = true;
        let inactive = !node.enabled;
        stack.revision = stack.revision.checked_add(1).ok_or_else(|| bad(SET, "filter revision overflow"))?;
        // Disabled nodes cannot bypass schema/kernel qualification with hidden invalid values.
        if inactive {
            let mut active = doc.clone();
            let active_node = active
                .xcf_state
                .as_mut()
                .and_then(|state| state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)))
                .and_then(|stack| stack.nodes.iter_mut().find(|node| node.id == id))
                .ok_or_else(|| bad(SET, "missing staged filter"))?;
            active_node.enabled = true;
            validate_current_pipeline_with_interrupt(&active, control)?;
        }
        validate_current_pipeline_with_interrupt(doc, control)?;
        Ok(())
    })?;
    Ok(json!({"filterId":id.0.to_string(),"updated":updated}))
}
fn enable(s: &mut Session, params: &Value) -> Result<Value> {
    enable_with_interrupt(s, params, &Interrupt::NONE)
}
pub fn enable_with_interrupt(s: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    pre_dispatch_set_change(s,ENABLE,params,false)?;
    object(ENABLE, params, &["filterId", "enabled"])?;
    let enabled = params.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad(ENABLE, "enabled must be a boolean"))?;
    let id = id(ENABLE, params)?;
    set_with_interrupt(s, &json!({"filterId":id.0.to_string(),"enabled":enabled}), control)
}
fn remove(s: &mut Session, params: &Value) -> Result<Value> {
    remove_with_interrupt(s, params, &Interrupt::NONE)
}
fn move_filter(s: &mut Session, params: &Value) -> Result<Value> {
    move_with_interrupt(s, params, &Interrupt::NONE)
}
/// MOVE/REMOVE-only dispatcher fence, called before floating normalization,
/// generic job setup or parameter cloning. Other command behavior is unchanged.
pub fn pre_dispatch_stack_change(s: &Session, cmd: &str, params: &Value) -> Result<()> {
    pre_dispatch_stack_change_with_background(s, cmd, params, false)
}
/// Explicit transport start mode before dispatcher job setup can erase context.
pub fn pre_dispatch_stack_change_with_background(s: &Session, cmd: &str, params: &Value, background: bool) -> Result<()> {
    if !matches!(cmd, REMOVE | MOVE) {
        return Err(bad(cmd, "not a filter stack command"));
    }
    if background { return Err(bad(cmd, "filter stack edit requires synchronous context")); }
    object(cmd, params, if cmd == REMOVE { &["filterId"] } else { &["filterId", "index"] })?;
    id(cmd, params)?;
    if cmd == MOVE {
        params.get("index").and_then(Value::as_u64).filter(|index| *index < 1024).ok_or_else(|| bad(cmd, "index must be a bounded owner-local integer"))?;
    }
    stack_context(s, cmd)
}

/// Separate typed Point preflight. This selects a paid transaction only;
/// complete current/staged storage and operative validation remain mandatory.
pub(crate) fn pre_dispatch_point_move(s: &Session, cmd: &str, params: &Value, background: bool) -> Result<()> {
    if cmd != MOVE || !s.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_point_storage()) {
        return Err(bad(cmd, "Point move requires actual Point storage"));
    }
    if background { return Err(bad(cmd, "filter stack edit requires synchronous context")); }
    object(cmd, params, &["filterId", "index"])?;
    id(cmd, params)?;
    params.get("index").and_then(Value::as_u64).filter(|index| *index < 1024).ok_or_else(|| bad(cmd, "index must be a bounded owner-local integer"))?;
    stack_context_for(s, cmd, true)
}

/// Separate actual-Point removal fence; the paid transaction enforces the
/// remaining-stack recipe and preserves removed mask metadata as inert data.
pub(crate) fn pre_dispatch_point_remove(s: &Session, cmd: &str, params: &Value, background: bool) -> Result<()> {
    if cmd != REMOVE || !s.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_point_storage()) {
        return Err(bad(cmd, "Point removal requires actual Point storage"));
    }
    if background { return Err(bad(cmd, "filter stack edit requires synchronous context")); }
    object(cmd, params, &["filterId"])?;
    id(cmd, params)?;
    stack_context_for(s, cmd, true)
}

fn stack_context(s: &Session, cmd: &str) -> Result<()> {
    stack_context_for(s, cmd, false)
}

fn stack_context_for(s: &Session, cmd: &str, point: bool) -> Result<()> {
    if s.coalesce_request.is_some() || s.color_restrict.is_some() || s.command_is_background() || s.command_has_pending_job() || s.has_jobs() {
        return Err(bad(cmd, "filter stack edit requires synchronous uncoalesced context without jobs"));
    }
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if s.documents().len() > 1024 { return Err(bad(cmd, "filter stack document census limit")); }
    if s.documents().iter().any(|document| document.floating.is_some() || document.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())) {
        return Err(bad(cmd, "filter stack edit refuses an existing floating selection or imported attachment"));
    }
    if active.revision == u64::MAX || if point {
        active.doc.accounted().is_none_or(|owner| !owner.is_readonly_point_storage())
    } else {
        active.doc.accounted().is_some()
    } {
        return Err(bad(cmd, "resident document and revision headroom required"));
    }
    Ok(())
}

fn enabled_stack_change(s: &Session) -> std::result::Result<(), String> {
    stack_context(s, MOVE).map_err(|error| error.to_string())?;
    enabled(s)
}
pub fn remove_with_interrupt(s: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    stack_change(s, params, true, control)
}
pub fn move_with_interrupt(s: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    stack_change(s, params, false, control)
}
fn stack_change(s: &mut Session, params: &Value, remove: bool, control: &Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let cmd = if remove { REMOVE } else { MOVE };
    pre_dispatch_stack_change(s, cmd, params)?;
    object(cmd, params, if remove { &["filterId"] } else { &["filterId", "index"] })?;
    let id = id(cmd, params)?;
    let destination = if remove {
        None
    } else {
        let index =
            params.get("index").and_then(Value::as_u64).filter(|index| *index < 1024).ok_or_else(|| bad(cmd, "index must be a bounded owner-local integer"))?;
        Some(usize::try_from(index).map_err(|_| bad(cmd, "index overflow"))?)
    };
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let document_id = active.doc.id;
    let document_revision = active.revision;
    let snapshot = active.doc.downgrade();
    let document = &active.doc;
    document.validate_resident().map_err(|error| bad(cmd, format!("filter stack edits require resident editable storage: {error}")))?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad(cmd, "missing imported filter state"))?;
    if state.items.len() > 10_000 || state.filter_stacks.len() > 10_000 || state.filter_masks.len() > 10_000 {
        return Err(bad(cmd, "filter stack metadata census limit"));
    }
    let mut nodes = 0usize;
    for stack in &state.filter_stacks {
        control.check().map_err(|_| EngineError::Cancelled)?;
        nodes = nodes.checked_add(stack.nodes.len()).filter(|count| *count <= 10_000).ok_or_else(|| bad(cmd, "filter node census limit"))?;
        if stack.nodes.len() > 1024 { return Err(bad(cmd, "owner filter stack limit")); }
    }
    document.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, e.to_string()) })?;
    crate::xcf_filter_insert_cmds::headroom(document, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, error.to_string()) })?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad(cmd, "missing imported filter state"))?;
    let stack = state.filter_stacks.iter().find(|stack| stack.nodes.iter().any(|node| node.id == id)).ok_or_else(|| bad(cmd, "unknown filterId"))?;
    let original_index = stack.nodes.iter().position(|node| node.id == id).ok_or_else(|| bad(cmd, "unknown filterId"))?;
    let ItemRef::Layer(owner) = stack.owner else {
        return Err(bad(cmd, "only admitted raster-layer filter stacks are editable"));
    };
    let effective = crate::xcf_item_locks::effective_layer_locks(document, owner).map_err(|e| bad(cmd, e.to_string()))?;
    let imported = effective.imported;
    let ordinary = effective.native;
    if imported.content || ordinary.all || ordinary.pixels {
        return Err(bad(cmd, "filter owner or inherited content is locked"));
    }
    if !document.layer(owner).is_some_and(|layer| matches!(layer.content, LayerContent::Raster(_))) {
        return Err(bad(cmd, "filter editing requires an ordinary raster owner"));
    }
    if destination.is_some_and(|index| index >= stack.nodes.len()) {
        return Err(bad(cmd, "index is outside the current owner stack"));
    }
    if stack.revision == u64::MAX {
        return Err(bad(cmd, "filter stack revision overflow"));
    }
    let stack_revision = stack.revision;
    // Prepare the small command response before any Session publication.
    let response = json!({"filterId":id.0.to_string(),"updated":destination != Some(original_index)});
    validate_current_pipeline_with_interrupt(document, control)?;
    // Order/removal does not activate a disabled descriptor. Qualify the
    // actual current and proposed renders, never an invented enabled state.
    control.check().map_err(|_| EngineError::Cancelled)?;
    stack_context(s, cmd)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let original = snapshot.upgrade().ok_or_else(|| bad(cmd, "stale filter stack target"))?;
    if active.doc.id != document_id || active.revision != document_revision || !original.shares_state(&active.doc) {
        return Err(bad(cmd, "stale filter stack target"));
    }
    if destination == Some(original_index) {
        return Ok(response);
    }
    s.edit_with_interrupt(if remove { "Remove GIMP Stock Filter" } else { "Move GIMP Stock Filter" }, control, |doc, _| {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let state = doc.xcf_state.as_mut().ok_or_else(|| bad(cmd, "missing filter state"))?;
        let stack = state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)).ok_or_else(|| bad(cmd, "missing filter owner"))?;
        let index = stack.nodes.iter().position(|node| node.id == id).ok_or_else(|| bad(cmd, "unknown filterId"))?;
        if stack.revision != stack_revision || index != original_index || index >= stack.nodes.len() {
            return Err(bad(cmd, "invalid filter index"));
        }
        let node = stack.nodes.remove(index);
        if let Some(destination) = destination {
            if destination > stack.nodes.len() {
                return Err(bad(cmd, "invalid destination index"));
            }
            stack.nodes.insert(destination, node);
        }
        stack.revision = stack.revision.checked_add(1).ok_or_else(|| bad(cmd, "filter revision overflow"))?;
        if remove {
            let masks: std::collections::BTreeSet<_> = state.filter_masks.iter().filter(|mask| mask.owner == id).map(|mask| ItemRef::Mask(mask.id)).collect();
            // Required surviving relationships and source-owned audit records cannot be erased.
            if state.opaque.iter().any(|record| {
                record.owner.is_some_and(|owner| masks.contains(&owner)) && (record.rendering_required || record.namespace != "gimp/xcf/source-property")
            }) || state.source_items.iter().any(|source| masks.contains(&source.owner))
                || state
                    .filter_stacks
                    .iter()
                    .flat_map(|stack| &stack.nodes)
                    .any(|node| node.mask.is_some_and(|mask| masks.contains(&mask)) || node.auxiliary.iter().any(|(_, input)| masks.contains(input)))
            {
                return Err(bad(cmd, "filter mask still owns required source or dependent state"));
            }
            let mut archived = std::collections::BTreeSet::new();
            for record in &state.opaque {
                if record.owner.is_none() {
                    archived.insert((record.namespace.clone(), record.identity.clone()));
                }
            }
            for record in &mut state.opaque {
                if let Some(ItemRef::Mask(mask)) = record.owner
                    && masks.contains(&ItemRef::Mask(mask))
                {
                    if record.identity.len() > 4096 {
                        return Err(bad(cmd, "deleted source property identity limit"));
                    }
                    let identity = format!("deleted-filter-mask/{}/{original}", mask.0, original = record.identity);
                    if !archived.insert((record.namespace.clone(), identity.clone())) {
                        return Err(bad(cmd, "deleted source property archive collision"));
                    }
                    // Exact raw source records remain inert document audit evidence.
                    // The original owner identity is explicit, never an active orphan.
                    record.owner = None;
                    record.identity = identity;
                }
            }
            state.filter_masks.retain(|mask| mask.owner != id);
            state.reconcile_filter_output_precision();
            state.items.retain(|item| !masks.contains(&item.item));
            state.selection.selected.retain(|item| !masks.contains(item));
            state.selection.selected_paths.retain(|item| !masks.contains(item));
            if state.selection.active.is_some_and(|item| masks.contains(&item)) {
                state.selection.active = Some(ItemRef::Layer(owner));
                if !state.selection.selected.contains(&ItemRef::Layer(owner)) {
                    state.selection.selected.push(ItemRef::Layer(owner));
                }
            }
            if state.selection.mask_target.is_some_and(|item| masks.contains(&item)) {
                state.selection.mask_target = None;
            }
            for set in &mut state.item_sets {
                set.members.retain(|item| !masks.contains(item));
            }
        }
        crate::xcf_filter_insert_cmds::headroom(doc, control)
            .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, error.to_string()) })?;
        validate_current_pipeline_with_interrupt(doc, control)?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        Ok(())
    })?;
    Ok(response)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: INSPECT,
            label: "Inspect GIMP Stock Filter",
            menu: &[],
            shortcut: None,
            params: "{filterId:decimal string} -> exact scalars and closed FREE256/Levels/Color Balance configuration bits; unresolved descriptors",
            enabled,
            run: inspect,
            journal: false,
        },
        CommandSpec {
            id: SET,
            label: "Edit GIMP Stock Filter",
            menu: &[],
            shortcut: None,
            params: "{filterId:decimal string,enabled?:bool,opacityF64Bits?:hex64,arguments?:[{name,value}]} exact scalars or admitted FREE256/Levels/Color Balance Config; Float64 bits fixed16 lowercase0x hex strings",
            enabled,
            run: set,
            journal: true,
        },
        CommandSpec {
            id: ENABLE,
            label: "Enable GIMP Stock Filter",
            menu: &[],
            shortcut: None,
            params: "{filterId:decimal string,enabled:bool}; exact qualified activation; isolated-small-auto Gaussian group singleton activation only, no group parameter/stack edits",
            enabled,
            run: enable,
            journal: true,
        },
        CommandSpec {
            id: REMOVE,
            label: "Remove GIMP Stock Filter",
            menu: &[],
            shortcut: None,
            params: "{filterId:decimal string}; preserves immutable source evidence",
            enabled: enabled_stack_change,
            run: remove,
            journal: true,
        },
        CommandSpec {
            id: MOVE,
            label: "Move GIMP Stock Filter",
            menu: &[],
            shortcut: None,
            params: "{filterId:decimal string,index:owner-local integer}; staged render qualification",
            enabled: enabled_stack_change,
            run: move_filter,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_transport_background_and_inactive_floating_refuse_without_normalizing() {
        let mut session = Session::new();
        let params = json!({"filterId":"1","index":0});
        assert!(pre_dispatch_stack_change_with_background(&session,MOVE,&params,true).unwrap_err().to_string().contains("synchronous"));
        let document = photocraft_doc::Document::new("Own inactive",photocraft_doc::Size::new(1,1),photocraft_doc::ColorMode::Rgb,photocraft_doc::SampleType::U8);
        session.add_document(document,None).unwrap();
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state = Some(photocraft_doc::XcfState {
            floating:Some(photocraft_doc::xcf_editing::FloatingAttachment {layer:photocraft_doc::LayerId(99),target:ItemRef::Layer(photocraft_doc::LayerId(100))}),
            ..Default::default()
        });
        let inactive = session.active().unwrap().doc.clone();
        session.add_document(photocraft_doc::Document::new("Own current",photocraft_doc::Size::new(1,1),photocraft_doc::ColorMode::Rgb,photocraft_doc::SampleType::U8),None).unwrap();
        let current = session.active().unwrap().doc.clone();
        assert!(pre_dispatch_stack_change(&session,MOVE,&params).unwrap_err().to_string().contains("floating"));
        assert!(session.documents().first().unwrap().doc.shares_state(&inactive));
        assert!(session.active().unwrap().doc.shares_state(&current));
        assert!(session.journal.is_empty());
    }

    #[test]
    fn disabled_unknown_descriptor_order_and_removal_do_not_require_activation() {
        use photocraft_doc::{ColorMode,Document,Layer,SampleEncoding,SampleType,Size,XcfState};
        use photocraft_doc::import_compositing::{GimpComposition,GimpOperator,GimpSpace,ResolvedGimpComposition};
        use photocraft_doc::xcf_filters::{FilterStack,OperationIdentity};
        let mut document = Document::new("Own inert descriptors",Size::new(2,1),ColorMode::Rgb,SampleType::F64);
        document.sample_encoding = Some(SampleEncoding::Linear);
        document.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let mut layer = Layer::raster("Own raster",document.pixel_format());
        layer.surface_mut().unwrap().try_write_region_f64(document.bounds(),&[0.25,0.5,0.75,1.0,0.125,0.25,0.5,1.0]).unwrap();
        let owner = layer.id; document.layers.push(layer);
        let mut state = XcfState::default(); state.provenance.format_version=22;
        state.reconcile_layers(&document).unwrap();
        let item = state.items.iter_mut().find(|item|item.item==ItemRef::Layer(owner)).unwrap();
        item.raster_bounds=Some(document.bounds());
        item.composition=Some(GimpComposition {compatibility_epoch:"own-current-filter-policy".into(),source_mode:28,legacy:false,stored_blend_space:1,stored_composite_space:1,stored_operator:1,pass_through:false,
            resolved:Some(ResolvedGimpComposition {blend_space:GimpSpace::LinearRgb,composite_space:GimpSpace::LinearRgb,operator:GimpOperator::Union})});
        let node = |id| FilterNode {id:FilterId(id),operation:OperationIdentity {name:"own:unqualified".into(),stored_version:"0:0".into(),schema_epoch:"own-retained-metadata".into()},
            source_display_name:Some("Own source label".into()),source_icon:Some("Own icon".into()),arguments:Vec::new(),enabled:false,opacity_f64_bits:1f64.to_bits(),source_clipping:None,source_region:None,mask:None,auxiliary:Vec::new(),composition:None};
        state.filter_stacks.push(FilterStack {owner:ItemRef::Layer(owner),revision:3,nodes:vec![node(1),node(2)]});
        document.xcf_state=Some(state);
        let mut session=Session::new(); session.add_document(document,None).unwrap();
        let before=session.active().unwrap().doc.clone();
        let native=photocraft_format::save_to_bytes(&before,&Default::default()).unwrap();
        let noop=move_with_interrupt(&mut session,&json!({"filterId":"1","index":0}),&Interrupt::NONE).unwrap();
        assert_eq!(noop["updated"],false); assert_eq!(session.active().unwrap().history.past_len(),0);
        move_with_interrupt(&mut session,&json!({"filterId":"1","index":1}),&Interrupt::NONE).unwrap();
        let moved=session.active().unwrap().doc.clone();
        assert_eq!(moved.layers,before.layers);
        assert_eq!(moved.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1],before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]);
        session.execute("edit.undo",json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc,&Default::default()).unwrap(),native);
        remove_with_interrupt(&mut session,&json!({"filterId":"1"}),&Interrupt::NONE).unwrap();
        assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes,vec![before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].clone()]);
        session.execute("edit.undo",json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc,&Default::default()).unwrap(),native);
    }

    #[test]
    fn stack_dispatch_refuses_inherited_transients_without_clearing_them() {
        let mut session = Session::new();
        session.coalesce_request = Some("retained edit group".into());
        let params = json!({"filterId":"1","index":0});
        let before = params.clone();
        assert!(pre_dispatch_stack_change(&session, MOVE, &params).unwrap_err().to_string().contains("uncoalesced"));
        assert_eq!(session.coalesce_request.as_deref(), Some("retained edit group"));
        session.coalesce_request = None;
        session.color_restrict = Some(2);
        assert!(pre_dispatch_stack_change(&session, REMOVE, &json!({"filterId":"1"})).unwrap_err().to_string().contains("uncoalesced"));
        assert_eq!(session.color_restrict, Some(2));
        assert_eq!(params, before);
        assert!(session.documents().is_empty());
        assert!(session.journal.is_empty());
    }

    #[test]
    fn stack_dispatch_requires_canonical_ids_and_closed_bounded_parameters() {
        let session = Session::new();
        for params in [
            json!({"filterId":"01","index":0}),
            json!({"filterId":"0","index":0}),
            json!({"filterId":"1","index":1024}),
            json!({"filterId":"1","index":0,"coalesce":"untrusted"}),
        ] {
            assert!(matches!(pre_dispatch_stack_change(&session, MOVE, &params), Err(EngineError::BadParams { .. })));
        }
        assert!(session.documents().is_empty());
        assert!(session.journal.is_empty());
    }
    #[test]
    fn exact_literal_color_wire_preserves_four_bits_and_refuses_assets_or_numeric_normalization() {
        let wire = json!({"Color":{"components":[{"Float32Bits":0.375f32.to_bits()},{"Float32Bits":0.5f32.to_bits()},{"Float32Bits":0.625f32.to_bits()},{"Float32Bits":(-0.0f32).to_bits()}],"encoding":"nonlinear_profile","profile":null}});
        let decoded = value(&wire).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
        assert!(same_type(&decoded, &decoded));
        for mutation in 0..9 {
            let mut altered = wire.clone();
            match mutation {
                0 => altered["Color"]["encoding"] = json!("Linear"),
                1 => altered["Color"]["profile"] = json!(1),
                2 => altered["Color"]["components"][0] = json!({"Float64Bits":0.375f64.to_bits()}),
                3 => altered["Color"]["components"][0] = json!({"Float32Bits":f32::NAN.to_bits()}),
                4 => altered["Color"]["components"][0] = json!({"Float32Bits":f32::INFINITY.to_bits()}),
                5 => altered["Color"]["components"][0] = json!({"Float32Bits":1.0001f32.to_bits()}),
                6 => altered["Color"]["components"][0] = json!(0.375),
                7 => altered["Color"]["components"].as_array_mut().unwrap().pop().map(|_| ()).unwrap(),
                _ => altered["Color"]["locator"] = json!("outside-input"),
            }
            assert!(value(&altered).is_err(), "{mutation}");
        }
    }
    fn curves() -> Value {
        let samples: Vec<_> = (0..256).map(|index| format!("0x{:016x}", (f64::from(index) / 255.0).to_bits())).collect();
        let channels =
            CHANNELS.map(|name| json!({"name":name,"value":{"Curve":{"free":true,"points_f64_bits":[],"point_types":[],"samples_f64_bits":samples}}}));
        json!({"Config":channels})
    }
    fn levels() -> Value {
        let fields: Vec<_> = LEVEL_FIELDS
            .into_iter()
            .zip([0.0_f64, 1.0, 1.0, 0.0, 1.0])
            .map(|(name, value)| json!({"name":name,"value":{"Number":{"Float64Bits":format!("0x{:016x}",value.to_bits())}}}))
            .collect();
        let channels = CHANNELS.map(|name| json!({"name":name,"value":{"Config":fields}}));
        json!({"Config":channels})
    }
    #[test]
    fn exact_structured_bits_and_shapes_round_trip() {
        for (operation, wire) in [("gimp:curves", curves()), ("gimp:levels", levels())] {
            let decoded = configuration(operation, &wire).unwrap();
            assert_eq!(config_wire(operation, &decoded).unwrap(), wire);
        }
        assert_eq!(bits64(&json!("0x8000000000000000")).unwrap(), (-0.0_f64).to_bits());
        for invalid in [
            json!(0),
            json!("0"),
            json!("0x0"),
            json!("0X0000000000000000"),
            json!("0x3FF0000000000000"),
            json!("0x7ff0000000000000"),
            json!("0x7ff8000000000001"),
        ] {
            assert!(bits64(&invalid).is_err());
        }
    }
    #[test]
    fn unknown_duplicate_missing_or_deep_configurations_fail_bounded() {
        let mut bad = curves();
        bad["Config"][1]["name"] = json!("value");
        assert!(configuration("gimp:curves", &bad).is_err());
        let mut bad = curves();
        bad["Config"][0]["value"]["Curve"]["free"] = json!(false);
        assert!(configuration("gimp:curves", &bad).is_err());
        let mut bad = curves();
        bad["Config"][0]["value"]["Curve"]["samples_f64_bits"] = json!(vec!["0x0000000000000000"; 257]);
        assert!(configuration("gimp:curves", &bad).is_err());
        let mut bad = curves();
        bad["Config"][0]["value"]["Curve"]["points_f64_bits"] = json!([[0, 0]]);
        assert!(configuration("gimp:curves", &bad).is_err());
        let mut bad = levels();
        bad["Config"][0]["value"]["Config"][1]["name"] = json!("low-input");
        assert!(configuration("gimp:levels", &bad).is_err());
        let mut bad = levels();
        bad["Config"][0]["value"]["Config"][2]["value"]["Number"]["Float64Bits"] = json!("0x0000000000000000");
        assert!(configuration("gimp:levels", &bad).is_err());
        let mut bad = levels();
        bad["Config"][0]["value"]["Config"] = json!([]);
        assert!(configuration("gimp:levels", &bad).is_err());
        let mut bad = levels();
        bad["Config"][0]["value"]["Config"][0]["value"] = json!({"Config":[]});
        assert!(configuration("gimp:levels", &bad).is_err());
        assert!(configuration("gegl:exposure", &levels()).is_err());
        assert!(configuration("gimp:levels", &json!({"Config":[],"extra":true})).is_err());
    }
}

#[cfg(test)]
mod set_admission_tests {
    use super::*;
    #[test]
    fn set_wire_refuses_unknown_nested_and_oversized_patch_before_dispatch() {
        let mut session=Session::new();
        session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        for params in [json!({"filterId":"1","enabled":"true"}),json!({"filterId":"01","enabled":true}),
            json!({"filterId":"1","arguments":[{"name":"threshold","value":{"Number":{"Float32Bits":2143289344u32}}}]}),
            json!({"filterId":"1","arguments":[{"name":"threshold","value":{"Resource":"inert"}}]}),
            json!({"filterId":"1","arguments":[{"name":"a","value":{"Bool":true}},{"name":"a","value":{"Bool":false}}]})] {
            let before=session.active().unwrap().doc.clone();
            assert!(pre_dispatch_set_change(&session,SET,&params,false).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
        }
        let args=vec![json!({"name":"a","value":{"Bool":true}});65];
        assert!(pre_dispatch_set_change(&session,SET,&json!({"filterId":"1","arguments":args}),false).is_err());
    }
    #[test]
    fn enable_admission_rejects_background_coalesce_and_overflow_before_edit() {
        let mut session=Session::new();
        session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        let params=json!({"filterId":"1","enabled":false});
        let before=session.active().unwrap().doc.clone();
        assert!(pre_dispatch_set_change(&session,ENABLE,&params,true).is_err());
        session.coalesce_request=Some("Own retained coalesce".into());
        assert!(pre_dispatch_set_change(&session,ENABLE,&params,false).is_err());
        session.coalesce_request=None;
        session.active_mut().unwrap().revision=u64::MAX;
        assert!(pre_dispatch_set_change(&session,ENABLE,&params,false).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
