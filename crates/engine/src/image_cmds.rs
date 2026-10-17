//! Image menu: Image Size, Canvas Size, Crop, Trim, Mode (colour model and
//! bit depth) conversions, Duplicate.

use photocraft_algo::resample::Resample;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Effect, Effects, FxPaint, Layer, LayerContent, LayerId, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

mod depth_controlled;
pub(crate) mod trim_controlled;
pub(crate) fn depth_dispatch_preflight(s: &Session, id: &str, p: &Value, background: bool) -> Result<bool> {
    depth_controlled::dispatch_preflight(s, id, p, background)
}

impl Session {
    /// Bind geometry view state to the same private edit as its pixels. Restore
    /// any caller state even if cancellation happens before the edit consumes it.
    pub(crate) fn edit_with_sampler_map<R>(
        &mut self,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        map: Option<photocraft_geom::Affine>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if map.is_some_and(|map| map.m.iter().any(|value| !value.is_finite())) {
            return Err(EngineError::Other("nonfinite sampler geometry".into()));
        }
        let previous = std::mem::replace(&mut self.sampler_map, map);
        let result = self.edit_with_interrupt(label, control, f);
        self.sampler_map = previous;
        result
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Applies `f` to every pixel surface of a layer tree: content (raster or
/// cached text/shape/smart pixels), fill caches, and (if `masks`) masks.
pub(crate) fn for_each_surface(layers: &mut [Layer], masks: bool, f: &mut dyn FnMut(&mut Surface, bool)) {
    for l in layers {
        if masks && let Some(m) = &mut l.mask {
            f(std::sync::Arc::make_mut(&mut m.surface), true);
        }
        if let Some(fc) = &mut l.fill_cache {
            f(&mut fc.surface, false);
        }
        match &mut l.content {
            LayerContent::Raster(s) => f(s, false),
            LayerContent::Text(t) => {
                if let Some(c) = &mut t.cache {
                    f(c, false)
                }
            }
            LayerContent::Shape(sh) => {
                if let Some(c) = &mut sh.cache {
                    f(c, false)
                }
            }
            LayerContent::Smart(sm) => {
                if let Some(c) = &mut sm.cache {
                    f(c, false)
                }
                // Selection-derived smart-filter masks live in document coordinates. Every
                // canvas resize/translation must carry them with the smart object's cache;
                // otherwise toggling a filter recomputes its coverage at stale positions (#3077).
                if masks && let Some(mask) = &mut sm.filter_mask {
                    f(std::sync::Arc::make_mut(&mut mask.surface), true);
                }
            }
            LayerContent::Group(g) => for_each_surface(&mut g.children, masks, f),
            _ => {}
        }
    }
}

/// Converts every pixel surface of a layer tree (masks included) to `depth`.
pub(crate) fn convert_layers_depth(layers: &mut [Layer], depth: SampleType) -> Result<()> {
    let mut error = None;
    for_each_surface(layers, true, &mut |surf, _| {
        if error.is_some() {
            return;
        }
        let f = surf.format().with_sample(depth);
        match surf.try_convert(f) {
            Ok(converted) => *surf = converted,
            Err(e) => error = Some(EngineError::Other(format!("cannot change sample depth: {e}"))),
        }
    });
    error.map_or(Ok(()), Err)
}

pub(crate) fn for_each_layer(layers: &mut [Layer], f: &mut dyn FnMut(&mut Layer)) {
    for l in layers {
        f(l);
        if let Some(ch) = l.children_mut() {
            for_each_layer(ch, f);
        }
    }
}

/// Assign new layer ids to a copied document and update every saved layer reference.
///
/// A document copy needs fresh identities, but its comps and variable definitions must
/// still target the copied layers. Deliberately leave references to already-deleted
/// layers untouched so their missing-layer warnings remain meaningful. The same applies
/// to Last Document State.
pub(crate) fn reidentify_copied_layers(doc: &mut Document) {
    let mut ids = std::collections::HashMap::new();
    for_each_layer(&mut doc.layers, &mut |layer| {
        let previous = layer.id;
        layer.id = LayerId::fresh();
        ids.insert(previous, layer.id);
    });

    let remap = |comp: &mut photocraft_doc::LayerComp| {
        for state in &mut comp.states {
            if let Some(&new_id) = ids.get(&state.layer) {
                state.layer = new_id;
            }
        }
    };
    for comp in &mut doc.layer_comps {
        remap(comp);
    }
    if let Some(last) = &mut doc.last_document_state {
        remap(last);
    }
    for def in &mut doc.variables.defs {
        if let Some(&new_id) = ids.get(&def.layer) {
            def.layer = new_id;
        }
    }
}

/// Scale every size and distance of a style by `k` (Image Size's Scale Styles, Layer Style â€º
/// Scale Effects, and previews that draw a style at a reduced resolution).
pub fn scale_effects(fx: &mut Effects, k: f32) {
    for e in &mut fx.items {
        match e {
            Effect::DropShadow(s) | Effect::InnerShadow(s) => {
                s.distance *= k;
                s.size *= k;
            }
            Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.size *= k,
            Effect::Stroke(s) => s.size *= k,
            Effect::Satin(s) => {
                s.distance *= k;
                s.size *= k;
            }
            Effect::BevelEmboss(b) => {
                b.size *= k;
                b.soften *= k;
            }
            Effect::PatternOverlay { scale, .. } => *scale *= k,
            Effect::GradientOverlay { .. } | Effect::ColorOverlay { .. } => {}
        }
        if let Effect::Stroke(s) = e
            && let FxPaint::Pattern { scale, .. } = &mut s.paint
        {
            *scale *= k;
        }
    }
    // Effects changed: the preserved PSD block no longer applies.
    if k != 1.0 {
        fx.psd_raw = None;
    }
}

fn parse_resample(name: &str) -> Result<Option<Resample>> {
    Ok(match name {
        "none" => None,
        "nearest" | "nearestNeighbor" => Some(Resample::Nearest),
        "bilinear" => Some(Resample::Bilinear),
        "bicubic" => Some(Resample::Bicubic),
        "bicubicSmoother" => Some(Resample::BicubicSmoother),
        "bicubicSharper" => Some(Resample::BicubicSharper),
        "bicubicAutomatic" | "automatic" => Some(Resample::BicubicAutomatic),
        "lanczos" => Some(Resample::Lanczos),
        "preserveDetails" => Some(Resample::PreserveDetails),
        _ => return Err(bad("image.imageSize", "resample must be none, nearest, nearestNeighbor, bilinear, bicubic, lanczos or preserveDetails")),
    })
}

/// The largest raster Image Size resamples to: the decoders' default allocation budget.
const MAX_RESAMPLE_BYTES: u64 = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 2 << 30 };

fn checked_scaled_bounds(bounds: Rect, sx: f64, sy: f64) -> Result<Rect> {
    let values =
        [(f64::from(bounds.x0) * sx).floor(), (f64::from(bounds.y0) * sy).floor(), (f64::from(bounds.x1) * sx).ceil(), (f64::from(bounds.y1) * sy).ceil()];
    if values.iter().any(|value| !value.is_finite() || *value < f64::from(i32::MIN) || *value > f64::from(i32::MAX)) {
        return Err(bad("image.imageSize", "scaled drawable bounds exceed coordinate range"));
    }
    Ok(Rect::new(values[0] as i32, values[1] as i32, values[2] as i32, values[3] as i32))
}

fn resize_poll(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn resize_checked_surface(
    surface: &mut Surface,
    bounds: Option<Rect>,
    sx: f64,
    sy: f64,
    filter: Resample,
    canvas: Rect,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    resize_poll(control)?;
    if bounds.is_some() || matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
        let bounds = match bounds {
            Some(bounds) => bounds,
            None => surface
                .try_content_bounds_with_interrupt(control)
                .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad("image.imageSize", error.to_string()) })?
                .union(&canvas),
        };
        let options = photocraft_algo::wide_resample::ResizeOptions { scale_x: sx, scale_y: sy, filter, clamp_edges: true };
        let limits = photocraft_algo::wide_geometry::GeometryLimits { resident_bytes: MAX_RESAMPLE_BYTES, work_units: 8_000_000_000 };
        let (resized, _) = photocraft_algo::wide_resample::try_resize(surface, bounds, options, limits, control).map_err(|error| {
            if control.cancelled() { EngineError::Cancelled } else { bad("image.imageSize", format!("cannot resize authoritative samples: {error}")) }
        })?;
        *surface = resized;
    } else {
        *surface = photocraft_algo::resample::try_resize_surface_in_canvas_with_interrupt(surface, sx, sy, filter, canvas, control)
            .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad("image.imageSize", format!("cannot resize samples: {error}")) })?;
    }
    resize_poll(control)
}

// Keep the explicit authority, borrowed payload and original-control proof inputs together.
#[allow(clippy::too_many_arguments)]
fn resize_layer_surfaces(
    layers: &mut [Layer],
    bounds: &std::collections::BTreeMap<(photocraft_doc::LayerId, bool), Rect>,
    scale: (f64, f64),
    filter: Resample,
    canvas: Rect,
    depth: usize,
    indexed: &std::collections::BTreeSet<photocraft_doc::LayerId>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    let (sx, sy) = scale;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("image.imageSize", "layer hierarchy exceeds resize limit"));
    }
    for layer in layers {
        resize_poll(control)?;
        let raster_bounds = bounds.get(&(layer.id, false)).copied();
        if let Some(mask) = &mut layer.mask {
            resize_checked_surface(
                std::sync::Arc::make_mut(&mut mask.surface),
                bounds.get(&(layer.id, true)).copied(),
                sx,
                sy,
                Resample::Bilinear,
                canvas,
                control,
            )?;
        }
        if let Some(cache) = &mut layer.fill_cache {
            resize_checked_surface(&mut cache.surface, raster_bounds, sx, sy, filter, canvas, control)?;
        }
        match &mut layer.content {
            LayerContent::Raster(surface) if !indexed.contains(&layer.id) => resize_checked_surface(surface, raster_bounds, sx, sy, filter, canvas, control)?,
            LayerContent::Text(text) => {
                if let Some(surface) = &mut text.cache {
                    resize_checked_surface(surface, raster_bounds, sx, sy, filter, canvas, control)?;
                }
            }
            LayerContent::Shape(shape) => {
                if let Some(surface) = &mut shape.cache {
                    resize_checked_surface(surface, raster_bounds, sx, sy, filter, canvas, control)?;
                }
            }
            LayerContent::Smart(smart) => {
                if let Some(surface) = &mut smart.cache {
                    resize_checked_surface(surface, raster_bounds, sx, sy, filter, canvas, control)?;
                }
                if let Some(mask) = &mut smart.filter_mask {
                    resize_checked_surface(std::sync::Arc::make_mut(&mut mask.surface), None, sx, sy, Resample::Bilinear, canvas, control)?;
                }
            }
            LayerContent::Group(group) => resize_layer_surfaces(&mut group.children, bounds, scale, filter, canvas, depth + 1, indexed, control)?,
            _ => {}
        }
    }
    Ok(())
}

/// Upstream output-size admission shared by Perspective Crop.
pub(crate) fn check_resample_budget(cmd: &str, doc: &Document, nw: u32, nh: u32, hint: &str) -> Result<()> {
    let bpp = doc.pixel_format().bytes_per_pixel().max(1) as u64;
    let bytes = u64::from(nw).saturating_mul(u64::from(nh)).saturating_mul(bpp);
    if bytes <= MAX_RESAMPLE_BYTES {
        return Ok(());
    }
    let mp = u64::from(nw).saturating_mul(u64::from(nh)) / 1_000_000;
    let max_mp = MAX_RESAMPLE_BYTES / bpp / 1_000_000;
    Err(bad(cmd, format!("{nw} x {nh} px is {mp} megapixels; resampling at this bit depth is limited to {max_mp} megapixels. Choose a smaller size{hint}")))
}

fn check_resize_budget(
    doc: &Document,
    bounds: &std::collections::BTreeMap<(photocraft_doc::LayerId, bool), Rect>,
    sx: f64,
    sy: f64,
    filter: Resample,
    canvas: Rect,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<u64> {
    resize_poll(control)?;
    let mut output_total = 0u64;
    let mut total = 0_u64;
    let mut total_work = 0_u64;
    let mut account = |surface: &Surface, extent: Option<Rect>, sampling: Resample| -> Result<()> {
        resize_poll(control)?;
        let extent = match extent {
            Some(extent) => extent,
            None => surface
                .try_content_bounds_with_interrupt(control)
                .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad("image.imageSize", error.to_string()) })?
                .union(&canvas),
        };
        let extent = checked_scaled_bounds(extent, sx, sy)?;
        let support = match sampling {
            Resample::Nearest => 0.0,
            Resample::Bilinear => 1.0,
            Resample::Bicubic | Resample::BicubicSmoother | Resample::BicubicSharper | Resample::BicubicAutomatic => 2.0,
            Resample::Lanczos | Resample::PreserveDetails => 3.0,
        };
        let taps = |scale: f64| -> Result<u64> {
            if sampling == Resample::Nearest {
                return Ok(1);
            }
            let count = (2.0 * support * (1.0 / scale).max(1.0)).ceil() + 2.0;
            if !count.is_finite() || count > 4096.0 {
                return Err(bad("image.imageSize", "resampling support exceeds work budget"));
            }
            Ok(count as u64)
        };
        let work = u64::from(extent.width())
            .checked_mul(u64::from(extent.height()))
            .and_then(|pixels| pixels.checked_mul(surface.format().channels() as u64))
            .and_then(|value| value.checked_mul(taps(sx).ok()?))
            .and_then(|value| value.checked_mul(taps(sy).ok()?))
            .ok_or_else(|| bad("image.imageSize", "resize work exceeds limits"))?;
        total_work = total_work
            .checked_add(work)
            .filter(|value| *value <= 8_000_000_000)
            .ok_or_else(|| bad("image.imageSize", "combined resampling work exceeds budget"))?;
        let mut resident = surface.default_bytes().len() as u64;
        for (_, tile) in surface.tiles() {
            resize_poll(control)?;
            resident = resident.checked_add(tile.bytes().len() as u64).ok_or_else(|| bad("image.imageSize", "resize residency exceeds limits"))?;
        }
        let tiles = if extent.is_empty() {
            0
        } else {
            let side = photocraft_geom::TILE_SIZE;
            let width = i64::from((extent.x1 - 1).div_euclid(side)) - i64::from(extent.x0.div_euclid(side)) + 1;
            let height = i64::from((extent.y1 - 1).div_euclid(side)) - i64::from(extent.y0.div_euclid(side)) + 1;
            u64::try_from(width)
                .ok()
                .and_then(|width| u64::try_from(height).ok().and_then(|height| width.checked_mul(height)))
                .ok_or_else(|| bad("image.imageSize", "resize tile count exceeds limits"))?
        };
        let tile_bytes = u64::from(photocraft_geom::TILE_SIZE as u32)
            .pow(2)
            .checked_mul(surface.format().bytes_per_pixel() as u64)
            .ok_or_else(|| bad("image.imageSize", "resize sample size exceeds limits"))?;
        output_total = output_total
            .checked_add(
                tiles
                    .checked_mul(tile_bytes.checked_add(256).ok_or_else(|| bad("image.imageSize", "resize residency exceeds limits"))?)
                    .ok_or_else(|| bad("image.imageSize", "resize residency exceeds limits"))?,
            )
            .ok_or_else(|| bad("image.imageSize", "resize residency exceeds limits"))?;
        total = total
            .checked_add(resident)
            .and_then(|value| tiles.checked_mul(tile_bytes).and_then(|output| value.checked_add(output)))
            .ok_or_else(|| bad("image.imageSize", "resize residency exceeds limits"))?;
        if total > MAX_RESAMPLE_BYTES {
            return Err(bad("image.imageSize", "combined source and destination tiles exceed resize memory budget"));
        }
        Ok(())
    };
    let mut pending = Vec::new();
    pending.try_reserve(1).map_err(|_| bad("image.imageSize", "cannot reserve layer traversal"))?;
    pending.push((doc.layers.as_slice(), 0usize));
    while let Some((layers, depth)) = pending.pop() {
        resize_poll(control)?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(bad("image.imageSize", "layer hierarchy exceeds resize limit"));
        }
        for layer in layers {
            resize_poll(control)?;
            let extent = bounds.get(&(layer.id, false)).copied();
            if let Some(mask) = &layer.mask {
                account(&mask.surface, bounds.get(&(layer.id, true)).copied(), Resample::Bilinear)?;
            }
            if let Some(cache) = &layer.fill_cache {
                account(&cache.surface, extent, filter)?;
            }
            match &layer.content {
                LayerContent::Raster(surface) => account(surface, extent, filter)?,
                LayerContent::Text(text) => {
                    if let Some(surface) = &text.cache {
                        account(surface, extent, filter)?;
                    }
                }
                LayerContent::Shape(shape) => {
                    if let Some(surface) = &shape.cache {
                        account(surface, extent, filter)?;
                    }
                }
                LayerContent::Smart(smart) => {
                    if let Some(surface) = &smart.cache {
                        account(surface, extent, filter)?;
                    }
                    if let Some(mask) = &smart.filter_mask {
                        account(&mask.surface, None, Resample::Bilinear)?;
                    }
                }
                LayerContent::Group(group) => {
                    pending.try_reserve(1).map_err(|_| bad("image.imageSize", "cannot reserve layer traversal"))?;
                    pending.push((group.children.as_slice(), depth + 1));
                }
                _ => {}
            }
        }
    }
    for channel in doc.channels.iter().chain(doc.quick_mask.iter()) {
        resize_poll(control)?;
        let extent = resize_channel_bounds(doc.xcf_state.as_ref(), channel.id, control)?;
        account(&channel.surface, extent, Resample::Bilinear)?;
    }
    if let Some(surface) = &doc.selection {
        account(surface, Some(canvas), Resample::Bilinear)?;
    }
    if let Some(state) = &doc.xcf_state {
        for mask in &state.filter_masks {
            resize_poll(control)?;
            account(&mask.surface, Some(mask.bounds), Resample::Bilinear)?;
        }
    }
    resize_poll(control)?;
    Ok(output_total)
}

fn resize_imported_metadata(doc: &mut Document, sx: f64, sy: f64, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    resize_poll(control)?;
    let Some(state) = &mut doc.xcf_state else { return Ok(()) };
    for mask in &mut state.filter_masks {
        resize_poll(control)?;
        resize_checked_surface(&mut mask.surface, Some(mask.bounds), sx, sy, Resample::Bilinear, mask.bounds, control)?;
        mask.bounds = checked_scaled_bounds(mask.bounds, sx, sy)?;
    }
    for item in &mut state.items {
        resize_poll(control)?;
        if let Some(bounds) = item.raster_bounds {
            item.raster_bounds = Some(checked_scaled_bounds(bounds, sx, sy)?);
        }
        if let Some(mask) = &mut item.mask {
            mask.bounds = checked_scaled_bounds(mask.bounds, sx, sy)?;
            let offset = Rect::new(mask.offset[0], mask.offset[1], mask.offset[0], mask.offset[1]);
            let scaled = checked_scaled_bounds(offset, sx, sy)?;
            mask.offset = [scaled.x0, scaled.y0];
        }
    }
    Ok(())
}

fn image_size_resolution(doc: &Document, p: &Value) -> Result<Option<photocraft_doc::PhysicalResolution>> {
    let axis = |key: &str| -> Result<Option<f64>> {
        p.get(key)
            .map(|value| {
                value
                    .as_f64()
                    .filter(|v| v.is_finite() && *v > 0.0 && *v <= 10_000.0)
                    .ok_or_else(|| bad("image.imageSize", format!("{key} must be finite and in (0, 10000]")))
            })
            .transpose()
    };
    let dpi = axis("resolution")?;
    let x_dpi = axis("resolutionX")?;
    let y_dpi = axis("resolutionY")?;
    if dpi.is_some() && (x_dpi.is_some() || y_dpi.is_some()) {
        return Err(bad("image.imageSize", "specify resolution or the independent resolutionX/resolutionY axes"));
    }
    if dpi.is_none() && x_dpi.is_none() && y_dpi.is_none() {
        return Ok(None);
    }
    let mut resolution = doc.effective_resolution();
    if let Some(dpi) = dpi {
        resolution.x_dpi = dpi;
        resolution.y_dpi = dpi;
    }
    if let Some(dpi) = x_dpi {
        resolution.x_dpi = dpi;
    }
    if let Some(dpi) = y_dpi {
        resolution.y_dpi = dpi;
    }
    resolution.validate().map_err(|e| bad("image.imageSize", e.to_string()))?;
    Ok(Some(resolution))
}

