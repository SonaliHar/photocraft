//! Window › Custom Panels: panels the user builds from buttons, like the button panels Photoshop
//! users make with Configurator or install as extensions.
//!
//! A custom panel is a floating window holding, in order:
//! - **buttons** that run any menu command or toolbar tool (a `command` item, with optional
//!   params, so "Gaussian Blur 3 px" can be one click) or play an action from the Actions panel
//!   (an `action` item);
//! - **headings** that start a section and fold it away when clicked;
//! - **dividers**.
//!
//! Buttons flow in a grid of `columns` between headings and dividers. The panel's pencil button
//! (or right-click › Edit Panel) switches it to edit mode: click an item to select it, rename or
//! colour it, move it, remove it, or add buttons from a search over every command, tool and
//! action.
//!
//! Every change is also a `window.customPanel.*` command, so the control channel and MCP can
//! build panels too, and `window.customPanel.export` / `import` move a panel between machines as
//! JSON. The definitions live in [`CustomPanelsUi`] (part of `UiState`, so `ui.inspect` shows
//! them) and are saved in `Preferences::custom_panels` on every change ([`save`]) and read back
//! at launch ([`restore`]). A malformed saved panel or item is dropped, never a crash.

use egui::{Color32, RichText, Sense, Stroke, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Every custom panel command id starts with this.
pub const PREFIX: &str = "window.customPanel.";
/// Window › Custom Panels › <panel>: shows or hides the panel at that index.
pub const OPEN_ITEM: &str = "window.customPanel.open.";

const IDS: [&str; 16] = [
    "window.customPanel.new",
    "window.customPanel.show",
    "window.customPanel.edit",
    "window.customPanel.rename",
    "window.customPanel.delete",
    "window.customPanel.addItem",
    "window.customPanel.updateItem",
    "window.customPanel.removeItem",
    "window.customPanel.moveItem",
    "window.customPanel.setLayout",
    "window.customPanel.run",
    "window.customPanel.list",
    "window.customPanel.export",
    "window.customPanel.import",
    // File dialogs: Window › Custom Panels › Import Panel…, and Export Panel… in edit mode.
    "window.customPanel.importFile",
    "window.customPanel.exportFile",
];

/// Most panels the user can keep.
pub const MAX_PANELS: usize = 32;
/// Most items one panel holds.
pub const MAX_ITEMS: usize = 400;
/// Longest panel name or item label, in characters.
pub const MAX_TEXT: usize = 64;
/// Buttons per row.
pub const COLUMNS: std::ops::RangeInclusive<u8> = 1..=6;
/// Panel width in points.
pub const WIDTH: std::ops::RangeInclusive<f32> = 160.0..=560.0;
/// The export format's version.
const EXPORT_VERSION: u64 = 1;

/// One entry of a custom panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PanelItem {
    /// A button that runs a menu command or selects a tool (`tools.select.<tool>`).
    Command {
        id: String,
        /// Params passed to the command; null runs it as the menu does (with its dialog).
        #[serde(default, skip_serializing_if = "Value::is_null")]
        params: Value,
        /// Empty shows the command's own name.
        #[serde(default)]
        label: String,
        /// One of [`photocraft_engine::actions_cmds::BUTTON_COLORS`]; empty is none.
        #[serde(default)]
        color: String,
        /// How many grid columns the button is wide.
        #[serde(default = "one", skip_serializing_if = "is_one")]
        span: u8,
        /// A tool button shows the tool's icon instead of its name.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        icon: bool,
    },
    /// A button that plays an action from the Actions panel.
    Action {
        action: String,
        /// The action set, when two sets hold actions of the same name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set: Option<String>,
        #[serde(default)]
        label: String,
        #[serde(default)]
        color: String,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        span: u8,
    },
    /// A section title; clicking it folds the items up to the next heading.
    Heading {
        label: String,
        #[serde(default)]
        collapsed: bool,
        /// Buttons per row in this section; none uses the panel's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        columns: Option<u8>,
    },
    /// A small centred title inside a section ("Sharpen", "Curves").
    Label {
        label: String,
        /// Buttons per row from here on; none keeps the current count.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        columns: Option<u8>,
    },
    /// A thin line between buttons.
    Separator {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        columns: Option<u8>,
    },
}

fn one() -> u8 {
    1
}

fn is_one(n: &u8) -> bool {
    *n == 1
}

fn clamp_columns(c: u8) -> u8 {
    c.clamp(*COLUMNS.start(), *COLUMNS.end())
}

impl PanelItem {
    fn is_button(&self) -> bool {
        matches!(self, PanelItem::Command { .. } | PanelItem::Action { .. })
    }

    /// Grid columns the button takes (1 for anything that isn't a button).
    pub fn span(&self) -> u8 {
        match self {
            PanelItem::Command { span, .. } | PanelItem::Action { span, .. } => (*span).max(1),
            _ => 1,
        }
    }

    /// The buttons-per-row a heading, label or divider sets.
    pub fn columns(&self) -> Option<u8> {
        match self {
            PanelItem::Heading { columns, .. } | PanelItem::Label { columns, .. } | PanelItem::Separator { columns } => *columns,
            _ => None,
        }
    }

    fn label_mut(&mut self) -> Option<&mut String> {
        match self {
            PanelItem::Command { label, .. } | PanelItem::Action { label, .. } | PanelItem::Heading { label, .. } | PanelItem::Label { label, .. } => {
                Some(label)
            }
            PanelItem::Separator { .. } => None,
        }
    }

    fn color(&self) -> &str {
        match self {
            PanelItem::Command { color, .. } | PanelItem::Action { color, .. } => color,
            _ => "",
        }
    }

    /// What the item shows: its own label, else the command's (translated) name or the
    /// action's name.
    pub fn display_label(&self) -> String {
        match self {
            PanelItem::Command { label, .. } | PanelItem::Action { label, .. } | PanelItem::Heading { label, .. } | PanelItem::Label { label, .. }
                if !label.trim().is_empty() =>
            {
                label.clone()
            }
            PanelItem::Command { id, .. } => default_label(id),
            PanelItem::Action { action, .. } => action.clone(),
            PanelItem::Heading { .. } => tl!("Section").to_string(),
            PanelItem::Label { .. } | PanelItem::Separator { .. } => String::new(),
        }
    }

    /// Clamp text, drop unknown colours and keep tool ids and command ids well formed.
    fn sanitize(&mut self) {
        match self {
            PanelItem::Command { label, color, params, span, .. } => {
                *label = clip(label);
                *color = known_color(color);
                *span = clamp_columns(*span);
                if !(params.is_null() || params.is_object()) {
                    *params = Value::Null;
                }
            }
            PanelItem::Action { action, set, label, color, span } => {
                *action = action.trim().to_string();
                *set = set.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                *label = clip(label);
                *color = known_color(color);
                *span = clamp_columns(*span);
            }
            PanelItem::Heading { label, columns, .. } | PanelItem::Label { label, columns } => {
                *label = clip(label);
                *columns = columns.map(clamp_columns);
            }
            PanelItem::Separator { columns } => *columns = columns.map(clamp_columns),
        }
    }
}

/// A user-built panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomPanel {
    /// Stable id for the window position (names can change).
    pub uid: u64,
    pub name: String,
    pub visible: bool,
    pub columns: u8,
    pub width: f32,
    /// One section open at a time: headings are drawn as full-width bars, and opening one folds
    /// the others.
    pub accordion: bool,
    pub items: Vec<PanelItem>,
}

impl Default for CustomPanel {
    fn default() -> Self {
        Self { uid: 0, name: String::new(), visible: true, columns: 3, width: 260.0, accordion: false, items: Vec::new() }
    }
}

impl CustomPanel {
    fn sanitize(&mut self) {
        self.name = clip(&self.name);
        self.columns = clamp_columns(self.columns);
        self.width = if self.width.is_finite() { self.width.clamp(*WIDTH.start(), *WIDTH.end()) } else { CustomPanel::default().width };
        self.items.truncate(MAX_ITEMS);
        for item in &mut self.items {
            item.sanitize();
        }
    }
}

/// The panels and the edit-mode state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomPanelsUi {
    pub panels: Vec<CustomPanel>,
    /// The panel in edit mode (its `uid`).
    pub editing: Option<u64>,
    /// The selected item of the panel being edited.
    pub selected: Option<usize>,
    /// The Add Button search, while it's open.
    pub picker: Option<String>,
    /// The panel name being typed in edit mode, until it is committed.
    pub name_draft: Option<String>,
    /// A button is running: a command it runs can't press another panel button.
    #[serde(skip)]
    running: bool,
}

// ------------------------------------------------------------------ helpers

fn clip(s: &str) -> String {
    s.trim().chars().take(MAX_TEXT).collect()
}

fn known_color(c: &str) -> String {
    let c = c.trim().to_ascii_lowercase();
    if photocraft_engine::actions_cmds::BUTTON_COLORS.contains(&c.as_str()) { c } else { String::new() }
}

