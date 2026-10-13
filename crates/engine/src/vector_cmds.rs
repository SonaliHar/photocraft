//! Vector commands: shape layers (`shape.*`), document paths (`path.*`, Paths panel), selection ⇄
//! path, and layer vector masks (`layer.vectorMask.*`).
//!
//! Coordinates are document pixels. Paths in parameters and results use a compact JSON form:
//!
//! ```json
//! {"subpaths":[{"closed":true,"op":"combine|subtract|intersect|exclude",
//!               "knots":[[x,y], {"anchor":[x,y],"in":[x,y],"out":[x,y],"smooth":true}, …]}],
//!  "fillRule":"nonzero|evenodd", "inverted":false}
//! ```
//!
//! A knot given as `[x, y]` is a corner with retracted handles; `in`/`out` default to the
//! anchor. The serde form of `photocraft_doc::Path` is accepted too. Colours are `"#rrggbb"`,
//! `"#rrggbbaa"` or `[r,g,b,a]` (0..1); opacities are percentages (0..100).
//!
//! Every edit re-renders the shape layer's cache with `photocraft-vector` in one undoable step.
//! PSD blocks stay byte-identical while they still decode to the model (see `photocraft-io`).

use photocraft_algo::selection as sel;
use photocraft_color::Color;
#[cfg(test)]
use photocraft_color::BlendMode;
use photocraft_doc::{
    Document, Fill, FillRule, GradientStyle, Layer, LayerContent, LayerId, LineCap, LineJoin, LiveShape, Path, PathOp,
    ShapeLayer, ShapeStroke, StrokeAlign, VectorMask,
};
#[cfg(test)]
use photocraft_doc::NamedPath;
use photocraft_geom::{Affine, Point, Rect};
use photocraft_vector as vector;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::{EngineError, Result, Session};

/// Keys PSD uses for a shape layer's vector data; dropped when a shape is rasterized.
const SHAPE_BLOCKS: [&[u8; 4]; 8] = [b"vmsk", b"vsms", b"vogk", b"vstk", b"vscg", b"SoCo", b"GdFl", b"PtFl"];

pub(crate) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

pub(crate) fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.filter(|id| d.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}

fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}

fn active_shape(s: &Session) -> std::result::Result<&ShapeLayer, String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    let layer = d.doc.layer(id).ok_or("no active layer")?;
    match &layer.content {
        LayerContent::Shape(shape) => Ok(shape),
        _ => Err("active layer is not a shape layer".into()),
    }
}

fn has_shape_fill(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?.fill.as_ref().map(|_| ()).ok_or_else(|| "shape has no fill".into())
}

fn has_shape_stroke(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?.stroke.as_ref().map(|_| ()).ok_or_else(|| "shape has no stroke".into())
}

fn can_paste_shape_fill(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?;
    s.path_fill_clipboard.as_ref().map(|_| ()).ok_or_else(|| "no shape fill copied".into())
}

fn can_paste_shape_stroke(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?;
    s.path_stroke_clipboard.as_ref().map(|_| ()).ok_or_else(|| "no shape stroke copied".into())
}

fn copy_shape_fill(s: &mut Session, _p: &Value) -> Result<Value> {
    let fill = active_shape(s).map_err(EngineError::Other)?.fill.clone().ok_or_else(|| EngineError::Other("shape has no fill".into()))?;
    s.path_fill_clipboard = Some(fill);
    Ok(Value::Null)
}

fn copy_shape_stroke(s: &mut Session, _p: &Value) -> Result<Value> {
    let stroke = active_shape(s).map_err(EngineError::Other)?.stroke.clone().ok_or_else(|| EngineError::Other("shape has no stroke".into()))?;
    s.path_stroke_clipboard = Some(stroke);
    Ok(Value::Null)
}

fn paste_shape_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let fill = s.path_fill_clipboard.clone().ok_or_else(|| EngineError::Other("no shape fill copied".into()))?;
    let id = layer_id(s, p)?;
    with_shape(s, id, "Paste Fill", |shape, _| {
        shape.fill = Some(fill);
        Ok(())
    })?;
    shape_info(s, id)
}

fn paste_shape_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let stroke = s.path_stroke_clipboard.clone().ok_or_else(|| EngineError::Other("no shape stroke copied".into()))?;
    let id = layer_id(s, p)?;
    with_shape(s, id, "Paste Complete Stroke", |shape, _| {
        shape.stroke = Some(stroke);
        Ok(())
    })?;
    shape_info(s, id)
}

pub(crate) fn layer_id(s: &Session, p: &Value) -> Result<LayerId> {
    match p.get("layer") {
        Some(value) => {
            let id = match value {
                Value::Number(number) => number.as_u64(),
                Value::String(text)
                    if !text.is_empty() && text.len() <= 20 && !text.starts_with('0') && text.bytes().all(|byte| byte.is_ascii_digit()) =>
                {
                    text.parse::<u64>().ok()
                }
                _ => None,
            };
            id.filter(|id| *id != 0)
                .map(LayerId)
                .ok_or_else(|| bad("vector", "layer must be a nonzero canonical decimal string or unsigned integer"))
        }
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}

fn f64p(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64)
}

pub(crate) fn pt(v: &Value) -> Option<Point> {
    match v {
        Value::Array(a) if a.len() >= 2 => Some(Point::new(a[0].as_f64()?, a[1].as_f64()?)),
        Value::Object(_) => Some(Point::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?)),
        _ => None,
    }
}

pub(crate) fn nums<const N: usize>(p: &Value, k: &str) -> Option<[f64; N]> {
    let a = p.get(k)?.as_array()?;
    if a.len() < N {
        return None;
    }
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(a) {
        *o = v.as_f64()?;
    }
    Some(out)
}

/// `[tl, tr, br, bl]` from an array or a single number.
fn radii(p: &Value) -> Option<[f64; 4]> {
    match p.get("radii").or_else(|| p.get("radius"))? {
        Value::Number(n) => n.as_f64().map(|r| [r; 4]),
        _ => nums::<4>(p, "radii"),
    }
}

