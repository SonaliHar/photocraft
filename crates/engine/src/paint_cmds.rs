//! Paint helpers: Paint Bucket and Gradient tool.

use photocraft_algo::paint::{GradientShape, bucket_fill_inks, bucket_fill_region_inks, bucket_fill_region_src, bucket_fill_src, paint_gradient};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::presets::gradients;
use crate::{EngineError, Result, Session};

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn color(v: Option<&Value>, d: [f32; 4]) -> [f32; 4] {
    match v {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            [c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)]
        }
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match (c(0), c(2), c(4)) {
                (Some(r), Some(g), Some(b)) => [r, g, b, c(6).unwrap_or(1.0)],
                _ => d,
            }
        }
        _ => d,
    }
}
fn point(p: &Value, k: &str) -> Option<(f32, f32)> {
    let a = p.get(k)?.as_array()?;
    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

pub(crate) fn wide_target(s: &Session, p: &Value) -> Result<Option<(Option<photocraft_doc::LayerId>, photocraft_color::PixelFormat)>> {
    use crate::channel_cmds::Target;
    if let Some(i) = p.get("target").and_then(|v| v.get("channel")) {
        let index = i.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| wide_bad("channel index is out of range"))?;
        if s.active().ok_or(EngineError::NoDocument)?.doc.channels.get(index).is_none() {
            return Err(wide_bad("no such channel"));
        }
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let target = crate::channel_cmds::target_of(p);
    let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(crate::commands::layer_param(s, p)?) };
    let surface = match target {
        Target::Alpha(i) => st.doc.channels.get(i).map(|c| &c.surface),
        Target::QuickMask => st.doc.quick_mask.as_ref().map(|c| &c.surface),
        Target::Mask => id.and_then(|id| st.doc.layer(id)).and_then(|l| l.mask.as_ref()).map(|m| m.surface.as_ref()),
        Target::Pixels => id.and_then(|id| st.doc.layer(id)).and_then(|l| l.surface()),
    }
    .ok_or_else(|| wide_bad("target has no editable pixels"))?;
    let format = surface.format();
    Ok(matches!(format.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64).then_some((id, format)))
}
fn wide_bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}
pub(crate) fn wide_color(v: Option<&Value>, default: [f32; 4]) -> Result<[f64; 4]> {
    let mut out = default.map(f64::from);
    match v {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) if a.len() == 3 || a.len() == 4 => {
            out = [0.0, 0.0, 0.0, 1.0];
            for (dst, src) in out.iter_mut().zip(a) {
                *dst = src.as_f64().filter(|v| v.is_finite()).ok_or_else(|| wide_bad("color components must be finite numbers"))?;
            }
        }
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            if h.len() != 6 && h.len() != 8 {
                return Err(wide_bad("color must be #rrggbb or #rrggbbaa"));
            }
            out = [0.0, 0.0, 0.0, 1.0];
            for (i, dst) in out.iter_mut().enumerate().take(h.len() / 2) {
                let part = h.get(i * 2..i * 2 + 2).ok_or_else(|| wide_bad("invalid hexadecimal color"))?;
                *dst = f64::from(u8::from_str_radix(part, 16).map_err(|_| wide_bad("invalid hexadecimal color"))?) / 255.0;
            }
        }
        _ => return Err(wide_bad("color must be an RGB/RGBA array or hexadecimal string")),
    }
    if !out.iter().all(|v| v.is_finite()) || !(0.0..=1.0).contains(&out[3]) {
        return Err(wide_bad("invalid color or alpha"));
    }
    Ok(out)
}
pub(crate) fn wide_opacity(p: &Value) -> Result<f64> {
    let value = match p.get("opacity") {
        None | Some(Value::Null) => 100.0,
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| wide_bad("opacity must be finite"))?,
    };
    Ok(value.clamp(0.0, 100.0) / 100.0)
}
fn wide_gradient(s: &mut Session, p: &Value, id: Option<photocraft_doc::LayerId>, format: photocraft_color::PixelFormat) -> Result<Value> {
    if s.color_restrict.is_some() || p.get("mode").is_some_and(|v| v.as_str() != Some("normal")) || p.get("dither").and_then(Value::as_bool) != Some(false) {
        return Err(wide_bad("wide gradients require Normal mode, all color channels and explicit dither:false"));
    }
    if p.get("transparency").is_some() {
        return Err(wide_bad("wide transparency overrides are not yet qualified"));
    }
    let point = |key| -> Result<(f64, f64)> {
        let a = p.get(key).and_then(Value::as_array).filter(|a| a.len() == 2).ok_or_else(|| wide_bad("gradient needs finite from/to pairs"))?;
        let x = a.first().and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| wide_bad("invalid gradient coordinate"))?;
        let y = a.get(1).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| wide_bad("invalid gradient coordinate"))?;
        Ok((x, y))
    };
    let from = point("from")?;
    let to = point("to")?;
    let shape = match p.get("style").and_then(Value::as_str).unwrap_or("linear") {
        "linear" => GradientShape::Linear,
        "radial" => GradientShape::Radial,
        "angle" => GradientShape::Angle,
        "reflected" => GradientShape::Reflected,
        "diamond" => GradientShape::Diamond,
        _ => return Err(wide_bad("unknown gradient style")),
    };
    let opacity = wide_opacity(p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let use_preset = p.get("colors").is_none() && (p.get("gradient").is_some() || p.get("stops").is_none());
    let mut stops = if use_preset { wide_preset_stops(s, p, format)? } else { Vec::new() };
    if !use_preset {
        let values = p.get("stops").or_else(|| p.get("colors")).and_then(Value::as_array).ok_or_else(|| wide_bad("invalid gradient stops or colors"))?;
        if values.is_empty() || values.len() > 4096 {
            return Err(wide_bad("gradient requires 1..4096 stops"));
        }
        stops.try_reserve_exact(values.len()).map_err(|_| wide_bad("gradient stop allocation failed"))?;
        for (i, v) in values.iter().enumerate() {
            let (position, value) = if p.get("stops").is_some() {
                let pair = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| wide_bad("stop must be [position,color]"))?;
                (pair.first().and_then(Value::as_f64).ok_or_else(|| wide_bad("invalid stop position"))?, pair.get(1))
            } else {
                (if values.len() > 1 { i as f64 / (values.len() - 1) as f64 } else { 0.0 }, Some(v))
            };
            let rgba = match value.and_then(Value::as_str) {
                Some("foreground") => s.tools.foreground.map(f64::from),
                Some("background") => s.tools.background.map(f64::from),
                _ => wide_color(value, s.tools.foreground)?,
            };
            if !format.alpha && rgba[3] != 1.0 {
                return Err(wide_bad("wide gradients with source alpha need an alpha target"));
            }
            stops.push(photocraft_algo::wide_paint::NativeStop { position, color: crate::brush_cmds::native_wide_color(&st.doc, format, p, rgba)? });
        }
    }
    let area = crate::brush_cmds::wide_paint_extent(&st.doc, id, p).map_or(st.doc.bounds(), |r| r.intersect(&st.doc.bounds()));
    let reverse = match p.get("reverse") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| wide_bad("reverse must be boolean"))?,
    };
    s.edit("Gradient", |doc, _| {
        let selection = doc.selection.clone();
        let (target, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
        photocraft_algo::wide_paint::try_paint_gradient(
            target,
            area,
            photocraft_algo::wide_paint::GradientOptions { from, to, shape, reverse, opacity, lock_transparency: lock },
            &stops,
            selection.as_ref(),
            Default::default(),
            &photocraft_raster::Interrupt::NONE,
        )
        .map_err(|e| wide_bad(e.to_string()))
    })?;
    Ok(Value::Null)
}

