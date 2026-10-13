//! Filter dialogs generated from engine command parameter specs, with live on-canvas preview.
//!
//! The preview runs the *same engine command* on the downsampled proxy document (pixel-sized
//! parameters scaled by the proxy factor), so what you preview is what you get.

use std::sync::Arc;

use photocraft_doc::Document;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Range {
        min: f32,
        max: f32,
        default: f32,
    },
    Choice(Vec<String>),
    Bool(bool),
    Int {
        default: i64,
    },
    /// Free text (e.g. a file path).
    Text,
    /// One of the open documents (stored as its index).
    Document,
    /// A row-major grid of integers (`int[25]` = 5×5).
    Grid(usize),
    /// Structured JSON (pins, curve points…): settable through the command, not shown in the dialog.
    Json,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub key: String,
    pub kind: Kind,
}

/// Parse the registry's parameter notation, e.g.
/// `{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0,"horizontal":px=0}`.
pub fn parse_spec(spec: &str) -> Vec<Param> {
    let inner = spec.trim().trim_start_matches('{').trim_end_matches('}');
    let mut out = Vec::new();
    // Split on commas that start a new `"key":` (not inside strings or brackets).
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut depth = 0i32;
    for ch in inner.chars() {
        if ch == '"' {
            in_str = !in_str;
        }
        if !in_str {
            match ch {
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if ch == ',' && !in_str && depth == 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    for part in parts {
        let Some((k, v)) = part.split_once(':') else { continue };
        let key = k.trim().trim_matches('"').to_string();
        if key.is_empty() || key == "layer" {
            continue;
        }
        let v = v.trim();
        let kind = if let Some(choices) = v.strip_prefix('"') {
            let body = choices.split('"').next().unwrap_or("");
            Kind::Choice(body.split('|').map(str::to_string).collect())
        } else if let Some(rest) = v.strip_prefix("bool") {
            Kind::Bool(rest.trim_start_matches('=').trim() == "true")
        } else if v == "text" {
            Kind::Text
        } else if v == "doc" {
            Kind::Document
        } else if v == "json" || v.starts_with("layer id") || v.starts_with('[') || v.starts_with('{') {
            Kind::Json
        } else if let Some(n) = v.strip_prefix("int[").and_then(|r| r.strip_suffix(']')).and_then(|n| n.parse().ok()) {
            Kind::Grid(n)
        } else if let Some((range, default)) = v.split_once('=').map(|(a, b)| (a, b.trim())).or(Some((v, ""))) {
            if let Some((lo, hi)) = range.split_once("..") {
                let min = lo.trim().parse().unwrap_or(0.0);
                let max = hi.trim().parse().unwrap_or(100.0);
                let default = default.parse().unwrap_or(min);
                Kind::Range { min, max, default }
            } else {
                Kind::Int { default: default.parse().unwrap_or(0) }
            }
        } else {
            continue;
        };
        out.push(Param { key, kind });
    }
    out
}

/// Parameter keys measured in pixels (scaled for proxy previews).
fn is_pixel_param(key: &str) -> bool {
    matches!(
        key,
        "radius"
            | "distance"
            | "cellSize"
            | "horizontal"
            | "vertical"
            | "height"
            | "wavelengthMin"
            | "wavelengthMax"
            | "amplitudeMin"
            | "amplitudeMax"
            | "maxRadius"
            | "size"
            | "blur"
            | "speed"
    )
}

/// Commands outside `filter.*` that get the schema dialog *with* live preview.
pub const PREVIEWED: &[&str] = &[
    "image.adjustments.selectiveColor",
    "image.adjustments.colorLookup",
    "image.adjustments.shadowsHighlights",
    "image.adjustments.replaceColor",
    "image.adjustments.matchColor",
    "image.adjustments.hdrToning",
    "image.rotation.arbitrary",
    "image.mode.indexedColor",
    "image.mode.bitmap",
    "image.mode.duotone",
    "layer.matting.defringe",
    "layer.matting.colorDecontaminate",
    "layer.layerStyle.scaleEffects",
];

pub fn has_dialog(command: &str) -> bool {
    // The Filter Gallery has its own full-window dialog (gallery_ui).
    if command == "filter.filterGallery" {
        return false;
    }
    (command.starts_with("filter.")
        || command.starts_with("select.modify.")
        || PREVIEWED.contains(&command)
        || matches!(
            command,
            "image.trim"
                | "image.duplicate"
                | "channel.duplicate"
                | "channel.new"
                | "channel.newSpot"
                | "view.newGuide"
                | "select.refineEdge"
                | "edit.assignProfile"
                | "edit.convertToProfile"
                | "view.proofSetup"
                | "layer.layerStyle.globalLight"
                | "image.mode.colorTable"
        ))
        && photocraft_engine::commands::find(command).is_some_and(|c| !parse_spec(c.params).is_empty())
}

pub fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    if command == "view.newGuide" {
        return crate::guide_dialog::open(app);
    }
    if matches!(command, "channel.new" | "channel.newSpot") {
        return crate::channel_create_dialog::open(app, command);
    }
    if command == "channel.duplicate" {
        return crate::channel_action_dialog::open(app, command, &Value::Null);
    }
    if matches!(command, "image.mode.indexedColor" | "image.mode.bitmap" | "image.mode.colorTable" | "image.mode.duotone") {
        return crate::mode_conversion_dialog::open(app, command);
    }
    if matches!(command, "edit.assignProfile" | "edit.convertToProfile") {
        return crate::profile_dialog::open(app, command);
    }
    if command == "image.duplicate" {
        return crate::duplicate::open(app);
    }
    if command == "image.trim" {
        return crate::trim::open(app);
    }
    let spec = photocraft_engine::commands::find(command)?;
    let mut fields = Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(spec.label));
    fields.insert("__filter".into(), json!(true));
    if command.starts_with("filter.") || PREVIEWED.contains(&command) {
        fields.insert("__preview".into(), json!(true));
    }
    for p in parse_spec(spec.params) {
        let v = match &p.kind {
            Kind::Range { default, .. } => json!(default),
            Kind::Choice(c) => json!(c.first().cloned().unwrap_or_default()),
            Kind::Bool(default) => json!(default),
            Kind::Int { default } => json!(default),
            Kind::Text => json!(""),
            Kind::Document => json!(-1),
            // Identity kernel: 1 in the centre.
            Kind::Grid(n) => json!((0..*n).map(|i| i64::from(i == *n / 2)).collect::<Vec<_>>()),
            // Colour inputs come from the current swatches, so the proxy preview matches the result.
            Kind::Json if p.key == "foreground" => json!(app.session.tools.foreground),
            Kind::Json if p.key == "background" => json!(app.session.tools.background),
            Kind::Json => continue,
        };
        fields.insert(p.key, v);
    }
    if command == "view.newGuide" {
        let active=app.session.active()?;
        let doc=&active.doc;
        fields.insert("documentId".into(),json!(doc.id.0.to_string()));
        fields.insert("expectedRevision".into(),json!(active.revision.to_string()));
        let legacy=photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
        let resolution=doc.physical_resolution.as_ref().unwrap_or(&legacy);
        resolution.validate().ok()?;
        let factor=match &resolution.unit {
            photocraft_doc::PhysicalUnit::Pixel=>None,
            photocraft_doc::PhysicalUnit::Inch=>Some(1.0),
            photocraft_doc::PhysicalUnit::Millimeter=>Some(25.4),
            photocraft_doc::PhysicalUnit::Point=>Some(72.0),
            photocraft_doc::PhysicalUnit::Pica=>Some(6.0),
            photocraft_doc::PhysicalUnit::Custom(unit)=>Some(unit.factor),
        };
        fields.insert("position".into(),json!(0.0));
        fields.insert("unit".into(),json!("px"));
        fields.insert("__guideDpiX".into(),json!(resolution.x_dpi));
        fields.insert("__guideDpiY".into(),json!(resolution.y_dpi));
        fields.insert("__guideDocumentFactor".into(),json!(factor));
    }
    if command == "image.rotation.arbitrary" {
        straighten_defaults(app, &mut fields);
    }
    if parse_spec(spec.params).iter().any(|p| p.kind == Kind::Document) {
        // The document picker lists every open document (params refer to them by index).
        let names: Vec<String> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        fields.insert("__docs".into(), json!(names));
    }
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields);
    Some(id)
}