/// The name a command or tool button shows when it has no label of its own.
fn default_label(id: &str) -> String {
    if let Some(tool) = crate::tool_shortcuts::tool(id) {
        let label = tl!(tool.label());
        // "Dodge Tool" reads as "Dodge" on a button, as on Photoshop panel buttons.
        return label.strip_suffix(" Tool").unwrap_or(label).to_string();
    }
    let label = crate::actions::label_of(id);
    crate::i18n::tr_id(crate::i18n::current(), id, &label).trim_end_matches('…').to_string()
}

/// Can a panel button run `id`? Live commands and tools, but not the custom panel commands
/// themselves (a button that presses buttons could loop) nor the Window › <document> items,
/// whose meaning changes as documents open and close.
pub fn can_add(id: &str) -> bool {
    !id.starts_with(PREFIX) && !id.starts_with(crate::menus::DOCUMENT_ITEM) && (crate::tool_shortcuts::tool(id).is_some() || crate::menus::is_live(id))
}

fn str_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}

fn index_param(p: &Value, key: &str) -> Result<usize, String> {
    p.get(key).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| format!("`{key}` must be a whole number"))
}

/// The panel named by `panel` (a name or an index).
fn find(app: &PhotocraftApp, p: &Value) -> Result<usize, String> {
    let list = &app.ui.custom_panels.panels;
    match p.get("panel") {
        Some(Value::String(name)) => list
            .iter()
            .position(|c| c.name == *name)
            .or_else(|| list.iter().position(|c| c.name.eq_ignore_ascii_case(name.trim())))
            .ok_or_else(|| format!("no custom panel named `{name}`")),
        Some(Value::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|i| *i < list.len()).ok_or_else(|| format!("no custom panel {n}")),
        _ => Err("`panel` must be a panel name or index".into()),
    }
}

fn unique_name(app: &PhotocraftApp, wanted: &str, except: Option<usize>) -> String {
    let taken = |n: &str| app.ui.custom_panels.panels.iter().enumerate().any(|(i, c)| Some(i) != except && c.name.eq_ignore_ascii_case(n));
    let base = if wanted.trim().is_empty() { tl!("Custom Panel").to_string() } else { clip(wanted) };
    if !taken(&base) {
        return base;
    }
    (2..=MAX_PANELS + 2).map(|n| format!("{base} {n}")).find(|n| !taken(n)).unwrap_or(base)
}

fn next_uid(app: &PhotocraftApp) -> u64 {
    app.ui.custom_panels.panels.iter().map(|c| c.uid).max().unwrap_or(0).saturating_add(1)
}

fn panel_mut(app: &mut PhotocraftApp, i: usize) -> Result<&mut CustomPanel, String> {
    app.ui.custom_panels.panels.get_mut(i).ok_or_else(|| "no such custom panel".to_string())
}

/// The item a params object describes (`addItem`, `import`).
fn item_from(p: &Value, check: bool, app: &PhotocraftApp) -> Result<PanelItem, String> {
    let kind = str_param(p, "kind").unwrap_or(if p.get("action").is_some() { "action" } else { "command" });
    let label = str_param(p, "label").unwrap_or("").to_string();
    let color = str_param(p, "color").unwrap_or("").to_string();
    let span = match p.get("span") {
        None | Some(Value::Null) => 1,
        Some(v) => v.as_u64().and_then(|n| u8::try_from(n).ok()).filter(|n| COLUMNS.contains(n)).ok_or("`span` must be 1 to 6")?,
    };
    let columns = match p.get("columns") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|n| u8::try_from(n).ok()).filter(|n| COLUMNS.contains(n)).ok_or("`columns` must be 1 to 6")?),
    };
    let mut item = match kind {
        "command" | "tool" => {
            let id = str_param(p, "id").map(str::trim).filter(|s| !s.is_empty()).ok_or("a command button needs an `id`")?.to_string();
            if check && !can_add(&id) {
                return Err(format!("`{id}` isn't a command a panel button can run"));
            }
            let params = p.get("params").cloned().unwrap_or(Value::Null);
            if !(params.is_null() || params.is_object()) {
                return Err("`params` must be an object".into());
            }
            let icon = p.get("icon").and_then(Value::as_bool).unwrap_or(false);
            if icon && crate::tool_shortcuts::tool(&id).is_none() {
                return Err("only tool buttons can show an icon".into());
            }
            PanelItem::Command { id, params, label, color, span, icon }
        }
        "action" => {
            let action = str_param(p, "action").map(str::trim).filter(|s| !s.is_empty()).ok_or("an action button needs an `action` name")?.to_string();
            let set = str_param(p, "set").map(str::to_string);
            if check && !app.session.actions.list.iter().any(|a| a.name == action && set.as_ref().is_none_or(|s| a.set.as_ref() == Some(s))) {
                return Err(format!("no action named `{action}` in the Actions panel"));
            }
            PanelItem::Action { action, set, label, color, span }
        }
        "heading" => PanelItem::Heading { label, collapsed: p.get("collapsed").and_then(Value::as_bool).unwrap_or(false), columns },
        "label" => PanelItem::Label { label, columns },
        "separator" => PanelItem::Separator { columns },
        other => return Err(format!("unknown item kind `{other}` (command, action, heading, label or separator)")),
    };
    item.sanitize();
    Ok(item)
}

// ------------------------------------------------------------------ persistence

/// Write the panels to the preferences when they changed.
pub fn save(app: &mut PhotocraftApp) {
    let now = serde_json::to_value(&app.ui.custom_panels.panels).unwrap_or(Value::Null);
    if app.session.prefs().custom_panels != now {
        app.session.prefs.edit(|p| p.custom_panels = now);
    }
}

/// Read the saved panels at launch. A panel or item that doesn't parse is dropped; names and
/// uids are made unique.
pub fn restore(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().custom_panels.clone();
    let mut panels: Vec<CustomPanel> = Vec::new();
    for v in saved.as_array().into_iter().flatten().take(MAX_PANELS) {
        let Some(mut panel) = parse_panel(v) else { continue };
        panel.sanitize();
        panels.push(panel);
    }
    app.ui.custom_panels = CustomPanelsUi::default();
    for mut panel in panels {
        panel.name = unique_name(app, &panel.name, None);
        if panel.uid == 0 || app.ui.custom_panels.panels.iter().any(|c| c.uid == panel.uid) {
            panel.uid = next_uid(app);
        }
        app.ui.custom_panels.panels.push(panel);
    }
}

/// A panel from JSON, keeping the items that parse. Every saved or exported panel has an `items`
/// list; an object without one isn't a panel.
fn parse_panel(v: &Value) -> Option<CustomPanel> {
    let obj = v.as_object().filter(|o| o.get("items").is_some_and(Value::is_array))?;
    let mut shell = obj.clone();
    let items = shell.remove("items");
    let mut panel: CustomPanel = serde_json::from_value(Value::Object(shell)).ok()?;
    panel.items = items.as_ref().and_then(Value::as_array).into_iter().flatten().filter_map(|i| serde_json::from_value(i.clone()).ok()).collect();
    Some(panel)
}

// ------------------------------------------------------------------ commands

/// Menu ids handled here.
pub fn handles(id: &str) -> bool {
    IDS.contains(&id) || id.strip_prefix(OPEN_ITEM).is_some_and(|n| n.parse::<usize>().is_ok())
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let i: usize = id.strip_prefix(OPEN_ITEM)?.parse().ok()?;
    app.ui.custom_panels.panels.get(i).map(|c| c.visible)
}

pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if let Some(n) = id.strip_prefix(OPEN_ITEM) {
        return Some(n.parse::<usize>().is_ok_and(|i| i < app.ui.custom_panels.panels.len()));
    }
    IDS.contains(&id).then_some(true)
}

/// Window › Custom Panels › <panel> rows, after New Custom Panel and Import Panel….
pub fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<crate::menus::MenuItem>) {
    // After the submenu's own items (New Custom Panel, Import Panel…).
    let Some(at) = items.iter().rposition(|i| i.id.starts_with(PREFIX) && !i.id.starts_with(OPEN_ITEM)) else { return };
    let Some(path) = items.get(at).map(|i| i.path.clone()) else { return };
    let panels = &app.ui.custom_panels.panels;
    if panels.is_empty() {
        return;
    }
    let row = |id: String, label: String, checked: Option<bool>, enabled: bool| crate::menus::MenuItem {
        id,
        label,
        path: path.clone(),
        shortcut: None,
        enabled,
        checked,
        color: None,
    };
    let rows: Vec<_> = std::iter::once(row("---".into(), "---".into(), None, false))
        .chain(panels.iter().enumerate().map(|(i, c)| row(format!("{OPEN_ITEM}{i}"), c.name.clone(), Some(c.visible), true)))
        .collect();
    items.splice(at + 1..at + 1, rows);
}

/// Run a custom panel command. `None` when `id` isn't ours.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if !handles(id) {
        return None;
    }
    let r = command(app, ctx, id, p);
    if r.is_ok() && !matches!(id, "window.customPanel.run" | "window.customPanel.list" | "window.customPanel.export" | "window.customPanel.exportFile") {
        save(app);
    }
    Some(r)
}

