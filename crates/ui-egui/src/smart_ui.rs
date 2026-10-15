//! Layers-panel pieces for smart objects: the thumbnail badge and the Smart Filters sub-rows
//! (eye toggles; double-click a filter to re-open its dialog with the recorded parameters; click
//! to select one; drag onto another smart object to move it there, ⌥-drag to copy it, #4271;
//! drag it up or down its own stack to reorder it).

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, SmartObject};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// What the badge on a layer thumbnail marks the layer as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThumbBadge {
    Smart,
    Shape,
}

/// The badge `l`'s thumbnail carries. Smart objects and shape layers (#4584) show their pixels
/// like any other layer, so only the badge tells them apart.
fn thumb_badge_kind(l: &Layer) -> Option<ThumbBadge> {
    match l.content {
        LayerContent::Smart(_) => Some(ThumbBadge::Smart),
        LayerContent::Shape(_) => Some(ThumbBadge::Shape),
        _ => None,
    }
}

/// Where the badge sits: over the bottom-right corner of the thumbnail.
fn thumb_badge_rect(thumb: Rect) -> Rect {
    Rect::from_min_size(pos2(thumb.right() - 9.0, thumb.bottom() - 9.0), vec2(11.0, 11.0))
}

/// The shape badge's path, inset from the badge so its corner anchors stay inside the frame.
const SHAPE_PATH_INSET: f32 = 3.0;
/// Side of an anchor point on the shape badge's path.
const SHAPE_ANCHOR: f32 = 3.0;

/// Smart-object or shape-layer badge in the bottom-right corner of a layer thumbnail.
pub fn thumb_badge(ui: &egui::Ui, l: &Layer, thumb: Rect) {
    let Some(kind) = thumb_badge_kind(l) else { return };
    let t = Tokens::get(ui.ctx());
    let r = thumb_badge_rect(thumb);
    ui.painter().rect_filled(r, 2.0, t.field);
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    match kind {
        ThumbBadge::Smart => icons::paint(ui, r, "app-window", 9.0, t.icon),
        // Photoshop's shape mark: a path with an anchor point on each corner. Painted rather
        // than an icon, so the anchors stay solid squares at this size.
        ThumbBadge::Shape => {
            let path = r.shrink(SHAPE_PATH_INSET);
            ui.painter().rect_stroke(path, 0.0, Stroke::new(1.0, t.icon), egui::StrokeKind::Middle);
            for anchor in [path.left_top(), path.right_top(), path.right_bottom(), path.left_bottom()] {
                ui.painter().rect_filled(Rect::from_center_size(anchor, vec2(SHAPE_ANCHOR, SHAPE_ANCHOR)), 0.0, t.icon);
            }
        }
    }
}

/// The display name of a smart filter (its command's label without the ellipsis). A Photoshop
/// filter PhotoCraft doesn't implement shows its own name and is marked as kept as is.
fn filter_label(f: &photocraft_doc::SmartFilter) -> String {
    let command = f.command.as_str();
    if command == photocraft_engine::smart_cmds::UNSUPPORTED_FILTER {
        let name = f.params.get("name").and_then(Value::as_str).unwrap_or("Smart filter");
        return format!("{} (kept, not editable)", name.trim_end_matches("...").trim_end_matches('…'));
    }
    photocraft_engine::commands::find(command)
        .map_or_else(|| command.rsplit('.').next().unwrap_or(command).to_string(), |c| tl!(c.label).trim_end_matches('…').to_string())
}

