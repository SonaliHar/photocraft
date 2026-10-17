# Custom panels

> **Last updated:** 2026-10-15 · **Change:** minor (layout options: one section open at a time, text labels, buttons per row, wide buttons, tool icons, text that shrinks to fit)

Photoshop users often build their own panels of one-click buttons, with Adobe Configurator in
the past or with extension panels that ship alongside actions. PhotoCraft has the same idea
built in: **Window › Custom Panels**. A custom panel is a floating window of buttons the user
arranges; it needs no plug-in and runs no third-party code.

Source: `crates/ui-egui/src/custom_panels.rs`. Tests: the `tests` module of that file.

## Using a panel

- **Window › Custom Panels › New Custom Panel** makes a panel and opens it in edit mode. Every
  panel is then listed under Window › Custom Panels, checked while it is shown.
- A panel holds **buttons**, **headings**, **text labels** and **dividers**. Buttons flow in a
  grid (1–6 columns) between them. Clicking a heading folds its section away.
- **One section open at a time** draws the headings as full-width bars, like the accordion
  panels many Photoshop extensions use; opening a section folds the others.
- A heading, label or divider can set its own **buttons per row** (Columns) for the buttons after
  it, so one section can mix rows of five small buttons with rows of two wide ones.
- A button can be **wider** than one grid cell (Width), up to the whole row.
- A tool button can **show the tool's icon** instead of its name.
- A long button name shrinks (down to 8.5 pt) to fit before it is cut short.
- A button does one of three things:
  - runs a menu command (`Image › Image Size…` opens its dialog, as the menu does);
  - selects a toolbar tool;
  - plays an action from the Actions panel, including actions loaded from Photoshop `.atn`
    files with Load Actions.
- A command button can carry params, so it runs without a dialog: for example
  `filter.blur.gaussianBlur` with `{"radius": 3}` labelled "Blur 3".
- A button whose command is unavailable right now (no document, wrong layer kind) is greyed, as
  the menu item is. A failure is reported in the status bar.

## Editing a panel

Click the pencil in the panel's title bar, or right-click the panel and choose **Edit Panel**.
In edit mode:

- the name in the title bar can be typed over;
- clicking an item selects it instead of running it; the selected item's label, colour (the
  Actions panel's Button Mode colours) and position can be changed, or it can be removed;
- **Add Button…** searches every menu command, tool and action by name;
- **Add Heading**, **Add Text** and **Add Divider** insert after the selected item;
- a selected button has **Width** and, for tools, **Show as Icon**; a selected heading, label or
  divider has **Columns** (Auto keeps the count above it);
- **Columns − / +** at the bottom change the panel's grid, and **One section open at a time**
  switches the accordion layout;
- **Delete Panel** removes the panel; **Done** (or the pencil again) leaves edit mode.

Window › Workspace › Lock Workspace stops panels from being moved, as with the other floating
panels.

## Saving and sharing

Panels are saved in the preferences (`customPanels`) on every change and come back at launch.
A saved panel or item that no longer parses is dropped rather than stopping the app; names and
window ids are made unique. A button for a command that a later version doesn't have stays in the
panel and reports that it isn't available when pressed.

`window.customPanel.export` returns a panel as JSON (`{"version": 1, "panel": {…}}`) and
`window.customPanel.import` adds one from that JSON, so a panel can be moved between machines or
shared. Actions a panel plays travel separately, as `.atn` files.

## Automation

Every change is a shell command, so agents can build panels through the control channel or MCP
(`ui.menu.invoke`). `panel` is a panel name or index.

| Command | Params | Does |
|---|---|---|
| `window.customPanel.new` | `name?`, `columns?`, `edit?` (default true) | Makes a panel |
| `window.customPanel.show` | `panel`, `show?` | Shows, hides or toggles it |
| `window.customPanel.edit` | `panel`, `on?` | Enters or leaves edit mode |
| `window.customPanel.rename` | `panel`, `name` | Renames it (names are unique) |
| `window.customPanel.delete` | `panel` | Deletes it |
| `window.customPanel.addItem` | `panel`, `kind` (`command`, `action`, `heading`, `label`, `separator`), `id` / `action` + `set?`, `params?`, `label?`, `color?`, `span?` (1–6), `icon?` (tools), `columns?` (headings, labels, dividers), `collapsed?`, `at?` | Adds an item |
| `window.customPanel.updateItem` | `panel`, `index`, `label?`, `color?`, `span?`, `icon?`, `columns?` (null for Auto), `collapsed?`, `params?` | Changes an item |
| `window.customPanel.removeItem` | `panel`, `index` | Removes an item |
| `window.customPanel.moveItem` | `panel`, `from`, `to` | Moves an item |
| `window.customPanel.setLayout` | `panel`, `columns?` (1–6), `width?` (160–560), `accordion?` | Changes the grid and the section style |
| `window.customPanel.run` | `panel`, `index` | Presses a button |
| `window.customPanel.list` | | All panels |
| `window.customPanel.export` / `import` | `panel` / `definition` | Moves a panel as JSON |
| `window.customPanel.open.<n>` | | Window › Custom Panels › <panel n> |

Limits: 32 panels, 400 items per panel, 64 characters per name or label. A button can't run a
`window.customPanel.*` command (a button that presses buttons could loop), and a button that is
running can't press another.

## What custom panels are not

They don't load Photoshop extension panels (CEP `.zxp` or UXP plug-ins): those are web code, and
PhotoCraft runs no JavaScript (`AGENTS.md`). The usual way to bring one over is to load its
actions with Load Actions and rebuild its buttons as a custom panel. Hosting UXP panels stays
open in [`gaps.md`](gaps.md) (G16).

Not yet: docking a custom panel into the right-hand dock, icons on buttons other than tools, and
two labels side by side over one row.
