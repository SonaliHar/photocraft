//! Lossless bounded view cursors; setting one never runs a source/resource query.
use photocraft_doc::{
    LayerId,
    item_identity::{ChannelId, ItemRef, MaskId, PathId},
};
use serde_json::{Value, json};
type Result<T> = std::result::Result<T, String>;
fn decimal(value: Option<&Value>, nonzero: bool) -> Result<u64> {
    let text = value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= 20 && (text.len() == 1 || !text.starts_with('0')) && text.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or("page identities must be canonical decimal strings")?;
    let number = text.parse::<u64>().map_err(|_| "page identity exceeds U64")?;
    if nonzero && number == 0 {
        return Err("page identity must be nonzero".into());
    }
    Ok(number)
}
fn owner(value: &Value) -> Result<ItemRef> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 2 && object.contains_key("kind") && object.contains_key("id"))
        .ok_or("source page owner requires exactly kind and id")?;
    let id = decimal(object.get("id"), true)?;
    match object.get("kind").and_then(Value::as_str) {
        Some("layer") => Ok(ItemRef::Layer(LayerId(id))),
        Some("channel") => Ok(ItemRef::Channel(ChannelId(id))),
        Some("mask") => Ok(ItemRef::Mask(MaskId(id))),
        Some("path") => Ok(ItemRef::Path(PathId(id))),
        _ => Err("source page owner kind must be layer, channel, mask or path".into()),
    }
}
pub(crate) fn source(p: &Value) -> Result<Option<Option<crate::state::XcfSourcePageDraft>>> {
    let Some(value) = p.get("xcfSourcePageDraft") else { return Ok(None) };
    if value.is_null() {
        return Ok(Some(None));
    }
    let object = value
        .as_object()
        .filter(|object| {
            (4..=5).contains(&object.len()) && object.keys().all(|key| matches!(key.as_str(), "document" | "revision" | "offset" | "owner" | "layer"))
        })
        .ok_or("source page requires document, revision, offset and owner or legacy layer")?;
    let document = decimal(object.get("document"), true)?;
    let revision = decimal(object.get("revision"), false)?;
    let offset = object
        .get("offset")
        .and_then(Value::as_u64)
        .filter(|offset| *offset <= 100_000 && *offset % 32 == 0)
        .ok_or("source page offset must be a multiple of 32 from 0 through 100000")?;
    let selected = object.get("owner").map(owner).transpose()?;
    let layer = match selected {
        Some(ItemRef::Layer(id)) => id.0,
        Some(_) => 0,
        None => decimal(object.get("layer"), true)?,
    };
    if selected.is_some() && object.get("layer").is_some() && decimal(object.get("layer"), false)? != layer {
        return Err("source page legacy layer does not match its typed owner".into());
    }
    Ok(Some(Some(crate::state::XcfSourcePageDraft { document, revision, layer, offset: offset as u32, owner: selected })))
}
pub(crate) fn resource(p: &Value) -> Result<Option<Option<crate::xcf_notices::ResourcePageDraft>>> {
    let Some(value) = p.get("xcfResourcePageDraft") else { return Ok(None) };
    if value.is_null() {
        return Ok(Some(None));
    }
    let object = value
        .as_object()
        .filter(|object| object.len() == 4 && object.keys().all(|key| matches!(key.as_str(), "document" | "revision" | "offset" | "open")))
        .ok_or("resource page requires exactly document, revision, offset and open")?;
    let offset = object
        .get("offset")
        .and_then(Value::as_u64)
        .filter(|offset| *offset <= 960 && *offset % 64 == 0)
        .ok_or("resource page offset must be a multiple of 64 from 0 through 960")?;
    Ok(Some(Some(crate::xcf_notices::ResourcePageDraft {
        document: decimal(object.get("document"), true)?,
        revision: decimal(object.get("revision"), false)?,
        offset: offset as usize,
        open: object.get("open").and_then(Value::as_bool).ok_or("resource page open must be boolean")?,
    })))
}
pub(crate) fn source_wire(page: Option<&crate::state::XcfSourcePageDraft>) -> Value {
    page.map_or(Value::Null, |page| {
        let selected = page.owner.unwrap_or(ItemRef::Layer(LayerId(page.layer)));
        let kind = match selected {
            ItemRef::Layer(_) => "layer",
            ItemRef::Channel(_) => "channel",
            ItemRef::Mask(_) => "mask",
            ItemRef::Path(_) => "path",
        };
        json!({"document":page.document.to_string(),"revision":page.revision.to_string(),"layer":page.layer.to_string(),
            "offset":page.offset,"owner":{"kind":kind,"id":selected.raw().to_string()}})
    })
}
pub(crate) fn resource_wire(page: Option<&crate::xcf_notices::ResourcePageDraft>) -> Value {
    page.map_or(Value::Null, |page| json!({"document":page.document.to_string(),"revision":page.revision.to_string(),"offset":page.offset,"open":page.open}))
}
/// View-only values cannot recreate the private composition edit origin.
pub(crate) fn composition(p: &Value) -> Result<Option<Option<crate::xcf_panels::composition::CompositionDraft>>> {
    let Some(value) = p.get("xcfCompositionDraft") else { return Ok(None) };
    if value.is_null() {
        return Ok(Some(None));
    }
    let object = value
        .as_object()
        .filter(|object| {
            object.len() == 7
                && object
                    .keys()
                    .all(|key| matches!(key.as_str(), "document" | "revision" | "layer" | "sourceMode" | "blendSpace" | "compositeSpace" | "operator"))
        })
        .ok_or("composition draft requires exactly document, revision, layer, sourceMode, blendSpace, compositeSpace and operator")?;
    let choice = |key: &str, allowed: &[u8]| -> Result<Option<u8>> {
        let value = object.get(key).ok_or("missing composition choice")?;
        if value.is_null() {
            return Ok(None);
        }
        let code = value.as_u64().and_then(|value| u8::try_from(value).ok()).filter(|value| allowed.contains(value)).ok_or("unsupported composition choice")?;
        Ok(Some(code))
    };
    Ok(Some(Some(crate::xcf_panels::composition::CompositionDraft {
        document: decimal(object.get("document"), true)?,
        revision: decimal(object.get("revision"), false)?,
        layer: decimal(object.get("layer"), true)?,
        source_mode: object
            .get("sourceMode")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or("composition sourceMode must be an I32 integer")?,
        blend_space: choice("blendSpace", &[1, 2, 4])?,
        composite_space: choice("compositeSpace", &[1, 2, 4])?,
        operator: choice("operator", &[1, 2, 3, 4])?,
    })))
}
pub(crate) fn composition_wire(draft: Option<&crate::xcf_panels::composition::CompositionDraft>) -> Value {
    draft.map_or(Value::Null, |draft| {
        json!({"document":draft.document.to_string(), "revision":draft.revision.to_string(),
        "layer":draft.layer.to_string(), "sourceMode":draft.source_mode, "blendSpace":draft.blend_space,
        "compositeSpace":draft.composite_space, "operator":draft.operator})
    })
}