fn command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, p: &Value) -> Result<Value, String> {
    if let Some(n) = id.strip_prefix(OPEN_ITEM) {
        let i: usize = n.parse().map_err(|_| "bad panel index".to_string())?;
        let panel = panel_mut(app, i)?;
        panel.visible = !panel.visible;
        return Ok(json!({"panel": panel.name, "visible": panel.visible}));
    }
    match id {
        "window.customPanel.new" => {
            if app.ui.custom_panels.panels.len() >= MAX_PANELS {
                return Err(format!("you can keep at most {MAX_PANELS} custom panels"));
            }
            let name = unique_name(app, str_param(p, "name").unwrap_or(""), None);
            let uid = next_uid(app);
            let mut panel = CustomPanel { uid, name: name.clone(), ..Default::default() };
            if let Some(c) = p.get("columns").and_then(Value::as_u64) {
                panel.columns = u8::try_from(c).unwrap_or(u8::MAX);
            }
            panel.sanitize();
            app.ui.custom_panels.panels.push(panel);
            // A new panel opens in edit mode unless the caller only wants it made.
            if p.get("edit").and_then(Value::as_bool).unwrap_or(true) {
                begin_edit(app, uid);
            }
            Ok(json!({"panel": name, "index": app.ui.custom_panels.panels.len() - 1}))
        }
        "window.customPanel.show" => {
            let i = find(app, p)?;
            let panel = panel_mut(app, i)?;
            panel.visible = p.get("show").and_then(Value::as_bool).unwrap_or(!panel.visible);
            let (name, visible, uid) = (panel.name.clone(), panel.visible, panel.uid);
            if !visible && app.ui.custom_panels.editing == Some(uid) {
                end_edit(app);
            }
            Ok(json!({"panel": name, "visible": visible}))
        }
        "window.customPanel.edit" => {
            let i = find(app, p)?;
            let uid = panel_mut(app, i)?.uid;
            let on = p.get("on").and_then(Value::as_bool).unwrap_or(app.ui.custom_panels.editing != Some(uid));
            if on {
                begin_edit(app, uid);
            } else if app.ui.custom_panels.editing == Some(uid) {
                end_edit(app);
            }
            Ok(json!({"editing": on}))
        }
        "window.customPanel.rename" => {
            let i = find(app, p)?;
            let wanted = clip(str_param(p, "name").ok_or("`name` is required")?);
            if wanted.is_empty() {
                return Err("a panel needs a name".into());
            }
            if app.ui.custom_panels.panels.iter().enumerate().any(|(j, c)| j != i && c.name.eq_ignore_ascii_case(&wanted)) {
                return Err(format!("there is already a panel named `{wanted}`"));
            }
            panel_mut(app, i)?.name = wanted.clone();
            Ok(json!({"panel": wanted}))
        }
        "window.customPanel.delete" => {
            let i = find(app, p)?;
            let removed = app.ui.custom_panels.panels.remove(i);
            if app.ui.custom_panels.editing == Some(removed.uid) {
                end_edit(app);
            }
            Ok(json!({"deleted": removed.name}))
        }
        "window.customPanel.addItem" => {
            let i = find(app, p)?;
            let item = item_from(p, true, app)?;
            let panel = panel_mut(app, i)?;
            if panel.items.len() >= MAX_ITEMS {
                return Err(format!("a panel holds at most {MAX_ITEMS} items"));
            }
            let at = p.get("at").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(panel.items.len()).min(panel.items.len());
            panel.items.insert(at, item);
            let uid = panel.uid;
            if app.ui.custom_panels.editing == Some(uid) {
                app.ui.custom_panels.selected = Some(at);
            }
            Ok(json!({"index": at}))
        }
        "window.customPanel.updateItem" => {
            let i = find(app, p)?;
            let index = index_param(p, "index")?;
            let panel = panel_mut(app, i)?;
            let accordion = panel.accordion;
            let item = panel.items.get_mut(index).ok_or("no such item")?;
            if let Some(l) = str_param(p, "label") {
                *item.label_mut().ok_or("a divider has no label")? = clip(l);
            }
            if let Some(v) = p.get("span") {
                let n = v.as_u64().and_then(|n| u8::try_from(n).ok()).filter(|n| COLUMNS.contains(n)).ok_or("`span` must be 1 to 6")?;
                match item {
                    PanelItem::Command { span, .. } | PanelItem::Action { span, .. } => *span = n,
                    _ => return Err("only buttons have a width".into()),
                }
            }
            if let Some(v) = p.get("columns") {
                let n = if v.is_null() {
                    None
                } else {
                    Some(v.as_u64().and_then(|n| u8::try_from(n).ok()).filter(|n| COLUMNS.contains(n)).ok_or("`columns` must be 1 to 6 or null")?)
                };
                match item {
                    PanelItem::Heading { columns, .. } | PanelItem::Label { columns, .. } | PanelItem::Separator { columns } => *columns = n,
                    _ => return Err("only headings, labels and dividers set buttons per row".into()),
                }
            }
            if let Some(v) = p.get("icon").and_then(Value::as_bool) {
                match item {
                    PanelItem::Command { id, icon, .. } if crate::tool_shortcuts::tool(id).is_some() => *icon = v,
                    _ => return Err("only tool buttons can show an icon".into()),
                }
            }
            if let Some(c) = str_param(p, "color") {
                let known = known_color(c);
                if known.is_empty() && !c.trim().is_empty() {
                    return Err(format!("unknown colour `{c}`"));
                }
                match item {
                    PanelItem::Command { color, .. } | PanelItem::Action { color, .. } => *color = known,
                    _ => return Err("only buttons have a colour".into()),
                }
            }
            let mut opened = false;
            if let Some(v) = p.get("collapsed").and_then(Value::as_bool) {
                match item {
                    PanelItem::Heading { collapsed, .. } => *collapsed = v,
                    _ => return Err("only headings fold".into()),
                }
                opened = !v;
            }
            if let Some(params) = p.get("params") {
                match item {
                    PanelItem::Command { params: old, .. } if params.is_null() || params.is_object() => *old = params.clone(),
                    PanelItem::Command { .. } => return Err("`params` must be an object".into()),
                    _ => return Err("only command buttons take params".into()),
                }
            }
            let out = serde_json::to_value(&*item).unwrap_or(Value::Null);
            // One section open at a time: opening this heading folds the others.
            if accordion && opened {
                for (k, other) in panel_mut(app, i)?.items.iter_mut().enumerate() {
                    if let PanelItem::Heading { collapsed, .. } = other
                        && k != index
                    {
                        *collapsed = true;
                    }
                }
            }
            Ok(out)
        }
        "window.customPanel.removeItem" => {
            let i = find(app, p)?;
            let index = index_param(p, "index")?;
            let panel = panel_mut(app, i)?;
            if index >= panel.items.len() {
                return Err("no such item".into());
            }
            panel.items.remove(index);
            let left = panel.items.len();
            let ui = &mut app.ui.custom_panels;
            ui.selected = ui.selected.filter(|_| left > 0).map(|s| if s > index { s - 1 } else { s.min(left.saturating_sub(1)) });
            Ok(json!({"items": left}))
        }
        "window.customPanel.moveItem" => {
            let i = find(app, p)?;
            let from = index_param(p, "from")?;
            let to = index_param(p, "to")?;
            let panel = panel_mut(app, i)?;
            if from >= panel.items.len() {
                return Err("no such item".into());
            }
            let to = to.min(panel.items.len() - 1);
            let item = panel.items.remove(from);
            panel.items.insert(to, item);
            if app.ui.custom_panels.selected == Some(from) {
                app.ui.custom_panels.selected = Some(to);
            }
            Ok(json!({"index": to}))
        }
        "window.customPanel.setLayout" => {
            let i = find(app, p)?;
            let panel = panel_mut(app, i)?;
            if let Some(c) = p.get("columns").and_then(Value::as_u64) {
                panel.columns = clamp_columns(u8::try_from(c).unwrap_or(u8::MAX));
            }
            if let Some(w) = p.get("width").and_then(Value::as_f64).filter(|w| w.is_finite()) {
                panel.width = (w as f32).clamp(*WIDTH.start(), *WIDTH.end());
            }
            if let Some(a) = p.get("accordion").and_then(Value::as_bool) {
                panel.accordion = a;
                // Keep at most the first open section open.
                if a {
                    let mut seen_open = false;
                    for item in &mut panel.items {
                        if let PanelItem::Heading { collapsed, .. } = item {
                            *collapsed = seen_open || *collapsed;
                            seen_open |= !*collapsed;
                        }
                    }
                }
            }
            Ok(json!({"columns": panel.columns, "width": panel.width, "accordion": panel.accordion}))
        }
        "window.customPanel.run" => {
            let i = find(app, p)?;
            let index = index_param(p, "index")?;
            let item = app.ui.custom_panels.panels.get(i).and_then(|c| c.items.get(index)).cloned().ok_or("no such item")?;
            run_item(app, ctx, &item)
        }
        "window.customPanel.list" => Ok(json!({"panels": app.ui.custom_panels.panels})),
        "window.customPanel.export" => {
            let i = find(app, p)?;
            let panel = app.ui.custom_panels.panels.get(i).ok_or("no such custom panel")?;
            Ok(export(panel))
        }
        "window.customPanel.import" => {
            if app.ui.custom_panels.panels.len() >= MAX_PANELS {
                return Err(format!("you can keep at most {MAX_PANELS} custom panels"));
            }
            let def = p.get("definition").or_else(|| p.get("panel")).ok_or("`definition` is required (what window.customPanel.export returns)")?;
            let def = def.get("panel").filter(|_| def.get("version").is_some()).unwrap_or(def);
            let mut panel = parse_panel(def).ok_or("the definition isn't a custom panel")?;
            panel.sanitize();
            panel.name = unique_name(app, &panel.name, None);
            panel.uid = next_uid(app);
            panel.visible = true;
            let name = panel.name.clone();
            let items = panel.items.len();
            app.ui.custom_panels.panels.push(panel);
            Ok(json!({"panel": name, "items": items}))
        }
        "window.customPanel.importFile" => app.pick_file_bytes_filtered(&["json"], |app, _name, bytes| import_bytes(app, &bytes)),
        "window.customPanel.exportFile" => {
            let i = find(app, p)?;
            let panel = app.ui.custom_panels.panels.get(i).ok_or("no such custom panel")?;
            let text = serde_json::to_string_pretty(&export(panel)).map_err(|e| e.to_string())?;
            let file = format!("{}.json", file_stem(&panel.name));
            app.pick_save(&file, move |app, path| {
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, text.as_bytes())?;
                Ok(json!({"path": path}))
            })
        }
        _ => Err(format!("unknown custom panel command {id}")),
    }
}

