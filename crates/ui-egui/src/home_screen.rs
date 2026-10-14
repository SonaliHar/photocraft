//! The Home screen: what PhotoCraft shows instead of a canvas when no document is open.
//!
//! Two layouts, one set of controls. A canvas narrower than [`DASHBOARD_MIN_WIDTH`] gets the
//! centred 460 wide card, unchanged: the wordmark and tagline, the New… / Open… buttons, the
//! file-drop hint (on Wayland, where winit 0.30 has no file drops, it points at File › Open and
//! paste instead), the Recent files list and the Discord and link row, on a card that grows with
//! the number of recent files. A wider one gets the three-column dashboard — the actions and
//! the drop hint on the left, the Templates gallery (every category and preset of the New
//! Document dialog, reachable without opening the dialog first) in the middle, the recent files,
//! the Discord button and the links on the right — and its body scrolls, so a short window
//! never cuts a row off.
//!
//! The screen draws no canvas, so it lives here rather than in [`canvas`].

use egui::{Rect, Sense};

use crate::PhotocraftApp;
use crate::canvas::paint_dots;

/// Below this width of canvas area the Home screen stays a centred card; at or above it, the
/// three-column dashboard.
///
/// The wide side has to be wide enough for what it draws: [`RAIL_WIDTH`] + [`GRID_WIDTH`] +
/// [`RIGHT_WIDTH`] plus the two [`COLUMN_GAP`]s = 1024, plus the 10 points egui's solid scroll bar
/// reserves when the body is taller than the window. 1040 is the first width that fits all of it,
/// and it keeps the 760 point minimum window (`apps/photocraft/src/main.rs:76`) firmly on the card.
pub const DASHBOARD_MIN_WIDTH: f32 = 1040.0;

/// Which Home screen a canvas area gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Today's centred card, for a canvas too narrow for the three columns.
    Compact,
    /// The three-column dashboard: actions, templates, recents and links.
    Dashboard,
}

/// The dashboard's left rail: New, Open and the file-drop hint.
pub(crate) const RAIL_WIDTH: f32 = 200.0;
/// The dashboard's right column: the recent files and the settings.
pub(crate) const RIGHT_WIDTH: f32 = 260.0;
/// The space between two dashboard columns.
const COLUMN_GAP: f32 = 28.0;
/// The gap [`templates`] leaves between two cards.
const TEMPLATE_GAP: f32 = 8.0;
/// The width the templates grid needs: `TEMPLATE_COLUMNS` cards of `TEMPLATE_CARD` wide and the
/// gaps between them. The centre column is never narrower — the cards are allocated at a fixed
/// size, so a narrower column would draw them over each other.
pub(crate) const GRID_WIDTH: f32 = TEMPLATE_COLUMNS as f32 * TEMPLATE_CARD.x + (TEMPLATE_COLUMNS as f32 - 1.0) * TEMPLATE_GAP;

// The threshold is the width all three columns and the scroll bar's reserve add up to, so it is
// checked while compiling: lowering a column or the threshold would draw the last templates card
// over the right one.
const _: () = assert!(DASHBOARD_MIN_WIDTH >= RAIL_WIDTH + GRID_WIDTH + RIGHT_WIDTH + COLUMN_GAP * 2.0 + 10.0);

/// Which Home screen a canvas area `available_width` wide gets.
pub fn layout_for(available_width: f32) -> Layout {
    if available_width >= DASHBOARD_MIN_WIDTH { Layout::Dashboard } else { Layout::Compact }
}

/// The Home screen: today's card when the canvas is narrow, the dashboard when it is not.
pub fn screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    match layout_for(ui.available_rect_before_wrap().width()) {
        Layout::Compact => start_screen(app, ui),
        Layout::Dashboard => full_dashboard(app, ui),
    }
}

