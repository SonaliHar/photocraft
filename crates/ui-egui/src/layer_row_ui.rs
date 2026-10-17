//! Layers panel row: the right-hand indicators and the name between them and the thumbnails
//! (#144).
//!
//! The indicators (lock, fx badge with its effects triangle, link, the blend-mode label) are
//! laid out first, from the row's right edge inward; the name gets the width that is left and
//! is cut with an ellipsis, so a long name never runs under an indicator at any panel width or
//! UI scale. The blend-mode label is optional: it is dropped before the name gets too narrow.
//! The rects drawn each frame are recorded ([`recorded`]) for tests and automation.

use egui::{Align2, FontId, Galley, Painter, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::Layer;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::icons;
use crate::theme::{self, Tokens};

/// Space kept free at a row's right edge: the overlay scrollbar sits there.
pub const RIGHT_PAD: f32 = 10.0;
/// Gap between indicators, and between the name and the first indicator.
const GAP: f32 = 4.0;
/// Width of an icon indicator (lock, link).
const ICON_W: f32 = 14.0;
/// Width of the effects triangle beside the fx badge.
const TRIANGLE_W: f32 = 10.0;
/// The blend-mode label is shown only while the name keeps at least this much room.
pub const MIN_NAME_W: f32 = 64.0;

/// A right-hand row indicator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator {
    Lock,
    FxTriangle,
    Fx,
    Link,
    Blend,
}

/// Where one row's name and indicators went this frame (screen points).
#[derive(Clone, Debug)]
pub struct RowRects {
    pub layer: u64,
    pub row: Rect,
    /// The name as painted (after truncation); `None` when there was no room for it.
    pub name: Option<Rect>,
    pub indicators: Vec<(Indicator, Rect)>,
}

/// Lay out `items` (right to left, with their widths; `Blend` is optional) inside `row`,
/// leaving the name the span from `name_left`. Returns the indicator rects and the name's
/// right limit.
pub fn layout(row: Rect, name_left: f32, items: &[(Indicator, f32)]) -> (Vec<(Indicator, Rect)>, f32) {
    let mut right = row.right() - RIGHT_PAD;
    let mut out = Vec::with_capacity(items.len());
    for &(kind, w) in items {
        let w = w.max(0.0);
        // The fx badge hugs its triangle; everything else keeps a gap.
        let gap = if kind == Indicator::Fx && out.last().is_some_and(|(k, _)| *k == Indicator::FxTriangle) { 1.0 } else { GAP };
        let left = right - w;
        if kind == Indicator::Blend && left - GAP - name_left < MIN_NAME_W {
            continue;
        }
        out.push((kind, Rect::from_min_max(pos2(left, row.top()), pos2(right, row.bottom()))));
        right = left - gap;
    }
    let name_right = out.last().map_or(right, |(_, r)| r.left() - GAP);
    (out, name_right)
}

/// Width the always-shown indicators of `l` take at the row's right (blend label excluded).
pub fn reserved_width(l: &Layer) -> f32 {
    let mut w = RIGHT_PAD;
    if l.locks.any() {
        w += ICON_W + GAP;
    }
    if !l.effects.items.is_empty() {
        w += TRIANGLE_W + 1.0 + 13.0 + GAP;
    }
    if l.link_group.is_some() {
        w += ICON_W + GAP;
    }
    w
}

/// Group indentation for a row at `depth`: 14 pt a level, squeezed when the panel is so narrow
/// that it would leave the name less than [`MIN_NAME_W`] (`fixed` = everything else in the row).
pub fn indent(depth: usize, row_width: f32, fixed: f32) -> f32 {
    (depth as f32 * 14.0).min((row_width - fixed - MIN_NAME_W).max(0.0))
}

/// A single-line galley cut with "…" to `max_width` (nothing when there's no room).
pub fn truncated(painter: &Painter, text: &str, font: FontId, color: egui::Color32, italics: bool, max_width: f32) -> Option<Arc<Galley>> {
    if max_width < 8.0 {
        return None;
    }
    let format = egui::text::TextFormat { font_id: font, color, italics, ..Default::default() };
    let layout = |s: &str, wrap: bool| {
        let mut job = egui::text::LayoutJob::default();
        job.append(s, 0.0, format.clone());
        if wrap {
            job.wrap = egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        }
        painter.layout_job(job)
    };
    // A right-to-left name (Arabic, Hebrew) too long for the row is cut at its logical end and
    // then put in display order, so its start stays and "…" is at the left (`bidi_text::elide`).
    if let Some(shown) = crate::bidi_text::elide(text, |s| layout(s, false).size().x <= max_width) {
        return Some(layout(&shown, false));
    }
    // Right-to-left names in display order: egui lays words out left to right. Left-to-right
    // names are cut at their right end by egui.
    Some(layout(&crate::bidi_text::visual(text), true))
}

