//! Home screen: the Templates panel, the category state behind it, and the routing between
//! today's card and the dashboard.

use egui::{Pos2, Rect, vec2};
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use photocraft_engine::document_preset_cmds::DocumentSettings;

use crate::PhotocraftApp;
use crate::chrome_ui::ChromeState;
use crate::home_screen;
use crate::new_doc_ui::CATEGORIES;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// A recently created document, at a size no built-in preset lists, so the dialog names its card
/// "Custom" rather than after a preset.
fn recent(width: u32, height: u32, resolution: f32) -> DocumentSettings {
    DocumentSettings {
        width,
        height,
        resolution,
        mode: "rgb".into(),
        depth: 8,
        background: "white".into(),
        background_color: None,
        unit: "px".into(),
        resolution_unit: "in".into(),
    }
}

/// The Templates panel over `app`, in a window wide enough for its three columns.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| home_screen::templates(app, ui));
        },
        app,
    );
    h.run_steps(2);
    h
}

/// Cards laid out for `category`: one per preset of the category whose name is on a card.
fn cards(h: &Harness<'static, PhotocraftApp>, category: usize) -> usize {
    CATEGORIES.get(category).into_iter().flat_map(|c| c.1).filter(|p| h.query_all_by_label(p.0).count() == 1).count()
}

/// The Home screen over `app` in a `size` window, recording the area it was given.
fn screen_harness(app: PhotocraftApp, size: egui::Vec2, theme: crate::theme::ThemeKind) -> Harness<'static, (PhotocraftApp, Option<Rect>, bool)> {
    // The harness draws a frame before the theme is set, and the theme is what binds the
    // semibold font family the wordmark uses, so the screen waits for it.
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, (app, area, ready): &mut (PhotocraftApp, Option<Rect>, bool)| {
            if !*ready {
                return;
            }
            *area = Some(ui.available_rect_before_wrap());
            home_screen::screen(app, ui);
        },
        (app, None, false),
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.state_mut().2 = true;
    h.run_steps(2);
    h
}

/// The area the Home screen was given to draw in.
fn area(h: &Harness<'_, (PhotocraftApp, Option<Rect>, bool)>) -> Rect {
    h.state().1.expect("the screen recorded the area it drew in")
}

/// [`screen_harness`] with the window widened until the Home screen is given exactly `width`
/// points: egui's test frame takes 8 off every side, so the inset is measured from the first
/// frame instead of guessed at.
fn harness_at_width(app: PhotocraftApp, width: f32, height: f32, theme: crate::theme::ThemeKind) -> Harness<'static, (PhotocraftApp, Option<Rect>, bool)> {
    let mut h = screen_harness(app, vec2(width, height), theme);
    let inset = width - area(&h).width();
    assert!(inset > 0.0, "the harness insets the area it hands the screen");
    h.set_size(vec2(width + inset, height));
    h.run_steps(2);
    assert!((area(&h).width() - width).abs() < 0.5, "the screen is {width} wide, got {:?}", area(&h));
    h
}

/// Every rect egui painted this frame, each clipped to the clip rect it was painted in: a
/// shape a scroll area or the panel's edge cuts only shows inside that clip rect.
fn painted_rects(h: &Harness<'_, (PhotocraftApp, Option<Rect>, bool)>) -> Vec<Rect> {
    h.output()
        .shapes
        .iter()
        .flat_map(|clipped| shape_rects(&clipped.shape).into_iter().map(move |r| r.intersect(clipped.clip_rect)))
        .filter(|r| !r.is_negative())
        .collect()
}

/// The rect a painted [`egui::epaint::Shape`] covers.
fn shape_rects(shape: &egui::epaint::Shape) -> Vec<Rect> {
    use egui::epaint::Shape;
    match shape {
        Shape::Noop | Shape::Callback(_) => vec![],
        Shape::Vec(items) => items.iter().flat_map(shape_rects).collect(),
        Shape::Rect(r) => vec![r.rect],
        Shape::Circle(c) => vec![Rect::from_center_size(c.center, vec2(2.0 * c.radius, 2.0 * c.radius))],
        Shape::Ellipse(e) => vec![Rect::from_center_size(e.center, 2.0 * e.radius)],
        Shape::LineSegment { points, stroke } => vec![points_rect(points.iter().copied()).expand(stroke.width / 2.0)],
        Shape::Path(p) => vec![points_rect(p.points.iter().copied())],
        Shape::Text(t) => vec![Rect::from_min_size(t.pos, t.galley.size())],
        Shape::QuadraticBezier(b) => vec![points_rect(b.points)],
        Shape::CubicBezier(b) => vec![points_rect(b.points)],
        Shape::Mesh(mesh) => vec![points_rect(mesh.vertices.iter().map(|v| v.pos))],
    }
}