pub fn start_screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let area = ui.available_rect_before_wrap();
    paint_dots(ui, area, app.session.prefs().interface.show_canvas_dots);
    // File › Open Recent, newest first (on the web there are no paths to reopen).
    let recent: Vec<String> = if cfg!(target_arch = "wasm32") { Vec::new() } else { app.ui.recent_files.iter().take(HOME_RECENT).cloned().collect() };
    let recent_h = if recent.is_empty() { 0.0 } else { 34.0 + recent.len() as f32 * HOME_RECENT_ROW };
    let card = Rect::from_center_size(area.center(), egui::vec2(460.0, 330.0 + recent_h));
    let new_label = crate::shortcuts::command_label(app, "New document…", "file.new");
    let open_label = crate::shortcuts::command_label(app, "Open…", "file.open");
    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        ui.vertical_centered(|ui| {
            header(ui, &t, card.width());
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.add_space(((card.width() - 2.0 * 190.0 - 12.0) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = 12.0;
                if crate::widgets::primary_button(ui, &new_label, 190.0).clicked() {
                    let fields = app.new_document_fields();
                    app.ui.open_dialog(crate::state::DialogKind::NewDocument, fields);
                }
                if crate::widgets::secondary_button(ui, &open_label, 190.0).clicked() {
                    let _ = app.open_dialog_file();
                }
            });
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                let msg = start_screen_drop_hint(app);
                let g = ui.painter().layout_no_wrap(msg.to_string(), egui::FontId::proportional(12.5), t.text_faint);
                ui.add_space(((card.width() - g.size().x - 24.0) / 2.0).max(0.0));
                let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
                crate::icons::paint(ui, r, "image", 15.0, t.text_faint);
                ui.label(egui::RichText::new(msg).color(t.text_faint));
            });
            if !recent.is_empty() {
                ui.add_space(22.0);
                home_recent(app, ui, &recent);
            }
            // The community links and the Discord button are not on the screen you land on: they
            // live in the app's own menus. The card shows the settings instead, as the dashboard
            // does, so the two layouts offer the same things.
            ui.add_space(26.0);
            settings(app, ui, &t);
        });
    });
}

/// The wordmark with its "open source" tag and the line under it: the header both Home screens
/// show, centred in the `width` they are given.
fn header(ui: &mut egui::Ui, t: &crate::theme::Tokens, width: f32) {
    ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.horizontal(|ui| {
            let title = ui.painter().layout_no_wrap(tl!("PhotoCraft").into(), crate::theme::semibold(38.0), t.text);
            let by = ui.painter().layout_no_wrap(tl!("open source").into(), egui::FontId::proportional(13.0), t.text_faint);
            // A `horizontal` takes the whole row whatever the layout above it is, so the
            // wordmark is centred in it by hand.
            ui.add_space(((width - title.size().x - by.size().x - 10.0) / 2.0).max(0.0));
            let (r, _) = ui.allocate_exact_size(title.size(), Sense::hover());
            ui.painter().galley(r.min, title, t.text);
            let (r2, _) = ui.allocate_exact_size(egui::vec2(by.size().x + 10.0, r.height()), Sense::hover());
            ui.painter().galley(egui::pos2(r2.left() + 10.0, r.bottom() - by.size().y - 8.0), by, t.text_faint);
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!("Create a new document or open an existing file.")).color(t.text_dim).size(14.0));
    });
}