pub(crate) fn color(v: &Value) -> Option<Color> {
    match v {
        Value::Array(a) if a.len() >= 3 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            Some(Color::rgba(c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)))
        }
        Value::String(s) => {
            let s = s.trim_start_matches('#');
            let h = |i: usize| s.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match s.len() {
                6 => Some(Color::rgb(h(0)?, h(2)?, h(4)?)),
                8 => Some(Color::rgba(h(0)?, h(2)?, h(4)?, h(6)?)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn hex(c: &Color) -> String {
    let [r, g, b] = c.to_rgb();
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c.alpha < 1.0 { format!("#{:02x}{:02x}{:02x}{:02x}", q(r), q(g), q(b), q(c.alpha)) } else { format!("#{:02x}{:02x}{:02x}", q(r), q(g), q(b)) }
}

fn op_from(s: &str) -> Option<PathOp> {
    Some(match s.to_ascii_lowercase().as_str() {
        "combine" | "add" | "union" => PathOp::Combine,
        "subtract" | "minus" => PathOp::Subtract,
        "intersect" => PathOp::Intersect,
        "exclude" | "xor" => PathOp::Exclude,
        "join" => PathOp::Join,
        _ => return None,
    })
}

fn op_name(op: PathOp) -> &'static str {
    match op {
        PathOp::Combine => "combine",
        PathOp::Subtract => "subtract",
        PathOp::Intersect => "intersect",
        PathOp::Exclude => "exclude",
        PathOp::Join => "join",
    }
}

#[path = "vector_path_input.rs"]
mod path_input;

/// Borrowed path admission before dispatcher parameter copying or path allocation.
pub fn preflight_path_value(v: &Value) -> std::result::Result<(), String> {
    path_input::preflight(v)
}

/// Parses bounded compact or serde paths using fallible owned vectors.
pub fn parse_path(v: &Value) -> std::result::Result<Path, String> {
    path_input::decode(v)
}

/// The compact JSON form of a path.
pub fn path_json(p: &Path) -> Value {
    let a = |q: Point| json!([q.x, q.y]);
    json!({
        "subpaths": p.subpaths.iter().map(|s| json!({
            "closed": s.closed,
            "op": op_name(s.op),
            "knots": s.knots.iter().map(|k| json!({ "anchor": a(k.anchor), "in": a(k.in_ctrl), "out": a(k.out_ctrl), "smooth": k.smooth })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "fillRule": if p.fill_rule == FillRule::EvenOdd { "evenodd" } else { "nonzero" },
        "inverted": p.inverted,
    })
}

fn fill_json(f: &Fill) -> Value {
    match f {
        Fill::Solid(c) => json!(hex(c)),
        Fill::Gradient { stops, angle, scale, style, reverse, .. } => json!({ "gradient": {
            "stops": stops.iter().map(|(t, c)| json!([t, hex(c)])).collect::<Vec<_>>(),
            "angle": angle, "scale": scale * 100.0, "style": format!("{style:?}").to_ascii_lowercase(), "reverse": reverse } }),
        Fill::Pattern { name, scale, .. } => json!({ "pattern": name, "scale": scale * 100.0 }),
    }
}

/// `"#rrggbb"` / `[r,g,b,a]` / `{"gradient":{"stops":[[t,"#hex"],…],"angle","scale","style","reverse"}}` /
/// `{"pattern":name,"scale"}`. `null` → `Ok(None)` (no fill).
fn parse_fill(v: &Value) -> std::result::Result<Option<Fill>, String> {
    if v.is_null() {
        return Ok(None);
    }
    if let Some(c) = color(v) {
        return Ok(Some(Fill::Solid(c)));
    }
    if let Some(g) = v.get("gradient") {
        let stops: Vec<(f32, Color)> = g
            .get("stops")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| Some((s.get(0)?.as_f64()? as f32, color(s.get(1)?)?))).collect())
            .unwrap_or_else(|| vec![(0.0, Color::BLACK), (1.0, Color::WHITE)]);
        let style = match g.get("style").and_then(Value::as_str).unwrap_or("linear") {
            "radial" => GradientStyle::Radial,
            "angle" => GradientStyle::Angle,
            "reflected" => GradientStyle::Reflected,
            "diamond" => GradientStyle::Diamond,
            _ => GradientStyle::Linear,
        };
        return Ok(Some(Fill::gradient(
            stops,
            f64p(g, "angle").unwrap_or(90.0) as f32,
            f64p(g, "scale").unwrap_or(100.0) as f32 / 100.0,
            style,
            g.get("reverse").and_then(Value::as_bool).unwrap_or(false),
        )));
    }
    if let Some(name) = v.get("pattern").and_then(Value::as_str) {
        return Ok(Some(Fill::Pattern {
            name: name.into(),
            scale: f64p(v, "scale").unwrap_or(100.0) as f32 / 100.0,
            id: String::new(),
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        }));
    }
    Err(format!("unrecognised fill {v}"))
}

fn stroke_json(s: &ShapeStroke) -> Value {
    json!({
        "width": s.width, "fill": fill_json(&s.paint), "opacity": s.opacity * 100.0,
        "align": format!("{:?}", s.align).to_ascii_lowercase(), "cap": format!("{:?}", s.cap).to_ascii_lowercase(),
        "join": format!("{:?}", s.join).to_ascii_lowercase(), "miterLimit": s.miter_limit, "dashes": s.dashes, "dashOffset": s.dash_offset,
    })
}

/// Applies stroke keys onto `base` (`null` → no stroke).
fn parse_stroke(v: &Value, base: Option<ShapeStroke>, fg: Color) -> std::result::Result<Option<ShapeStroke>, String> {
    if v.is_null() || v.as_bool() == Some(false) {
        return Ok(None);
    }
    let mut s = base.unwrap_or(ShapeStroke { paint: Fill::Solid(fg), ..Default::default() });
    if let Some(w) = f64p(v, "width") {
        s.width = w.max(0.0) as f32;
    }
    if let Some(c) = v.get("color").or_else(|| v.get("fill")) {
        s.paint = parse_fill(c)?.ok_or("stroke colour cannot be null")?;
    }
    if let Some(o) = f64p(v, "opacity") {
        s.opacity = (o / 100.0).clamp(0.0, 1.0) as f32;
    }
    if let Some(a) = v.get("align").and_then(Value::as_str) {
        s.align = match a {
            "inside" => StrokeAlign::Inside,
            "outside" => StrokeAlign::Outside,
            "center" | "centre" => StrokeAlign::Center,
            o => return Err(format!("unknown align `{o}`")),
        };
    }
    if let Some(c) = v.get("cap").and_then(Value::as_str) {
        s.cap = match c {
            "butt" => LineCap::Butt,
            "round" => LineCap::Round,
            "square" => LineCap::Square,
            o => return Err(format!("unknown cap `{o}`")),
        };
    }
    if let Some(j) = v.get("join").and_then(Value::as_str) {
        s.join = match j {
            "miter" => LineJoin::Miter,
            "round" => LineJoin::Round,
            "bevel" => LineJoin::Bevel,
            o => return Err(format!("unknown join `{o}`")),
        };
    }
    if let Some(m) = f64p(v, "miterLimit") {
        s.miter_limit = m.max(1.0) as f32;
    }
    if let Some(d) = v.get("dashes") {
        s.dashes = match d {
            Value::Null => Vec::new(),
            Value::Array(a) => a.iter().filter_map(Value::as_f64).map(|x| x.max(0.0) as f32).collect(),
            _ => return Err("`dashes` must be an array (multiples of the width)".into()),
        };
    }
    if let Some(o) = f64p(v, "dashOffset") {
        s.dash_offset = o as f32;
    }
    Ok(Some(s))
}

fn live_json(l: &LiveShape) -> Value {
    serde_json::to_value(l).unwrap_or(Value::Null)
}

/// Builds a live shape from `kind` + geometry params.
fn live_from_params(cmd: &str, kind: &str, p: &Value, prev: Option<&LiveShape>) -> Result<LiveShape> {
    let prev_rect = match prev {
        Some(LiveShape::Rect { rect, .. } | LiveShape::Ellipse { rect } | LiveShape::Polygon { rect, .. }) => Some(*rect),
        _ => None,
    };
    let rect = || nums::<4>(p, "rect").or(prev_rect).ok_or_else(|| bad(cmd, "`rect`: [x, y, w, h] is required"));
    Ok(match kind {
        "rect" | "rectangle" => {
            let r = match prev {
                Some(LiveShape::Rect { radii, .. }) => *radii,
                _ => [0.0; 4],
            };
            LiveShape::Rect { rect: rect()?, radii: radii(p).unwrap_or(r) }
        }
        "roundedRect" | "roundRect" => {
            let r = match prev {
                Some(LiveShape::Rect { radii, .. }) if radii.iter().any(|v| *v > 0.0) => *radii,
                _ => [10.0; 4],
            };
            LiveShape::Rect { rect: rect()?, radii: radii(p).unwrap_or(r) }
        }
        "ellipse" => LiveShape::Ellipse { rect: rect()? },
        "polygon" | "star" => {
            let (ps, pr) = match prev {
                Some(LiveShape::Polygon { sides, star_ratio, .. }) => (*sides, *star_ratio),
                _ => (5, if kind == "star" { 0.5 } else { 1.0 }),
            };
            let sides = f64p(p, "sides").map_or(ps, |v| v.round().clamp(3.0, 100.0) as u32);
            let ratio = f64p(p, "starRatio").map_or(if kind == "star" && pr >= 1.0 { 0.5 } else { pr }, |v| v.clamp(0.01, 1.0));
            LiveShape::Polygon { rect: rect()?, sides, star_ratio: if kind == "polygon" && f64p(p, "starRatio").is_none() { 1.0 } else { ratio } }
        }
        "line" => {
            let (pf, pto, pw) = match prev {
                Some(LiveShape::Line { from, to, weight }) => (Some(*from), Some(*to), *weight),
                _ => (None, None, 1.0),
            };
            LiveShape::Line {
                from: nums::<2>(p, "from").or(pf).ok_or_else(|| bad(cmd, "`from`: [x, y] is required"))?,
                to: nums::<2>(p, "to").or(pto).ok_or_else(|| bad(cmd, "`to`: [x, y] is required"))?,
                weight: f64p(p, "weight").unwrap_or(pw).max(0.0),
            }
        }
        o => return Err(bad(cmd, format!("unknown kind `{o}`"))),
    })
}

/// The path `shape.create` draws for live-shape params `p` (the Shape tools preview it while dragging).
pub fn shape_path(p: &Value) -> Result<Path> {
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("rect");
    Ok(vector::shapes::live_path(&live_from_params("shape.create", kind, p, None)?))
}

fn live_kind(l: &LiveShape) -> &'static str {
    match l {
        LiveShape::Rect { radii, .. } if radii.iter().any(|r| *r > 0.0) => "roundedRect",
        LiveShape::Rect { .. } => "rect",
        LiveShape::Ellipse { .. } => "ellipse",
        LiveShape::Polygon { star_ratio, .. } if *star_ratio < 1.0 => "star",
        LiveShape::Polygon { .. } => "polygon",
        LiveShape::Line { .. } => "line",
    }
}

/// Re-renders a shape layer's cache for `doc` (canvas-clipped, document pixel format).
pub fn refresh_shape(doc: &Document, sh: &mut ShapeLayer) {
    sh.cache = Some(vector::render_shape(sh, doc.pixel_format(), doc.bounds()));
}

/// Moves a layer's vector content by `(dx, dy)`: shape paths (re-rendered) and linked vector
/// masks, recursing into groups. For the Move tool / `layer.move`.
pub fn translate_vectors(doc: &Document, l: &mut Layer, dx: f64, dy: f64) {
    let a = Affine::translate(dx, dy);
    if let Some(vm) = l.vector_mask.as_mut()
        && vm.linked
    {
        vm.path = vm.path.transform(&a);
    }
    match &mut l.content {
        LayerContent::Shape(sh) => {
            transform_shape(sh, &a);
            refresh_shape(doc, sh);
        }
        LayerContent::Group(g) => {
            for c in &mut g.children {
                translate_vectors(doc, c, dx, dy);
            }
        }
        _ => {}
    }
}

/// Applies an affine transform to a shape's path, keeping the live shape when the transform
/// is a translation plus axis-aligned scale (otherwise the shape stops being parametric).
pub fn transform_shape(sh: &mut ShapeLayer, a: &Affine) {
    sh.path = sh.path.transform(a);
    let [m0, m1, m2, m3, tx, ty] = a.m;
    let axis_aligned = m1.abs() < 1e-12 && m2.abs() < 1e-12 && m0 > 0.0 && m3 > 0.0;
    sh.live = match (sh.live.take(), axis_aligned) {
        (Some(l), true) => {
            let r = |r: [f64; 4]| [r[0] * m0 + tx, r[1] * m3 + ty, r[2] * m0, r[3] * m3];
            let pt = |p: [f64; 2]| [p[0] * m0 + tx, p[1] * m3 + ty];
            Some(match l {
                LiveShape::Rect { rect, radii } => {
                    let k = m0.min(m3);
                    LiveShape::Rect { rect: r(rect), radii: radii.map(|v| v * k) }
                }
                LiveShape::Ellipse { rect } => LiveShape::Ellipse { rect: r(rect) },
                LiveShape::Polygon { rect, sides, star_ratio } => LiveShape::Polygon { rect: r(rect), sides, star_ratio },
                LiveShape::Line { from, to, weight } => LiveShape::Line { from: pt(from), to: pt(to), weight },
            })
        }
        // Flips and 90° turns keep rectangles, ellipses and lines live (Image Rotation): the new
        // live box is the mapped box. Polygons/stars would change orientation, so they don't.
        (Some(l), false) if (m1.abs() < 1e-12 && m2.abs() < 1e-12) || (m0.abs() < 1e-12 && m3.abs() < 1e-12) => {
            let pt = |p: [f64; 2]| [m0 * p[0] + m2 * p[1] + tx, m1 * p[0] + m3 * p[1] + ty];
            let r = |r: [f64; 4]| {
                let (p, q) = (pt([r[0], r[1]]), pt([r[0] + r[2], r[1] + r[3]]));
                [p[0].min(q[0]), p[1].min(q[1]), (q[0] - p[0]).abs(), (q[1] - p[1]).abs()]
            };
            let k = (m0.abs() + m1.abs()).min(m2.abs() + m3.abs());
            match l {
                LiveShape::Rect { rect, radii } if radii.iter().all(|v| (v - radii[0]).abs() < 1e-9) => {
                    Some(LiveShape::Rect { rect: r(rect), radii: radii.map(|v| v * k) })
                }
                LiveShape::Ellipse { rect } => Some(LiveShape::Ellipse { rect: r(rect) }),
                LiveShape::Line { from, to, weight } => Some(LiveShape::Line { from: pt(from), to: pt(to), weight: weight * k }),
                _ => None,
            }
        }
        _ => None,
    };
    sh.psd_raw = None;
}

fn shape_info(s: &Session, id: LayerId) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Shape(sh) = &l.content else {
        return Err(EngineError::Other(format!("layer {} is {} {} layer, not a shape layer", id.0, l.content.article(), l.content.kind_name())));
    };
    let bounds = sh.cache.as_ref().map(|c| c.content_bounds()).filter(|r| !r.is_empty()).map(|r| json!([r.x0, r.y0, r.width(), r.height()]));
    Ok(json!({
        "layer": id.0,
        "name": l.name,
        "path": path_json(&sh.path),
        "fill": sh.fill.as_ref().map(fill_json),
        "stroke": sh.stroke.as_ref().map(stroke_json),
        "live": sh.live.as_ref().map(live_json),
        "kind": sh.live.as_ref().map_or("path", live_kind),
        "bounds": bounds,
    }))
}

pub(crate) fn with_shape<R>(s: &mut Session, id: LayerId, label: &str, f: impl FnOnce(&mut ShapeLayer, &mut Layer) -> Result<R>) -> Result<R> {
    s.edit(label, |doc, _| {
        let snapshot = doc.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if !matches!(l.content, LayerContent::Shape(_)) {
            return Err(EngineError::Other(format!("layer {} is {} {} layer, not a shape layer", id.0, l.content.article(), l.content.kind_name())));
        }
        let LayerContent::Shape(mut sh) = std::mem::replace(&mut l.content, LayerContent::Fill(Fill::Solid(Color::BLACK))) else {
            return Err(EngineError::Other(format!("layer {} is not a shape layer", id.0)));
        };
        let r = f(&mut sh, l);
        refresh_shape(&snapshot, &mut sh);
        l.content = LayerContent::Shape(sh);
        r
    })
}

fn default_name(kind: &str) -> &'static str {
    match kind {
        "rect" | "rectangle" => "Rectangle",
        "roundedRect" | "roundRect" => "Rounded Rectangle",
        "ellipse" => "Ellipse",
        "polygon" => "Polygon",
        "star" => "Star",
        "line" => "Line",
        _ => "Shape",
    }
}

fn shape_create(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.create";
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("rect");
    let (path, live) = if kind == "path" {
        let v = p.get("path").ok_or_else(|| bad(CMD, "kind \"path\" needs `path`"))?;
        (parse_path(v).map_err(|e| bad(CMD, e))?, None)
    } else {
        let l = live_from_params(CMD, kind, p, None)?;
        (vector::shapes::live_path(&l), Some(l))
    };
    let fg = s.tools.foreground;
    let fgc = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    let fill = match p.get("fill") {
        Some(v) => parse_fill(v).map_err(|e| bad(CMD, e))?,
        None => Some(Fill::Solid(fgc)),
    };
    let stroke = match p.get("stroke") {
        Some(v) => parse_stroke(v, None, fgc).map_err(|e| bad(CMD, e))?,
        None => None,
    };
    // Add to an existing shape layer with a path operation (Shape tool in combine/subtract… mode).
    if let Some(target) = p.get("addTo").and_then(Value::as_u64) {
        let op = p.get("op").and_then(Value::as_str).map_or(Some(PathOp::Combine), op_from).ok_or_else(|| bad(CMD, "unknown `op`"))?;
        let id = LayerId(target);
        with_shape(s, id, "Add Shape", |sh, _| {
            for mut sp in path.subpaths {
                sp.op = op;
                sh.path.subpaths.push(sp);
            }
            sh.live = None;
            sh.psd_raw = None;
            Ok(())
        })?;
        return shape_info(s, id);
    }
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let mut sh = ShapeLayer { path, fill, stroke, live, ..Default::default() };
    let id = s.edit("New Shape Layer", |doc, active| {
        refresh_shape(doc, &mut sh);
        let name = name.unwrap_or_else(|| doc.next_layer_name(default_name(kind)));
        let id = doc.insert_above(*active, Layer::new(name, LayerContent::Shape(sh)));
        *active = Some(id);
        Ok(id)
    })?;
    shape_info(s, id)
}

fn shape_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.edit";
    let id = layer_id(s, p)?;
    let fg = s.tools.foreground;
    let fgc = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    with_shape(s, id, "Edit Shape", |sh, layer| {
        if let Some(v) = p.get("path") {
            sh.path = parse_path(v).map_err(|e| bad(CMD, e))?;
            sh.live = None;
            sh.psd_raw = None;
        }
        let geom_keys = ["kind", "rect", "radii", "radius", "sides", "starRatio", "from", "to", "weight"];
        if geom_keys.iter().any(|k| p.get(*k).is_some()) {
            let kind = p.get("kind").and_then(Value::as_str).map(str::to_string).or_else(|| sh.live.as_ref().map(|l| live_kind(l).to_string()));
            let kind = kind.ok_or_else(|| bad(CMD, "this shape has no live-shape parameters; pass `kind` or `path`"))?;
            // Explicit radii on a plain rectangle keep it a (now rounded) rectangle.
            let kind = if kind == "rect" && radii(p).is_some_and(|r| r.iter().any(|v| *v > 0.0)) { "roundedRect".to_string() } else { kind };
            let l = live_from_params(CMD, &kind, p, sh.live.as_ref())?;
            sh.path = vector::shapes::live_path(&l);
            sh.live = Some(l);
            sh.psd_raw = None;
        }
        if let Some(m) = nums::<6>(p, "transform") {
            transform_shape(sh, &Affine { m });
        }
        if let Some([dx, dy]) = nums::<2>(p, "move") {
            transform_shape(sh, &Affine::translate(dx, dy));
        }
        if let Some(op) = p.get("op").and_then(Value::as_str) {
            // Operation of the selected subpath(s) (`subpath`: index, default all but the first).
            let op = op_from(op).ok_or_else(|| bad(CMD, format!("unknown op `{op}`")))?;
            match p.get("subpath").and_then(Value::as_u64) {
                Some(i) => sh.path.subpaths.get_mut(i as usize).ok_or_else(|| bad(CMD, "no such subpath"))?.op = op,
                None => sh.path.subpaths.iter_mut().skip(1).for_each(|sp| sp.op = op),
            }
            sh.live = None;
        }
        if let Some(r) = p.get("fillRule").and_then(Value::as_str) {
            sh.path.fill_rule = if r.eq_ignore_ascii_case("evenodd") { FillRule::EvenOdd } else { FillRule::NonZero };
        }
        if let Some(v) = p.get("fill") {
            sh.fill = parse_fill(v).map_err(|e| bad(CMD, e))?;
        }
        if let Some(v) = p.get("stroke") {
            sh.stroke = parse_stroke(v, sh.stroke.clone(), fgc).map_err(|e| bad(CMD, e))?;
        }
        if let Some(n) = name {
            layer.name = n;
        }
        Ok(())
    })?;
    shape_info(s, id)
}