/// Arbitrary rotation starts at the angle that straightens the ruler line, when there is one.
fn straighten_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(r) = app.session.active().and_then(|d| d.doc.measurement.ruler) else { return };
    let rot = photocraft_engine::analysis_cmds::straighten_angle(&r);
    // A ruler read from a damaged file could hold non-finite ends: keep the 0° default then.
    if !rot.is_finite() {
        return;
    }
    fields.insert("angle".into(), json!(rot.abs()));
    fields.insert("direction".into(), json!(if rot < 0.0 { "ccw" } else { "cw" }));
}

/// A filter dialog with live preview for `command` whose parameters follow `spec` (registry
/// notation) instead of the command's own; `fixed` params (e.g. a plug-in id) are passed through.
pub fn open_with_spec(app: &mut PhotocraftApp, command: &str, label: &str, spec: &str, fixed: Map<String, Value>) -> u64 {
    let mut fields = fixed;
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    fields.insert("__filter".into(), json!(true));
    fields.insert("__preview".into(), json!(true));
    fields.insert("__spec".into(), json!(spec));
    for p in parse_spec(spec) {
        let v = match &p.kind {
            Kind::Range { default, .. } => json!(default),
            Kind::Choice(c) => json!(c.first().cloned().unwrap_or_default()),
            Kind::Bool(default) => json!(default),
            Kind::Int { default } => json!(default),
            _ => continue,
        };
        fields.insert(p.key, v);
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, fields)
}