/// Smart Filters header + one row per filter (top filter first, as in Photoshop) under a smart
/// layer. Edits are queued in `actions` as engine commands.
pub fn filter_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer, depth: usize, actions: &mut Vec<(String, Value)>) {
    let LayerContent::Smart(sm) = &l.content else { return };
    if sm.smart_filters.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let indent = 30.0 + depth as f32 * 14.0 + 34.0;
    let Some(st) = app.session.active() else { return };
    let (active, doc) = (st.active_layer == Some(l.id), st.doc.id);
    let picked = ui.ctx().data(|d| d.get_temp::<FilterDrag>(selected_key(doc)));
    let rows = std::iter::once(None).chain((0..sm.smart_filters.len()).rev().map(Some));
    for index in rows {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
        if resp.drag_started() {
            ui.ctx().data_mut(|d| d.insert_temp::<FilterDrag>(drag_key(), (l.id.0, index)));
        }
        if let Some((to, action)) = reorder_drop(ui, l, sm.smart_filters.len(), index, rect) {
            // The moved filter ends up selected, with its smart object active, as after a click.
            ui.ctx().data_mut(|d| d.insert_temp::<FilterDrag>(selected_key(doc), (l.id.0, Some(to))));
            actions.push(action);
            if !active {
                actions.push(("layer.select".into(), json!({"layer": l.id.0})));
            }
        }
        // Selected as in Photoshop: the row stays highlighted while its smart object is active.
        let selected = active && picked == Some((l.id.0, index));
        if selected {
            ui.painter().rect_filled(rect, 0.0, if t.pro { t.row_selected } else { t.hover });
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.35));
        }
        if t.pro {
            ui.painter().line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        }
        let (name, on) = match index {
            None => ("Smart Filters".to_string(), sm.filters_enabled),
            Some(i) => (filter_label(&sm.smart_filters[i]), sm.smart_filters[i].visible && sm.filters_enabled),
        };
        let eye = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        let eye_resp = ui.interact(eye, ui.id().with(("sf-eye", l.id.0, index)), Sense::click());
        let shown = match index {
            None => sm.filters_enabled,
            Some(i) => sm.smart_filters[i].visible,
        };
        if shown {
            icons::paint(ui, eye, "eye", 12.0, t.icon);
        }
        if eye_resp.clicked() {
            actions.push(match index {
                None => ("layer.smartFilter.disableSmartFilters".into(), json!({"layer": l.id.0})),
                Some(i) => ("layer.smartFilter.setVisible".into(), json!({"layer": l.id.0, "index": i})),
            });
        }
        let x = rect.left() + indent + if index.is_none() { 0.0 } else { 16.0 };
        if index.is_none() {
            let sw = Rect::from_center_size(pos2(x - 12.0, rect.center().y), vec2(14.0, 10.0));
            ui.painter().rect_filled(sw, 1.0, if sm.filter_mask.as_ref().is_some_and(|m| m.enabled) { Color32::from_gray(200) } else { Color32::WHITE });
            ui.painter().rect_stroke(sw, 1.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
        }
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, &name, egui::FontId::proportional(11.5), if on { t.text_dim } else { t.text_faint });
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name));
        if resp.clicked() {
            ui.ctx().data_mut(|d| d.insert_temp::<FilterDrag>(selected_key(doc), (l.id.0, index)));
            if !active {
                actions.push(("layer.select".into(), json!({"layer": l.id.0})));
            }
        }
        if let Some(i) = index
            && resp.double_clicked()
        {
            open_editor(app, ui.ctx(), l.id.0, sm, i);
        }
    }
}

/// Smart filter rows being dragged: the smart object and the filter index (`None` = the Smart
/// Filters row, all of them). Also what a click selects.
pub(crate) type FilterDrag = (u64, Option<usize>);

pub(crate) fn drag_key() -> egui::Id {
    egui::Id::new("smart-filter-drag")
}

/// The selected smart filter row of a document.
fn selected_key(doc: photocraft_doc::DocId) -> egui::Id {
    egui::Id::new(("smart-filter-selected", doc))
}

/// The selected smart filter row, while its smart object is active and the filter still exists.
pub(crate) fn selected(ctx: &egui::Context, doc: &Document, active: Option<LayerId>) -> Option<FilterDrag> {
    let (layer, filter) = ctx.data(|d| d.get_temp::<FilterDrag>(selected_key(doc.id)))?;
    if active != Some(LayerId(layer)) {
        return None;
    }
    let LayerContent::Smart(sm) = &doc.layer(LayerId(layer))?.content else { return None };
    let exists = match filter {
        Some(i) => i < sm.smart_filters.len(),
        None => !sm.smart_filters.is_empty(),
    };
    exists.then_some((layer, filter))
}

/// Forgets the selected smart filter row: its smart object's own row was clicked, or the filter
/// was deleted (its index would now name the next one).
pub(crate) fn deselect(ctx: &egui::Context, doc: photocraft_doc::DocId) {
    ctx.data_mut(|d| d.remove::<FilterDrag>(selected_key(doc)));
}