fn shape_rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    s.edit("Rasterize Shape", |doc, _| {
        let snapshot = doc.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let LayerContent::Shape(sh) = &mut l.content else {
            return Err(EngineError::Other(format!("layer {} is not a shape layer", id.0)));
        };
        if sh.cache.is_none() {
            refresh_shape(&snapshot, sh);
        }
        let fmt = snapshot.pixel_format();
        let surface = sh.cache.take().unwrap_or_else(|| photocraft_raster::Surface::new(fmt));
        let surface = if surface.format() == fmt {
            surface
        } else {
            surface.try_convert(fmt).map_err(|e| crate::EngineError::Other(format!("pixel conversion: {e}")))?
        };
        l.content = LayerContent::Raster(surface);
        l.psd_blocks.retain(|(k, _)| !SHAPE_BLOCKS.contains(&k));
        Ok(())
    })?;
    Ok(json!({ "layer": id.0 }))
}

// ---------------------------------------------------------------------------
// Document paths
// ---------------------------------------------------------------------------

pub(crate) fn is_work(name: Option<&str>) -> bool {
    name.is_none_or(|n| n.is_empty() || n.eq_ignore_ascii_case("work") || n == "Work Path")
}

/// The path a command targets: `"work"`/absent → the work path (falling back to the active
/// layer's shape path or vector mask when there is none), `"layer"` → the active layer's,
/// otherwise a saved path by name.
fn resolve_path(s: &Session, name: Option<&str>) -> Result<Path> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let layer_path = || -> Option<Path> {
        let l = d.doc.layer(d.active_layer?)?;
        match &l.content {
            LayerContent::Shape(sh) => Some(sh.path.clone()),
            _ => l.vector_mask.as_ref().map(|v| v.path.clone()),
        }
    };
    if name == Some("layer") {
        return layer_path().ok_or_else(|| EngineError::Other("active layer has no shape path or vector mask".into()));
    }
    if is_work(name) {
        return d.doc.work_path.clone().or_else(layer_path).ok_or_else(|| EngineError::Other("no work path".into()));
    }
    let n = name.unwrap_or_default();
    d.doc.paths.iter().find(|p| p.name == n).map(|p| p.path.clone()).ok_or_else(|| EngineError::Other(format!("no path named \"{n}\"")))
}

