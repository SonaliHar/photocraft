//! Image menu remnants: Image Rotation › Arbitrary, and the palette / ink modes — Mode › Indexed
//! Color, Color Table, Bitmap and Duotone.
//!
//! Indexed, Bitmap and Duotone documents keep expanded pixels (RGB 8-bit / Gray 8-bit / Gray, see
//! `Document::pixel_format`): Indexed and Bitmap quantise the flattened image in place and record
//! the palette (`Document::color_table`) or the 1-bit look; Duotone keeps the gray pixels and the
//! inks (`Document::duotone`), which display and flat export render through. Like Photoshop the
//! conversions flatten (Indexed, Bitmap) and require a grayscale source (Bitmap, Duotone).

use photocraft_algo::quantize::{self, Dither, Forced};
use photocraft_color::ColorMode;
#[cfg(test)]
use photocraft_color::{PixelFormat, SampleType};
#[cfg(test)]
use photocraft_raster::Surface;
use photocraft_doc::{Document, Duotone, Layer, LayerContent, Size};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

mod rotation_controlled;
mod indexed_controlled;
mod bitmap_controlled;
mod duotone_controlled;
mod color_table_controlled;
pub(crate) fn color_table_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    color_table_controlled::dispatch_preflight(s, p, background)
}
pub(crate) fn duotone_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    duotone_controlled::dispatch_preflight(s, p, background)
}
pub(crate) fn indexed_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    indexed_controlled::dispatch_preflight(s, p, background)
}
pub(crate) fn bitmap_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    bitmap_controlled::dispatch_preflight(s, p, background)
}
pub(crate) fn rotation_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    rotation_controlled::dispatch_preflight(s, p, background)
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

type Enabled = std::result::Result<(), String>;

