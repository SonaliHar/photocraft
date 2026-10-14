# Home screen dashboard — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the Home screen out of `canvas.rs` into its own module, and give it a responsive three-column dashboard with a Templates gallery covering all 33 presets that already exist in `new_doc_ui::CATEGORIES`.

**Architecture:** A new `crates/ui-egui/src/home_screen.rs` owns every Home-screen function. `canvas.rs` keeps only the call. Below 900 px of available width the screen renders today's 460 px card unchanged; at or above 900 px it renders a dashboard whose centre column is a category-tabbed preset grid. Clicking a preset opens the *existing* `DialogKind::NewDocument` dialog with `apply_preset` already applied — no new dialog, no new command, no engine change.

**Tech Stack:** Rust 1.98 (edition 2024), egui 0.36, `photocraft-engine` (L5), `photocraft-doc` (L1). Tests use the crate's existing `kittest` harness (`crates/ui-egui/src/direction_tests.rs:97`).

**Spec:** `docs/superpowers/specs/2026-10-12-home-screen-dashboard-design.md` — read it before starting; this plan argues from it and the spec travels with it.

## Global Constraints

- **Pure Rust.** No JavaScript, no TypeScript, no Tauri/Electron/webview. Never add one.
- **L6 crate only.** `crates/ui-egui` may depend on L0–L5. Nothing below L6 may use egui, eframe, winit or rfd. Verify with `cargo xtask layers`.
- **Never panic.** Non-test code contains no `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` or unchecked indexing. Input-derived indices use `.get()` with a fallback. `clippy.toml` enforces this crate-wide.
- **Input-derived numbers are hostile.** `home_category` arrives from a serialised state file and from the control channel; it is not trusted.
- **No hard-coded colours or radii.** Everything comes from `theme::Tokens`. This crate denies `clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable`.
- **No new i18n strings.** Category and preset labels reuse the dynamic `tl!(name)` lookup the New Document dialog already uses (`new_doc_ui.rs:452`). If a task needs a string not already in a catalog, the task is wrong — find the existing key instead.
- **No new command.** Clicking a template opens an existing dialog and dispatches the existing `file.new`. Do not register anything in the engine command registry.
- **Product name is always `PhotoCraft`** in user-facing text.
- **Clean-room.** Implement from public specs and observation. No copied Adobe code, shaders, profiles or assets. Do not bundle images; that would need licence files and an `ATTRIBUTION.md` row.
- **Never break wasm.** `crates/ui-egui` must still compile for `wasm32-unknown-unknown`.

## Review Focus

Five conditions a reasonable person would expect to work but no happy-path test covers. Each is pinned by a test in the task that owns the code.

1. **`home_category` out of range.** The control channel, MCP, or a hand-edited state file can set `home_category` to `999` or `-1`. Expected: the Recent category renders, no panic. (`layout` helpers in Task 2, test in Task 2)
2. **`home_category` deserialised as a negative.** `usize` serde refuses a negative, but a stale file written before the field existed omits it entirely. Expected: `#[serde(default)]` yields `0`, never a deserialisation error. (Task 2)
3. **Window exactly at the 900 px boundary.** A user dragging a window edge lands on 900.0 repeatedly. Expected: deterministic — one side owns the boundary, tested on both 899.9 and 900.0. (Task 3)
4. **Window at the 760 × 480 minimum** (`apps/photocraft/src/main.rs:76`). Expected: nothing clipped, nothing overlapping, in both themes. (Task 3)
5. **"Recent" category on a fresh install.** `session.presets.recent_documents` is empty, so Recent renders zero cards while the six static categories render 31. Expected: an empty Recent panel is drawn, not an error, and the category chips stay usable. (Task 2)

---

### Task 1: Move the Home screen into its own module

**Files:**
- Create: `crates/ui-egui/src/home_screen.rs`
- Modify: `crates/ui-egui/src/canvas.rs` (delete lines 2369–2513; change the call at line 1730)
- Modify: `crates/ui-egui/src/lib.rs:94` (add the module declaration)