/// Add a panel from an exported `.json` file's bytes (Import Panel…). The result is saved.
pub fn import_bytes(app: &mut PhotocraftApp, bytes: &[u8]) -> Result<Value, String> {
    // Exports are small; anything this large isn't one.
    if bytes.len() > 4 << 20 {
        return Err("the file is too large to be a custom panel".into());
    }
    let def: Value = serde_json::from_slice(bytes).map_err(|e| format!("not a custom panel file: {e}"))?;
    let r = command(app, &egui::Context::default(), "window.customPanel.import", &json!({"definition": def}));
    if r.is_ok() {
        save(app);
    }
    r
}

/// A panel name made safe as a file name.
fn file_stem(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '-' }).collect();
    let stem = stem.trim();
    if stem.is_empty() { "Custom Panel".into() } else { stem.to_string() }
}

/// A panel as a shareable definition (no window state).
pub fn export(panel: &CustomPanel) -> Value {
    json!({
        "version": EXPORT_VERSION,
        "panel": {"name": panel.name, "columns": panel.columns, "width": panel.width, "accordion": panel.accordion, "items": panel.items},
    })
}

fn begin_edit(app: &mut PhotocraftApp, uid: u64) {
    let ui = &mut app.ui.custom_panels;
    if ui.editing != Some(uid) {
        ui.selected = None;
        ui.picker = None;
        ui.name_draft = None;
    }
    ui.editing = Some(uid);
    if let Some(c) = ui.panels.iter_mut().find(|c| c.uid == uid) {
        c.visible = true;
    }
}

fn end_edit(app: &mut PhotocraftApp) {
    let ui = &mut app.ui.custom_panels;
    ui.editing = None;
    ui.selected = None;
    ui.picker = None;
    ui.name_draft = None;
}

/// Press a button: run its command or play its action. Failures land in the status bar too.
pub fn run_item(app: &mut PhotocraftApp, ctx: &egui::Context, item: &PanelItem) -> Result<Value, String> {
    if app.ui.custom_panels.running {
        return Err("a panel button is already running".into());
    }
    app.ui.custom_panels.running = true;
    let r = match item {
        PanelItem::Command { id, params, .. } => {
            if !can_add(id) {
                Err(format!("`{id}` is not available in this version of PhotoCraft"))
            } else if !crate::menus::is_enabled(app, id) {
                Err(crate::i18n::fmt(tl!("{name} isn't available right now"), &[("name", &item.display_label())]))
            } else {
                let params = if params.is_null() { json!({}) } else { params.clone() };
                crate::menus::invoke(app, ctx, id, params)
            }
        }
        PanelItem::Action { action, set, .. } => {
            let mut p = json!({"action": action});
            if let (Some(set), Some(o)) = (set, p.as_object_mut()) {
                o.insert("set".into(), json!(set));
            }
            let r = app.run("actions.play", p);
            if let Ok(v) = &r {
                crate::actions::report_play(app, v);
            }
            r
        }
        PanelItem::Heading { .. } | PanelItem::Label { .. } | PanelItem::Separator { .. } => Err("headings, labels and dividers aren't buttons".into()),
    };
    app.ui.custom_panels.running = false;
    if let Err(e) = &r {
        app.ui.status = e.clone();
        app.ui.status_error = true;
    }
    r
}

// ------------------------------------------------------------------ Add Button search

/// A search hit: what it shows, where it comes from, and the item it adds.
struct Hit {
    label: String,
    hint: String,
    item: PanelItem,
}

const HITS: usize = 8;

fn picker_hits(app: &PhotocraftApp, query: &str) -> Vec<Hit> {
    let q = query.trim();
    let mut hits: Vec<(i32, Hit)> = Vec::new();
    // Actions first on an empty search: playing them is what most panels are for.
    for a in &app.session.actions.list {
        let set = a.set.clone().unwrap_or_default();
        if let Some(s) = crate::palette::fuzzy_score(q, &format!("{} {set}", a.name)) {
            let hint = if set.is_empty() { tl!("Action").to_string() } else { format!("{} › {set}", tl!("Action")) };
            hits.push((
                s + 3,
                Hit {
                    label: a.name.clone(),
                    hint,
                    item: PanelItem::Action { action: a.name.clone(), set: a.set.clone(), label: String::new(), color: a.color.clone(), span: 1 },
                },
            ));
        }
    }
    if !q.is_empty() {
        let lang = crate::i18n::current();
        for m in crate::menus::menu_items(app) {
            if m.label == "---" || !can_add(&m.id) {
                continue;
            }
            let label = crate::i18n::tr_id(lang, &m.id, &m.label).trim_end_matches('…').to_string();
            let path = m.path.iter().map(|s| crate::i18n::tr(lang, s)).collect::<Vec<_>>().join(" › ");
            if let Some(s) = crate::palette::fuzzy_score(q, &format!("{label} {path} {}", m.label)) {
                hits.push((
                    s,
                    Hit {
                        label,
                        hint: path,
                        item: PanelItem::Command { id: m.id, params: Value::Null, label: String::new(), color: String::new(), span: 1, icon: false },
                    },
                ));
            }
        }
        for tool in crate::state::Tool::ALL {
            let label = tl!(tool.label()).to_string();
            if let Some(s) = crate::palette::fuzzy_score(q, &format!("{label} {}", tool.label())) {
                let id = crate::tool_shortcuts::id(tool);
                hits.push((
                    s + 1,
                    Hit {
                        label,
                        hint: tl!("Tool").to_string(),
                        item: PanelItem::Command { id, params: Value::Null, label: String::new(), color: String::new(), span: 1, icon: false },
                    },
                ));
            }
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    let mut seen = std::collections::HashSet::new();
    hits.into_iter().map(|(_, h)| h).filter(|h| seen.insert(format!("{:?}", h.item))).take(HITS).collect()
}

// ------------------------------------------------------------------ drawing

/// What the user did in a panel this frame, applied once the window is drawn.
enum Intent {
    Run(usize),
    Close,
    Edit(bool),
    Select(Option<usize>),
    Fold(usize),
    Rename(String),
    Label(usize, String),
    Color(usize, &'static str),
    Span(usize, u8),
    ItemColumns(usize, Option<u8>),
    Icon(usize, bool),
    Move(usize, usize),
    Remove(usize),
    Add(PanelItem),
    Picker(Option<String>),
    Columns(u8),
    Accordion(bool),
    Delete,
    Export,
}

/// Height of a button and of a section bar.
const ROW_H: f32 = 24.0;
/// Space between buttons.
const GAP: f32 = 4.0;

/// Fill and text colour of a button: the Actions panel's Button Mode colours, and the field
/// colour for an uncoloured button.
fn button_colors(color: &str, t: &Tokens) -> (Color32, Color32) {
    if color.is_empty() { (t.field, t.text) } else { (crate::actions::button_fill(color, t), crate::actions::button_label_color(color, t)) }
}

/// A button colour's name, as the Actions panel's Color menu shows it.
fn color_name(c: &str) -> &'static str {
    match c {
        "red" => tl!("Red"),
        "orange" => tl!("Orange"),
        "yellow" => tl!("Yellow"),
        "green" => tl!("Green"),
        "blue" => tl!("Blue"),
        "violet" => tl!("Violet"),
        "gray" => tl!("Gray"),
        _ => tl!("None"),
    }
}

/// Draws every visible custom panel.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let shown: Vec<usize> = app.ui.custom_panels.panels.iter().enumerate().filter(|(_, c)| c.visible).map(|(i, _)| i).collect();
    for i in shown {
        draw(app, ctx, i);
    }
}

fn draw(app: &mut PhotocraftApp, ctx: &egui::Context, index: usize) {
    let Some(panel) = app.ui.custom_panels.panels.get(index).cloned() else { return };
    let t = Tokens::get(ctx);
    let editing = app.ui.custom_panels.editing == Some(panel.uid);
    let selected = app.ui.custom_panels.selected.filter(|_| editing);
    let enabled: Vec<bool> = panel
        .items
        .iter()
        .map(|it| match it {
            PanelItem::Command { id, .. } => can_add(id) && crate::menus::is_enabled(app, id),
            PanelItem::Action { action, .. } => app.session.actions.list.iter().any(|a| &a.name == action),
            _ => true,
        })
        .collect();
    let picker = app.ui.custom_panels.picker.clone().filter(|_| editing);
    let hits = picker.as_deref().map(|q| picker_hits(app, q)).unwrap_or_default();
    let mut name_draft = app.ui.custom_panels.name_draft.clone().unwrap_or_else(|| panel.name.clone());
    let mut intents: Vec<Intent> = Vec::new();
    let offset = vec2(-280.0 - 24.0 * (index % 6) as f32, 90.0 + 28.0 * (index % 6) as f32);
    let window_id = format!("custom-panel-{}", panel.uid);
    crate::analysis_ui::panel_window(app, ctx, &window_id, &panel.name, offset, panel.width, |ui| {
        title_row(ui, &panel, editing, &mut name_draft, &mut intents, &t);
        let body = items(ui, &panel, editing, selected, &enabled, &mut intents, &t);
        if !editing {
            body.context_menu(|ui| {
                if ui.button(tl!("Edit Panel")).clicked() {
                    intents.push(Intent::Edit(true));
                    ui.close();
                }
                if ui.button(tl!("Close Panel")).clicked() {
                    intents.push(Intent::Close);
                    ui.close();
                }
            });
        }
        if editing {
            editor(ui, &panel, selected, picker.as_deref(), &hits, &mut intents, &t);
        }
    });
    if editing && app.ui.custom_panels.editing == Some(panel.uid) {
        app.ui.custom_panels.name_draft = Some(name_draft);
    }
    apply(app, ctx, index, &panel, intents);
}

fn title_row(ui: &mut egui::Ui, panel: &CustomPanel, editing: bool, name_draft: &mut String, intents: &mut Vec<Intent>, t: &Tokens) {
    ui.horizontal(|ui| {
        if editing {
            let resp = ui.add(egui::TextEdit::singleline(name_draft).char_limit(MAX_TEXT).desired_width(ui.available_width() - 56.0).hint_text(tl!("Name")));
            if resp.lost_focus() && name_draft.trim() != panel.name {
                intents.push(Intent::Rename(name_draft.clone()));
            }
        } else {
            ui.label(RichText::new(&panel.name).color(t.text).size(12.0).strong());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                intents.push(Intent::Close);
            }
            let tip = if editing { tl!("Done") } else { tl!("Edit Panel") };
            if crate::icons::button(ui, "pencil", 20.0, editing, tip).clicked() {
                intents.push(Intent::Edit(!editing));
            }
        });
    });
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
}