/// Image â†’ Image Size.
fn image_size_target_pair(session: &Session, params: &Value) -> Result<bool> {
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => Ok(false),
        (Some(document), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| bad("image.imageSize", "target IDs must be canonical decimal strings"))?;
                text.parse::<u64>().map_err(|_| bad("image.imageSize", "target IDs exceed range"))
            };
            let document = exact(document)?;
            if document == 0 || document != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(bad("image.imageSize", "captured target document or revision changed"));
            }
            Ok(true)
        }
        _ => Err(bad("image.imageSize", "documentId and expectedRevision must be supplied together")),
    }
}
/// Bounded borrowed validation before floating normalization/parameter cloning.
/// No pixel/model scan, custom-unit clone or document mutation. Valid legacy
/// no-pair requests retain the existing authorized auto-anchor behavior.
pub(crate) fn image_size_dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    let object = params.as_object().filter(|object| object.len() <= 32).ok_or_else(|| bad("image.imageSize", "parameters must be a bounded object"))?;
    for (key, value) in object {
        if key.len() > 128 || key.contains('\0') {
            return Err(bad("image.imageSize", "parameter name limit"));
        }
        match value {
            Value::String(text) if text.len() <= 256 && !text.contains('\0') => {}
            Value::Number(number) if number.as_f64().is_some_and(f64::is_finite) => {}
            Value::Bool(_) | Value::Null => {}
            _ => return Err(bad("image.imageSize", "parameters require bounded flat scalar values")),
        }
    }
    let captured = image_size_target_pair(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX {
        return Err(bad("image.imageSize", "document revision limit"));
    }
    if session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(bad("image.imageSize", "target document has pending work"));
    }
    if captured
        && (state.floating.is_some()
            || session.has_jobs()
            || session.coalesce_request.is_some()
            || session.color_restrict.is_some()
            || object.contains_key("coalesce"))
    {
        return Err(bad("image.imageSize", "captured sizing target has floating or transient context"));
    }
    let name = params
        .get("resample")
        .map(|value| {
            value.as_str().filter(|name| name.len() <= 32).ok_or_else(|| bad("image.imageSize", "resample must be a bounded named interpolation or none"))
        })
        .transpose()?
        .unwrap_or("bicubic");
    let resample = parse_resample(name)?;
    let axis = |key: &str| -> Result<Option<f64>> {
        params
            .get(key)
            .map(|value| {
                value
                    .as_f64()
                    .filter(|value| value.is_finite() && *value > 0.0 && *value <= 10_000.0)
                    .ok_or_else(|| bad("image.imageSize", "resolution axes must be finite and in (0, 10000]"))
            })
            .transpose()
    };
    let scalar = axis("resolution")?;
    let x = axis("resolutionX")?;
    let y = axis("resolutionY")?;
    if scalar.is_some() && (x.is_some() || y.is_some()) {
        return Err(bad("image.imageSize", "specify scalar resolution or independent axes"));
    }
    let legacy = photocraft_doc::PhysicalResolution::square(f64::from(state.doc.resolution_dpi));
    let base = state.doc.physical_resolution.as_ref().unwrap_or(&legacy);
    // Stage only scalars; borrow the original unit strings. Existing explicit
    // resolution can repair stale legacy axes without cloning invalid metadata.
    let context = photocraft_doc::PhysicalResolution {
        x_dpi: scalar.or(x).unwrap_or(base.x_dpi),
        y_dpi: scalar.or(y).unwrap_or(base.y_dpi),
        unit: photocraft_doc::PhysicalUnit::Pixel,
    };
    context.validate().map_err(|_| bad("image.imageSize", "invalid physical resolution context"))?;
    photocraft_doc::physical_size::units_to_pixels(0., &context, &base.unit, photocraft_doc::physical_size::Axis::X)
        .map_err(|_| bad("image.imageSize", "invalid physical unit context"))?;
    let name =
        params.get("sizeUnit").map(|value| value.as_str().ok_or_else(|| bad("image.imageSize", "sizeUnit must be a string"))).transpose()?.unwrap_or("px");
    let named = match name {
        "px" | "document" => photocraft_doc::PhysicalUnit::Pixel,
        "in" => photocraft_doc::PhysicalUnit::Inch,
        "mm" => photocraft_doc::PhysicalUnit::Millimeter,
        "pt" => photocraft_doc::PhysicalUnit::Point,
        "pc" => photocraft_doc::PhysicalUnit::Pica,
        _ => return Err(bad("image.imageSize", "sizeUnit must be px, in, mm, pt, pc or document")),
    };
    let unit = if name == "document" { &base.unit } else { &named };
    let dimension = |key: &str, axis| -> Result<Option<f64>> {
        params
            .get(key)
            .map(|value| {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| bad("image.imageSize", "dimensions must be positive finite numbers"))?;
                photocraft_doc::physical_size::units_to_pixels(value, &context, unit, axis)
                    .map_err(|_| bad("image.imageSize", "dimension conversion exceeds range"))
            })
            .transpose()
    };
    let (ow, oh) = (state.doc.size.width, state.doc.size.height);
    let mut requested_width = dimension("width", photocraft_doc::physical_size::Axis::X)?;
    let mut requested_height = dimension("height", photocraft_doc::physical_size::Axis::Y)?;
    if let Some(value) = params.get("percent") {
        let pc = value.as_f64().filter(|v| v.is_finite() && *v > 0.0).ok_or_else(|| bad("image.imageSize", "percent must be positive finite"))?;
        requested_width = requested_width.or(Some(f64::from(ow) * pc / 100.0));
        requested_height = requested_height.or(Some(f64::from(oh) * pc / 100.0));
    }
    let keep =
        params.get("keepPrintSize").map(|v| v.as_bool().ok_or_else(|| bad("image.imageSize", "keepPrintSize must be boolean"))).transpose()?.unwrap_or(false);
    if keep && requested_width.is_none() && requested_height.is_none() && (scalar.is_some() || x.is_some() || y.is_some()) {
        requested_width = Some(f64::from(ow) * context.x_dpi / base.x_dpi);
        requested_height = Some(f64::from(oh) * context.y_dpi / base.y_dpi);
    }
    let (width, height) = match (requested_width, requested_height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * f64::from(oh) / f64::from(ow.max(1))),
        (None, Some(h)) => (h * f64::from(ow) / f64::from(oh.max(1)), h),
        (None, None) => (f64::from(ow), f64::from(oh)),
    };
    let (width, height) = if matches!(unit, photocraft_doc::PhysicalUnit::Pixel) {
        (width.round().clamp(1., 300_000.) as u32, height.round().clamp(1., 300_000.) as u32)
    } else {
        let dimensions = photocraft_doc::physical_size::pixel_dimensions(
            width,
            height,
            &context,
            &photocraft_doc::PhysicalUnit::Pixel,
            photocraft_doc::physical_size::DimensionRounding::Nearest,
        )
        .map_err(|_| bad("image.imageSize", "dimensions exceed supported canvas range"))?;
        (dimensions.width, dimensions.height)
    };
    if resample.is_some()
        && (width, height) != (ow, oh)
        && u64::from(width).saturating_mul(u64::from(height)).saturating_mul(state.doc.pixel_format().bytes_per_pixel() as u64) > MAX_RESAMPLE_BYTES
    {
        return Err(bad("image.imageSize", "resampling megapixels exceeds the current bit-depth raster budget"));
    }
    Ok(())
}

fn image_size(s: &mut Session, p: &Value) -> Result<Value> {
    image_size_dispatch_preflight(s, p, s.command_is_background())?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (ow, oh) = (d.doc.size.width, d.doc.size.height);
    let resample_name = match p.get("resample") {
        None => "bicubic",
        Some(value) => value.as_str().ok_or_else(|| bad("image.imageSize", "resample must be a named interpolation or none"))?,
    };
    let resample = parse_resample(resample_name)?;
    let resolution = image_size_resolution(&d.doc, p)?;
    let context = resolution.clone().unwrap_or_else(|| d.doc.effective_resolution());
    let unit = match p.get("sizeUnit") {
        None => photocraft_doc::PhysicalUnit::Pixel,
        Some(Value::String(unit)) => match unit.as_str() {
            "px" => photocraft_doc::PhysicalUnit::Pixel,
            "in" => photocraft_doc::PhysicalUnit::Inch,
            "mm" => photocraft_doc::PhysicalUnit::Millimeter,
            "pt" => photocraft_doc::PhysicalUnit::Point,
            "pc" => photocraft_doc::PhysicalUnit::Pica,
            "document" => context.unit.clone(),
            _ => return Err(bad("image.imageSize", "sizeUnit must be px, in, mm, pt, pc or document")),
        },
        _ => return Err(bad("image.imageSize", "sizeUnit must be a string")),
    };
    let dimension = |key: &str, axis| -> Result<Option<f64>> {
        p.get(key)
            .map(|value| {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| bad("image.imageSize", format!("{key} must be a positive finite number")))?;
                photocraft_doc::physical_size::units_to_pixels(value, &context, &unit, axis).map_err(|e| bad("image.imageSize", e.to_string()))
            })
            .transpose()
    };
    let mut pw = dimension("width", photocraft_doc::physical_size::Axis::X)?;
    let mut ph = dimension("height", photocraft_doc::physical_size::Axis::Y)?;
    if let Some(value) = p.get("percent") {
        let percent = value.as_f64().filter(|v| v.is_finite() && *v > 0.0).ok_or_else(|| bad("image.imageSize", "percent must be positive finite"))?;
        pw = pw.or(Some(f64::from(ow) * percent / 100.0));
        ph = ph.or(Some(f64::from(oh) * percent / 100.0));
    }
    let keep = p.get("keepPrintSize").map(|v| v.as_bool().ok_or_else(|| bad("image.imageSize", "keepPrintSize must be boolean"))).transpose()?.unwrap_or(false);
    if keep && pw.is_none() && ph.is_none() && resolution.is_some() {
        let old = d.doc.effective_resolution();
        pw = Some(f64::from(ow) * context.x_dpi / old.x_dpi);
        ph = Some(f64::from(oh) * context.y_dpi / old.y_dpi);
    }

    // Missing dimension keeps the aspect ratio.
    let (nw, nh) = match (pw, ph) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * oh as f64 / ow.max(1) as f64),
        (None, Some(h)) => (h * ow as f64 / oh.max(1) as f64, h),
        (None, None) => (ow as f64, oh as f64),
    };
    let (nw, nh) = if matches!(unit, photocraft_doc::PhysicalUnit::Pixel) {
        (nw.round().clamp(1.0, 300_000.0) as u32, nh.round().clamp(1.0, 300_000.0) as u32)
    } else {
        let dimensions = photocraft_doc::physical_size::pixel_dimensions(
            nw,
            nh,
            &context,
            &photocraft_doc::PhysicalUnit::Pixel,
            photocraft_doc::physical_size::DimensionRounding::Nearest,
        )
        .map_err(|e| bad("image.imageSize", e.to_string()))?;
        (dimensions.width, dimensions.height)
    };
    // Each side may reach 300000 px, but resampling writes real pixels: refuse a canvas whose
    // raster alone would pass the budget, before allocating any of it (#1544).
    let resample =
        if resample_name == "automatic" { Some(if nw > ow || nh > oh { Resample::PreserveDetails } else { Resample::BicubicSharper }) } else { resample };
    if d.revision == u64::MAX {
        return Err(bad("image.imageSize", "document revision limit"));
    }
    d.doc.validate_resident()?;
    let baseline = d.doc.clone();
    let revision = d.revision;
    let work_base = baseline.clone();
    crate::jobs::run(
        s,
        "Image Size",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Image Size", |control| {
                resize_poll(control)?;
                let preparation = photocraft_io::NativeSaveLimits::default().preparation;
                let payload = photocraft_io::retained_native_payload_bytes(&work_base, preparation, control)
                    .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad("image.imageSize", error.to_string()) })?;
                let model = photocraft_io::retained_native_model_bytes(&work_base, preparation, control)
                    .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad("image.imageSize", error.to_string()) })?;
                let raster = work_base.resident_raster_footprint_with(control)?;
                let bounds_index = work_base.xcf_state.as_ref().map_or(Ok(0u64), |state| {
                    (state.items.len() as u64).checked_mul(512).ok_or_else(|| bad("image.imageSize", "declared resize bounds index exceeds budget"))
                })?;
                let clone_base = (model as u64)
                    .checked_mul(3)
                    .and_then(|n| n.checked_add(payload as u64))
                    .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(3)?))
                    .and_then(|n| n.checked_add(bounds_index))
                    .filter(|n| *n <= MAX_RESAMPLE_BYTES)
                    .ok_or_else(|| bad("image.imageSize", "retained/model/index resize staging exceeds budget"))?;
                let plan = prepare_image_resize(&work_base, (ow, oh), (nw, nh), resample, control)?;
                let output = plan.as_ref().map_or(0, |plan| plan.output_bytes);
                // Shared payloads are retained once; measured model and sparse-map
                // capacities cover the worker, refresh snapshot and final edit clones.
                // Legacy private text/effect/vector internals/allocator overhead are
                // unmeasured: this is clone admission, never an RSS guarantee.
                if clone_base.checked_add(output).is_none_or(|n| n > MAX_RESAMPLE_BYTES) {
                    return Err(bad("image.imageSize", "retained/model/output resize staging exceeds budget"));
                }
                resize_poll(control)?;
                let mut document = (*work_base).clone();
                apply_image_resize(&mut document, resolution, plan, (nw, nh), control)?;
                resize_poll(control)?;
                Ok((document, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id || current.revision != revision || !current.doc.shares_state(&baseline) {
                return Err(bad("image.imageSize", "target changed while preparing resize"));
            }
            ctx.stage(0.95, 1.0, "Image Size", |control| {
                resize_poll(control)?;
                let actual_size = document.size;
                let sampler = (resample.is_some() && (nw, nh) != (ow, oh))
                    .then_some(photocraft_geom::Affine { m: [f64::from(nw) / f64::from(ow.max(1)), 0.0, 0.0, f64::from(nh) / f64::from(oh.max(1)), 0.0, 0.0] });
                session.edit_with_sampler_map("Image Size", control, sampler, move |current, _| {
                    *current = document;
                    Ok(())
                })?;
                Ok(json!({ "width": actual_size.width, "height": actual_size.height }))
            })
        },
    )
}