**Interfaces:**
- Consumes: nothing from later tasks.
- Produces:
  ```rust
  // crates/ui-egui/src/home_screen.rs
  pub fn start_screen(app: &mut PhotocraftApp, ui: &mut egui::Ui)
  pub(crate) fn home_recent(app: &mut PhotocraftApp, ui: &mut egui::Ui, recent: &[String])
  pub(crate) fn start_screen_drop_hint(app: &PhotocraftApp) -> std::borrow::Cow<'static, str>
  pub const HOME_RECENT: usize = 6;
  pub const HOME_RECENT_ROW: f32 = 30.0;
  ```

- [ ] **Step 1: Record the baseline so the move can be proven behaviour-neutral**

Run: `cargo test -p photocraft-ui-egui --lib 2>&1 | Select-Object -Last 5`
Expected: the suite runs. Note the final `test result: ok. N passed` line — Task 4 must not lower N.

If the workspace has not finished its first build yet, wait for `cargo build -p photocraft` to succeed first; a red baseline is not a baseline.

- [ ] **Step 2: Create `home_screen.rs` with the moved code verbatim**

Create `crates/ui-egui/src/home_screen.rs` containing a module doc comment in the crate's house style, then the **five items copied unchanged** from `canvas.rs`: `start_screen` (2369–2424), `start_screen_drop_hint` (2427–2433), `HOME_RECENT` (2436), `HOME_RECENT_ROW` (2438), `home_recent` (2443–2513).

