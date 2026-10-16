//! Window › Plug-ins: the menu row an installed panel gets, the floating window it opens into, and
//! the events its widgets send back.
//!
//! The panel under test is the real artifact from `examples/plugins/panel-rs` — the one a person
//! installs — not a fixture written to match this code.
//!
//! Two things about writing this test are worth knowing. The installed plug-ins *and* the open panels
//! are process-wide, so every other test in this binary drives this panel too: each of their frames
//! ticks it, and their sessions announce their own documents to it. Assertions are therefore about
//! what the panel does in answer to *this* test's events, never about text another test's document
//! could have moved. And the panel is stopped and removed at the end, so nothing is left ticking
//! behind the other 2,900 tests.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

/// The example panel, as its `build.sh` leaves it (see `examples/plugins/panel-rs`).
const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/panel.wasm");
const PANEL: &str = "org.photocraft.example.panel";
const ITEM: &str = "plugin.panel.item.org.photocraft.example.panel";
/// The note the test types into the panel's field.
const NOTE: &str = "chapter 1";

/// The whole app, with the example panel installed and a 40 × 28 document open. One test drives all
/// of it: the registry and the open panels are process-wide, so two of these running at once would be
/// testing each other.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1100.0, 760.0)).with_max_steps(96).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("plugin.install", json!({"path": EXAMPLE})).unwrap();
    h.state_mut().run("file.new", json!({"width": 40, "height": 28})).unwrap();
    h.run_steps(3);
    h
}

/// What the engine holds for the panel, including the view it last sent.
fn inspected(h: &mut Harness<'static, PhotocraftApp>) -> Value {
    h.state_mut().run("plugin.panel.inspect", json!({"id": PANEL})).expect("the panel is open")
}

/// One widget of the panel's view, as the plug-in sent it: `column` children, in order.
fn widget(h: &mut Harness<'static, PhotocraftApp>, index: usize) -> Value {
    inspected(h)["view"]["children"][index].clone()
}

/// The text of one of the panel's labels.
fn text(h: &mut Harness<'static, PhotocraftApp>, index: usize) -> String {
    widget(h, index)["text"].as_str().unwrap_or_default().to_string()
}