struct ImageResizePlan {
    scale: (f64, f64),
    filter: Resample,
    canvas: Rect,
    bounds: std::collections::BTreeMap<(photocraft_doc::LayerId, bool), Rect>,
    output_bytes: u64,
}
fn prepare_image_resize(
    doc: &Document,
    old: (u32, u32),
    new: (u32, u32),
    resample: Option<Resample>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Option<ImageResizePlan>> {
    resize_poll(control)?;
    let Some(filter) = resample else { return Ok(None) };
    let (sx, sy) = (new.0 as f64 / old.0.max(1) as f64, new.1 as f64 / old.1.max(1) as f64);
    if (sx - 1.0).abs() < 1e-12 && (sy - 1.0).abs() < 1e-12 {
        return Ok(None);
    }
    let canvas = Rect::new(0, 0, old.0 as i32, old.1 as i32);
    if filter != Resample::Nearest && doc.xcf_state.as_ref().is_some_and(|state| !state.editable_indexed.is_empty()) {
        return Err(bad("image.imageSize", "current indexed planes support exact nearest resampling; interpolated indexed resampling is not qualified"));
    }
    let _index_bytes = doc.xcf_state.as_ref().map_or(Ok(0u64), |state| {
        (state.items.len() as u64)
            .checked_mul(512)
            .filter(|bytes| *bytes <= MAX_RESAMPLE_BYTES)
            .ok_or_else(|| bad("image.imageSize", "declared resize bounds index exceeds budget"))
    })?;
    let mut bounds = std::collections::BTreeMap::new();
    if let Some(state) = &doc.xcf_state {
        for item in &state.items {
            resize_poll(control)?;
            if let photocraft_doc::item_identity::ItemRef::Layer(id) = item.item
                && let Some(extent) = item.raster_bounds
            {
                bounds.insert((id, false), extent);
            }
            if let Some(mask) = &item.mask
                && let photocraft_doc::item_identity::ItemRef::Layer(id) = mask.owner
                && !resize_filter_mask_item(state, item.item, control)?
                && bounds.insert((id, true), mask.bounds).is_some()
            {
                return Err(bad("image.imageSize", "ambiguous declared layer mask bounds"));
            }
        }
    }
    let output_bytes = check_resize_budget(doc, &bounds, sx, sy, filter, canvas, control)?;
    resize_poll(control)?;
    Ok(Some(ImageResizePlan { scale: (sx, sy), filter, canvas, bounds, output_bytes }))
}
fn resize_filter_mask_item(
    state: &photocraft_doc::XcfState,
    item: photocraft_doc::item_identity::ItemRef,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<bool> {
    for mask in &state.filter_masks {
        resize_poll(control)?;
        if item == photocraft_doc::item_identity::ItemRef::Mask(mask.id) {
            return Ok(true);
        }
    }
    Ok(false)
}
fn resize_channel_bounds(
    state: Option<&photocraft_doc::XcfState>,
    id: photocraft_doc::item_identity::ChannelId,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Option<Rect>> {
    if let Some(state) = state {
        for item in &state.items {
            resize_poll(control)?;
            if item.item == photocraft_doc::item_identity::ItemRef::Channel(id) {
                return Ok(item.raster_bounds);
            }
        }
    }
    Ok(None)
}
fn resize_effects_with_interrupt(layers: &mut [Layer], scale: f32, depth: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("image.imageSize", "layer hierarchy exceeds resize limit"));
    }
    for layer in layers {
        resize_poll(control)?;
        scale_effects(&mut layer.effects, scale);
        if let Some(children) = layer.children_mut() {
            resize_effects_with_interrupt(children, scale, depth + 1, control)?;
        }
    }
    resize_poll(control)
}
fn apply_image_resize(
    doc: &mut Document,
    resolution: Option<photocraft_doc::PhysicalResolution>,
    plan: Option<ImageResizePlan>,
    new: (u32, u32),
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    resize_poll(control)?;
    if let Some(resolution) = resolution {
        doc.set_physical_resolution(resolution).map_err(|error| bad("image.imageSize", error.to_string()))?;
    }
    let Some(ImageResizePlan { scale: (sx, sy), filter, canvas, bounds, .. }) = plan else { return resize_poll(control) };
    let (nw, nh) = new;
    // The indexed subphase accepts only lowered limits. The surrounding
    // resampler's larger ceiling does not widen that independent domain.
    let indexed_limits = photocraft_algo::wide_geometry::GeometryLimits::default();
    let indexed = crate::xcf_indexed_geometry::resize_nearest(
        doc,
        sx,
        sy,
        photocraft_algo::wide_geometry::GeometryLimits {
            resident_bytes: MAX_RESAMPLE_BYTES.min(indexed_limits.resident_bytes),
            work_units: 8_000_000_000u64.min(indexed_limits.work_units),
        },
        control,
    )?
    .into_iter()
    .map(|(id, _)| id)
    .collect();
    resize_layer_surfaces(&mut doc.layers, &bounds, (sx, sy), filter, canvas, 0, &indexed, control)?;
    let k = ((sx + sy) / 2.0) as f32;
    resize_effects_with_interrupt(&mut doc.layers, k, 0, control)?;
    for ch in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
        resize_poll(control)?;
        let bounds = resize_channel_bounds(doc.xcf_state.as_ref(), ch.id, control)?;
        resize_checked_surface(&mut ch.surface, bounds, sx, sy, Resample::Bilinear, canvas, control)?;
    }
    if let Some(sel) = &mut doc.selection {
        let bounds = doc.selection_channel_id.map(|id| resize_channel_bounds(doc.xcf_state.as_ref(), id, control)).transpose()?.flatten();
        resize_checked_surface(sel, bounds, sx, sy, Resample::Bilinear, canvas, control)?;
    }
    resize_imported_metadata(doc, sx, sy, control)?;
    // Vector geometry, guides and marks scale with the pixels; vectors re-render sharp.
    resize_poll(control)?;
    crate::canvas_geom::transform_geometry_with_interrupt(doc, &photocraft_geom::Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] }, control)?;
    doc.size = Size::new(nw, nh);
    resize_poll(control)?;
    crate::canvas_geom::refresh_with_interrupt(doc, crate::canvas_geom::Refresh::All, control)?;
    resize_poll(control)
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Crops the document to `r` (in current document coordinates).
fn crop_poll(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn crop_error(control: &photocraft_raster::Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad("image.crop", error.to_string()) }
}
fn crop_offset(r: Rect) -> Result<(i32, i32)> {
    if r.is_empty() || i64::from(r.x1) - i64::from(r.x0) > i64::from(MAX_CROP_SIDE) || i64::from(r.y1) - i64::from(r.y0) > i64::from(MAX_CROP_SIDE) {
        return Err(bad("image.crop", format!("crop dimensions must be positive and within the {MAX_CROP_SIDE} px limit")));
    }
    Ok((
        r.x0.checked_neg().ok_or_else(|| bad("image.crop", "crop origin cannot be moved to zero in the 32-bit coordinate range"))?,
        r.y0.checked_neg().ok_or_else(|| bad("image.crop", "crop origin cannot be moved to zero in the 32-bit coordinate range"))?,
    ))
}
fn crop_visit_surfaces(
    layers: &[Layer],
    depth: usize,
    count: &mut usize,
    control: &photocraft_raster::Interrupt<'_>,
    indexed: bool,
    visit: &mut dyn FnMut(&Surface, bool) -> Result<()>,
) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("image.crop", "layer depth limit"));
    }
    for layer in layers {
        crop_poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| bad("image.crop", "layer count limit"))?;
        if layer.video.is_some() || (!indexed && layer.indexed_source.is_some()) {
            return Err(bad("image.crop", "unsupported video or indexed source geometry"));
        }
        if let Some(mask) = &layer.mask {
            visit(&mask.surface, false)?;
        }
        if let Some(cache) = &layer.fill_cache {
            visit(&cache.surface, true)?;
        }
        match &layer.content {
            LayerContent::Group(group) => crop_visit_surfaces(&group.children, depth + 1, count, control, indexed, visit)?,
            LayerContent::BackedRaster(_) => return Err(bad("image.crop", "backed source geometry is unsupported")),
            LayerContent::Smart(smart) => {
                if let Some(cache) = &smart.cache {
                    visit(cache, true)?;
                }
                if let Some(mask) = &smart.filter_mask {
                    visit(&mask.surface, false)?;
                }
            }
            _ => {
                if let Some(surface) = layer.surface() {
                    visit(surface, true)?;
                }
            }
        }
    }
    crop_poll(control)
}
/// Conservative aggregate clone/output admission before any staged Document clone.
/// Private legacy metadata allocation and process RSS are not covered by this ledger.
fn crop_prepare(doc: &Document, r: Rect, delete: bool, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    crop_poll(control)?;
    let offset = crop_offset(r)?;
    let indexed = doc.xcf_state.as_ref().is_some_and(|state| !state.editable_indexed.is_empty());
    if !delete && !indexed {
        canvas_translation_preflight(doc, offset, control).map_err(|e| crop_error(control, format!("32-bit coordinates/staging admission: {e}")))?;
        return crop_poll(control);
    }
    crate::canvas_geom::preflight_geometry_with_interrupt(doc, control)?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, control).map_err(|e| crop_error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, control).map_err(|e| crop_error(control, e))? as u64;
    let raster = crate::document_snapshot::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let mut resident = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .ok_or_else(|| bad("image.crop", "crop clone budget overflow"))?;
    let (indexed_bytes, indexed_work) = crop_indexed_budget(doc, r, delete, control)?;
    resident = resident.checked_add(indexed_bytes).ok_or_else(|| bad("image.crop", "indexed crop budget overflow"))?;
    let mut scratch = 0u64;
    let mut work = indexed_work;
    let mut visit = |surface: &Surface, cropped: bool| -> Result<()> {
        crop_poll(control)?;
        let count = surface.tile_count() as u64;
        let bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64))
            .ok_or_else(|| bad("image.crop", "tile size overflow"))?;
        // Includes cropped output and up to four translated tiles per source tile.
        resident = resident
            .checked_add(
                count.checked_mul(5).and_then(|n| n.checked_mul(bytes.checked_add(1024)?)).ok_or_else(|| bad("image.crop", "crop output budget overflow"))?,
            )
            .ok_or_else(|| bad("image.crop", "crop output budget overflow"))?;
        scratch = scratch.max(bytes.checked_mul(8).and_then(|n| n.checked_add(4096)).ok_or_else(|| bad("image.crop", "crop scratch overflow"))?);
        work = work
            .checked_add(
                count
                    .checked_mul(count)
                    .and_then(|n| n.checked_mul(1536))
                    .and_then(|n| n.checked_add(count.checked_mul(bytes)?.checked_mul(16)?))
                    .ok_or_else(|| bad("image.crop", "crop work overflow"))?,
            )
            .filter(|n| *n <= 8_000_000_000)
            .ok_or_else(|| bad("image.crop", "crop work budget exceeded"))?;
        let bounds = surface.try_content_bounds_with_interrupt(control).map_err(|e| crop_error(control, e))?;
        let bounds = if delete && cropped { bounds.intersect(&r) } else { bounds };
        if !bounds.is_empty() {
            let _ = canvas_tiles(canvas_moved(bounds, offset).map_err(|e| crop_error(control, format!("32-bit translated coordinates: {e}")))?)
                .map_err(|e| crop_error(control, e))?;
        }
        Ok(())
    };
    crop_visit_surfaces(&doc.layers, 0, &mut 0, control, indexed, &mut visit)?;
    for channel in doc.channels.iter().chain(doc.quick_mask.as_ref()) {
        visit(&channel.surface, false)?;
    }
    if let Some(selection) = &doc.selection {
        visit(selection, false)?;
    }
    if let Some(state) = &doc.xcf_state {
        if !state.source_items.is_empty() || !state.filter_stacks.is_empty() || (indexed && !state.filter_masks.is_empty()) {
            return Err(bad("image.crop", "source or filter geometry transformation is not qualified"));
        }
        if state.items.len() > 100_000 || state.filter_masks.len() > 100_000 {
            return Err(bad("image.crop", "current geometry index exceeds limit"));
        }
        for mask in &state.filter_masks {
            visit(&mask.surface, false)?;
            let _ = canvas_moved(mask.bounds, offset)?;
        }
        for item in &state.items {
            crop_poll(control)?;
            if let Some(bounds) = item.raster_bounds {
                let _ = canvas_moved(bounds, offset)?;
            }
            if let Some(mask) = &item.mask {
                let _ = canvas_moved(mask.bounds, offset)?;
                for (value, delta) in mask.offset.iter().zip([offset.0, offset.1]) {
                    value.checked_add(delta).ok_or_else(|| bad("image.crop", "32-bit mask offset overflow"))?;
                }
            }
        }
    }
    if delete
        && !indexed
        && !doc.bounds().contains_rect(&r)
        && let Some(layer) = doc.layers.first().filter(|layer| layer.name == "Background" && layer.locks.transparency)
        && let LayerContent::Raster(surface) = &layer.content
    {
        crop_poll(control)?;
        // Background extension writes the complete new canvas, even when no
        // old pixel overlaps. Admit physical output and repeated COW map work
        // independently of the number of retained source tiles, before cloning.
        let canvas = Rect::new(0, 0, r.width() as i32, r.height() as i32);
        let count = canvas_tiles(canvas)?;
        let source = (surface.tile_count() as u64).checked_mul(4).ok_or_else(|| bad("image.crop", "background map overflow"))?;
        let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64))
            .ok_or_else(|| bad("image.crop", "background tile overflow"))?;
        work = work
            .checked_add(
                count
                    .checked_mul(source.checked_add(count).ok_or_else(|| bad("image.crop", "background scheduler overflow"))?)
                    .and_then(|n| n.checked_mul(256))
                    .ok_or_else(|| bad("image.crop", "background scheduler overflow"))?,
            )
            .and_then(|n| n.checked_add(count.checked_mul(tile_bytes)?.checked_mul(8)?))
            .filter(|n| *n <= 8_000_000_000)
            .ok_or_else(|| bad("image.crop", "background work budget exceeded"))?;
        resident = resident
            .checked_add(
                count
                    .checked_mul(tile_bytes.checked_add(1024).ok_or_else(|| bad("image.crop", "background ledger overflow"))?)
                    .ok_or_else(|| bad("image.crop", "background ledger overflow"))?,
            )
            .and_then(|n| n.checked_add(source.checked_mul(1024)?))
            .ok_or_else(|| bad("image.crop", "background ledger overflow"))?;
        scratch = scratch.max(tile_bytes.checked_mul(8).and_then(|n| n.checked_add(4096)).ok_or_else(|| bad("image.crop", "background scratch overflow"))?);
        let _ = work;
        crop_poll(control)?;
    }
    if resident.checked_add(scratch).is_none_or(|n| n > MAX_RESAMPLE_BYTES.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES)) {
        return Err(bad("image.crop", "crop staging exceeds raster budget"));
    }
    crop_poll(control)
}
fn crop_surface_with_interrupt(source: &Surface, keep: Rect, control: &photocraft_raster::Interrupt<'_>) -> Result<Surface> {
    crop_poll(control)?;
    let bounds = source.try_content_bounds_with_interrupt(control).map_err(|e| crop_error(control, e))?.intersect(&keep);
    let mut output = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|e| crop_error(control, e))?;
    for (coordinate, _) in source.tiles() {
        crop_poll(control)?;
        let part = coordinate.rect().intersect(&bounds);
        if part.is_empty() {
            continue;
        }
        let bytes = source.try_read_interleaved_with(part, control).map_err(|e| crop_error(control, e))?;
        // A single-tile writer keeps sparse map staging bounded; insertion into
        // the private output never repeatedly clones the whole output map.
        let mut tile = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|e| crop_error(control, e))?;
        tile.try_write_interleaved_with(part, &bytes, control).map_err(|e| crop_error(control, e))?;
        crop_poll(control)?;
        output.put_tiles(tile.tiles().map(|(position, tile)| (*position, std::sync::Arc::clone(tile))));
    }
    crop_poll(control)?;
    Ok(output)
}
fn crop_layers_with_interrupt(layers: &mut [Layer], keep: Rect, depth: usize, count: &mut usize, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("image.crop", "layer depth limit"));
    }
    for layer in layers {
        crop_poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| bad("image.crop", "layer count limit"))?;
        if let Some(cache) = &mut layer.fill_cache {
            cache.surface = crop_surface_with_interrupt(&cache.surface, keep, control)?;
        }
        match &mut layer.content {
            LayerContent::Group(group) => crop_layers_with_interrupt(&mut group.children, keep, depth + 1, count, control)?,
            LayerContent::Raster(surface) => *surface = crop_surface_with_interrupt(surface, keep, control)?,
            LayerContent::Text(text) => {
                if let Some(surface) = &mut text.cache {
                    *surface = crop_surface_with_interrupt(surface, keep, control)?;
                }
            }
            LayerContent::Shape(shape) => {
                if let Some(surface) = &mut shape.cache {
                    *surface = crop_surface_with_interrupt(surface, keep, control)?;
                }
            }
            LayerContent::Smart(smart) => {
                if let Some(surface) = &mut smart.cache {
                    *surface = crop_surface_with_interrupt(surface, keep, control)?;
                }
            }
            _ => {}
        }
    }
    crop_poll(control)
}
struct CropIndexed {
    owner: photocraft_doc::LayerId,
    raster: photocraft_doc::EditableIndexedRaster,
    carrier: Surface,
}
fn crop_indexed_config(bounds: Rect, crop: Rect, delete: bool) -> Result<photocraft_algo::indexed_geometry::Config> {
    let (dx, dy) = crop_offset(crop)?;
    Ok(photocraft_algo::indexed_geometry::Config {
        clip: if delete { crop } else { bounds },
        origin: photocraft_algo::indexed_geometry::Origin::Translate { dx, dy },
    })
}
/// Extra current-plane/expansion/synchronization ledger, without source-plane mutation.
fn crop_indexed_budget(doc: &Document, crop: Rect, delete: bool, control: &photocraft_raster::Interrupt<'_>) -> Result<(u64, u64)> {
    let Some(state) = doc.xcf_state.as_ref().filter(|state| !state.editable_indexed.is_empty()) else { return Ok((0, 0)) };
    let checked = crate::xcf_indexed_geometry::validate_current_with_interrupt(doc, control)?;
    let mut bytes = checked.retained_bytes.checked_add(checked.scratch_bytes).ok_or_else(|| bad("image.crop", "indexed verification scratch overflow"))?;
    // Private owner and metadata lookups are bounded but not O(1); charge their
    // actual inventory envelope rather than hiding a quadratic publication walk.
    let visits = (checked.layer_count as u64)
        .checked_add(state.items.len() as u64)
        .and_then(|n| n.checked_add(state.editable_indexed.len() as u64))
        .ok_or_else(|| bad("image.crop", "indexed publication visits"))?;
    let searches = (state.editable_indexed.len() as u64)
        .checked_mul(visits)
        .and_then(|n| n.checked_mul(16))
        .ok_or_else(|| bad("image.crop", "indexed publication work"))?;
    let mut work = checked.work_units.checked_add(searches).filter(|n| *n <= 1 << 30).ok_or_else(|| bad("image.crop", "indexed publication work limit"))?;
    for indexed in &state.editable_indexed {
        crop_poll(control)?;
        let bounds = indexed.raster.bounds();
        let clipped = if delete { bounds.intersect(&crop) } else { bounds };
        if clipped.is_empty() {
            return Err(bad("image.crop", "empty current indexed owners are unsupported; convert explicitly before deleting their entire extent"));
        }
        let config = crop_indexed_config(bounds, crop, delete)?;
        let required = photocraft_algo::indexed_geometry::requirements(&indexed.raster, &config, control).map_err(|e| crop_error(control, e))?;
        let target = canvas_moved(clipped, crop_offset(crop)?).map_err(|e| crop_error(control, e))?;
        let pixels = u64::try_from(i64::from(target.x1) - i64::from(target.x0))
            .ok()
            .and_then(|width| width.checked_mul(u64::try_from(i64::from(target.y1) - i64::from(target.y0)).ok()?))
            .ok_or_else(|| bad("image.crop", "indexed crop pixel count"))?;
        let tiles = canvas_tiles(target).map_err(|e| crop_error(control, e))?;
        let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| bad("image.crop", "indexed carrier tile bytes"))?;
        let carrier = pixels
            .checked_mul(4)
            .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_add(1024)?)?))
            .and_then(|n| n.checked_add((target.width() as u64).checked_mul(4)?))
            .ok_or_else(|| bad("image.crop", "indexed carrier staging budget"))?;
        // Conservative independent source/output charges; no upstream refunds.
        bytes = bytes
            .checked_add(required.peak_resident_bytes)
            .and_then(|n| n.checked_add(carrier))
            .and_then(|n| n.checked_add(std::mem::size_of::<CropIndexed>() as u64))
            .filter(|n| *n <= photocraft_raster::MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| bad("image.crop", "indexed crop resident budget"))?;
        let verify = crate::xcf_editing::direct_indexed_sync_work(
            usize::try_from(pixels).map_err(|_| bad("image.crop", "indexed pixel count"))?,
            usize::try_from(tiles).map_err(|_| bad("image.crop", "indexed tile count"))?,
        )?;
        work = work
            .checked_add(required.work_units.checked_mul(2).ok_or_else(|| bad("image.crop", "indexed validation work"))?)
            .and_then(|n| n.checked_add(pixels.checked_mul(8)?))
            .and_then(|n| n.checked_add(verify.checked_mul(2)?))
            .filter(|n| *n <= 1 << 30)
            .ok_or_else(|| bad("image.crop", "indexed crop work budget"))?;
    }
    crop_poll(control)?;
    Ok((bytes, work))
}
fn crop_stage_indexed(doc: &Document, crop: Rect, delete: bool, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<CropIndexed>> {
    let Some(state) = doc.xcf_state.as_ref().filter(|state| !state.editable_indexed.is_empty()) else { return Ok(Vec::new()) };
    crop_poll(control)?;
    let mut staged = Vec::new();
    staged.try_reserve_exact(state.editable_indexed.len()).map_err(|_| bad("image.crop", "indexed stage allocation"))?;
    if staged.capacity().checked_mul(std::mem::size_of::<CropIndexed>()).is_none_or(|n| n as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("image.crop", "indexed stage capacity"));
    }
    let limits =
        photocraft_algo::indexed_geometry::Limits { resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES.min(MAX_RESAMPLE_BYTES), work_units: 1 << 30 };
    for indexed in &state.editable_indexed {
        crop_poll(control)?;
        let config = crop_indexed_config(indexed.raster.bounds(), crop, delete)?;
        let outcome = photocraft_algo::indexed_geometry::try_crop(&indexed.raster, &config, limits, control).map_err(|e| crop_error(control, e))?;
        let photocraft_algo::indexed_geometry::Outcome::NonEmpty(raster) = outcome else {
            return Err(bad("image.crop", "empty current indexed owner unsupported"));
        };
        let carrier = crate::xcf_editing::expanded_with(
            &raster,
            photocraft_color::PixelFormat::RGBA8,
            doc.color_table.as_ref().and_then(|table| table.transparent),
            control,
        )?;
        crop_poll(control)?;
        staged.push(CropIndexed { owner: indexed.owner, raster, carrier });
    }
    crop_poll(control)?;
    Ok(staged)
}
fn crop_publish_indexed(doc: &mut Document, staged: Vec<CropIndexed>, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    for proposed in staged {
        crop_poll(control)?;
        let bounds = proposed.raster.bounds();
        let layer = doc.layer_mut(proposed.owner).ok_or_else(|| bad("image.crop", "indexed owner changed"))?;
        // The immutable original IndexedSource is deliberately retained on its owner.
        layer.content = LayerContent::Raster(proposed.carrier);
        let state = doc.xcf_state.as_mut().ok_or_else(|| bad("image.crop", "indexed state changed"))?;
        let current =
            state.editable_indexed.iter_mut().find(|item| item.owner == proposed.owner).ok_or_else(|| bad("image.crop", "indexed owner state changed"))?;
        current.raster = proposed.raster;
        let item = state
            .items
            .iter_mut()
            .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(proposed.owner))
            .ok_or_else(|| bad("image.crop", "indexed metadata owner changed"))?;
        item.raster_bounds = Some(bounds);
    }
    crop_poll(control)
}
fn crop_doc_with_interrupt(
    doc: &mut Document,
    r: Rect,
    delete_pixels: bool,
    indexed: Vec<CropIndexed>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    let offset = crop_offset(r)?;
    crop_poll(control)?;

    if delete_pixels {
        crop_layers_with_interrupt(&mut doc.layers, r, 0, &mut 0, control)?;
    }
    canvas_translate_doc_current_with_interrupt(doc, offset, false, control)?;
    crop_publish_indexed(doc, indexed, control)?;
    doc.size = Size::new(r.width(), r.height());
    crate::canvas_geom::refresh_with_interrupt(doc, crate::canvas_geom::Refresh::Shapes, control)?;
    crop_poll(control)
}
/// The nine documented canvas-size anchors, in the order the command lists them, each mapped to
/// its (x, y) factor (0.0 = top/left edge, 0.5 = centre, 1.0 = bottom/right edge).
const ANCHORS: [(&str, (f64, f64)); 9] = [
    ("topLeft", (0.0, 0.0)),
    ("top", (0.5, 0.0)),
    ("topRight", (1.0, 0.0)),
    ("left", (0.0, 0.5)),
    ("center", (0.5, 0.5)),
    ("right", (1.0, 0.5)),
    ("bottomLeft", (0.0, 1.0)),
    ("bottom", (0.5, 1.0)),
    ("bottomRight", (1.0, 1.0)),
];

/// Resolves an anchor name to its (x, y) factors, matching the nine documented values exactly.
/// Anything else is an error naming `anchor` and listing them, rather than resolving the two axes
/// independently (which let a misspelling like `"topleft"` land between two valid anchors, #2946).
fn anchor_factors(cmd: &str, a: &str) -> Result<(f64, f64)> {
    ANCHORS.iter().find(|(name, _)| *name == a).map(|&(_, f)| f).ok_or_else(|| {
        let names = ANCHORS.iter().map(|&(n, _)| n).collect::<Vec<_>>().join("|");
        bad(cmd, format!("`anchor` must be one of {names}"))
    })
}
fn extension_color(s: &Session, p: &Value) -> Result<[f32; 4]> {
    let name = match p.get("extensionColor") {
        None | Some(Value::Null) => "background",
        Some(value) => value.as_str().ok_or_else(|| bad("image.canvasSize", "extensionColor must be a named color or #rrggbb"))?,
    };
    let color = match name {
        "foreground" => s.tools.foreground,
        "background" => s.tools.background,
        "white" => [1.0; 4],
        "black" => [0.0, 0.0, 0.0, 1.0],
        "transparent" => [0.0; 4],
        hex if hex.len() == 7 && hex.starts_with('#') && hex.as_bytes().get(1..).is_some_and(|bytes| bytes.iter().all(u8::is_ascii_hexdigit)) => {
            let component = |start: usize| -> Result<f32> {
                let text = hex
                    .get(start..start.checked_add(2).ok_or_else(|| bad("image.canvasSize", "color range"))?)
                    .ok_or_else(|| bad("image.canvasSize", "color range"))?;
                Ok(f32::from(u8::from_str_radix(text, 16).map_err(|_| bad("image.canvasSize", "invalid hexadecimal color"))?) / 255.0)
            };
            [component(1)?, component(3)?, component(5)?, 1.0]
        }
        _ => return Err(bad("image.canvasSize", "unknown extensionColor")),
    };
    if color.iter().any(|value| !value.is_finite()) || !(0.0..=1.0).contains(&color[3]) {
        return Err(bad("image.canvasSize", "extension color must be finite with unit alpha"));
    }
    Ok(color)
}