/// Paint `l`'s right-hand indicators in `row` and handle the effects triangle. Returns the
/// name's right limit, the indicator rects, and whether the triangle was clicked this frame (so
/// the row doesn't also treat the click as a selection).
#[allow(clippy::too_many_arguments)]
pub fn indicators(
    ui: &egui::Ui,
    painter: &Painter,
    row: Rect,
    name_left: f32,
    l: &Layer,
    source_blend_label: Option<&str>,
    fx_open: bool,
    actions: &mut Vec<(String, Value)>,
) -> (f32, Vec<(Indicator, Rect)>, bool) {
    let t = Tokens::get(ui.ctx());
    let locked = l.locks.any();
    let fx_font = theme::semibold(11.0);
    let blend_font = FontId::proportional(10.5);
    let has_fx = !l.effects.items.is_empty();
    let show_blend = source_blend_label.is_some() || (l.blend != BlendMode::Normal && l.blend != BlendMode::PassThrough);
    let fx_galley = has_fx.then(|| painter.layout_no_wrap("fx".into(), fx_font, t.text_dim));
    let blend_galley = show_blend.then(|| painter.layout_no_wrap(tl!(source_blend_label.unwrap_or(l.blend.label())).into(), blend_font, t.text_faint));
    let mut items = Vec::new();
    if locked {
        items.push((Indicator::Lock, ICON_W));
    }
    if let Some(g) = &fx_galley {
        items.push((Indicator::FxTriangle, TRIANGLE_W));
        items.push((Indicator::Fx, g.size().x));
    }
    if l.link_group.is_some() {
        items.push((Indicator::Link, ICON_W));
    }
    if let Some(g) = &blend_galley {
        items.push((Indicator::Blend, g.size().x));
    }
    let (rects, name_right) = layout(row, name_left, &items);
    let mut clicked = false;
    let cy = row.center().y;
    for &(kind, r) in &rects {
        match kind {
            Indicator::Lock => icons::paint(ui, Rect::from_center_size(r.center(), vec2(ICON_W, ICON_W)), "lock", 12.0, t.text_faint),
            Indicator::Link => icons::paint(ui, Rect::from_center_size(r.center(), vec2(ICON_W, ICON_W)), "link-2", 12.0, t.text_faint),
            Indicator::Fx => {
                if let Some(g) = fx_galley.clone() {
                    painter.galley(pos2(r.left(), cy - g.size().y / 2.0), g, t.text_dim);
                }
            }
            Indicator::Blend => {
                if let Some(g) = blend_galley.clone() {
                    painter.galley(pos2(r.left(), cy - g.size().y / 2.0), g, t.text_faint);
                }
            }
            Indicator::FxTriangle => {
                let resp = ui.interact(r, ui.id().with(("fx-disclosure", l.id.0)), Sense::click());
                paint_triangle(painter, r.center(), fx_open, if resp.hovered() { t.text } else { t.icon });
                if resp.clicked() {
                    clicked = true;
                    let all = ui.input(|i| i.modifiers.alt);
                    actions.push(("layer.setEffectsExpanded".into(), json!({"layer": l.id.0, "expanded": !fx_open, "all": all})));
                }
                let (verb, name) = (if fx_open { tl!("Collapse") } else { tl!("Expand") }, l.name.clone());
                let tip = if fx_open {
                    tl!("Hide the layer's effects  ({key}-click: all layers)")
                } else {
                    tl!("Show the layer's effects  ({key}-click: all layers)")
                };
                let resp = resp.on_hover_text(crate::i18n::fmt(tip, &[("key", &crate::shortcuts::pretty("Alt"))]));
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{verb} effects {name}")));
            }
        }
    }
    (name_right, rects, clicked)
}