/// The rect a run of points covers, or a negative rect for none.
fn points_rect(points: impl IntoIterator<Item = Pos2>) -> Rect {
    let mut min = Pos2::new(f32::INFINITY, f32::INFINITY);
    let mut max = Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in points {
        min = min.min(p);
        max = max.max(p);
    }
    Rect::from_min_max(min, max)
}

/// The label and rect of every widget egui put in the accessibility tree: every button,
/// label and row the Home screen allocated.
fn widget_rects(h: &Harness<'_, (PhotocraftApp, Option<Rect>, bool)>) -> Vec<(String, Rect)> {
    h.query_all(By::new()).filter_map(|n| n.accesskit_node().label().map(|l| (l, n.rect()))).collect()
}

/// The rect of the newest "Custom" card, the last one in the Recent grid.
fn last_card(h: &Harness<'_, (PhotocraftApp, Option<Rect>, bool)>) -> Rect {
    h.query_all_by_label("Custom").last().expect("a card per document recently created with the dialog").rect()
}

#[test]
fn a_search_matches_across_every_category() {
    // The tabs are seven pages to flip through; a name is one.
    let mut h = harness(app());
    h.state_mut().ui.chrome.home_search = "iphone".into();
    h.run_steps(2);
    assert!(h.query_by_label("iPhone 16").is_some(), "a Mobile preset is found from the Photo tab");
    assert!(h.query_by_label("iPhone 16 Pro Max").is_some());
    assert!(h.query_by_label("A4").is_none(), "and the category it is not in is not shown");

    h.state_mut().ui.chrome.home_search = "IPHONE".into();
    h.run_steps(2);
    assert!(h.query_by_label("iPhone 16").is_some(), "the search is case-insensitive");

    h.state_mut().ui.chrome.home_search = "zzz".into();
    h.run_steps(2);
    assert!(h.query_by_label("iPhone 16").is_none(), "a query nothing matches shows nothing");
}

#[test]
fn the_card_offers_the_settings_instead_of_the_community_links() {
    // Under 980 points the Home screen is the card, and it offers what the dashboard offers:
    // settings, not a Discord button and a row of links.
    let h = screen_harness(app(), vec2(760.0, 480.0), crate::theme::ThemeKind::Pro);
    assert!(h.query_by_label_contains("Preferences").is_some(), "the card offers Preferences");
    assert!(h.query_by_label_contains("Keyboard Shortcuts").is_some(), "and the shortcut list");
    for gone in ["Join us on Discord", "PhotoCraft website", "GitHub", "ArtCraft"] {
        assert!(h.query_all_by_label_contains(gone).next().is_none(), "{gone:?} is not on the card");
    }
}

#[test]
fn the_settings_column_offers_a_theme_preferences_and_shortcuts() {
    // The settings column replaces the Discord button and the link row. It lives on the screen, not
    // in the Templates panel, so this drives the whole Home screen.
    let h = screen_harness(app(), vec2(1440.0, 900.0), crate::theme::ThemeKind::Pro);
    assert!(h.query_by_label("Settings").is_some(), "the column is labelled");
    assert!(h.query_by_label("Preferences…").is_some(), "it opens Preferences");
    assert!(h.query_by_label("Keyboard Shortcuts…").is_some(), "and the shortcut list");
    // The theme button shows whatever theme the app is in, so the label is asserted against the
    // app's own state rather than a fixed string.
    let theme = h.state().0.ui.theme;
    assert!(h.query_all_by_label_contains(&theme.id()[..6]).next().is_some(), "and the current theme, {}", theme.label());
}

#[test]
fn chrome_category_defaults_and_round_trips() {
    assert_eq!(ChromeState::default().home_category, home_screen::DEFAULT_CATEGORY);
    let json = serde_json::to_string(&ChromeState { home_category: 4, ..Default::default() }).unwrap();
    assert_eq!(serde_json::from_str::<ChromeState>(&json).unwrap().home_category, 4);
}