pub(crate) fn saved_path_id(params: &Value) -> Result<Option<photocraft_doc::item_identity::PathId>> {
    params
        .get("pathId")
        .map(|value| {
            let id = match value {
                Value::String(text) if !text.is_empty() && text.len() <= 20 && !text.starts_with('0') && text.bytes().all(|byte| byte.is_ascii_digit()) => text.parse::<u64>().ok(),
                Value::Number(number) => number.as_u64(),
                _ => None,
            };
            id.filter(|id| *id != 0)
                .map(photocraft_doc::item_identity::PathId)
                .ok_or_else(|| bad("path", "pathId must be a nonzero canonical decimal string or unsigned integer"))
        })
        .transpose()
}
fn resolve_path_params(s: &Session, params: &Value) -> Result<Path> {
    if let Some(id) = saved_path_id(params)? {
        return s
            .active()
            .ok_or(EngineError::NoDocument)?
            .doc
            .paths
            .iter()
            .find(|path| path.id == id)
            .map(|path| path.path.clone())
            .ok_or_else(|| bad("path", "no saved path with pathId"));
    }
    resolve_path(s, params.get("name").and_then(Value::as_str))
}

/// The path an edit targets, except a shape layer's (edit those with [`with_shape`], which
/// re-renders them): `layer`'s vector mask when given, else the work path or a saved path.
pub(crate) fn path_mut<'a>(doc: &'a mut Document, name: &str, layer: Option<LayerId>) -> Result<&'a mut Path> {
    if let Some(id) = layer {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        return l.vector_mask.as_mut().map(|vm| &mut vm.path).ok_or_else(|| EngineError::Other("active layer has no shape path or vector mask".into()));
    }
    if is_work(Some(name)) {
        return doc.work_path.as_mut().ok_or_else(|| EngineError::Other("no work path".into()));
    }
    doc.paths.iter_mut().find(|q| q.name == name).map(|q| &mut q.path).ok_or_else(|| EngineError::Other(format!("no path named \"{name}\"")))
}