#[derive(Clone, Copy)]
struct CanvasRequest {
    old: (u32, u32),
    new: (u32, u32),
    offset: (i32, i32),
    extension: [f32; 4],
}
fn canvas_poll(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn canvas_error(control: &photocraft_raster::Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad("image.canvasSize", error.to_string()) }
}
fn canvas_request(s: &Session, p: &Value) -> Result<CanvasRequest> {
    if !p.is_null()
        && !p.as_object().is_some_and(|object| {
            object.len() <= 7
                && object
                    .keys()
                    .all(|key| matches!(key.as_str(), "width" | "height" | "relative" | "anchor" | "extensionColor" | "documentId" | "expectedRevision"))
        })
    {
        return Err(bad("image.canvasSize", "expected bounded Canvas Size parameters"));
    }
    let state = s.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(document), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| bad("image.canvasSize", "captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| bad("image.canvasSize", "captured ID exceeds range"))
            };
            if exact(document)? == 0 || exact(document)? != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(bad("image.canvasSize", "captured target changed"));
            }
            if state.floating.is_some() || state.coalesce.is_some() || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
                return Err(bad("image.canvasSize", "captured target has a floating or coalesced edit"));
            }
        }
        _ => return Err(bad("image.canvasSize", "documentId and expectedRevision must be supplied together")),
    }
    let old = (state.doc.size.width, state.doc.size.height);
    if old.0 == 0 || old.1 == 0 || old.0 > i32::MAX as u32 || old.1 > i32::MAX as u32 || state.revision == u64::MAX {
        return Err(bad("image.canvasSize", "invalid current canvas or revision limit"));
    }
    let relative = match p.get("relative") {
        None | Some(Value::Null) => false,
        Some(value) => value.as_bool().ok_or_else(|| bad("image.canvasSize", "relative must be boolean"))?,
    };
    let dimension = |key: &str, old: u32| -> Result<u32> {
        let supplied = match p.get(key) {
            None | Some(Value::Null) => {
                if relative {
                    0.0
                } else {
                    f64::from(old)
                }
            }
            Some(value) => {
                value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| bad("image.canvasSize", format!("{key} must be finite numeric pixels")))?
            }
        };
        let value = if relative { f64::from(old) + supplied } else { supplied };
        if !value.is_finite() {
            return Err(bad("image.canvasSize", "relative dimension overflow"));
        }
        // Preserve the existing valid-pixel rounding/clamping contract.
        Ok(value.round().clamp(1.0, 300_000.0) as u32)
    };
    let new = (dimension("width", old.0)?, dimension("height", old.1)?);
    let anchor = match p.get("anchor") {
        None | Some(Value::Null) => "center",
        Some(value) => value.as_str().ok_or_else(|| bad("image.canvasSize", "anchor must be a named position"))?,
    };
    if !matches!(anchor, "topLeft" | "top" | "topRight" | "left" | "center" | "right" | "bottomLeft" | "bottom" | "bottomRight") {
        return Err(bad("image.canvasSize", "unknown anchor"));
    }
    let (ax, ay) = anchor_factors("image.canvasSize", anchor)?;
    let offset = |new: u32, old: u32, factor: f64| -> Result<i32> {
        let value = ((f64::from(new) - f64::from(old)) * factor).round();
        if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
            return Err(bad("image.canvasSize", "anchor offset exceeds coordinates"));
        }
        Ok(value as i32)
    };
    Ok(CanvasRequest { old, new, offset: (offset(new.0, old.0, ax)?, offset(new.1, old.1, ay)?), extension: extension_color(s, p)? })
}
/// Cheap borrowed validation must run before floating-selection normalization,
/// job setup or any document clone. Background and inline use the same envelope.
pub(crate) fn canvas_size_dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    canvas_request(s, p).map(|_| ())
}
fn canvas_moved(rect: Rect, offset: (i32, i32)) -> Result<Rect> {
    Ok(Rect::new(
        rect.x0.checked_add(offset.0).ok_or_else(|| bad("image.canvasSize", "translated coordinates overflow"))?,
        rect.y0.checked_add(offset.1).ok_or_else(|| bad("image.canvasSize", "translated coordinates overflow"))?,
        rect.x1.checked_add(offset.0).ok_or_else(|| bad("image.canvasSize", "translated coordinates overflow"))?,
        rect.y1.checked_add(offset.1).ok_or_else(|| bad("image.canvasSize", "translated coordinates overflow"))?,
    ))
}
fn canvas_tiles(rect: Rect) -> Result<u64> {
    use photocraft_geom::{TILE_SIZE, TileCoord};
    if rect.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(rect.x0, rect.y0);
    let last = TileCoord::containing(
        rect.x1.checked_sub(1).ok_or_else(|| bad("image.canvasSize", "tile bounds"))?,
        rect.y1.checked_sub(1).ok_or_else(|| bad("image.canvasSize", "tile bounds"))?,
    );
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(|| bad("image.canvasSize", "tile coordinates overflow"))?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(|| bad("image.canvasSize", "tile coordinates overflow"))?;
    }
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| bad("image.canvasSize", "tile span"))?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| bad("image.canvasSize", "tile span"))?;
    width.checked_mul(height).ok_or_else(|| bad("image.canvasSize", "tile count overflow"))
}
fn canvas_layer_surfaces(
    layers: &[Layer],
    depth: usize,
    count: &mut usize,
    indexed: bool,
    control: &photocraft_raster::Interrupt<'_>,
    visit: &mut dyn FnMut(&Surface) -> Result<()>,
) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("image.canvasSize", "layer depth limit"));
    }
    for layer in layers {
        canvas_poll(control)?;
        *count = count.checked_add(1).filter(|count| *count <= 100_000).ok_or_else(|| bad("image.canvasSize", "layer count limit"))?;
        if layer.video.is_some() || (layer.indexed_source.is_some() && !indexed) {
            return Err(bad("image.canvasSize", "unsupported video or unpaired indexed source geometry"));
        }
        if indexed
            && (layer.import_compositing.is_some() || !layer.effects.items.is_empty() || layer.effects.psd_raw.is_some() || layer.effects.reference.is_some())
        {
            return Err(bad("image.canvasSize", "indexed source composition/effect geometry is not qualified"));
        }
        if let Some(mask) = &layer.mask {
            visit(&mask.surface)?;
        }
        if let Some(cache) = &layer.fill_cache {
            visit(&cache.surface)?;
        }
        match &layer.content {
            LayerContent::Group(group) => canvas_layer_surfaces(&group.children, depth + 1, count, indexed, control, visit)?,
            LayerContent::BackedRaster(_) => return Err(bad("image.canvasSize", "backed source geometry is unsupported")),
            LayerContent::Smart(smart) => {
                if let Some(cache) = &smart.cache {
                    visit(cache)?;
                }
                if let Some(mask) = &smart.filter_mask {
                    visit(&mask.surface)?;
                }
            }
            _ => {
                if let Some(surface) = layer.surface() {
                    visit(surface)?;
                }
            }
        }
    }
    Ok(())
}
fn canvas_translate_surface(source: &Surface, offset: (i32, i32), control: &photocraft_raster::Interrupt<'_>) -> Result<Surface> {
    canvas_poll(control)?;
    let bounds = source.try_content_bounds_with_interrupt(control).map_err(|error| canvas_error(control, error))?;
    let _ = canvas_moved(bounds, offset)?;
    let mut output = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|error| canvas_error(control, error))?;
    // Iterate allocated tiles only: a sparse, far-apart source never expands its
    // entire bounding rectangle. Unallocated pixels retain the exact raw default.
    for (coordinate, _) in source.tiles() {
        canvas_poll(control)?;
        let part = coordinate.rect().intersect(&bounds);
        if part.is_empty() {
            continue;
        }
        let destination = canvas_moved(part, offset)?;
        let _ = canvas_tiles(destination)?;
        let bytes = source.try_read_interleaved_with(part, control).map_err(|error| canvas_error(control, error))?;
        output.try_write_interleaved_with(destination, &bytes, control).map_err(|error| canvas_error(control, error))?;
    }
    canvas_poll(control)?;
    Ok(output)
}
fn canvas_translate_layers(layers: &mut [Layer], offset: (i32, i32), control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    for layer in layers {
        canvas_poll(control)?;
        if let Some(mask) = &mut layer.mask {
            let moved = canvas_translate_surface(&mask.surface, offset, control)?;
            *std::sync::Arc::make_mut(&mut mask.surface) = moved;
        }
        if let Some(cache) = &mut layer.fill_cache {
            cache.surface = canvas_translate_surface(&cache.surface, offset, control)?;
        }
        match &mut layer.content {
            LayerContent::Group(group) => canvas_translate_layers(&mut group.children, offset, control)?,
            LayerContent::Raster(surface) => {
                *surface = canvas_translate_surface(surface, offset, control)?;
            }
            LayerContent::Text(text) => {
                if let Some(cache) = &mut text.cache {
                    *cache = canvas_translate_surface(cache, offset, control)?;
                }
            }
            LayerContent::Shape(shape) => {
                if let Some(cache) = &mut shape.cache {
                    *cache = canvas_translate_surface(cache, offset, control)?;
                }
            }
            LayerContent::Smart(smart) => {
                if let Some(cache) = &mut smart.cache {
                    *cache = canvas_translate_surface(cache, offset, control)?;
                }
                if let Some(mask) = &mut smart.filter_mask {
                    let moved = canvas_translate_surface(&mask.surface, offset, control)?;
                    *std::sync::Arc::make_mut(&mut mask.surface) = moved;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
/// Compatibility seam: pre-admit all geometry before private-candidate translation.
pub(crate) fn translate_doc(doc: &mut Document, cmd: &str, dx: i32, dy: i32) -> Result<()> {
    let control = &photocraft_raster::Interrupt::NONE;
    canvas_translation_preflight(doc, (dx, dy), control).map_err(|error| bad(cmd, error.to_string()))?;
    canvas_translate_doc_with_interrupt(doc, (dx, dy), control)
}
pub(crate) fn canvas_translate_doc_with_interrupt(doc: &mut Document, offset: (i32, i32), control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    canvas_translate_doc_current_with_interrupt(doc, offset, true, control)
}
/// Crop authors replacement indexed extents/planes separately; it must not move
/// those canonical extents a second time while translating the other carriers.
pub(crate) fn canvas_translate_doc_current_with_interrupt(
    doc: &mut Document,
    offset: (i32, i32),
    translate_indexed: bool,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    canvas_poll(control)?;
    if offset == (0, 0) {
        return Ok(());
    }
    canvas_translate_layers(&mut doc.layers, offset, control)?;
    for channel in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
        channel.surface = canvas_translate_surface(&channel.surface, offset, control)?;
    }
    if let Some(selection) = &mut doc.selection {
        *selection = canvas_translate_surface(selection, offset, control)?;
    }
    if let Some(state) = &mut doc.xcf_state {
        if translate_indexed {
            for indexed in &mut state.editable_indexed {
                canvas_poll(control)?;
                indexed.raster.translate_bounds(offset.0, offset.1).map_err(|error| canvas_error(control, error))?;
            }
        }
        for mask in &mut state.filter_masks {
            canvas_poll(control)?;
            mask.surface = canvas_translate_surface(&mask.surface, offset, control)?;
            mask.bounds = canvas_moved(mask.bounds, offset)?;
        }
        for item in &mut state.items {
            canvas_poll(control)?;
            if let Some(bounds) = item.raster_bounds {
                item.raster_bounds = Some(canvas_moved(bounds, offset)?);
            }
            if let Some(mask) = &mut item.mask {
                mask.bounds = canvas_moved(mask.bounds, offset)?;
                mask.offset = [
                    mask.offset[0].checked_add(offset.0).ok_or_else(|| bad("image.canvasSize", "mask offset overflow"))?,
                    mask.offset[1].checked_add(offset.1).ok_or_else(|| bad("image.canvasSize", "mask offset overflow"))?,
                ];
            }
        }
    }
    crate::canvas_geom::transform_geometry_with_interrupt(doc, &photocraft_geom::Affine::translate(f64::from(offset.0), f64::from(offset.1)), control)
}
#[derive(Clone, Copy)]
struct BackgroundExtension {
    owner: photocraft_doc::LayerId,
    keep: Rect,
    pixel: [u8; 40],
    length: usize,
}
fn prepare_background(doc: &Document, request: CanvasRequest, control: &photocraft_raster::Interrupt<'_>) -> Result<Option<BackgroundExtension>> {
    prepare_background_for_crop(doc, request, None, control)
}
// A deleting crop retains only the old canvas intersection, so discarded old
// corners must not require representable translated coordinates. Other canvas
// operations keep the original full-canvas translation admission unchanged.
fn prepare_background_for_crop(
    doc: &Document,
    request: CanvasRequest,
    deleting_crop: Option<Rect>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Option<BackgroundExtension>> {
    if request.extension[3] <= 0.0 {
        return Ok(None);
    }
    let Some(background) = doc.layers.first().filter(|layer| layer.name == "Background" && layer.locks.transparency) else { return Ok(None) };
    let LayerContent::Raster(surface) = &background.content else { return Ok(None) };
    if doc.mode == ColorMode::Indexed || doc.xcf_state.as_ref().is_some_and(|state| !state.editable_indexed.is_empty()) {
        return Err(bad("image.canvasSize", "indexed background extension requires explicit ordinal/alpha authoring; use transparent extension"));
    }
    let format = surface.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(bad("image.canvasSize", "background extension color conversion is unsupported for this model"));
    }
    let neutral = request.extension[0] == request.extension[1]
        && request.extension[1] == request.extension[2]
        && matches!(request.extension[0], 0.0 | 1.0)
        && request.extension[3] == 1.0;
    if (doc.sample_encoding.is_some()
        || !matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc)
        || matches!(format.sample, SampleType::U32 | SampleType::F64))
        && !neutral
    {
        return Err(bad("image.canvasSize", "current interpreted/wide background extension requires qualified black or white native colors"));
    }
    if doc.xcf_state.is_some() {
        let locks = crate::xcf_item_locks::effective_layer_locks(doc, background.id)?;
        if locks.imported.content || locks.imported.alpha {
            return Err(bad("image.canvasSize", "imported background content/alpha is locked"));
        }
    }
    let original = Rect::new(
        0,
        0,
        i32::try_from(request.old.0).map_err(|_| bad("image.canvasSize", "old width"))?,
        i32::try_from(request.old.1).map_err(|_| bad("image.canvasSize", "old height"))?,
    );
    let retained_old = deleting_crop.map_or(original, |crop| original.intersect(&crop));
    let old = canvas_moved(retained_old, request.offset)?;
    let occupied = surface.try_content_bounds_with_interrupt(control).map_err(|error| canvas_error(control, error))?;
    let retained = deleting_crop.map_or(occupied, |crop| occupied.intersect(&original).intersect(&crop));
    let keep = canvas_moved(retained, request.offset)?.intersect(&old);
    let mut pixel = [0u8; 40];
    let length = format.bytes_per_pixel();
    let native = pixel.get_mut(..length).ok_or_else(|| bad("image.canvasSize", "extension pixel width"))?;
    if matches!(format.sample, SampleType::U32 | SampleType::F64) {
        for channel in 0..format.channels() {
            let value = if channel >= format.mode.color_channels() { 1.0 } else { f64::from(request.extension[0]) };
            photocraft_color::try_write_sample_f64(native, format.sample, channel, value).map_err(|error| canvas_error(control, error))?;
        }
    } else {
        let mut normalized = [0.0; 8];
        let count = photocraft_raster::from_rgba_into(&format, request.extension, &mut normalized);
        let normalized = normalized.get(..count).filter(|_| count == format.channels()).ok_or_else(|| bad("image.canvasSize", "extension channels"))?;
        photocraft_raster::encode_pixel(&format, normalized, native);
    }
    Ok(Some(BackgroundExtension { owner: background.id, keep, pixel, length }))
}

/// Translation admission covers public model/map capacities and retained native
/// payloads, not private legacy renderer allocations or process RSS.
pub(crate) fn canvas_translation_preflight(doc: &Document, offset: (i32, i32), control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    canvas_admission(doc, offset, None, control)
}
fn canvas_admission(
    doc: &Document,
    offset: (i32, i32),
    background: Option<(photocraft_doc::LayerId, Rect)>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    canvas_poll(control)?;
    crate::canvas_geom::preflight_geometry_with_interrupt(doc, control)?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, control).map_err(|error| canvas_error(control, error))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, control).map_err(|error| canvas_error(control, error))? as u64;
    let raster = crate::document_snapshot::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let mut resident = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .ok_or_else(|| bad("image.canvasSize", "clone budget overflow"))?;
    let mut work = 0u64;
    let mut scratch = 0u64;
    let limit = MAX_RESAMPLE_BYTES.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES);
    let mut visit = |surface: &Surface| -> Result<()> {
        canvas_poll(control)?;
        let count = surface.tile_count() as u64;
        let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64))
            .ok_or_else(|| bad("image.canvasSize", "tile byte overflow"))?;
        let output = if offset == (0, 0) { 0 } else { count.checked_mul(4).ok_or_else(|| bad("image.canvasSize", "translated tile count overflow"))? };
        // Raw writes validate/COW sparse maps. Charge the repeated source/map
        // traversal conservatively, before any native occupancy scan or clone.
        let scheduler = count
            .checked_mul(count.checked_add(output).ok_or_else(|| bad("image.canvasSize", "scheduler overflow"))?)
            .and_then(|n| n.checked_mul(256))
            .ok_or_else(|| bad("image.canvasSize", "scheduler overflow"))?;
        work = work
            .checked_add(scheduler)
            .and_then(|n| n.checked_add(count.checked_mul(tile_bytes)?.checked_mul(8)?))
            .filter(|n| *n <= 8_000_000_000)
            .ok_or_else(|| bad("image.canvasSize", "translation work budget exceeded"))?;
        resident = resident
            .checked_add(
                output
                    .checked_mul(tile_bytes.checked_add(1024).ok_or_else(|| bad("image.canvasSize", "tile ledger overflow"))?)
                    .ok_or_else(|| bad("image.canvasSize", "output ledger overflow"))?,
            )
            .filter(|n| *n <= limit)
            .ok_or_else(|| bad("image.canvasSize", "translation staging budget exceeded"))?;
        scratch = scratch.max(tile_bytes.checked_mul(8).and_then(|n| n.checked_add(4096)).ok_or_else(|| bad("image.canvasSize", "scratch overflow"))?);
        let bounds = surface.try_content_bounds_with_interrupt(control).map_err(|error| canvas_error(control, error))?;
        let _ = canvas_tiles(canvas_moved(bounds, offset)?)?;
        Ok(())
    };
    let indexed = doc.xcf_state.as_ref().is_some_and(|state| !state.editable_indexed.is_empty());
    canvas_layer_surfaces(&doc.layers, 0, &mut 0, indexed, control, &mut visit)?;
    for channel in doc.channels.iter().chain(doc.quick_mask.as_ref()) {
        visit(&channel.surface)?;
    }
    if let Some(selection) = &doc.selection {
        visit(selection)?;
    }
    if let Some(state) = &doc.xcf_state {
        if !state.source_items.is_empty() {
            return Err(bad("image.canvasSize", "source geometry transformation is not qualified"));
        }
        if indexed && (!state.filter_stacks.is_empty() || !state.filter_masks.is_empty() || state.floating.is_some()) {
            return Err(bad("image.canvasSize", "indexed filter/floating geometry is not qualified"));
        }
        if state.items.len() > 100_000 || state.filter_masks.len() > 100_000 {
            return Err(bad("image.canvasSize", "current geometry index exceeds limit"));
        }
        for mask in &state.filter_masks {
            visit(&mask.surface)?;
            let _ = canvas_moved(mask.bounds, offset)?;
        }
        for item in &state.items {
            canvas_poll(control)?;
            if indexed && (item.source_flags != 0 || item.composition.is_some()) {
                return Err(bad("image.canvasSize", "indexed source flags/composition geometry is not qualified"));
            }
            if let Some(bounds) = item.raster_bounds {
                let _ = canvas_moved(bounds, offset)?;
            }
            if let Some(mask) = &item.mask {
                let _ = canvas_moved(mask.bounds, offset)?;
                for (value, delta) in mask.offset.iter().zip([offset.0, offset.1]) {
                    value.checked_add(delta).ok_or_else(|| bad("image.canvasSize", "mask offset overflow"))?;
                }
            }
        }
        for indexed in &state.editable_indexed {
            canvas_poll(control)?;
            let _ = canvas_moved(indexed.raster.bounds(), offset)?;
        }
    }
    if let Some((owner, canvas)) = background {
        let layer = doc.layers.first().filter(|layer| layer.id == owner).ok_or_else(|| bad("image.canvasSize", "background owner changed"))?;
        let surface = layer.surface().ok_or_else(|| bad("image.canvasSize", "background raster absent"))?;
        let count = canvas_tiles(canvas)?;
        let source = (surface.tile_count() as u64).checked_mul(4).ok_or_else(|| bad("image.canvasSize", "background map overflow"))?;
        let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64))
            .ok_or_else(|| bad("image.canvasSize", "background tile overflow"))?;
        work = work
            .checked_add(
                count
                    .checked_mul(source.checked_add(count).ok_or_else(|| bad("image.canvasSize", "background scheduler overflow"))?)
                    .and_then(|n| n.checked_mul(256))
                    .ok_or_else(|| bad("image.canvasSize", "background scheduler overflow"))?,
            )
            .and_then(|n| n.checked_add(count.checked_mul(tile_bytes)?.checked_mul(8)?))
            .filter(|n| *n <= 8_000_000_000)
            .ok_or_else(|| bad("image.canvasSize", "background work budget exceeded"))?;
        resident = resident
            .checked_add(
                count
                    .checked_mul(tile_bytes.checked_add(1024).ok_or_else(|| bad("image.canvasSize", "background ledger overflow"))?)
                    .ok_or_else(|| bad("image.canvasSize", "background ledger overflow"))?,
            )
            .and_then(|n| n.checked_add(source.checked_mul(1024)?))
            .ok_or_else(|| bad("image.canvasSize", "background ledger overflow"))?;
        scratch =
            scratch.max(tile_bytes.checked_mul(8).and_then(|n| n.checked_add(4096)).ok_or_else(|| bad("image.canvasSize", "background scratch overflow"))?);
    }
    let _ = work;
    if resident.checked_add(scratch).is_none_or(|n| n > limit) {
        return Err(bad("image.canvasSize", "canvas staging exceeds raster budget"));
    }
    if indexed {
        let remaining = limit
            .checked_sub(resident)
            .and_then(|n| n.checked_sub(scratch))
            .ok_or_else(|| bad("image.canvasSize", "indexed validation staging exceeds budget"))?;
        let validation = crate::xcf_indexed_geometry::validate_current_with_limits(
            doc,
            photocraft_algo::wide_geometry::GeometryLimits {
                resident_bytes: remaining.min(photocraft_algo::wide_geometry::GeometryLimits::default().resident_bytes),
                work_units: 8_000_000_000u64
                    .checked_sub(work)
                    .ok_or_else(|| bad("image.canvasSize", "indexed validation work exceeds budget"))?
                    .min(photocraft_algo::wide_geometry::GeometryLimits::default().work_units),
            },
            control,
        )?;
        // The validator reports its own conservative held-source/temporary
        // envelope. This is an intentionally conservative overlapping reservation,
        // not an additional clone of shared ordinal/alpha/source payloads.
        if resident
            .checked_add(scratch)
            .and_then(|n| n.checked_add(validation.retained_bytes))
            .and_then(|n| n.checked_add(validation.scratch_bytes))
            .is_none_or(|n| n > limit)
            || work.checked_add(validation.work_units).is_none_or(|n| n > 8_000_000_000)
        {
            return Err(bad("image.canvasSize", "aggregate indexed validation limit"));
        }
    }
    canvas_poll(control)
}
fn canvas_prepare(doc: &Document, request: CanvasRequest, control: &photocraft_raster::Interrupt<'_>) -> Result<Option<BackgroundExtension>> {
    canvas_poll(control)?;
    // Admission precedes even the background occupancy scan.
    let owner = doc
        .layers
        .first()
        .filter(|layer| {
            request.extension[3] > 0.0 && layer.name == "Background" && layer.locks.transparency && matches!(&layer.content, LayerContent::Raster(_))
        })
        .map(|layer| layer.id);
    if owner.is_some() && (doc.mode == ColorMode::Indexed || doc.xcf_state.as_ref().is_some_and(|state| !state.editable_indexed.is_empty())) {
        return Err(bad("image.canvasSize", "indexed background extension requires explicit ordinal/alpha authoring; use transparent extension"));
    }
    let canvas = Rect::new(0, 0, request.new.0 as i32, request.new.1 as i32);
    canvas_admission(doc, request.offset, owner.map(|id| (id, canvas)), control)?;
    prepare_background(doc, request, control)
}
fn canvas_extend_background(doc: &mut Document, prepared: BackgroundExtension, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    canvas_poll(control)?;
    let canvas = doc.bounds();
    let layer = doc.layers.first_mut().filter(|layer| layer.id == prepared.owner).ok_or_else(|| bad("image.canvasSize", "background owner changed"))?;
    let LayerContent::Raster(surface) = &mut layer.content else { return Err(bad("image.canvasSize", "background raster changed")) };
    let before = surface.clone();
    let pixel = prepared
        .pixel
        .get(..prepared.length)
        .filter(|_| prepared.length == surface.format().bytes_per_pixel() && prepared.length != 0)
        .ok_or_else(|| bad("image.canvasSize", "background pixel mismatch"))?;
    let maximum = (photocraft_geom::TILE_SIZE as usize)
        .checked_mul(photocraft_geom::TILE_SIZE as usize)
        .and_then(|n| n.checked_mul(prepared.length))
        .ok_or_else(|| bad("image.canvasSize", "background scratch overflow"))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(maximum).map_err(|_| bad("image.canvasSize", "background scratch allocation failed"))?;
    if bytes.capacity() as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES || bytes.capacity() > maximum.saturating_add(4096) {
        return Err(bad("image.canvasSize", "background scratch capacity exceeded"));
    }
    while bytes.len() < maximum {
        canvas_poll(control)?;
        bytes.resize(bytes.len().saturating_add(64 * 1024).min(maximum), 0);
    }
    for coordinate in canvas.tiles() {
        canvas_poll(control)?;
        let part = coordinate.rect().intersect(&canvas);
        let width = usize::try_from(i64::from(part.x1) - i64::from(part.x0)).map_err(|_| bad("image.canvasSize", "background width"))?;
        let height = usize::try_from(i64::from(part.y1) - i64::from(part.y0)).map_err(|_| bad("image.canvasSize", "background height"))?;
        let row_bytes = width.checked_mul(prepared.length).ok_or_else(|| bad("image.canvasSize", "background row overflow"))?;
        let length = row_bytes.checked_mul(height).ok_or_else(|| bad("image.canvasSize", "background tile overflow"))?;
        let output = bytes.get_mut(..length).ok_or_else(|| bad("image.canvasSize", "background scratch range"))?;
        for (row, data) in output.chunks_exact_mut(row_bytes).enumerate() {
            canvas_poll(control)?;
            for target in data.chunks_exact_mut(prepared.length) {
                target.copy_from_slice(pixel);
            }
            let y = part
                .y0
                .checked_add(i32::try_from(row).map_err(|_| bad("image.canvasSize", "background row"))?)
                .ok_or_else(|| bad("image.canvasSize", "background row overflow"))?;
            let kept =
                Rect::new(part.x0, y, part.x1, y.checked_add(1).ok_or_else(|| bad("image.canvasSize", "background row overflow"))?).intersect(&prepared.keep);
            if !kept.is_empty() {
                let start = usize::try_from(i64::from(kept.x0) - i64::from(part.x0))
                    .ok()
                    .and_then(|n| n.checked_mul(prepared.length))
                    .ok_or_else(|| bad("image.canvasSize", "background keep offset"))?;
                let count = usize::try_from(i64::from(kept.x1) - i64::from(kept.x0))
                    .ok()
                    .and_then(|n| n.checked_mul(prepared.length))
                    .ok_or_else(|| bad("image.canvasSize", "background keep length"))?;
                let end = start.checked_add(count).ok_or_else(|| bad("image.canvasSize", "background keep overflow"))?;
                before
                    .try_read_interleaved_into_with(kept, data.get_mut(start..end).ok_or_else(|| bad("image.canvasSize", "background keep range"))?, control)
                    .map_err(|error| canvas_error(control, error))?;
            }
        }
        surface.try_write_interleaved_with(part, output, control).map_err(|error| canvas_error(control, error))?;
    }
    canvas_poll(control)
}