#[test]
fn a_state_file_without_the_category_still_loads() {
    let loaded: ChromeState = serde_json::from_str(r#"{"statusInfo":"dimensions"}"#).unwrap();
    assert_eq!(loaded.home_category, home_screen::DEFAULT_CATEGORY);
}

#[test]
fn every_static_preset_renders_a_card() {
    let mut h = harness(app());
    let mut total = 0;
    for (i, (name, presets)) in CATEGORIES.iter().enumerate().skip(1) {
        h.state_mut().ui.chrome.home_category = i;
        h.run_steps(2);
        assert_eq!(cards(&h, i), presets.len(), "{name}: a card per preset");
        total += presets.len();
    }
    assert_eq!(total, 33, "every preset outside Recent is reachable from the Home screen");
}

#[test]
fn recent_category_shows_recent_documents() {
    // Recent is the dialog's own rule (`new_doc_ui::recent_cards`): the documents created with
    // the dialog first, then the built-in defaults they don't already cover. A card is named
    // after the preset it matches, so a document at a size no preset lists reads "Custom".
    let mut h = harness(app());
    h.state_mut().ui.chrome.home_category = 0;
    h.run_steps(2);
    assert_eq!(h.query_all_by_label("Custom").count(), 0, "no document created with the dialog yet");
    assert!(h.query_by_label("Default PhotoCraft Size").is_some(), "the dialog's Recent defaults still show");

    h.state_mut().session.presets.recent_documents = vec![recent(777, 333, 100.0), recent(640, 480, 96.0), recent(321, 654, 90.0)];
    h.run_steps(2);
    assert_eq!(h.query_all_by_label("Custom").count(), 3, "one card per recently created document");
    assert!(h.query_by_label("Default PhotoCraft Size").is_some(), "and the dialog's defaults beside them");
}

#[test]
fn the_panel_opens_on_a_populated_category() {
    // Recent on a fresh install is two default sizes, which reads as an empty Home screen.
    let h = harness(app());
    assert_eq!(h.state().ui.chrome.home_category, home_screen::DEFAULT_CATEGORY);
    assert_eq!(home_screen::category_presets(home_screen::DEFAULT_CATEGORY).len(), 6, "the default category lists presets");
    assert!(h.query_by_label("Landscape, 6 x 4").is_some(), "so the panel opens on a category with real presets in it");
}

#[test]
fn the_editing_chrome_is_hidden_while_home_is_up() {
    // No document is open, so the tool strip, the tool options and the panels dock would be a
    // column of dead controls: no tool acts without a document to act on. The menu bar and the
    // status bar stay, because they still open documents and report the zoom.
    let mut h = harness(app());
    assert_eq!(h.state().session.documents().len(), 0, "no document is open");
    assert!(h.state_mut().ui.chrome.shows_home(0, true), "Home is up");
    // The panel decides this from the same two facts, so a state where Home is down hides nothing.
    assert!(h.state_mut().ui.chrome.shows_home(0, false) == false, "the Home preference gates it");
}

#[test]
fn an_out_of_range_category_falls_back_to_the_default() {
    // 999 is what a hand-edited state file can leave behind.
    let mut app = app();
    app.ui.chrome.home_category = 999;
    let h = harness(app);
    assert!(home_screen::category_presets(999).is_empty(), "an index past the last category has no presets to look up");
    assert!(h.query_by_label("Photo").is_some(), "999 draws the default category's first preset");
    assert!(h.query_by_label("Film & Video").is_some(), "the category row still offers every category");
}

#[test]
fn clicking_a_chip_switches_category() {
    let mut h = harness(app());
    h.get_by_label("Web").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.chrome.home_category, 4, "Web is the fifth category");
    assert!(h.query_by_label("MacBook Pro 16\"").is_some(), "and its cards are the ones drawn");
}

#[test]
fn clicking_a_card_opens_new_document_with_that_preset() {
    let mut h = harness(app());
    h.state_mut().ui.chrome.home_category = 6;
    h.run_steps(2);
    h.get_by_label("UHD 4K").click();
    h.run_steps(2);
    let app = h.state();
    let d = app.ui.dialogs.iter().find(|d| d.kind == crate::state::DialogKind::NewDocument).expect("File › New is open");
    let fields = &d.fields;
    assert_eq!((fields["width"].as_u64(), fields["height"].as_u64()), (Some(3840), Some(2160)));
    assert_eq!((fields["resolution"].as_f64(), fields["__preset"].as_str()), (Some(72.0), Some("UHD 4K")));
}

/// The wide side owns the boundary: one point under it is still the card, one point on it is
/// the dashboard. `home_screen.rs` checks while compiling that the width it routes to is wide
/// enough for the three columns it draws.
#[test]
fn the_dashboard_boundary_is_owned_by_the_wide_side() {
    use home_screen::{DASHBOARD_MIN_WIDTH, Layout, layout_for};
    assert_eq!(layout_for(DASHBOARD_MIN_WIDTH - 0.1), Layout::Compact);
    assert_eq!(layout_for(DASHBOARD_MIN_WIDTH), Layout::Dashboard);
    assert_eq!(layout_for(1440.0), Layout::Dashboard);
}

