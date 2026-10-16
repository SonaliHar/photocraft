//! The style library: the styles, the folders they live in, and the settings beside them.
//!
//! The shape is **TyperTools' own** so a letterer can move a library between Photoshop TyperTools
//! and this port in either direction: the export is
//! `{ignoreLinePrefixes, defaultStyleId, textItemKind, folders, styles, version}` with each style
//! `{id, name, folder, prefixes, prefixColor, textProps, edited}`, and `textProps` is kept verbatim
//! in the jam `layerText` shape TyperTools writes — the port never rewrites a property it does not
//! understand, so a style edited here and taken back to Photoshop keeps everything.
//!
//! Two deliberate differences from TyperTools 1.4.9's own import, both recorded here because they
//! are observable:
//!
//! * `textItemKind` **is** read back (TyperTools exports it and then drops it, because its importer
//!   only accepts keys its state already has and its state calls the same setting `pastePointText`);
//!   `pastePointText` is accepted too, for a file that carries either name;
//! * `exported` is not written. The sandbox has no clock beyond the `now` on each event, and nothing
//!   reads the field; a library that has it is imported all the same.

use serde_json::{Map, Value, json};

use crate::props;

/// TyperTools' default prefix colour (`config.defaultPrefixColor`).
pub const DEFAULT_PREFIX_COLOR: &str = "#FFF3B0";

/// The prefix colours TyperTools offers as presets, in its order.
pub const PREFIX_COLORS: [&str; 8] = ["#FFA6A4", "#FDD4BB", "#FFF3B0", "#CDFDD3", "#BEDADC", "#C2F2FF", "#C3C9FF", "#F6D5FF"];

/// The version written into an export, so a file says what wrote it. This port is at parity with
/// TyperTools 1.4.9 and writes that version.
pub const FORMAT_VERSION: &str = "1.4.9";

/// A folder of styles, in the order the letterer put them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
}

/// One saved text style: the name, where it lives, the script prefixes that pick it, and the text
/// properties it applies (the jam `textProps` object, verbatim).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub id: String,
    pub name: String,
    /// The folder's id, or `None` for the unsorted group TyperTools shows first and locks.
    pub folder: Option<String>,
    pub prefixes: Vec<String>,
    pub prefix_color: String,
    /// Milliseconds since the epoch, as TyperTools stamps it (`Date.now()`); 0 when unknown. Used
    /// only to settle an import that collides with a style edited locally.
    pub edited: u64,
    pub text_props: Value,
}

impl Style {
    /// A new style with TyperTools' starting properties: the default family at 14, centred.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Style {
        Style {
            id: id.into(),
            name: name.into(),
            folder: None,
            prefixes: Vec::new(),
            prefix_color: DEFAULT_PREFIX_COLOR.to_string(),
            edited: 0,
            text_props: props::default_props(),
        }
    }

    /// The prefixes as one space-separated string, the way the editor shows them.
    pub fn prefixes_text(&self) -> String {
        self.prefixes.join(" ")
    }

    /// Splits a space-separated prefix list the way TyperTools does (`split(/\s+/)`, blanks
    /// dropped).
    pub fn set_prefixes_text(&mut self, text: &str) {
        self.prefixes = text.split_whitespace().map(str::to_string).collect();
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "folder": self.folder,
            "prefixes": self.prefixes,
            "prefixColor": self.prefix_color,
            "textProps": self.text_props,
            "edited": self.edited,
        })
    }

    fn from_json(v: &Value) -> Option<Style> {
        let id = v.get("id").and_then(Value::as_str)?.to_string();
        Some(Style {
            id,
            name: v.get("name").and_then(Value::as_str).unwrap_or("Style").to_string(),
            folder: v.get("folder").and_then(Value::as_str).map(str::to_string),
            prefixes: strings(v.get("prefixes")),
            prefix_color: v.get("prefixColor").and_then(Value::as_str).unwrap_or(DEFAULT_PREFIX_COLOR).to_string(),
            edited: v.get("edited").and_then(Value::as_u64).unwrap_or(0),
            text_props: props::normalise(v.get("textProps").cloned().unwrap_or(Value::Null)),
        })
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        // TyperTools accepts the string form on save (`prefixes: "d: D:"`).
        Some(Value::String(s)) => s.split_whitespace().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Everything the panel keeps: the styles, the folders, and the settings around them.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    /// In **matching order**: the unsorted styles first, then each folder's, as TyperTools sorts
    /// them. The first style whose prefix a line starts with wins, so this order is behaviour, not
    /// presentation ([`Library::resort`] is what maintains it).
    pub styles: Vec<Style>,
    pub folders: Vec<Folder>,
    pub ignore_prefixes: Vec<String>,
    pub default_style_id: Option<String>,
    /// `true`: Create and Align work on point text. `false` (TyperTools' default): on a text box.
    pub point_text: bool,
}