/// A small disclosure triangle: pointing down when open, right when closed.
pub fn paint_triangle(painter: &Painter, c: Pos2, open: bool, color: egui::Color32) {
    let s = 3.0;
    let pts = if open {
        vec![pos2(c.x - s, c.y - s * 0.6), pos2(c.x + s, c.y - s * 0.6), pos2(c.x, c.y + s * 0.6)]
    } else {
        vec![pos2(c.x - s * 0.6, c.y - s), pos2(c.x + s * 0.6, c.y), pos2(c.x - s * 0.6, c.y + s)]
    };
    painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Paint `text` left-aligned at `x`, vertically centred on `cy`, cut to end before `right`.
/// Returns the painted rect.
pub fn label(painter: &Painter, x: f32, cy: f32, right: f32, text: &str, font: FontId, color: egui::Color32) -> Option<Rect> {
    let g = truncated(painter, text, font, color, false, right - x)?;
    let r = Align2::LEFT_CENTER.anchor_size(pos2(x, cy), g.size());
    painter.galley(r.min, g, color);
    Some(r)
}

/// The Layers panel's own items at the top of its panel menu (Photoshop's flyout).
pub fn panel_menu(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui) {
    let can = app.session.is_enabled("layer.setExpanded");
    if ui.add_enabled(can, egui::Button::new(tl!("Collapse All Groups"))).clicked() {
        if let Err(e) = app.run("layer.setExpanded", json!({"all": true, "expanded": false})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        ui.close();
    }
    ui.separator();
    // Photoshop's Clip Thumbnails to Layer Bounds: frame each layer's pixels, not the canvas (#4281).
    let mut clip = app.session.prefs().interface.clip_thumbnails_to_layer_bounds;
    if crate::widgets::checkbox(ui, &mut clip, tl!("Clip Thumbnails to Layer Bounds")).changed() {
        if let Err(e) = app.run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": clip})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        ui.close();
    }
}

fn rects_id() -> egui::Id {
    egui::Id::new("layer-row-rects")
}

/// Start a frame's record (the Layers panel calls it before drawing its rows).
pub fn begin(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(rects_id(), Vec::<RowRects>::new()));
}

pub fn record(ctx: &egui::Context, r: RowRects) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<RowRects>>(rects_id()).push(r));
}

/// The rows the Layers panel drew last frame.
pub fn recorded(ctx: &egui::Context) -> Vec<RowRects> {
    ctx.data(|d| d.get_temp::<Vec<RowRects>>(rects_id())).unwrap_or_default()
}

// ------------------------------------------------------------------ in-place rename (#314)

/// The Layers panel's in-place rename. One at a time, like Photoshop: Enter, Tab or a click
/// anywhere else commits it, Esc cancels it, and starting another rename commits this one.
#[derive(Clone, Debug, PartialEq)]
pub struct Rename {
    pub layer: u64,
    /// The name when the rename started (unchanged text renames nothing).
    pub original: String,
    pub text: String,
    /// Focus was given to the field (only once: asking every frame kept it from ever losing focus).
    pub focused: bool,
    /// The pass its field was last drawn in.
    pub pass: u64,
}

impl Rename {
    /// The command that applies it, if it changes the name.
    pub fn commit_action(&self) -> Option<(String, Value)> {
        let name = self.text.trim();
        (!name.is_empty() && name != self.original).then(|| ("layer.setProps".to_string(), json!({"layer": self.layer, "name": name})))
    }
}

fn rename_id() -> egui::Id {
    egui::Id::new("layer-rename")
}

/// The rename in progress, if any.
pub fn rename(ctx: &egui::Context) -> Option<Rename> {
    ctx.data(|d| d.get_temp::<Rename>(rename_id()))
}

/// The layer being renamed, if any.
pub fn renaming(ctx: &egui::Context) -> Option<u64> {
    rename(ctx).map(|r| r.layer)
}

/// Is a rename field on screen (drawn this pass or the last)? It owns the keyboard meanwhile.
pub fn rename_active(ctx: &egui::Context) -> bool {
    rename(ctx).is_some_and(|r| r.pass + 1 >= ctx.cumulative_pass_nr())
}

fn store(ctx: &egui::Context, r: Option<Rename>) {
    ctx.data_mut(|d| match r {
        Some(r) => {
            d.insert_temp(rename_id(), r);
        }
        None => {
            d.remove::<Rename>(rename_id());
        }
    });
}

/// Start renaming `layer` (now called `name`). A rename open on another layer is committed: its
/// command is returned for the caller to run.
pub fn start_rename(ctx: &egui::Context, layer: u64, name: &str) -> Option<(String, Value)> {
    let previous = rename(ctx);
    if previous.as_ref().is_some_and(|r| r.layer == layer) {
        return None;
    }
    store(ctx, Some(Rename { layer, original: name.to_string(), text: name.to_string(), focused: false, pass: ctx.cumulative_pass_nr() }));
    previous.and_then(|r| r.commit_action())
}