/// Image â†’ Canvas Size.
fn canvas_size(s: &mut Session, p: &Value) -> Result<Value> {
    let request = canvas_request(s, p)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let baseline = active.doc.clone();
    let witness = baseline.downgrade();
    let revision = active.revision;
    let work_base = baseline.clone();
    crate::jobs::run(
        s,
        "Canvas Size",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Canvas Size", |control| {
                let background = canvas_prepare(&work_base, request, control)?;
                canvas_poll(control)?;
                let mut document = (*work_base).clone();
                canvas_translate_doc_with_interrupt(&mut document, request.offset, control)?;
                document.size = Size::new(request.new.0, request.new.1);
                crate::canvas_geom::refresh_with_interrupt(&mut document, crate::canvas_geom::Refresh::Shapes, control)?;
                if request.extension[3] == 0.0 && (request.new.0 > request.old.0 || request.new.1 > request.old.1) {
                    if document.xcf_state.is_some() && document.background_mut().is_some() {
                        return Err(bad("image.canvasSize", "typed Background transparency conversion is unsupported"));
                    }
                    if let Some(background) = document.background_mut() {
                        crate::extra_cmds::unlock_background(background);
                    }
                }
                if let Some(background) = background {
                    canvas_extend_background(&mut document, background, control)?;
                }
                canvas_poll(control)?;
                Ok((document, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let active = session.active().ok_or(EngineError::NoDocument)?;
            let original = witness.upgrade().ok_or_else(|| bad("image.canvasSize", "captured canvas state expired"))?;
            if active.doc.id != baseline.id || active.revision != revision || !active.doc.shares_state(&original) {
                return Err(bad("image.canvasSize", "target changed while preparing canvas"));
            }
            ctx.stage(0.95, 1.0, "Canvas Size", |control| {
                canvas_poll(control)?;
                session.edit_with_sampler_map(
                    "Canvas Size",
                    control,
                    Some(photocraft_geom::Affine::translate(f64::from(request.offset.0), f64::from(request.offset.1))),
                    move |current, _| {
                        *current = document;
                        Ok(())
                    },
                )?;
                Ok(json!({ "width": request.new.0, "height": request.new.1, "offset": [request.offset.0, request.offset.1] }))
            })
        },
    )
}

/// The largest side an explicit crop may give the canvas (the `file.new` and Canvas Size limit).
const MAX_CROP_SIDE: i32 = 300_000;

/// Cheap borrowed validation before floating normalization or parameter cloning.
pub(crate) fn crop_dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    if !p.is_null()
        && !p.as_object().is_some_and(|object| {
            object.len() <= 13
                && object.keys().all(|key| {
                    matches!(
                        key.as_str(),
                        "x" | "y"
                            | "width"
                            | "height"
                            | "deleteCroppedPixels"
                            | "contentAware"
                            | "documentId"
                            | "expectedRevision"
                            | "angle"
                            | "targetWidth"
                            | "targetHeight"
                            | "resolution"
                            | "resample"
                    )
                })
        })
    {
        return Err(bad("image.crop", "expected bounded Crop parameters"));
    }
    let state = s.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX {
        return Err(bad("image.crop", "document revision limit"));
    }
    if s.command_has_pending_job() || s.command_has_job_on(state.doc.id) {
        return Err(bad("image.crop", "target document has pending work"));
    }
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(document), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| bad("image.crop", "captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| bad("image.crop", "captured ID exceeds range"))
            };
            if exact(document)? == 0 || exact(document)? != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(bad("image.crop", "captured target changed"));
            }
            if state.floating.is_some()
                || state.coalesce.is_some()
                || s.has_jobs()
                || s.coalesce_request.is_some()
                || s.color_restrict.is_some()
                || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
            {
                return Err(bad("image.crop", "captured crop target has floating or transient work"));
            }
        }
        _ => return Err(bad("image.crop", "documentId and expectedRevision must be supplied together")),
    }
    if let Some(state) = &state.doc.xcf_state {
        if !state.source_items.is_empty() || !state.filter_stacks.is_empty() || (!state.editable_indexed.is_empty() && !state.filter_masks.is_empty()) {
            return Err(bad("image.crop", "source or filter geometry transformation is not qualified"));
        }
        if !state.editable_indexed.is_empty() && (state.editable_indexed.len() > 10_000 || state.items.len() > 100_000) {
            return Err(bad("image.crop", "indexed geometry index limit"));
        }
    }
    let explicit = crop_rectangle(p)?;
    let angle = crop_angle(p)?;
    let target = crop_target(p)?;
    if angle.is_some() && explicit.is_none() {
        return Err(bad("image.crop", "angle requires explicit crop rectangle"));
    }
    if let Some((w, h)) = target.size
        && u64::from(w)
            .checked_mul(u64::from(h))
            .and_then(|n| n.checked_mul(state.doc.pixel_format().bytes_per_pixel() as u64))
            .is_none_or(|n| n > MAX_RESAMPLE_BYTES)
    {
        return Err(bad("image.crop", "target output raster budget"));
    }
    let _ = crop_delete(p)?;
    let _ = crop_content_aware(p)?;
    Ok(())
}

/// The Crop tool's W x H x Resolution: the size (px) the cropped area is resampled to and the
/// resolution the document gets, both optional.
#[derive(Clone, Copy)]
struct CropTarget {
    size: Option<(u32, u32)>,
    resolution: Option<f32>,
    filter: Resample,
}

impl CropTarget {
    /// Applies the target to a document that has just been cropped (inside the crop's history step).
    fn apply_with_interrupt(&self, doc: &mut Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        if let Some((w, h)) = self.size {
            let old = (doc.size.width, doc.size.height);
            let plan = prepare_image_resize(doc, old, (w, h), Some(self.filter), control)?;
            apply_image_resize(doc, None, plan, (w, h), control)?;
        }
        if let Some(resolution) = self.resolution {
            doc.set_physical_resolution(photocraft_doc::PhysicalResolution::square(f64::from(resolution))).map_err(|e| bad("image.crop", e.to_string()))?;
        }
        crop_poll(control)
    }
}

/// The highest resolution (ppi) a crop may set, as for Image Size.
const MAX_CROP_RESOLUTION: f64 = 10_000.0;

/// `image.crop`'s optional `targetWidth`/`targetHeight` (whole px, both or neither), `resolution`
/// (ppi) and `resample` (as for `image.imageSize`; "none" is refused with a target size).
fn crop_target(p: &Value) -> Result<CropTarget> {
    const CMD: &str = "image.crop";
    let side = |key: &str| -> Result<Option<u32>> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (1..=MAX_CROP_SIDE as u32).contains(n))
                .map(Some)
                .ok_or_else(|| bad(CMD, format!("`{key}` = {v} must be a whole number of pixels from 1 to {MAX_CROP_SIDE}"))),
        }
    };
    let size = match (side("targetWidth")?, side("targetHeight")?) {
        (Some(w), Some(h)) => Some((w, h)),
        (None, None) => None,
        _ => return Err(bad(CMD, "`targetWidth` and `targetHeight` go together")),
    };
    let resolution = match p.get("resolution") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let r = v.as_f64().filter(|r| r.is_finite() && (1.0..=MAX_CROP_RESOLUTION).contains(r));
            Some(r.ok_or_else(|| bad(CMD, format!("`resolution` = {v} must be a number of pixels per inch from 1 to {MAX_CROP_RESOLUTION}")))? as f32)
        }
    };
    let filter = match p.get("resample") {
        None | Some(Value::Null) => Some(Resample::Bicubic),
        Some(v) => parse_resample(v.as_str().ok_or_else(|| bad(CMD, "resample must be a string"))?)?,
    };
    let filter = match (filter, size) {
        (Some(f), _) => f,
        (None, None) => Resample::Bicubic,
        (None, Some(_)) => return Err(bad(CMD, "a `targetWidth` x `targetHeight` crop needs resampling: `resample` can't be \"none\"")),
    };
    Ok(CropTarget { size, resolution, filter })
}

/// A crop's result: the frame it cropped to, plus the document's size and resolution afterwards.
fn crop_result(s: &Session, mut frame: Value) -> Value {
    if let (Some(d), Some(o)) = (s.active(), frame.as_object_mut()) {
        o.insert("documentWidth".into(), json!(d.doc.size.width));
        o.insert("documentHeight".into(), json!(d.doc.size.height));
        o.insert("resolution".into(), json!(d.doc.resolution_dpi));
    }
    frame
}

/// The largest crop angle accepted (degrees), as for Image Rotation â€º Arbitrary.
const MAX_CROP_ANGLE: f64 = 3600.0;

/// `image.crop`'s optional `angle` in degrees, normalised to (-180, 180]; `None` when it is absent,
/// null or (after normalising) zero, so an unrotated crop takes the plain, resampling-free path.
fn crop_angle(p: &Value) -> Result<Option<f64>> {
    let a = match p.get("angle") {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v.as_f64().ok_or_else(|| bad("image.crop", "`angle` must be a number of degrees"))?,
    };
    if !a.is_finite() || a.abs() > MAX_CROP_ANGLE {
        return Err(bad("image.crop", format!("`angle` must be a finite number of degrees within Â±{MAX_CROP_ANGLE}")));
    }
    let a = a.rem_euclid(360.0);
    let a = if a > 180.0 { a - 360.0 } else { a };
    Ok((a.abs() >= 1e-9).then_some(a))
}

/// The Crop tool's rotated frame: `r` (document px) turned clockwise on screen by `angle` degrees
/// about its centre. The document is rotated counter-clockwise by `angle` (Image Rotation â€º
/// Arbitrary: the canvas grows to fit, the Background fills what the rotation uncovers with the
/// background colour, other layers keep transparency there and every pixel of theirs), which makes
/// the frame upright, then cropped to it (`deleteCroppedPixels` as for an unrotated crop), all in
/// one undo step.
fn rotated_crop_rect(size: Size, r: Rect, angle: f64) -> Result<Rect> {
    let (w, h) = (f64::from(r.width()), f64::from(r.height()));
    let (cx, cy) = (f64::from(r.x0) + w / 2.0, f64::from(r.y0) + h / 2.0);
    let new = crate::mode_cmds::rotated_size(size, -angle);
    let (sn, cs) = (-angle).to_radians().sin_cos();
    let (dx, dy) = (cx - f64::from(size.width) / 2.0, cy - f64::from(size.height) / 2.0);
    let (x, y) = ((f64::from(new.width) / 2.0 + cs * dx - sn * dy - w / 2.0).round(), (f64::from(new.height) / 2.0 + sn * dx + cs * dy - h / 2.0).round());
    let limit = f64::from(i32::MAX / 2);
    if !x.is_finite() || !y.is_finite() || x.abs() > limit || y.abs() > limit {
        return Err(bad("image.crop", "rotated frame coordinate range"));
    }
    let (x, y) = (x as i32, y as i32);
    let x1 =
        x.checked_add(i32::try_from(r.width()).map_err(|e| bad("image.crop", e.to_string()))?).ok_or_else(|| bad("image.crop", "rotated width overflow"))?;
    let y1 =
        y.checked_add(i32::try_from(r.height()).map_err(|e| bad("image.crop", e.to_string()))?).ok_or_else(|| bad("image.crop", "rotated height overflow"))?;
    Ok(Rect::new(x, y, x1, y1))
}