Adjust exactly three things and nothing else:
- the `use` list, which must now import what those functions used from `super::` — including `crate::{icons, theme, widgets, shortcuts, tab_strip, file_open, notices, links, i18n}` and `crate::PhotocraftApp`, and `egui::{Rect, Sense, pos2, vec2, RichText}`;
- `tl!` is imported by `use crate::tl;` if the file does not already have it (see `lib.rs:11-15` for the macro's definition);
- visibility as listed in this task's Interfaces block.

Do not rename anything, reformat anything, or "fix" anything. The point of this task is that behaviour cannot change.

- [ ] **Step 3: Declare the module**

In `crates/ui-egui/src/lib.rs`, add `pub mod home_screen;` on its own line between `pub mod guide_layout_ui;` (line 93) and `pub mod hold_keys;` (line 94), matching the surrounding indentation and ordering.

- [ ] **Step 4: Delete the moved code from `canvas.rs` and repoint the call**

In `crates/ui-egui/src/canvas.rs`:
- delete lines 2369–2513 (the five moved items, including their doc comments);
- change line 1730 from `start_screen(app, ui);` to `crate::home_screen::start_screen(app, ui);`.

Leave everything else in `canvas.rs` alone — in particular `paint_dots` stays, because `canvas.rs` still calls it at line 1735.

- [ ] **Step 5: Build and run the suite**

Run: `cargo test -p photocraft-ui-egui --lib 2>&1 | Select-Object -Last 20`
Expected: PASS, and the same test count as Step 1. `chrome_ui.rs:380-420` (Home shows/closes, auto-show off) and `canvas.rs:6000-6013` (Wayland hint, remove a recent entry) must still be green — those files are untouched and now exercise the moved code through the new module path.

- [ ] **Step 6: Lint**

Run: `cargo clippy -p photocraft-ui-egui --all-targets -- -D warnings 2>&1 | Select-Object -Last 20`
Expected: no warnings. A `dead_code` warning on `home_recent` or `start_screen_drop_hint` means the visibility is wrong — they are `pub(crate)` and used within the new module, so they must be referenced by `start_screen`; they are.

- [ ] **Step 7: Commit**

```bash
git add crates/ui-egui/src/home_screen.rs crates/ui-egui/src/canvas.rs crates/ui-egui/src/lib.rs
git commit -m "refactor(ui): move the Home screen out of canvas.rs into home_screen.rs

canvas.rs is over 6000 lines and the Home screen does not draw a canvas. Move
start_screen, home_recent, start_screen_drop_hint, HOME_RECENT and
HOME_RECENT_ROW verbatim into a module of their own.

No behaviour change: the width heuristic, the card size, the recent-file list,
the Wayland drop hint and the Discord/link row are byte-for-byte as they were.
The existing Home tests in chrome_ui.rs and canvas.rs cover the moved path."
```

---

### Task 2: Templates gallery and the category state

**Files:**
- Modify: `crates/ui-egui/src/home_screen.rs` (add `templates`, helpers, and the tests module declaration)
- Create: `crates/ui-egui/src/home_screen_tests.rs`
- Modify: `crates/ui-egui/src/chrome_ui.rs:14-20` and `:37-41` (one new field + `Default`)
- Modify: `crates/ui-egui/src/new_doc_ui.rs:346` (`Card` and `recent_cards` → `pub(crate)`; visibility only)
- Modify: `crates/ui-egui/src/lib.rs` (declare `home_screen_tests` in the existing test-module block near line 2347)

**Interfaces:**
- Consumes: `PhotocraftApp`, `new_doc_ui::CATEGORIES`, `new_doc_ui::apply_preset`, `new_doc_ui::Preset` from Task 1's crate context.
- Produces:
  ```rust
  // crates/ui-egui/src/home_screen.rs
  pub const TEMPLATE_CARD: egui::Vec2 = egui::vec2(164.0, 112.0);
  pub const TEMPLATE_COLUMNS: usize = 3;
  pub fn templates(app: &mut PhotocraftApp, ui: &mut egui::Ui)
  pub fn category_presets(category: usize) -> &'static [crate::new_doc_ui::Preset]

  // crates/ui-egui/src/chrome_ui.rs
  pub struct ChromeState { /* existing */ pub home_category: usize }

  // crates/ui-egui/src/home_screen_tests.rs
  pub fn every_static_preset_renders_a_card()        // 6 categories sum to 31
  pub fn recent_category_shows_recent_documents()
  pub fn an_out_of_range_category_falls_back_to_recent()
  pub fn chrome_category_defaults_and_round_trips()
  ```

- [ ] **Step 1: Add the state field with a serde default**

In `crates/ui-egui/src/chrome_ui.rs`, append to `ChromeState` (after the `home` field, line 19) a documented field:

```rust
/// Index into `new_doc_ui::CATEGORIES` for the Home screen's Templates panel; 0 is "Recent".
/// Defaults to 0 so a state file written before this field existed still loads.
#[serde(default)]
pub home_category: usize,
```

Update `impl Default for ChromeState` (line 39) to add `home_category: 0`.

The struct already carries `#[serde(default, rename_all = "camelCase")]` at line 13, so an older state file without the key deserialises to `0`. The per-field `#[serde(default)]` is redundant but harmless; keep it only if the file's existing fields use it, and drop it otherwise to match local style.

- [ ] **Step 2: Write the failing test for the state field**

Create `crates/ui-egui/src/home_screen_tests.rs`:

```rust
//! Home screen: the Templates panel and the category state behind it.

use crate::chrome_ui::ChromeState;

#[test]
fn chrome_category_defaults_and_round_trips() {
    assert_eq!(ChromeState::default().home_category, 0);
    let json = serde_json::to_string(&ChromeState { home_category: 4, ..Default::default() }).unwrap();
    assert_eq!(serde_json::from_str::<ChromeState>(&json).unwrap().home_category, 4);
}

#[test]
fn a_state_file_without_the_category_still_loads() {
    let loaded: ChromeState = serde_json::from_str(r#"{"statusInfo":"dimensions"}"#).unwrap();
    assert_eq!(loaded.home_category, 0);
}
```

Declare it in `lib.rs` beside the existing test modules (the block around line 2347, e.g. `mod canvas_menu_tests;`): `mod home_screen_tests;`.

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p photocraft-ui-egui --lib chrome_category 2>&1 | Select-Object -Last 20`
Expected: FAIL to compile — `no field home_category on type ChromeState`.

- [ ] **Step 4: Add the field and make the test pass**

Step 1 already did this. Re-run:
Run: `cargo test -p photocraft-ui-egui --lib chrome_category 2>&1 | Select-Object -Last 10`
Expected: PASS, 2 tests.

- [ ] **Step 5: Add `category_presets`, the total-safe lookup**

In `crates/ui-egui/src/home_screen.rs`:

```rust
/// The presets of `category`, or an empty slice when the index is out of range.
///
/// `home_category` is deserialised state and can be set through the control channel,
/// so the index is never trusted.
pub fn category_presets(category: usize) -> &'static [crate::new_doc_ui::Preset] {
    crate::new_doc_ui::CATEGORIES.get(category).map_or(&[], |c| c.1)
}
```

- [ ] **Step 6: Write the failing tests for the panel**

Add to `crates/ui-egui/src/home_screen_tests.rs` three `kittest`-based tests mirroring the harness style at `crates/ui-egui/src/direction_tests.rs:97`. Each builds a `PhotocraftApp` with the same helper the existing UI tests use, runs the panel inside `egui::CentralPanel`, and counts laid-out cards by their widget label:

- `every_static_preset_renders_a_card` — for each category index 1..6 (skipping index 0, see the next test), the card count equals `CATEGORIES[i].1.len()`, and those six counts sum to 31.
- `recent_category_shows_recent_documents` — with `home_category == 0` and an empty `session.presets.recent_documents`, zero preset cards render and the panel still draws. This mirrors `new_doc_ui.rs:466-469`, where index 0 substitutes recently-used sizes for the static presets. With three fake recent sizes in `session.presets.recent_documents`, exactly three cards render.
- `an_out_of_range_category_falls_back_to_recent` — set `home_category = 999` and `-1` is impossible on `usize`; assert 999 renders the Recent category and does not panic.

- [ ] **Step 7: Run the tests to verify they fail**

Run: `cargo test -p photocraft-ui-egui --lib home_screen 2>&1 | Select-Object -Last 20`
Expected: FAIL — `templates` is not defined.

- [ ] **Step 8: Implement `templates`**

In `crates/ui-egui/src/home_screen.rs`, add `pub fn templates(app: &mut PhotocraftApp, ui: &mut egui::Ui)` with this structure:

1. `let t = crate::theme::Tokens::get(ui.ctx());`
2. A chip row over `crate::new_doc_ui::CATEGORIES`, one `tl!(c.0)` label per category, active one underlined in `t.text` exactly as `new_doc_ui.rs:453-455` draws its category tabs — reuse that styling rather than inventing one. Clicking a chip sets `app.ui.chrome.home_category`.
3. Below it, the grid. Resolve the category with `category_presets(app.ui.chrome.home_category)`, then if the resolved category is index 0 (`CATEGORIES[0]`), render `app.session.presets.recent_documents`. **Do not reimplement this.** `new_doc_ui.rs:346` already has
   ```rust
   fn recent_cards(f: &Map<String, Value>, recent: &[DocumentSettings], units: &UnitsAndRulers) -> Vec<Card>
   ```
   Widen exactly two items in `crates/ui-egui/src/new_doc_ui.rs` from private to `pub(crate)`: the `Card` enum and `recent_cards`. No signature change, no logic change, no test change. Two private items becoming crate-visible is what guarantees the Home screen and the New dialog can never drift apart — the alternative, a second implementation of the recent-cards rule, is exactly the kind of duplication a reviewer rejects. Call it with a fresh empty field map and let the card click handler fill it.
4. Cards in chunks of `TEMPLATE_COLUMNS`, each `TEMPLATE_CARD`, drawn at the preset's real aspect ratio: fit `(width, height)` into a 64 × 44 box preserving the ratio, paint a filled `rect` with `t.panel`-family colour and a subtle border. Under it, `tl!(preset.0)` at 11.0 and the dimensions at 9.5 in `t.text_faint`. This matches the sizes at `new_doc_ui.rs:479-481`.
5. Clicking a card: build the dialog fields the way the dialog's own card click does, then `crate::new_doc_ui::apply_preset(&mut fields, preset, &app.session.prefs().units_and_rulers)`, then `app.open_dialog(crate::state::DialogKind::NewDocument, fields)`. Read the dialog's click handler and mirror it; do not invent a second creation path.

Empty category or empty Recent renders the chip row and an empty grid area — no error, no special case.

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test -p photocraft-ui-egui --lib home_screen 2>&1 | Select-Object -Last 15`
Expected: PASS, 5 tests (2 from Step 2, 3 from Step 6).