/// Resolve original preset color data without the legacy sampled transparency approximation.
/// Only an opaque or constant-opacity curve has an exact representation in this stop kernel.
fn wide_preset_stops(s: &Session, p: &Value, format: photocraft_color::PixelFormat) -> Result<Vec<photocraft_algo::wide_paint::NativeStop>> {
    let preset = match p.get("gradient") {
        None => &s.presets.gradient,
        Some(value) => {
            let name = value.as_str().ok_or_else(|| wide_bad("gradient preset name must be a string"))?;
            s.presets
                .gradients
                .iter()
                .flat_map(|group| &group.items)
                .find(|item| item.name == name)
                .ok_or_else(|| wide_bad(format!("no gradient preset \"{name}\"")))?
        }
    };
    if preset.stops.len() > 4096 || preset.opacity.len() > 4096 {
        return Err(wide_bad("gradient requires at most4096 color/transparency stops"));
    }
    let alpha = preset.opacity.first().map_or(1.0, |(_, alpha)| f64::from(*alpha));
    if !alpha.is_finite()
        || !(0.0..=1.0).contains(&alpha)
        || preset.opacity.iter().any(|(position, value)| !position.is_finite() || !(0.0..=1.0).contains(position) || f64::from(*value) != alpha)
    {
        return Err(wide_bad("wide preset gradients currently require finite constant opacity"));
    }
    if !format.alpha && alpha != 1.0 {
        return Err(wide_bad("wide gradients with source alpha need an alpha target"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut stops = Vec::new();
    stops.try_reserve_exact(preset.stops.len().max(2)).map_err(|_| wide_bad("gradient stop allocation failed"))?;
    let mut push = |position: f64, rgb: [f32; 3]| -> Result<()> {
        if !position.is_finite() || rgb.iter().any(|v| !v.is_finite()) {
            return Err(wide_bad("invalid preset color stop"));
        }
        let rgba = [f64::from(rgb[0]), f64::from(rgb[1]), f64::from(rgb[2]), alpha];
        stops.push(photocraft_algo::wide_paint::NativeStop {
            position: position.clamp(0.0, 1.0),
            color: crate::brush_cmds::native_wide_color(&st.doc, format, p, rgba)?,
        });
        Ok(())
    };
    if preset.stops.is_empty() {
        push(0.0, [s.tools.foreground[0], s.tools.foreground[1], s.tools.foreground[2]])?;
        push(1.0, [s.tools.background[0], s.tools.background[1], s.tools.background[2]])?;
    } else {
        for (position, color) in &preset.stops {
            let rgb = match color {
                gradients::StopColor::Foreground => [s.tools.foreground[0], s.tools.foreground[1], s.tools.foreground[2]],
                gradients::StopColor::Background => [s.tools.background[0], s.tools.background[1], s.tools.background[2]],
                gradients::StopColor::Rgb(rgb) => *rgb,
            };
            push(f64::from(*position), rgb)?;
        }
    }
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    Ok(stops)
}

fn wide_bucket(s: &mut Session, p: &Value, id: Option<photocraft_doc::LayerId>, format: photocraft_color::PixelFormat) -> Result<Value> {
    let flag = |key: &str, default| -> Result<bool> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v.as_bool().ok_or_else(|| wide_bad(format!("{key} must be boolean"))),
        }
    };
    if s.color_restrict.is_some() || flag("sampleAllLayers", false)? || flag("antiAlias", true)? || p.get("mode").is_some_and(|v| v.as_str() != Some("normal"))
    {
        return Err(wide_bad("wide bucket currently requires native target sampling, all color channels, Normal mode and antiAlias:false"));
    }
    let tolerance = match p.get("tolerance") {
        None => 32.0,
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| wide_bad("invalid tolerance"))?,
    };
    if tolerance != 0.0 {
        return Err(wide_bad("wide bucket currently supports exact native matching only; set tolerance:0"));
    }
    if p.get("contents").or_else(|| p.get("source")).is_some_and(|v| v.as_str() != Some("foreground")) {
        return Err(wide_bad("wide bucket pattern sources are not yet qualified"));
    }
    let coordinate = |key| -> Result<i32> {
        let value = match p.get(key) {
            None => 0.0,
            Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| wide_bad("invalid bucket coordinate"))?,
        }
        .floor();
        if value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
            return Err(wide_bad("bucket coordinate is out of range"));
        }
        Ok(value as i32)
    };
    let seed = (coordinate("x")?, coordinate("y")?);
    let contiguous = flag("contiguous", true)?;
    let rgba = wide_color(p.get("color"), s.tools.foreground)?;
    let mut opacity = wide_opacity(p)?;
    if !format.alpha {
        opacity *= rgba[3];
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let color = crate::brush_cmds::native_wide_color(&st.doc, format, p, rgba)?;
    let area = crate::brush_cmds::wide_paint_extent(&st.doc, id, p).map_or(st.doc.bounds(), |r| r.intersect(&st.doc.bounds()));
    let filled = s.edit("Paint Bucket", |doc, _| {
        let selection = doc.selection.clone();
        let (target, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
        photocraft_algo::wide_paint::try_paint_bucket_exact(
            target,
            area,
            photocraft_algo::wide_paint::BucketOptions { seed, contiguous, opacity, lock_transparency: lock },
            color.get(..format.channels()).ok_or_else(|| wide_bad("invalid color layout"))?,
            selection.as_ref(),
            Default::default(),
            &photocraft_raster::Interrupt::NONE,
        )
        .map_err(|e| wide_bad(e.to_string()))
    })?;
    Ok(json!({"filled":filled}))
}

fn bucket(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some((id, format)) = wide_target(s, p)? {
        return wide_bucket(s, p, id, format);
    }
    let fg = s.tools.foreground;
    let c = color(p.get("color"), fg);
    let (x, y) = (f(p, "x", 0.0).floor() as i32, f(p, "y", 0.0).floor() as i32);
    let (tol, contiguous, aa) = (f(p, "tolerance", 32.0), b(p, "contiguous", true), b(p, "antiAlias", true));
    let opacity = opacity(p, "paint.bucket")?;
    let all = b(p, "sampleAllLayers", false) && crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels;
    // Other visible layers can define the region, but the fill still writes only to the target.
    let region = if all {
        let (area, img) = crate::selection_cmds::sample_rgba8(s, true)?;
        let region = photocraft_algo::selection::wand_region(&img, area, (x, y), tol, contiguous, aa);
        drop(img);
        region
    } else {
        None
    };
    // Fill source: foreground colour (default) or a pattern (the Paint Bucket "Fill" dropdown).
    let source = p.get("contents").or_else(|| p.get("source")).and_then(Value::as_str).unwrap_or("foreground");
    let pattern = if source == "pattern" {
        let pat = crate::pattern_cmds::resolve_param(s, "paint.bucket", p)?;
        let tile = photocraft_compose::pattern::Tile::new(&pat)
            .ok_or_else(|| EngineError::BadParams { cmd: "paint.bucket".into(), msg: "the pattern is empty or too large to paint".into() })?;
        let (scale, angle, _, phase) = crate::pattern_cmds::placement(p);
        Some((tile, scale, angle, phase))
    } else {
        None
    };
    // A foreground picked as CMYK fills a CMYK document with the picked inks (#4196).
    let inks = s.tools.picked_cmyk([c[0], c[1], c[2]]).filter(|_| c[3] >= 1.0 && pattern.is_none());
    let filled = s.edit("Paint Bucket", |doc, active| {
        let area = doc.bounds();
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        let ok = if all && region.is_none() {
            false
        } else if let Some((tile, scale, angle, phase)) = &pattern {
            // Render the pattern over the canvas once, then sample it at each filled pixel.
            let place = photocraft_compose::pattern::Placement::new(photocraft_geom::Rect::EMPTY, false, *phase, *scale, *angle);
            let rendered = photocraft_compose::pattern::render(tile, &place, area);
            let w = area.width() as usize;
            let src = |px: i32, py: i32| rendered[(py - area.y0) as usize * w + (px - area.x0) as usize];
            if let Some(region) = &region {
                bucket_fill_region_src(surf, region, opacity, sel.as_ref(), lock, src);
                true
            } else {
                bucket_fill_src(surf, area, (x, y), tol, contiguous, aa, opacity, sel.as_ref(), lock, src)
            }
        } else if let Some(region) = &region {
            bucket_fill_region_inks(surf, region, opacity, sel.as_ref(), lock, |_, _| c, inks);
            true
        } else {
            bucket_fill_inks(surf, area, (x, y), tol, contiguous, aa, opacity, sel.as_ref(), lock, |_, _| c, inks)
        };
        surf.prune();
        Ok(ok)
    })?;
    Ok(json!({ "filled": filled }))
}

fn gradient(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some((id, format)) = wide_target(s, p)? {
        return wide_gradient(s, p, id, format);
    }
    let from = point(p, "from").ok_or_else(|| EngineError::BadParams { cmd: "paint.gradient".into(), msg: "missing from".into() })?;
    let to = point(p, "to").ok_or_else(|| EngineError::BadParams { cmd: "paint.gradient".into(), msg: "missing to".into() })?;
    let shape = match p.get("style").and_then(Value::as_str).unwrap_or("linear") {
        "radial" => GradientShape::Radial,
        "angle" => GradientShape::Angle,
        "reflected" => GradientShape::Reflected,
        "diamond" => GradientShape::Diamond,
        _ => GradientShape::Linear,
    };
    let (fg, bg) = (s.tools.foreground, s.tools.background);
    // A preset (`gradient`), explicit `stops`, or the current gradient (Gradients panel); the
    // legacy `colors` list spaces its colours evenly. Explicit `transparency` applies to all.
    let stops: Vec<(f32, [f32; 4])> = match gradients::tool_stops(s, p)? {
        Some(st) => st,
        None => {
            let colors: Vec<[f32; 4]> =
                p.get("colors").and_then(Value::as_array).map(|a| a.iter().map(|v| color(Some(v), fg)).collect()).unwrap_or_else(|| vec![fg, bg]);
            let n = colors.len();
            let cs = colors.into_iter().enumerate().map(|(i, c)| (if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 }, c)).collect();
            gradients::apply_opacity(cs, &gradients::transparency_param(p, "paint.gradient")?.unwrap_or_default())
        }
    };
    // Photoshop's interpolation (`method`, with `smoothness` 0..=100, default 100) baked into dense
    // stops, as for Gradient Overlay: Photoshop 27.11's Classic black → white is 0.037 at t = 1/16
    // where straight stops give 0.0625.
    let stops = match p.get("method").and_then(Value::as_str).and_then(photocraft_io::GradientMethod::from_name) {
        Some(m) if stops.len() >= 2 => {
            let colors = stops.iter().map(|(t, c)| (*t, photocraft_color::Color::rgba(c[0], c[1], c[2], c[3]))).collect();
            let mids = vec![0.5; stops.len() - 1];
            photocraft_io::bake_gradient(colors, &mids, (f(p, "smoothness", 100.0) / 100.0).clamp(0.0, 1.0), m)
                .into_iter()
                .map(|(t, c)| {
                    let rgb = c.to_rgb();
                    (t, [rgb[0], rgb[1], rgb[2], c.alpha])
                })
                .collect()
        }
        _ => stops,
    };
    let reverse = b(p, "reverse", false);
    let opacity = opacity(p, "paint.gradient")?;
    let blend = p.get("mode").and_then(Value::as_str).and_then(blend_from_str).unwrap_or(photocraft_color::BlendMode::Normal);
    let dither = b(p, "dither", true); // Photoshop dithers gradients by default (reduces banding).
    s.edit("Gradient", |doc, active| {
        let sel = doc.selection.clone();
        let area = sel.as_ref().map(|m| m.content_bounds()).filter(|r| !r.is_empty()).unwrap_or_else(|| doc.bounds()).intersect(&doc.bounds());
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        paint_gradient(surf, area, from, to, shape, &stops, reverse, opacity, blend, dither, sel.as_ref(), lock);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Paint helper command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "paint.bucket",
            label: "Paint Bucket",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"tolerance":0..255=32,"contiguous":bool=true,"antiAlias":bool=true,"sampleAllLayers":bool=false,"contents":"foreground|pattern"="foreground","color":"#rrggbb"=foreground,"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"opacity":1..100=100,"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            enabled: crate::commands::has_paintable,
            run: bucket,
            journal: true,
        },
        CommandSpec {
            id: "paint.gradient",
            label: "Gradient",
            menu: &[],
            shortcut: None,
            params: r##"{"from":[x,y],"to":[x,y],"style":"linear|radial|angle|reflected|diamond"="linear","colors":["#rrggbb",…]? (evenly spaced),"gradient":preset name?,"stops":[[t,"#rrggbb"|"foreground"|"background"],…]?,"transparency":[[t,0..100],…]? (default: the current gradient, see gradient.presets.select),"method":"classic|perceptual|linear|smooth"? (Photoshop's interpolation; omitted: straight stops),"smoothness":0..100=100,"reverse":bool=false,"dither":bool=true,"opacity":1..100=100,"mode":"normal|multiply|…"="normal","target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            enabled: crate::commands::has_paintable,
            run: gradient,
            journal: true,
        },
    ]
}