fn crop_content_aware(p: &Value) -> Result<bool> {
    match p.get("contentAware") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(value) => Err(bad("image.crop", format!("`contentAware` must be true or false, not {value}"))),
    }
}
fn crop_delete(p: &Value) -> Result<bool> {
    match p.get("deleteCroppedPixels") {
        None => Ok(true),
        Some(value) => value.as_bool().ok_or_else(|| bad("image.crop", "deleteCroppedPixels must be boolean")),
    }
}
fn crop_rectangle(p: &Value) -> Result<Option<Rect>> {
    let keys = ["x", "y", "width", "height"];
    let supplied = keys.iter().filter(|key| p.get(**key).is_some()).count();
    if supplied == 0 {
        return Ok(None);
    }
    if supplied != 4 {
        return Err(bad("image.crop", "x, y, width and height must be supplied together"));
    }
    let number = |key| -> Result<i32> {
        let n =
            p.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).ok_or_else(|| bad("image.crop", "rectangle fields must be finite numbers"))?.round();
        if n < f64::from(i32::MIN) || n > f64::from(i32::MAX) {
            return Err(bad("image.crop", "rectangle exceeds the 32-bit coordinate range"));
        }
        Ok(n as i32)
    };
    let (x, y, w, h) = (number("x")?, number("y")?, number("width")?, number("height")?);
    if w <= 0 || h <= 0 || w > MAX_CROP_SIDE || h > MAX_CROP_SIDE {
        return Err(bad("image.crop", format!("crop dimensions must be positive and within the {MAX_CROP_SIDE} px limit")));
    }
    let r = Rect::new(
        x,
        y,
        x.checked_add(w).ok_or_else(|| bad("image.crop", "crop right edge overflows the 32-bit coordinate range"))?,
        y.checked_add(h).ok_or_else(|| bad("image.crop", "crop bottom edge overflows the 32-bit coordinate range"))?,
    );
    let _ = crop_offset(r)?;
    Ok(Some(r))
}
/// Image â†’ Crop: prepare privately from the frozen original tab, then commit once.
fn crop(s: &mut Session, p: &Value) -> Result<Value> {
    crop_dispatch_preflight(s, p, s.command_is_background())?;
    let explicit = crop_rectangle(p)?;
    let delete = crop_delete(p)?;
    let content_aware = crop_content_aware(p)?;
    let angle = crop_angle(p)?;
    let target = crop_target(p)?;
    let background = s.tools.background;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let work_base = baseline.clone();
    crate::jobs::run(
        s,
        "Crop",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Crop", |control| {
                crop_poll(control)?;
                let requested_rectangle = match explicit {
                    Some(rectangle) => rectangle,
                    None => match &work_base.selection {
                        Some(selection) => {
                            selection.try_content_bounds_with_interrupt(control).map_err(|e| crop_error(control, e))?.intersect(&work_base.bounds())
                        }
                        None => Rect::EMPTY,
                    },
                };
                if requested_rectangle.is_empty() {
                    return Err(bad("image.crop", "nothing to crop: pass x/y/width/height or make a selection"));
                }
                let mut sampler = photocraft_geom::Affine::IDENTITY;
                let rectangle = if let Some(angle) = angle { rotated_crop_rect(work_base.size, requested_rectangle, angle)? } else { requested_rectangle };
                let mut document = if let Some(angle) = angle {
                    let document =
                        crate::mode_cmds::rotate_document_with_interrupt(&work_base, -angle, photocraft_algo::transform::Interp::Bicubic, background, control)?;
                    let degrees = f64::from((-angle) as f32);
                    sampler = photocraft_geom::Affine::translate(-f64::from(work_base.size.width) / 2.0, -f64::from(work_base.size.height) / 2.0)
                        .then(&photocraft_geom::Affine::rotate(degrees.to_radians()))
                        .then(&photocraft_geom::Affine::translate(f64::from(document.size.width) / 2.0, f64::from(document.size.height) / 2.0));
                    document
                } else {
                    crop_prepare(&work_base, rectangle, delete, control)?;
                    (*work_base).clone()
                };
                crop_prepare(&document, rectangle, delete, control)?;
                let extension = if rectangle.x0 < 0 || rectangle.y0 < 0 || rectangle.x1 > document.bounds().x1 || rectangle.y1 > document.bounds().y1 {
                    let request = CanvasRequest {
                        old: (document.size.width, document.size.height),
                        new: (rectangle.width(), rectangle.height()),
                        offset: crop_offset(rectangle)?,
                        extension: [background[0], background[1], background[2], 1.0],
                    };
                    prepare_background_for_crop(&document, request, delete.then_some(rectangle), control)?
                } else {
                    None
                };
                let indexed = crop_stage_indexed(&document, rectangle, delete, control)?;
                crop_doc_with_interrupt(&mut document, rectangle, delete, indexed, control)?;
                if let Some(extension) = extension {
                    canvas_extend_background(&mut document, extension, control)?;
                }
                if content_aware {
                    // Preserve the upstream fill geometry in the privately staged document.
                    // This existing backend keeps its own canvas limit; cancellation is
                    // checked at its boundary as for the upstream content-aware operation.
                    let coverage = if let Some(angle) = angle {
                        let (sn, cs) = (-angle).to_radians().sin_cos();
                        let new = crate::mode_cmds::rotated_size(work_base.size, -angle);
                        let ox = f64::from(rectangle.x0) - f64::from(new.width) / 2.0;
                        let oy = f64::from(rectangle.y0) - f64::from(new.height) / 2.0;
                        crate::crop_fill::Coverage {
                            map: [
                                cs,
                                sn,
                                -sn,
                                cs,
                                cs * ox + sn * oy + f64::from(work_base.size.width) / 2.0,
                                -sn * ox + cs * oy + f64::from(work_base.size.height) / 2.0,
                            ],
                            width: f64::from(work_base.size.width),
                            height: f64::from(work_base.size.height),
                            rotated: true,
                        }
                    } else {
                        crate::crop_fill::Coverage::crop(rectangle, work_base.size.width, work_base.size.height)
                    };
                    crop_poll(control)?;
                    crate::crop_fill::fill_uncovered(&mut document, active, &coverage)?;
                    crop_poll(control)?;
                }
                document.selection = None;
                sampler = sampler.then(&photocraft_geom::Affine::translate(-f64::from(rectangle.x0), -f64::from(rectangle.y0)));
                let cropped_size = document.size;
                target.apply_with_interrupt(&mut document, control)?;
                if document.size != cropped_size {
                    sampler = sampler.then(&photocraft_geom::Affine {
                        m: [
                            f64::from(document.size.width) / f64::from(cropped_size.width.max(1)),
                            0.0,
                            0.0,
                            f64::from(document.size.height) / f64::from(cropped_size.height.max(1)),
                            0.0,
                            0.0,
                        ],
                    });
                }

                crop_poll(control)?;
                Ok((document, requested_rectangle, sampler, ctx.clone()))
            })
        },
        move |session, (document, rectangle, sampler, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id || current.revision != revision || !current.doc.shares_state(&baseline) {
                return Err(bad("image.crop", "target changed while preparing crop"));
            }
            ctx.stage(0.95, 1.0, "Crop", |control| {
                crop_poll(control)?;
                session.edit_with_sampler_map("Crop", control, Some(sampler), move |current, _| {
                    *current = document;
                    Ok(())
                })?;
                let mut result = json!({"x":rectangle.x0,"y":rectangle.y0,"width":rectangle.width(),"height":rectangle.height()});
                if let (Some(angle), Some(map)) = (angle, result.as_object_mut()) {
                    map.insert("angle".into(), json!(angle));
                }
                Ok(crop_result(session, result))
            })
        },
    )
}
#[cfg(test)]
#[path = "image_cmds/crop_controlled_tests.rs"]
mod crop_controlled_tests;
/// Image â†’ Trim.
fn trim(s: &mut Session, p: &Value) -> Result<Value> {
    trim_controlled::run(s, p)
}

/// Image â†’ Mode â†’ colour model, through the CMS (see [`crate::color_cmds::convert_mode`]).
fn convert_mode(s: &mut Session, mode: ColorMode, p: &Value) -> Result<Value> {
    crate::color_cmds::convert_mode(s, mode, p)
}

/// Image â†’ Mode â†’ 8/16/32 Bits/Channel.
fn convert_depth(s: &mut Session, depth: SampleType, p: &Value) -> Result<Value> {
    depth_controlled::convert_depth(s, depth, p)
}

mod duplicate_controlled;
mod duplicate_refs;
pub(crate) fn duplicate_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    duplicate_controlled::dispatch_preflight(s, p, background)
}

/// Image â†’ Duplicate.
fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    duplicate_controlled::run(s, p)
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