pub(crate) fn label(key: &str) -> String {
    tl!(&source_label(key)).to_owned()
}

pub(crate) fn source_label(key: &str) -> String {
    // camelCase → "Camel Case"
    let mut s = String::new();
    for (i, ch) in key.chars().enumerate() {
        if i == 0 {
            s.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            s.push(' ');
            s.push(ch);
        } else {
            s.push(ch);
        }
    }
    s
}

fn choice_label(v: &str) -> String {
    label(v)
}

fn uses_logarithmic_slider(min: f32, max: f32) -> bool {
    min > 0.0 && max / min > 500.0
}

fn guide_position(text:&str)->Option<f64> {
    if text.len()>64 { return None; }
    let value=text.trim().parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}
// Captured axes are preview metadata only. The registered command revalidates
// the current document and converts in f64 before deliberate native f32 storage.
fn guide_preview(f:&Map<String,Value>)->Option<f64> {
    let value=f.get("position")?.as_f64()?;
    if !value.is_finite() { return None; }
    let dpi=match f.get("orientation")?.as_str()? {
        "vertical"=>f.get("__guideDpiX")?.as_f64()?,
        "horizontal"=>f.get("__guideDpiY")?.as_f64()?,
        _=>return None,
    };
    if !dpi.is_finite() || dpi<=0.0 { return None; }
    let factor=match f.get("unit")?.as_str()? {
        "px"=>None,"in"=>Some(1.0),"mm"=>Some(25.4),"pt"=>Some(72.0),"pc"=>Some(6.0),
        "document"=>match f.get("__guideDocumentFactor")? { Value::Null=>None,value=>Some(value.as_f64()?) },
        _=>return None,
    };
    let result=if let Some(factor)=factor {
        if !factor.is_finite() || factor<=0.0 { return None; }
        // The shared checked converter handles subnormal/extreme products.
        let resolution=photocraft_doc::PhysicalResolution::square(dpi);
        let unit=photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {name:"Guide preview".into(),factor,digits:0,symbol:String::new(),abbreviation:String::new()});
        let pixels=photocraft_doc::physical_size::units_to_pixels(value.abs(),&resolution,&unit,photocraft_doc::physical_size::Axis::X).ok()?;
        pixels.copysign(value)
    } else {value};
    (result.is_finite() && result.abs()<=f64::from(1<<29)).then_some(result)
}
fn change_guide_unit(f:&mut Map<String,Value>,unit:&str)->Option<()> {
    let pixels=guide_preview(f)?;
    let dpi=match f.get("orientation")?.as_str()? {"vertical"=>f.get("__guideDpiX")?.as_f64()?,"horizontal"=>f.get("__guideDpiY")?.as_f64()?,_=>return None};
    let factor=match unit {"px"=>None,"in"=>Some(1.0),"mm"=>Some(25.4),"pt"=>Some(72.0),"pc"=>Some(6.0),
        "document"=>match f.get("__guideDocumentFactor")? {Value::Null=>None,value=>Some(value.as_f64()?)},_=>return None};
    let position=if let Some(factor)=factor {
        let resolution=photocraft_doc::PhysicalResolution::square(dpi);
        let unit=photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {name:"Guide preview".into(),factor,digits:0,symbol:String::new(),abbreviation:String::new()});
        photocraft_doc::physical_size::pixels_to_units(pixels.abs(),&resolution,&unit,photocraft_doc::physical_size::Axis::X).ok()?.copysign(pixels)
    } else {pixels};
    if !position.is_finite() {return None;}
    f.insert("position".into(),json!(position));
    f.insert("unit".into(),json!(unit));
    Some(())
}
fn new_guide_body(ui:&mut egui::Ui,f:&mut Map<String,Value>) {
    let t=Tokens::get(ui.ctx());
    let Some(mut orientation)=f.get("orientation").and_then(Value::as_str).map(str::to_owned) else { return; };
    let mut unit=match f.get("unit") { None=>"px".to_owned(),Some(value)=>{ let Some(unit)=value.as_str() else { return; }; unit.to_owned() } };
    let Some(mut position)=f.get("position").and_then(Value::as_f64) else { return; };
    ui.horizontal(|ui| {
        ui.label(tl!("Orientation"));
        if crate::widgets::dropdown(ui,"new-guide-orientation",&mut orientation,&[("horizontal".into(),tl!("Horizontal")),("vertical".into(),tl!("Vertical"))],140.0) {
            f.insert("orientation".into(),json!(orientation));
        }
    });
    ui.horizontal(|ui| {
        let label=ui.label(tl!("Position"));
        let edited=ui.push_id("new-guide-position",|ui|ui.add_sized([110.0,22.0],egui::DragValue::new(&mut position)
            .speed(0.5).clamp_existing_to_range(false).custom_parser(guide_position).custom_formatter(|value,_|value.to_string())
            .update_while_editing(false)).labelled_by(label.id).changed()).inner;
        if edited && position.is_finite() { f.insert("position".into(),json!(position)); }
        let units=[("px".into(),tl!("Pixels")),("in".into(),tl!("Inches")),("mm".into(),tl!("Millimeters")),
            ("pt".into(),tl!("Points")),("pc".into(),tl!("Picas")),("document".into(),tl!("Document"))];
        if crate::widgets::dropdown(ui,"new-guide-unit",&mut unit,&units,140.0) { let _=change_guide_unit(f,&unit); }
    });
    if let Some(pixels)=guide_preview(f) { ui.label(egui::RichText::new(format!("{pixels} px")).color(t.text_dim)); }
}