/// The three-column dashboard. Every control the card above shows is here, in three columns
/// instead of one stack: the layout changed, the feature set did not.
///
/// The header stays put at the top and the columns scroll under it, so a window shorter than
/// the templates grid scrolls rather than cutting rows off.
fn full_dashboard(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let area = ui.available_rect_before_wrap();
    paint_dots(ui, area, app.session.prefs().interface.show_canvas_dots);
    let width = ui.available_width();
    header(ui, &t, width);
    ui.add_space(16.0);
    // File › Open Recent, newest first (on the web there are no paths to reopen).
    let recent: Vec<String> = if cfg!(target_arch = "wasm32") { Vec::new() } else { app.ui.recent_files.iter().take(HOME_RECENT).cloned().collect() };
    egui::ScrollArea::vertical().id_salt("home-dashboard").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            // The columns are laid edge to edge, so the item spacing would come out of the
            // templates grid; it is zeroed and the gaps below are explicit instead.
            ui.spacing_mut().item_spacing.x = 0.0;
            // The three columns are centred as a group. A wide window has slack to spend, and
            // pushing the last column against the window edge instead left a void between the
            // templates and the settings that read as a mistake rather than as space.
            let total = RAIL_WIDTH + GRID_WIDTH + RIGHT_WIDTH + COLUMN_GAP * 2.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.set_width(RAIL_WIDTH);
                actions(app, ui, &t);
            });
            ui.add_space(COLUMN_GAP);
            ui.vertical(|ui| {
                // The grid is the one column that cannot be squeezed: its cards are allocated
                // at a fixed size, so the column is exactly as wide as they need.
                ui.set_width(GRID_WIDTH);
                templates(app, ui);
            });
            ui.add_space(COLUMN_GAP);
            ui.vertical(|ui| {
                ui.set_width(RIGHT_WIDTH);
                if !recent.is_empty() {
                    home_recent(app, ui, &recent);
                    ui.add_space(20.0);
                }
                settings(app, ui, &t);
            });
        });
    });
}

/// The dashboard's right column under the recent files: the settings a person wants before they
/// start working rather than after — the theme they work in, the language the UI is in, and the
/// way Preferences opens.
///
/// This replaces the Discord button and the link row the card shows. Those point at the project's
/// community, which belongs in the app's own menus rather than on the screen you land on.
fn settings(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &crate::theme::Tokens) {
    let width = ui.available_width();
    ui.label(egui::RichText::new(tl!("Settings")).font(crate::theme::semibold(12.5)).color(t.text_dim));
    ui.add_space(6.0);

    // The theme: the current one, and the next one on a click, the way the title bar's theme
    // button already cycles them.
    let current = app.ui.theme;
    ui.horizontal(|ui| {
        let label = egui::RichText::new(current.label()).size(12.0).color(t.text);
        if ui.add(egui::Button::new(label).min_size(egui::vec2(width - 86.0, 30.0))).clicked() {
            let next = current.next();
            if let Err(e) = app.run("prefs.set", serde_json::json!({ "path": "interface.theme", "value": next.id() })) {
                let template = "Could not change the theme: {error}";
                app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
                app.ui.status_error = true;
            }
        }
        // A label for the gesture: the button changes the theme, it does not open a picker.
        ui.label(egui::RichText::new(tl!("Change")).size(10.5).color(t.text_faint));
    });

    ui.add_space(8.0);
    if crate::widgets::secondary_button(ui, &tl!("Preferences…"), width).clicked() {
        crate::prefs_ui::open_preferences(app, "interface");
    }
    ui.add_space(6.0);
    if crate::widgets::secondary_button(ui, &tl!("Keyboard Shortcuts…"), width).clicked() {
        crate::prefs_ui::open_shortcuts(app, 0);
    }
}

/// The left rail: New, Open and the file-drop hint, the card's actions in a column.
fn actions(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &crate::theme::Tokens) {
    let new_label = crate::shortcuts::command_label(app, "New document…", "file.new");
    let open_label = crate::shortcuts::command_label(app, "Open…", "file.open");
    let width = ui.available_width();
    if crate::widgets::primary_button(ui, &new_label, width).clicked() {
        let fields = app.new_document_fields();
        app.ui.open_dialog(crate::state::DialogKind::NewDocument, fields);
    }
    if crate::widgets::secondary_button(ui, &open_label, width).clicked() {
        let _ = app.open_dialog_file();
    }
    ui.add_space(20.0);
    // The same hint the card shows under its buttons, wrapped to the rail.
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
        crate::icons::paint(ui, r, "image", 15.0, t.text_faint);
        ui.add(egui::Label::new(egui::RichText::new(start_screen_drop_hint(app)).color(t.text_faint).size(12.5)).wrap());
    });
}