/// Image menu command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "image.imageSize",
            "Image Size…",
            ["Image"],
            r##"{"width":number,"height":number,"sizeUnit":"px|in|mm|pt|pc|document"="px","resolution":ppi OR "resolutionX":ppi,"resolutionY":ppi (each optional; preserve omitted axis),"percent":%?,"keepPrintSize":bool=false,"resample":"automatic|bicubicSmoother|bicubicSharper|bicubicAutomatic|bicubic|bilinear|nearest|lanczos|preserveDetails|none"="bicubic","documentId":canonical decimal string,"expectedRevision":canonical decimal string (optional paired captured target)}"##,
            has_doc,
            image_size
        ),
        spec!(
            "image.canvasSize",
            "Canvas Size…",
            ["Image"],
            r##"{"width":px,"height":px,"relative":bool=false,"anchor":"topLeft|top|topRight|left|center|right|bottomLeft|bottom|bottomRight"="center","extensionColor":"background|foreground|white|black|transparent|#rrggbb"="background","documentId":canonical decimal string,"expectedRevision":canonical decimal string (optional paired captured target)}"##,
            has_doc,
            canvas_size
        ),
        spec!(
            "image.crop",
            "Crop",
            ["Image"],
            r##"{"x":px,"y":px,"width":px,"height":px,"angle":-3600..3600=0,"deleteCroppedPixels":bool=true,"contentAware":bool=false,"targetWidth":px,"targetHeight":px,"resolution":ppi,"resample":"bicubic|bicubicSmoother|bicubicSharper|bicubicAutomatic|bilinear|nearest|lanczos|preserveDetails"="bicubic","documentId"?:canonical-u64-string,"expectedRevision"?:canonical-u64-string}; rectangle fields and target dimensions each all-or-none; captured pair all-or-none â†’ {x,y,width,height,documentWidth,documentHeight,resolution}"##,
            has_doc,
            crop
        ),
        spec!(
            "image.trim",
            "Trim…",
            ["Image"],
            r##"{"basedOn":"transparent|topLeft|bottomRight"="transparent","top":bool=true,"bottom":bool=true,"left":bool=true,"right":bool=true,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            trim
        ),
        spec!(
            "image.mode.rgb",
            "RGB Color",
            ["Image", "Mode"],
            r##"{"profile":"<builtin id>|working|/path/to/profile.icc"=working,"intent":"perceptual|relative|saturation|absolute"="relative","bpc":bool=true,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            |s, p| convert_mode(s, ColorMode::Rgb, p)
        ),
        spec!(
            "image.mode.grayscale",
            "Grayscale",
            ["Image", "Mode"],
            r##"{"profile":"<builtin id>|working|/path/to/profile.icc"=working,"intent":"perceptual|relative|saturation|absolute"="relative","bpc":bool=true,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            |s, p| convert_mode(s, ColorMode::Grayscale, p)
        ),
        spec!(
            "image.mode.cmyk",
            "CMYK Color",
            ["Image", "Mode"],
            r##"{"profile":"<builtin id>|working|/path/to/profile.icc"=working,"intent":"perceptual|relative|saturation|absolute"="relative","bpc":bool=true,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            |s, p| convert_mode(s, ColorMode::Cmyk, p)
        ),
        spec!(
            "image.mode.lab",
            "Lab Color",
            ["Image", "Mode"],
            r##"{"profile":"<builtin id>|working|/path/to/profile.icc"=working,"intent":"perceptual|relative|saturation|absolute"="relative","bpc":bool=true,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            |s, p| convert_mode(s, ColorMode::Lab, p)
        ),
        spec!(
            "image.mode.bits8",
            "8 Bits/Channel",
            ["Image", "Mode"],
            r##"{"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            |s, p| convert_depth(s, SampleType::U8, p)
        ),
        spec!(
            "image.mode.bits16",
            "16 Bits/Channel",
            ["Image", "Mode"],
            r##"{"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            |s, p| convert_depth(s, SampleType::U16, p)
        ),
        spec!(
            "image.mode.bits32",
            "32 Bits/Channel",
            ["Image", "Mode"],
            r##"{"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            |s, p| convert_depth(s, SampleType::F32, p)
        ),
        spec!(
            "image.duplicate",
            "Duplicate…",
            ["Image"],
            r##"{"name":str,"mergedOnly":bool=false,"documentId":"canonical u64 string"?,"expectedRevision":"canonical u64 string"?}"##,
            has_doc,
            duplicate
        ),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn sampler_geometry_failed_edit_does_not_arm_a_later_transaction() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":10,"height":10})).unwrap();
        s.execute("view.colorSamplers.add", json!({"x":3,"y":4})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let points = s.active().unwrap().color_samplers.clone();
        let map = Some(photocraft_geom::Affine::translate(2.0, 1.0));
        let failure: Result<()> = s.edit_with_sampler_map("Fail geometry", &photocraft_raster::Interrupt::NONE, map, |doc, _| {
            doc.size = Size::new(20, 20);
            Err(EngineError::Other("synthetic private stage failure".into()))
        });
        assert!(failure.is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().color_samplers, points);
        assert!(s.sampler_map.is_none());
        let cancel = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(s.edit_with_sampler_map("Cancelled geometry", &cancel, map, |_, _| Ok(())).is_err());
        assert!(s.sampler_map.is_none());
        s.edit("Unrelated metadata", |doc, _| {
            doc.layers[0].name.push_str(" changed");
            Ok(())
        })
        .unwrap();
        assert_eq!(s.active().unwrap().color_samplers, points);
        s.execute("image.imageSize", json!({"width":20,"height":20})).unwrap();
        assert_eq!(s.active().unwrap().color_samplers, vec![[6.0, 8.0]]);
        assert!(s.sampler_map.is_none());
    }

    use super::*;

    #[test]
    fn crop_rejects_rectangles_that_would_wrap() {
        let mut s = session();
        // 2^32 + 100 wrapped to `x = 100` through the narrowing casts: a crop rectangle
        // somewhere unrelated to the numbers passed.
        let err = s.execute("image.crop", json!({"x": 4_294_967_396_i64, "y": 0, "width": 10, "height": 10})).unwrap_err();
        assert!(err.to_string().contains("32-bit"), "{err}");
        // In-range rectangles past the canvas still work (clamped by the crop itself).
        s.execute("image.crop", json!({"x": -10, "y": -10, "width": 1000, "height": 1000})).unwrap();
    }

    /// A crop whose translation to the origin can't be represented in i32 is rejected before
    /// anything changes (#959): `-i32::MIN` overflowed the negation in `crop_doc`, `x + width`
    /// past i32::MAX silently shortened the crop, and an origin one past i32::MIN moved the
    /// layer pixels beyond i32::MAX (a length-mismatch panic while translating surfaces).
    #[test]
    fn crop_rejects_origins_that_cannot_move_to_zero() {
        const MIN: i64 = i32::MIN as i64;
        const MAX: i64 = i32::MAX as i64;
        let cases = [
            json!({"x": MIN, "y": 0, "width": 1, "height": 1}),
            json!({"x": 0, "y": MIN, "width": 1, "height": 1}),
            json!({"x": MIN, "y": MIN, "width": 1, "height": 1}),
            json!({"x": MIN, "y": 0, "width": 1, "height": 1, "deleteCroppedPixels": false}),
            json!({"x": MAX - 5, "y": 0, "width": 10, "height": 1}),
            json!({"x": 0, "y": MAX - 5, "width": 1, "height": 10}),
            json!({"x": MIN + 1, "y": 0, "width": 1, "height": 1, "deleteCroppedPixels": false}),
            json!({"x": 0, "y": MIN + 1, "width": 1, "height": 1, "deleteCroppedPixels": false}),
        ];
        for p in cases {
            let mut s = session();
            let before = s.active().unwrap().doc.clone();
            let (rev, steps) =
                (s.active().unwrap().revision, s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().len());
            let err = s.execute("image.crop", p.clone()).unwrap_err();
            assert!(matches!(err, EngineError::BadParams { .. }), "{p}: {err}");
            assert!(err.to_string().contains("32-bit"), "{p}: {err}");
            let st = s.active().unwrap();
            assert!(st.doc.shares_state(&before), "{p}: document changed");
            assert_eq!(st.doc.size, Size::new(40, 20), "{p}");
            assert_eq!(st.doc.layers[1].surface().unwrap().content_bounds(), Rect::new(10, 5, 20, 15), "{p}");
            assert_eq!(
                (st.revision, st.history.entries(&crate::read_control_none()).unwrap().values().to_vec().len()),
                (rev, steps),
                "{p}: history step recorded"
            );
        }
        // Control: a legal negative origin past the canvas still crops (and extends) the canvas.
        let mut s = session();
        s.execute("image.crop", json!({"x": -10, "y": -5, "width": 60, "height": 30})).unwrap();
        assert_eq!(doc(&s).size, Size::new(60, 30));
        assert_eq!(doc(&s).layers[1].surface().unwrap().pixel(20, 10), vec![1.0, 0.0, 0.0, 1.0]);
        // A far-off crop that deletes the (non-overlapping) pixels has nothing left to move.
        let mut s = session();
        s.execute("image.crop", json!({"x": MIN + 1, "y": 0, "width": 1, "height": 1})).unwrap();
        assert_eq!(doc(&s).size, Size::new(1, 1));
    }

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(10, 5, 20, 15), &[1.0, 0.0, 0.0, 1.0]);
            l.mask = Some(photocraft_doc::LayerMask::reveal_all());
            l.effects.items.push(Effect::default_drop_shadow());
            Ok(())
        })
        .unwrap();
        s
    }

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    // Unrun early-dispatch atomicity controls with real valid/stale floating.
    #[test]
    fn image_size_invalid_or_captured_floating_refuses_before_normalization() {
        for stale in [false, true] {
            let mut s = session();
            s.execute("select.all", json!({})).unwrap();
            s.edit("Selected real redo", |document, _| {
                document.name = "redo after refusal".into();
                Ok(())
            })
            .unwrap();
            assert!(s.undo());
            s.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
            if stale {
                s.active_mut().unwrap().floating.as_mut().unwrap().revision = 0;
            }
            let state = s.active().unwrap();
            let before = state.doc.clone();
            let revision = state.revision;
            let floating = state.floating.as_ref().unwrap().clone();
            let labels = state.history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
            let redo = state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
            let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
            for params in [
                json!({"width":0}),
                json!({"resample":"unknown"}),
                json!({"width":80,"documentId":before.id.0.to_string(),"expectedRevision":revision.to_string()}),
                json!({"width":80,"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
                json!({"resample":"x".repeat(1<<20)}),
                json!({"unused":[0,1,2]}),
            ] {
                assert!(s.execute("image.imageSize", params).is_err());
                let state = s.active().unwrap();
                assert!(state.doc.shares_state(&before));
                assert_eq!(state.revision, revision);
                let now = state.floating.as_ref().unwrap();
                assert_eq!((now.layer, now.offset, now.revision), (floating.layer, floating.offset, floating.revision));
                assert!(std::sync::Arc::ptr_eq(&now.parts, &floating.parts));
                assert_eq!(state.history.entry_labels().collect::<Vec<_>>(), labels);
                assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), redo);
                assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native);
            }
        }
        // Only captured/invalid requests refuse early: existing authorized
        // legacy auto-anchor remains available for a valid no-pair resize.
        let mut s = session();
        s.execute("select.all", json!({})).unwrap();
        s.execute("select.float", json!({"dx":1,"copy":true})).unwrap();
        image_size_dispatch_preflight(&s, &json!({"width":80,"resample":"nearest"}), false).unwrap();
        s.execute("image.imageSize", json!({"width":80,"resample":"nearest"})).unwrap();
        assert!(s.active().unwrap().floating.is_none());
    }
    // Unrun Image Size token/job/fence drafts; source admission stays unchanged.
    #[test]
    fn image_size_captured_pair_is_exact_and_refuses_stale_or_partial_target() {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        for parameters in [
            json!({"width":80,"documentId":before.id.0.to_string()}),
            json!({"width":80,"documentId":before.id.0.to_string(),"expectedRevision":revision}),
            json!({"width":80,"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
        ] {
            assert!(s.execute("image.imageSize", parameters).is_err());
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().revision, revision);
        }
        let response = s
            .execute("image.imageSize", json!({"width":80,"resample":"nearest","documentId":before.id.0.to_string(),"expectedRevision":revision.to_string()}))
            .unwrap();
        assert_eq!(response, json!({"width":80,"height":40}));
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
    #[test]
    fn image_size_controlled_preparation_and_edit_cancel_keep_native_model_redo_and_dpi() {
        let mut s = session();
        s.edit("Exact physical axes", |document, _| {
            document
                .set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.25, y_dpi: 287.75, unit: photocraft_doc::PhysicalUnit::Millimeter })
                .unwrap();
            Ok(())
        })
        .unwrap();
        s.edit("Real redo sentinel", |document, _| {
            document.name = "redo retained".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(matches!(prepare_image_resize(&before, (40, 20), (80, 40), Some(Resample::Nearest), &cancelled), Err(EngineError::Cancelled)));
        let plan = prepare_image_resize(&before, (40, 20), (80, 40), Some(Resample::Nearest), &photocraft_raster::Interrupt::NONE).unwrap();
        assert!(matches!(
            s.edit_with_interrupt("Cancelled resize", &cancelled, move |document, _| apply_image_resize(document, None, plan, (80, 40), &cancelled)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), native);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.name, "redo retained");
        assert_eq!(s.active().unwrap().doc.effective_resolution(), before.effective_resolution());
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn image_size_job_cancel_revision_fence_and_original_tab_are_atomic() {
        use crate::jobs::Started;
        let mut s = session();
        s.edit("Real redo", |document, _| {
            document.name = "redo sentinel".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let Started::Job(job) = s.start("image.imageSize", json!({"width":80,"resample":"nearest"})).unwrap() else {
            panic!("native Image Size must prepare a job")
        };
        assert!(s.cancel_job(job));
        s.join_cancelled_jobs();
        assert!(s.poll_jobs().is_empty());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.redo());
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let labels = s.active().unwrap().history.entry_labels().count();
        let Started::Job(job) = s.start("image.imageSize", json!({"width":80,"resample":"nearest"})).unwrap() else { panic!("native Image Size job") };
        s.active_mut().unwrap().revision += 1;
        assert!(s.wait_job(job).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.entry_labels().count(), labels);
        let mut s = session();
        let original = s.active().unwrap().doc.id;
        let Started::Job(job) = s.start("image.imageSize", json!({"width":80,"resample":"nearest"})).unwrap() else { panic!("native Image Size job") };
        s.execute("file.new", json!({"width":3,"height":2})).unwrap();
        let other = s.active().unwrap().doc.clone();
        assert_eq!(s.wait_job(job).unwrap(), json!({"width":80,"height":40}));
        assert_eq!(s.active().unwrap().doc, other);
        assert_eq!(s.documents().iter().find(|state| state.doc.id == original).unwrap().doc.size, Size::new(80, 40));
    }
    /// Resample on with only a new resolution keeps the print size, as Photoshop's Image Size
    /// does (an action's `imageSize` with `Rslt` alone); without `keepPrintSize` only the
    /// resolution changes.
    #[test]
    fn image_size_keep_print_size_resamples_to_the_resolution() {
        let mut s = session();
        s.execute("image.imageSize", json!({"resolution": 144, "resample": "bicubic"})).unwrap();
        assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(40, 20), 144.0));
        s.execute("image.imageSize", json!({"resolution": 288, "resample": "bicubic", "keepPrintSize": true})).unwrap();
        assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(80, 40), 288.0));
        // An explicit side still wins.
        s.execute("image.imageSize", json!({"width": 20, "resolution": 72, "resample": "bicubic", "keepPrintSize": true})).unwrap();
        assert_eq!(doc(&s).size, Size::new(20, 10));
    }

    #[test]
    fn image_size_scales_everything() {
        for resample in ["bicubic", "bicubicSmoother", "bicubicSharper", "bicubicAutomatic", "automatic", "bilinear", "nearest", "lanczos", "preserveDetails"] {
            let mut s = session();
            s.execute("image.imageSize", json!({"width": 80, "resample": resample})).unwrap();
            let d = doc(&s);
            assert_eq!(d.size, Size::new(80, 40), "{resample}");
            let l = &d.layers[1];
            let b = l.surface().unwrap().content_bounds();
            assert!((b.x0 - 20).abs() <= 1 && (b.x1 - 40).abs() <= 1, "{resample} {b:?}");
            let Effect::DropShadow(sh) = &l.effects.items[0] else { panic!() };
            assert_eq!(sh.distance, 10.0);
            assert_eq!(l.mask.as_ref().unwrap().surface.default_pixel(), vec![1.0]);
        }
    }

    /// The canvas border stays opaque after Image Size (it used to fade into transparency, so a
    /// Background or a 200 % export got a translucent frame), and a full selection stays full.
    /// #1544: 64 x 48 to 300000 x 300000 (90 billion pixels) started allocating resampled bands
    /// without a budget. It is refused before any allocation and the document is untouched; a
    /// change of resolution only, and an ordinary resize, still work.
    #[test]
    fn image_size_refuses_a_raster_past_the_budget() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        let before = (doc(&s).size, s.active().unwrap().history.past_len());
        let err = s.execute("image.imageSize", json!({"width": 300_000, "height": 300_000})).unwrap_err().to_string();
        assert!(err.contains("megapixels"), "{err}");
        assert_eq!((doc(&s).size, s.active().unwrap().history.past_len()), before);
        s.execute("image.imageSize", json!({"width": 300_000, "height": 300_000, "resample": "none", "resolution": 300})).unwrap();
        s.execute("image.imageSize", json!({"width": 128, "height": 96})).unwrap();
        assert_eq!(doc(&s).size, Size::new(128, 96));
    }

    #[test]
    fn image_size_keeps_canvas_edges_opaque() {
        for depth in [8, 16, 32] {
            for (w, resample) in [
                (60, "bicubic"),
                (41, "lanczos"),
                (15, "bilinear"),
                (77, "preserveDetails"),
                (21, "bicubicSharper"),
                (47, "automatic"),
                (9, "bicubicAutomatic"),
            ] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 30, "height": 20, "depth": depth, "background": "#336699"})).unwrap();
                s.execute("select.all", json!({})).unwrap();
                s.execute("image.imageSize", json!({"width": w, "resample": resample})).unwrap();
                let d = doc(&s);
                let flat = photocraft_compose::flatten(d).unwrap();
                let (w, h) = (flat.rect.width() as usize, flat.rect.height() as usize);
                let mid = flat.px[(h / 2) * w + w / 2];
                assert!((mid[3] - 1.0).abs() < 1e-3, "{depth} {resample}: {mid:?}");
                for p in &flat.px {
                    let off = p.iter().zip(mid).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                    assert!(off < 0.01, "{depth} {resample} {w}: {p:?} vs {mid:?}");
                }
                let sel = d.selection.as_ref().unwrap();
                let (x1, y1) = (d.size.width as i32 - 1, d.size.height as i32 - 1);
                for (x, y) in [(0, 0), (x1, 0), (0, y1), (x1, y1)] {
                    assert!(sel.pixel(x, y)[0] > 0.99, "{depth} {resample}: selection at ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn resolution_only_change() {
        let mut s = session();
        let r = s.execute("image.imageSize", json!({"resolution": 300, "resample": "none", "width": 999})).unwrap();
        assert_eq!(doc(&s).size, Size::new(40, 20));
        assert_eq!(doc(&s).resolution_dpi, 300.0);
        // #1815: the result is the size the document has, not the ignored request.
        assert_eq!(r, json!({"width": 40, "height": 20}));
    }

    #[test]
    fn image_size_reports_the_size_it_applied() {
        // #1815: every path reports the document's size afterwards, at every depth.
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
            let size = |s: &Session| json!({"width": doc(s).size.width, "height": doc(s).size.height});
            for p in [
                json!({"width": 999, "height": 777, "resample": "none", "resolution": 150}),
                json!({"width": 32}),
                json!({"height": 96, "resample": "nearest"}),
                json!({"width": 96, "height": 72, "resample": "none"}),
                json!({}),
            ] {
                let r = s.execute("image.imageSize", p.clone()).unwrap();
                assert_eq!(r, size(&s), "{depth}-bit {p}");
            }
            // 64Ã—48 â†’ (none) â†’ 32Ã—24 â†’ 128Ã—96 (aspect kept) â†’ (none) â†’ unchanged.
            assert_eq!(doc(&s).size, Size::new(128, 96), "{depth}-bit");
        }
    }

    #[test]
    fn independent_resolution_axes_preserve_unit_pixels_and_history() {
        let mut s = session();
        let mut original = (*s.active().unwrap().doc).clone();
        original
            .set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 120.0, y_dpi: 240.0, unit: photocraft_doc::PhysicalUnit::Millimeter })
            .unwrap();
        let mut s2 = Session::new();
        s2.add_document(original.clone(), None).unwrap();
        s = s2;
        s.execute("image.imageSize", json!({"resolutionX":300.0,"resample":"none"})).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert_eq!(edited.effective_resolution().x_dpi, 300.0);
        assert_eq!(edited.effective_resolution().y_dpi, 240.0);
        assert_eq!(edited.effective_resolution().unit, photocraft_doc::PhysicalUnit::Millimeter);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.size, original.size);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, original);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute("image.imageSize", json!({"resolution":72.0,"resolutionY":144.0,"resample":"none"})).is_err());
        assert_eq!(*s.active().unwrap().doc, edited);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }

    #[test]
    fn physical_image_size_uses_each_axis_and_rejects_invalid_lengths_atomically() {
        let mut s = session();
        s.execute("image.imageSize", json!({"resolutionX":120.0,"resolutionY":240.0,"resample":"none"})).unwrap();
        let original = (*s.active().unwrap().doc).clone();
        s.execute("image.imageSize", json!({"width":0.1,"height":0.1,"sizeUnit":"in","resample":"nearest"})).unwrap();
        assert_eq!(doc(&s).size, Size::new(12, 24));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, original);
        for params in [
            json!({"width":0,"sizeUnit":"mm"}),
            json!({"width":"2","sizeUnit":"in"}),
            json!({"width":f64::MAX,"sizeUnit":"in"}),
            json!({"width":1,"sizeUnit":"unknown"}),
            json!({"resample":false,"width":12,"sizeUnit":"mm"}),
        ] {
            assert!(s.execute("image.imageSize", params).is_err());
            assert_eq!(*s.active().unwrap().doc, original);
        }
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(doc(&s).size, Size::new(12, 24));
    }

    /// #1114: nearest neighbor reduction keeps source values without averaging.
    #[test]
    fn nearest_neighbor_reduction_keeps_source_values() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                if (x + y) % 2 == 0 {
                    s.execute("select.rect", json!({"x": x, "y": y, "width": 1, "height": 1})).unwrap();
                    s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
                }
            }
        }
        s.execute("select.deselect", json!({})).unwrap();
        let st = s.active().unwrap();
        let before = st.doc.layers[0].surface().unwrap().rgba(0, 0);
        assert!(before[0] < 0.01, "setup: (0,0) should be black, got {before:?}");
        s.execute("image.imageSize", json!({"width": 4, "height": 4, "resample": "nearest"})).unwrap();
        let st = s.active().unwrap();
        let surf = st.doc.layers[0].surface().unwrap();
        for (x, y) in [(0, 0), (1, 1), (2, 3)] {
            let v = surf.rgba(x, y)[0];
            assert!(v.is_finite() && !(0.01..=0.99).contains(&v), "nearest produced an averaged value {v} at ({x},{y})");
        }
    }

    #[test]
    fn canvas_size_anchors_and_extends_background() {
        let mut s = session();
        s.execute("image.canvasSize", json!({"width": 60, "height": 30, "anchor": "topLeft", "extensionColor": "#00ff00"})).unwrap();
        let d = doc(&s);
        assert_eq!(d.size, Size::new(60, 30));
        assert_eq!(d.layers[1].surface().unwrap().content_bounds(), Rect::new(10, 5, 20, 15));
        let bg = d.layers[0].surface().unwrap();
        assert_eq!(bg.pixel(50, 25), vec![0.0, 1.0, 0.0, 1.0]);
        assert_eq!(bg.pixel(5, 5), vec![1.0, 1.0, 1.0, 1.0]);
        let mut s = session();
        s.execute("image.canvasSize", json!({"width": 20, "height": 10, "relative": true, "anchor": "center"})).unwrap();
        assert_eq!(doc(&s).layers[1].surface().unwrap().content_bounds(), Rect::new(20, 10, 30, 20));
    }

    // Draft regressions: intentionally not executed in this implementation wave.
    #[test]
    fn canvas_size_borrowed_envelope_keeps_legacy_null_defaults_and_rejects_stale_targets() {
        let s = session();
        for value in [Value::Null, json!({}), json!({"width":null,"height":null,"relative":null,"anchor":null,"extensionColor":null})] {
            let request = canvas_request(&s, &value).unwrap();
            assert_eq!(request.old, request.new);
            assert_eq!(request.offset, (0, 0));
        }
        let active = s.active().unwrap();
        let before = active.doc.clone();
        for value in [
            json!({"relative":0}),
            json!({"width":"20"}),
            json!({"anchor":"unknown"}),
            json!({"extensionColor":[]}),
            json!({"extensionColor":"#Ã©0000"}),
            json!({"documentId":active.doc.id.0.to_string()}),
            json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":(active.revision+1).to_string()}),
            json!({"documentId":format!("0{}",active.doc.id.0),"expectedRevision":active.revision.to_string()}),
        ] {
            assert!(canvas_size_dispatch_preflight(&s, &value, true).is_err(), "{value}");
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
    }

    #[test]
    fn canvas_size_wide_background_keep_and_outside_are_native_exact() {
        use photocraft_color::PixelFormat;
        for sample in [SampleType::U32, SampleType::F64] {
            let format = PixelFormat { mode: ColorMode::Rgb, sample, alpha: true };
            let mut surface = Surface::new(format);
            let words: Vec<u8> = match sample {
                SampleType::U32 => [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect(),
                _ => [1.0f64.to_bits() + 1, (-0.0f64).to_bits(), 0x7ff8_0000_0000_0123, 1.0f64.to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
            };
            surface.try_write_interleaved_with(Rect::new(0, 0, 1, 1), &words, &photocraft_raster::Interrupt::NONE).unwrap();
            surface.try_write_interleaved_with(Rect::new(8, 0, 9, 1), &words, &photocraft_raster::Interrupt::NONE).unwrap();
            let mut document = Document::new("wide background", Size::new(2, 2), ColorMode::Rgb, sample);
            let mut layer = Layer::new("Background", LayerContent::Raster(surface));
            layer.locks.transparency = true;
            document.layers.push(layer);
            let request = CanvasRequest { old: (2, 2), new: (4, 2), offset: (1, 0), extension: [1.0; 4] };
            let prepared = canvas_prepare(&document, request, &photocraft_raster::Interrupt::NONE).unwrap().unwrap();
            canvas_translate_doc_with_interrupt(&mut document, request.offset, &photocraft_raster::Interrupt::NONE).unwrap();
            document.size = Size::new(4, 2);
            canvas_extend_background(&mut document, prepared, &photocraft_raster::Interrupt::NONE).unwrap();
            let output = document.layers.first().unwrap().surface().unwrap();
            assert_eq!(output.try_read_interleaved_with(Rect::new(1, 0, 2, 1), &photocraft_raster::Interrupt::NONE).unwrap(), words);
            assert_eq!(output.try_read_interleaved_with(Rect::new(9, 0, 10, 1), &photocraft_raster::Interrupt::NONE).unwrap(), words);
            assert_eq!(output.default_bytes(), vec![0; format.bytes_per_pixel()]);
        }
    }

    #[test]
    fn canvas_size_cancelled_preparation_keeps_snapshot_and_real_redo() {
        let mut s = session();
        s.edit("canvas redo sentinel", |document, _| {
            document.name = "redo sentinel".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let request = canvas_request(&s, &json!({"width":80,"height":40,"extensionColor":"transparent"})).unwrap();
        let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(matches!(canvas_prepare(&before, request, &cancelled), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.name, "redo sentinel");
    }

    #[test]
    fn canvas_size_indexed_translation_retains_ordinals_alpha_palette_and_original_identity() {
        use photocraft_doc::{ColorTable, EditableIndexedLayer, EditableIndexedRaster, IndexedSource};
        use std::sync::Arc;
        let bounds = Rect::new(-1, 0, 2, 1);
        let indices = Arc::new(vec![0, 1, 2]);
        let alpha = Arc::new(vec![37, 0, 0]);
        let palette = vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]];
        let raster = EditableIndexedRaster::try_new(bounds, indices.clone(), Some(alpha.clone()), palette.clone()).unwrap();
        let carrier = crate::xcf_editing::expanded(&raster, photocraft_color::PixelFormat::RGBA8, Some(1)).unwrap();
        let original = IndexedSource::new(indices.clone(), Some(alpha.clone()), palette.clone(), bounds, &carrier).unwrap();
        let mut document = Document::new("indexed canvas", Size::new(2, 2), ColorMode::Indexed, SampleType::U8);
        document.color_table = Some(ColorTable { colors: palette.clone(), transparent: Some(1) });
        let mut layer = Layer::new("Ordinals", LayerContent::Raster(carrier));
        layer.indexed_source = Some(original.clone());
        let owner = layer.id;
        document.layers.push(layer);
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(&document).unwrap();
        state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner)).unwrap().raster_bounds = Some(bounds);
        state.editable_indexed.push(EditableIndexedLayer { owner, raster });
        document.xcf_state = Some(state);
        let request = CanvasRequest { old: (2, 2), new: (4, 2), offset: (1, 0), extension: [0.0; 4] };
        assert!(canvas_prepare(&document, request, &photocraft_raster::Interrupt::NONE).unwrap().is_none());
        let mut hostile = document.clone();
        hostile
            .xcf_state
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
            .unwrap()
            .source_flags = u32::MAX;
        assert!(canvas_prepare(&hostile, request, &photocraft_raster::Interrupt::NONE).is_err());
        assert!(Arc::ptr_eq(document.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices(), &indices));
        let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(matches!(canvas_prepare(&document, request, &cancelled), Err(EngineError::Cancelled)));
        canvas_translate_doc_with_interrupt(&mut document, request.offset, &photocraft_raster::Interrupt::NONE).unwrap();
        document.size = Size::new(request.new.0, request.new.1);
        let current = &document.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert_eq!(current.bounds(), Rect::new(0, 0, 3, 1));
        assert!(Arc::ptr_eq(current.indices(), &indices));
        assert!(Arc::ptr_eq(current.alpha().unwrap(), &alpha));
        assert_eq!(current.palette(), palette);
        assert_eq!(document.layers[0].indexed_source.as_ref(), Some(&original));
        assert_eq!(
            document.layers[0].surface().unwrap().try_read_interleaved_with(Rect::new(0, 0, 3, 1), &photocraft_raster::Interrupt::NONE).unwrap(),
            vec![255, 0, 0, 37, 255, 0, 0, 0, 0, 0, 255, 0]
        );
        document.layers[0].name = "Background".into();
        document.layers[0].locks.transparency = true;
        let fill = CanvasRequest { old: request.new, new: (6, 2), offset: (0, 0), extension: [1.0; 4] };
        assert!(prepare_background(&document, fill, &photocraft_raster::Interrupt::NONE).is_err());
    }

    #[test]
    fn canvas_size_anchor_offsets_and_rejects_unlisted() {
        // 64x48 -> 100x80: dx spans 36 and dy spans 32, so each anchor factor lands on a known
        // offset. All nine documented values must anchor exactly where they do today (#2946).
        let expected = [
            ("topLeft", [0, 0]),
            ("top", [18, 0]),
            ("topRight", [36, 0]),
            ("left", [0, 16]),
            ("center", [18, 16]),
            ("right", [36, 16]),
            ("bottomLeft", [0, 32]),
            ("bottom", [18, 32]),
            ("bottomRight", [36, 32]),
        ];
        for (anchor, off) in expected {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
            let r = s.execute("image.canvasSize", json!({"width": 100, "height": 80, "anchor": anchor})).unwrap();
            assert_eq!(r["offset"], json!(off), "anchor {anchor}");
        }
        // An omitted anchor still defaults to center.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
        assert_eq!(s.execute("image.canvasSize", json!({"width": 100, "height": 80})).unwrap()["offset"], json!([18, 16]));
        // A misspelling or unknown value is a BadParams error naming `anchor`, never an anchor
        // between two of the nine (the old independent-axis matching produced top-centre etc.).
        for anchor in ["topleft", "TopLeft", "TOPLEFT", "bottomleft", "bottomright", "middle", "", "NOPE"] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
            let err = s.execute("image.canvasSize", json!({"width": 100, "height": 80, "anchor": anchor})).unwrap_err();
            assert!(matches!(err, EngineError::BadParams { .. }), "anchor {anchor:?} must be rejected, got {err:?}");
            assert!(err.to_string().contains("anchor"), "the error for {anchor:?} must name `anchor`: {err}");
        }
    }

    #[test]
    fn crop_to_selection_and_trim() {
        let mut s = session();
        assert!(s.execute("image.crop", json!({})).is_err(), "no selection, no rect");
        s.execute("select.rect", json!({"x": 5, "y": 2, "width": 20, "height": 10})).unwrap();
        s.execute("image.crop", json!({})).unwrap();
        let d = doc(&s);
        assert_eq!(d.size, Size::new(20, 10));
        assert!(d.selection.is_none());
        assert_eq!(d.layers[1].surface().unwrap().content_bounds(), Rect::new(5, 3, 15, 10));
        // Trim transparent: hide the background, trim to the red square.
        let mut s = session();
        s.execute("layer.setProps", json!({"layer": doc(&s).layers[0].id.0, "visible": false})).unwrap();
        s.execute("image.trim", json!({"basedOn": "transparent"})).unwrap();
        // The red square (10Ã—10) plus its drop shadow (distance 5, size 5).
        let size = doc(&s).size;
        assert!(size.width > 10 && size.width < 30 && size.height > 10 && size.height < 25, "{size:?}");
    }

    #[test]
    fn crop_to_explicit_rect_can_extend_canvas() {
        let mut s = session();
        s.execute("image.crop", json!({"x": 10, "y": 5, "width": 10, "height": 10})).unwrap();
        assert_eq!(doc(&s).size, Size::new(10, 10));
        assert_eq!(doc(&s).layers[1].surface().unwrap().pixel(0, 0), vec![1.0, 0.0, 0.0, 1.0]);
        let mut s = session();
        s.execute("image.crop", json!({"x": -10, "y": 0, "width": 60, "height": 20})).unwrap();
        assert_eq!(doc(&s).size, Size::new(60, 20));
        assert_eq!(doc(&s).layers[1].surface().unwrap().pixel(20, 5), vec![1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn crop_extension_fills_the_background_with_the_background_colour() {
        let mut shrink = Session::new();
        shrink.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
        shrink.execute("image.crop", json!({"x": 2, "y": 0, "width": 12, "height": 16})).unwrap();
        let layer = &doc(&shrink).layers[0];
        assert_eq!(layer.name, "Background", "a shrink-only crop keeps the Background");
        assert!(layer.locks.transparency && layer.locks.position);

        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
        s.tools.background = [0.0, 1.0, 0.0, 1.0];
        s.execute("image.crop", json!({"x": -4, "y": 0, "width": 20, "height": 16})).unwrap();
        let layer = &doc(&s).layers[0];
        assert_eq!(layer.name, "Background", "an extending crop keeps the Background (#3081)");
        assert!(layer.locks.transparency && layer.locks.position);
        assert_eq!(layer.surface().unwrap().rgba(1, 8), [0.0, 1.0, 0.0, 1.0], "the added border takes the background colour");
        assert_eq!(layer.surface().unwrap().rgba(8, 8), [1.0, 0.0, 0.0, 1.0], "the original pixels stay");

        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let layer = &doc(&s).layers[0];
        assert_eq!(layer.surface().unwrap().rgba(1, 8), [0.0, 0.0, 1.0, 1.0], "Fill paints the extended border");
        assert_eq!(layer.surface().unwrap().rgba(8, 8), [0.0, 0.0, 1.0, 1.0]);

        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let layer = &doc(&s).layers[0];
        assert_eq!(doc(&s).size, Size::new(16, 16));
        assert_eq!(layer.surface().unwrap().rgba(8, 8), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(0, 0, 16, 16), "undo removes the filled border");

        // A normal layer gets transparent pixels in the added area.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        s.execute("image.crop", json!({"x": -4, "y": 0, "width": 20, "height": 16})).unwrap();
        assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(1, 8), [0.0; 4]);
    }

    /// A document (`mode`, `depth`) with a white Background and a layer holding a red pixel at
    /// (12, 8) and a blue one at (30, 3).
    fn marked(mode: &str, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "mode": mode, "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        for (x, y, c) in [(12, 8, "#ff0000"), (30, 3, "#0000ff")] {
            s.execute("select.rect", json!({"x": x, "y": y, "width": 1, "height": 1})).unwrap();
            s.execute("edit.fill", json!({"color": c})).unwrap();
        }
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    fn layer_rgba(s: &Session, layer: usize, x: i32, y: i32) -> [f32; 4] {
        doc(s).layers[layer].surface().unwrap().rgba(x, y)
    }

    /// #1792: a frame turned 90Â° clockwise crops what Image Rotation â€º Arbitrary 90Â° CCW and a
    /// plain crop give, in one undo step that restores the document, at 8, 16 and 32 bits.
    #[test]
    fn crop_at_90_degrees_is_rotate_then_crop_in_one_step() {
        for depth in [8, 16, 32] {
            let mut s = marked("rgb", depth);
            let before = doc(&s).clone();
            let past = s.active().unwrap().history.past_len();
            // The 10 Ã— 20 frame at (10, 0), turned 90Â° about (15, 10), covers x 5..25, y 5..15.
            let r = s.execute("image.crop", json!({"x": 10, "y": 0, "width": 10, "height": 20, "angle": 90})).unwrap();
            assert_eq!(r["angle"], json!(90.0), "{depth}");
            assert_eq!(doc(&s).size, Size::new(10, 20), "{depth}");
            assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{depth}: one undo step");
            // (12, 8) is 2.5 left of and 1.5 above the centre: in the upright frame, 1.5 left of
            // and 2.5 below its centre (5, 10).
            let red = layer_rgba(&s, 1, 3, 12);
            assert!(red[0] > 0.9 && red[1] < 0.1 && red[3] > 0.9, "{depth}: {red:?}");
            let mut reference = marked("rgb", depth);
            reference.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "ccw"})).unwrap();
            reference.execute("image.crop", json!({"x": 5, "y": 15, "width": 10, "height": 20})).unwrap();
            for l in 0..2 {
                for y in 0..20 {
                    for x in 0..10 {
                        let (a, b) = (layer_rgba(&s, l, x, y), layer_rgba(&reference, l, x, y));
                        assert!(a.iter().zip(b).all(|(p, q)| (p - q).abs() < 1e-3), "{depth} layer {l} ({x}, {y}): {a:?} vs {b:?}");
                    }
                }
            }
            s.execute("edit.undo", json!({})).unwrap();
            let d = doc(&s);
            assert_eq!(d.size, before.size, "{depth}");
            assert_eq!(d.depth, before.depth);
            for y in 0..20 {
                for x in 0..40 {
                    assert_eq!(layer_rgba(&s, 1, x, y), before.layers[1].surface().unwrap().rgba(x, y), "{depth} undo at ({x}, {y})");
                }
            }
        }
    }

    /// A slightly turned frame keeps its own size, the pixel under its centre and the Background's
    /// colour; the turn's sign matters (a counter-clockwise frame is a negative angle).
    #[test]
    fn crop_at_a_small_angle_keeps_the_frame_size_in_every_mode() {
        for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("grayscale", 16), ("cmyk", 8), ("lab", 16)] {
            for angle in [5.0, -7.5, 365.0] {
                let mut s = marked(mode, depth);
                s.execute("image.crop", json!({"x": 2, "y": 1, "width": 21, "height": 15, "angle": angle})).unwrap();
                let d = doc(&s);
                assert_eq!(d.size, Size::new(21, 15), "{mode} {depth} {angle}");
                assert_eq!(d.depth.bytes() * 8, depth as usize, "{mode} {depth}");
                assert_eq!(format!("{:?}", d.mode).to_lowercase(), mode, "{mode} {depth}");
                assert_eq!(d.layers.len(), 2);
                // The frame's centre (12.5, 8.5) is the red pixel (the layer's only paint nearby);
                // it stays at the centre, its colour kept.
                let want = layer_rgba(&marked(mode, depth), 1, 12, 8);
                let red = layer_rgba(&s, 1, 10, 7);
                assert!(red[3] > 0.5 && (0..3).all(|c| (red[c] - want[c]).abs() < 0.2), "{mode} {depth} {angle}: {red:?} vs {want:?}");
                // The Background stays opaque white inside the canvas it came from.
                let bg = layer_rgba(&s, 0, 10, 7);
                assert!(bg.iter().all(|v| (v - 1.0).abs() < 0.02), "{mode} {depth} {angle}: {bg:?}");
            }
        }
    }

    /// Angle 0 (or a whole turn) is the plain crop: no resampling, the same document.
    #[test]
    fn crop_at_angle_zero_is_the_plain_crop() {
        let mut plain = marked("rgb", 16);
        plain.execute("image.crop", json!({"x": 3, "y": 2, "width": 30, "height": 10})).unwrap();
        for angle in [json!(0), json!(0.0), json!(360), json!(-720), json!(null)] {
            let mut s = marked("rgb", 16);
            let r = s.execute("image.crop", json!({"x": 3, "y": 2, "width": 30, "height": 10, "angle": angle})).unwrap();
            assert!(r.get("angle").is_none(), "{angle}: {r}");
            assert_eq!(doc(&s).size, Size::new(30, 10));
            for l in 0..2 {
                for y in 0..10 {
                    for x in 0..30 {
                        assert_eq!(layer_rgba(&s, l, x, y), layer_rgba(&plain, l, x, y), "{angle} layer {l} ({x}, {y})");
                    }
                }
            }
        }
    }

    /// Bad angles fail cleanly, before anything changes.
    #[test]
    fn crop_angle_rejects_bad_values_without_changing_the_document() {
        let bad = [
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": "NaN"}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": "12"}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": [1]}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": true}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": 1e308}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": -3601}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": f64::MAX}),
            // An angle needs the frame: a selection crop has none.
            json!({"angle": 10}),
            json!({"x": 0, "y": 0, "width": 0, "height": 10, "angle": 10}),
            json!({"x": 0, "y": 0, "width": 300_001, "height": 10, "angle": 10}),
            json!({"x": i32::MAX - 5, "y": 0, "width": 10, "height": 10, "angle": 10}),
        ];
        for p in bad {
            let mut s = marked("rgb", 8);
            s.execute("select.all", json!({})).unwrap();
            let before = doc(&s).clone();
            let past = s.active().unwrap().history.past_len();
            assert!(s.execute("image.crop", p.clone()).is_err(), "{p}");
            assert_eq!(doc(&s).size, before.size, "{p}");
            assert_eq!(s.active().unwrap().history.past_len(), past, "{p}");
        }
        // Far-off but representable frames turn and crop without panicking.
        for (x, y) in [(-1_000_000, 0), (0, 1_000_000), (i32::MIN / 4, i32::MAX / 4)] {
            let mut s = marked("rgb", 8);
            s.execute("image.crop", json!({"x": x, "y": y, "width": 4, "height": 3, "angle": 33.3})).unwrap();
            assert_eq!(doc(&s).size, Size::new(4, 3));
        }
    }

    #[test]
    fn crop_refuses_a_canvas_side_past_the_new_document_limit() {
        // #960: an unbounded explicit crop left a canvas that a later Trim or Duplicate Merged
        // could only abort on allocating.
        for (w, h) in [(100_000_000, 100_000_000), (300_001, 10), (10, 300_001)] {
            let mut s = session();
            let (size, past) = (doc(&s).size, s.active().unwrap().history.past_len());
            let err = s.execute("image.crop", json!({"x": 0, "y": 0, "width": w, "height": h})).unwrap_err();
            assert!(err.to_string().contains("300000 px limit"), "{w} x {h}: {err}");
            assert_eq!((doc(&s).size, s.active().unwrap().history.past_len()), (size, past), "{w} x {h}: nothing changed");
        }
        // At the limit it still extends the canvas (the tiles stay lazy).
        let mut s = session();
        s.execute("image.crop", json!({"x": 0, "y": 0, "width": 300_000, "height": 2})).unwrap();
        assert_eq!(doc(&s).size, Size::new(300_000, 2));
    }

    /// #2443: the Crop tool's W x H x Resolution crops, resamples the cropped area to exactly the
    /// target size and sets the resolution, in one undo step, at 8 and 16 bits.
    #[test]
    fn crop_to_a_target_size_and_resolution_is_one_step() {
        for depth in [8, 16] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72, "depth": depth})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
            s.edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(100, 100, 200, 225), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
            let past = s.active().unwrap().history.past_len();
            // 4 x 5 in at 300 ppi from a 100 x 125 px frame.
            let r = s
                .execute("image.crop", json!({"x": 100, "y": 100, "width": 100, "height": 125, "targetWidth": 1200, "targetHeight": 1500, "resolution": 300}))
                .unwrap();
            assert_eq!((r["documentWidth"].as_u64(), r["documentHeight"].as_u64()), (Some(1200), Some(1500)), "{depth}: {r}");
            assert_eq!(r["resolution"].as_f64(), Some(300.0), "{depth}");
            let d = doc(&s);
            assert_eq!(d.size, Size::new(1200, 1500), "{depth}");
            assert_eq!(d.resolution_dpi, 300.0, "{depth}");
            assert_eq!(d.layers[0].surface().unwrap().format().sample, if depth == 8 { SampleType::U8 } else { SampleType::U16 });
            // The red frame filled the crop; it fills the result.
            let red = d.layers[1].surface().unwrap();
            for (x, y) in [(0, 0), (600, 750), (1199, 1499)] {
                assert_eq!(red.pixel(x, y), vec![1.0, 0.0, 0.0, 1.0], "{depth}: ({x}, {y})");
            }
            assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{depth}: one history step");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(400, 300), 72.0), "{depth}: undo restores the original");
        }
    }

    #[test]
    fn crop_target_resamples_after_a_turned_crop_and_resolution_alone_keeps_the_pixels() {
        let mut s = session();
        let past = s.active().unwrap().history.past_len();
        s.execute("image.crop", json!({"x": 10, "y": 0, "width": 10, "height": 20, "angle": 90, "targetWidth": 30, "targetHeight": 60, "resample": "nearest"}))
            .unwrap();
        assert_eq!(doc(&s).size, Size::new(30, 60));
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "rotate, crop and resample: one step");
        let mut s = session();
        s.execute("image.crop", json!({"x": 2, "y": 3, "width": 30, "height": 10, "resolution": 150})).unwrap();
        assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(30, 10), 150.0));
    }

    #[test]
    fn crop_target_rejects_bad_params_without_changing_the_document() {
        let rect = json!({"x": 0, "y": 0, "width": 10, "height": 10});
        let bad_params = [
            json!({"targetWidth": 100}),
            json!({"targetHeight": 100}),
            json!({"targetWidth": 0, "targetHeight": 10}),
            json!({"targetWidth": -5, "targetHeight": 10}),
            json!({"targetWidth": 10.5, "targetHeight": 10}),
            json!({"targetWidth": "10", "targetHeight": 10}),
            json!({"targetWidth": 300_001, "targetHeight": 10}),
            json!({"targetWidth": 4_294_967_306_u64, "targetHeight": 10}),
            json!({"targetWidth": 300_000, "targetHeight": 300_000}),
            json!({"targetWidth": 10, "targetHeight": 10, "resample": "none"}),
            json!({"targetWidth": 10, "targetHeight": 10, "resample": 3}),
            json!({"resolution": 0}),
            json!({"resolution": -72}),
            json!({"resolution": 1e9}),
            json!({"resolution": "300"}),
        ];
        for extra in bad_params {
            let mut s = session();
            let (size, dpi, past) = (doc(&s).size, doc(&s).resolution_dpi, s.active().unwrap().history.past_len());
            let mut p = rect.clone();
            if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            assert!(s.execute("image.crop", p.clone()).is_err(), "{p}");
            assert_eq!((doc(&s).size, doc(&s).resolution_dpi, s.active().unwrap().history.past_len()), (size, dpi, past), "{p}: nothing changed");
        }
    }

    #[test]
    fn trim_based_on_top_left_colour() {
        let mut s = session();
        s.edit("no fx", |doc, _| {
            doc.layers[1].effects.items.clear();
            Ok(())
        })
        .unwrap();
        s.execute("image.trim", json!({"basedOn": "topLeft"})).unwrap();
        assert_eq!(doc(&s).size, Size::new(10, 10));
    }

    #[test]
    fn thirty_two_bit_holds_linear_light() {
        // Photoshop 27.11: an 8-bit sRGB grey 128 becomes 0.2159 in 32 bits (and 32 â†’ 0.0144),
        // and comes back unchanged; alpha keeps its value.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 1, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 2, 1), &[128.0 / 255.0, 32.0 / 255.0, 1.0, 0.5]);
            Ok(())
        })
        .unwrap();
        s.execute("image.mode.bits32", json!({})).unwrap();
        let px = doc(&s).layers[1].surface().unwrap().pixel(0, 0);
        for (got, want) in px.iter().zip([0.2159, 0.0144, 1.0, 128.0 / 255.0]) {
            assert!((got - want).abs() < 2e-4, "{px:?}");
        }
        s.execute("image.mode.bits8", json!({})).unwrap();
        let px = doc(&s).layers[1].surface().unwrap().pixel(0, 0);
        for (got, want) in px.iter().zip([128.0 / 255.0, 32.0 / 255.0, 1.0, 128.0 / 255.0]) {
            assert!((got - want).abs() < 0.6 / 255.0, "{px:?}");
        }
    }

    #[test]
    fn grayscale_conversion_matches_photoshop() {
        // Photoshop 27.11, default Color Settings (working gray Dot Gain 20 %): Image > Mode >
        // Grayscale turns sRGB 64 / 128 / 192 grey and pure red into 46 / 105 / 177 / 107.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 1, "background": "white"})).unwrap();
        s.edit("paint", |doc, _| {
            let l = doc.layers.first_mut().unwrap();
            let surf = l.surface_mut().unwrap();
            for (x, c) in [[64.0, 64.0, 64.0], [128.0, 128.0, 128.0], [192.0, 192.0, 192.0], [255.0, 0.0, 0.0]].iter().enumerate() {
                surf.fill_rect(Rect::new(x as i32, 0, x as i32 + 1, 1), &[c[0] / 255.0, c[1] / 255.0, c[2] / 255.0, 1.0]);
            }
            Ok(())
        })
        .unwrap();
        s.execute("image.mode.grayscale", json!({})).unwrap();
        let surf = doc(&s).layers[0].surface().unwrap();
        for (x, want) in [46.0, 105.0, 177.0, 107.0].iter().enumerate() {
            let got = surf.pixel(x as i32, 0)[0] * 255.0;
            assert!((got - want).abs() <= 1.5, "pixel {x}: {got} vs {want}");
        }
    }

    #[test]
    fn mode_and_depth_conversions() {
        let mut s = session();
        s.execute("image.mode.grayscale", json!({})).unwrap();
        assert_eq!(doc(&s).mode, ColorMode::Grayscale);
        assert_eq!(doc(&s).layers[1].surface().unwrap().format().mode, ColorMode::Grayscale);
        s.execute("image.mode.bits16", json!({})).unwrap();
        assert_eq!(doc(&s).depth, SampleType::U16);
        assert_eq!(doc(&s).layers[1].surface().unwrap().format().sample, SampleType::U16);
        assert_eq!(doc(&s).layers[1].mask.as_ref().unwrap().surface.format().sample, SampleType::U16);
        s.execute("image.mode.cmyk", json!({})).unwrap();
        s.execute("image.mode.bits32", json!({})).unwrap();
        s.execute("image.mode.lab", json!({})).unwrap();
        s.execute("image.mode.rgb", json!({})).unwrap();
        s.execute("image.mode.bits8", json!({})).unwrap();
        let d = doc(&s);
        assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U8));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(doc(&s).depth, SampleType::F32);
        let revision = s.active().unwrap().revision;
        s.execute("image.mode.bits32", json!({})).unwrap();
        assert_eq!(s.active().unwrap().revision, revision, "an unchanged bit depth must preserve the history branch");
        assert!(s.redo(), "selecting the current depth must not discard the undone conversion");
        assert_eq!(doc(&s).depth, SampleType::U8);
    }

    #[test]
    fn duplicate_document() {
        let mut s = session();
        let r = s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
        assert_eq!(r["document"], json!(1));
        assert_eq!(s.documents().len(), 2);
        assert_eq!(doc(&s).layers.len(), 1);
        assert!(doc(&s).name.ends_with("copy"));
    }
}