/// Dialog body for filter commands.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.get("__command").and_then(Value::as_str)==Some("view.newGuide") {
        new_guide_body(ui,f);
        return;
    }
    let t = Tokens::get(ui.ctx());
    let cmd = f.get("__command").and_then(Value::as_str).unwrap_or_default().to_string();
    // Plug-in dialogs carry their own spec (from the plug-in's manifest).
    let spec = match f.get("__spec").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => match photocraft_engine::commands::find(&cmd) {
            Some(c) => c.params.to_string(),
            None => return,
        },
    };
    for p in parse_spec(&spec) {
        match p.kind {
            Kind::Range { min, max, default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(default as f64) as f32;
                let unit = if is_pixel_param(&p.key) {
                    "px"
                } else if p.key == "angle" {
                    "°"
                } else if p.key == "amount" && max <= 500.0 {
                    "%"
                } else {
                    ""
                };
                if uses_logarithmic_slider(min, max) {
                    // Keep one numeric field for direct entry and one log-scaled slider for
                    // wide ranges. slider_row would add a second, linear slider (#146).
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            crate::widgets::value_field(ui, &mut v, min..=max, unit, 74.0);
                        });
                    });
                    let mut lv = v.clamp(min, max).ln();
                    if crate::widgets::slider(ui, &mut lv, min.ln()..=max.ln(), None).changed() {
                        v = lv.exp();
                    }
                    ui.add_space(4.0);
                } else {
                    crate::widgets::slider_row(ui, &label(&p.key), &mut v, min..=max, unit, None);
                }
                f.insert(p.key, json!((v * 10.0).round() / 10.0));
            }
            Kind::Choice(options) => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    let mut cur = f.get(&p.key).and_then(Value::as_str).unwrap_or(&options[0]).to_string();
                    let labels: Vec<String> = options.iter().map(|o| choice_label(o)).collect();
                    let opts: Vec<(String, &str)> = options.iter().cloned().zip(labels.iter().map(String::as_str)).collect();
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &mut cur, &opts, 170.0);
                    f.insert(p.key.clone(), json!(cur));
                });
            }
            Kind::Bool(default) => {
                let mut b = f.get(&p.key).and_then(Value::as_bool).unwrap_or(default);
                crate::widgets::checkbox(ui, &mut b, &label(&p.key));
                f.insert(p.key, json!(b));
            }
            Kind::Text => {
                let mut v = f.get(&p.key).and_then(Value::as_str).unwrap_or_default().to_string();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    ui.add(egui::TextEdit::singleline(&mut v).desired_width(200.0));
                });
                if v.is_empty() {
                    f.remove(&p.key);
                } else {
                    f.insert(p.key, json!(v));
                }
            }
            Kind::Document => {
                let names: Vec<String> =
                    f.get("__docs").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let cur = f.get(&p.key).and_then(Value::as_i64).unwrap_or(-1);
                let mut sel = cur.to_string();
                let mut opts: Vec<(String, &str)> = vec![("-1".to_string(), tl!("None"))];
                opts.extend(names.iter().enumerate().map(|(i, n)| (i.to_string(), n.as_str())));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &mut sel, &opts, 170.0);
                });
                match sel.parse::<i64>() {
                    Ok(i) if i >= 0 => f.insert(p.key, json!(i)),
                    _ => f.insert(p.key, json!(-1)),
                };
            }
            Kind::Grid(n) => {
                let side = (n as f32).sqrt().round().max(1.0) as usize;
                let mut vals: Vec<f32> =
                    f.get(&p.key).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                vals.resize(n, 0.0);
                ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                egui::Grid::new(format!("flt-grid-{cmd}-{}", p.key)).spacing([4.0, 4.0]).show(ui, |ui| {
                    for (i, v) in vals.iter_mut().enumerate() {
                        crate::widgets::value_field(ui, v, -999.0..=999.0, "", 44.0);
                        if (i + 1) % side == 0 {
                            ui.end_row();
                        }
                    }
                });
                f.insert(p.key, json!(vals.iter().map(|v| v.round() as i64).collect::<Vec<_>>()));
            }
            Kind::Json => {}
            Kind::Int { default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(default as f64) as f32;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    crate::widgets::value_field(ui, &mut v, -30000.0..=30000.0, "", 80.0);
                });
                f.insert(p.key, json!(v.round() as i64));
            }
        }
    }
    // Read-only context the dialog opener supplies (e.g. the monitor profile in use).
    if let Some(note) = f.get("__note").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(note).color(t.text_dim)).wrap());
    }
    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