/// While smart filters are dragged: their name by the pointer, and the copy cursor while ⌥ is held.
pub(crate) fn drag_feedback(ctx: &egui::Context, doc: &Document) {
    let Some((from, filter)) = ctx.data(|d| d.get_temp::<FilterDrag>(drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let label = match (filter, doc.layer(LayerId(from)).map(|l| &l.content)) {
        (Some(i), Some(LayerContent::Smart(sm))) => sm.smart_filters.get(i).map_or_else(String::new, filter_label),
        _ => "Smart Filters".to_string(),
    };
    crate::layer_transfer::ghost(ctx, p, &label);
    if ctx.input(|i| i.modifiers.alt) {
        ctx.set_cursor_icon(egui::CursorIcon::Copy);
    }
}

/// The command for dropping the smart filters of `from` on the smart object `target`: they move
/// there, or with ⌥ held on release are copied (Photoshop).
pub(crate) fn drop_action(from: u64, filter: Option<usize>, target: LayerId, copy: bool) -> (String, Value) {
    let mut payload = json!({"from": from, "to": target.0, "copy": copy});
    if let (Some(i), Some(o)) = (filter, payload.as_object_mut()) {
        o.insert("filter".into(), json!(i));
    }
    ("layer.smartFilter.transfer".into(), payload)
}

/// Another smart object's row accepts the smart filters being dragged: outlined while the pointer
/// is over it, dropped on release. Other layers don't take them, as in Photoshop.
pub(crate) fn drop_on_row(ctx: &egui::Context, ui: &egui::Ui, l: &Layer, rect: Rect, actions: &mut Vec<(String, Value)>) {
    let Some((from, filter)) = ctx.data(|d| d.get_temp::<FilterDrag>(drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if from == l.id.0 || !matches!(l.content, LayerContent::Smart(_)) || !rect.contains(p) {
        return;
    }
    let t = Tokens::get(ctx);
    ui.painter().rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    if ctx.input(|i| i.pointer.any_released()) {
        actions.push(drop_action(from, filter, l.id, ctx.input(|i| i.modifiers.alt)));
    }
}

/// Where `layer.smartFilter.move` puts filter `src` (stack index, 0 = bottom) dropped on the row
/// of filter `row` (`None`: the Smart Filters row, i.e. on top of the stack), `above` or below
/// it, in a stack of `n`. `None` when the filter would stay where it is.
fn reorder_target(src: usize, row: Option<usize>, above: bool, n: usize) -> Option<usize> {
    let to = match row {
        None => n.checked_sub(1)?,
        Some(i) if i == src => return None,
        // Above a row is just over it in the stack, below a row just under it. `move` removes
        // the filter first, so the filters over `src` shift down by one.
        Some(i) if above => {
            if src < i {
                i
            } else {
                i + 1
            }
        }
        Some(i) => {
            if src < i {
                i - 1
            } else {
                i
            }
        }
    };
    (to < n && to != src).then_some(to)
}

/// A filter dragged up or down its own smart object's stack (Photoshop): over another row of the
/// stack a line marks where it lands, above or below that row (the Smart Filters row: on top),
/// and the release moves it there. The bottom filter is applied first, so the order changes the
/// result (Unsharp Mask over a blur sharpens the blurred image; under it, the sharpening is
/// blurred). Returns the filter's new index and the command.
fn reorder_drop(ui: &egui::Ui, l: &Layer, n: usize, row: Option<usize>, rect: Rect) -> Option<(usize, (String, Value))> {
    let ctx = ui.ctx();
    let (from, src) = ctx.data(|d| d.get_temp::<FilterDrag>(drag_key()))?;
    let (src, p) = (src?, ctx.input(|i| i.pointer.interact_pos())?);
    if from != l.id.0 || !rect.contains(p) {
        return None;
    }
    let above = p.y < rect.center().y;
    let to = reorder_target(src, row, above, n)?;
    let t = Tokens::get(ctx);
    let y = if above && row.is_some() { rect.top() } else { rect.bottom() };
    ui.painter().line_segment([pos2(rect.left() + 30.0, y), pos2(rect.right(), y)], Stroke::new(2.0, t.accent));
    ctx.input(|i| i.pointer.any_released()).then(|| (to, ("layer.smartFilter.move".into(), json!({"layer": l.id.0, "index": src, "to": to}))))
}

/// Smart filters dropped on the footer's Delete button: a filter's row deletes that filter, the
/// Smart Filters row clears them all.
pub(crate) fn trash_action((layer, filter): FilterDrag) -> (String, Value) {
    match filter {
        Some(i) => ("layer.smartFilter.delete".into(), json!({"layer": layer, "index": i})),
        None => ("layer.smartFilter.clearSmartFilters".into(), json!({"layer": layer})),
    }
}

/// Re-opens a smart filter's dialog with its recorded parameters; OK runs
/// `layer.smartFilter.setParams` instead of adding another filter (see `dialogs::confirm`).
pub fn open_editor(app: &mut PhotocraftApp, ctx: &egui::Context, layer: u64, sm: &SmartObject, index: usize) {
    let Some(f) = sm.smart_filters.get(index) else { return };
    // Camera Raw has its own full-window dialog rather than a generic parameter form.
    if f.command == photocraft_engine::lens_cmds::RAW {
        if let Err(e) = crate::camera_raw_ui::open_smart_filter(app, ctx, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    // So has the Filter Gallery.
    if f.command == "filter.filterGallery" {
        if let Err(e) = crate::gallery_ui::open_smart_filter(app, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    // An adjustment recorded on the smart object (Brightness/Contrast, Levels…) re-opens its own
    // Image › Adjustments editor, like the filters above.
    if crate::adjust_dialog::has_dialog(&f.command) {
        if let Some(id) = crate::adjust_dialog::open(app, &f.command) {
            edit_filter(app, id, layer, index, &f.params);
        }
        return;
    }
    if !crate::filter_dialog::has_dialog(&f.command) {
        return;
    }
    let Some(id) = crate::filter_dialog::open(app, &f.command) else { return };
    edit_filter(app, id, layer, index, &f.params);
}

/// Hands the open dialog `id` the smart filter's recorded `params` and makes OK update that
/// filter (`layer.smartFilter.setParams`, see [`confirm_command`]) instead of adding another.
fn edit_filter(app: &mut PhotocraftApp, id: u64, layer: u64, index: usize, params: &Value) {
    let mode = app.session.active().map_or(photocraft_doc::ColorMode::Rgb, |s| s.doc.mode);
    let Some(d) = app.ui.dialog_mut(id) else { return };
    // The live preview would stack a second copy of the filter on the smart object.
    d.fields.remove("__preview");
    d.fields.insert("__smartFilter".into(), json!({"layer": layer, "index": index}));
    // An adjustment dialog holds the kind's complete settings: show the ones the filter renders
    // with, read as the engine reads the recorded params. Copying only the keys the defaults
    // have would drop the others (a monochrome Channel Mixer's gray row, a pencil Curves), and
    // OK on the untouched dialog would change the image.
    let adjustment = d
        .fields
        .get("__adjust")
        .and_then(Value::as_str)
        .and_then(|kind| photocraft_engine::adjust_params::from_params(kind, params, None, mode).ok())
        .map(|adj| photocraft_engine::adjust_params::to_params(&adj));
    match (adjustment, params) {
        (Some(Value::Object(p)), _) => {
            d.fields.retain(|k, _| k.starts_with("__"));
            d.fields.extend(p);
            // OK sets these complete settings, keys the editor drops included (`confirm_command`).
            d.fields.insert("__smartFilter".into(), json!({"layer": layer, "index": index, "replace": true}));
        }
        (_, Value::Object(p)) => {
            for (k, v) in p {
                if d.fields.contains_key(k) {
                    d.fields.insert(k.clone(), v.clone());
                }
            }
        }
        _ => {}
    }
}

/// The command a confirmed filter dialog runs: smart-filter edits update the recorded filter.
pub fn confirm_command(fields: &serde_json::Map<String, Value>, command: String, params: Value) -> (String, Value) {
    match fields.get("__smartFilter") {
        Some(sf) => {
            let mut p = json!({"layer": sf["layer"], "index": sf["index"], "params": params});
            if sf.get("replace") == Some(&Value::Bool(true)) {
                p["replace"] = Value::Bool(true);
            }
            ("layer.smartFilter.setParams".into(), p)
        }
        None => (command, params),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pixel layer, a shape layer and a smart object, in that order.
    fn layers() -> [Layer; 3] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        // An empty layer can't become a smart object: fill it first.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        let shape = s.execute("shape.create", json!({"kind": "rect", "rect": [8, 8, 40, 24], "fill": "#cc3333"})).unwrap()["layer"].as_u64().unwrap();
        let pixel = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let doc = &s.active().unwrap().doc;
        [pixel, shape, smart].map(|id| doc.layer(photocraft_doc::LayerId(id)).unwrap().clone())
    }

    /// Stack [A, B, C] (A at the bottom, drawn last): where a dragged filter lands.
    #[test]
    fn a_filter_dropped_above_or_below_a_row_lands_next_to_it() {
        // A over C: [B, C, A]. A under C: [B, A, C].
        assert_eq!(reorder_target(0, Some(2), true, 3), Some(2));
        assert_eq!(reorder_target(0, Some(2), false, 3), Some(1));
        // C under A: [C, A, B]. C over A: [A, C, B].
        assert_eq!(reorder_target(2, Some(0), false, 3), Some(0));
        assert_eq!(reorder_target(2, Some(0), true, 3), Some(1));
        // The Smart Filters row: on top.
        assert_eq!(reorder_target(0, None, false, 3), Some(2));
        // Next to where it already is, or on itself: nothing to do.
        assert_eq!(reorder_target(1, Some(2), false, 3), None);
        assert_eq!(reorder_target(1, Some(0), true, 3), None);
        assert_eq!(reorder_target(1, Some(1), true, 3), None);
        assert_eq!(reorder_target(2, None, true, 3), None);
        assert_eq!(reorder_target(0, None, true, 0), None);
    }

    #[test]
    fn shape_layers_and_smart_objects_carry_a_thumbnail_badge() {
        let [pixel, shape, smart] = layers();
        assert_eq!(thumb_badge_kind(&pixel), None);
        assert_eq!(thumb_badge_kind(&shape), Some(ThumbBadge::Shape), "#4584");
        assert_eq!(thumb_badge_kind(&smart), Some(ThumbBadge::Smart));
    }

    /// The rects `thumb_badge` fills for `l` over `thumb`, with their colours.
    fn filled(l: &Layer, thumb: Rect) -> Vec<(Rect, Color32)> {
        let l = l.clone();
        let mut h = egui_kittest::Harness::new_ui(move |ui| thumb_badge(ui, &l, thumb));
        h.run();
        let badge = thumb_badge_rect(thumb).expand(0.5);
        let rects = h.output().shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if badge.contains_rect(r.rect) && r.fill != Color32::TRANSPARENT => Some((r.rect, r.fill)),
            _ => None,
        });
        rects.collect()
    }

    /// #4584: a shape layer's thumbnail shows a path with four anchor points in its corner.
    #[test]
    fn shape_badge_paints_a_path_with_an_anchor_on_each_corner() {
        let [pixel, shape, smart] = layers();
        let thumb = Rect::from_min_size(pos2(20.0, 20.0), vec2(34.0, 34.0));
        let t = Tokens::get(&egui::Context::default());
        let frame = thumb_badge_rect(thumb);
        assert!(frame.right() > thumb.right() && frame.bottom() > thumb.bottom(), "over the bottom-right corner");
        assert!(filled(&pixel, thumb).is_empty(), "a pixel layer has no badge");

        let drawn = filled(&shape, thumb);
        assert!(drawn.contains(&(frame, t.field)), "the badge's frame");
        let anchors: Vec<Rect> = drawn.iter().filter(|(_, c)| *c == t.icon).map(|(r, _)| *r).collect();
        let path = frame.shrink(SHAPE_PATH_INSET);
        let corners = [path.left_top(), path.right_top(), path.right_bottom(), path.left_bottom()];
        assert_eq!(anchors.iter().map(Rect::center).collect::<Vec<_>>(), corners);
        for a in &anchors {
            assert_eq!(a.size(), vec2(SHAPE_ANCHOR, SHAPE_ANCHOR));
            assert!(frame.shrink(1.0).contains_rect(*a), "inside the frame's border");
        }

        // The smart-object badge keeps its frame and draws no anchors.
        let drawn = filled(&smart, thumb);
        assert!(drawn.contains(&(frame, t.field)));
        assert!(drawn.iter().all(|(_, c)| *c != t.icon));
    }

    /// #5689: an Image › Adjustments recorded on a smart object (Brightness/Contrast…) opens its
    /// own editor from the Smart Filters row, with the recorded settings, and OK updates that
    /// filter instead of stacking another.
    #[test]
    fn an_adjustment_smart_filter_opens_its_editor_and_updates_it() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("image.adjustments.brightnessContrast", json!({"brightness": 40, "contrast": -10})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let sm = match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
            LayerContent::Smart(sm) => sm.clone(),
            other => panic!("expected a smart object, got {other:?}"),
        };
        assert_eq!(sm.smart_filters.len(), 1);
        open_editor(&mut app, &egui::Context::default(), id, &sm, 0);
        let d = app.ui.dialogs.last().expect("the adjustment editor opened");
        assert_eq!(d.fields.get("__adjust").and_then(Value::as_str), Some("brightnessContrast"));
        assert_eq!(d.fields.get("brightness").and_then(Value::as_f64), Some(40.0), "the recorded settings");
        assert_eq!(d.fields.get("contrast").and_then(Value::as_f64), Some(-10.0));
        let dialog = d.id;
        // OK updates the recorded filter (one history step), it does not add a second filter.
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content else {
            panic!("the layer stays a smart object");
        };
        assert_eq!(sm.smart_filters.len(), 1, "no filter was stacked");
        assert_eq!(sm.smart_filters[0].params.get("brightness").and_then(Value::as_f64), Some(40.0));
        assert_eq!(sm.smart_filters[0].params.get("contrast").and_then(Value::as_f64), Some(-10.0));
    }

    /// #5689 follow-up: an adjustment re-opens with the settings it renders with, without a
    /// Preview checkbox, and OK sets them whole. A monochrome Channel Mixer recorded without its
    /// gray row renders Photoshop's grey mix: copying only the keys the defaults have showed the
    /// identity rows instead, and OK on the untouched dialog swapped the mix for the red channel.
    /// Turning Monochrome off drops the gray row, and OK must drop it from the filter too, not
    /// merge the rest over it (the old row kept overriding the red channel).
    #[test]
    fn an_adjustment_smart_filter_reopens_with_the_settings_it_renders() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#c06030"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let flat = |app: &PhotocraftApp| photocraft_compose::flatten(&app.session.active().unwrap().doc).px;
        let unfiltered = flat(&app);
        app.run("image.adjustments.channelMixer", json!({"monochrome": true})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            LayerContent::Smart(sm) => sm.clone(),
            _ => panic!("not a smart object"),
        };
        let (before, steps) = (flat(&app), app.session.active().unwrap().history.past_len());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let ctx = h.ctx.clone();

        let sm = smart(h.state());
        open_editor(h.state_mut(), &ctx, layer.0, &sm, 0);
        h.run();
        assert!(h.query_by_label("Preview").is_none(), "no Preview: it would stack a second filter");
        let d = h.state().ui.dialogs.last().expect("the adjustment dialog opened");
        assert_eq!(d.fields["__adjust"], "channelMixer");
        assert_eq!(d.fields["__smartFilter"], json!({"layer": layer.0, "index": 0, "replace": true}));
        assert_eq!(d.fields["monochrome"], true);
        assert_eq!(d.fields["gray"], json!([40.0, 40.0, 20.0, 0.0]), "the grey mix the filter renders");
        let id = d.id;
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        assert_eq!(smart(h.state()).smart_filters.len(), 1, "edited in place, not added");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
        assert_eq!(flat(h.state()), before, "OK on the untouched dialog keeps the image");

        // Monochrome off, as the editor leaves it: no gray row, the identity mix.
        let sm = smart(h.state());
        open_editor(h.state_mut(), &ctx, layer.0, &sm, 0);
        let d = h.state_mut().ui.dialogs.last_mut().expect("the adjustment dialog opened");
        d.fields.insert("monochrome".into(), json!(false));
        d.fields.remove("gray");
        for (row, mix) in [("red", [100, 0, 0, 0]), ("green", [0, 100, 0, 0]), ("blue", [0, 0, 100, 0])] {
            d.fields.insert(row.into(), json!(mix));
        }
        let id = d.id;
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        assert!(smart(h.state()).smart_filters[0].params.get("gray").is_none(), "the gray row went with Monochrome");
        let off = flat(h.state());
        assert!(off.iter().zip(&unfiltered).all(|(a, b)| (0..4).all(|c| (a[c] - b[c]).abs() < 1e-6)), "the identity mix");

        // A dialog opened from the menu keeps its Preview.
        crate::adjust_dialog::open(h.state_mut(), "image.adjustments.channelMixer").unwrap();
        h.run();
        assert!(h.query_by_label("Preview").is_some());
    }
}