/// Is anything drawn with this label?
fn shown(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

/// Whether the panel's row carries its checkmark — what the user sees in the menu, rather than the
/// state behind it.
fn row_checked(h: &Harness<'_, PhotocraftApp>) -> bool {
    crate::menus::menu_items(h.state()).into_iter().find(|i| i.id == ITEM).expect("the panel's row").checked == Some(true)
}

/// Focuses the panel's note field, reporting whether there was one to focus.
///
/// A panel's field has no accessible name of its own — the ABI gives a widget a placeholder, not a
/// label — so it cannot be found the way the buttons below are. It is found by role instead, in the
/// window the panel drew: the first (and here only) panel is placed at the canvas' top-left plus its
/// cascade offset, which is where the UI put it too.
fn focus_note_field(h: &mut Harness<'static, PhotocraftApp>) -> bool {
    let window = egui::Rect::from_min_size(h.state().last_canvas_rect.left_top() + vec2(40.0, 60.0), vec2(320.0, 240.0));
    h.query_all_by_role(egui::accesskit::Role::TextInput).find(|n| window.contains(n.rect().center())).map(|n| n.click()).is_some()
}

#[test]
fn a_panel_gets_a_window_row_that_opens_into_the_view_it_sent() {
    let mut h = harness();
    // A row under Window › Plug-ins, named by the title the module declares — not by its package
    // name, and not in Filter › Plug-ins, where filters are run on a layer.
    let items = crate::menus::menu_items(h.state());
    let row = items.iter().find(|i| i.id == ITEM).expect("the panel is listed in the menus");
    assert_eq!(row.label, "Document Info");
    assert_eq!(row.path, ["Window", "Plug-ins"]);
    assert!(row.enabled, "opening a panel needs no document and no selection");
    assert!(
        !items.iter().any(|i| i.id.contains(PANEL) && i.path == ["Filter", "Plug-ins"]),
        "a panel is not a filter, so it gets no row where filters are run"
    );
    // The path is the untranslated English menu path, like every other item's, so the row survives a
    // translated UI (#2540) — while the label stays the plug-in's own string, which is not ours to
    // translate.
    let ja = crate::i18n::Lang::from_code("ja").expect("registered language");
    assert_ne!(crate::i18n::tr(ja, "Window"), "Window", "the language must really translate the path");
    let lang = crate::i18n::language_scope(ja);
    let items = crate::menus::menu_items(h.state());
    let row = items.iter().find(|i| i.id == ITEM).expect("still listed in Japanese");
    assert_eq!(row.path, ["Window", "Plug-ins"]);
    assert_eq!(row.label, "Document Info");
    drop(lang);

    // Picking the row opens it: the plug-in's view is drawn, in the app's own theme.
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, ITEM, json!({})).unwrap();
    h.run_steps(3);
    assert!(row_checked(&h), "the row is checked while the panel is open");
    assert!(shown(&h, "Document Info"), "the window is titled with the plug-in's own title");
    assert_eq!(widget(&mut h, 1)["value"], "", "the field the plug-in sent, empty");
    assert_eq!(widget(&mut h, 2)["children"][0]["id"], "read", "and the buttons it sent");
    assert_eq!(text(&mut h, 4), "Reads 0, ticks 0", "nothing has happened yet");
    assert!(focus_note_field(&mut h), "its field is drawn");
    h.run_steps(1);

    // A press reaches the plug-in, which asks the host about the document and answers with a new view
    // — all inside the round trip, before the next frame is drawn.
    h.get_by_label("Read Document").click();
    h.run_steps(3);
    assert_eq!(text(&mut h, 4), "Reads 1, ticks 0", "the click reached the plug-in");
    // What the host answered is drawn in the panel's first label, but *which* document is in it is not
    // this test's to pin down: the panels are process-wide, so every other test's session announces its
    // own document while this one runs, and the last frame to arrive wins. The exact text —
    // `Document 40 x 28` — is asserted in the engine test, where one session owns the panel. What can
    // be read here is that the read came back answered rather than refused.
    let status = text(&mut h, 0);
    assert!(!status.contains("would not read"), "the host answered the read: {status}");

    // A panel can act, not only look: New Type Layer asks the host to run `type.create` (`doc.command`)
    // and the host answers with the command's own result. What is asserted here is the *document* — the
    // layer the host made — and not the panel's status line, which another test's document can move.
    h.get_by_label("New Type Layer").click();
    h.run_steps(3);
    let doc = h.state_mut().run("document.inspect", json!({})).unwrap();
    let layers = doc["layers"].as_array().cloned().unwrap_or_default();
    assert_eq!(layers.len(), 2, "the panel's command added a layer: {doc}");
    assert_eq!(layers[0]["kind"], "Type");
    assert_eq!(layers[0]["name"], "Balloon");
    let status = text(&mut h, 0);
    assert!(!status.contains("would not add"), "the host ran the command: {status}");

    // Watch Document arms a timer; its label changes because the plug-in said so, not because the host
    // did.
    h.get_by_label("Watch Document").click();
    h.run_steps(2);
    assert_eq!(inspected(&mut h)["timers"], 1, "the button armed a timer");
    assert!(shown(&h, "Stop Watching"), "and the plug-in redrew it");
    h.get_by_label("Stop Watching").click();
    h.run_steps(2);
    assert_eq!(inspected(&mut h)["timers"], 0, "the same button clears the timer it armed");

    // Typing stays on this side of the sandbox until the field is committed: Enter is the commit, and
    // only then does the plug-in hear about it — and only then does it store the note. (The buttons
    // above took the focus, so the field is clicked again first.)
    assert!(focus_note_field(&mut h));
    h.run_steps(1);
    h.event(egui::Event::Text(NOTE.into()));
    h.run_steps(1);
    assert_eq!(inspected(&mut h)["storage"].get("note"), None, "keystrokes do not cross the sandbox");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(inspected(&mut h)["storage"]["note"], NOTE, "the committed edit was stored");
    assert_eq!(widget(&mut h, 1)["value"], NOTE, "and the plug-in sent it back in the field");
    // The panel says so in its status line as well ("Note saved to plug-in storage"), but that line
    // belongs to the document and any other test's session can rewrite it between two frames, so the
    // acknowledgement is left to the engine test: the note above is what proves the commit happened.

    // Picking the row again closes the window; the same row is the way back.
    crate::menus::invoke(h.state_mut(), &ctx, ITEM, json!({})).unwrap();
    h.run_steps(2);
    assert!(!row_checked(&h));
    assert!(!shown(&h, "Read Document"), "the window went with it");
    crate::menus::invoke(h.state_mut(), &ctx, ITEM, json!({"show": true})).unwrap();
    h.run_steps(2);
    assert!(row_checked(&h), "`show` opens it whether or not it was open");
    // What a field holds is its value, not a label, so it is read from the view rather than found
    // drawn. Hiding a window keeps the panel's instance, so the note is still there.
    assert_eq!(widget(&mut h, 1)["value"], NOTE, "and it opens as it was left");

    // Restarting is allowed while it runs (it is what a stopped panel needs, and a panel that is
    // running is simply started over). It starts a *new* instance, so this is where plug-in storage
    // proves itself: the note comes back with `init`.
    crate::menus::invoke(h.state_mut(), &ctx, ITEM, json!({"restart": true})).unwrap();
    h.run_steps(2);
    assert_eq!(inspected(&mut h)["state"], "running");
    assert_eq!(text(&mut h, 4), "Reads 0, ticks 0", "a restarted panel starts at `init` again");
    assert_eq!(widget(&mut h, 1)["value"], NOTE, "with what the last instance stored");

    // Leave nothing behind: the panel is process-wide, and the tests after this one should not have to
    // share a running plug-in.
    h.state_mut().run("plugin.panel.close", json!({"id": PANEL, "stop": true})).unwrap();
    h.state_mut().run("plugin.remove", json!({"id": PANEL})).unwrap();
    assert!(photocraft_engine::plugin_panel_cmds::open_ids().is_empty(), "no panel is left open");
}
