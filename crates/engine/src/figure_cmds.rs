//! File › Automate › Make Figure: lays several images out as one publication figure, the way
//! journal figures with panels a, b, c are built. The figure is a new document at a print width
//! (a journal column, or any width in mm) and resolution (300 dpi by default).
//!
//! Images of different sizes are matched: in the Rows layout every image in a row is scaled to the
//! same height and the row fills the width exactly, so nothing is cropped; in the Grid layout every
//! image is fitted into equal cells. Gutters (the lines between panels) have a width in mm and a
//! colour. Each image is its own pixel layer, and each panel gets an editable type layer label
//! (a, b, c / A, B, C / (a)…) in a corner, with an optional outline so it reads on any image.

use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{file_name, import, read_file, stem};
use crate::{EngineError, Result, Session};

const CMD: &str = "figure.make";
const MM_PER_INCH: f64 = 25.4;
/// Journal figure widths in mm, as Export for Publication offers them.
const COLUMNS: [(&str, f64); 3] = [("single", 85.0), ("oneAndHalf", 120.0), ("double", 175.0)];
const MAX_PANELS: usize = 48;
/// Largest side of the figure in pixels.
const MAX_SIDE: f64 = 30_000.0;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The figure's options, checked before anything is opened or created.
struct Opts {
    files: Vec<String>,
    columns: usize,
    grid: bool,
    width_px: u32,
    dpi: f64,
    gutter_px: u32,
    gutter: String,
    labels: Option<String>,
    corner: String,
    label_pt: f64,
    label_color: String,
    outline: bool,
    name: String,
}

fn opts(p: &Value) -> Result<Opts> {
    let files: Vec<String> = match p.get("files") {
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad("`files` must be a list of paths"))).collect::<Result<_>>()?
        }
        Some(Value::String(dir)) => crate::file_cmds::list_images(dir)?,
        _ => return Err(bad("missing `files` (a list of image paths, or a folder)")),
    };
    if files.is_empty() {
        return Err(bad("no images to place"));
    }
    if files.len() > MAX_PANELS {
        return Err(bad(format!("at most {MAX_PANELS} images per figure")));
    }
    let num = |k: &str, lo: f64, hi: f64, d: f64| -> Result<f64> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(d),
            Some(v) => {
                v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)).ok_or_else(|| bad(format!("`{k}` must be a number between {lo} and {hi}")))
            }
        }
    };
    let text = |k: &str, d: &str| -> Result<String> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(d.to_string()),
            Some(v) => v.as_str().map(str::to_string).ok_or_else(|| bad(format!("`{k}` must be text"))),
        }
    };
    let default_cols = match files.len() {
        1..=3 => files.len(),
        4 => 2,
        _ => 3,
    };
    let columns = num("columns", 1.0, 12.0, default_cols as f64)?.round() as usize;
    let grid = match text("layout", "rows")?.as_str() {
        "rows" => false,
        "grid" => true,
        o => return Err(bad(format!("unknown layout `{o}` (rows|grid)"))),
    };
    let dpi = num("dpi", 72.0, 2400.0, 300.0)?;
    let width_mm = match text("width", "double")?.as_str() {
        "custom" => num("widthMm", 10.0, 2000.0, 175.0)?,
        w => COLUMNS.iter().find(|(n, _)| *n == w).map(|(_, mm)| *mm).ok_or_else(|| bad(format!("unknown width `{w}` (single|oneAndHalf|double|custom)")))?,
    };
    let width_px = (width_mm / MM_PER_INCH * dpi).round();
    if width_px > MAX_SIDE {
        return Err(bad(format!("{width_mm} mm at {dpi} dpi is wider than {MAX_SIDE} px")));
    }
    let gutter_mm = num("gutterMm", 0.0, 20.0, 1.0)?;
    let gutter = text("gutterColor", "#ffffff")?;
    if !is_hex(&gutter) {
        return Err(bad("`gutterColor` must be #rrggbb"));
    }
    let labels = match text("labels", "abc")?.as_str() {
        "none" => None,
        l @ ("abc" | "ABC" | "(a)" | "(A)" | "a)" | "A)") => Some(l.to_string()),
        o => return Err(bad(format!("unknown labels `{o}` (abc|ABC|(a)|(A)|a)|A)|none)"))),
    };
    let corner = text("labelPosition", "topLeft")?;
    if !matches!(corner.as_str(), "topLeft" | "topRight" | "bottomLeft" | "bottomRight") {
        return Err(bad("`labelPosition` must be topLeft|topRight|bottomLeft|bottomRight"));
    }
    let label_color = text("labelColor", "#ffffff")?;
    if !is_hex(&label_color) {
        return Err(bad("`labelColor` must be #rrggbb"));
    }
    let outline = match p.get("labelOutline") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool().ok_or_else(|| bad("`labelOutline` must be true or false"))?,
    };
    Ok(Opts {
        files,
        columns,
        grid,
        width_px: width_px.max(16.0) as u32,
        dpi,
        gutter_px: (gutter_mm / MM_PER_INCH * dpi).round() as u32,
        gutter,
        labels,
        corner,
        label_pt: num("labelSizePt", 4.0, 72.0, 12.0)?,
        label_color,
        outline,
        name: text("name", "Figure")?,
    })
}

fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// The label of panel `i` (0-based): a, b, … z, aa, ab …
fn label(style: &str, i: usize) -> String {
    let mut n = i;
    let mut s = String::new();
    loop {
        s.insert(0, (b'a' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    match style {
        "ABC" => s.to_uppercase(),
        "(a)" => format!("({s})"),
        "(A)" => format!("({})", s.to_uppercase()),
        "a)" => format!("{s})"),
        "A)" => format!("{})", s.to_uppercase()),
        _ => s,
    }
}

/// Panel rectangles `[x, y, w, h]` for images of `aspects` (width / height), and the figure height.
fn layout(aspects: &[f64], columns: usize, grid: bool, width: u32, gutter: u32) -> (Vec<[u32; 4]>, u32) {
    let (w, g) = (f64::from(width), f64::from(gutter));
    let mut rects = Vec::with_capacity(aspects.len());
    let mut y = 0.0;
    let cols = columns.max(1);
    // Grid cells share one size: the width split evenly, the height of the widest image's shape.
    let cell_w = ((w - (cols as f64 - 1.0) * g) / cols as f64).max(1.0);
    let cell_h = (cell_w / aspects.iter().copied().fold(f64::MIN, f64::max).max(0.01)).max(1.0);
    for row in aspects.chunks(cols) {
        if grid {
            for (i, a) in row.iter().enumerate() {
                let k = (cell_w / a).min(cell_h) / cell_h;
                let (pw, ph) = ((cell_h * k * a).round().max(1.0), (cell_h * k).round().max(1.0));
                let x0 = i as f64 * (cell_w + g) + (cell_w - pw) / 2.0;
                let y0 = y + (cell_h - ph) / 2.0;
                rects.push([x0.round() as u32, y0.round() as u32, pw as u32, ph as u32]);
            }
            y += cell_h.round() + g;
        } else {
            // Same height across the row, filling the width exactly.
            let sum: f64 = row.iter().sum();
            let h = ((w - (row.len() as f64 - 1.0) * g) / sum.max(0.01)).max(1.0).round();
            let mut x = 0.0;
            for (i, a) in row.iter().enumerate() {
                let pw = if i + 1 == row.len() { (w - x).max(1.0) } else { (h * a).round().max(1.0) };
                rects.push([x.round() as u32, y.round() as u32, pw as u32, h as u32]);
                x += pw + g;
            }
            y += h + g;
        }
    }
    (rects, (y - g).max(1.0).round() as u32)
}

/// One image flattened and resampled to `w` x `h`, as straight RGBA rows.
fn panel_pixels(path: &str, w: u32, h: u32) -> Result<photocraft_compose::Buffer> {
    let bytes = read_file(path)?;
    let src = import(&file_name(path), &bytes)?;
    let mut t = Session::new();
    t.add_document(src, None);
    t.execute("image.imageSize", json!({"width": w, "height": h, "resample": "automatic"}))?;
    let d = &t.active().ok_or(EngineError::NoDocument)?.doc;
    Ok(photocraft_compose::flatten(d))
}

fn make_figure(s: &mut Session, p: &Value) -> Result<Value> {
    let o = opts(p)?;
    // Each image's shape first, so the layout is known before the figure is created.
    let mut aspects = Vec::with_capacity(o.files.len());
    for f in &o.files {
        let bytes = read_file(f)?;
        let d = import(&file_name(f), &bytes).map_err(|e| bad(format!("{f}: {e}")))?;
        aspects.push(f64::from(d.size.width.max(1)) / f64::from(d.size.height.max(1)));
    }
    let (rects, height) = layout(&aspects, o.columns, o.grid, o.width_px, o.gutter_px);
    if f64::from(height) > MAX_SIDE {
        return Err(bad(format!("the figure would be {height} px tall; use more columns or a smaller width")));
    }
    s.execute("file.new", json!({"width": o.width_px, "height": height, "background": o.gutter, "resolution": o.dpi, "name": o.name}))?;
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let label_px = o.label_pt / 72.0 * o.dpi;
    let mut panels = Vec::new();
    for (i, (path, r)) in o.files.iter().zip(&rects).enumerate() {
        let [x, y, w, h] = *r;
        let buf = panel_pixels(path, w, h).map_err(|e| bad(format!("{path}: {e}")))?;
        let n = fmt.channels();
        let mut data = vec![0.0f32; buf.px.len() * n];
        for (px, out) in buf.px.iter().zip(data.chunks_exact_mut(n)) {
            photocraft_raster::from_rgba_into(&fmt, *px, out);
        }
        let mut surf = photocraft_doc::Surface::new(fmt);
        let (x0, y0) = (x as i32, y as i32);
        surf.write_region(Rect::new(x0, y0, x0 + w as i32, y0 + h as i32), &data);
        let name = stem(path);
        let id = s.edit("Make Figure", |doc, active| {
            let l = Layer::new(name.clone(), LayerContent::Raster(surf));
            let id = l.id;
            *active = Some(id);
            doc.layers.push(l);
            Ok(id)
        })?;
        let mut label_id = Value::Null;
        if let Some(style) = &o.labels {
            let text = label(style, i);
            let margin = (label_px * 0.35).max(2.0);
            let (right, bottom) = (o.corner.ends_with("Right"), o.corner.starts_with("bottom"));
            let lx = if right { f64::from(x + w) - margin } else { f64::from(x) + margin };
            let ly = if bottom { f64::from(y + h) - margin - label_px * 0.2 } else { f64::from(y) + margin + label_px * 0.75 };
            let t = s.execute(
                "type.create",
                json!({"x": lx, "y": ly, "text": text, "size": o.label_pt, "color": o.label_color, "fauxBold": true, "align": if right { "right" } else { "left" }, "name": format!("Label {text}")}),
            )?;
            label_id = t["layer"].clone();
            if o.outline
                && let Some(lid) = label_id.as_u64()
            {
                let outline_color = if o.label_color.eq_ignore_ascii_case("#000000") { "#ffffff" } else { "#000000" };
                s.execute(
                    "layer.layerStyle.stroke",
                    json!({"layer": lid, "size": (label_px * 0.06).max(1.0).round(), "position": "outside", "color": outline_color}),
                )?;
            }
        }
        panels.push(json!({"file": path, "layer": id.0, "rect": [x, y, w, h], "label": label_id}));
    }
    let size_mm = |px: u32| (f64::from(px) / o.dpi * MM_PER_INCH * 10.0).round() / 10.0;
    Ok(json!({"width": o.width_px, "height": height, "dpi": o.dpi, "widthMm": size_mm(o.width_px), "heightMm": size_mm(height), "panels": panels}))
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Make Figure…",
        menu: &["File", "Automate"],
        shortcut: None,
        params: r##"{"files":[paths]|folder (1..48 images, in panel order),"columns":1..12 (default by count),"layout":"rows|grid"="rows" (rows: equal height per row, filling the width, no cropping; grid: equal cells),"width":"single|oneAndHalf|double|custom"="double" (85, 120, 175 mm),"widthMm":mm (custom),"dpi":72..2400=300,"gutterMm":0..20=1,"gutterColor":"#rrggbb"="#ffffff","labels":"abc|ABC|(a)|(A)|a)|A)|none"="abc","labelPosition":"topLeft|topRight|bottomLeft|bottomRight"="topLeft","labelSizePt":4..72=12,"labelColor":"#rrggbb"="#ffffff","labelOutline":bool=true,"name":str="Figure"} → {width,height,dpi,widthMm,heightMm,panels:[{file,layer,rect,label}]}. Opens a new document"##,
        enabled: always,
        run: make_figure,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