/// Wayland has no native file drops (winit 0.30, #386), so the hint offers File › Open and paste.
pub(crate) fn start_screen_drop_hint(app: &PhotocraftApp) -> std::borrow::Cow<'static, str> {
    if app.services.is_wayland {
        crate::i18n::fmt(tl!("Use File › Open, or paste a copied image with {paste}."), &[("paste", &crate::notices::paste_hint(app))]).into()
    } else {
        tl!("Drop an image or PSD anywhere to open it.").into()
    }
}

/// Recent files listed on the Home screen.
pub const HOME_RECENT: usize = 6;
/// Height of one Home-screen recent-file row.
pub const HOME_RECENT_ROW: f32 = 30.0;
/// How wide a recent-file row is on the card; the dashboard's right column is narrower and the
/// rows take the width they are given, so they do not reach over the templates.
const HOME_RECENT_WIDTH: f32 = 380.0;

/// The Home screen's "Recent" list: file name, its folder on the right, click to open. The
/// hovered row shows a × that takes it off the list, and right-click offers Remove from Recent
/// and Clear Recent Files (#2691).
pub(crate) fn home_recent(app: &mut PhotocraftApp, ui: &mut egui::Ui, recent: &[String]) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let width = ui.available_width().min(HOME_RECENT_WIDTH);
    ui.horizontal(|ui| {
        // Line the heading up with the file icons.
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0) + 8.0);
        ui.label(egui::RichText::new(tl!("Recent")).font(crate::theme::semibold(12.5)).color(t.text_dim));
    });
    ui.add_space(4.0);
    // What is left of the row beside the folder: the card has always given the name 170 of its
    // 380, a narrower column leaves the folder a little more.
    let name_max = 170.0f32.min(width * 0.55);
    let mut open = None;
    let mut remove = None;
    let mut clear = false;
    for path in recent {
        let name = crate::file_open::display_name(path);
        let folder = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let (row, resp) = ui.allocate_exact_size(egui::vec2(width, HOME_RECENT_ROW), Sense::click());
        // The × sits on top of the row, so the row counts as hovered while the pointer is on it.
        let hovered = ui.rect_contains_pointer(row);
        if hovered {
            ui.painter().rect_filled(row, t.radius_sm, t.hover);
        }
        let close = Rect::from_center_size(egui::pos2(row.right() - 16.0, row.center().y), egui::vec2(22.0, 22.0));
        let right = if hovered { close.left() - 4.0 } else { row.right() - 10.0 };
        let icon = Rect::from_center_size(egui::pos2(row.left() + 16.0, row.center().y), egui::vec2(16.0, 16.0));
        crate::icons::paint(ui, icon, "file", 14.0, t.text_dim);
        let x = row.left() + 32.0;
        let name_g = crate::tab_strip::elided(ui, &name, egui::FontId::proportional(12.5), t.text, name_max);
        let name_w = name_g.size().x;
        ui.painter().galley(egui::pos2(x, row.center().y - name_g.size().y / 2.0), name_g, t.text);
        let folder_max = (right - (x + name_w + 14.0)).max(0.0);
        if folder_max > 20.0 && !folder.is_empty() {
            let fg = crate::tab_strip::elided(ui, &folder, egui::FontId::proportional(11.5), t.text_faint, folder_max);
            ui.painter().galley(egui::pos2(right - fg.size().x, row.center().y - fg.size().y / 2.0), fg, t.text_faint);
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
        let resp = resp.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand);
        if hovered {
            let x = ui.interact(close, resp.id.with("remove"), Sense::click());
            let label = crate::i18n::fmt(tl!("Remove {name} from Recent"), &[("name", &name)]);
            x.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            let color = crate::icons::button_chrome(ui, close, false, x.hovered());
            crate::icons::paint(ui, close, "x", 12.0, color);
            if x.on_hover_text(tl!("Remove from Recent")).clicked() {
                remove = Some(path.clone());
            }
        }
        if resp.clicked() && remove.is_none() {
            open = Some(path.clone());
        }
        resp.context_menu(|ui| {
            if ui.button(tl!("Remove from Recent")).clicked() {
                remove = Some(path.clone());
                ui.close();
            }
            if ui.button(tl!("Clear Recent Files")).clicked() {
                clear = true;
                ui.close();
            }
        });
    }
    if clear {
        app.clear_recent();
    } else if let Some(path) = remove {
        app.remove_recent(&path);
    }
    if let Some(path) = open
        && let Err(e) = app.open_path(&path)
    {
        app.open_failed(&crate::file_open::display_name(&path), &e);
    }
}