fn opacity(p: &Value, cmd: &str) -> Result<f32> {
    Ok(crate::eraser_cmds::number(p, cmd, "opacity", 100.0, 1.0..=100.0)? as f32 / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Rect;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
        s
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
    }

    /// A visible wall on the Background with an empty, active layer above it.
    fn bucket_layers(depth: u32) -> (Session, photocraft_doc::LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10, "depth": depth, "background": "white"})).unwrap();
        let bottom = s.active().unwrap().active_layer.unwrap();
        s.edit("wall", |doc, _| {
            doc.layer_mut(bottom).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        (s, bottom)
    }

    #[test]
    fn bucket_sample_all_layers_uses_visible_boundaries_at_all_depths() {
        for depth in [8, 16, 32] {
            for all in [None, Some(false), Some(true)] {
                let (mut s, bottom) = bucket_layers(depth);
                let before = s.active().unwrap().doc.layer(bottom).unwrap().clone();
                let mut p = json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0});
                if let Some(all) = all {
                    p["sampleAllLayers"] = json!(all);
                }
                assert_eq!(s.execute("paint.bucket", p.clone()).unwrap()["filled"], true);
                assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 0.0, 1.0], "depth {depth}, all {all:?}");
                let beyond = if all == Some(true) { vec![0.0; 4] } else { vec![1.0, 0.0, 0.0, 1.0] };
                assert_eq!(px(&s, 15, 5), beyond, "depth {depth}, all {all:?}: sample flag controls the boundary");
                assert_eq!(s.active().unwrap().doc.layer(bottom).unwrap(), &before, "only the active layer is written");
                if all == Some(true) {
                    assert_eq!(px(&s, 10, 5), vec![0.0; 4], "wall itself is untouched");
                    assert!(s.undo());
                    p["contiguous"] = json!(false);
                    s.execute("paint.bucket", p).unwrap();
                    assert_eq!(px(&s, 15, 5), vec![1.0, 0.0, 0.0, 1.0], "noncontiguous finds both white regions");
                    assert_eq!(px(&s, 10, 5), vec![0.0; 4]);
                    assert!(s.undo());
                    assert_eq!(s.execute("paint.bucket", json!({"x": -1, "y": 2, "sampleAllLayers": true})).unwrap()["filled"], false);
                    assert_eq!(px(&s, 5, 5), vec![0.0; 4], "an off-canvas sample does not fill the layer");
                }
            }
        }
    }

    #[test]
    fn bucket_sample_all_layers_ignores_hidden_layers() {
        let (mut s, _) = bucket_layers(8);
        s.edit("hidden wall", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(4, 0, 5, 10), &[0.0, 0.0, 0.0, 1.0]);
            l.visible = false;
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true})).unwrap();
        assert_eq!(px(&s, 4, 5), vec![1.0, 0.0, 0.0, 1.0], "hidden wall does not bound the fill");
        assert_eq!(px(&s, 8, 5), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&s, 15, 5), vec![0.0; 4], "visible wall still bounds the fill");
    }

    #[test]
    fn bucket_sample_all_layers_keeps_selection_opacity_and_history() {
        for depth in [8, 16, 32] {
            let (mut s, _) = bucket_layers(depth);
            s.execute("select.rect", json!({"x": 0, "y": 2, "width": 20, "height": 6})).unwrap();
            s.edit("feather", |doc, _| {
                doc.selection.as_mut().unwrap().fill_rect(Rect::new(0, 2, 4, 8), &[0.5]);
                Ok(())
            })
            .unwrap();
            let selection = s.active().unwrap().doc.selection.clone();
            s.execute("paint.bucket", json!({"x": 2, "y": 5, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "opacity": 50, "sampleAllLayers": true}))
                .unwrap();
            let (feathered, full) = (px(&s, 2, 5), px(&s, 6, 5));
            assert!((feathered[3] - 0.25).abs() < 0.005 && (full[3] - 0.5).abs() < 0.005, "depth {depth}: selection coverage multiplies opacity");
            assert_eq!(full[..3], [1.0, 0.0, 0.0]);
            assert_eq!(px(&s, 6, 1), vec![0.0; 4], "outside selection");
            assert_eq!(px(&s, 15, 5), vec![0.0; 4], "outside sampled region");
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Paint Bucket"));
            assert!(s.undo());
            assert_eq!(px(&s, 2, 5), vec![0.0; 4]);
            assert_eq!(s.active().unwrap().doc.selection, selection);
            assert!(s.redo());
            assert_eq!(px(&s, 2, 5), feathered);
            assert_eq!(px(&s, 6, 5), full);
            assert_eq!(s.active().unwrap().doc.selection, selection);
        }
    }

    #[test]
    fn bucket_sample_all_layers_pattern_uses_the_same_region() {
        let (mut s, _) = bucket_layers(8);
        s.execute(
            "paint.bucket",
            json!({"x": 2, "y": 2, "contents": "pattern", "pattern": "Diagonal Lines", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true}),
        )
        .unwrap();
        let left: Vec<f32> = (0..10).map(|x| px(&s, x, 0)[0]).collect();
        assert!(left.iter().any(|&r| r < 0.3) && left.iter().any(|&r| r > 0.9), "pattern varies inside the region: {left:?}");
        assert_eq!(px(&s, 10, 5), vec![0.0; 4], "wall untouched");
        assert_eq!(px(&s, 15, 5), vec![0.0; 4], "region past the wall untouched");
    }

    #[test]
    fn bucket_sample_all_layers_mask_and_channel_targets_keep_their_own_boundaries() {
        for target in [json!("mask"), json!({"channel": 0}), json!("quickMask")] {
            for all in [false, true] {
                let (mut s, _) = bucket_layers(8);
                match crate::channel_cmds::target_of(&json!({"target": target})) {
                    crate::channel_cmds::Target::Mask => {
                        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    }
                    crate::channel_cmds::Target::Alpha(_) => {
                        s.execute("channel.new", json!({"fill": "white"})).unwrap();
                    }
                    crate::channel_cmds::Target::QuickMask => {
                        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                    }
                    _ => panic!("expected a grayscale target"),
                }
                let p = json!({"x": 2, "y": 2, "color": "#000000", "antiAlias": false, "tolerance": 0, "target": target, "sampleAllLayers": all});
                s.edit("target wall", |doc, active| {
                    let area = doc.bounds();
                    let (surf, _) = crate::channel_cmds::target_surface(doc, *active, &p).unwrap();
                    surf.fill_rect(area, &[1.0]);
                    surf.fill_rect(Rect::new(5, 0, 6, 10), &[0.0]);
                    Ok(())
                })
                .unwrap();
                s.execute("paint.bucket", p.clone()).unwrap();
                let d = &s.active().unwrap().doc;
                let surf = match crate::channel_cmds::target_of(&p) {
                    crate::channel_cmds::Target::Mask => &d.layer(s.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface,
                    crate::channel_cmds::Target::Alpha(i) => &d.channels[i].surface,
                    crate::channel_cmds::Target::QuickMask => &d.quick_mask.as_ref().unwrap().surface,
                    _ => panic!("expected a grayscale target"),
                };
                assert_eq!(surf.sample_channel(2, 5, 0), 0.0, "{target}, all {all}: filled target");
                assert_eq!(surf.sample_channel(7, 5, 0), 1.0, "{target}, all {all}: stopped at target wall");
                assert_eq!(px(&s, 2, 5), vec![0.0; 4], "layer pixels remain untouched");
            }
        }
    }

    #[test]
    fn bucket_fill_contiguous() {
        let mut s = session();
        s.edit("wall", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false})).unwrap();
        assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&s, 15, 5), vec![1.0, 1.0, 1.0, 1.0]);
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#0000ff", "contiguous": false, "antiAlias": false, "tolerance": 0})).unwrap();
        assert_eq!(px(&s, 5, 5), vec![0.0, 0.0, 1.0, 1.0]);
    }

    /// #4196: the Paint Bucket and Edit > Stroke write a CMYK pick as its inks into a CMYK document,
    /// not as its sRGB stand-in converted back (100/0/0/0 cyan would come out as about 100/28/0/17).
    #[test]
    fn bucket_and_stroke_write_a_cmyk_pick_as_its_inks() {
        let cyan = [1.0f32, 0.0, 0.0, 0.0];
        let mut s = session();
        s.execute("image.mode.cmyk", json!({})).unwrap();
        let rgb = photocraft_color::convert::cmyk_to_rgb(cyan).map(|c| c.clamp(0.0, 1.0));
        s.execute("tools.setColors", json!({"foreground": rgb, "foregroundCmyk": cyan})).unwrap();
        let inks_at = |s: &Session, x: i32, y: i32| -> Vec<f32> {
            let st = s.active().unwrap();
            let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
            surf.read_region(Rect::new(x, y, x + 1, y + 1)).into_iter().take(4).collect()
        };
        let is_cyan = |got: &[f32]| got.iter().zip(cyan).all(|(a, b)| (a - b).abs() < 0.02);
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "antiAlias": false})).unwrap();
        let got = inks_at(&s, 5, 5);
        assert!(is_cyan(&got), "bucket: {got:?}");
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.stroke", json!({"width": 2, "location": "inside", "color": rgb})).unwrap();
        let got = inks_at(&s, 0, 5);
        assert!(is_cyan(&got), "stroke: {got:?}");
    }

    #[test]
    fn bucket_fills_with_a_pattern() {
        // Paint Bucket with contents="pattern" flood-fills the region with a pattern, not a colour.
        let mut s = session();
        s.edit("wall", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "contents": "pattern", "pattern": "Diagonal Lines", "antiAlias": false, "tolerance": 0})).unwrap();
        // The filled region (left of the wall) now carries the pattern's light/dark variation,
        // while the wall and the region past it are untouched.
        let left: Vec<[f32; 4]> = (0..10)
            .map(|x| {
                let p = px(&s, x, 0);
                [p[0], p[1], p[2], p[3]]
            })
            .collect();
        assert!(left.iter().any(|p| p[0] < 0.3) && left.iter().any(|p| p[0] > 0.9), "pattern varies in the fill: {left:?}");
        assert_eq!(px(&s, 15, 5), vec![1.0, 1.0, 1.0, 1.0], "region past the wall untouched");
    }

    #[test]
    fn gradient_tool_with_selection_and_reverse() {
        let mut s = session();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [20, 0], "colors": ["#000000", "#ffffff"]})).unwrap();
        assert!(px(&s, 0, 5)[0] < 0.05 && px(&s, 19, 5)[0] > 0.95);
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [20, 0], "colors": ["#000000", "#ffffff"], "reverse": true})).unwrap();
        assert!(px(&s, 0, 5)[0] > 0.95);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 10})).unwrap();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [0, 10], "colors": ["#ff0000"]})).unwrap();
        assert_eq!(px(&s, 2, 5)[..3], [1.0, 0.0, 0.0]);
        assert!(px(&s, 10, 5)[1] > 0.4, "outside the selection untouched");
        assert!(s.execute("paint.gradient", json!({"from": [0, 0]})).is_err());
    }

    #[test]
    fn gradient_dither_breaks_banding() {
        // A flat gradient (same colour both ends) is perfectly smooth with dither off, and gains
        // per-pixel noise with dither on (which breaks 8-bit banding).
        let run = |dither: bool| {
            let mut s = session();
            s.execute("paint.gradient", json!({"from": [0, 0], "to": [19, 0], "colors": [[0.5, 0.5, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0]], "dither": dither}))
                .unwrap();
            (0..19).map(|x| px(&s, x, 5)[0]).collect::<Vec<f32>>()
        };
        let smooth = run(false);
        assert!(smooth.iter().all(|&v| v == smooth[0]), "no dither: perfectly flat");
        let noisy = run(true);
        assert!(noisy.windows(2).any(|w| w[0] != w[1]), "dither: per-pixel variation");
    }

    /// The two commands under test, each painting #808080 over a white document.
    fn grey_fill(cmd: &str, opacity: Value) -> Value {
        match cmd {
            "paint.bucket" => json!({"x": 2, "y": 2, "color": "#808080", "opacity": opacity}),
            _ => json!({"from": [0, 0], "to": [19, 0], "colors": ["#808080", "#808080"], "dither": false, "opacity": opacity}),
        }
    }

    fn white(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10, "depth": depth, "background": "white"})).unwrap();
        s
    }

    /// Issue #4167: an opacity above 100 overshot the blend (200 painted near-black on white).
    #[test]
    fn bucket_and_gradient_clamp_opacity_like_the_magic_eraser() {
        for cmd in ["paint.bucket", "paint.gradient"] {
            for depth in [8, 16, 32] {
                let mut full = white(depth);
                full.execute(cmd, grey_fill(cmd, json!(100))).unwrap();
                let expected = px(&full, 5, 5);
                assert!((expected[0] - 128.0 / 255.0).abs() < 0.01, "{cmd} depth {depth}: 100% paints #808080, got {expected:?}");
                for over in [200.0, 1e9] {
                    let mut s = white(depth);
                    s.execute(cmd, grey_fill(cmd, json!(over))).unwrap();
                    assert_eq!(px(&s, 5, 5), expected, "{cmd} depth {depth}: opacity {over} clamps to 100");
                }
                let (mut low, mut one) = (white(depth), white(depth));
                low.execute(cmd, grey_fill(cmd, json!(-50))).unwrap();
                one.execute(cmd, grey_fill(cmd, json!(1))).unwrap();
                assert_eq!(px(&low, 5, 5), px(&one, 5, 5), "{cmd} depth {depth}: opacity below 1 clamps to 1");
                assert!(px(&low, 5, 5).iter().all(|v| (0.0..=1.0).contains(v)), "{cmd} depth {depth}: no overshoot");
            }
        }
    }

    #[test]
    fn bucket_and_gradient_reject_non_numeric_opacity_without_editing() {
        for cmd in ["paint.bucket", "paint.gradient"] {
            for bad in [json!("50"), json!(true), json!([50]), json!({"v": 50})] {
                let mut s = white(8);
                let (before, steps) = (px(&s, 5, 5), s.active().unwrap().history.past_len());
                let err = s.execute(cmd, grey_fill(cmd, bad.clone())).unwrap_err();
                assert!(matches!(err, EngineError::BadParams { .. }), "{cmd} {bad}: {err:?}");
                assert_eq!(px(&s, 5, 5), before, "{cmd} {bad}: pixels unchanged");
                assert_eq!(s.active().unwrap().history.past_len(), steps, "{cmd} {bad}: no history step");
            }
            // An explicit null keeps the default (100%), like the Magic Eraser.
            let mut s = white(8);
            s.execute(cmd, grey_fill(cmd, Value::Null)).unwrap();
            assert!((px(&s, 5, 5)[0] - 128.0 / 255.0).abs() < 0.01, "{cmd}: null opacity is 100%");
        }
    }
}