fn paths_list(s: &Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &d.doc;
    let knots = |p: &Path| p.subpaths.iter().map(|s| s.knots.len()).sum::<usize>();
    let layer = d.active_layer.and_then(|id| doc.layer(id)).and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(json!({ "kind": "shape", "layer": l.id.0, "name": format!("{} Shape Path", l.name), "knots": knots(&sh.path) })),
        _ => {
            l.vector_mask.as_ref().map(|v| json!({ "kind": "vectorMask", "layer": l.id.0, "name": format!("{} Vector Mask", l.name), "knots": knots(&v.path) }))
        }
    });
    Ok(json!({
        "paths": doc.paths.iter().map(|p| json!({ "pathId": p.id.0, "pathIdString": p.id.0.to_string(), "name": p.name, "knots": knots(&p.path), "subpaths": p.path.subpaths.len() })).collect::<Vec<_>>(),
        "workPath": doc.work_path.as_ref().map(|p| json!({ "knots": knots(p), "subpaths": p.subpaths.len() })),
        "clippingPath": doc.clipping_path.as_ref().map(|c| json!({ "name": c.name, "flatness": c.flatness })),
        "layerPath": layer,
    }))
}

mod path_storage_controlled;

pub(crate) fn path_storage_dispatch_preflight(s: &Session, command: &str, p: &Value, background: bool) -> Result<()> {
    path_storage_controlled::dispatch_preflight(s, command, p, background)
}

fn path_set(s: &mut Session, p: &Value) -> Result<Value> {
    path_storage_controlled::run(s, "path.set", p)
}

fn path_delete(s: &mut Session, p: &Value) -> Result<Value> {
    path_storage_controlled::run(s, "path.delete", p)
}

mod clipping_controlled;

pub(crate) fn clipping_dispatch_preflight(s: &Session, command: &str, p: &Value, background: bool) -> Result<()> {
    clipping_controlled::dispatch_preflight(s, command, p, background)
}

fn clipping_path_set(s: &mut Session, p: &Value) -> Result<Value> {
    clipping_controlled::run(s, "path.clippingPath.set", p)
}

fn clipping_path_clear(s: &mut Session, p: &Value) -> Result<Value> {
    clipping_controlled::run(s, "path.clippingPath.clear", p)
}

mod path_transform_controlled;

pub(crate) fn path_transform_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    path_transform_controlled::dispatch_preflight(s, p, background)
}

fn path_transform(s: &mut Session, p: &Value) -> Result<Value> {
    path_transform_controlled::run(s, p)
}

mod path_name_controlled;

pub(crate) fn path_name_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    path_name_controlled::dispatch_preflight(s, "path.rename", p, background).map(|_| ())
}

fn path_rename(s: &mut Session, p: &Value) -> Result<Value> {
    path_name_controlled::path_rename(s, p)
}

mod path_selection_controlled;

pub(crate) fn path_selection_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    path_selection_controlled::dispatch_preflight(s, "path.toSelection", p, background).map(|_| ())
}

fn path_to_selection(s: &mut Session, p: &Value) -> Result<Value> {
    path_selection_controlled::path_to_selection(s, p)
}

mod trace_controlled;

pub(crate) fn trace_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    trace_controlled::dispatch_preflight(s, p, background)
}

fn select_to_work_path(s: &mut Session, p: &Value) -> Result<Value> {
    trace_controlled::run(s, p)
}

mod path_fill_controlled;

pub(crate) fn path_fill_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    path_fill_controlled::dispatch_preflight(s, p, background)
}

fn path_fill(s: &mut Session, p: &Value) -> Result<Value> {
    path_fill_controlled::run(s, p)
}

/// Composites `src` × coverage onto a surface over `area` (any pixel format).
#[cfg(test)]
fn paint_coverage(surf: &mut photocraft_raster::Surface, area: Rect, cov: &[f32], src: [f32; 4], opacity: f32, mode: BlendMode, lock_alpha: bool) {
    let fmt = surf.format();
    let ch = fmt.channels();
    let mut vals = surf.read_region(area);
    for (px, c) in vals.chunks_exact_mut(ch).zip(cov) {
        if *c <= 0.0 {
            continue;
        }
        let dst = photocraft_raster::to_rgba(&fmt, px);
        let mut out = photocraft_compose::psblend::composite(mode, dst, [src[0], src[1], src[2], src[3] * c], opacity);
        if lock_alpha {
            out[3] = dst[3];
        }
        photocraft_raster::from_rgba_into(&fmt, out, px);
    }
    surf.write_region(area, &vals);
    surf.prune();
}

