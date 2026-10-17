//! Owned native fixed-color preparation recipe. Immutable tips remain shared;
//! arbitrary embedded sample payloads are decoded cooperatively, not by serde.
use super::*;
use crate::preset_store::{PreparedTip, TipPreparationLimits, TipStoreWitness};
use photocraft_paint::stroke_controlled::StrokeLimits;
use photocraft_raster::Interrupt;
use std::io::Write;
fn fail(m: impl Into<String>) -> EngineError {
    bad("paint.stroke", m)
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| fail("brush recipe accounting"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| fail("brush recipe accounting"))
}
fn string(s: &str, c: &Interrupt<'_>) -> Result<String> {
    let mut v = String::new();
    v.try_reserve_exact(s.len()).map_err(|_| fail("brush string allocation"))?;
    for (index, ch) in s.chars().enumerate() {
        if index.is_multiple_of(1024) {
            poll(c)?;
        }
        v.push(ch);
    }
    Ok(v)
}
fn clone_json(v: &Value, c: &Interrupt<'_>) -> Result<Value> {
    poll(c)?;
    Ok(match v {
        Value::String(s) => Value::String(string(s, c)?),
        Value::Array(a) => {
            let mut out = Vec::new();
            out.try_reserve_exact(a.len()).map_err(|_| fail("brush array allocation"))?;
            for v in a {
                out.push(clone_json(v, c)?);
            }
            Value::Array(out)
        }
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, v) in o {
                poll(c)?;
                out.insert(string(k, c)?, clone_json(v, c)?);
            }
            Value::Object(out)
        }
        v => v.clone(),
    })
}
fn merge_json(base: &mut Value, patch: &Value, c: &Interrupt<'_>) -> Result<()> {
    poll(c)?;
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) if b.len() == 1 && p.len() == 1 && b.keys().next() != p.keys().next() => {
            *b = match clone_json(patch, c)? {
                Value::Object(v) => v,
                _ => return Err(fail("brush object")),
            }
        }
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                poll(c)?;
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => merge_json(bv, v, c)?,
                    _ => {
                        b.insert(string(k, c)?, clone_json(v, c)?);
                    }
                }
            }
        }
        (b, p) => *b = clone_json(p, c)?,
    }
    Ok(())
}
fn set(value: &mut Value, path: &[&str], replacement: Value) -> Result<()> {
    let (last, parents) = path.split_last().ok_or_else(|| fail("empty brush field path"))?;
    let mut target = value;
    for key in parents {
        target = target.get_mut(*key).filter(|v| v.is_object()).ok_or_else(|| fail("invalid brush field object"))?;
    }
    let map = target.as_object_mut().ok_or_else(|| fail("invalid brush object"))?;
    map.insert((*last).into(), replacement);
    Ok(())
}
fn rgba(p: &Value, default: [f32; 4]) -> [f32; 4] {
    match p.get("color") {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let get = |i| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            [get(0), get(1), get(2), if a.len() > 3 { get(3) } else { 1.0 }]
        }
        v => color(v, default),
    }
}
fn touches(p: &Value, path: &[&str]) -> bool {
    let mut v = p;
    for k in path {
        match v.get(k) {
            Some(x) => v = x,
            None => return !v.is_object(),
        }
    }
    true
}
fn reserve<T>(n: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| fail("brush buffer allocation"))?;
    Ok(v)
}
fn tile_json(t: &GrayTile, c: &Interrupt<'_>) -> Result<Value> {
    if t.data.len() as u64 > crate::preset_store::MAX_TIP_SAMPLES {
        return Err(fail("brush serialized tile sample limit"));
    }
    let bytes = t.data.len().checked_mul(2).ok_or_else(|| fail("brush tile length"))?;
    let cap = bytes.div_ceil(3).checked_mul(4).ok_or_else(|| fail("brush base64 length"))?;
    let mut raw = reserve(bytes)?;
    for chunk in t.data.chunks(256) {
        poll(c)?;
        for v in chunk {
            raw.extend_from_slice(&v.to_le_bytes());
        }
    }
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    out.try_reserve_exact(cap).map_err(|_| fail("brush base64 allocation"))?;
    for (index, chunk) in raw.chunks(3).enumerate() {
        if index.is_multiple_of(256) {
            poll(c)?;
        }
        let n = (u32::from(*chunk.first().ok_or_else(|| fail("brush sample"))?) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for shift in [18, 12] {
            out.push(char::from(*alphabet.get(((n >> shift) & 63) as usize).ok_or_else(|| fail("brush alphabet"))?));
        }
        out.push(if chunk.len() > 1 { char::from(*alphabet.get(((n >> 6) & 63) as usize).ok_or_else(|| fail("brush alphabet"))?) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(*alphabet.get((n & 63) as usize).ok_or_else(|| fail("brush alphabet"))?) } else { '=' });
    }
    Ok(json!({"width":t.width,"height":t.height,"data":out}))
}
fn stored_json(t: &StoredTile, c: &Interrupt<'_>) -> Result<Value> {
    Ok(json!({"key":string(&t.key,c)?,"width":t.width,"height":t.height,"preview":tile_json(&t.preview,c)?}))
}
fn tip_json(t: &TipShape, c: &Interrupt<'_>) -> Result<Value> {
    Ok(match t {
        TipShape::Round => json!("round"),
        TipShape::Sampled(t) => json!({"sampled":tile_json(t,c)?}),
        TipShape::Stored(t) => match &t.full {
            Some(g) => json!({"sampled":tile_json(g,c)?}),
            None => json!({"stored":stored_json(t,c)?}),
        },
    })
}
fn pattern_json(t: &Pattern, c: &Interrupt<'_>) -> Result<Value> {
    Ok(match t {
        Pattern::Tile(g) => json!({"tile":tile_json(g,c)?}),
        Pattern::Stored(t) => match &t.full {
            Some(g) => json!({"tile":tile_json(g,c)?}),
            None => json!({"stored":stored_json(t,c)?}),
        },
        t => serde_json::to_value(t).map_err(|e| fail(e.to_string()))?,
    })
}
fn tile(v: &Value, l: StrokeLimits, c: &Interrupt<'_>) -> Result<GrayTile> {
    let width = v.get("width").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| fail("brush tile width"))?;
    let height = v.get("height").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| fail("brush tile height"))?;
    let n = usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| fail("brush tile samples"))?;
    if n == 0
        || width > crate::preset_store::MAX_TIP_SIDE
        || height > crate::preset_store::MAX_TIP_SIDE
        || n as u64 > crate::preset_store::MAX_TIP_SAMPLES
        || mul(n as u64, 16)? > l.resident_bytes
        || mul(n as u64, 64)? > l.work_units
    {
        return Err(fail("brush embedded tile budget"));
    }
    let mut data = reserve(n)?;
    match v.get("data").ok_or_else(|| fail("brush tile data"))? {
        Value::Array(a) => {
            if a.len() != n {
                return Err(fail("brush tile samples mismatch"));
            }
            for chunk in a.chunks(256) {
                poll(c)?;
                for v in chunk {
                    let value = v.as_f64().ok_or_else(|| fail("brush tile sample type"))? as f32;
                    data.push((value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
                }
            }
        }
        Value::String(s) => {
            if s.len() as u64 > l.resident_bytes || mul(s.len() as u64, 16)? > l.work_units {
                return Err(fail("brush base64 budget"));
            }
            let mut raw = reserve(n.checked_mul(2).and_then(|v| v.checked_add(3)).ok_or_else(|| fail("brush decoded size"))?)?;
            let mut group = [0u8; 4];
            let mut used = 0usize;
            let mut visited = 0usize;
            for byte in s.bytes() {
                visited += 1;
                if visited.is_multiple_of(1024) {
                    poll(c)?;
                }
                if byte.is_ascii_whitespace() {
                    continue;
                }
                *group.get_mut(used).ok_or_else(|| fail("brush base64 group"))? = byte;
                used += 1;
                if used == 4 {
                    let pad = group.iter().rev().take_while(|b| **b == b'=').count();
                    let mut value = 0u32;
                    for (index, byte) in group.iter().enumerate() {
                        let digit = if index >= 4 - pad {
                            0
                        } else {
                            match byte {
                                b'A'..=b'Z' => u32::from(*byte - b'A'),
                                b'a'..=b'z' => u32::from(*byte - b'a' + 26),
                                b'0'..=b'9' => u32::from(*byte - b'0' + 52),
                                b'+' => 62,
                                b'/' => 63,
                                _ => return Err(fail("invalid brush base64")),
                            }
                        };
                        value = (value << 6) | digit;
                    }
                    if raw.len() + 3 > raw.capacity() {
                        return Err(fail("brush base64 length"));
                    }
                    raw.push((value >> 16) as u8);
                    if pad < 2 {
                        raw.push((value >> 8) as u8);
                    }
                    if pad < 1 {
                        raw.push(value as u8);
                    }
                    used = 0;
                }
            }
            if used != 0 || raw.len() / 2 != n {
                return Err(fail("brush decoded samples mismatch"));
            }
            for chunk in raw.chunks(4096) {
                poll(c)?;
                for bytes in chunk.as_chunks::<2>().0.iter() {
                    let pair: [u8; 2] = *bytes;
                    data.push(u16::from_le_bytes(pair));
                }
            }
        }
        _ => return Err(fail("brush tile data type")),
    }
    poll(c)?;
    Ok(GrayTile { width, height, data: data.into() })
}
fn stored(v: &Value, l: StrokeLimits, c: &Interrupt<'_>) -> Result<StoredTile> {
    Ok(StoredTile {
        key: string(v.get("key").and_then(Value::as_str).ok_or_else(|| fail("stored tip key"))?, c)?,
        width: v.get("width").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| fail("stored tip width"))?,
        height: v.get("height").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| fail("stored tip height"))?,
        preview: std::sync::Arc::new(tile(v.get("preview").ok_or_else(|| fail("stored tip preview"))?, l, c)?),
        full: None,
    })
}
fn tip(v: &Value, l: StrokeLimits, c: &Interrupt<'_>) -> Result<TipShape> {
    if v.as_object().is_some_and(|v| v.len() != 1) {
        return Err(fail("invalid brush tip variant"));
    }
    if let Some(v) = v.get("sampled") {
        Ok(TipShape::Sampled(tile(v, l, c)?))
    } else if let Some(v) = v.get("stored") {
        Ok(TipShape::Stored(stored(v, l, c)?))
    } else {
        serde_json::from_value(clone_json(v, c)?).map_err(|e| fail(e.to_string()))
    }
}
fn pattern(v: &Value, l: StrokeLimits, c: &Interrupt<'_>) -> Result<Pattern> {
    if v.as_object().is_some_and(|v| v.len() != 1) {
        return Err(fail("invalid brush pattern variant"));
    }
    if let Some(v) = v.get("tile") {
        Ok(Pattern::Tile(tile(v, l, c)?))
    } else if let Some(v) = v.get("stored") {
        Ok(Pattern::Stored(stored(v, l, c)?))
    } else {
        serde_json::from_value(clone_json(v, c)?).map_err(|e| fail(e.to_string()))
    }
}
fn effective_mode(base: &BrushSettings, cmd: &str, p: &Value) -> photocraft_color::BlendMode {
    if (cmd == "paint.pencil" && flag(p, "block", false)) || flag(p, "erase", false) || clear_mode(p) {
        return photocraft_color::BlendMode::Normal;
    }
    p.get("mode")
        .and_then(Value::as_str)
        .and_then(crate::commands::blend_from_str)
        .or_else(|| p.pointer("/brush/mode").and_then(Value::as_str).and_then(crate::commands::blend_from_str))
        .unwrap_or(base.mode)
}
pub(super) struct Recipe {
    base: BrushSettings,
    inputs: Vec<(String, std::result::Result<PreparedTip, String>)>,
    pub witnesses: Vec<TipStoreWitness>,
    pub retained_bytes: u64,
    foreground: [f32; 4],
    background: [f32; 4],
    picked: Option<([f32; 3], [f32; 4])>,
}
impl Recipe {
    pub(super) fn capture_for(s: &Session, cmd: &str, p: &Value, c: &Interrupt<'_>) -> Result<Self> {
        stroke_controlled::dispatch_preflight(s, cmd, p, false)?;
        poll(c)?;
        let mut selected = &s.tools.brush;
        let mut preset = None;
        if let Some(name) = p.get("preset").and_then(Value::as_str) {
            if name.len() > 4096 || s.tools.presets.len() > 200_000 {
                return Err(fail("brush preset lookup limit"));
            }
            for candidate in &s.tools.presets {
                poll(c)?;
                if candidate.name.eq_ignore_ascii_case(name) {
                    preset = Some(candidate);
                    break;
                }
            }
            selected = &preset.ok_or_else(|| fail("brush preset not found"))?.brush;
        }
        let standard = StrokeLimits::default();
        let req = StrokeRenderer::new_requirements(selected, None, 1.0, standard, c).map_err(|e| fail(e.to_string()))?;
        let current = StrokeRenderer::new_requirements(&s.tools.brush, None, 1.0, standard, c).map_err(|e| fail(e.to_string()))?;
        let mut retained_bytes = add(add(req.retained_bytes, req.staged_bytes)?, add(current.retained_bytes, current.staged_bytes)?)?;
        if retained_bytes > standard.resident_bytes / 4 {
            return Err(fail("brush recipe capture limit"));
        }
        // Fixed-size settings + shared Arc samples; string clone storage admitted.
        let base = match preset {
            Some(preset) => preset.picked_over(&s.tools.brush),
            None => s.tools.brush.clone(),
        };
        poll(c)?;
        let state = s.active().ok_or(EngineError::NoDocument)?;
        let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
        super::wide_one_shot_controlled::admit_blend_source(&state.doc, id, p, effective_mode(&base, cmd, p), c)?;
        let mut keys: Vec<String> = reserve(6)?;
        for t in [&base.tip, &base.dual_brush.tip] {
            if let TipShape::Stored(t) = t
                && t.full.is_none()
            {
                keys.push(string(&t.key, c)?);
            }
        }
        if let Pattern::Stored(t) = &base.texture.pattern
            && t.full.is_none()
        {
            keys.push(string(&t.key, c)?);
        }
        for path in ["/brush/tip/stored/key", "/brush/dualBrush/tip/stored/key", "/brush/texture/pattern/stored/key"] {
            if let Some(key) = p.pointer(path).and_then(Value::as_str) {
                if key.len() > 128 {
                    return Err(fail("stored tip key limit"));
                }
                keys.push(string(key, c)?);
            }
        }
        let mut inputs: Vec<(String, std::result::Result<PreparedTip, String>)> = reserve(keys.len())?;
        let mut witnesses = reserve(keys.len())?;
        for key in keys {
            poll(c)?;
            if inputs.iter().any(|(old, _)| old == &key) {
                continue;
            }
            let prepared = match s.preset_store.as_ref() {
                Some(store) => store
                    .prepare_tip(&key, (standard.resident_bytes / 4).checked_sub(retained_bytes).ok_or_else(|| fail("tip capture headroom"))?, c)
                    .map_err(|e| e.to_string()),
                None => Err("no preset store is attached".into()),
            };
            if let Ok(tip) = &prepared {
                retained_bytes = add(retained_bytes, tip.held_bytes)?;
                if retained_bytes > standard.resident_bytes / 4 {
                    return Err(fail("tip captured bytes limit"));
                }
                witnesses.push(tip.witness());
            }
            inputs.push((key, prepared));
        }
        let rgba = rgba(p, s.tools.foreground);
        let picked = s.tools.picked_cmyk([rgba[0], rgba[1], rgba[2]]).map(|v| ([rgba[0], rgba[1], rgba[2]], v));
        Ok(Self { base, inputs, witnesses, retained_bytes, foreground: s.tools.foreground, background: s.tools.background, picked })
    }
    pub(super) fn effective_mode(&self, cmd: &str, p: &Value) -> photocraft_color::BlendMode {
        effective_mode(&self.base, cmd, p)
    }
    pub(super) fn resolve_for(mut self, cmd: &str, p: &Value, l: StrokeLimits, c: &Interrupt<'_>) -> Result<(BrushSettings, Vec<StrokePoint>)> {
        poll(c)?;
        let mut base = self.base;
        let param_bytes = super::wide_one_shot_controlled::parameter_bytes_with_interrupt(p, c)?;
        let mut config_work = mul(param_bytes, 16)?;
        let mut config_bytes = mul(param_bytes, 4)?;
        if let Some(patch) = p.get("brush").filter(|v| v.is_object()) {
            for (path, bitmap) in [
                (&["tip"][..], base.tip.bitmap()),
                (&["dualBrush", "tip"][..], base.dual_brush.tip.bitmap()),
                (&["texture", "pattern"][..], base.texture.pattern.bitmap()),
            ] {
                if touches(patch, path)
                    && let Some(bitmap) = bitmap
                {
                    config_work = add(config_work, mul(bitmap.data.len() as u64, 128)?)?;
                    config_bytes = add(config_bytes, mul(bitmap.data.len() as u64, 32)?)?;
                }
            }
        }
        if config_work > l.work_units / 4 || config_bytes > l.resident_bytes {
            return Err(fail("aggregate brush merge/seed budget"));
        }
        if let Some(patch) = p.get("brush").filter(|v| v.is_object()) {
            let tip_keep = (!touches(patch, &["tip"])).then(|| base.tip.clone());
            let dual_keep = (!touches(patch, &["dualBrush", "tip"])).then(|| base.dual_brush.tip.clone());
            let pattern_keep = (!touches(patch, &["texture", "pattern"])).then(|| base.texture.pattern.clone());
            let tip_value = if tip_keep.is_none() { Some(tip_json(&base.tip, c)?) } else { None };
            let dual_value = if dual_keep.is_none() { Some(tip_json(&base.dual_brush.tip, c)?) } else { None };
            let pattern_value = if pattern_keep.is_none() { Some(pattern_json(&base.texture.pattern, c)?) } else { None };
            base.tip = TipShape::Round;
            base.dual_brush.tip = TipShape::Round;
            base.texture.pattern = Pattern::default();
            // The remaining fixed brush settings contain no bitmap payload;
            // serde is bounded to this small structural boundary, not a deep token promise.
            let mut value = serde_json::to_value(&base).map_err(|e| fail(e.to_string()))?;
            if let Some(v) = tip_value {
                set(&mut value, &["tip"], v)?;
            }
            if let Some(v) = dual_value {
                set(&mut value, &["dualBrush", "tip"], v)?;
            }
            if let Some(v) = pattern_value {
                set(&mut value, &["texture", "pattern"], v)?;
            }
            merge_json(&mut value, patch, c)?;
            let t = match tip_keep {
                Some(t) => t,
                None => tip(value.get("tip").ok_or_else(|| fail("brush tip missing"))?, l, c)?,
            };
            let d = match dual_keep {
                Some(t) => t,
                None => tip(value.pointer("/dualBrush/tip").ok_or_else(|| fail("dual tip missing"))?, l, c)?,
            };
            let pat = match pattern_keep {
                Some(t) => t,
                None => pattern(value.pointer("/texture/pattern").ok_or_else(|| fail("pattern missing"))?, l, c)?,
            };
            set(&mut value, &["tip"], json!("round"))?;
            set(&mut value, &["dualBrush", "tip"], json!("round"))?;
            set(&mut value, &["texture", "pattern"], serde_json::to_value(Pattern::default()).map_err(|e| fail(e.to_string()))?)?;
            if super::wide_one_shot_controlled::parameter_bytes_with_interrupt(&value, c)? > 64 << 10 {
                return Err(fail("brush structural config limit"));
            }
            poll(c)?;
            base = serde_json::from_value(value).map_err(|e| fail(e.to_string()))?;
            poll(c)?;
            base.tip = t;
            base.dual_brush.tip = d;
            base.texture.pattern = pat;
        }
        let mut planned = 0u64;
        for t in [&base.tip, &base.dual_brush.tip] {
            poll(c)?;
            if let Some(bitmap) = t.bitmap() {
                planned = add(planned, mul(bitmap.data.len() as u64, 2)?)?;
            }
            if let TipShape::Stored(t) = t
                && t.full.is_none()
            {
                planned = add(planned, mul(u64::from(t.width) * u64::from(t.height), 8)?)?;
            }
        }
        if let Some(bitmap) = base.texture.pattern.bitmap() {
            planned = add(planned, mul(bitmap.data.len() as u64, 2)?)?;
        }
        if let Pattern::Stored(t) = &base.texture.pattern
            && t.full.is_none()
        {
            planned = add(planned, mul(u64::from(t.width) * u64::from(t.height), 8)?)?;
        }
        let tip_limits = StrokeLimits {
            resident_bytes: l.resident_bytes.checked_sub(planned).ok_or_else(|| fail("aggregate tip decode headroom"))?,
            work_units: l.work_units / 4,
        };
        for t in [&mut base.tip, &mut base.dual_brush.tip] {
            if let TipShape::Stored(t) = t {
                load(t, &mut self.inputs, tip_limits, c)?;
            }
        }
        if let Pattern::Stored(t) = &mut base.texture.pattern {
            load(t, &mut self.inputs, tip_limits, c)?;
        }
        if let Some(v) = num(p, "size") {
            base.size = v.max(0.5);
        }
        if let Some(v) = num(p, "hardness") {
            base.hardness = v.clamp(0.0, 1.0);
        }
        if let Some(v) = num(p, "opacity") {
            base.opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = num(p, "flow") {
            base.flow = v.clamp(0.0, 1.0);
        }
        if let Some(v) = num(p, "spacing") {
            base.spacing = v.clamp(0.01, 10.0);
        }
        if let Some(v) = num(p, "smoothing") {
            base.smoothing.enabled = true;
            base.smoothing.amount = v.clamp(0.0, 1.0);
        }
        base.color = rgba(p, self.foreground);
        base.background = self.background;
        base.color_cmyk = self.picked;
        base.erase = flag(p, "erase", false);
        struct Seed<'a, 'b> {
            hash: u64,
            c: &'a Interrupt<'b>,
        }
        impl Write for Seed<'_, '_> {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                for chunk in b.chunks(4096) {
                    self.c.check().map_err(|_| std::io::Error::other("cancelled"))?;
                    for byte in chunk {
                        self.hash ^= u64::from(*byte);
                        self.hash = self.hash.wrapping_mul(0x00000100000001B3);
                    }
                }
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        base.seed = match p.get("seed").and_then(Value::as_u64) {
            Some(v) => v,
            None => {
                let mut writer = Seed { hash: 0xCBF29CE484222325, c };
                if let Some(points) = p.get("points") {
                    serde_json::to_writer(&mut writer, points).map_err(|e| if c.cancelled() { EngineError::Cancelled } else { fail(e.to_string()) })?;
                }
                photocraft_paint::rng::mix64(writer.hash)
            }
        };
        validate_brush(&base, cmd)?;
        base = with_blend_mode(base, p);
        if cmd == "paint.pencil" {
            base = pencil_settings(base, p)?;
        }
        let array = p.get("points").and_then(Value::as_array).ok_or_else(|| fail("missing points"))?;
        let mut points = reserve(array.len())?;
        for value in array {
            poll(c)?;
            let point = match value {
                Value::Array(a) => {
                    let g = |i| a.get(i).and_then(Value::as_f64);
                    match (g(0), g(1)) {
                        (Some(x), Some(y)) => {
                            let mut q = StrokePoint::new(x, y, g(2).unwrap_or(1.0) as f32);
                            q.tilt_x = g(3).unwrap_or(0.0) as f32;
                            q.tilt_y = g(4).unwrap_or(0.0) as f32;
                            q.rotation = g(5).unwrap_or(0.0) as f32;
                            q.time = g(6).unwrap_or(0.0);
                            q.wheel = g(7).unwrap_or(1.0) as f32;
                            Some(q)
                        }
                        _ => None,
                    }
                }
                Value::Object(_) => serde_json::from_value(clone_json(value, c)?).ok(),
                _ => None,
            };
            if let Some(point) = point {
                points.push(point);
            }
        }
        if points.is_empty() {
            return Err(fail("points is empty"));
        }
        check_coords(&points, cmd)?;
        poll(c)?;
        Ok((base, points))
    }
}
fn load(t: &mut StoredTile, inputs: &mut [(String, std::result::Result<PreparedTip, String>)], l: StrokeLimits, c: &Interrupt<'_>) -> Result<()> {
    if t.full.is_some() {
        return Ok(());
    }
    poll(c)?;
    let at = inputs.iter().position(|(key, _)| key == &t.key).ok_or_else(|| fail("stored tip has no captured input"))?;
    let (_, input) = inputs.get_mut(at).ok_or_else(|| fail("stored input missing"))?;
    let prepared = input.as_mut().map_err(|e| fail(e.clone()))?;
    if prepared.key() != t.key {
        return Err(fail("stored input key changed"));
    }
    t.full = Some(prepared.decode(t.width, t.height, TipPreparationLimits { resident_bytes: l.resident_bytes, work_units: l.work_units }, c)?);
    Ok(())
}

/// Closed accounted Round recipe. Bitmap and preset workflows remain on their
/// existing resident routes; this owns only prospectively admitted scalar data.
pub(crate) struct AccountedRoundRecipe {
    pub(crate) brush: BrushSettings,
    pub(crate) points: Vec<StrokePoint>,
    original: BrushSettings,
    foreground: [u32; 4],
    background: [u32; 4],
    _fee: photocraft_raster::backing::quota::QuotaLease,
}
impl AccountedRoundRecipe {
    pub(crate) fn tools_match(&self, s: &Session) -> bool {
        s.tools.foreground.map(f32::to_bits) == self.foreground
            && s.tools.background.map(f32::to_bits) == self.background
            && photocraft_paint::native_stroke_controlled::checked_round_brush(&s.tools.brush).is_ok_and(|b| b == self.original)
    }
}
fn accounted_shape(v: &Value, depth: usize, n: &mut usize, c: &photocraft_raster::backing::ReadControl<'_>) -> Result<u64> {
    c.check(1)?;
    *n = n.checked_add(1).ok_or(photocraft_raster::backing::BackingError::Length)?;
    if depth > 8 || *n > 4096 {
        return Err(fail("native brush configuration shape"));
    }
    let mut bytes = std::mem::size_of::<Value>() as u64;
    match v {
        Value::String(s) => {
            if s.len() > 4096 {
                return Err(fail("native brush string limit"));
            }
            c.check(s.len() as u64)?;
            bytes = add(bytes, s.len() as u64)?
        }
        Value::Array(a) => {
            if a.len() > 4096 {
                return Err(fail("native brush array limit"));
            }
            for v in a {
                bytes = add(bytes, accounted_shape(v, depth + 1, n, c)?)?
            }
        }
        Value::Object(o) => {
            if o.len() > 128 {
                return Err(fail("native brush field limit"));
            }
            for (k, v) in o {
                c.check(k.len() as u64)?;
                bytes = add(bytes, add(k.len() as u64 + 256, accounted_shape(v, depth + 1, n, c)?)?)?
            }
        }
        _ => {}
    }
    if bytes > 64 << 10 {
        return Err(fail("native brush structural config limit"));
    }
    Ok(bytes)
}
pub(crate) fn accounted_round_recipe(
    s: &Session,
    cmd: &str,
    p: &Value,
    q: &photocraft_raster::backing::quota::BackingQuota,
    c: &photocraft_raster::backing::ReadControl<'_>,
) -> Result<AccountedRoundRecipe> {
    use photocraft_raster::backing::{BackingError, quota::QuotaKind};
    c.check(1)?;
    if p.get("preset").is_some() || p.get("autoErase").is_some_and(|v| v.as_bool() != Some(false)) || p.get("block").is_some_and(|v| v.as_bool() != Some(false))
    {
        return Err(fail("native Round stroke does not yet support presets, Auto Erase or block mode"));
    }
    let original = photocraft_paint::native_stroke_controlled::checked_round_brush(&s.tools.brush).map_err(|e| fail(e.to_string()))?;
    let array = p.get("points").and_then(Value::as_array).filter(|a| !a.is_empty() && a.len() <= 100_000).ok_or_else(|| fail("bounded points required"))?;
    let patch = p.get("brush").filter(|v| v.is_object());
    let shape = if let Some(v) = patch { accounted_shape(v, 0, &mut 0, c)? } else { 0 };
    if let Some(v) = patch
        && v.as_object().is_some_and(|o| o.keys().any(|k| matches!(k.as_str(), "tip" | "texture" | "dualBrush" | "scattering" | "colorDynamics" | "mixer")))
    {
        return Err(fail("native Round stroke asset/advanced patch is unsupported"));
    }
    // Pay and bound every point value before streamed seed serialization or any
    // recursive serde use; direct Prepared callers also take this path.
    for v in array {
        match v {
            Value::Array(a) if a.len() <= 8 => {
                for v in a {
                    accounted_shape(v, 0, &mut 0, c)?;
                }
            }
            Value::Object(_) => {
                accounted_shape(v, 0, &mut 0, c)?;
            }
            Value::Array(_) => return Err(fail("native point field limit")),
            _ => {
                accounted_shape(v, 0, &mut 0, c)?;
            }
        }
    }
    let bytes = add(add(mul(array.len() as u64, std::mem::size_of::<StrokePoint>() as u64 * 2)?, mul(shape, 16)?)?, 1 << 20)?;
    let fee = q.reserve(QuotaKind::Index, bytes).map_err(|_| BackingError::Quota)?;
    c.check(add(mul(array.len() as u64, 256)?, mul(shape, 128)?)?)?;
    let mut base = photocraft_paint::native_stroke_controlled::checked_round_brush(&s.tools.brush).map_err(|e| fail(e.to_string()))?;
    if let Some(patch) = patch {
        let mut value = serde_json::to_value(&base).map_err(|e| fail(e.to_string()))?;
        merge_json(&mut value, patch, c.interrupt)?;
        base = serde_json::from_value(value).map_err(|e| fail(e.to_string()))?;
    }
    if let Some(v) = num(p, "size") {
        base.size = v.max(0.5)
    }
    if let Some(v) = num(p, "hardness") {
        base.hardness = v.clamp(0.0, 1.0)
    }
    if let Some(v) = num(p, "opacity") {
        base.opacity = v.clamp(0.0, 1.0)
    }
    if let Some(v) = num(p, "flow") {
        base.flow = v.clamp(0.0, 1.0)
    }
    if let Some(v) = num(p, "spacing") {
        base.spacing = v.clamp(0.01, 10.0)
    }
    if let Some(v) = num(p, "smoothing") {
        base.smoothing.enabled = true;
        base.smoothing.amount = v.clamp(0.0, 1.0)
    }
    base.color = rgba(p, s.tools.foreground);
    base.background = s.tools.background;
    base.color_cmyk = None;
    base.erase = flag(p, "erase", false);
    struct PaidSeed<'a, 'b> {
        hash: u64,
        c: &'a photocraft_raster::backing::ReadControl<'b>,
        failure: Option<BackingError>,
    }
    impl Write for PaidSeed<'_, '_> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            for chunk in b.chunks(4096) {
                if let Err(e) = self.c.check(chunk.len() as u64) {
                    self.failure = Some(e);
                    return Err(std::io::Error::other("controlled seed"));
                }
                for byte in chunk {
                    self.hash ^= u64::from(*byte);
                    self.hash = self.hash.wrapping_mul(0x00000100000001B3)
                }
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    base.seed = if let Some(seed) = p.get("seed").and_then(Value::as_u64) {
        seed
    } else {
        let mut writer = PaidSeed { hash: 0xCBF29CE484222325, c, failure: None };
        if let Some(points) = p.get("points")
            && let Err(e) = serde_json::to_writer(&mut writer, points)
        {
            return Err(writer.failure.map_or_else(|| fail(e.to_string()), EngineError::from));
        }
        photocraft_paint::rng::mix64(writer.hash)
    };
    validate_brush(&base, cmd)?;
    base = with_blend_mode(base, p);
    if cmd == "paint.pencil" {
        base = pencil_settings(base, p)?
    }
    base = photocraft_paint::native_stroke_controlled::checked_round_brush(&base).map_err(|e| fail(e.to_string()))?;
    let mut points = Vec::new();
    points.try_reserve_exact(array.len()).map_err(|_| BackingError::Allocation)?;
    if points.capacity() > array.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for value in array {
        c.check(1)?;
        let point = match value {
            Value::Array(a) => {
                if a.len() > 8 {
                    return Err(fail("native point field limit"));
                }
                let g = |i| a.get(i).and_then(Value::as_f64);
                match (g(0), g(1)) {
                    (Some(x), Some(y)) => {
                        let mut v = StrokePoint::new(x, y, g(2).unwrap_or(1.0) as f32);
                        v.tilt_x = g(3).unwrap_or(0.0) as f32;
                        v.tilt_y = g(4).unwrap_or(0.0) as f32;
                        v.rotation = g(5).unwrap_or(0.0) as f32;
                        v.time = g(6).unwrap_or(0.0);
                        v.wheel = g(7).unwrap_or(1.0) as f32;
                        Some(v)
                    }
                    _ => None,
                }
            }
            Value::Object(_) => {
                let shape = accounted_shape(value, 0, &mut 0, c)?;
                c.check(mul(shape, 32)?)?;
                serde_json::from_value(clone_json(value, c.interrupt)?).ok()
            }
            _ => None,
        };
        if let Some(point) = point {
            points.push(point)
        }
    }
    if points.is_empty() {
        return Err(fail("points is empty"));
    }
    check_coords(&points, cmd)?;
    c.check(1)?;
    Ok(AccountedRoundRecipe {
        brush: base,
        points,
        original,
        foreground: s.tools.foreground.map(f32::to_bits),
        background: s.tools.background.map(f32::to_bits),
        _fee: fee,
    })
}

// Wide fixed paint stays separate from inert finite geometry; preserve the
// existing narrow resolver above literally, including its color semantics.
pub(crate) fn accounted_wide_round_recipe(
    s: &Session,
    cmd: &str,
    p: &Value,
    q: &photocraft_raster::backing::quota::BackingQuota,
    c: &photocraft_raster::backing::ReadControl<'_>,
) -> Result<(AccountedRoundRecipe, [f64; 4])> {
    use photocraft_raster::backing::{BackingError, quota::QuotaKind};
    c.check(128)?;
    let rgba = crate::paint_cmds::wide_color(p.get("color"), s.tools.foreground)?;
    c.check(1)?;
    if p.get("preset").is_some() || p.get("autoErase").is_some_and(|v| v.as_bool() != Some(false)) || p.get("block").is_some_and(|v| v.as_bool() != Some(false))
    {
        return Err(fail("native Round stroke does not yet support presets, Auto Erase or block mode"));
    }
    let original = photocraft_paint::native_stroke_controlled::checked_round_brush(&s.tools.brush).map_err(|e| fail(e.to_string()))?;
    let array = p.get("points").and_then(Value::as_array).filter(|a| !a.is_empty() && a.len() <= 100_000).ok_or_else(|| fail("bounded points required"))?;
    let patch = p.get("brush").filter(|v| v.is_object());
    let shape = if let Some(v) = patch { accounted_shape(v, 0, &mut 0, c)? } else { 0 };
    if let Some(v) = patch
        && v.as_object().is_some_and(|o| o.keys().any(|k| matches!(k.as_str(), "tip" | "texture" | "dualBrush" | "scattering" | "colorDynamics" | "mixer")))
    {
        return Err(fail("native Round stroke asset/advanced patch is unsupported"));
    }
    // Pay and bound every point value before streamed seed serialization or any
    // recursive serde use; direct Prepared callers also take this path.
    for v in array {
        match v {
            Value::Array(a) if a.len() <= 8 => {
                for v in a {
                    accounted_shape(v, 0, &mut 0, c)?;
                }
            }
            Value::Object(_) => {
                accounted_shape(v, 0, &mut 0, c)?;
            }
            Value::Array(_) => return Err(fail("native point field limit")),
            _ => {
                accounted_shape(v, 0, &mut 0, c)?;
            }
        }
    }
    let bytes = add(add(mul(array.len() as u64, std::mem::size_of::<StrokePoint>() as u64 * 2)?, mul(shape, 16)?)?, 1 << 20)?;
    let fee = q.reserve(QuotaKind::Index, bytes).map_err(|_| BackingError::Quota)?;
    c.check(add(mul(array.len() as u64, 256)?, mul(shape, 128)?)?)?;
    let mut base = photocraft_paint::native_stroke_controlled::checked_round_brush(&s.tools.brush).map_err(|e| fail(e.to_string()))?;
    if let Some(patch) = patch {
        let mut value = serde_json::to_value(&base).map_err(|e| fail(e.to_string()))?;
        merge_json(&mut value, patch, c.interrupt)?;
        base = serde_json::from_value(value).map_err(|e| fail(e.to_string()))?;
    }
    if let Some(v) = num(p, "size") {
        base.size = v.max(0.5)
    }
    if let Some(v) = num(p, "hardness") {
        base.hardness = v.clamp(0.0, 1.0)
    }
    if let Some(v) = num(p, "opacity") {
        base.opacity = v.clamp(0.0, 1.0)
    }
    if let Some(v) = num(p, "flow") {
        base.flow = v.clamp(0.0, 1.0)
    }
    if let Some(v) = num(p, "spacing") {
        base.spacing = v.clamp(0.01, 10.0)
    }
    if let Some(v) = num(p, "smoothing") {
        base.smoothing.enabled = true;
        base.smoothing.amount = v.clamp(0.0, 1.0)
    }
    base.color = [0.0, 0.0, 0.0, 1.0];
    base.background = s.tools.background;
    base.color_cmyk = None;
    base.erase = flag(p, "erase", false);
    struct PaidSeed<'a, 'b> {
        hash: u64,
        c: &'a photocraft_raster::backing::ReadControl<'b>,
        failure: Option<BackingError>,
    }
    impl Write for PaidSeed<'_, '_> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            for chunk in b.chunks(4096) {
                if let Err(e) = self.c.check(chunk.len() as u64) {
                    self.failure = Some(e);
                    return Err(std::io::Error::other("controlled seed"));
                }
                for byte in chunk {
                    self.hash ^= u64::from(*byte);
                    self.hash = self.hash.wrapping_mul(0x00000100000001B3)
                }
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    base.seed = if let Some(seed) = p.get("seed").and_then(Value::as_u64) {
        seed
    } else {
        let mut writer = PaidSeed { hash: 0xCBF29CE484222325, c, failure: None };
        if let Some(points) = p.get("points")
            && let Err(e) = serde_json::to_writer(&mut writer, points)
        {
            return Err(writer.failure.map_or_else(|| fail(e.to_string()), EngineError::from));
        }
        photocraft_paint::rng::mix64(writer.hash)
    };
    validate_brush(&base, cmd)?;
    base = with_blend_mode(base, p);
    if cmd == "paint.pencil" {
        base = pencil_settings(base, p)?
    }
    base = photocraft_paint::native_stroke_controlled::checked_round_brush(&base).map_err(|e| fail(e.to_string()))?;
    let mut points = Vec::new();
    points.try_reserve_exact(array.len()).map_err(|_| BackingError::Allocation)?;
    if points.capacity() > array.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for value in array {
        c.check(1)?;
        let point = match value {
            Value::Array(a) => {
                if a.len() > 8 {
                    return Err(fail("native point field limit"));
                }
                let g = |i| a.get(i).and_then(Value::as_f64);
                match (g(0), g(1)) {
                    (Some(x), Some(y)) => {
                        let mut v = StrokePoint::new(x, y, g(2).unwrap_or(1.0) as f32);
                        v.tilt_x = g(3).unwrap_or(0.0) as f32;
                        v.tilt_y = g(4).unwrap_or(0.0) as f32;
                        v.rotation = g(5).unwrap_or(0.0) as f32;
                        v.time = g(6).unwrap_or(0.0);
                        v.wheel = g(7).unwrap_or(1.0) as f32;
                        Some(v)
                    }
                    _ => None,
                }
            }
            Value::Object(_) => {
                let shape = accounted_shape(value, 0, &mut 0, c)?;
                c.check(mul(shape, 32)?)?;
                serde_json::from_value(clone_json(value, c.interrupt)?).ok()
            }
            _ => None,
        };
        if let Some(point) = point {
            points.push(point)
        }
    }
    if points.is_empty() {
        return Err(fail("points is empty"));
    }
    check_coords(&points, cmd)?;
    c.check(1)?;
    Ok((
        AccountedRoundRecipe {
            brush: base,
            points,
            original,
            foreground: s.tools.foreground.map(f32::to_bits),
            background: s.tools.background.map(f32::to_bits),
            _fee: fee,
        },
        rgba,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        s.tools.brush.tip = TipShape::Sampled(GrayTile { width: 3, height: 1, data: vec![0, 32769, 65535].into() });
        s
    }
    #[test]
    fn faithful_patch_and_stream_seed_match_existing_resolver() {
        let s = fixture();
        for patch in [json!({"size":7,"spacing":0.37}), json!({"tip":{"sampled":{"width":2,"height":1,"data":[0.25,0.75]}}}), json!({"tip":"round"})] {
            let p = json!({"points":[[1.125,2.25,0.5],{"x":3.25,"y":4.5,"pressure":0.75}],"color":[0.3,0.6,0.2,0.8],"brush":patch});
            let expected = with_blend_mode(resolve_brush(&s, &p, "paint.stroke").unwrap(), &p);
            let points = parse_points(&p, "paint.stroke").unwrap();
            let recipe = Recipe::capture_for(&s, "paint.stroke", &p, &Interrupt::NONE).unwrap();
            let (actual, actual_points) = recipe.resolve_for("paint.stroke", &p, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(serde_json::to_value(actual_points).unwrap(), serde_json::to_value(points).unwrap());
        }
    }
    #[test]
    fn pencil_and_block_recipe_match_existing_forced_settings_and_seed() {
        let s = fixture();
        for patch in
            [json!({}), json!({"hardness":0.3,"flow":0.4}), json!({"block":true,"zoom":2.5,"mode":"multiply"}), json!({"block":true,"zoom":0.125,"erase":true})]
        {
            let mut p = json!({"points":[[1.125,2.25,0.5],[3.25,4.5,0.75]],"seed":79});
            for (k, v) in patch.as_object().unwrap() {
                p[k] = v.clone();
            }
            let expected = pencil_brush(&s, &p).unwrap();
            let recipe = Recipe::capture_for(&s, "paint.pencil", &p, &Interrupt::NONE).unwrap();
            assert_eq!(recipe.effective_mode("paint.pencil", &p), photocraft_color::BlendMode::Normal);
            let (actual, _) = recipe.resolve_for("paint.pencil", &p, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn retained_tip_arc_and_cancelled_recipe_keep_authoritative_source() {
        let s = fixture();
        let p = json!({"points":[[2,2,1]],"brush":{"size":4}});
        let old = s.tools.brush.tip.bitmap().unwrap().data.clone();
        let recipe = Recipe::capture_for(&s, "paint.stroke", &p, &Interrupt::NONE).unwrap();
        let (brush, _) = recipe.resolve_for("paint.stroke", &p, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert!(std::sync::Arc::ptr_eq(&old, &brush.tip.bitmap().unwrap().data));
        let recipe = Recipe::capture_for(&s, "paint.stroke", &p, &Interrupt::NONE).unwrap();
        let cancel = || true;
        assert!(matches!(recipe.resolve_for("paint.stroke", &p, StrokeLimits::default(), &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert!(std::sync::Arc::ptr_eq(&old, &s.tools.brush.tip.bitmap().unwrap().data));
    }
}
