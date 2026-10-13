//! The Color panel tab draws Photoshop's spectrum field in every theme (#4367): the light themes
//! used to show HSB sliders only. The sliders are the panel menu's HSB Sliders mode (#294).

use egui::pos2;

use crate::PhotocraftApp;
use crate::dock::Group;
use crate::theme::{ThemeKind, Tokens};

/// The Color tab of `kind`, alone in a 300 pt wide panel, starting from a red foreground.
fn color_tab(kind: ThemeKind) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    let tab = Group::Color.tabs(Tokens::for_kind(kind).pro).iter().position(|name| *name == "Color").unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(300.0, 400.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::dock_body(app, ui, Group::Color, tab);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(3);
    h
}

#[test]
fn every_theme_shows_the_spectrum_field() {
    for kind in ThemeKind::ALL {
        let mut h = color_tab(kind);
        let rects = crate::color_panel_ui::last_rects(&h.ctx);
        assert_eq!(rects.mode, Some(crate::color_panel_ui::ColorPanelMode::HueCube), "{kind:?}: the default mode");
        let field = rects.field.unwrap_or_else(|| panic!("{kind:?}: no spectrum field"));
        // The field's left edge, halfway down: no saturation, half brightness.
        let p = pos2(field.left() + 0.5, field.center().y);
        h.hover_at(p);
        h.run_steps(1);
        h.drag_at(p);
        h.run_steps(1);
        h.drop_at(p);
        h.run_steps(2);
        let [r, g, b, _] = h.state().session.tools.foreground;
        assert!((r - g).abs() < 0.02 && (g - b).abs() < 0.02 && (0.3..0.7).contains(&r), "{kind:?}: the field picks a mid grey, got {r} {g} {b}");
    }
}
