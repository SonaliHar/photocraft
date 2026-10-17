//! Controlled ordinary arbitrary rotation. No indexed/source/wide admission.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::{
    transform::{Homography, Interp},
    wide_geometry::GeometryLimits,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample};
use photocraft_doc::{Document, Layer, LayerContent, Size};
use photocraft_geom::{Affine, Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
use std::sync::Arc;
const CMD: &str = "image.rotation.arbitrary";
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.to_string() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn failure(error: impl std::fmt::Display, control: &Interrupt<'_>) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error) }
}
#[derive(Clone, Copy)]
struct Parameters {
    degrees: f64,
    interpolation: Interp,
}
fn exact(value: &Value, nonzero: bool) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("captured target requires canonical decimal strings"))?;
    let number = text.parse::<u64>().map_err(|_| bad("captured target exceeds range"))?;
    if nonzero && number == 0 {
        return Err(bad("documentId must be nonzero"));
    }
    Ok(number)
}
fn parameters(value: &Value) -> Result<Parameters> {
    if !value.is_null()
        && !value.as_object().is_some_and(|p| {
            p.len() <= 5 && p.keys().all(|k| matches!(k.as_str(), "angle" | "direction" | "interpolation" | "documentId" | "expectedRevision"))
        })
    {
        return Err(bad("expected angle, direction, interpolation and optional captured target"));
    }
    let angle = match value.get("angle") {
        None => 0.0,
        Some(value) => value.as_f64().filter(|n| n.is_finite() && n.abs() <= 3601.0).ok_or_else(|| bad("angle out of range"))?,
    };
    // Existing command first narrows through num(...)->f32, then widens to f64.
    let angle = f64::from(angle as f32);
    if !angle.is_finite() || angle.abs() > 3600.0 {
        return Err(bad("angle out of range"));
    }
    let direction = match value.get("direction") {
        None => "cw",
        Some(value) => value.as_str().ok_or_else(|| bad("direction must be cw or ccw"))?,
    };
    let degrees = match direction {
        "cw" => angle,
        "ccw" => -angle,
        _ => return Err(bad("direction must be cw or ccw")),
    };
    let interpolation = match value.get("interpolation").map(Value::as_str) {
        None | Some(Some("bicubic")) => Interp::Bicubic,
        Some(Some("nearest" | "nearestNeighbor")) => Interp::Nearest,
        Some(Some("bilinear")) => Interp::Bilinear,
        _ => return Err(bad("interpolation must be nearest, nearestNeighbor, bilinear or bicubic")),
    };
    Ok(Parameters { degrees, interpolation })
}
fn context(session: &Session, value: &Value, allow_legacy_normalization: bool) -> Result<()> {
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (value.get("documentId"), value.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if exact(id, true)? == state.doc.id.0 && exact(revision, false)? == state.revision => {}
        _ => return Err(bad("captured document/revision changed or incomplete")),
    }
    let ordinary = state.doc.xcf_state.is_none()
        && !matches!(state.doc.mode, ColorMode::Indexed | ColorMode::Bitmap | ColorMode::Duotone)
        && matches!(state.doc.depth, SampleType::U8 | SampleType::U16 | SampleType::F32);
    if state.revision == u64::MAX
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || (state.floating.is_some() && (!allow_legacy_normalization || value.get("documentId").is_some() || !ordinary))
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
    {
        return Err(bad("finish pixel-move previews, coalescing and pending work before rotating"));
    }
    if state.doc.accounted().is_some() {
        return Err(bad("accounted arbitrary rotation is unsupported"));
    }
    Ok(())
}
/// Borrowed admission before the dispatcher normalizes previews or clones params.
pub(super) fn dispatch_preflight(session: &Session, value: &Value, _background: bool) -> Result<()> {
    parameters(value)?;
    context(session, value, true)
}
fn mapping(old: Size, degrees: f64) -> Result<(Size, Affine, Homography)> {
    if old.width == 0 || old.height == 0 || old.width > 262144 || old.height > 262144 {
        return Err(bad("canvas extent exceeds rotation limits"));
    }
    let new = super::rotated_size(old, degrees);
    if new.width == 0 || new.height == 0 || new.width > 262144 || new.height > 262144 {
        return Err(bad("rotated canvas extent exceeds limits"));
    }
    let (sn, cs) = degrees.to_radians().sin_cos();
    let (cx, cy) = (f64::from(old.width) / 2.0, f64::from(old.height) / 2.0);
    let (nx, ny) = (f64::from(new.width) / 2.0, f64::from(new.height) / 2.0);
    let affine = Affine { m: [cs, sn, -sn, cs, nx - (cs * cx - sn * cy), ny - (sn * cx + cs * cy)] };
    if affine.m.iter().any(|n| !n.is_finite()) {
        return Err(bad("nonfinite rotation matrix"));
    }
    let homography = Homography([affine.m[0], affine.m[2], affine.m[4], affine.m[1], affine.m[3], affine.m[5], 0.0, 0.0, 1.0]);
    Ok((new, affine, homography))
}
fn visit(layers: &[Layer], depth: usize, count: &mut usize, control: &Interrupt<'_>, f: &mut dyn FnMut(&Surface, bool) -> Result<()>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("rotation hierarchy depth limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad("rotation item count limit"))?;
        // A whole-image coordinate transform preserves nested layer locks.
        // Per-layer Free Transform still checks those locks independently.
        if layer.indexed_source.is_some() || layer.video.is_some() {
            return Err(bad("original indexed/video geometry is not qualified"));
        }
        if let Some(mask) = &layer.mask {
            f(&mask.surface, true)?;
        }
        if let Some(cache) = &layer.fill_cache {
            f(&cache.surface, false)?;
        }
        match &layer.content {
            LayerContent::Group(group) => visit(&group.children, depth + 1, count, control, f)?,
            LayerContent::BackedRaster(_) => return Err(bad("backed rotation is unsupported")),
            LayerContent::Raster(surface) => f(surface, false)?,
            LayerContent::Text(text) => {
                if let Some(surface) = &text.cache {
                    f(surface, false)?;
                }
            }
            LayerContent::Shape(shape) => {
                if let Some(surface) = &shape.cache {
                    f(surface, false)?;
                }
            }
            LayerContent::Smart(smart) => {
                if matches!(smart.source, photocraft_doc::SmartSource::Linked { .. }) {
                    return Err(bad("linked smart-object rotation requires an owned source"));
                }
                if let Some(surface) = &smart.cache {
                    f(surface, false)?;
                }
                if let Some(mask) = &smart.filter_mask {
                    f(&mask.surface, true)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn tiles(rect: Rect) -> Result<u64> {
    if rect.is_empty() {
        return Ok(0);
    }
    let side = i64::from(TILE_SIZE);
    let w = (i64::from(rect.x1) - 1).div_euclid(side) - i64::from(rect.x0).div_euclid(side) + 1;
    let h = (i64::from(rect.y1) - 1).div_euclid(side) - i64::from(rect.y0).div_euclid(side) + 1;
    u64::try_from(w).ok().and_then(|w| w.checked_mul(u64::try_from(h).ok()?)).ok_or_else(|| bad("rotation tile count overflow"))
}
fn supported(surface: &Surface, gray: bool) -> Result<()> {
    let format = surface.format();
    if !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
        || format.channels() == 0
        || format.channels() > 5
        || format.bytes_per_pixel() > 40
    {
        return Err(bad("arbitrary rotation requires ordinary U8/U16/F32 samples"));
    }
    if gray && (format.mode != ColorMode::Grayscale || format.channels() != 1) {
        return Err(bad("mask/channel must be ordinary single-channel Gray"));
    }
    Ok(())
}
fn prepare(snapshot: &DocumentSnapshot, parameters: Parameters, control: &Interrupt<'_>) -> Result<(Size, Affine, Homography)> {
    poll(control)?;
    snapshot.validate_resident()?;
    if matches!(snapshot.mode, ColorMode::Indexed | ColorMode::Bitmap | ColorMode::Duotone)
        || matches!(snapshot.depth, SampleType::U32 | SampleType::F64)
        || snapshot.xcf_state.is_some()
        || snapshot.timeline.is_some()
    {
        return Err(bad("indexed, wide or retained XCF arbitrary geometry is not qualified"));
    }
    if snapshot.channels.len() > 100000 {
        return Err(bad("channel count limit"));
    }
    let (new, affine, h) = mapping(snapshot.size, parameters.degrees)?;
    let canvas = Rect::new(0, 0, i32::try_from(new.width).map_err(bad)?, i32::try_from(new.height).map_err(bad)?);
    photocraft_raster::checked_region_byte_len(canvas, snapshot.pixel_format()).map_err(bad)?;
    for layer in &snapshot.layers {
        poll(control)?;
        if layer.name == "Background"
            && layer.locks.position
            && let LayerContent::Raster(surface) = &layer.content
            && (surface.format() != snapshot.pixel_format() || !surface.format().alpha)
        {
            return Err(bad("Background format must match the ordinary document carrier"));
        }
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control).map_err(|e| failure(e, control))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control).map_err(|e| failure(e, control))? as u64;
    let footprint = snapshot.resident_raster_footprint_with(control)?;
    let mut peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(4)?))
        .ok_or_else(|| bad("rotation model headroom overflow"))?;
    let mut work = 0u64;
    let tile_pixels = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).ok_or_else(|| bad("tile count overflow"))?;
    let mut charge = |surface: &Surface, gray: bool| -> Result<()> {
        poll(control)?;
        supported(surface, gray)?;
        let bounds = surface.tile_bounds();
        let output = if bounds.is_empty() {
            Rect::EMPTY
        } else {
            let b = photocraft_algo::transform::try_warp_bounds(bounds, &h).map_err(bad)?;
            Rect::new(
                b.x0.checked_sub(1).ok_or_else(|| bad("rotation support overflow"))?,
                b.y0.checked_sub(1).ok_or_else(|| bad("rotation support overflow"))?,
                b.x1.checked_add(1).ok_or_else(|| bad("rotation support overflow"))?,
                b.y1.checked_add(1).ok_or_else(|| bad("rotation support overflow"))?,
            )
        };
        let mut format = surface.format();
        format.alpha = true;
        photocraft_raster::checked_region_byte_len(output, format).map_err(bad)?;
        let output_tiles = tiles(output)?;
        let input_tiles = surface.tile_count() as u64;
        let bytes = tile_pixels.checked_mul(format.bytes_per_pixel() as u64).ok_or_else(|| bad("rotation output charge overflow"))?;
        let addition = output_tiles
            .checked_mul(bytes.checked_add(1024).ok_or_else(|| bad("rotation map charge overflow"))?)
            .and_then(|n| n.checked_mul(if gray { 4 } else { 3 }))
            .and_then(|n| n.checked_add(input_tiles.checked_mul(bytes.checked_add(1024)?)?.checked_mul(3)?))
            .and_then(|n| n.checked_add(bytes.checked_mul(8)?))
            .ok_or_else(|| bad("rotation staging charge overflow"))?;
        peak = peak.checked_add(addition).filter(|n| *n <= MAX_CHECKED_RASTER_BYTES).ok_or_else(|| bad("aggregate rotation resident limit"))?;
        work = work
            .checked_add(
                input_tiles
                    .checked_add(output_tiles)
                    .and_then(|n| n.checked_mul(tile_pixels))
                    .and_then(|n| n.checked_mul(128))
                    .ok_or_else(|| bad("rotation work overflow"))?,
            )
            .filter(|n| *n <= 8_000_000_000)
            .ok_or_else(|| bad("aggregate rotation work limit"))?;
        Ok(())
    };
    visit(&snapshot.layers, 0, &mut 0, control, &mut charge)?;
    for channel in snapshot.channels.iter().chain(snapshot.quick_mask.iter()) {
        poll(control)?;
        charge(&channel.surface, true)?;
    }
    if let Some(selection) = &snapshot.selection {
        charge(selection, true)?;
    }
    // Canvas-sized background and regeneration output remains private. Reserve
    // before clone; this is a known allocation ledger, not process RSS evidence.
    let canvas_charge = tiles(canvas)?
        .checked_mul(tile_pixels)
        .and_then(|n| n.checked_mul(snapshot.pixel_format().bytes_per_pixel() as u64))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| bad("background charge overflow"))?;
    peak = peak.checked_add(canvas_charge).filter(|n| *n <= MAX_CHECKED_RASTER_BYTES).ok_or_else(|| bad("background rotation resident limit"))?;
    work = work
        .checked_add(
            u64::from(new.width)
                .checked_mul(u64::from(new.height))
                .and_then(|n| n.checked_mul(snapshot.pixel_format().channels() as u64))
                .and_then(|n| n.checked_mul(8))
                .ok_or_else(|| bad("background rotation work overflow"))?,
        )
        .filter(|n| *n <= 8_000_000_000)
        .ok_or_else(|| bad("background rotation work limit"))?;
    let _admitted = (peak, work);
    crate::canvas_geom::preflight_geometry_with_interrupt(snapshot, control)?;
    // Finite numeric admission follows metadata/work/headroom admission.
    let mut finite = |surface: &Surface, _gray: bool| -> Result<()> { validate_samples(surface, control) };
    visit(&snapshot.layers, 0, &mut 0, control, &mut finite)?;
    for channel in snapshot.channels.iter().chain(snapshot.quick_mask.iter()) {
        finite(&channel.surface, true)?;
    }
    if let Some(selection) = &snapshot.selection {
        finite(selection, true)?;
    }
    poll(control)?;
    Ok((new, affine, h))
}
fn validate_samples(surface: &Surface, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    surface.try_validate_tile_origins_with(control).map_err(|e| failure(e, control))?;
    let sample = surface.format().sample;
    let bps = sample.bytes();
    if surface.default_bytes().len() != surface.format().bytes_per_pixel()
        || surface.default_bytes().chunks_exact(bps).any(|bytes| !read_sample(bytes, sample, 0).is_finite())
    {
        return Err(bad("nonfinite or malformed rotation default"));
    }
    let tile_bytes = (TILE_SIZE as usize)
        .checked_mul(TILE_SIZE as usize)
        .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel()))
        .ok_or_else(|| bad("tile length overflow"))?;
    for (_, tile) in surface.tiles() {
        poll(control)?;
        if tile.bytes().len() != tile_bytes {
            return Err(bad("rotation tile length mismatch"));
        }
        for chunk in tile.bytes().chunks(4096 * bps) {
            poll(control)?;
            if chunk.chunks_exact(bps).any(|bytes| !read_sample(bytes, sample, 0).is_finite()) {
                return Err(bad("nonfinite rotation samples"));
            }
        }
    }
    Ok(())
}
fn pixel(surface: &Surface, x: i32, y: i32, values: &mut [f32; 5]) -> Result<()> {
    let format = surface.format();
    let mut bytes = [0u8; 40];
    let native = bytes.get_mut(..format.bytes_per_pixel()).ok_or_else(|| bad("pixel length"))?;
    surface.try_read_pixel_bytes(x, y, native).map_err(bad)?;
    for (target, sample) in values.iter_mut().zip(native.chunks_exact(format.sample.bytes())) {
        *target = read_sample(sample, format.sample, 0);
    }
    Ok(())
}
fn tiled_write(
    bounds: Rect,
    format: PixelFormat,
    control: &Interrupt<'_>,
    mut sample: impl FnMut(i32, i32) -> Result<[f32; 5]>,
    output: &mut Surface,
) -> Result<()> {
    for coordinate in bounds.tiles() {
        poll(control)?;
        let area = coordinate.rect().intersect(&bounds);
        let length = photocraft_raster::checked_region_byte_len(area, format).map_err(bad)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(bad)?;
        if bytes.capacity() > length.checked_mul(2).ok_or_else(|| bad("rotation tile capacity overflow"))? {
            return Err(bad("rotation tile capacity exceeds admitted scratch"));
        }
        for y in area.y0..area.y1 {
            poll(control)?;
            for x in area.x0..area.x1 {
                let value = sample(x, y)?;
                if value.iter().take(format.channels()).any(|n| !n.is_finite()) {
                    return Err(bad("nonfinite rotation arithmetic"));
                }
                let mut encoded = [0u8; 40];
                let native = encoded.get_mut(..format.bytes_per_pixel()).ok_or_else(|| bad("rotation pixel length"))?;
                photocraft_raster::encode_pixel(&format, value.get(..format.channels()).ok_or_else(|| bad("rotation channel length"))?, native);
                bytes.extend_from_slice(native);
            }
        }
        output.try_write_interleaved_with(area, &bytes, control).map_err(|e| failure(e, control))?;
    }
    poll(control)
}
fn warp(surface: &Surface, h: &Homography, interpolation: Interp, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let bounds = surface.try_content_bounds_with_interrupt(control).map_err(|e| failure(e, control))?;
    photocraft_algo::transform::try_warp_surface_legacy_with_limits(
        surface,
        bounds,
        h,
        interpolation,
        GeometryLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 },
        control,
    )
    .map_err(|e| failure(e, control))
}
fn gray(source: &Surface, h: &Homography, interpolation: Interp, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    supported(source, true)?;
    let format = source.format();
    let default = read_sample(source.default_bytes(), format.sample, 0);
    let bounds = source.try_content_bounds_with_interrupt(control).map_err(|e| failure(e, control))?;
    let mut output = Surface::try_with_default_bytes(format, source.default_bytes()).map_err(bad)?;
    if bounds.is_empty() {
        return Ok(output);
    }
    let with_alpha = PixelFormat::new(format.mode, format.sample, true);
    let mut temporary =
        Surface::try_with_default_bytes(with_alpha, [0u8; 40].get(..with_alpha.bytes_per_pixel()).ok_or_else(|| bad("gray default length"))?).map_err(bad)?;
    tiled_write(
        bounds,
        with_alpha,
        control,
        |x, y| {
            let mut value = [0.0; 5];
            pixel(source, x, y, &mut value)?;
            if let Some(alpha) = value.get_mut(1) {
                *alpha = 1.0;
            }
            Ok(value)
        },
        &mut temporary,
    )?;
    let warped = warp(&temporary, h, interpolation, control)?;
    let target = warped.try_content_bounds_with_interrupt(control).map_err(|e| failure(e, control))?;
    if !target.is_empty() {
        tiled_write(
            target,
            format,
            control,
            |x, y| {
                let mut value = [0.0; 5];
                pixel(&warped, x, y, &mut value)?;
                let color = *value.first().ok_or_else(|| bad("gray sample"))?;
                let alpha = *value.get(1).ok_or_else(|| bad("gray alpha"))?;
                value[0] = color * alpha + default * (1.0 - alpha);
                Ok(value)
            },
            &mut output,
        )?;
    }
    output.try_prune_with(control).map_err(|e| failure(e, control))?;
    poll(control)?;
    Ok(output)
}
fn background(
    surface: &Surface,
    format: PixelFormat,
    canvas: Rect,
    h: &Homography,
    interpolation: Interp,
    color: [f32; 4],
    control: &Interrupt<'_>,
) -> Result<Surface> {
    if surface.format() != format || !format.alpha {
        return Err(bad("Background format must match the ordinary document carrier"));
    }
    let rotated = warp(surface, h, interpolation, control)?;
    let mut output =
        Surface::try_with_default_bytes(format, [0u8; 40].get(..format.bytes_per_pixel()).ok_or_else(|| bad("background default length"))?).map_err(bad)?;
    let mut fill = [0.0; 5];
    photocraft_raster::from_rgba_into(&format, color, &mut fill);
    tiled_write(canvas, format, control, |_, _| Ok(fill), &mut output)?;
    let bounds = rotated.try_content_bounds_with_interrupt(control).map_err(|e| failure(e, control))?;
    let alpha = format.channels().checked_sub(1).ok_or_else(|| bad("Background channels"))?;
    // Exact existing straight-alpha source-over equation and component order.
    let base = output.clone();
    tiled_write(
        bounds,
        format,
        control,
        |x, y| {
            let mut top = [0.0; 5];
            let mut under = [0.0; 5];
            pixel(&rotated, x, y, &mut top)?;
            pixel(&base, x, y, &mut under)?;
            let ta = *top.get(alpha).ok_or_else(|| bad("top alpha"))?;
            let da = *under.get(alpha).ok_or_else(|| bad("base alpha"))?;
            let oa = ta + da * (1.0 - ta);
            if oa > 0.0 {
                for (c, dp) in under.iter_mut().enumerate().take(alpha) {
                    *dp = (*top.get(c).ok_or_else(|| bad("top component"))? * ta + *dp * da * (1.0 - ta)) / oa;
                }
            }
            if let Some(value) = under.get_mut(alpha) {
                *value = oa;
            }
            Ok(under)
        },
        &mut output,
    )?;
    output.try_prune_with(control).map_err(|e| failure(e, control))?;
    poll(control)?;
    Ok(output)
}
// Keep the admitted geometry, native format, background and original control explicit without a new heap-owning context.
#[allow(clippy::too_many_arguments)]
fn move_pixels(
    layers: &mut [Layer],
    depth: usize,
    h: &Homography,
    parameters: Parameters,
    canvas: Rect,
    format: PixelFormat,
    bg: [f32; 4],
    control: &Interrupt<'_>,
) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("rotation hierarchy depth"));
    }
    for layer in layers {
        poll(control)?;
        let is_background = depth == 0 && layer.name == "Background" && layer.locks.position && matches!(layer.content, LayerContent::Raster(_));
        match &mut layer.content {
            LayerContent::Group(group) => move_pixels(&mut group.children, depth + 1, h, parameters, canvas, format, bg, control)?,
            LayerContent::Raster(surface) => {
                *surface = if is_background {
                    background(surface, format, canvas, h, parameters.interpolation, bg, control)?
                } else {
                    warp(surface, h, parameters.interpolation, control)?
                }
            }
            LayerContent::Smart(smart) => {
                if let Some(cache) = &mut smart.cache {
                    *cache = warp(cache, h, parameters.interpolation, control)?;
                }
                if let Some(mask) = &mut smart.filter_mask {
                    mask.surface = Arc::new(gray(&mask.surface, h, parameters.interpolation, control)?);
                }
            }
            _ => {}
        }
        if let Some(mask) = &mut layer.mask
            && (is_background || mask.linked)
        {
            mask.surface = Arc::new(gray(&mask.surface, h, parameters.interpolation, control)?);
        }
        // Old Free Transform metadata pass invalidated fill caches before refresh.
        layer.fill_cache = None;
    }
    poll(control)
}
fn stage(snapshot: &DocumentSnapshot, parameters: Parameters, bg: [f32; 4], control: &Interrupt<'_>) -> Result<Document> {
    if bg.iter().any(|n| !n.is_finite()) {
        return Err(bad("nonfinite Background color"));
    }
    let (new, affine, h) = prepare(snapshot, parameters, control)?;
    poll(control)?;
    let mut doc = (**snapshot).clone();
    doc.size = new;
    let canvas = doc.bounds();
    let format = doc.pixel_format();
    move_pixels(&mut doc.layers, 0, &h, parameters, canvas, format, bg, control)?;
    crate::canvas_geom::transform_geometry_with_interrupt(&mut doc, &affine, control)?;
    for channel in doc.channels.iter_mut().chain(doc.quick_mask.iter_mut()) {
        poll(control)?;
        channel.surface = gray(&channel.surface, &h, parameters.interpolation, control)?;
    }
    if let Some(selection) = &doc.selection {
        let transformed = gray(selection, &h, Interp::Bilinear, control)?;
        doc.selection =
            if transformed.try_content_bounds_with_interrupt(control).map_err(|e| failure(e, control))?.is_empty() { None } else { Some(transformed) };
    }
    crate::canvas_geom::refresh_with_interrupt(&mut doc, crate::canvas_geom::Refresh::All, control)?;
    doc.guides = Default::default();
    poll(control)?;
    Ok(doc)
}
/// Private document preparation for compound controlled callers. No Session or history mutation.
pub(crate) fn rotate_document_with_interrupt(
    snapshot: &DocumentSnapshot,
    degrees: f64,
    interpolation: Interp,
    background: [f32; 4],
    control: &Interrupt<'_>,
) -> Result<Document> {
    let degrees = f64::from(degrees as f32);
    if !degrees.is_finite() || degrees.abs() > 3600.0 {
        return Err(bad("angle out of range"));
    }
    stage(snapshot, Parameters { degrees, interpolation }, background, control)
}
pub(super) fn run(session: &mut Session, value: &Value) -> Result<Value> {
    let parameters = parameters(value)?;
    context(session, value, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if parameters.degrees.rem_euclid(360.0).abs() < 1e-9 {
        return Ok(json!({"width":state.doc.size.width,"height":state.doc.size.height}));
    }
    let baseline = state.doc.clone();
    let weak = baseline.downgrade();
    let revision = state.revision;
    let work = baseline.clone();
    let bg = session.tools.background;
    crate::jobs::run(
        session,
        "Rotate Canvas",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Rotate Canvas", |control| {
                let doc = stage(&work, parameters, bg, control)?;
                Ok((doc, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
                || current.floating.is_some()
                || current.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
            {
                return Err(bad("target changed while preparing rotation"));
            }
            ctx.stage(0.95, 1.0, "Rotate Canvas", |control| {
                poll(control)?;
                let size = document.size;
                let (_, sampler, _) = mapping(baseline.size, parameters.degrees)?;
                session.edit_with_sampler_map("Rotate Canvas", control, Some(sampler), move |current, _| {
                    poll(control)?;
                    *current = document;
                    Ok(())
                })?;
                Ok(json!({"width":size.width,"height":size.height}))
            })
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_active_preview_can_normalize_but_captured_and_run_contexts_refuse() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        session.execute("select.all", Value::Null).unwrap();
        session.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
        assert!(session.active().unwrap().floating.is_some());
        let state = session.active().unwrap();
        let captured = json!({"angle":15,"documentId":state.doc.id.0.to_string(),"expectedRevision":state.revision.to_string()});
        assert!(dispatch_preflight(&session, &json!({"angle":15}), false).is_ok());
        assert!(dispatch_preflight(&session, &captured, false).is_err());
        assert!(context(&session, &json!({"angle":15}), false).is_err());
        assert!(session.active().unwrap().floating.is_some());
        // Keep that genuine preview in its original tab; a different ordinary
        // document is not a source consumer and must not be blocked by it.
        let second = Document::new("Independent target", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        session.add_document(second, None).unwrap();
        assert!(session.set_active(1));
        assert!(session.documents().first().unwrap().floating.is_some());
        assert!(dispatch_preflight(&session, &json!({"angle":15}), false).is_ok());
        assert!(context(&session, &json!({"angle":15}), false).is_ok());
    }
    #[test]
    fn params_are_bounded_before_preview_normalization() {
        for value in [json!({"angle":[]}), json!({"angle":3602}), json!({"direction":false}), json!({"interpolation":"unknown"}), json!({"angle":1,"extra":{}})]
        {
            assert!(parameters(&value).is_err());
        }
        assert_eq!(parameters(&json!({"angle":17.123456789})).unwrap().degrees, f64::from(17.123456789f64 as f32));
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        assert!(dispatch_preflight(&session, &json!({"documentId":"0","expectedRevision":"0"}), false).is_err());
    }
    #[test]
    fn cancellation_and_unsupported_source_preserve_real_redo_dpi_and_original() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":3,"height":2})).unwrap();
        session.execute("image.mode.bits16", Value::Null).unwrap();
        assert!(session.undo());
        let original = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        assert!(matches!(
            stage(&original, parameters(&json!({"angle":15})).unwrap(), [0.0, 0.0, 0.0, 1.0], &Interrupt::cancel_only(&|| true)),
            Err(EngineError::Cancelled)
        ));
        assert!(original.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
        let mut doc = (*original).clone();
        doc.xcf_state = Some(Default::default());
        let source = DocumentSnapshot::from_resident(Arc::new(doc)).unwrap();
        assert!(prepare(&source, parameters(&json!({"angle":15})).unwrap(), &Interrupt::NONE).is_err());
    }
}