impl Default for Library {
    fn default() -> Self {
        Library { styles: Vec::new(), folders: Vec::new(), ignore_prefixes: vec!["##".to_string()], default_style_id: None, point_text: false }
    }
}

impl Library {
    pub fn style(&self, id: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.id == id)
    }

    pub fn style_mut(&mut self, id: &str) -> Option<&mut Style> {
        self.styles.iter_mut().find(|s| s.id == id)
    }

    /// The styles of one folder in order — or, with `None`, the unsorted ones.
    pub fn styles_in(&self, folder: Option<&str>) -> Vec<&Style> {
        self.styles.iter().filter(|s| s.folder.as_deref() == folder).collect()
    }

    /// Puts the unsorted styles first and each folder's after it, keeping the order within each
    /// group: what TyperTools' reducer does on every action, and the order prefixes match in.
    pub fn resort(&mut self) {
        let mut out: Vec<Style> = Vec::with_capacity(self.styles.len());
        for s in &self.styles {
            if s.folder.is_none() {
                out.push(s.clone());
            }
        }
        for f in &self.folders {
            for s in &self.styles {
                if s.folder.as_deref() == Some(f.id.as_str()) {
                    out.push(s.clone());
                }
            }
        }
        // A style naming a folder that is gone would be unreachable: it goes back to the unsorted
        // group, which is exactly where the manager puts it.
        let known: Vec<&str> = self.folders.iter().map(|f| f.id.as_str()).collect();
        for s in &self.styles {
            if let Some(f) = &s.folder
                && !known.contains(&f.as_str())
            {
                let mut s = s.clone();
                s.folder = None;
                out.push(s);
            }
        }
        self.styles = out;
    }

    /// Every style prefix, in matching order: `(style id, prefix)`.
    pub fn prefix_order(&self) -> Vec<(String, String)> {
        self.styles.iter().flat_map(|s| s.prefixes.iter().map(move |p| (s.id.clone(), p.clone()))).collect()
    }

    /// The style a line's prefix picked, or the default style, or the first one: TyperTools'
    /// `currentStyle` fallback chain.
    pub fn current_style(&self, line_style: Option<&str>) -> Option<&Style> {
        line_style.and_then(|id| self.style(id)).or_else(|| self.default_style_id.as_deref().and_then(|id| self.style(id))).or_else(|| self.styles.first())
    }

    /// A new id no style or folder has: `s1`, `s2`, … TyperTools makes a random id; a sequence is
    /// the same thing without a source of randomness, and it is easier to read in an export.
    pub fn new_style_id(&self) -> String {
        let mut n = self.styles.len() + 1;
        loop {
            let id = format!("s{n}");
            if self.style(&id).is_none() {
                return id;
            }
            n += 1;
        }
    }

    pub fn new_folder_id(&self) -> String {
        let mut n = self.folders.len() + 1;
        loop {
            let id = format!("f{n}");
            if !self.folders.iter().any(|f| f.id == id) {
                return id;
            }
            n += 1;
        }
    }

    /// Removes a folder; its styles move to the unsorted group rather than going with it, which is
    /// what the folder editor does (it only ever unassigns).
    pub fn remove_folder(&mut self, id: &str) {
        self.folders.retain(|f| f.id != id);
        for s in &mut self.styles {
            if s.folder.as_deref() == Some(id) {
                s.folder = None;
            }
        }
        self.resort();
    }

    /// The library as TyperTools exports it, and as the plug-in's own storage keeps it.
    pub fn to_json(&self) -> Value {
        json!({
            "ignoreLinePrefixes": self.ignore_prefixes,
            "defaultStyleId": self.default_style_id,
            "textItemKind": if self.point_text { "point" } else { "box" },
            "folders": self.folders.iter().map(|f| json!({"id": f.id, "name": f.name})).collect::<Vec<_>>(),
            "styles": self.styles.iter().map(Style::to_json).collect::<Vec<_>>(),
            "version": FORMAT_VERSION,
        })
    }

    /// A library from a TyperTools export (or from this panel's own storage). Never fails: a field
    /// that is missing or the wrong shape falls back to the default, because the file comes from
    /// outside and half a library is better than none — but a style without an id is dropped, since
    /// nothing could refer to it.
    pub fn from_json(v: &Value) -> Library {
        let mut lib = Library::default();
        if let Some(p) = v.get("ignoreLinePrefixes") {
            lib.ignore_prefixes = strings(Some(p));
        }
        lib.default_style_id = v.get("defaultStyleId").and_then(Value::as_str).map(str::to_string);
        // Either name, whichever the file carries (see the module note).
        lib.point_text = match v.get("textItemKind").and_then(Value::as_str) {
            Some(kind) => kind.eq_ignore_ascii_case("point"),
            None => v.get("pastePointText").and_then(Value::as_bool).unwrap_or(false),
        };
        if let Some(folders) = v.get("folders").and_then(Value::as_array) {
            lib.folders = folders
                .iter()
                .filter_map(|f| {
                    let id = f.get("id").and_then(Value::as_str)?;
                    Some(Folder { id: id.to_string(), name: f.get("name").and_then(Value::as_str).unwrap_or("Folder").to_string() })
                })
                .collect();
        }
        if let Some(styles) = v.get("styles").and_then(Value::as_array) {
            let mut seen: Vec<String> = Vec::new();
            for s in styles {
                if let Some(style) = Style::from_json(s)
                    && !seen.contains(&style.id)
                {
                    seen.push(style.id.clone());
                    lib.styles.push(style);
                }
            }
        }
        lib.resort();
        // A default style that is not there would silently letter with the first style instead.
        if let Some(id) = lib.default_style_id.clone()
            && lib.style(&id).is_none()
        {
            lib.default_style_id = None;
        }
        lib
    }

    /// Merges an imported library into this one the way TyperTools' importer does: styles this
    /// library has edited more recently are kept, and everything else comes from the file. Settings
    /// are always taken from the file. Returns how many of this library's own styles were kept.
    pub fn merge(&mut self, imported: &Library) -> usize {
        let mut kept = 0;
        for mine in &self.styles {
            let theirs = imported.style(&mine.id);
            let keep = match theirs {
                None => true,
                Some(t) => mine.edited > 0 && mine.edited > t.edited,
            };
            if keep {
                kept += 1;
            }
        }
        let mut styles: Vec<Style> = Vec::new();
        for mine in &self.styles {
            match imported.style(&mine.id) {
                None => styles.push(mine.clone()),
                Some(t) => {
                    if mine.edited > 0 && mine.edited > t.edited {
                        styles.push(mine.clone());
                    } else {
                        styles.push(t.clone());
                    }
                }
            }
        }
        for t in &imported.styles {
            if !styles.iter().any(|s| s.id == t.id) {
                styles.push(t.clone());
            }
        }
        self.styles = styles;
        self.folders = imported.folders.clone();
        self.ignore_prefixes = imported.ignore_prefixes.clone();
        self.default_style_id = imported.default_style_id.clone();
        self.point_text = imported.point_text;
        self.resort();
        kept
    }
}