/// End the rename in progress: commit it (its command is returned) or cancel it.
pub fn end_rename(ctx: &egui::Context, commit: bool) -> Option<(String, Value)> {
    let r = rename(ctx)?;
    store(ctx, None);
    if commit { r.commit_action() } else { None }
}

/// Where Tab or ⇧Tab sends a rename: the layer it left and whether the next one is above.
fn next_rename_id() -> egui::Id {
    egui::Id::new("layer-rename-next")
}

/// The rename that Tab (`up` false) or ⇧Tab (`up` true) just left on `layer`, for the Layers panel
/// to continue on the layer below or above, as Photoshop does. Taken once.
pub fn take_next_rename(ctx: &egui::Context) -> Option<(u64, bool)> {
    ctx.data_mut(|d| d.remove_temp::<(u64, bool)>(next_rename_id()))
}

/// Draw the rename field of `layer` in `rect` when it is the one being renamed, and handle its
/// keys and focus. Returns the rename command when it was committed this frame. Tab and ⇧Tab
/// commit too and ask for the next rename ([`take_next_rename`]).
pub fn rename_field(ui: &mut egui::Ui, layer: u64, rect: Rect) -> Option<(String, Value)> {
    let ctx = ui.ctx().clone();
    let mut r = rename(&ctx).filter(|r| r.layer == layer)?;
    let id = egui::Id::new(("layer-rename-field", layer));
    let te = ui.put(rect, egui::TextEdit::singleline(&mut r.text).char_limit(4096).id(id).font(FontId::proportional(12.5)));
    r.pass = ctx.cumulative_pass_nr();
    if !r.focused {
        te.request_focus();
        // Photoshop selects the whole name, so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(&ctx, id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(r.text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(&ctx, id);
        }
        r.focused = true;
        store(&ctx, Some(r));
        return None;
    }
    let (enter, tab, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Escape)));
    if tab {
        let up = ctx.input(|i| i.modifiers.shift);
        ctx.data_mut(|d| d.insert_temp(next_rename_id(), (layer, up)));
    }
    if esc {
        store(&ctx, None);
        None
    } else if enter || tab || te.lost_focus() || !te.has_focus() {
        store(&ctx, None);
        r.commit_action()
    } else {
        store(&ctx, Some(r));
        None
    }
}

#[derive(Clone)]
struct RenameWitness {
    document: photocraft_doc::DocId,
    revision: u64,
    layer: u64,
    snapshot: photocraft_engine::document_snapshot::WeakDocumentSnapshot,
}
fn witness_id() -> egui::Id {
    egui::Id::new("layer-rename-current-witness")
}
fn witness(ctx: &egui::Context) -> Option<RenameWitness> {
    ctx.data(|data| data.get_temp::<RenameWitness>(witness_id()))
}
fn checked_rename_target(app: &crate::PhotocraftApp, target: &RenameWitness) -> Result<(), &'static str> {
    let active = app.session.active().ok_or("layer.rename.ui.stale-target")?;
    let snapshot = target.snapshot.upgrade().ok_or("layer.rename.ui.stale-target")?;
    if active.doc.id != target.document || active.revision != target.revision || !active.doc.shares_state(&snapshot) {
        return Err("layer.rename.ui.stale-target");
    }
    if app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err("layer.rename.ui.context");
    }
    Ok(())
}
fn checked_rename_action(app: &mut crate::PhotocraftApp, target: Option<RenameWitness>, action: Option<(String, Value)>) -> Option<(String, Value)> {
    let (_, mut params) = action?;
    let result = (|| {
        let target = target.ok_or("layer.rename.ui.stale-target")?;
        checked_rename_target(app, &target)?;
        if params.get("layer").and_then(Value::as_u64) != Some(target.layer) {
            return Err("layer.rename.ui.stale-target");
        }
        let name = params.get("name").and_then(Value::as_str).ok_or("layer.rename.ui.invalid-name")?;
        if name.len() > 4096 || name.contains('\0') {
            return Err("layer.rename.ui.invalid-name");
        }
        let object = params.as_object_mut().ok_or("layer.rename.ui.invalid-name")?;
        object.remove("layer");
        object.insert("layerId".into(), json!(target.layer.to_string()));
        object.insert("documentId".into(), json!(target.document.0.to_string()));
        object.insert("expectedRevision".into(), json!(target.revision.to_string()));
        Ok(("layer.renameLayer".into(), params))
    })();
    match result {
        Ok(action) => Some(action),
        Err(error) => {
            app.ui.status = error.into();
            app.ui.status_error = true;
            None
        }
    }
}
/// Production entry point: retain a snapshot witness alongside the legacy edit
/// field without changing its trim/focus/menu behavior or serialized document.
pub fn start_rename_checked(app: &mut crate::PhotocraftApp, ctx: &egui::Context, layer: u64, name: &str) -> Option<(String, Value)> {
    if renaming(ctx) == Some(layer) {
        return None;
    }
    let old = witness(ctx);
    let active = app.session.active()?;
    if name.len() > 4096 || name.contains('\0') || active.doc.layer(photocraft_doc::LayerId(layer)).is_none() {
        app.ui.status = "layer.rename.ui.invalid-name".into();
        app.ui.status_error = true;
        return None;
    }
    let requested_document = active.doc.id;
    let action = start_rename(ctx, layer, name);
    // Commit the previous editor before capturing the next editor's revision.
    // Otherwise that successful rename would invalidate the new target itself.
    if let Some((command, params)) = checked_rename_action(app, old, action)
        && let Err(error) = app.run(&command, params)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    let Some(active) = app
        .session
        .active()
        .filter(|active| active.doc.id == requested_document && active.doc.layer(photocraft_doc::LayerId(layer)).is_some_and(|current| current.name == name))
    else {
        store(ctx, None);
        ctx.data_mut(|data| data.remove::<RenameWitness>(witness_id()));
        app.ui.status = "layer.rename.ui.stale-target".into();
        app.ui.status_error = true;
        return None;
    };
    let target = RenameWitness { document: active.doc.id, revision: active.revision, layer, snapshot: active.doc.downgrade() };
    ctx.data_mut(|data| data.insert_temp(witness_id(), target));
    None
}