/// A Templates card's size: the New Document dialog's preset card, so the same preset looks like
/// the same thing in both places.
pub const TEMPLATE_CARD: egui::Vec2 = egui::vec2(164.0, 112.0);
/// Templates cards per row.
pub const TEMPLATE_COLUMNS: usize = 3;

/// The category the panel opens on: Photo (1).
///
/// Not Recent (0). Recent is documents this machine created, so on a fresh install it is two
/// default sizes and the Home screen looks empty; Photo is the category most people open a blank
/// document from, and it lists six. Recent stays one click away on the tab row.
pub const DEFAULT_CATEGORY: usize = 1;

/// The category `index` names, or [`DEFAULT_CATEGORY`] when the index is out of range.
///
/// The index is serialised state — a state file can hold any number — so it is resolved before
/// use and never indexed with. Resolving it first also means the fallback is visible in the UI:
/// the tab stays underlined instead of every tab going flat.
fn resolved_category(category: usize) -> usize {
    if category < crate::new_doc_ui::CATEGORIES.len() { category } else { DEFAULT_CATEGORY }
}

/// The presets of `category`, or an empty slice when the index is out of range.
pub fn category_presets(category: usize) -> &'static [crate::new_doc_ui::Preset] {
    crate::new_doc_ui::CATEGORIES.get(category).map_or(&[], |c| c.1)
}

/// The Home screen's Templates panel: the New Document dialog's categories as a tab row and their
/// presets as cards; clicking a card opens File › New with that preset applied. The categories, the
/// cards and the Recent rule are the dialog's own — [`crate::new_doc_ui::recent_cards`] for Recent,
/// and the dialog's card drawing for each card — so the two screens cannot drift apart.
///
/// An empty category (or an empty Recent) draws the tab row and nothing else — no error, no
/// placeholder.
pub fn templates(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let category = resolved_category(app.ui.chrome.home_category);
    // The search box: across the tab row's own width, so it reads as the top of the panel.
    // It matches presets by name across every category, which is the reason it exists — the
    // category tabs are seven choices to page through, a name is one.
    ui.horizontal(|ui| {
        let width = ui.available_width();
        let mut text = app.ui.chrome.home_search.clone();
        let edit = egui::TextEdit::singleline(&mut text)
            .hint_text(tl!("Search templates…"))
            .desired_width(width - if app.ui.chrome.home_search.is_empty() { 0.0 } else { 22.0 })
            .font(egui::TextStyle::Body);
        let resp = ui.add(edit);
        if resp.changed() {
            app.ui.chrome.home_search = text;
        }
        // A clear button, so a long query does not have to be selected and deleted by hand. It sits
        // after the field, which is the leading edge in a right-to-left layout and the trailing one
        // in a left-to-right one, so it needs no direction of its own.
        if !app.ui.chrome.home_search.is_empty() {
            let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
            let clicked = ui.interact(r, resp.id.with("clear"), Sense::click());
            crate::icons::paint(ui, r, "x", 12.0, t.text_faint);
            if clicked.clicked() {
                app.ui.chrome.home_search.clear();
                resp.request_focus();
            }
        }
    });
    ui.add_space(6.0);
    // The category tabs, drawn exactly as the dialog draws them.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (i, c) in crate::new_doc_ui::CATEGORIES.iter().enumerate() {
            let on = category == i;
            let label = egui::Label::new(egui::RichText::new(tl!(c.0)).size(13.0).color(if on { t.text } else { t.text_dim })).sense(Sense::click());
            let r = ui.add(label);
            if on {
                let under = [r.rect.left_bottom() + egui::vec2(0.0, 3.0), r.rect.right_bottom() + egui::vec2(0.0, 3.0)];
                ui.painter().line_segment(under, egui::Stroke::new(2.0, t.text));
            }
            if r.clicked() {
                app.ui.chrome.home_category = i;
            }
        }
    });
    ui.add_space(6.0);
    let units = app.session.prefs().units_and_rulers.clone();
    let presets: Vec<crate::new_doc_ui::Card> = if category == 0 {
        // Recent is the dialog's rule: the documents created with the dialog, then the built-in
        // defaults they don't cover. An empty field map: no clipboard image was copied to make a
        // card out of here, and a card's click fills the dialog's fields when there are any.
        crate::new_doc_ui::recent_cards(&serde_json::Map::new(), &app.session.presets.recent_documents, &units)
    } else {
        category_presets(category).iter().copied().map(crate::new_doc_ui::Card::Builtin).collect()
    };
    // A search matches across every category, not just the tab: the tabs are seven pages to flip
    // through and a name is one. Case-insensitive, and the underlined tab stays the one the query
    // started from, so a hit from another category is a click away rather than a surprise.
    let query = app.ui.chrome.home_search.trim().to_lowercase();
    let presets: Vec<crate::new_doc_ui::Card> = if query.is_empty() {
        presets
    } else {
        crate::new_doc_ui::CATEGORIES
            .iter()
            .flat_map(|c| c.1)
            .filter(|p| p.0.to_lowercase().contains(&query))
            .copied()
            .map(crate::new_doc_ui::Card::Builtin)
            .collect()
    };
    if presets.is_empty() {
        // A category with no presets, or Recent on a machine that has created none. The tab row is
        // still there, so the next category is one click away.
        return;
    }
    for row in presets.chunks(TEMPLATE_COLUMNS) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for card in row {
                template_card(app, ui, card, &t, &units);
            }
        });
        ui.add_space(8.0);
    }
}