/// The layer-level keys of a style that are not text properties: TyperTools strips exactly these
/// when a style is copied off a layer (`editStyle.jsx` › `copyLayerStyle`), and they mean nothing in
/// a library.
pub const LAYER_KEYS: [&str; 7] = ["textKey", "textShape", "textClickPoint", "transform", "boundingBox", "bounds", "warp"];

/// Drops the layer-level keys from a `layerText` object, in place. The style editor calls this after
/// copying a style off the active layer, as TyperTools does.
pub fn strip_layer_keys(layer_text: &mut Map<String, Value>) {
    for k in LAYER_KEYS {
        layer_text.remove(k);
    }
}

/// One line of the starter table below: id, name, prefixes, folder, size in points, face, caps,
/// alignment, colour.
type Recipe = (&'static str, &'static str, &'static str, &'static str, f64, &'static str, &'static str, &'static str, &'static str);

/// The library the panel starts from when there is none: the five kinds of line a letterer reaches
/// for on a first page, in two folders, with the default style and TyperTools' `##` ignore prefix.
///
/// This is **this port's own**, not a copy of TyperTools' shipped library — that one is not ours to
/// redistribute, and a letterer's real library is their own file anyway (Help ▸ Load). The point is
/// that the panel is usable the minute it opens, and that a prefix is what selects a style.
///
/// `font` is the first family the host has, so the starter styles letter in something that is
/// actually installed; with `None` they keep TyperTools' own default, Tahoma.
///
/// The sizes below are **points**, and the styles say so (`typeUnit`), which is the one place this
/// library departs from TyperTools' `getDefaultStyle()`. Its own style is 14 *pixels*, and the
/// letterer scales it with the Text Size field; a starter library is meant to be usable as it opens,
/// and 14 px is 3.4 pt on the 300 dpi page a comic is drawn at — a tenth of the size the balloon
/// needs. Points mean the same thing on every document, which is what a starting point should do.
pub fn starter(font: Option<&str>) -> Library {
    let mut lib = Library {
        folders: vec![Folder { id: "f1".into(), name: "Dialogue".into() }, Folder { id: "f2".into(), name: "Sound effects".into() }],
        ..Library::default()
    };

    // id, name, prefixes, folder, size in points, face, caps, alignment, colour
    let recipes: [Recipe; 5] = [
        ("s1", "Dialogue", "d. D.", "f1", 14.0, "Regular", "normal", "center", "#000000"),
        ("s2", "Whisper", "w. W.", "f1", 10.0, "Italic", "normal", "center", "#000000"),
        ("s3", "Shout", "s. S.", "f1", 22.0, "Bold", "allCaps", "center", "#000000"),
        ("s4", "Thought", "t. T.", "f1", 12.0, "Italic", "normal", "center", "#000000"),
        ("s5", "Sound effect", "x. X.", "f2", 30.0, "Bold", "allCaps", "center", "#ffffff"),
    ];
    for (i, (id, name, prefixes, folder, size, face, caps, align, colour)) in recipes.iter().enumerate() {
        let mut s = Style::new(*id, *name);
        s.text_props["typeUnit"] = json!("pointsUnit");
        s.folder = Some((*folder).to_string());
        s.set_prefixes_text(prefixes);
        s.prefix_color = PREFIX_COLORS[i % PREFIX_COLORS.len()].to_string();
        if let Some(f) = font {
            props::set_font(&mut s.text_props, f, face, None);
        }
        props::set_char(&mut s.text_props, "size", json!(size));
        props::set_char(&mut s.text_props, "impliedFontSize", json!(size));
        props::set_char(&mut s.text_props, "fontCaps", json!(caps));
        props::set_char(&mut s.text_props, "color", props::hex_color(colour));
        props::set_para(&mut s.text_props, "alignment", json!(align));
        s.edited = 1;
        lib.styles.push(s);
    }
    lib.default_style_id = Some("s1".into());
    lib.point_text = false;
    lib.resort();
    lib
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(id: &str, name: &str, prefix: &str) -> Style {
        let mut s = Style::new(id, name);
        s.set_prefixes_text(prefix);
        s
    }

    #[test]
    fn the_order_styles_match_in_is_unsorted_first_then_folders() {
        let mut lib = Library { folders: vec![Folder { id: "f1".into(), name: "Shouts".into() }], ..Library::default() };
        let mut a = style("s1", "A", "a:");
        a.folder = Some("f1".into());
        lib.styles = vec![a, style("s2", "B", "b:"), style("s3", "C", "c:")];
        lib.resort();
        assert_eq!(
            lib.styles.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["s2", "s3", "s1"],
            "the unsorted ones first, in their own order, then the folder's"
        );
        assert_eq!(lib.prefix_order(), [("s2".into(), "b:".into()), ("s3".into(), "c:".into()), ("s1".into(), "a:".into())]);
        assert_eq!(lib.styles_in(None).len(), 2);
        assert_eq!(lib.styles_in(Some("f1")).len(), 1);
    }

    #[test]
    fn a_style_whose_folder_is_gone_falls_back_to_the_unsorted_group() {
        let mut lib = Library::default();
        let mut a = style("s1", "A", "a:");
        a.folder = Some("gone".into());
        lib.styles = vec![a];
        lib.resort();
        assert_eq!(lib.styles[0].folder, None, "an unreachable style would never match a line");
    }

    #[test]
    fn removing_a_folder_keeps_its_styles() {
        let mut lib = Library { folders: vec![Folder { id: "f1".into(), name: "Shouts".into() }], ..Library::default() };
        let mut a = style("s1", "A", "a:");
        a.folder = Some("f1".into());
        lib.styles = vec![a];
        lib.remove_folder("f1");
        assert!(lib.folders.is_empty());
        assert_eq!(lib.styles.len(), 1, "deleting a folder does not delete its styles");
        assert_eq!(lib.styles[0].folder, None);
    }

    #[test]
    fn an_export_comes_back_the_same() {
        let mut lib = Library { folders: vec![Folder { id: "f1".into(), name: "Shouts".into() }], ..Library::default() };
        lib.ignore_prefixes = vec!["##".into(), "//".into()];
        lib.default_style_id = Some("s1".into());
        lib.point_text = true;
        let mut a = style("s1", "Dialogue", "d: D:");
        a.folder = Some("f1".into());
        a.prefix_color = PREFIX_COLORS[3].into();
        a.edited = 1_700_000_000_000;
        // A property the port knows nothing about survives the round trip: that is the whole point
        // of keeping `textProps` verbatim.
        a.text_props["layerText"]["textStyleRange"][0]["textStyle"]["fontTechnology"] = json!(1);
        lib.styles = vec![a];
        let json = lib.to_json();
        assert_eq!(json["textItemKind"], "point");
        assert_eq!(json["version"], FORMAT_VERSION);
        let back = Library::from_json(&json);
        assert_eq!(back, lib, "an export re-imports as the library it was");
        assert_eq!(back.styles[0].text_props["layerText"]["textStyleRange"][0]["textStyle"]["fontTechnology"], 1);
    }

    #[test]
    fn a_hostile_file_imports_as_far_as_it_can() {
        // Every field the wrong shape, a style with no id, a folder with no name, a duplicate id.
        let v = json!({
            "ignoreLinePrefixes": "## //",
            "defaultStyleId": 7,
            "textItemKind": "sideways",
            "folders": [{"name": "no id"}, {"id": "f1"}],
            "styles": [{"name": "no id"}, {"id": "s1", "prefixes": 3, "textProps": "not an object"}, {"id": "s1", "name": "twice"}],
        });
        let lib = Library::from_json(&v);
        assert_eq!(lib.ignore_prefixes, ["##", "//"], "the string form is accepted");
        assert_eq!(lib.default_style_id, None);
        assert!(!lib.point_text);
        assert_eq!(lib.folders.len(), 1);
        assert_eq!(lib.styles.len(), 1, "a style with no id is dropped, a duplicate id once");
        // The *first* entry for an id is the one that counts: a second one later in the file cannot
        // shadow the style everything above it already refers to.
        assert_eq!(lib.styles[0].name, "Style", "the first of the two `s1`s, with a name it never had");
        assert!(lib.styles[0].prefixes.is_empty());
        assert!(lib.styles[0].text_props.is_object(), "bad properties become the defaults");
        // And the whole thing is still a library that can be exported: nothing here panics.
        assert!(Library::from_json(&json!({"styles": null, "folders": 5})).styles.is_empty());
        assert!(Library::from_json(&Value::Null).styles.is_empty());
    }

    #[test]
    fn a_merge_keeps_the_styles_edited_here_and_takes_the_rest() {
        let mut mine = Library::default();
        let mut local = style("s1", "Local", "d:");
        local.edited = 2_000;
        mine.styles = vec![local, style("s2", "Only here", "x:")];
        let mut theirs = Library::default();
        let mut theirs_s1 = style("s1", "Theirs", "d:");
        theirs_s1.edited = 1_000;
        theirs.styles = vec![theirs_s1, style("s3", "Only there", "y:")];
        let kept = mine.merge(&theirs);
        assert_eq!(kept, 2, "s1 was edited here later, s2 is not in the file");
        assert_eq!(mine.styles.len(), 3);
        assert_eq!(mine.style("s1").map(|s| s.name.as_str()), Some("Local"));
        assert_eq!(mine.style("s3").map(|s| s.name.as_str()), Some("Only there"));
    }

    #[test]
    fn ids_are_sequence_numbers_that_are_not_taken() {
        let mut lib = Library { styles: vec![style("s1", "A", "a:"), style("s2", "B", "b:")], ..Library::default() };
        assert_eq!(lib.new_style_id(), "s3");
        lib.folders = vec![Folder { id: "f1".into(), name: "F".into() }];
        assert_eq!(lib.new_folder_id(), "f2");
        lib.folders.push(Folder { id: "f2".into(), name: "G".into() });
        assert_eq!(lib.new_folder_id(), "f3");
    }

    #[test]
    fn the_current_style_falls_back_the_way_typertools_does() {
        let mut lib = Library::default();
        let mut a = style("s1", "A", "a:");
        a.edited = 1;
        lib.styles = vec![a, style("s2", "B", "b:")];
        lib.default_style_id = Some("s2".into());
        assert_eq!(lib.current_style(Some("s1")).map(|s| s.id.as_str()), Some("s1"), "the line's own style");
        assert_eq!(lib.current_style(None).map(|s| s.id.as_str()), Some("s2"), "else the default");
        lib.default_style_id = None;
        assert_eq!(lib.current_style(Some("gone")).map(|s| s.id.as_str()), Some("s1"), "else the first style");
        lib.styles.clear();
        assert!(lib.current_style(None).is_none(), "and nothing at all in an empty library");
    }

    #[test]
    fn the_starter_library_is_usable_and_names_the_font_it_was_given() {
        let lib = starter(Some("Inter"));
        assert_eq!(lib.styles.len(), 5);
        assert_eq!(lib.folders.len(), 2);
        assert_eq!(lib.ignore_prefixes, vec!["##".to_string()]);
        assert_eq!(lib.default_style_id.as_deref(), Some("s1"));
        assert!(lib.styles.iter().all(|s| s.folder.is_some()), "every starter style is in a folder");
        assert!(lib.styles.iter().all(|s| props::family(&s.text_props) == "Inter"));
        assert!(lib.styles.iter().all(|s| !s.prefixes.is_empty()));
        // Every prefix matches a line, and the style it matches is the one that carries it.
        let order = lib.prefix_order();
        let lines = crate::script::parse("d. hello\nx. BANG\nplain\n## a note", &lib.ignore_prefixes, &order);
        assert_eq!(lines[0].style_id.as_deref(), Some("s1"));
        assert_eq!(lines[1].style_id.as_deref(), Some("s5"));
        assert_eq!(lines[2].style_id, None);
        assert!(lines[3].ignored);
        // With no font to offer, a starter style keeps TyperTools' own default rather than nothing.
        let lib = starter(None);
        assert!(lib.styles.iter().all(|s| props::family(&s.text_props) == props::DEFAULT_FONT));
        // The whole thing survives a round trip through the library format.
        let back = Library::from_json(&lib.to_json());
        assert_eq!(back, lib);
    }
}