- [ ] **Step 10: Verify no i18n regressions and lint**

Run: `cargo xtask i18n-coverage 2>&1 | Select-Object -Last 20`
Expected: every catalog still covers the English key universe. This task must add **no** rows to any `.tsv` — if coverage reports a new missing key, a label was written as a literal instead of going through `tl!(name)`.

Run: `cargo clippy -p photocraft-ui-egui --all-targets -- -D warnings 2>&1 | Select-Object -Last 20`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add crates/ui-egui/src/home_screen.rs crates/ui-egui/src/home_screen_tests.rs crates/ui-egui/src/chrome_ui.rs crates/ui-egui/src/lib.rs
git commit -m "feat(ui): Templates gallery on the Home screen, from the New dialog's presets

The Home screen offered New and Open and nothing else, while the New Document
dialog already carries seven categories and 33 presets that nothing outside that
dialog could reach. Surface them.

Cards are 164x112 in three columns, the same size and styling the dialog uses, so
the same preset looks like the same thing in both places. Labels go through the
dialog's existing dynamic tl!(name) lookup, so no catalog gains a row.

'recent' behaves as it does in the dialog: index 0 lists recently-used sizes from
session.presets.recent_documents instead of its two static presets. An index
that came in out of range falls back to Recent rather than indexing.
home_category is serde-defaulted to 0 and lives on ChromeState, so the control
channel and MCP can read and set it."
```

---

### Task 3: Responsive routing and the dashboard layout

**Files:**
- Modify: `crates/ui-egui/src/home_screen.rs` (add `screen`, `layout_for`, `Layout`, `full_dashboard`)
- Modify: `crates/ui-egui/src/canvas.rs:1730` (call `screen` instead of `start_screen`)
- Modify: `crates/ui-egui/src/home_screen_tests.rs`

**Interfaces:**
- Consumes: `start_screen`, `templates`, `home_recent`, `start_screen_drop_hint` from Tasks 1–2.
- Produces:
  ```rust
  // crates/ui-egui/src/home_screen.rs
  pub const DASHBOARD_MIN_WIDTH: f32 = 900.0;
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Layout { Compact, Dashboard }
  pub fn layout_for(available_width: f32) -> Layout
  pub fn screen(app: &mut PhotocraftApp, ui: &mut egui::Ui)
  fn full_dashboard(app: &mut PhotocraftApp, ui: &mut egui::Ui)
  ```

- [ ] **Step 1: Write the failing tests for the boundary and the minimum window**

Add to `crates/ui-egui/src/home_screen_tests.rs`:

```rust
#[test]
fn the_dashboard_boundary_is_owned_by_the_wide_side() {
    assert_eq!(layout_for(899.9), Layout::Compact);
    assert_eq!(layout_for(900.0), Layout::Dashboard);
    assert_eq!(layout_for(1440.0), Layout::Dashboard);
}

