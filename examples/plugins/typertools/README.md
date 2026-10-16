# TyperTools for PhotoCraft

A **panel** plug-in (plug-in ABI v2, [`docs/plugins-v2.md`](../../../docs/plugins-v2.md)) that ports
[TyperTools 1.4.9](https://github.com/Swirt/typertools-src) — the manga and comic lettering workflow
panel for Adobe Photoshop — to PhotoCraft, as a Linux-first, clean-room Rust reimplementation.

It is the port the panel ABI was designed for, and the largest thing that runs on it: about 6,500
lines of ordinary safe Rust, no `unsafe`, no imports, no JavaScript, and one `.wasm` module.

## What it does

A letterer pastes a script — one line of dialogue per row — tags a line with a style's prefix, and
sets it into the page in that style. The panel keeps a library of styles organised in folders,
exports and imports it in TyperTools' own JSON format, and drives the document through PhotoCraft's
own type engine.

| Page | Controls | Operations it sends |
|---|---|---|
| Lettering | the script box, line field and ◀ ▶ ⏮ ⏭ | `ui.set` for the values, nothing to the document |
| | Create / Insert / Align | `doc.info`, then `doc.command` → `type.create` / `type.edit`, one history step per action |
| | A− / A+ (size steps) | `doc.info`, `type.edit` — one document pixel of points, keeping the layer where it was |
| | Apply / Apply, keep size | `doc.command` → `type.setStyle` on every selected layer |
| Text size | 1–999 % | scales the size *and* the leading on Create and Insert, as TyperTools' does |
| Styles | style rows, the default style, the apply arrow | `type.setStyle`; the arrow with Ctrl (Apply, keep size) adopts the layer's current size first |
| Style editor | name, prefixes, colour, folder, font, face, size, leading, tracking, kerning, both scales, baseline shift, caps, align, faux bold/italic, underline, strike, ligatures, indents, space before/after, colour, anti-aliasing | `doc.command` → `type.fonts` for the two dropdowns and `type.create` / `type.setStyle` for a preview |
| Folders | add, rename, delete; a style changes folder in the editor's Folder list | carried in the style library, stored with it |
| Settings | point text or box text, ignore prefixes, text size default, starter library, reset | stored with the library |
| Help | what is here, what is not and why, the library as JSON (Show JSON / Load / Merge / Clear) | `storage.set` |

Everything the panel changes in the document goes through five engine commands — `type.create`,
`type.edit`, `type.setStyle`, `type.info`, `type.fonts` — which are exactly the five the host's
allow-list grants a panel that asks for `read` and `addLayers`
(`crates/engine/src/plugin_panel_cmds/permissions.rs`). Nothing else is reachable from it: no file,
no preference, no plug-in and no action.

## Build

```sh
rustup target add wasm32-unknown-unknown
sh build.sh
```

`build.sh` runs `cargo build --release --target wasm32-unknown-unknown` and copies the module to
`crates/plugins/tests/fixtures/typertools.wasm`, which is the fixture the host's tests use
(`crates/engine/src/plugin_panel_cmds/tests.rs`, `the_typertools_plug_in_letters_a_page_end_to_end`).
This crate is not a member of the PhotoCraft workspace: it builds for wasm only, and nothing in it
may reach a host crate.

## Install

```sh
# Into a running app (control channel) or the CLI, as a command:
plugin.install {"path": "examples/plugins/typertools/target/wasm32-unknown-unknown/release/photocraft_plugin_typertools.wasm"}
plugin.panel.open {"id": "org.photocraft.typertools"}
```

Or copy the `.wasm` into the folder set in Edit › Preferences › Plug-ins (with *Use Additional
Plug-ins Folder* on); it then appears under Window › Plug-ins, and picking that row opens its window.

## What it answers

The reply to each event is a JSON **array** of operations, written into the buffer `pc_alloc` handed
out and returned as `len << 32 | ptr`; `0` means "nothing to say". The module exports the ABI v2
entry points and **imports nothing at all** — the host refuses any module that imports, so a panel
reaches the host by returning operations, never by calling into it.

No secrets, no files, and no network: a panel's whole outside world is `doc.info`, the commands it was
granted, and the key-value store the host keeps for it under the plug-in's own id.

## What is not ported, and why

Four of TyperTools' features have no equivalent in ABI v2 or in PhotoCraft's type engine yet. Each is
refused with a message on screen rather than silently doing nothing, or — where the difference is only
in the control — is a button instead of the gesture:

- **Keyboard shortcuts.** The three hotkey actions are buttons: ABI v2 has no command or shortcut
  contribution, so a panel cannot add one. On Wayland this is also the better answer — an in-app
  shortcut behaves, a system-global hotkey needs the XDG portal.
- **File dialogs for import/export.** The ABI has no file dialogs, so the library JSON is shown in a
  text box to copy out or paste in. The format is TyperTools' own, so a library still moves between
  Photoshop's TyperTools and this port.
- **Hanging punctuation (burasagari)** and **hyphenation rendering**: `type.edit` stores both, but
  PhotoCraft's line breaking applies neither yet.
- **The marquee is not touched.** TyperTools insets the selection by 10 px and clears it when it
  aligns; this port reads the selection and leaves it alone, because ABI v2 grants no selection
  command.
- **Dragging a style or folder to reorder it.** The widget set has no drag list, so the Styles and
  Folders pages move a row with Up/Down buttons — the same reorder, and the order is what decides
  which of two styles sharing a prefix wins.

## Licensing

Clean-room. TyperTools is MIT (© 2017 Swirt) and this port reimplements its *behaviour*; no source,
artwork, locale file or vendored library of it is copied, and in particular the GPL `app_src/lib/jam/*`
that TyperTools bundles (© Michel MARIANI) and Adobe's proprietary `CSInterface.js` are not used at
all — there is no ExtendScript here. See the row in [`ATTRIBUTION.md`](../../../ATTRIBUTION.md).

The port itself is MIT OR Apache-2.0, like the rest of PhotoCraft. Its interface is English only:
TyperTools' own locale strings are MIT and could be reused with a row in `ATTRIBUTION.md`, but none
are — the panel has one language until someone asks for a second.
