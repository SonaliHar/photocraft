//! Scoped exact-font text edits; retained source raster evidence is immutable.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{Document, LayerId, item_identity::ItemRef, xcf_items::EditableSource};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
const SET: &str = "xcf.text.content.set";
const SIZE: &str = "xcf.text.fontSize.set";
const POSITION: &str = "xcf.text.position.set";
const LINE_SPACING: &str = "xcf.text.lineSpacing.set";
const SPACING: &str = "xcf.text.letterSpacing.set";
const COLOR: &str = "xcf.text.color.set";
const UNDERLINE: &str = "xcf.text.underline.set";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: SET.into(), msg: message.into() }
}
fn text_error(e: photocraft_text::xcf::TextSourceError, ctl: &Interrupt<'_>) -> EngineError {
    if ctl.cancelled() || e == photocraft_text::xcf::TextSourceError::Cancelled { EngineError::Cancelled } else { bad(e.to_string()) }
}
fn index(doc: &Document, owner: LayerId, ctl: &Interrupt<'_>) -> Result<(usize, usize, [i32; 2])> {
    qualified_index(doc, owner, ctl, true)
}
fn qualified_index(doc: &Document, owner: LayerId, ctl: &Interrupt<'_>, require_editable: bool) -> Result<(usize, usize, [i32; 2])> {
    let (idx, rid, origin) = source_index(doc, owner, ctl, require_editable)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing source state"))?;
    let item = state.source_items.get(idx).ok_or_else(|| bad("missing text source"))?;
    let resource = state.resources.get(rid).ok_or_else(|| bad("missing exact font"))?;
    let source = photocraft_text::xcf::qualify_placed_layout_colored_rectilinear_source(item, resource, origin, ctl).map_err(|e| text_error(e, ctl))?;
    let font = photocraft_text::xcf::ExactFont::bundled_inter(ctl).map_err(|e| text_error(e, ctl))?;
    photocraft_text::xcf::qualify_layout_rectilinear_geometry(&font, source, ctl).map_err(|e| text_error(e, ctl))?;
    Ok((idx, rid, origin))
}
fn source_index(doc: &Document, owner: LayerId, ctl: &Interrupt<'_>, require_editable: bool) -> Result<(usize, usize, [i32; 2])> {
    if ctl.cancelled() {
        return Err(EngineError::Cancelled);
    }
    doc.validate_xcf_state_with(ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing retained text source"))?;
    let mut matches = state.source_items.iter().enumerate().filter(|(_, s)| s.owner == ItemRef::Layer(owner));
    let (idx, item) = matches.next().ok_or_else(|| bad("unknown text source owner"))?;
    if matches.next().is_some() {
        return Err(bad("duplicate text source owner"));
    }
    let EditableSource::Text { font, .. } = &item.source else { return Err(bad("owner is not text")) };
    let mut resources = state.resources.iter().enumerate().filter(|(_, r)| r.id == *font);
    let (rid, _resource) = resources.next().ok_or_else(|| bad("missing exact font identity"))?;
    if resources.next().is_some() {
        return Err(bad("duplicate font identity"));
    }
    let bounds = state.items.iter().find(|s| s.item == item.owner).and_then(|s| s.raster_bounds).ok_or_else(|| bad("missing declared text placement"))?;
    let origin = [bounds.x0, bounds.y0];
    let layer = doc.layer(owner).ok_or_else(|| bad("unknown layer"))?;
    if layer.is_group() {
        return Err(bad("text owner is a group"));
    }
    let locks = state.effective_locks(item.owner).map_err(|e| bad(e.to_string()))?;
    let ordinary = doc.effective_locks(owner);
    if require_editable && (locks.content || locks.position || ordinary.all || ordinary.pixels || ordinary.position) {
        return Err(bad("text owner or inherited content/position is locked"));
    }
    Ok((idx, rid, origin))
}
/// Cache eligibility by revision; dispatch independently rechecks locks.
pub fn content_edit_candidate(doc: &Document, owner: LayerId) -> bool {
    index(doc, owner, &Interrupt::NONE).is_ok()
}
/// Qualified current AA pixel size; no stored path activates a font.
pub fn current_font_size(doc: &Document, owner: LayerId) -> Option<u32> {
    let (idx, rid, origin) = qualified_index(doc, owner, &Interrupt::NONE, false).ok()?;
    let state = doc.xcf_state.as_ref()?;
    let (request, mode, _, _) = photocraft_text::xcf::qualify_placed_spaced_colored_rectilinear_source(
        state.source_items.get(idx)?,
        state.resources.get(rid)?,
        origin,
        &Interrupt::NONE,
    )
    .ok()?;
    if mode != photocraft_text::xcf::RectilinearAntialias::AreaCoverage {
        return None;
    }
    match request.size_px {
        16. => Some(16),
        24. => Some(24),
        32. => Some(32),
        40. => Some(40),
        _ => None,
    }
}
pub fn font_size_edit_candidate(doc: &Document, owner: LayerId) -> bool {
    index(doc, owner, &Interrupt::NONE).is_ok() && current_font_size(doc, owner).is_some()
}
/// Read-only declared color inspection; locks do not hide retained metadata.
pub fn current_text_color(doc: &Document, owner: LayerId) -> Option<photocraft_text::xcf::TextColor> {
    let (idx, rid, origin) = qualified_index(doc, owner, &Interrupt::NONE, false).ok()?;
    let state = doc.xcf_state.as_ref()?;
    let (_, _, color, _) = photocraft_text::xcf::qualify_placed_spaced_colored_rectilinear_source(
        state.source_items.get(idx)?,
        state.resources.get(rid)?,
        origin,
        &Interrupt::NONE,
    )
    .ok()?;
    Some(color)
}
/// Editing rechecks ordinary and inherited locks independently of inspection.
pub fn color_edit_candidate(doc: &Document, owner: LayerId) -> bool {
    index(doc, owner, &Interrupt::NONE).is_ok() && current_text_color(doc, owner).is_some()
}
/// Read-only operative spacing; locked text keeps its declared value visible.
pub fn current_letter_spacing(doc: &Document, owner: LayerId) -> Option<f32> {
    let (idx, rid, origin) = qualified_index(doc, owner, &Interrupt::NONE, false).ok()?;
    let state = doc.xcf_state.as_ref()?;
    let (_, _, _, spacing) = photocraft_text::xcf::qualify_placed_spaced_colored_rectilinear_source(
        state.source_items.get(idx)?,
        state.resources.get(rid)?,
        origin,
        &Interrupt::NONE,
    )
    .ok()?;
    Some(spacing)
}
pub fn letter_spacing_edit_candidate(doc: &Document, owner: LayerId) -> bool {
    let Ok((idx, rid, origin)) = index(doc, owner, &Interrupt::NONE) else { return false };
    let Some(state) = &doc.xcf_state else { return false };
    let Some(item) = state.source_items.get(idx) else { return false };
    let Some(resource) = state.resources.get(rid) else { return false };
    photocraft_text::xcf::qualify_placed_spaced_colored_rectilinear_source(item, resource, origin, &Interrupt::NONE)
        .is_ok_and(|(request, _, _, _)| matches!(request.size_px, 16. | 32. | 40.))
}
/// Current operative position; locked sources retain read-only inspection.
pub fn current_text_origin(doc: &Document, owner: LayerId) -> Option<[i32; 2]> {
    let (idx, rid, original) = qualified_index(doc, owner, &Interrupt::NONE, false).ok()?;
    let state = doc.xcf_state.as_ref()?;
    photocraft_text::xcf::qualify_placed_spaced_colored_rectilinear_source(state.source_items.get(idx)?, state.resources.get(rid)?, original, &Interrupt::NONE)
        .ok()
        .map(|(request, _, _, _)| request.origin)
}
pub fn position_edit_candidate(doc: &Document, owner: LayerId) -> bool {
    index(doc, owner, &Interrupt::NONE).is_ok() && current_text_origin(doc, owner).is_some()
}
/// Locked qualified sources retain read-only operative line-spacing metadata.
pub fn current_line_spacing(doc: &Document, owner: LayerId) -> Option<f32> {
    let (idx, rid, origin) = qualified_index(doc, owner, &Interrupt::NONE, false).ok()?;
    let state = doc.xcf_state.as_ref()?;
    let source = photocraft_text::xcf::qualify_placed_layout_colored_rectilinear_source(
        state.source_items.get(idx)?,
        state.resources.get(rid)?,
        origin,
        &Interrupt::NONE,
    )
    .ok()?;
    if let photocraft_text::xcf::RectilinearLineLayout::TwoLines { line_spacing_px } = source.layout { Some(line_spacing_px) } else { None }
}
pub fn line_spacing_edit_candidate(doc: &Document, owner: LayerId) -> bool {
    index(doc, owner, &Interrupt::NONE).is_ok() && current_line_spacing(doc, owner).is_some()
}

fn owner(p: &Value, field: &str) -> Result<LayerId> {
    let object = p.as_object().ok_or_else(|| bad("parameters must be an object"))?;
    if object.len() != 2 || object.keys().any(|k| k != "layerId" && k != field) {
        return Err(bad("exact layerId and edit value parameters required"));
    }
    let id = p.get("layerId").and_then(Value::as_str).ok_or_else(|| bad("layerId requires an exact decimal U64 string"))?;
    if id.is_empty() || id.len() > 20 || id.starts_with('0') || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("invalid layerId"));
    }
    Ok(LayerId(id.parse().map_err(|_| bad("layerId exceeds U64"))?))
}
enum Edit<'a> {
    Content(&'a str),
    Position([i32; 2]),
    Spacing(f32),
    LineSpacing(f32),
    Size(f32),
    Color(photocraft_text::xcf::TextColor),
}
fn update(doc: &mut Document, owner: LayerId, edit: &Edit<'_>, ctl: &Interrupt<'_>) -> Result<bool> {
    use photocraft_text::xcf::{ExactFont, ScaledText, TextSourceError, set_rectilinear_font_size, set_scaled_rectilinear_content};
    let (idx, rid, origin) = index(doc, owner, ctl)?;
    let font = ExactFont::bundled_inter(ctl).map_err(|e| text_error(e, ctl))?;
    let state = doc.xcf_state.as_mut().ok_or_else(|| bad("missing source state"))?;
    let resource = state.resources.get(rid).ok_or_else(|| bad("missing exact font"))?;
    let item = state.source_items.get_mut(idx).ok_or_else(|| bad("missing text owner"))?;
    let source = photocraft_text::xcf::qualify_placed_layout_colored_rectilinear_source(item, resource, origin, ctl).map_err(|e| text_error(e, ctl))?;
    let current = source.request;
    let mode = source.antialias;
    let mut retained = String::new();
    retained.try_reserve_exact(current.content.len()).map_err(|_| bad("content allocation failed"))?;
    retained.push_str(current.content);
    let size = current.size_px;
    let multiline = matches!(source.layout, photocraft_text::xcf::RectilinearLineLayout::TwoLines { .. });
    let limit = if multiline { 16 << 20 } else { 8 << 20 };
    let mut held = 0_u64;
    let mut charge = |n: u64| {
        held = held.checked_add(n).ok_or(TextSourceError::Budget)?;
        if held > limit { Err(TextSourceError::Budget) } else { Ok(()) }
    };
    if multiline {
        let result = match edit {
            Edit::Content(content) => photocraft_text::xcf::set_multiline_rectilinear_content(item, resource, origin, content, &font, limit, ctl, &mut charge),
            Edit::LineSpacing(spacing) => photocraft_text::xcf::set_rectilinear_line_spacing(item, resource, origin, *spacing, &font, limit, ctl, &mut charge),
            _ => return Err(bad("multiline editing currently supports content and line spacing only")),
        };
        return result.map_err(|e| text_error(e, ctl));
    }
    let result = match edit {
        Edit::LineSpacing(_) => return Err(bad("line spacing requires a qualified two-line source")),
        Edit::Position(next) => photocraft_text::xcf::set_rectilinear_origin(item, resource, origin, *next, &font, 8 << 20, ctl, &mut charge),
        Edit::Spacing(spacing) => photocraft_text::xcf::set_rectilinear_letter_spacing(item, resource, origin, *spacing, &font, 8 << 20, ctl, &mut charge),
        Edit::Color(color) => photocraft_text::xcf::set_rectilinear_color(item, resource, origin, *color, &font, 8 << 20, ctl, &mut charge),
        Edit::Content(content) => {
            set_scaled_rectilinear_content(item, resource, ScaledText { content, origin, size_px: size }, &font, 8 << 20, ctl, &mut charge)
        }
        Edit::Size(size_px) => {
            if mode != photocraft_text::xcf::RectilinearAntialias::AreaCoverage {
                return Err(bad("font size editing requires qualified AA text"));
            }
            set_rectilinear_font_size(item, resource, ScaledText { content: &retained, origin, size_px: *size_px }, &font, 8 << 20, ctl, &mut charge)
        }
    };
    result.map_err(|e| text_error(e, ctl))
}
fn apply(s: &mut Session, owner: LayerId, edit: Edit<'_>, ctl: &Interrupt<'_>) -> Result<bool> {
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let mut proposed = (**doc).clone();
    let changed = update(&mut proposed, owner, &edit, ctl)?;
    if changed {
        crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&proposed, ctl)?;
        ctl.progress(1.);
        if ctl.cancelled() {
            return Err(EngineError::Cancelled);
        }
        let label = match edit {
            Edit::Content(_) => "Edit GIMP Text Content",
            Edit::Position(_) => "Edit GIMP Text Position",
            Edit::Size(_) => "Edit GIMP Text Font Size",
            Edit::Color(_) => "Edit GIMP Text Color",
            Edit::Spacing(_) => "Edit GIMP Text Letter Spacing",
            Edit::LineSpacing(_) => "Edit GIMP Text Line Spacing",
        };
        s.edit_with_interrupt(label, ctl, |doc, _| {
            update(doc, owner, &edit, ctl)?;
            Ok(())
        })?;
    }
    if ctl.cancelled() {
        return Err(EngineError::Cancelled);
    }
    Ok(changed)
}
pub fn set_content_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let owner = owner(p, "content")?;
    let content = p.get("content").and_then(Value::as_str).ok_or_else(|| bad("content must be UTF8 string"))?;
    if !(1..=9).contains(&content.len()) || content.bytes().any(|b| !(32..=126).contains(&b) && b != b'\n') {
        return Err(bad("content outside bounded exact-font rectilinear domain"));
    }
    let changed = apply(s, owner, Edit::Content(content), ctl)?;
    Ok(json!({"layerId":owner.0.to_string(),"updated":changed}))
}
pub fn set_line_spacing_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let result = (|| {
        let owner = owner(p, "lineSpacingPx")?;
        let value = p.get("lineSpacingPx").and_then(Value::as_u64).ok_or_else(|| bad("lineSpacingPx requires an exact integer"))?;
        let spacing = match value {
            0 => 0.,
            2 => 2.,
            4 => 4.,
            _ => return Err(bad("qualified additional line spacing is0,2,4 pixels")),
        };
        let updated = apply(s, owner, Edit::LineSpacing(spacing), ctl)?;
        Ok(json!({"layerId":owner.0.to_string(),"lineSpacingPx":value,"updated":updated}))
    })();
    result.map_err(|e| match e {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: LINE_SPACING.into(), msg },
        other => other,
    })
}
fn run_line_spacing(s: &mut Session, p: &Value) -> Result<Value> {
    set_line_spacing_with_interrupt(s, p, &Interrupt::NONE)
}
pub fn set_font_size_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    font_size_inner(s, p, ctl).map_err(|e| match e {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: SIZE.into(), msg },
        other => other,
    })
}
fn font_size_inner(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let owner = owner(p, "sizePx")?;
    let size = p.get("sizePx").and_then(Value::as_u64).ok_or_else(|| bad("sizePx requires an exact integer"))?;
    let size_px = match size {
        16 => 16.,
        24 => 24.,
        32 => 32.,
        40 => 40.,
        _ => return Err(bad("qualified AA sizes are 16,24,32,40 pixels")),
    };
    let changed = apply(s, owner, Edit::Size(size_px), ctl)?;
    Ok(json!({"layerId":owner.0.to_string(),"sizePx":size,"updated":changed}))
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    set_content_with_interrupt(s, p, &Interrupt::NONE)
}
fn run_size(s: &mut Session, p: &Value) -> Result<Value> {
    set_font_size_with_interrupt(s, p, &Interrupt::NONE)
}
pub fn set_color_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    use photocraft_text::xcf::{TextColor, TextColorEncoding};
    let result = (|| {
        let object = p.as_object().ok_or_else(|| bad("parameters must be an object"))?;
        if object.len() != 3 || object.keys().any(|k| !matches!(k.as_str(), "layerId" | "encoding" | "rgbaF32Bits")) {
            return Err(bad("exact layerId/encoding/rgbaF32Bits required"));
        }
        let owner = owner(&json!({"layerId":p.get("layerId"),"color":true}), "color")?;
        let encoding = match p.get("encoding").and_then(Value::as_str) {
            Some("linear") => TextColorEncoding::Linear,
            Some("nonlinear") => TextColorEncoding::Nonlinear,
            _ => return Err(bad("unknown explicit color encoding")),
        };
        let values = p.get("rgbaF32Bits").and_then(Value::as_array).filter(|v| v.len() == 4).ok_or_else(|| bad("four exact f32 hex strings required"))?;
        let mut bits = [0_u32; 4];
        for (slot, value) in bits.iter_mut().zip(values) {
            let hex = value
                .as_str()
                .filter(|h| h.len() == 8 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
                .ok_or_else(|| bad("lowercase eight-digit f32 hex required"))?;
            *slot = u32::from_str_radix(hex, 16).map_err(|_| bad("invalid f32 bits"))?;
        }
        let color = TextColor::checked(encoding, bits).map_err(|e| text_error(e, ctl))?;
        let changed = apply(s, owner, Edit::Color(color), ctl)?;
        Ok(json!({"changed":changed}))
    })();
    result.map_err(|e| match e {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: COLOR.into(), msg },
        other => other,
    })
}
fn run_color(s: &mut Session, p: &Value) -> Result<Value> {
    set_color_with_interrupt(s, p, &Interrupt::NONE)
}
pub fn set_letter_spacing_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let result = (|| {
        let owner = owner(p, "letterSpacingPx")?;
        let spacing = match p.get("letterSpacingPx").and_then(Value::as_u64) {
            Some(0) => 0.,
            Some(2) => 2.,
            Some(4) => 4.,
            _ => return Err(bad("letterSpacingPx requires exact integer 0, 2 or 4")),
        };
        let updated = apply(s, owner, Edit::Spacing(spacing), ctl)?;
        Ok(json!({"layerId":owner.0.to_string(),"letterSpacingPx":spacing as u32,"updated":updated}))
    })();
    result.map_err(|e| match e {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: SPACING.into(), msg },
        other => other,
    })
}
fn run_spacing(s: &mut Session, p: &Value) -> Result<Value> {
    set_letter_spacing_with_interrupt(s, p, &Interrupt::NONE)
}
pub fn set_position_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let result = (|| {
        let owner = owner(p, "originPx")?;
        let values = p.get("originPx").and_then(Value::as_array).ok_or_else(|| bad("originPx requires exactly two integer coordinates"))?;
        if values.len() != 2 {
            return Err(bad("originPx requires exactly two integer coordinates"));
        }
        let mut origin = [0_i32; 2];
        for (slot, value) in origin.iter_mut().zip(values) {
            let integer = value.as_i64().ok_or_else(|| bad("originPx coordinates require exact signed integers"))?;
            if !(-(1_i64 << 29)..=(1_i64 << 29)).contains(&integer) {
                return Err(bad("originPx exceeds safe coordinates"));
            }
            *slot = i32::try_from(integer).map_err(|_| bad("originPx exceeds I32"))?;
        }
        let updated = apply(s, owner, Edit::Position(origin), ctl)?;
        Ok(json!({"layerId":owner.0.to_string(),"originPx":origin,"updated":updated}))
    })();
    result.map_err(|e| match e {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: POSITION.into(), msg },
        other => other,
    })
}
fn run_position(s: &mut Session, p: &Value) -> Result<Value> {
    set_position_with_interrupt(s, p, &Interrupt::NONE)
}