#[test]
fn a_window_at_the_minimum_size_does_not_clip() { /* 760x480, every label rect inside available */ }
#[test]
fn a_wide_window_uses_the_dashboard() { /* 1440x900 draws the three columns */ }
```

The clipping test walks the painted `egui::Shape` rects and every allocated label rect and asserts each lies inside `ui.available_rect_before_wrap()`. Run it once at `760 × 480` in the Pro theme and once in the default theme — the Pro theme enables the Home button (`chrome_ui.rs:199`) and can change metrics.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p photocraft-ui-egui --lib home_screen 2>&1 | Select-Object -Last 20`
Expected: FAIL — `layout_for` is not defined.

- [ ] **Step 3: Add `Layout`, `layout_for` and `screen`**

```rust
/// Below this width of canvas area the Home screen stays a centred card; at or above
/// it, the three-column dashboard. 900 px leaves the 760 px minimum window
/// (`apps/photocraft/src/main.rs:76`) on the compact side.
pub const DASHBOARD_MIN_WIDTH: f32 = 900.0;

pub fn layout_for(available_width: f32) -> Layout {
    if available_width >= DASHBOARD_MIN_WIDTH { Layout::Dashboard } else { Layout::Compact }
}

/// The Home screen: today's card when the canvas is narrow, the dashboard when it
/// is not.
pub fn screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    match layout_for(ui.available_rect_before_wrap().width()) {
        Layout::Compact => start_screen(app, ui),
        Layout::Dashboard => full_dashboard(app, ui),
    }
}
```

In `canvas.rs`, change line 1730 to `crate::home_screen::screen(app, ui);`.

- [ ] **Step 4: Implement `full_dashboard`**

Three columns over `ui.available_rect_before_wrap()`: **200 px** left, the remainder centre, **260 px** right. Inside each column stack `widgets::primary_button` / `widgets::secondary_button`, `templates`, `home_recent` and `links::discord_button` / `links::link_row` — every one of those already exists and is already called by `start_screen`, so the dashboard reuses them rather than reimplementing.

Wrap the whole body in `egui::ScrollArea::vertical().id_salt("home-dashboard")` so short windows scroll instead of clipping, the same way `new_doc_ui.rs:483` wraps its preset grid.