fn has_doc(s: &Session) -> Enabled {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn mode_is(s: &Session, modes: &[ColorMode], what: &str) -> Enabled {
    let d = s.active().ok_or("no document open")?;
    if modes.contains(&d.doc.mode) { Ok(()) } else { Err(format!("{what} (the document is {:?})", d.doc.mode)) }
}

// ---------- Image Rotation › Arbitrary ----------

/// Canvas size after rotating `size` by `deg` (the canvas grows to fit, as in Photoshop).
pub fn rotated_size(size: Size, deg: f64) -> Size {
    let (s, c) = deg.to_radians().sin_cos();
    let (w, h) = (f64::from(size.width), f64::from(size.height));
    let nw = (w * c.abs() + h * s.abs() - 1e-6).ceil().max(1.0);
    let nh = (w * s.abs() + h * c.abs() - 1e-6).ceil().max(1.0);
    Size::new(nw as u32, nh as u32)
}

fn rotate_arbitrary(s: &mut Session, p: &Value) -> Result<Value> {
    rotation_controlled::run(s, p)
}

#[cfg(test)]
fn composite(doc: &Document, opaque: bool) -> Result<Vec<[f32; 4]>> {
    if matches!(doc.depth, SampleType::U32 | SampleType::F64)
        || doc.walk().iter().any(|(_, _, layer)| layer.surface().is_some_and(|s| matches!(s.format().sample, SampleType::U32 | SampleType::F64)))
    {
        return Err(bad(
            "image.mode",
            "wide color quantization requires a qualified f64 composite; convert precision explicitly before changing to Indexed or Bitmap",
        ));
    }
    let buf = photocraft_compose::flatten(doc)?;
    Ok(if opaque { buf.over_background([1.0, 1.0, 1.0]).px } else { buf.px })
}

/// Parse the existing forced-color selection names.
fn parse_forced(s: &str) -> Option<Forced> {
    Some(match s {
        "none" => Forced::None,
        "blackWhite" | "blackAndWhite" => Forced::BlackWhite,
        "primaries" => Forced::Primaries,
        "web" => Forced::Web,
        _ => return None,
    })
}

fn parse_dither(s: &str) -> Option<Dither> {
    Some(match s {
        "none" => Dither::None,
        "diffusion" => Dither::Diffusion,
        "pattern" => Dither::Pattern,
        "noise" => Dither::Noise,
        _ => return None,
    })
}

fn indexed_color(s: &mut Session, p: &Value) -> Result<Value> {
    indexed_controlled::run(s, p)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn parse_color(v: &Value) -> Option<[u8; 3]> {
    match v {
        Value::String(s) => {
            let s = s.trim_start_matches('#');
            let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
            Some([b(0)?, b(2)?, b(4)?])
        }
        Value::Array(a) if a.len() >= 3 => {
            let c = |i: usize| a[i].as_f64().map(|x| x.round().clamp(0.0, 255.0) as u8);
            Some([c(0)?, c(1)?, c(2)?])
        }
        _ => None,
    }
}

/// Built-in Color Table presets.
pub fn preset_table(name: &str) -> Option<Vec<[u8; 3]>> {
    let ramp = |f: &dyn Fn(f32) -> [f32; 3]| (0..256).map(|i| f(i as f32 / 255.0).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    Some(match name {
        "grayscale" => ramp(&|t| [t; 3]),
        // Black → red → yellow → white, the incandescence scale.
        "blackBody" => ramp(&|t| [(t * 3.0).min(1.0), (t * 3.0 - 1.0).clamp(0.0, 1.0), (t * 3.0 - 2.0).clamp(0.0, 1.0)]),
        "spectrum" => ramp(&|t| {
            let h = t * 300.0 / 60.0;
            let x = 1.0 - (h % 2.0 - 1.0).abs();
            match h as i32 {
                0 => [1.0, x, 0.0],
                1 => [x, 1.0, 0.0],
                2 => [0.0, 1.0, x],
                3 => [0.0, x, 1.0],
                _ => [x, 0.0, 1.0],
            }
        }),
        "systemMac" | "system" => quantize::mac_palette(),
        "systemWindows" => quantize::windows_palette(),
        "web" => quantize::web_palette(),
        _ => return None,
    })
}

/// Image › Mode › Color Table: replace entries (pixels keep their index, so their colour follows
/// the table, as in Photoshop) and set the transparent entry. No changes: report the table.
fn color_table(s: &mut Session, p: &Value) -> Result<Value> {
    color_table_controlled::run(s, p)
}

fn bitmap(s: &mut Session, p: &Value) -> Result<Value> {
    bitmap_controlled::run(s, p)
}

fn duotone(s: &mut Session, p: &Value) -> Result<Value> {
    duotone_controlled::run(s, p)
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "image.rotation.arbitrary",
            "Arbitrary…",
            ["Image", "Image Rotation"],
            r##"{"angle":-359.99..359.99=0,"direction":"cw|ccw"="cw","interpolation":"bicubic|bilinear|nearest"="bicubic","documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            rotate_arbitrary
        ),
        spec!(
            "image.mode.indexedColor",
            "Indexed Color…",
            ["Image", "Mode"],
            r##"{"palette":"selective|perceptual|adaptive|exact|systemMac|systemWindows|web|uniform"="selective","colors":2..256=256,"forced":"blackWhite|none|primaries|web"="blackWhite","transparency":bool=true,"dither":"diffusion|none|pattern|noise"="diffusion","amount":0..100=75,"canonicalIndexed":bool=false,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?} (flattens; canonicalIndexed stores palette ordinals and palette RGB with quantized alpha, including alpha0 hidden RGB; ordinary RGB only)"##,
            |s| mode_is(
                s,
                &[ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Indexed, ColorMode::Duotone, ColorMode::Cmyk, ColorMode::Lab],
                "Indexed Color needs an RGB or Grayscale image"
            ),
            indexed_color
        ),
        spec!(
            "image.mode.colorTable",
            "Color Table…",
            ["Image", "Mode"],
            r##"{"table":"custom|blackBody|grayscale|spectrum|systemMac|systemWindows|web"="custom","colors":json,"entries":json,"transparent":json,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?} (colors: ["#rrggbb", …]; entries: {"index": "#rrggbb"}; transparent: index|null)"##,
            |s| mode_is(s, &[ColorMode::Indexed], "Color Table needs an Indexed Color image"),
            color_table
        ),
        spec!(
            "image.mode.bitmap",
            "Bitmap…",
            ["Image", "Mode"],
            r##"{"method":"diffusion|threshold|pattern|halftone"="diffusion","frequency":1..999=53,"angle":-180..180=45,"shape":"round|ellipse|line|square|diamond|cross"="round","documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?} (halftone frequency in lines/inch; flattens)"##,
            |s| mode_is(s, &[ColorMode::Grayscale], "Bitmap needs a Grayscale image (Image › Mode › Grayscale first)"),
            bitmap
        ),
        spec!(
            "image.mode.duotone",
            "Duotone…",
            ["Image", "Mode"],
            r##"{"type":"duotone|monotone|tritone|quadtone"="duotone","inks":json,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?} (inks: [{"name","color":"#rrggbb","curve":[[in,out],…] in 0..100}, …])"##,
            |s| mode_is(s, &[ColorMode::Grayscale, ColorMode::Duotone], "Duotone needs a Grayscale image (Image › Mode › Grayscale first)"),
            duotone
        ),
    ]
}

/// The gradient-map layer that renders Duotone inks over the gray composite.
pub fn duotone_display_layer(d: &Duotone) -> Layer {
    Layer::new("Duotone", LayerContent::Adjustment(d.display_adjustment()))
}

/// A copy of `doc` that displays like the printed result (Duotone inks), or None when the
/// document displays as stored.
pub fn display_document(doc: &Document) -> Option<Document> {
    let d = doc.duotone.as_ref().filter(|_| doc.mode == ColorMode::Duotone)?;
    let mut out = doc.clone();
    out.layers.push(duotone_display_layer(d));
    Some(out)
}

#[cfg(test)]
mod tests;