/// One row of buttons: `(item index, columns it spans)`, laid out on a `cols`-column grid.
fn button_row(
    ui: &mut egui::Ui,
    panel: &CustomPanel,
    row: &[(usize, u8)],
    (cols, width): (u8, f32),
    (editing, selected, enabled): (bool, Option<usize>, &[bool]),
    intents: &mut Vec<Intent>,
    t: &Tokens,
) {
    if row.is_empty() {
        return;
    }
    let cols_f = f32::from(cols.max(1));
    let cell = ((width - GAP * (cols_f - 1.0)) / cols_f).max(12.0);
    ui.horizontal(|ui| {
        for &(i, span) in row {
            let Some(item) = panel.items.get(i) else { continue };
            let span_f = f32::from(span.max(1));
            let w = cell * span_f + GAP * (span_f - 1.0);
            let (fill, text) = button_colors(item.color(), t);
            let label = item.display_label();
            let tool_icon = match item {
                PanelItem::Command { id, icon: true, .. } => crate::tool_shortcuts::tool(id).map(crate::icons::tool_icon),
                _ => None,
            };
            let on = enabled.get(i).copied().unwrap_or(false);
            let caption = if tool_icon.is_some() { String::new() } else { label.clone() };
            let size = fit_size(ui, &caption, w - 2.0 * ui.spacing().button_padding.x - 6.0);
            let mut b = egui::Button::new(RichText::new(caption).color(text).size(size)).fill(fill).truncate();
            if selected == Some(i) {
                b = b.stroke(Stroke::new(2.0, t.accent));
            }
            let resp = ui.add_enabled_ui(on || editing, |ui| ui.add_sized(vec2(w, ROW_H), b)).inner;
            if let Some(icon) = tool_icon {
                let tint = if on || editing { t.text } else { t.text_dim };
                crate::icons::paint(ui, resp.rect, icon, 15.0, tint);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on || editing, &label));
            }
            let resp = match item {
                PanelItem::Action { action, .. } => resp.on_hover_text(format!("{}: {action}", tl!("Play"))),
                _ => resp.on_hover_text(&label),
            };
            if resp.clicked() {
                intents.push(if editing { Intent::Select(Some(i)) } else { Intent::Run(i) });
            }
        }
    });
}

/// Button text size: 11 pt, shrunk down to 8.5 pt so a long name fits its button before it is
/// cut short.
fn fit_size(ui: &egui::Ui, text: &str, room: f32) -> f32 {
    const FULL: f32 = 11.0;
    const SMALLEST: f32 = 8.5;
    if text.is_empty() || room <= 0.0 {
        return FULL;
    }
    // Measure in the font buttons use, at the full size.
    let family = egui::TextStyle::Button.resolve(ui.style()).family;
    let width = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), egui::FontId::new(FULL, family), Color32::WHITE).size().x);
    if width <= room || !width.is_finite() { FULL } else { (FULL * room / width).clamp(SMALLEST, FULL) }
}

/// A section title: a full-width bar when one section opens at a time, else a fold arrow and
/// the title.
fn heading(ui: &mut egui::Ui, panel: &CustomPanel, i: usize, collapsed: bool, selected: bool, t: &Tokens) -> egui::Response {
    let label = panel.items.get(i).map(PanelItem::display_label).unwrap_or_default();
    let color = if selected { t.accent } else { t.text };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if panel.accordion { ROW_H } else { 20.0 }), Sense::click());
    if panel.accordion {
        let fill = if resp.hovered() || !collapsed { t.hover } else { t.field };
        let stroke = if selected { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.card_border) };
        ui.painter().rect(rect, t.radius_sm, fill, stroke, egui::StrokeKind::Inside);
        ui.painter().text(rect.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, &label, egui::FontId::proportional(11.5), color);
    } else {
        let chevron = if collapsed { "chevron-right" } else { "chevron-down" };
        crate::icons::paint(ui, egui::Rect::from_min_size(rect.min, vec2(16.0, rect.height())), chevron, 12.0, t.text_dim);
        ui.painter().text(rect.left_center() + vec2(18.0, 0.0), egui::Align2::LEFT_CENTER, &label, egui::FontId::proportional(11.5), color);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    resp
}

/// The buttons, headings, labels and dividers. Returns the area's response for its context menu.
fn items(
    ui: &mut egui::Ui,
    panel: &CustomPanel,
    editing: bool,
    selected: Option<usize>,
    enabled: &[bool],
    intents: &mut Vec<Intent>,
    t: &Tokens,
) -> egui::Response {
    let width = ui.available_width();
    let state = (editing, selected, enabled);
    let inner = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        // In edit mode the Add buttons right below say what to do.
        if panel.items.is_empty() && !editing {
            ui.label(RichText::new(tl!("This panel is empty. Click the pencil to add buttons.")).color(t.text_dim).size(11.0));
        }
        let mut cols = panel.columns.max(1);
        let mut folded = false;
        let mut row: Vec<(usize, u8)> = Vec::new();
        let mut used: u8 = 0;
        for (i, item) in panel.items.iter().enumerate() {
            let is_selected = selected == Some(i);
            match item {
                PanelItem::Heading { collapsed, columns, .. } => {
                    button_row(ui, panel, &row, (cols, width), state, intents, t);
                    (row, used) = (Vec::new(), 0);
                    cols = columns.unwrap_or(panel.columns).max(1);
                    folded = *collapsed && !editing;
                    if heading(ui, panel, i, *collapsed, is_selected, t).clicked() {
                        intents.push(if editing { Intent::Select(Some(i)) } else { Intent::Fold(i) });
                    }
                }
                PanelItem::Label { columns, .. } | PanelItem::Separator { columns } => {
                    button_row(ui, panel, &row, (cols, width), state, intents, t);
                    (row, used) = (Vec::new(), 0);
                    if let Some(c) = columns {
                        cols = (*c).max(1);
                    }
                    if folded {
                        continue;
                    }
                    let is_label = matches!(item, PanelItem::Label { .. });
                    let color = if is_selected {
                        t.accent
                    } else if is_label {
                        t.text
                    } else {
                        t.text_dim
                    };
                    if is_label {
                        let (rect, resp) = ui.allocate_exact_size(vec2(width, 16.0), Sense::click());
                        let text = item.display_label();
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &text, egui::FontId::proportional(11.0), color);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
                        if editing && resp.clicked() {
                            intents.push(Intent::Select(Some(i)));
                        }
                    } else if editing {
                        let resp = ui.add(egui::Label::new(RichText::new(format!("—— {} ——", tl!("Divider"))).color(color).size(10.0)).sense(Sense::click()));
                        if resp.clicked() {
                            intents.push(Intent::Select(Some(i)));
                        }
                    } else {
                        crate::widgets::hairline(ui);
                    }
                }
                _ if folded => {}
                _ => {
                    let span = item.span().min(cols);
                    if used.saturating_add(span) > cols {
                        button_row(ui, panel, &row, (cols, width), state, intents, t);
                        (row, used) = (Vec::new(), 0);
                    }
                    row.push((i, span));
                    used = used.saturating_add(span);
                    if used >= cols {
                        button_row(ui, panel, &row, (cols, width), state, intents, t);
                        (row, used) = (Vec::new(), 0);
                    }
                }
            }
        }
        button_row(ui, panel, &row, (cols, width), state, intents, t);
    });
    inner.response.interact(Sense::click())
}