The left rail is `New`, `Open`, the drop hint (`start_screen_drop_hint`), and recent files; the centre is `templates`; the right is recent files with thumbnails omitted, then the Discord button and link row. Keep every element the current card shows — a layout change that drops a control is a regression, not a redesign.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p photocraft-ui-egui --lib home_screen 2>&1 | Select-Object -Last 20`
Expected: PASS, 8 tests. Then run the whole crate:
Run: `cargo test -p photocraft-ui-egui --lib 2>&1 | Select-Object -Last 5`
Expected: PASS, and the test count is the Step 1 baseline plus 8.

- [ ] **Step 6: Verify visually**

Run: `cargo run -p photocraft-ui-egui --example snapshot 2>&1 | Select-Object -Last 5`
Expected: a PNG is written. **Open the PNG and look at it.** A dashboard that renders but reads badly is not done.

Then launch the app and look at it at 1440 × 900, 900 × 600 and 760 × 480, in both the default and Pro themes.

- [ ] **Step 7: Full gate**

Run each and read the output:
```sh
cargo clippy -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo xtask i18n-coverage
```
Expected: all clean, no layer violation, wasm still compiles, coverage unchanged.

- [ ] **Step 8: Commit**

```bash
git add crates/ui-egui/src/home_screen.rs crates/ui-egui/src/canvas.rs crates/ui-egui/src/home_screen_tests.rs
git commit -m "feat(ui): responsive Home screen — dashboard at 900px and wider

The Home screen was a fixed 460px card, so on a wide window it floated in the
middle of a large empty workspace while the templates next to it went unused.

Route on the canvas width: below 900px today's card, unchanged; at or above it
a three-column dashboard (actions and recents, templates, recents and links).
The body scrolls, so the 760x480 minimum window never clips.

Every control the card had is still there — the layout changed, the feature set
did not."
```

---

### Task 4: Record the result where the project tracks it

**Files:**
- Modify: `docs/ui-parity.md`
- Modify: `docs/scorecard.toml` → the scorecard source (`scorecard/ui.toml`) — regenerate, do not hand-edit `docs/scorecard.md`
- Modify: `log/devlog.md`

**Interfaces:**
- Consumes: the merged PR 1–3 behaviour.
- Produces: no code.

- [ ] **Step 1: Update the UI parity rows**

In `docs/ui-parity.md`, add a Home-screen row set covering: Templates gallery, category tabs, recent files, and the responsive split. Cite the behaviour with file:line references into `home_screen.rs`.

- [ ] **Step 2: Regenerate the scorecard**

Run: `cargo xtask scorecard 2>&1 | Select-Object -Last 15`
Expected: `docs/scorecard.md` regenerated. UI-217-10's "no option to hide the Home screen recents" is resolved by this layout (recents are a column, not a fixed block) — flip that row in `scorecard/ui.toml` first, then regenerate. Do not hand-edit the generated markdown; CI checks it.

- [ ] **Step 3: Append the dev log entry**

Append one terse entry to `log/devlog.md`: what landed (module move, Templates gallery, responsive dashboard), the measured numbers (33 presets, 7 categories, the 900 px threshold, test count), and what is still open (PR 3 Learn column, PR 4 thumbnails).

- [ ] **Step 4: Commit**

```bash
git add docs/ui-parity.md scorecard/ui.toml docs/scorecard.md log/devlog.md
git commit -m "docs: record the Home screen dashboard in the parity docs and dev log"
```

---

## Notes for the implementer

- **Task 1 is the one that must be provably inert.** If any test in `chrome_ui.rs` or `canvas.rs` that touches Home changes behaviour during that task, something was edited rather than moved. Revert and re-do it as a cut-and-paste.
- **Task 2 reads `new_doc_ui.rs` before writing anything.** The recent-card helper, the card click handler and the field map for `DialogKind::NewDocument` all live there. Mirroring the dialog's own code path is the point — a second creation path would be a new command by the back door, which the Global Constraints forbid.
- **Nothing here needs a new `i18n` row.** If Step 10 of Task 2 reports missing coverage, a label bypassed `tl!`.
- **If a task turns out to need a new string, a new command, or an engine change, stop.** That means the spec's boundary is wrong, and the spec is what should change, not the constraint.