/// Read-only current UTF-8 intervals; locks do not hide retained semantics.
/// The fixed eight-byte qualified domain bounds the returned interval count.
pub fn current_underline_ranges(doc: &Document, owner: LayerId) -> Option<Vec<[usize; 2]>> {
    let (idx, _, _) = source_index(doc, owner, &Interrupt::NONE, false).ok()?;
    let item = doc.xcf_state.as_ref()?.source_items.get(idx)?;
    let qualified = photocraft_compose::gimp_text_items::qualify_underlined_source(doc, item, &Interrupt::NONE).ok()?;
    let flags = qualified.runs.underlined_bytes();
    let mut ranges = Vec::new();
    ranges.try_reserve_exact(8).ok()?;
    let mut start = None;
    for (i, value) in flags.iter().copied().take(qualified.request.content.len()).chain(std::iter::once(false)).enumerate() {
        if value && start.is_none() {
            start = Some(i);
        }
        if !value && let Some(a) = start.take() {
            ranges.push([a, i]);
        }
    }
    Some(ranges)
}
/// Dispatch still independently validates both ordinary and inherited locks.
pub fn underline_edit_candidate(doc: &Document, owner: LayerId) -> bool {
    source_index(doc, owner, &Interrupt::NONE, true).is_ok() && current_underline_ranges(doc, owner).is_some()
}