mod path_stroke_controlled;
pub use path_stroke_controlled::brush_snapshot as path_stroke_brush_snapshot;

pub(crate) fn path_stroke_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    path_stroke_controlled::dispatch_preflight(s, p, background)
}

fn path_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    path_stroke_controlled::run(s, p)
}
// ---------------------------------------------------------------------------
// Vector masks
// ---------------------------------------------------------------------------

fn vector_mask_info(s: &Session, id: LayerId) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    Ok(match &l.vector_mask {
        Some(v) => {
            json!({ "layer": id.0, "path": path_json(&v.path), "enabled": v.enabled, "linked": v.linked, "density": v.density * 100.0, "feather": v.feather })
        }
        None => json!({ "layer": id.0, "vectorMask": null }),
    })
}

fn vector_mask_add(s: &mut Session, p: &Value, from_path: bool) -> Result<Value> {
    const CMD: &str = "layer.vectorMask.add";
    let id = layer_id(s, p)?;
    let mut path = match p.get("path") {
        Some(v) => parse_path(v).map_err(|e| bad(CMD, e))?,
        None if from_path || p.get("name").is_some() => {
            let name = p.get("name").and_then(Value::as_str);
            // Never fall back to the layer's own path here.
            if is_work(name) {
                s.active().and_then(|d| d.doc.work_path.clone()).ok_or_else(|| EngineError::Other("no work path".into()))?
            } else {
                resolve_path(s, name)?
            }
        }
        None => Path::default(),
    };
    if p.get("hide").and_then(Value::as_bool).unwrap_or(false) {
        path.inverted = !path.inverted;
    }
    s.edit("Add Vector Mask", |doc, _| {
        let locked = doc.effective_locks(id).all;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if matches!(l.content, LayerContent::Shape(_)) {
            return Err(EngineError::Other("a shape layer's path is already its vector mask".into()));
        }
        if locked {
            return Err(EngineError::Other(format!("layer \"{}\" is locked", l.name)));
        }
        l.vector_mask = Some(VectorMask::new(path));
        Ok(())
    })?;
    vector_mask_info(s, id)
}

fn vector_mask_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.vectorMask.edit";
    let id = layer_id(s, p)?;
    let path = p.get("path").map(parse_path).transpose().map_err(|e| bad(CMD, e))?;
    s.edit("Edit Vector Mask", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let vm = l.vector_mask.as_mut().ok_or_else(|| EngineError::Other("layer has no vector mask".into()))?;
        if let Some(pth) = path {
            vm.path = pth;
        }
        if let Some(b) = p.get("enabled").and_then(Value::as_bool) {
            vm.enabled = b;
        }
        if let Some(b) = p.get("linked").and_then(Value::as_bool) {
            vm.linked = b;
        }
        if let Some(d) = f64p(p, "density") {
            vm.density = (d / 100.0).clamp(0.0, 1.0) as f32;
        }
        if let Some(f) = f64p(p, "feather") {
            vm.feather = f.max(0.0) as f32;
        }
        if let Some(b) = p.get("invert").and_then(Value::as_bool)
            && b
        {
            vm.path.inverted = !vm.path.inverted;
        }
        Ok(())
    })?;
    vector_mask_info(s, id)
}

fn vector_mask_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    s.edit("Delete Vector Mask", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.vector_mask.take().map(|_| ()).ok_or_else(|| EngineError::Other("layer has no vector mask".into()))
    })?;
    Ok(Value::Null)
}

/// Layer › Rasterize › Vector Mask: multiplies the vector mask into the pixel mask.
fn vector_mask_rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    s.edit("Rasterize Vector Mask", |doc, _| {
        let area = doc.bounds();
        let depth = doc.depth;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let vm = l.vector_mask.take().ok_or_else(|| EngineError::Other("layer has no vector mask".into()))?;
        let vals = vector::vector_mask_values(&vm, area);
        let mut mask = l.mask.take().unwrap_or_else(|| {
            let mut m = photocraft_doc::LayerMask::reveal_all();
            m.surface = std::sync::Arc::new(photocraft_raster::Surface::with_default(
                photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, depth, false),
                &[1.0],
            ));
            m
        });
        // Outside the canvas the vector mask is 0 unless it is empty/inverted; keep the old default there.
        let old = mask.surface.read_region(area);
        let merged: Vec<f32> = old.iter().zip(&vals).map(|(a, b)| a * b).collect();
        std::sync::Arc::make_mut(&mut mask.surface).write_region(area, &merged);
        std::sync::Arc::make_mut(&mut mask.surface).prune();
        l.mask = Some(mask);
        Ok(())
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// `p` with `key` set (for alias commands).
fn with(p: &Value, key: &str, v: Value) -> Value {
    let mut o = if p.is_object() { p.clone() } else { json!({}) };
    o[key] = v;
    o
}

fn toggle_vector_mask(s: &mut Session, p: &Value, key: &str) -> Result<Value> {
    let id = layer_id(s, p)?;
    let cur = {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let vm = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?.vector_mask.as_ref().ok_or_else(|| EngineError::Other("layer has no vector mask".into()))?;
        if key == "enabled" { vm.enabled } else { vm.linked }
    };
    let v = p.get(key).and_then(Value::as_bool).unwrap_or(!cur);
    vector_mask_edit(s, &json!({ "layer": id.0, key: v }))
}

/// Traces the exact coverage of the shape's path back into plain outlines.
fn merge_components(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    let tol = f64p(p, "tolerance").unwrap_or(0.1).clamp(0.05, 10.0);
    with_shape(s, id, "Merge Shape Components", |sh, _| {
        // Trace at 4× resolution so the fitted outline follows the anti-aliased edge closely.
        const K: f64 = 4.0;
        let up = sh.path.transform(&Affine::scale(K));
        let r = vector::fill_rasterizer(&up, vector::DEFAULT_TOLERANCE);
        let Some(b) = r.pixel_bounds() else {
            sh.path.subpaths.clear();
            return Ok(());
        };
        let b = b.inflate(2);
        if u64::from(b.width()) * u64::from(b.height()) > 400_000_000 {
            return Err(EngineError::Other("shape too large to merge".into()));
        }
        let cov = r.render(b);
        let traced = vector::trace::trace_mask(&cov, b, 0.0, tol * K);
        sh.path = traced.transform(&Affine::scale(1.0 / K));
        sh.live = None;
        sh.psd_raw = None;
        Ok(())
    })?;
    shape_info(s, id)
}

fn selection_to_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let tol = f64p(p, "tolerance").unwrap_or(2.0).clamp(0.5, 10.0);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    let mask = sel::mask_from_surface(d.doc.selection.as_ref(), area);
    let path = vector::trace::trace_mask(&mask, area, 0.0, tol);
    let mut q = json!({ "kind": "path", "path": path_json(&path) });
    for k in ["fill", "name", "stroke"] {
        if let Some(v) = p.get(k) {
            q[k] = v.clone();
        }
    }
    shape_create(s, &q)
}

