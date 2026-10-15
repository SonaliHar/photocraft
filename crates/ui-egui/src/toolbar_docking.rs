//! Movable tools palette. Docked tools are placed after panel columns, beside the canvas.
use crate::PhotocraftApp;
use egui::{Id, Pos2, Rect, Sense, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Left,
    Right,
    Floating,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarLayout {
    pub placement: Placement,
    pub position: [f32; 2],
    pub height: f32,
}
impl Default for ToolbarLayout {
    fn default() -> Self {
        Self { placement: Placement::Left, position: [120.0, 120.0], height: 720.0 }
    }
}
pub(crate) fn valid(layout: &ToolbarLayout) -> bool {
    layout.position.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0) && layout.height.is_finite() && (100.0..=4096.0).contains(&layout.height)
}
fn key(name: &str) -> Id {
    Id::new("modular-toolbar").with(name)
}

pub(crate) fn command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    if app.session.prefs().workspace_locked {
        return Err("workspace is locked".into());
    }
    let object = params.as_object().ok_or("toolbar layout must be an object")?;
    if object.keys().any(|key| !matches!(key.as_str(), "placement" | "position" | "height")) {
        return Err("unknown toolbar layout field".into());
    }
    let mut layout = app.ui.toolbar_layout.clone();
    if let Some(value) = params.get("placement") {
        layout.placement = serde_json::from_value(value.clone()).map_err(|_| "placement must be left, right or floating")?;
    }
    if let Some(value) = params.get("position") {
        layout.position = serde_json::from_value(value.clone()).map_err(|_| "position must contain two coordinates")?;
    }
    if let Some(value) = params.get("height") {
        layout.height = value.as_f64().ok_or("height must be a number")? as f32;
    }
    if !valid(&layout) {
        return Err("invalid toolbar coordinates or height".into());
    }
    app.ui.toolbar_layout = layout;
    app.ui.panels.toolbar = true;
    Ok(json!({"toolbarLayout":app.ui.toolbar_layout}))
}

pub(crate) fn begin_frame(ui: &egui::Ui) {
    let frame = ui.ctx().cumulative_pass_nr();
    let work = ui.available_rect_before_wrap();
    ui.ctx().data_mut(|data| {
        data.insert_temp(key("frame"), frame);
        data.insert_temp(key("work"), work);
        data.remove::<Rect>(key("docked"));
        data.remove::<u64>(key("rendered"));
    });
}
pub(crate) fn managed(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<u64>(key("frame"))) == Some(ctx.cumulative_pass_nr())
}
pub(crate) fn was_rendered(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<u64>(key("rendered"))) == Some(ctx.cumulative_pass_nr())
}
pub(crate) fn docked_rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|data| data.get_temp(key("docked")))
}

#[derive(Clone)]
struct Drag {
    original: ToolbarLayout,
    generation: u64,
    grab: Vec2,
    size: Vec2,
}

fn width(app: &PhotocraftApp, pro: bool) -> f32 {
    let (single, button) = if pro { (40.0, 30.0) } else { (50.0, 36.0) };
    single + if app.ui.panels.toolbar_double { button + 2.0 } else { 0.0 }
}

fn bounded(rect: Rect, work: Rect) -> Rect {
    let size = rect.size().max(vec2(1.0, 1.0)).min(work.size().max(vec2(1.0, 1.0)));
    Rect::from_min_size(rect.min.max(work.min).min(work.max - size), size)
}

fn target(pointer: Pos2, work: Rect, canvas: Rect) -> Placement {
    if pointer.x <= work.left() + 24.0 || (pointer.x - canvas.left()).abs() < 28.0 {
        Placement::Left
    } else if pointer.x >= work.right() - 24.0 || (pointer.x - canvas.right()).abs() < 28.0 {
        Placement::Right
    } else {
        Placement::Floating
    }
}

pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !valid(&app.ui.toolbar_layout) {
        app.ui.toolbar_layout = ToolbarLayout::default();
    }
    let ctx = ui.ctx().clone();
    let frame = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(key("rendered"), frame));
    let work = ctx.data(|data| data.get_temp::<Rect>(key("work"))).filter(|r| r.is_positive()).unwrap_or_else(|| ui.available_rect_before_wrap());
    let tokens = crate::theme::Tokens::get(&ctx);
    let layout = app.ui.toolbar_layout.clone();
    let w = width(app, tokens.pro);
    let margin = if tokens.pro { 5 } else { 7 };
    let palette_frame = egui::Frame::NONE.fill(tokens.chrome).inner_margin(egui::Margin::symmetric(margin, 8));
    match layout.placement {
        Placement::Left | Placement::Right => {
            let panel = if layout.placement == Placement::Left { egui::Panel::left("toolbar") } else { egui::Panel::right("toolbar") };
            let response = panel.resizable(false).exact_size(w).frame(palette_frame).show(ui, |ui| crate::panels::toolbar_contents(app, ui));
            ctx.data_mut(|data| data.insert_temp(key("docked"), response.response.rect));
        }
        Placement::Floating => {
            let rect = bounded(Rect::from_min_size(pos2(layout.position[0], layout.position[1]), vec2(w, layout.height)), work);
            if let Some(response) = egui::Window::new(tl!("Tools"))
                .id(key("window"))
                .order(egui::Order::Foreground)
                .title_bar(false)
                .movable(false)
                .resizable([false, !app.session.prefs().workspace_locked])
                .fixed_pos(rect.min)
                .min_width(w)
                .max_width(w)
                .default_width(w)
                .default_height(rect.height())
                .min_height(100.0_f32.min(work.height()))
                .max_height(work.height().max(1.0))
                .constrain_to(work)
                .frame(palette_frame)
                .show(&ctx, |ui| crate::panels::toolbar_contents(app, ui))
            {
                let height = response.response.rect.height().clamp(100.0, 4096.0);
                if !app.session.prefs().workspace_locked {
                    app.ui.toolbar_layout.height = height;
                }
            }
        }
    }
    let canvas = ui.available_rect_before_wrap();
    ctx.data_mut(|data| data.insert_temp(key("canvas"), canvas));
    finish_drag(app, &ctx, work, canvas, w);
    crate::dock::persist(app, &ctx);
}

/// Separate from the existing one/two-column chevron, so selecting a tool never starts a drag.
pub(crate) fn handle(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::drag());
    let response = response.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag Toolbar"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Drag Toolbar")));
    let tokens = crate::theme::Tokens::get(ui.ctx());
    ui.painter().line_segment([rect.center() - vec2(8.0, 0.0), rect.center() + vec2(8.0, 0.0)], egui::Stroke::new(2.0, tokens.text_faint));
    let locked = app.session.prefs().workspace_locked;
    response.context_menu(|ui| {
        for (placement, label) in
            [(Placement::Left, tl!("Dock Toolbar Left")), (Placement::Right, tl!("Dock Toolbar Right")), (Placement::Floating, tl!("Float Toolbar"))]
        {
            if ui.add_enabled(!locked, egui::Button::new(label)).clicked() {
                if let Err(error) = app.run("window.toolbar.layout", json!({"placement":placement})) {
                    app.ui.status = error;
                }
                ui.close();
            }
        }
    });
    if response.drag_started()
        && !locked
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        let source = ui.max_rect();
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                key("drag"),
                Drag { original: app.ui.toolbar_layout.clone(), generation: app.ui.docking_generation, grab: pointer - source.min, size: source.size() },
            )
        });
    }
}

fn finish_drag(app: &mut PhotocraftApp, ctx: &egui::Context, work: Rect, canvas: Rect, width: f32) {
    let Some(drag) = ctx.data(|data| data.get_temp::<Drag>(key("drag"))) else { return };
    let (escape, focused, down, released, pointer) =
        ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.focused, i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.interact_pos()));
    let changed = drag.generation != app.ui.docking_generation || app.session.prefs().workspace_locked;
    if changed || escape || !focused || (!down && !released) {
        ctx.data_mut(|data| data.remove::<Drag>(key("drag")));
        if !changed {
            app.ui.toolbar_layout = drag.original;
        }
        return;
    }
    let Some(pointer) = pointer.filter(|p| p.is_finite()) else { return };
    let placement = target(pointer, work, canvas);
    let rect = bounded(Rect::from_min_size(pointer - drag.grab, vec2(width, drag.size.y)), work);
    if released {
        ctx.data_mut(|data| data.remove::<Drag>(key("drag")));
        if !work.contains(pointer) {
            app.ui.toolbar_layout = drag.original;
            return;
        }
        if let Err(error) =
            app.run("window.toolbar.layout", json!({"placement":placement,"position":[rect.left(),rect.top()],"height":rect.height().clamp(100.0,4096.0)}))
        {
            app.ui.status = error;
        }
    } else {
        let preview = match placement {
            Placement::Left => Rect::from_min_size(canvas.min, vec2(width, canvas.height())),
            Placement::Right => Rect::from_min_size(pos2(canvas.right() - width, canvas.top()), vec2(width, canvas.height())),
            Placement::Floating => rect,
        };
        let tokens = crate::theme::Tokens::get(ctx);
        ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, key("preview"))).rect_stroke(
            preview,
            0.0,
            egui::Stroke::new(2.0, tokens.accent),
            egui::StrokeKind::Inside,
        );
        ctx.request_repaint();
    }
}

#[cfg(test)]
#[path = "toolbar_docking_tests.rs"]
mod tests;
