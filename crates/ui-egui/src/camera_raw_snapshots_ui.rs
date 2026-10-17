//! Named Camera Raw recipe snapshots for the current open dialog.
use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use photocraft_algo::camera_raw::CameraRaw;

use crate::theme::Tokens;

const MAX_SNAPSHOTS: usize = 128;

#[derive(Default)]
pub(super) struct Store {
    items: Vec<(String, CameraRaw)>,
    name: String,
    selected: Option<usize>,
}

impl Store {
    pub(super) fn names(&self) -> Vec<&str> {
        self.items.iter().map(|(name, _)| name.as_str()).collect()
    }

    fn capture(&mut self, parameters: &CameraRaw) -> bool {
        if self.items.len() >= MAX_SNAPSHOTS {
            return false;
        }
        let name = self.name.trim().chars().take(128).collect::<String>();
        let name = if name.is_empty() { format!("{} {}", tl!("Snapshot"), self.items.len() + 1) } else { name };
        self.items.push((name, parameters.clone()));
        self.selected = self.items.len().checked_sub(1);
        self.name.clear();
        true
    }
}

pub(super) fn header(ui: &mut Ui, store: &mut Store, parameters: &CameraRaw) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), design.edit_bar_height), Sense::hover());
    let label = tl!("Snapshots");
    let heading = Rect::from_min_size(
        pos2(rect.left() + design.tab_heading_inset, rect.top() + design.tab_heading_offset_y),
        vec2((rect.width() - design.tab_heading_inset - 50.0).max(1.0), rect.height()),
    );
    super::info::heading(ui, heading, label, design.tab_heading_font_size);
    let button = Rect::from_center_size(pos2(rect.right() - design.section_right_padding - 10.0, rect.center().y), vec2(22.0, 22.0));
    let response = ui.interact(button, ui.id().with("create-camera-raw-snapshot"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled() && store.items.len() < MAX_SNAPSHOTS, tl!("Create Snapshot")));
    let response = response.on_hover_text(tl!("Create Snapshot"));
    egui::Popup::menu(&response).show(|ui| {
        ui.set_width(220.0);
        ui.label(tl!("Create Snapshot"));
        ui.add(egui::TextEdit::singleline(&mut store.name).char_limit(128));
        if ui.add_enabled(store.items.len() < MAX_SNAPSHOTS, egui::Button::new(tl!("Create"))).clicked() && store.capture(parameters) {
            ui.close();
        }
    });
    // The observed small sheet glyph does not require a bundled bitmap asset.
    let glyph = button.center() - vec2(5.0, 5.0);
    ui.painter().add(egui::Shape::closed_line(
        vec![glyph, glyph + vec2(9.0, 0.0), glyph + vec2(9.0, 10.0), glyph + vec2(5.0, 10.0), glyph + vec2(0.0, 5.0)],
        Stroke::new(1.2, t.icon),
    ));
    ui.painter().line_segment([glyph + vec2(0.0, 5.0), glyph + vec2(5.0, 5.0)], Stroke::new(1.2, t.icon));
    ui.painter().line_segment([glyph + vec2(5.0, 5.0), glyph + vec2(5.0, 10.0)], Stroke::new(1.2, t.icon));
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
}

pub(super) fn list(ui: &mut Ui, store: &mut Store) -> Option<CameraRaw> {
    let mut restore = None;
    let mut remove = None;
    for (index, (name, parameters)) in store.items.iter().enumerate() {
        let response = ui.selectable_label(store.selected == Some(index), name);
        if response.clicked() {
            store.selected = Some(index);
            restore = Some(parameters.clone());
        }
        response.context_menu(|ui| {
            if ui.button(tl!("Delete")).clicked() {
                remove = Some(index);
                ui.close();
            }
        });
    }
    if let Some(index) = remove {
        store.items.remove(index);
        store.selected =
            store.selected.and_then(|selected| if selected == index { None } else { Some(selected.saturating_sub(usize::from(selected > index))) });
    }
    restore
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_a_named_snapshot_restores_the_captured_recipe() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut store = Store::default();
        let mut recipe = CameraRaw { exposure: 1.0, ..Default::default() };
        store.name = "First".into();
        assert!(store.capture(&recipe));
        recipe.exposure = 2.0;
        store.name = "Second".into();
        assert!(store.capture(&recipe));
        let mut harness = Harness::new_ui_state(
            |ui, state: &mut (Store, CameraRaw)| {
                if let Some(parameters) = list(ui, &mut state.0) {
                    state.1 = parameters;
                }
            },
            (store, recipe),
        );
        harness.run();
        harness.get_by_label("First").click();
        harness.run();
        assert_eq!(harness.state().1.exposure, 1.0);
        harness.get_by_label("Second").click();
        harness.run();
        assert_eq!(harness.state().1.exposure, 2.0);
    }

    #[test]
    fn snapshots_keep_independent_recipe_values_and_bound_names_and_count() {
        let mut store = Store::default();
        let mut recipe = CameraRaw { exposure: 1.0, ..Default::default() };
        store.name = "  First  ".into();
        assert!(store.capture(&recipe));
        recipe.exposure = 2.0;
        assert_eq!(store.items.first().unwrap().0, "First");
        assert_eq!(store.items.first().unwrap().1.exposure, 1.0);
        store.name = "影".repeat(200);
        assert!(store.capture(&recipe));
        assert_eq!(store.items.get(1).unwrap().0.chars().count(), 128);
        while store.items.len() < MAX_SNAPSHOTS {
            assert!(store.capture(&recipe));
        }
        assert!(!store.capture(&recipe));
        assert_eq!(store.items.len(), MAX_SNAPSHOTS);
    }
}