fn update_underline(doc: &mut Document, owner: LayerId, ranges: &[[usize; 2]], ctl: &Interrupt<'_>) -> Result<bool> {
    let (idx, rid, origin) = source_index(doc, owner, ctl, true)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing source state"))?;
    let item = state.source_items.get(idx).ok_or_else(|| bad("missing text source"))?;
    photocraft_compose::gimp_text_items::qualify_underlined_source(doc, item, ctl)
        .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    let font = photocraft_text::xcf::ExactFont::bundled_inter(ctl).map_err(|e| text_error(e, ctl))?;
    let state = doc.xcf_state.as_mut().ok_or_else(|| bad("missing source state"))?;
    let resource = state.resources.get(rid).ok_or_else(|| bad("missing font"))?;
    let item = state.source_items.get_mut(idx).ok_or_else(|| bad("missing source"))?;
    let mut used = 0_u64;
    photocraft_text::xcf::set_underlined_ranges(item, resource, origin, ranges, &font, 8 << 20, ctl, &mut |n| {
        used = used.checked_add(n).ok_or(photocraft_text::xcf::TextSourceError::Budget)?;
        if used > 8 << 20 { Err(photocraft_text::xcf::TextSourceError::Budget) } else { Ok(()) }
    })
    .map_err(|e| text_error(e, ctl))
}
/// Current semantic underline ranges; no opaque source bytes or font authorities
/// are rewritten. All inputs and pipeline output are checked before history.
pub fn set_underline_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let result = (|| {
        let owner = owner(p, "ranges")?;
        let values = p
            .get("ranges")
            .and_then(Value::as_array)
            .filter(|v| v.len() <= 8)
            .ok_or_else(|| bad("ranges requires at most8 sorted nonoverlapping UTF8 byte intervals"))?;
        let mut ranges = [[0_usize; 2]; 8];
        for (slot, v) in ranges.iter_mut().zip(values) {
            let pair = v.as_array().filter(|v| v.len() == 2).ok_or_else(|| bad("range requires exact start/end integer pair"))?;
            for (n, value) in slot.iter_mut().zip(pair) {
                *n = value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("range offsets require exact nonnegative integers"))?;
            }
        }
        let ranges = ranges.get(..values.len()).ok_or_else(|| bad("range limit"))?;
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let mut proposed = (**doc).clone();
        let updated = update_underline(&mut proposed, owner, ranges, ctl)?;
        if updated {
            crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&proposed, ctl)?;
            ctl.progress(1.);
            if ctl.cancelled() {
                return Err(EngineError::Cancelled);
            }
            s.edit_with_interrupt("Edit GIMP Text Underline", ctl, |doc, _| {
                update_underline(doc, owner, ranges, ctl)?;
                Ok(())
            })?;
        }
        if ctl.cancelled() {
            return Err(EngineError::Cancelled);
        }
        Ok(json!({"layerId":owner.0.to_string(),"updated":updated}))
    })();
    result.map_err(|e| match e {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: UNDERLINE.into(), msg },
        other => other,
    })
}
fn run_underline(s: &mut Session, p: &Value) -> Result<Value> {
    set_underline_with_interrupt(s, p, &Interrupt::NONE)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: UNDERLINE,
            label: "Edit GIMP Text Underline",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"nonzero decimal U64","ranges":[[0,2]]}; at most8 sorted nonoverlapping UTF8 byte intervals [start,end), exact nonnegative integers. Empty ranges removes current underline. Qualified single-line exact-font source only; preserves original opaque AST/cache/raster/name/font."#,
            enabled,
            run: run_underline,
            journal: true,
        },
        CommandSpec {
            id: LINE_SPACING,
            label: "Edit GIMP Text Line Spacing",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"nonzero decimal U64","lineSpacingPx":0|2|4}; exact integer additional baseline spacing on a qualified two-line source, letter spacing0. Zero omits the raw/typed field; retains original cache/font/placement. Single-line domain transitions refused."#,
            enabled,
            run: run_line_spacing,
            journal: true,
        },
        CommandSpec {
            id: POSITION,
            label: "Edit GIMP Text Position",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"nonzero decimal U64","originPx":[11,2]}; exact signed integers bounded to +/-2^29; current geometry must also fit. Original AST/cache/raster bounds/resources/transform remain immutable; returning to original origin removes the PhotoCraft current-placement extension."#,
            enabled,
            run: run_position,
            journal: true,
        },
        CommandSpec {
            id: SPACING,
            label: "Edit GIMP Text Letter Spacing",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"nonzero decimal U64","letterSpacingPx":0|2|4}; zero omits the known spacing field; nonzero spacing supports qualified sizes 16/32/40 only; current 24px nonzero changes are refused. Preserves color/content/size and immutable source cache/font authority."#,
            enabled,
            run: run_spacing,
            journal: true,
        },
        CommandSpec {
            id: COLOR,
            label: "Edit GIMP Text Color",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"nonzero decimal U64","encoding":"linear|nonlinear","rgbaF32Bits":["3f800000","00000000","00000000","3f800000"]}; exact finite unit-range RGBA. Stored color alpha is retained independently from owner opacity; measured stock glyph fill is opaque. Preserves current content/size/cache/font authority."#,
            enabled,
            run: run_color,
            journal: true,
        },
        CommandSpec {
            id: SET,
            label: "Edit GIMP Text Content",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"nonzero decimal U64","content":"single line:1-8 printable ASCII; qualified two-line source:up to8 printable glyphbytes and exactly one newline; domain transitions refused"}; preserves size/mode and immutable source cache/resources."#,
            enabled,
            run,
            journal: true,
        },
        CommandSpec {
            id: SIZE,
            label: "Edit GIMP Text Font Size",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"nonzero decimal U64","sizePx":16|24|32|40}; qualified AA sizes, immutable source raster/cache/font identity."#,
            enabled,
            run: run_size,
            journal: true,
        },
    ]
}