/// One Templates card: drawn by the New Document dialog's own card drawing — the page at the
/// preset's aspect ratio, the preset's name (wrapped and elided so a long translation stays
/// inside the card) and its size in the unit the preset is specified in. Clicking applies it to a
/// fresh set of dialog fields and opens File › New, as the dialog's own card click does.
fn template_card(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    card: &crate::new_doc_ui::Card,
    t: &crate::theme::Tokens,
    units: &photocraft_engine::prefs::UnitsAndRulers,
) {
    let (w, h, _) = card.size(units);
    let name = tl!(card.title()).to_owned();
    // `size_line` reads the dialog's fields only to resolve the Clipboard preset's unit, and the
    // Home screen offers no Clipboard preset, so an empty map gives the same answer.
    let size = card.size_line(units, &serde_json::Map::new());
    let (r, resp) = ui.allocate_exact_size(TEMPLATE_CARD, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    ui.painter().rect_filled(r, t.radius, if resp.hovered() { t.hover } else { t.field });
    crate::new_doc_ui::page_icon(ui, Rect::from_center_size(egui::pos2(r.center().x, r.top() + 34.0), egui::vec2(40.0, 40.0)), w, h, t);
    let title = crate::new_doc_ui::card_title(ui.painter(), &name, TEMPLATE_CARD.x - 12.0, t.text);
    let elided = title.elided;
    ui.painter().galley(egui::pos2(r.center().x, r.top() + 70.0 - title.size().y / 2.0), title, t.text);
    ui.painter().text(egui::pos2(r.center().x, r.top() + 97.0), egui::Align2::CENTER_CENTER, &size, egui::FontId::proportional(10.5), t.text_faint);
    // A title too long for the card shows in full on hover, as it does in the dialog.
    let resp = if elided { resp.on_hover_text(name) } else { resp };
    if resp.clicked() {
        let mut fields = app.new_document_fields();
        match card {
            crate::new_doc_ui::Card::Builtin(p) => crate::new_doc_ui::apply_preset(&mut fields, p, units),
            crate::new_doc_ui::Card::Recent(s) => crate::new_doc_ui::apply_settings(&mut fields, s),
        }
        app.ui.open_dialog(crate::state::DialogKind::NewDocument, fields);
    }
}