/// A − n + stepper; returns the new value when one of its buttons was clicked.
fn stepper(ui: &mut egui::Ui, label: &str, value: Option<u8>, t: &Tokens) -> Option<Option<u8>> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(t.text_dim).size(11.0));
        let n = value.unwrap_or(0);
        if ui.add_enabled(n > *COLUMNS.start(), egui::Button::new("−").small()).clicked() {
            out = Some(Some(n - 1));
        }
        let shown = value.map_or_else(|| tl!("Auto").to_string(), |v| v.to_string());
        ui.label(RichText::new(shown).color(t.text).size(11.0));
        if ui.add_enabled(n < *COLUMNS.end(), egui::Button::new("+").small()).clicked() {
            out = Some(Some(if value.is_none() { *COLUMNS.start() } else { n + 1 }));
        }
    });
    out
}

/// Edit mode: the selected item's settings and the add / layout / delete controls.
fn editor(ui: &mut egui::Ui, panel: &CustomPanel, selected: Option<usize>, picker: Option<&str>, hits: &[Hit], intents: &mut Vec<Intent>, t: &Tokens) {
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
    if let Some((i, item)) = selected.and_then(|i| panel.items.get(i).map(|it| (i, it))) {
        let mut copy = item.clone();
        if let Some(label) = copy.label_mut() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Label")).color(t.text_dim).size(11.0));
                let hint = item.display_label();
                if ui.add(egui::TextEdit::singleline(label).char_limit(MAX_TEXT).hint_text(hint).desired_width(ui.available_width())).changed() {
                    intents.push(Intent::Label(i, label.clone()));
                }
            });
        }
        if item.is_button() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Color")).color(t.text_dim).size(11.0));
                for &c in photocraft_engine::actions_cmds::BUTTON_COLORS.iter() {
                    let (fill, _) = button_colors(c, t);
                    let tip = color_name(c);
                    let stroke = if item.color() == c { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.card_border) };
                    if ui.add(egui::Button::new("").fill(fill).stroke(stroke).min_size(vec2(16.0, 16.0))).on_hover_text(tip).clicked() {
                        intents.push(Intent::Color(i, c));
                    }
                }
            });
            if let Some(Some(n)) = stepper(ui, tl!("Width"), Some(item.span()), t) {
                intents.push(Intent::Span(i, n));
            }
            if let PanelItem::Command { id, icon, .. } = item
                && crate::tool_shortcuts::tool(id).is_some()
            {
                let mut on = *icon;
                if ui.checkbox(&mut on, tl!("Show as Icon")).changed() {
                    intents.push(Intent::Icon(i, on));
                }
            }
        } else if let Some(c) = stepper(ui, tl!("Columns"), item.columns(), t) {
            intents.push(Intent::ItemColumns(i, c));
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(i > 0, egui::Button::new(tl!("Move Up")).small()).clicked() {
                intents.push(Intent::Move(i, i - 1));
            }
            if ui.add_enabled(i + 1 < panel.items.len(), egui::Button::new(tl!("Move Down")).small()).clicked() {
                intents.push(Intent::Move(i, i + 1));
            }
            if ui.add(egui::Button::new(tl!("Remove")).small()).clicked() {
                intents.push(Intent::Remove(i));
            }
        });
        ui.add_space(4.0);
    } else if !panel.items.is_empty() {
        ui.label(RichText::new(tl!("Click an item to change or move it.")).color(t.text_dim).size(10.5));
    }
    match picker {
        Some(q) => {
            let mut query = q.to_string();
            let resp = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tl!("Search commands, tools and actions…")).desired_width(ui.available_width()));
            if !resp.has_focus() && q.is_empty() {
                resp.request_focus();
            }
            if resp.changed() {
                intents.push(Intent::Picker(Some(query)));
            }
            if hits.is_empty() && !q.trim().is_empty() {
                ui.label(RichText::new(tl!("No matches")).color(t.text_dim).size(10.5));
            }
            for h in hits {
                let resp = ui.add(egui::Button::new(RichText::new(format!("{}   {}", h.label, h.hint)).size(11.0)).frame(false).truncate());
                if resp.clicked() {
                    intents.push(Intent::Add(h.item.clone()));
                }
            }
            if ui.add(egui::Button::new(tl!("Cancel")).small()).clicked() {
                intents.push(Intent::Picker(None));
            }
        }
        None => {
            ui.horizontal_wrapped(|ui| {
                if ui.add(egui::Button::new(tl!("Add Button…")).small()).clicked() {
                    intents.push(Intent::Picker(Some(String::new())));
                }
                if ui.add(egui::Button::new(tl!("Add Heading")).small()).clicked() {
                    intents.push(Intent::Add(PanelItem::Heading { label: tl!("Section").to_string(), collapsed: false, columns: None }));
                }
                if ui.add(egui::Button::new(tl!("Add Text")).small()).clicked() {
                    intents.push(Intent::Add(PanelItem::Label { label: tl!("Section").to_string(), columns: None }));
                }
                if ui.add(egui::Button::new(tl!("Add Divider")).small()).clicked() {
                    intents.push(Intent::Add(PanelItem::Separator { columns: None }));
                }
            });
        }
    }
    ui.horizontal(|ui| {
        if let Some(Some(c)) = stepper(ui, tl!("Columns"), Some(panel.columns), t) {
            intents.push(Intent::Columns(c));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(tl!("Export Panel…")).small()).clicked() {
                intents.push(Intent::Export);
            }
        });
    });
    let mut accordion = panel.accordion;
    if ui.checkbox(&mut accordion, tl!("One section open at a time")).changed() {
        intents.push(Intent::Accordion(accordion));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if crate::widgets::secondary_button(ui, tl!("Delete Panel"), 0.0).clicked() {
            intents.push(Intent::Delete);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::primary_button(ui, tl!("Done"), 0.0).clicked() {
                intents.push(Intent::Edit(false));
            }
        });
    });
}

/// Apply what the user did in panel `index` through the same commands automation uses.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, index: usize, panel: &CustomPanel, intents: Vec<Intent>) {
    let at = json!(index);
    let item = |i: usize, key: &str, v: Value| Map::from_iter([("index".to_string(), json!(i)), (key.to_string(), v)]);
    for intent in intents {
        let (id, mut p): (&str, Map<String, Value>) = match intent {
            Intent::Run(i) => {
                // `run_item` reports its own failure in the status bar.
                if let Some(item) = panel.items.get(i) {
                    let _ = run_item(app, ctx, item);
                }
                continue;
            }
            Intent::Close => ("window.customPanel.show", Map::from_iter([("show".into(), json!(false))])),
            Intent::Edit(on) => {
                if !on {
                    // Commit a name still being typed.
                    if let Some(name) = app.ui.custom_panels.name_draft.clone().filter(|n| n.trim() != panel.name) {
                        let r = menu(app, ctx, "window.customPanel.rename", &json!({"panel": index, "name": name}));
                        report(app, r);
                    }
                }
                ("window.customPanel.edit", Map::from_iter([("on".into(), json!(on))]))
            }
            Intent::Select(s) => {
                app.ui.custom_panels.selected = s;
                continue;
            }
            Intent::Fold(i) => {
                let collapsed = matches!(panel.items.get(i), Some(PanelItem::Heading { collapsed: true, .. }));
                ("window.customPanel.updateItem", item(i, "collapsed", json!(!collapsed)))
            }
            Intent::Rename(name) => ("window.customPanel.rename", Map::from_iter([("name".into(), json!(name))])),
            Intent::Label(i, label) => ("window.customPanel.updateItem", item(i, "label", json!(label))),
            Intent::Color(i, c) => ("window.customPanel.updateItem", item(i, "color", json!(c))),
            Intent::Span(i, n) => ("window.customPanel.updateItem", item(i, "span", json!(n))),
            Intent::ItemColumns(i, c) => ("window.customPanel.updateItem", item(i, "columns", json!(c))),
            Intent::Icon(i, on) => ("window.customPanel.updateItem", item(i, "icon", json!(on))),
            Intent::Move(from, to) => ("window.customPanel.moveItem", Map::from_iter([("from".into(), json!(from)), ("to".into(), json!(to))])),
            Intent::Remove(i) => ("window.customPanel.removeItem", Map::from_iter([("index".into(), json!(i))])),
            Intent::Add(new) => {
                let mut m = serde_json::to_value(&new).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
                // Insert after the selection, as Photoshop inserts after the selected action.
                if let Some(s) = app.ui.custom_panels.selected {
                    m.insert("at".into(), json!(s + 1));
                }
                if new.is_button() {
                    app.ui.custom_panels.picker = None;
                }
                ("window.customPanel.addItem", m)
            }
            Intent::Picker(q) => {
                app.ui.custom_panels.picker = q;
                continue;
            }
            Intent::Columns(c) => ("window.customPanel.setLayout", Map::from_iter([("columns".into(), json!(c))])),
            Intent::Accordion(on) => ("window.customPanel.setLayout", Map::from_iter([("accordion".into(), json!(on))])),
            Intent::Delete => ("window.customPanel.delete", Map::new()),
            Intent::Export => ("window.customPanel.exportFile", Map::new()),
        };
        p.insert("panel".into(), at.clone());
        let r = menu(app, ctx, id, &Value::Object(p));
        report(app, r);
    }
    if app.ui.custom_panels.editing.is_none() {
        app.ui.custom_panels.name_draft = None;
    }
}