/// User-facing params (strip the dialog's private `__` keys).
pub fn params_of(f: &Map<String, Value>) -> Value {
    // A document picker left at "None" (-1) means "not given".
    let unset = |k: &str, v: &Value| k == "mapDocument" && v.as_i64() == Some(-1);
    Value::Object(f.iter().filter(|(k, v)| !k.starts_with("__") && !unset(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// Compute a preview document: run `command` with `params` on the proxy (scaled) copy of `doc`.
pub fn preview_document(doc: &Document, active: Option<photocraft_doc::LayerId>, command: &str, params: &Value, k: u32) -> Result<Document, String> {
    let proxy = crate::proxy::proxy_document(doc, k).map_err(|error| error.to_string())?;
    let mut s = photocraft_engine::Session::new();
    s.add_document(proxy, None).map_err(|error| error.to_string())?;
    if let Some(id) = active {
        s.select_layer(id).map_err(|error| error.to_string())?;
    }
    let mut p = params.clone();
    if k > 1
        && let Some(o) = p.as_object_mut()
    {
        for (key, v) in o.iter_mut() {
            if is_pixel_param(key)
                && let Some(x) = v.as_f64()
            {
                *v = json!((x / k as f64).max(if key == "cellSize" { 1.0 } else { 0.1 }));
            }
        }
    }
    s.execute(command, p).map_err(|error| error.to_string())?;
    s.active().and_then(|d| d.doc.resident_arc()).map(|d| d.as_ref().clone()).ok_or_else(|| "Preview command produced no document".into())
}

/// Cached preview state on the app.
pub struct FilterPreview {
    pub doc: photocraft_doc::DocId,
    pub revision: u64,
    pub hash: u64,
    pub k: u32,
    pub result: Option<Arc<Document>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_notation() {
        let p = parse_spec(r#"{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0,"horizontal":px=0}"#);
        assert_eq!(p[0], Param { key: "radius".into(), kind: Kind::Range { min: 0.1, max: 1000.0, default: 1.0 } });
        assert_eq!(p[1].kind, Kind::Choice(vec!["spin".into(), "zoom".into()]));
        assert_eq!(p[2].kind, Kind::Bool(false));
        assert_eq!(p[3].kind, Kind::Int { default: 0 });
        assert_eq!(p[4].kind, Kind::Int { default: 0 });
        assert!(parse_spec("{}").is_empty());
        let p = parse_spec(r#"{"lighting":bool=true,"kernel":int[25],"pins":json,"points":[[0,0],[1,0]],"mapPath":text,"mapDocument":doc,"scale":1..9999=1}"#);
        let kinds: Vec<&Kind> = p.iter().map(|p| &p.kind).collect();
        assert_eq!(
            kinds,
            [&Kind::Bool(true), &Kind::Grid(25), &Kind::Json, &Kind::Json, &Kind::Text, &Kind::Document, &Kind::Range { min: 1.0, max: 9999.0, default: 1.0 }]
        );
    }

    /// A layer-id param has no number field: drawing the Auto-Align dialog used to write
    /// `reference: 0` back, which the engine rejects as not a selected layer (#674).
    #[test]
    fn layer_id_params_stay_out_of_the_dialog() {
        let mut f = Map::new();
        f.insert("__command".into(), json!("edit.autoAlignLayers"));
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
        assert!(!params_of(&f).as_object().unwrap().contains_key("reference"), "{f:?}");
    }

    #[test]
    fn wide_positive_ranges_use_the_logarithmic_slider_path() {
        assert!(uses_logarithmic_slider(0.1, 1000.0), "Gaussian Blur radius");
        assert!(uses_logarithmic_slider(1.0, 9999.0));
        assert!(!uses_logarithmic_slider(1.0, 500.0));
        assert!(!uses_logarithmic_slider(0.0, 1000.0));
    }

    #[test]
    fn new_filters_have_dialogs_or_run_directly() {
        for id in [
            "filter.stylize.oilPaint",
            "filter.blur.lensBlur",
            "filter.blurGallery.irisBlur",
            "filter.other.custom",
            "filter.distort.displace",
            "filter.pixelate.mezzotint",
            "filter.render.lightingEffects",
            "filter.render.relight",
        ] {
            assert!(has_dialog(id), "{id}");
        }
        for id in ["filter.pixelate.facet", "filter.pixelate.fragment", "filter.video.ntscColors"] {
            assert!(!has_dialog(id), "{id}");
        }
    }

    #[test]
    fn every_filter_command_spec_parses() {
        for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("filter.")) {
            let _ = parse_spec(c.params);
        }
        assert!(has_dialog("filter.blur.gaussianBlur"));
        assert!(!has_dialog("filter.stylize.findEdges"));
    }

    #[test]
    fn korean_covers_generated_filter_options_and_gallery_names() {
        let ko = crate::i18n::lang_from_tag("ko-KR").unwrap();
        let mut missing = std::collections::BTreeSet::new();
        let mut check = |s: String| {
            if !matches!(s.as_str(), "X" | "Y" | "A" | "B") && !crate::i18n::has(ko, &s) {
                missing.insert(s);
            }
        };
        for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("filter.") || c.id.starts_with("image.adjustments.")) {
            for p in parse_spec(c.params) {
                if !p.key.chars().all(|c| c.is_ascii_alphanumeric()) || matches!(p.kind, Kind::Json) {
                    continue;
                }
                check(source_label(&p.key));
                if let Kind::Choice(choices) = p.kind {
                    for choice in choices {
                        // Only symbolic options are UI choices; registry docs also contain
                        // colour syntax and array notation, which are not translatable names.
                        if choice.chars().all(|c| c.is_ascii_alphanumeric()) {
                            check(source_label(&choice));
                        }
                    }
                }
            }
        }
        for f in photocraft_algo::GalleryFilter::ALL {
            check(f.name().to_string());
        }
        for cat in photocraft_algo::GALLERY_CATEGORIES {
            check(cat.to_string());
        }
        assert!(missing.is_empty(), "missing Korean dynamic labels: {missing:#?}");
    }

    #[test]
    fn preview_runs_engine_command_on_proxy() {
        let mut doc = Document::with_background(
            "p",
            photocraft_doc::Size::new(64, 64),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let bg = doc.layers[0].id;
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[0.0, 0.0, 0.0, 1.0]);
        let out = preview_document(&doc, Some(bg), "filter.blur.gaussianBlur", &json!({"radius": 4.0}), 1).unwrap();
        let p = out.layers[0].surface().unwrap().pixel(32, 32);
        assert!(p[0] > 0.2 && p[0] < 0.8, "edge blurred: {p:?}");
        assert_eq!(label("wavelengthMin"), "Wavelength Min");
    }

    #[test]
    fn preview_stays_inside_the_selection_at_every_proxy_factor() {
        use photocraft_doc::SampleType;
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc =
                Document::with_background("p", photocraft_doc::Size::new(64, 64), photocraft_doc::ColorMode::Rgb, depth, photocraft_doc::Color::gray(0.5));
            let bg = doc.layers[0].id;
            let mut sel = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8);
            sel.fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[1.0]);
            doc.selection = Some(sel);
            for k in [1, 2, 4] {
                let out = preview_document(&doc, Some(bg), "filter.noise.addNoise", &json!({"amount": 100.0}), k).unwrap();
                let s = out.layers[0].surface().unwrap();
                let half = 32 / k as i32;
                let changed = |x0: i32, x1: i32| (0..64 / k as i32).any(|y| (x0..x1).any(|x| s.pixel(x, y) != doc.layers[0].surface().unwrap().pixel(0, 0)));
                assert!(changed(0, half), "{depth:?} k={k}: noise inside the selection");
                assert!(!changed(half, 64 / k as i32), "{depth:?} k={k}: nothing outside the selection");
            }
        }
    }
}

#[cfg(test)]
mod guide_unit_tests {
    use super::*;
    #[test]
    fn guide_f64_field_keeps_low_bits_and_axis_preview() {
        let exact=f64::from_bits(1.25f64.to_bits()+1);
        assert_ne!(exact,f64::from(exact as f32));
        assert_eq!(guide_position(&exact.to_string()).unwrap().to_bits(),exact.to_bits());
        let mut f=Map::new();
        f.insert("position".into(),json!(1.0));
        f.insert("unit".into(),json!("in"));
        f.insert("orientation".into(),json!("vertical"));
        f.insert("__guideDpiX".into(),json!(144.0));
        f.insert("__guideDpiY".into(),json!(288.0));
        f.insert("__guideDocumentFactor".into(),json!(2.0));
        assert_eq!(guide_preview(&f),Some(144.0));
        f.insert("orientation".into(),json!("horizontal"));
        assert_eq!(guide_preview(&f),Some(288.0));
        f.insert("unit".into(),json!("document"));
        f.insert("position".into(),json!(-1.0));
        assert_eq!(guide_preview(&f),Some(-144.0));
        change_guide_unit(&mut f,"px").unwrap();
        assert_eq!(f["position"],-144.0);
        assert_eq!(guide_preview(&f),Some(-144.0));
        change_guide_unit(&mut f,"in").unwrap();
        assert_eq!(f["position"],-0.5);
        assert!(guide_position("NaN").is_none());
        assert!(guide_position(&"1".repeat(65)).is_none());
    }
    #[test]
    fn guide_dialog_dispatches_per_axis_unit_without_mutating_resolution() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":144,"height":288})).unwrap();
        let resolution=photocraft_doc::PhysicalResolution {x_dpi:144.0,y_dpi:288.0,unit:photocraft_doc::PhysicalUnit::Inch};
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().set_physical_resolution(resolution.clone()).unwrap();
        for orientation in ["vertical","horizontal"] {
            let id=open(&mut app,"view.newGuide").unwrap();
            let f=&mut app.ui.dialog_mut(id).unwrap().fields;
            f.insert("orientation".into(),json!(orientation));
            f.insert("position".into(),json!(1.0));
            f.insert("unit".into(),json!("in"));
            assert_eq!(params_of(f)["unit"],"in");
            assert!(!params_of(f).as_object().unwrap().keys().any(|key|key.starts_with("__")));
            crate::dialogs::confirm(&mut app,id).unwrap();
        }
        let doc=&app.session.active().unwrap().doc;
        assert!(doc.guides.vertical.contains(&144.0));
        assert!(doc.guides.horizontal.contains(&288.0));
        assert_eq!(doc.effective_resolution(),resolution);
    }
    #[test]
    fn revised_guide_dialog_target_refuses_without_adding_guide() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();
        let id=open(&mut app,"view.newGuide").unwrap();
        let initial=app.session.active().unwrap().doc.id.0.to_string();
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["documentId"],initial);
        app.session.execute("view.newGuide",json!({"orientation":"vertical","position":1.0})).unwrap();
        let before=app.session.active().unwrap().doc.clone();
        let revision=app.session.active().unwrap().revision;
        assert!(crate::dialogs::confirm(&mut app,id).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.active().unwrap().revision,revision);
    }

}