/// Complete a row's queued selection before minting the rename revision fence.
/// Selection may advance revision, but cannot replace the captured model or
/// transfer the gesture to another document/layer.
pub(crate) fn start_rename_after_selection(
    app: &mut crate::PhotocraftApp,
    ctx: &egui::Context,
    snapshot: &photocraft_engine::DocumentSnapshot,
    layer: u64,
    name: &str,
    selection_succeeded: bool,
) {
    if !selection_succeeded
        || !app.session.active().is_some_and(|active| {
            active.doc.id == snapshot.id
                && active.doc.shares_state(snapshot)
                && active.active_layer == Some(photocraft_doc::LayerId(layer))
                && active.doc.layer(photocraft_doc::LayerId(layer)).is_some_and(|current| current.name == name)
        })
    {
        app.ui.status = "layer.rename.ui.stale-target".into();
        app.ui.status_error = true;
        return;
    }
    let _ = start_rename_checked(app, ctx, layer, name);
}
pub fn end_rename_checked(app: &mut crate::PhotocraftApp, ctx: &egui::Context, commit: bool) -> Option<(String, Value)> {
    let target = witness(ctx);
    let action = end_rename(ctx, commit);
    ctx.data_mut(|data| data.remove::<RenameWitness>(witness_id()));
    checked_rename_action(app, target, action)
}
pub fn rename_field_checked(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, layer: u64, rect: Rect) -> Option<(String, Value)> {
    let target = witness(ui.ctx());
    let action = rename_field(ui, layer, rect);
    // A Tab continuation is still the original editor's gesture, even when its
    // text was unchanged. Validate that witness before retaining the next cue.
    if ui.data(|data| data.get_temp::<(u64, bool)>(next_rename_id())).is_some_and(|(from, _)| from == layer) {
        let valid =
            target.as_ref().filter(|target| target.layer == layer).ok_or("layer.rename.ui.stale-target").and_then(|target| checked_rename_target(app, target));
        if let Err(error) = valid {
            ui.data_mut(|data| {
                data.remove::<(u64, bool)>(next_rename_id());
                data.remove::<RenameWitness>(witness_id());
            });
            app.ui.status = error.into();
            app.ui.status_error = true;
            return None;
        }
    }
    if renaming(ui.ctx()).is_none() {
        ui.data_mut(|data| data.remove::<RenameWitness>(witness_id()));
    }
    let committing = action.is_some();
    let checked = checked_rename_action(app, target, action);
    if committing && checked.is_none() {
        // Refused text must not transfer its Tab gesture to another editor.
        ui.data_mut(|data| data.remove::<(u64, bool)>(next_rename_id()));
    }
    checked
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod rename_tests;

#[cfg(test)]
mod edit_tests;