/// The minimum window (`apps/photocraft/src/main.rs:76`) in the Pro theme, the default one
/// and a non-Pro one: today's card, with every control and every painted rect inside the
/// area the screen was given. The dashboard scrolls rather than clipping.
#[test]
fn a_window_at_the_minimum_size_does_not_clip() {
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::default(), crate::theme::ThemeKind::Studio] {
        let h = screen_harness(app(), vec2(760.0, 480.0), theme);
        let area = area(&h);
        assert!(h.query_by_label_contains("New document").is_some(), "{theme:?}: the card's buttons are drawn");
        for r in painted_rects(&h) {
            assert!(area.contains_rect(r), "{theme:?}: painted {r:?} outside {area:?}");
        }
        for (label, r) in widget_rects(&h) {
            assert!(area.contains_rect(r), "{theme:?}: {label:?} at {r:?} is cut off by {area:?}");
        }
    }
    // The dashboard body scrolls instead of clipping: at the routing threshold and 480 tall,
    // ten recently created documents make a grid four rows deep, the last row starts below
    // the fold, and a wheel event brings it on screen.
    let mut many = app();
    // Recent: the cards are named after the documents created with the dialog, and ten of them
    // make a grid four rows deep.
    many.ui.chrome.home_category = 0;
    many.session.presets.recent_documents = (0..10).map(|i| recent(101 + i * 37, 67 + i * 11, 90.0)).collect();
    let mut h = harness_at_width(many, home_screen::DASHBOARD_MIN_WIDTH, 480.0, crate::theme::ThemeKind::Pro);
    let area = area(&h);
    for r in painted_rects(&h) {
        assert!(area.contains_rect(r), "painted {r:?} outside {area:?}");
    }
    assert!(last_card(&h).bottom() > area.bottom(), "the last row is below the fold at 480 tall");
    h.hover_at(area.center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -300.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(last_card(&h).bottom() <= area.bottom(), "the body scrolls: the last row is on screen after a wheel");
}

/// A wide window draws the dashboard: every control the card has, in three columns, and the
/// templates grid still fits its column at the width that routes to it.
#[test]
fn a_wide_window_uses_the_dashboard() {
    let mut home = app();
    for name in ["one.png", "two.psd"] {
        home.push_recent(&format!("/tmp/photocraft-home/{name}"));
    }
    let h = screen_harness(home, vec2(1440.0, 900.0), crate::theme::ThemeKind::Pro);
    for label in [
        "New document…",
        "Open…",
        "Drop an image or PSD anywhere to open it.",
        "one.png",
        "Settings",
        "Preferences…",
        "Keyboard Shortcuts…",
        "Web",
        // The default category is Photo, so its presets are the ones on the dashboard by default.
        "Landscape, 6 x 4",
        "Portrait, 5 x 7",
    ] {
        assert!(h.query_all_by_label_contains(label).next().is_some(), "{label:?} is on the dashboard");
    }
    // The community links are not on the screen you land on: they belong in the app's own menus.
    for gone in ["Join us on Discord", "PhotoCraft website", "GitHub", "ArtCraft"] {
        assert!(h.query_all_by_label_contains(gone).next().is_none(), "{gone:?} is not on the Home screen");
    }
    // Three columns: the actions on the left, the templates in the middle, recents and settings
    // on the right.
    let rail = h.get_by_label_contains("New document").rect();
    let grid = h.get_by_label("Landscape, 6 x 4").rect();
    let recents = h.get_by_label("one.png").rect();
    let right = h.get_by_label("Preferences…").rect();
    assert!(rail.left() < grid.left(), "the rail is left of the templates: {rail:?} vs {grid:?}");
    assert!(grid.left() < recents.left(), "the right column is right of the templates: {recents:?} vs {grid:?}");
    assert!((recents.left() - right.left()).abs() < 1.0, "the recents and the settings share the right column: {recents:?} vs {right:?}");

    // At the routing threshold the grid still fits the column between the rail and the right
    // column: three cards and the two gaps between them.
    let mut photo = app();
    photo.ui.chrome.home_category = 1; // Photo: six presets, two rows of three
    let h = harness_at_width(photo, home_screen::DASHBOARD_MIN_WIDTH, 900.0, crate::theme::ThemeKind::Pro);
    let third = h.get_by_label("Landscape, 10 x 8").rect();
    let settings = h.get_by_label("Preferences…").rect();
    assert!(third.right() <= settings.left(), "the third card fits the column at the threshold: {third:?} reaches into {settings:?}");
}