// ---------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------

pub(crate) const PATH_FORM: &str = r##"path: {"subpaths":[{"closed":bool=true,"op":"combine|subtract|intersect|exclude","knots":[[x,y] | {"anchor":[x,y],"in":[x,y],"out":[x,y],"smooth":bool}]}],"fillRule":"nonzero|evenodd","inverted":bool}"##;

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub(crate) fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub fn specs() -> Vec<CommandSpec> {
    let stroke = r##""stroke":{"width":px,"color":"#rrggbb"|fill,"opacity":0..100,"align":"inside|center|outside","cap":"butt|round|square","join":"miter|round|bevel","miterLimit":n,"dashes":[multiples of width],"dashOffset":n}|null"##;
    let fill = r##""fill":"#rrggbb"|[r,g,b,a]|{"gradient":{"stops":[[t,"#hex"]],"angle":deg,"scale":%,"style":"linear|radial|angle|reflected|diamond","reverse":bool}}|{"pattern":name}|null (no fill)"##;
    vec![
        spec!(
            "shape.create",
            "New Shape Layer",
            ["Layer", "New"],
            leak(format!(
                r##"{{"kind":"rect|roundedRect|ellipse|polygon|star|line|path"="rect","rect":[x,y,w,h] (rect/roundedRect/ellipse/polygon/star),"radii":[tl,tr,br,bl]|r (roundedRect=10),"sides":3..100=5,"starRatio":0..1 (star=0.5),"from":[x,y],"to":[x,y],"weight":px=1 (line),"path":{{…}} (kind path),{fill}=foreground,{stroke}=none,"name":str?,"addTo":layerId? + "op":"combine|subtract|intersect|exclude" (append to an existing shape layer)}} → shape.info. {PATH_FORM}"##
            )),
            has_doc,
            shape_create
        ),
        spec!(
            "shape.edit",
            "Edit Shape",
            [],
            leak(format!(
                r##"{{"layer":id?,"path":{{…}}? (replaces; drops live shape),"kind":str?,"rect":[x,y,w,h]?,"radii":[tl,tr,br,bl]|r?,"sides":n?,"starRatio":r?,"from":[x,y]?,"to":[x,y]?,"weight":px? (live-shape params regenerate the path),"move":[dx,dy]?,"transform":[a,b,c,d,e,f]?,"op":"combine|…"? + "subpath":index? (default all but the first),"fillRule":"nonzero|evenodd"?,{fill}?,{stroke}? (merged into the current stroke),"name":str?}} → shape.info"##
            )),
            has_layer,
            shape_edit
        ),
        CommandSpec {
            id: "shape.info",
            label: "Shape Layer Info",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?} → {layer,name,kind,path,fill,stroke,live,bounds:[x,y,w,h]}"##,
            enabled: has_layer,
            run: |s, p| {
                let id = layer_id(s, p)?;
                shape_info(s, id)
            },
            journal: false,
        },
        spec!("shape.rasterize", "Rasterize Shape", ["Layer", "Rasterize"], r##"{"layer":id?}"##, has_layer, shape_rasterize),
        CommandSpec {
            id: "path.list",
            label: "List Paths",
            menu: &[],
            shortcut: None,
            params: r##"{} → {paths:[{pathId:legacy u64,pathIdString:canonical string,name,knots,subpaths}],workPath,clippingPath,layerPath (active layer's shape path / vector mask)}"##,
            enabled: has_doc,
            run: |s, _| paths_list(s),
            journal: false,
        },
        CommandSpec {
            id: "path.info",
            label: "Path Info",
            menu: &[],
            shortcut: None,
            params: r##"{"pathId":id? (saved path ID takes precedence),"name":str|"work"|"layer"="work"} → {name,path}"##,
            enabled: has_doc,
            run: |s, p| {
                let name = p.get("name").and_then(Value::as_str);
                let path = resolve_path_params(s, p)?;
                let path_id = saved_path_id(p)?;
                let name =
                    path_id.and_then(|id| s.active()?.doc.paths.iter().find(|path| path.id == id).map(|path| path.name.as_str())).or(name).unwrap_or("work");
                Ok(json!({ "pathId": path_id.map(|id| id.0), "pathIdString": path_id.map(|id| id.0.to_string()), "name": name, "path": path_json(&path) }))
            },
            journal: false,
        },
        spec!(
            "path.set",
            "Set Path",
            [],
            leak(format!(
                r##"{{"pathId":id? (saved path ID takes precedence),"name":str|"work"="work","path":{{…}},"op":"combine|subtract|intersect|exclude"? (append to the existing path with this op instead of replacing),"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}}. {PATH_FORM}"##
            )),
            has_doc,
            path_set
        ),
        spec!("path.delete", "Delete Path", [], r##"{"pathId":id? (saved path ID takes precedence),"name":str|"work"="work","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##, has_doc, path_delete),
        spec!(
            "path.transform",
            "Free Transform Path",
            [],
            r##"{"pathId":id? (saved path ID takes precedence),"name":str|"work"|"layer"="work","layer":id? (with layer target),"matrix":[a,b,c,d,e,f] | "translateX":px?,"translateY":px?,"scaleX":factor?,"scaleY":factor?,"angle":degrees?,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (numeric transforms pivot on path bounds center)"##,
            has_doc,
            path_transform
        ),
        spec!(
            "path.clippingPath.set",
            "Clipping Path",
            [],
            r##"{"name":savedPathName,"flatness":0..100=0,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (PSD export clipping path)"##,
            has_doc,
            clipping_path_set
        ),
        spec!("path.clippingPath.clear", "Clear Clipping Path", [], r##"{"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##, has_doc, clipping_path_clear),
        spec!("path.style.copyFill", "Copy Fill", [], r##"{} (copies active shape layer fill)"##, has_shape_fill, copy_shape_fill),
        spec!("path.style.copyStroke", "Copy Complete Stroke", [], r##"{} (copies active shape layer stroke)"##, has_shape_stroke, copy_shape_stroke),
        spec!(
            "path.style.pasteFill",
            "Paste Fill",
            [],
            r##"{"layer":id?} (pastes copied fill onto active shape layer)"##,
            can_paste_shape_fill,
            paste_shape_fill
        ),
        spec!(
            "path.style.pasteStroke",
            "Paste Complete Stroke",
            [],
            r##"{"layer":id?} (pastes copied stroke onto active shape layer)"##,
            can_paste_shape_stroke,
            paste_shape_stroke
        ),
        spec!(
            "path.rename",
            "Rename Path",
            [],
            r##"{"pathId":id? (saved path ID takes precedence),"name":str|"work"="work","to":str,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (renaming the work path saves it)"##,
            has_doc,
            path_rename
        ),
        // ⌘↩ / Ctrl+Enter, as in Photoshop (the UI loads the path selected in the Paths panel, #306).
        CommandSpec {
            id: "path.toSelection",
            label: "Make Selection from Path",
            menu: &[],
            shortcut: Some("Cmd+Enter"),
            params: r##"{"pathId":id? (saved path ID takes precedence),"name":str|"work"|"layer"="work","feather":px=0,"antiAlias":bool=true,"mode":"replace|add|subtract|intersect"="replace","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            enabled: has_doc,
            run: path_to_selection,
            journal: true,
        },
        spec!("select.toWorkPath", "Make Work Path", [], r##"{"tolerance":0.5..10 px=2,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} → {subpaths,knots}"##, has_selection, select_to_work_path),
        spec!(
            "path.fill",
            "Fill Path",
            [],
            r##"{"pathId":id? (saved path ID takes precedence),"name":str|"work"|"layer"="work","layer":id? (pixel layer, default active),"color":"#rrggbb"=foreground,"opacity":0..100=100,"mode":"normal|multiply|…"="normal","feather":px=0,"antiAlias":bool=true,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_layer,
            path_fill
        ),
        spec!(
            "path.stroke",
            "Stroke Path",
            [],
            r##"{"pathId":id? (saved path ID takes precedence),"name":str|"work"|"layer"="work","layer":id? (pixel layer),"tool":"brush|pencil|eraser"="brush","size":0.5..5000 px?,"hardness":0..1?,"opacity":0..100?,"color":"#rrggbb"=foreground,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (current brush settings otherwise)"##,
            has_layer,
            path_stroke
        ),
        spec!(
            "layer.vectorMask.add",
            "Add Vector Mask",
            ["Layer", "Vector Mask"],
            r##"{"layer":id?,"path":{…}? | "name":str|"work"? (copy a document path),"hide":bool=false (Hide All = inverted)} → vector mask info. Without a path the mask reveals all."##,
            has_layer,
            |s, p| vector_mask_add(s, p, false)
        ),
        spec!(
            "layer.vectorMask.fromPath",
            "Vector Mask from Current Path",
            ["Layer", "Vector Mask"],
            r##"{"layer":id?,"name":str|"work"="work","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_layer,
            |s, p| vector_mask_add(s, p, true)
        ),
        spec!(
            "layer.vectorMask.edit",
            "Edit Vector Mask",
            [],
            leak(format!(r##"{{"layer":id?,"path":{{…}}?,"enabled":bool?,"linked":bool?,"density":0..100?,"feather":px?,"invert":true?}}. {PATH_FORM}"##)),
            has_layer,
            vector_mask_edit
        ),
        spec!("layer.vectorMask.delete", "Delete Vector Mask", ["Layer", "Vector Mask"], r##"{"layer":id?}"##, has_layer, vector_mask_delete),
        spec!(
            "layer.rasterize.vectorMask",
            "Rasterize Vector Mask",
            [],
            r##"{"layer":id?} (multiplied into the layer mask)"##,
            has_layer,
            vector_mask_rasterize
        ),
        // Menu ids of Photoshop's Layer › Vector Mask / Rasterize / Combine Shapes.
        spec!("layer.vectorMask.revealAll", "Vector Mask: Reveal All", [], r##"{"layer":id?}"##, has_layer, |s, p| vector_mask_add(
            s,
            &with(p, "hide", json!(false)),
            false
        )),
        spec!("layer.vectorMask.hideAll", "Vector Mask: Hide All", [], r##"{"layer":id?}"##, has_layer, |s, p| vector_mask_add(
            s,
            &with(p, "hide", json!(true)),
            false
        )),
        spec!("layer.vectorMask.currentPath", "Vector Mask: Current Path", [], r##"{"layer":id?,"name":str|"work"="work","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##, has_layer, |s, p| {
            vector_mask_add(s, p, true)
        }),
        spec!("layer.vectorMask.enabled", "Enable Vector Mask", [], r##"{"layer":id?,"enabled":bool? (default: toggle)}"##, has_layer, |s, p| {
            toggle_vector_mask(s, p, "enabled")
        }),
        spec!("layer.vectorMask.linked", "Link Vector Mask", [], r##"{"layer":id?,"linked":bool? (default: toggle)}"##, has_layer, |s, p| toggle_vector_mask(
            s, p, "linked"
        )),
        spec!("layer.rasterize.shape", "Rasterize Shape", [], r##"{"layer":id?}"##, has_layer, shape_rasterize),
        spec!(
            "layer.combineShapes.unite",
            "Unite Shapes",
            [],
            r##"{"layer":id?,"subpath":index? (default all but the first)}"##,
            has_layer,
            |s, p| shape_edit(s, &with(p, "op", json!("combine")))
        ),
        spec!("layer.combineShapes.subtractFrontShape", "Subtract Front Shape", [], r##"{"layer":id?,"subpath":index?}"##, has_layer, |s, p| shape_edit(
            s,
            &with(p, "op", json!("subtract"))
        )),
        spec!("layer.combineShapes.intersectShapeAreas", "Intersect Shape Areas", [], r##"{"layer":id?,"subpath":index?}"##, has_layer, |s, p| shape_edit(
            s,
            &with(p, "op", json!("intersect"))
        )),
        spec!("layer.combineShapes.excludeOverlappingShapes", "Exclude Overlapping Shapes", [], r##"{"layer":id?,"subpath":index?}"##, has_layer, |s, p| {
            shape_edit(s, &with(p, "op", json!("exclude")))
        }),
        spec!(
            "layer.combineShapes.mergeShapeComponents",
            "Merge Shape Components",
            [],
            r##"{"layer":id?,"tolerance":px=0.1} (bakes the path operations into plain combined outlines, traced from the exact coverage)"##,
            has_layer,
            merge_components
        ),
        spec!(
            "select.convertToShape",
            "Convert Selection to Shape",
            [],
            r##"{"tolerance":px=2,"fill":"#rrggbb"=foreground,"name":str?} → shape.info"##,
            has_selection,
            selection_to_shape
        ),
        CommandSpec {
            id: "layer.vectorMask.info",
            label: "Vector Mask Info",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?} → {layer,path,enabled,linked,density,feather}"##,
            enabled: has_layer,
            run: |s, p| {
                let id = layer_id(s, p)?;
                vector_mask_info(s, id)
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod canonical_path_id_tests {
    use super::*;
    #[test]
    fn saved_path_id_accepts_full_u64_and_rejects_noncanonical_values() {
        for value in [json!(u64::MAX), json!(u64::MAX.to_string())] {
            assert_eq!(saved_path_id(&json!({"pathId":value})).unwrap(), Some(photocraft_doc::item_identity::PathId(u64::MAX)));
        }
        for value in [json!("01"), json!("0"), json!("+1"), json!(" 1"), json!("18446744073709551616"), json!(1.0), json!(-1), json!(true), Value::Null] {
            assert!(saved_path_id(&json!({"pathId":value})).is_err(), "{value}");
        }
        assert_eq!(saved_path_id(&json!({})).unwrap(), None);
    }

    #[test]
    fn path_queries_expose_maximum_identity_as_decimal_string_without_retargeting() {
        let mut document = Document::new("Own identities", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut path = NamedPath::new("Exact maximum", Path::default());
        path.id = photocraft_doc::item_identity::PathId(u64::MAX);
        document.paths.push(path);
        let mut session = Session::new();
        session.open_document(document, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let listing = session.execute("path.list", json!({})).unwrap();
        assert_eq!(listing["paths"][0]["pathIdString"], u64::MAX.to_string());
        let details = session.execute("path.info", json!({"pathId":u64::MAX.to_string()})).unwrap();
        assert_eq!(details["pathIdString"], u64::MAX.to_string());
        assert_eq!(details["pathId"], u64::MAX);
        assert_eq!(session.active().unwrap().doc, before);
    }
}