fn report(app: &mut PhotocraftApp, r: Option<Result<Value, String>>) {
    if let Some(Err(e)) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (PhotocraftApp, egui::Context) {
        (PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()), egui::Context::default())
    }

    fn inv(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, p: Value) -> Result<Value, String> {
        crate::menus::invoke(app, ctx, id, p)
    }

    #[test]
    fn new_panel_appears_in_the_window_menu_and_opens_in_edit_mode() {
        let (mut app, ctx) = app();
        assert!(crate::menus::is_live("window.customPanel.new"));
        let r = inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "Retouch"})).unwrap();
        assert_eq!(r["panel"], "Retouch");
        let uid = app.ui.custom_panels.panels[0].uid;
        assert_eq!(app.ui.custom_panels.editing, Some(uid));
        let items = crate::menus::menu_items(&app);
        let row = items.iter().find(|i| i.id == "window.customPanel.open.0").expect("panel row");
        assert_eq!(row.label, "Retouch");
        assert_eq!(row.checked, Some(true));
        assert_eq!(row.path, ["Window", "Custom Panels"]);
        // The row toggles the panel.
        inv(&mut app, &ctx, "window.customPanel.open.0", json!({})).unwrap();
        assert!(!app.ui.custom_panels.panels[0].visible);
        // A second panel with the same name gets a unique one.
        let r = inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "retouch"})).unwrap();
        assert_eq!(r["panel"], "retouch 2");
    }

    #[test]
    fn buttons_run_commands_tools_and_actions() {
        let (mut app, ctx) = app();
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.session.actions.list.push(photocraft_engine::actions_cmds::Action::new("Add Layer", vec![("layer.new.layer".into(), json!({}))]));
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "P", "edit": false})).unwrap();
        let add = |app: &mut PhotocraftApp, p: Value| {
            let mut p = p;
            p["panel"] = json!("P");
            inv(app, &ctx, "window.customPanel.addItem", p)
        };
        add(&mut app, json!({"kind": "heading", "label": "Basics"})).unwrap();
        add(&mut app, json!({"id": "layer.new.layer", "params": {"name": "From Panel"}, "label": "New Layer", "color": "blue"})).unwrap();
        add(&mut app, json!({"kind": "action", "action": "Add Layer"})).unwrap();
        add(&mut app, json!({"id": crate::tool_shortcuts::id(crate::state::Tool::Eraser)})).unwrap();
        let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.len();
        let before = layers(&app);
        inv(&mut app, &ctx, "window.customPanel.run", json!({"panel": "P", "index": 1})).unwrap();
        assert_eq!(layers(&app), before + 1, "the command button ran");
        inv(&mut app, &ctx, "window.customPanel.run", json!({"panel": "P", "index": 2})).unwrap();
        assert_eq!(layers(&app), before + 2, "the action button played");
        inv(&mut app, &ctx, "window.customPanel.run", json!({"panel": "P", "index": 3})).unwrap();
        assert_eq!(app.ui.tool, crate::state::Tool::Eraser, "the tool button selected the tool");
        assert!(inv(&mut app, &ctx, "window.customPanel.run", json!({"panel": "P", "index": 0})).is_err(), "a heading isn't a button");
        // Labels: their own, else the command's name.
        let p = &app.ui.custom_panels.panels[0];
        assert_eq!(p.items[1].display_label(), "New Layer");
        assert_eq!(p.items[2].display_label(), "Add Layer");
        assert_eq!(p.items[3].display_label(), "Eraser");
    }

    #[test]
    fn bad_params_are_errors_not_crashes() {
        let (mut app, ctx) = app();
        for id in IDS {
            for p in [json!(null), json!({}), json!({"panel": 99}), json!({"panel": "nope", "index": -1}), json!({"panel": [], "name": 5, "from": "a"})] {
                let r = inv(&mut app, &ctx, id, p);
                if !matches!(id, "window.customPanel.new" | "window.customPanel.list") {
                    assert!(r.is_err(), "{id} accepted junk");
                }
            }
        }
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "P"})).unwrap();
        let p = |v: Value| {
            let mut v = v;
            v["panel"] = json!("P");
            v
        };
        for bad in [
            json!({"id": "no.such.command"}),
            json!({"id": "window.customPanel.new"}),
            json!({"kind": "action", "action": "missing"}),
            json!({"kind": "bogus"}),
            json!({"id": "layer.new.layer", "params": 5}),
        ] {
            assert!(inv(&mut app, &ctx, "window.customPanel.addItem", p(bad.clone())).is_err(), "{bad}");
        }
        assert!(inv(&mut app, &ctx, "window.customPanel.updateItem", p(json!({"index": 0, "label": "x"}))).is_err());
        assert!(inv(&mut app, &ctx, "window.customPanel.moveItem", p(json!({"from": 0, "to": 3}))).is_err());
        assert!(inv(&mut app, &ctx, "window.customPanel.rename", p(json!({"name": "   "}))).is_err());
        assert!(inv(&mut app, &ctx, "window.customPanel.import", json!({"definition": "nope"})).is_err());
        assert!(inv(&mut app, &ctx, "window.customPanel.open.7", json!({})).is_err());
        // Extreme layout values are clamped.
        let r = inv(&mut app, &ctx, "window.customPanel.setLayout", p(json!({"columns": 999, "width": 1e30}))).unwrap();
        assert_eq!(r["columns"], 6);
        assert_eq!(r["width"], json!(*WIDTH.end()));
    }

    #[test]
    fn editing_moves_recolours_and_removes_items() {
        let (mut app, ctx) = app();
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "P"})).unwrap();
        for id in ["layer.new.layer", "edit.undo", "edit.redo"] {
            inv(&mut app, &ctx, "window.customPanel.addItem", json!({"panel": "P", "id": id})).unwrap();
        }
        inv(&mut app, &ctx, "window.customPanel.moveItem", json!({"panel": "P", "from": 2, "to": 0})).unwrap();
        inv(&mut app, &ctx, "window.customPanel.updateItem", json!({"panel": "P", "index": 0, "color": "green", "label": "Redo it"})).unwrap();
        assert!(inv(&mut app, &ctx, "window.customPanel.updateItem", json!({"panel": "P", "index": 0, "color": "plaid"})).is_err());
        inv(&mut app, &ctx, "window.customPanel.removeItem", json!({"panel": "P", "index": 1})).unwrap();
        let items = &app.ui.custom_panels.panels[0].items;
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            PanelItem::Command { id: "edit.redo".into(), params: Value::Null, label: "Redo it".into(), color: "green".into(), span: 1, icon: false }
        );
        assert!(matches!(&items[1], PanelItem::Command { id, .. } if id == "edit.undo"));
    }

    #[test]
    fn panels_persist_and_bad_saved_data_is_dropped() {
        let (mut app, ctx) = app();
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "Keep", "columns": 2})).unwrap();
        inv(&mut app, &ctx, "window.customPanel.addItem", json!({"panel": "Keep", "id": "edit.undo", "color": "red"})).unwrap();
        inv(&mut app, &ctx, "window.customPanel.addItem", json!({"panel": "Keep", "kind": "separator"})).unwrap();
        let json_text = app.session.prefs_to_json();
        let mut s2 = photocraft_engine::Session::new();
        s2.load_prefs_json(&json_text).unwrap();
        let mut app2 = PhotocraftApp::new(s2, crate::Services::default());
        restore(&mut app2);
        assert_eq!(app2.ui.custom_panels.panels, app.ui.custom_panels.panels);
        // Hand-edited junk: a bad panel and a bad item are dropped, the rest survive.
        app2.session.prefs.edit(|p| {
            p.custom_panels = json!([
                "not a panel",
                {"name": "Ok", "columns": 99, "width": -5, "items": [{"kind": "command", "id": "edit.undo"}, {"kind": "??"}, 7, {"kind": "heading", "label": "H"}]},
                {"name": "Ok", "uid": 1, "items": []},
                {"name": "No items list"},
            ])
        });
        restore(&mut app2);
        let panels = &app2.ui.custom_panels.panels;
        assert_eq!(panels.len(), 2);
        assert_eq!(panels[0].items.len(), 2);
        assert_eq!(panels[0].columns, 6);
        assert_eq!(panels[0].width, *WIDTH.start());
        assert_ne!(panels[0].name, panels[1].name, "names are made unique");
        assert_ne!(panels[0].uid, panels[1].uid, "uids are made unique");
    }

    #[test]
    fn export_and_import_round_trip() {
        let (mut app, ctx) = app();
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "Share"})).unwrap();
        inv(
            &mut app,
            &ctx,
            "window.customPanel.addItem",
            json!({"panel": "Share", "id": "filter.blur.gaussianBlur", "params": {"radius": 3}, "label": "Blur 3"}),
        )
        .unwrap();
        let def = inv(&mut app, &ctx, "window.customPanel.export", json!({"panel": "Share"})).unwrap();
        assert_eq!(def["version"], 1);
        let r = inv(&mut app, &ctx, "window.customPanel.import", json!({"definition": def})).unwrap();
        assert_eq!(r["panel"], "Share 2");
        let (a, b) = (&app.ui.custom_panels.panels[0], &app.ui.custom_panels.panels[1]);
        assert_eq!(a.items, b.items);
        assert_ne!(a.uid, b.uid);
    }

    #[test]
    fn import_file_bytes_adds_a_panel_and_rejects_junk() {
        let (mut app, ctx) = app();
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "A/B: Share?"})).unwrap();
        inv(&mut app, &ctx, "window.customPanel.addItem", json!({"panel": 0, "kind": "heading", "label": "H"})).unwrap();
        let def = inv(&mut app, &ctx, "window.customPanel.export", json!({"panel": 0})).unwrap();
        let bytes = serde_json::to_vec_pretty(&def).unwrap();
        let r = import_bytes(&mut app, &bytes).unwrap();
        assert_eq!(r["items"], 1);
        assert_eq!(app.ui.custom_panels.panels.len(), 2);
        assert_eq!(app.session.prefs().custom_panels.as_array().map(Vec::len), Some(2), "the import is saved");
        for junk in [&b"not json"[..], b"[]", b"{\"version\": 1}", b"\xff\xfe"] {
            assert!(import_bytes(&mut app, junk).is_err());
        }
        assert_eq!(file_stem("A/B: Share?"), "A-B- Share-");
        assert_eq!(file_stem("///"), "---");
        assert_eq!(file_stem("  "), "Custom Panel");
    }

    #[test]
    fn accordion_opens_one_section_at_a_time() {
        let (mut app, ctx) = app();
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "A", "edit": false})).unwrap();
        for label in ["One", "Two", "Three"] {
            inv(&mut app, &ctx, "window.customPanel.addItem", json!({"panel": "A", "kind": "heading", "label": label})).unwrap();
            inv(&mut app, &ctx, "window.customPanel.addItem", json!({"panel": "A", "id": "edit.undo"})).unwrap();
        }
        let open = |app: &PhotocraftApp| -> Vec<bool> {
            app.ui.custom_panels.panels[0]
                .items
                .iter()
                .filter_map(|i| if let PanelItem::Heading { collapsed, .. } = i { Some(!collapsed) } else { None })
                .collect()
        };
        // Turning it on keeps only the first open section.
        inv(&mut app, &ctx, "window.customPanel.setLayout", json!({"panel": "A", "accordion": true})).unwrap();
        assert_eq!(open(&app), [true, false, false]);
        // Opening another folds the rest.
        inv(&mut app, &ctx, "window.customPanel.updateItem", json!({"panel": "A", "index": 4, "collapsed": false})).unwrap();
        assert_eq!(open(&app), [false, false, true]);
        // Without it, sections open independently.
        inv(&mut app, &ctx, "window.customPanel.setLayout", json!({"panel": "A", "accordion": false})).unwrap();
        inv(&mut app, &ctx, "window.customPanel.updateItem", json!({"panel": "A", "index": 0, "collapsed": false})).unwrap();
        assert_eq!(open(&app), [true, false, true]);
        let def = inv(&mut app, &ctx, "window.customPanel.export", json!({"panel": "A"})).unwrap();
        assert_eq!(def["panel"]["accordion"], false);
    }

    #[test]
    fn labels_row_counts_wide_buttons_and_icons() {
        let (mut app, ctx) = app();
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "L", "edit": false})).unwrap();
        let add = |app: &mut PhotocraftApp, v: Value| {
            let mut v = v;
            v["panel"] = json!("L");
            inv(app, &ctx, "window.customPanel.addItem", v)
        };
        add(&mut app, json!({"kind": "label", "label": "Radius", "columns": 5})).unwrap();
        add(&mut app, json!({"id": "edit.undo", "span": 2})).unwrap();
        let eraser = crate::tool_shortcuts::id(crate::state::Tool::Eraser);
        add(&mut app, json!({"id": eraser, "icon": true})).unwrap();
        assert!(add(&mut app, json!({"id": "edit.undo", "icon": true})).is_err(), "only tools have icons");
        assert!(add(&mut app, json!({"id": "edit.undo", "span": 9})).is_err());
        assert!(add(&mut app, json!({"kind": "label", "label": "x", "columns": 0})).is_err());
        inv(&mut app, &ctx, "window.customPanel.updateItem", json!({"panel": "L", "index": 0, "columns": null})).unwrap();
        assert_eq!(app.ui.custom_panels.panels[0].items[0].columns(), None);
        assert!(inv(&mut app, &ctx, "window.customPanel.updateItem", json!({"panel": "L", "index": 1, "columns": 3})).is_err(), "buttons don't set columns");
        inv(&mut app, &ctx, "window.customPanel.updateItem", json!({"panel": "L", "index": 1, "span": 3})).unwrap();
        assert_eq!(app.ui.custom_panels.panels[0].items[1].span(), 3);
        // Older saved items (no span, unit divider) still load, and new fields round-trip.
        let old: PanelItem = serde_json::from_value(json!({"kind": "separator"})).unwrap();
        assert_eq!(old, PanelItem::Separator { columns: None });
        let old: PanelItem = serde_json::from_value(json!({"kind": "command", "id": "edit.undo"})).unwrap();
        assert_eq!(old.span(), 1);
        let item = app.ui.custom_panels.panels[0].items[2].clone();
        assert_eq!(serde_json::from_value::<PanelItem>(serde_json::to_value(&item).unwrap()).unwrap(), item);
    }

    #[test]
    fn search_finds_actions_commands_and_tools() {
        let (mut app, _ctx) = app();
        app.session.actions.list.push(photocraft_engine::actions_cmds::Action::new("Frequency Split", vec![]));
        assert!(picker_hits(&app, "").iter().any(|h| h.label == "Frequency Split"), "actions show before typing");
        let hits = picker_hits(&app, "gaussian");
        assert!(hits.iter().any(|h| matches!(&h.item, PanelItem::Command { id, .. } if id == "filter.blur.gaussianBlur")));
        let hits = picker_hits(&app, "eraser tool");
        assert!(hits.iter().any(|h| matches!(&h.item, PanelItem::Command { id, .. } if crate::tool_shortcuts::tool(id).is_some())));
        assert!(
            picker_hits(&app, "custom panel").iter().all(|h| !matches!(&h.item, PanelItem::Command { id, .. } if id.starts_with(PREFIX))),
            "panel commands can't be buttons"
        );
        assert!(picker_hits(&app, "x").len() <= HITS);
    }

    #[test]
    fn clicking_a_drawn_button_runs_it_and_edit_mode_selects_instead() {
        use egui_kittest::{Harness, kittest::Queryable};
        let (mut app, ctx) = app();
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        inv(&mut app, &ctx, "window.customPanel.new", json!({"name": "P", "edit": false})).unwrap();
        inv(
            &mut app,
            &ctx,
            "window.customPanel.addItem",
            json!({"panel": "P", "id": "layer.new.layer", "params": {"name": "From Panel"}, "label": "Make Layer"}),
        )
        .unwrap();
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(|ui, app: &mut PhotocraftApp| windows(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(3);
        let before = h.state().session.active().unwrap().doc.layers.len();
        h.get_by_label("Make Layer").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
        // Edit mode: a click selects the button instead of running it.
        h.get_by_label("Edit Panel").click();
        h.run_steps(3);
        h.get_by_label("Make Layer").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
        assert_eq!(h.state().ui.custom_panels.selected, Some(0));
        assert!(h.query_by_label("Add Button…").is_some());
        // The pencil (also "Done" while editing) and the Done button both end edit mode.
        h.get_all_by_label("Done").last().unwrap().click();
        h.run_steps(3);
        assert_eq!(h.state().ui.custom_panels.editing, None);
    }
}
