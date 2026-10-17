//! Controlled explicit Bitmap conversion, preserving legacy ordinary f32 math.
//! No wide, indexed, imported-source or explicit current-encoding reinterpretation.
use crate::{EngineError, Result, Session};
use photocraft_algo::quantize::{self, BitmapMethod, ControlledLimits, HalftoneShape};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
const ID: &str = "image.mode.bitmap";
fn error(message: impl std::fmt::Display) -> EngineError {
    super::bad(ID, message.to_string())
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
fn named<'a>(params: &'a Value, key: &str, default: &'a str) -> Result<&'a str> {
    params
        .get(key)
        .map(|value| value.as_str().filter(|value| value.len() <= 32 && !value.contains('\0')).ok_or_else(|| error("Bitmap names must be bounded strings")))
        .transpose()
        .map(|value| value.unwrap_or(default))
}
fn parse(session: &Session, params: &Value) -> Result<BitmapMethod> {
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 6 && object.keys().all(|key| matches!(key.as_str(), "method" | "frequency" | "angle" | "shape" | "documentId" | "expectedRevision"))
        })
    {
        return Err(error("expected bounded Bitmap parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.mode != ColorMode::Grayscale {
        return Err(error("Bitmap needs a Grayscale image"));
    }
    if state.revision == u64::MAX || session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(error("target revision/pending work refused"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| error("captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| error("captured ID exceeds range"))
            };
            let id = exact(id)?;
            if id == 0 || id != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(error("captured target changed"));
            }
            if state.floating.is_some()
                || state.coalesce.is_some()
                || session.has_jobs()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(error("captured Bitmap target has floating or transient work"));
            }
        }
        _ => return Err(error("documentId and expectedRevision must be supplied together")),
    }
    if !matches!(state.doc.depth, SampleType::U8 | SampleType::U16 | SampleType::F32)
        || state.doc.xcf_state.is_some()
        || state.doc.sample_encoding.is_some()
        || state.doc.working_profile_interpretation != photocraft_doc::WorkingProfileInterpretation::Icc
    {
        return Err(error("wide/indexed/current source interpretation is not qualified for Bitmap"));
    }
    let numeric = |key, default: f64| -> Result<f64> {
        params
            .get(key)
            .map(|value| value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| error("Bitmap numeric parameters must be finite")))
            .transpose()
            .map(|value| value.unwrap_or(default))
    };
    let frequency = numeric("frequency", 53.)?;
    let angle = numeric("angle", 45.)?;
    if !(-180.0..=180.0).contains(&angle) {
        return Err(error("Bitmap angle must be between -180 and 180 degrees"));
    }
    let angle = angle as f32;
    let shape = named(params, "shape", "round")?;
    if !matches!(shape, "round" | "ellipse" | "line" | "square" | "diamond" | "cross") {
        return Err(error("unknown halftone shape"));
    }
    match named(params, "method", "diffusion")? {
        "threshold" | "threshold50" => Ok(BitmapMethod::Threshold),
        "pattern" => Ok(BitmapMethod::Pattern),
        "diffusion" => Ok(BitmapMethod::Diffusion),
        "halftone" => {
            let legacy = photocraft_doc::PhysicalResolution::square(f64::from(state.doc.resolution_dpi));
            let resolution = state.doc.physical_resolution.as_ref().unwrap_or(&legacy);
            resolution.validate().map_err(|e| error(format!("invalid physical resolution: {e}")))?;
            if resolution.x_dpi != resolution.y_dpi {
                return Err(error("halftone screens with unequal DPI axes are not supported"));
            }
            // Retain old f32 frequency rounding/clamping before the canonical
            // f64 DPI division. Metadata itself is never narrowed or rewritten.
            let frequency = (frequency as f32).clamp(1., 999.);
            let cell = (resolution.x_dpi / f64::from(frequency)).max(2.);
            if !cell.is_finite() || cell > f64::from(f32::MAX) {
                return Err(error("physical halftone cell exceeds supported precision range"));
            }
            Ok(BitmapMethod::Halftone { cell: cell as f32, angle, shape: HalftoneShape::from_id(shape).ok_or_else(|| error("unknown halftone shape"))? })
        }
        _ => Err(error("method must be threshold, threshold50, pattern, diffusion or halftone")),
    }
}
/// Before dispatcher floating normalization/parameter cloning, using only
/// bounded parameters and scalar metadata. Valid legacy no-pair auto-anchor stays.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    parse(session, params).map(|_| ())
}
fn surface(source: &Surface, tiles: &mut u64, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if !matches!(source.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Err(error("wide samples cannot enter Bitmap f32 composition"));
    }
    source.try_validate_tile_origins_with(control).map_err(|e| controlled(control, e))?;
    *tiles = tiles.checked_add(source.tile_count() as u64).ok_or_else(|| error("source tile count overflow"))?;
    Ok(())
}
fn layers(values: &[Layer], depth: usize, count: &mut u64, tiles: &mut u64, control: &Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("layer hierarchy limit"));
    }
    for layer in values {
        poll(control)?;
        *count = count.checked_add(1).filter(|count| *count <= 100_000).ok_or_else(|| error("layer count limit"))?;
        if layer.import_compositing.is_some()
            || layer.indexed_source.is_some()
            || layer.video.is_some()
            || matches!(&layer.content, LayerContent::BackedRaster(_))
        {
            return Err(error("current imported/backed/video composition is not qualified for Bitmap"));
        }
        if let Some(mask) = &layer.mask {
            surface(&mask.surface, tiles, control)?;
        }
        if let Some(cache) = &layer.fill_cache {
            surface(&cache.surface, tiles, control)?;
        }
        if let Some(cache) = layer.surface() {
            surface(cache, tiles, control)?;
        }
        if let LayerContent::Smart(smart) = &layer.content
            && let Some(mask) = &smart.filter_mask
        {
            surface(&mask.surface, tiles, control)?;
        }
        if let LayerContent::Group(group) = &layer.content {
            layers(&group.children, depth + 1, count, tiles, control)?;
        }
    }
    Ok(())
}
fn prepare(snapshot: &crate::DocumentSnapshot, method: BitmapMethod, control: &Interrupt<'_>) -> Result<(usize, quantize::ControlledRequirements)> {
    poll(control)?;
    snapshot.validate_resident()?;
    if snapshot.size.width == 0 || snapshot.size.height == 0 || snapshot.size.width > 300_000 || snapshot.size.height > 300_000 {
        return Err(error("invalid Bitmap canvas"));
    }
    let width = usize::try_from(snapshot.size.width).map_err(|_| error("canvas width exceeds address range"))?;
    let pixels = width
        .checked_mul(usize::try_from(snapshot.size.height).map_err(|_| error("canvas height exceeds address range"))?)
        .ok_or_else(|| error("canvas sample count overflow"))?;
    let required = quantize::bitmap_requirements(pixels, width, method).map_err(|e| controlled(control, e))?;
    let (mut count, mut tiles) = (0, 0);
    layers(&snapshot.layers, 0, &mut count, &mut tiles, control)?;
    if snapshot.channels.len() > 100_000 {
        return Err(error("channel count limit"));
    }
    for channel in snapshot.channels.iter().chain(snapshot.quick_mask.iter()) {
        surface(&channel.surface, &mut tiles, control)?;
    }
    if let Some(selection) = &snapshot.selection {
        surface(selection, &mut tiles, control)?;
    }
    let native = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, native, control).map_err(|e| controlled(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(snapshot, native, control).map_err(|e| controlled(control, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    let output_tiles = u64::from(snapshot.size.width.div_ceil(TILE_SIZE as u32))
        .checked_mul(u64::from(snapshot.size.height.div_ceil(TILE_SIZE as u32)))
        .ok_or_else(|| error("output tile span overflow"))?;
    let output = output_tiles.checked_mul((TILE_SIZE as u64).pow(2) * 2 + 1024).ok_or_else(|| error("output tile size overflow"))?;
    // Quantizer descriptors may overlap the held gray buffer: conservatively
    // charge all declared fields rather than refunding or inferring ownership.
    let peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(required.retained_bytes))
        .and_then(|n| n.checked_add(required.scratch_bytes))
        .and_then(|n| n.checked_add(required.output_bytes))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(384 << 20))
        .ok_or_else(|| error("Bitmap residency overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(error("aggregate gray/render/source/output Bitmap residency limit"));
    }
    let work = (pixels as u64)
        .checked_mul(count.max(1))
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(required.work_units))
        .and_then(|n| n.checked_add(output_tiles.checked_mul(tiles)?.checked_mul(16)?))
        .and_then(|n| n.checked_add(output_tiles.checked_mul((TILE_SIZE as u64).pow(2) * 16 + 1024)?))
        .ok_or_else(|| error("Bitmap work overflow"))?;
    if work > 8_000_000_000 {
        return Err(error("aggregate Bitmap render/quantize/write work limit"));
    }
    poll(control)?;
    Ok((pixels, required))
}
fn composite_gray(document: &Document, pixels: usize, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    let mut gray = Vec::new();
    gray.try_reserve_exact(pixels).map_err(|_| error("gray buffer allocation failed"))?;
    if gray.capacity() > pixels.saturating_add(1024) {
        return Err(error("gray buffer capacity exceeded"));
    }
    while gray.len() < pixels {
        poll(control)?;
        gray.resize(gray.len().saturating_add(16_384).min(pixels), 0.);
    }
    let width = usize::try_from(document.size.width).map_err(|_| error("canvas width"))?;
    // Only one bounded RGBA tile is held, with the original f32 over-white
    // arithmetic and rgb_to_gray helper, never a whole-canvas RGBA/out buffer.
    for coordinate in document.bounds().tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&document.bounds());
        let buffer = photocraft_compose::render_with_interrupt(document, part, control).map_err(|e| controlled(control, e))?;
        let columns = part.width() as usize;
        if columns == 0 || buffer.px.len() != columns.checked_mul(part.height() as usize).ok_or_else(|| error("render tile length overflow"))? {
            return Err(error("render tile length mismatch"));
        }
        for (index, pixel) in buffer.px.iter().enumerate() {
            if index.is_multiple_of(columns) {
                poll(control)?;
            }
            let [r, g, b, a] = *pixel;
            let value = photocraft_color::convert::rgb_to_gray([r * a + 1. * (1. - a), g * a + 1. * (1. - a), b * a + 1. * (1. - a)]);
            let x = (part.x0 as usize).checked_add(index % columns).ok_or_else(|| error("render column overflow"))?;
            let y = (part.y0 as usize).checked_add(index / columns).ok_or_else(|| error("render row overflow"))?;
            *gray
                .get_mut(y.checked_mul(width).and_then(|n| n.checked_add(x)).ok_or_else(|| error("gray offset overflow"))?)
                .ok_or_else(|| error("gray offset exceeds canvas"))? = value;
        }
    }
    poll(control)?;
    Ok(gray)
}
struct OutputBudget {
    resident: u64,
    work: u64,
}
impl photocraft_raster::staged::StagingBudget for OutputBudget {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), photocraft_raster::RasterInputError> {
        self.resident = self.resident.checked_sub(bytes).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), photocraft_raster::RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn output(document: &Document, gray: &[f32], control: &Interrupt<'_>) -> Result<Surface> {
    use photocraft_raster::staged::{SOURCE_CELL_SIZE, StagedSurfaceBuilder};
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
    let canvas = document.bounds();
    let width = document.size.width as usize;
    let tiles = u64::from(document.size.width.div_ceil(TILE_SIZE as u32))
        .checked_mul(u64::from(document.size.height.div_ceil(TILE_SIZE as u32)))
        .ok_or_else(|| error("output tile span overflow"))?;
    let resident = tiles
        .checked_mul((TILE_SIZE as u64).pow(2) * 2 + 1024)
        .and_then(|n| n.checked_add(4096))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("output staging exceeds bitmap budget"))?;
    let mut budget = OutputBudget { resident, work: 8_000_000_000 };
    // Fixed-cell builder initializes each destination tile once; it does not
    // clone an ever-growing sparse map on every output cell write.
    let mut output = StagedSurfaceBuilder::new(format, &[0, 0], canvas, &mut budget, control).map_err(|e| controlled(control, e))?;
    let mut top = canvas.y0;
    while top < canvas.y1 {
        let bottom = top.checked_add(SOURCE_CELL_SIZE).ok_or_else(|| error("output row overflow"))?.min(canvas.y1);
        let mut left = canvas.x0;
        while left < canvas.x1 {
            poll(control)?;
            let right = left.checked_add(SOURCE_CELL_SIZE).ok_or_else(|| error("output column overflow"))?.min(canvas.x1);
            let part = photocraft_geom::Rect::new(left, top, right, bottom);
            let length = (part.width() as usize)
                .checked_mul(part.height() as usize)
                .and_then(|n| n.checked_mul(2))
                .ok_or_else(|| error("output cell length overflow"))?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(length).map_err(|_| error("output cell allocation failed"))?;
            if bytes.capacity() > length.saturating_add(4096) {
                return Err(error("output cell capacity exceeded"));
            }
            for y in top..bottom {
                poll(control)?;
                for x in left..right {
                    let offset = (y as usize).checked_mul(width).and_then(|n| n.checked_add(x as usize)).ok_or_else(|| error("output gray offset overflow"))?;
                    let value = *gray.get(offset).ok_or_else(|| error("output gray offset exceeds input"))?;
                    let encoded = if value == 0. {
                        0
                    } else if value == 1. {
                        255
                    } else {
                        return Err(error("quantizer did not return binary Bitmap values"));
                    };
                    bytes.extend_from_slice(&[encoded, 255]);
                }
            }
            output.push_region(part, &bytes, &mut budget, control).map_err(|e| controlled(control, e))?;
            left = right;
        }
        top = bottom;
    }
    poll(control)?;
    let (surface, ()) = output.finalize(&mut budget, control).map_err(|e| controlled(control, e))?.into_parts();
    poll(control)?;
    Ok(surface)
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let method = parse(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let work_base = baseline.clone();
    crate::jobs::run(
        session,
        "Bitmap",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Bitmap", |control| {
                let (pixels, required) = prepare(&work_base, method, control)?;
                let mut gray = composite_gray(&work_base, pixels, control)?;
                let limits = ControlledLimits {
                    resident_bytes: required
                        .retained_bytes
                        .checked_add(required.scratch_bytes)
                        .and_then(|n| n.checked_add(required.output_bytes))
                        .ok_or_else(|| error("quantizer limit overflow"))?,
                    work_units: required.work_units,
                };
                quantize::to_bitmap_with_interrupt(&mut gray, work_base.size.width as usize, method, limits, control).map_err(|e| controlled(control, e))?;
                let surface = output(&work_base, &gray, control)?;
                drop(gray);
                poll(control)?;
                let mut document = (*work_base).clone();
                document.mode = ColorMode::Bitmap;
                document.depth = SampleType::U8;
                let mut background = Layer::new("Background", LayerContent::Raster(surface));
                background.locks.transparency = true;
                background.locks.position = true;
                let active = Some(background.id);
                document.layers.clear();
                document.layers.try_reserve(1).map_err(|_| error("background layer allocation failed"))?;
                document.layers.push(background);
                let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
                for channel in &mut document.channels {
                    poll(control)?;
                    channel.surface = channel
                        .surface
                        .try_convert_with(PixelFormat { alpha: channel.surface.format().alpha, ..format }, control)
                        .map_err(|e| controlled(control, e))?;
                }
                document.quick_mask = None;
                document.color_table = None;
                document.duotone = None;
                poll(control)?;
                Ok((document, active, ctx.clone()))
            })
        },
        move |session, (document, active, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id || current.revision != revision || !current.doc.shares_state(&baseline) {
                return Err(error("target changed while preparing Bitmap"));
            }
            ctx.stage(0.95, 1., "Bitmap", |control| {
                poll(control)?;
                session.edit_with_interrupt("Bitmap", control, move |current, selected| {
                    *current = document;
                    *selected = active;
                    Ok(())
                })?;
                Ok(Value::Null)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(depth: u64) -> Session {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":67,"height":3,"mode":"grayscale","depth":depth})).unwrap();
        session
            .edit("fixture", |document, active| {
                let bounds = document.bounds();
                let surface = document.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                let format = surface.format();
                let mut pixels = Vec::new();
                for y in bounds.y0..bounds.y1 {
                    for x in bounds.x0..bounds.x1 {
                        let gray = ((x * 13 + y * 19) % 101) as f32 / 100.;
                        let alpha = if x % 3 == 0 { 0.25 } else { 1. };
                        pixels.extend(photocraft_raster::from_rgba(&format, [gray, gray, gray, alpha]));
                    }
                }
                surface.write_region(bounds, &pixels);
                Ok(())
            })
            .unwrap();
        session
    }

    // Unrun oracle draft: shared controlled math must retain the old complete
    // row-major gray quantizer results, including cells clipped at x=67.
    #[test]
    fn ordinary_depths_match_legacy_all_bitmap_methods() {
        let mut methods = vec![BitmapMethod::Threshold, BitmapMethod::Pattern, BitmapMethod::Diffusion];
        for shape in [HalftoneShape::Round, HalftoneShape::Ellipse, HalftoneShape::Line, HalftoneShape::Square, HalftoneShape::Diamond, HalftoneShape::Cross] {
            methods.push(BitmapMethod::Halftone { cell: 3.25, angle: 27., shape });
        }
        for depth in [8, 16, 32] {
            let session = fixture(depth);
            let document = &session.active().unwrap().doc;
            let source = document.layers[0].surface().unwrap();
            let original = source.try_read_interleaved_with(document.bounds(), &Interrupt::NONE).unwrap();
            for method in &methods {
                let pixels = super::super::composite(document, true).unwrap();
                let mut expected: Vec<f32> = pixels.iter().map(|pixel| photocraft_color::convert::rgb_to_gray([pixel[0], pixel[1], pixel[2]])).collect();
                quantize::to_bitmap(&mut expected, 67, *method);
                let (_, requirements) = prepare(document, *method, &Interrupt::NONE).unwrap();
                let mut actual = composite_gray(document, expected.len(), &Interrupt::NONE).unwrap();
                quantize::to_bitmap_with_interrupt(
                    &mut actual,
                    67,
                    *method,
                    ControlledLimits {
                        resident_bytes: requirements.retained_bytes + requirements.scratch_bytes + requirements.output_bytes,
                        work_units: requirements.work_units,
                    },
                    &Interrupt::NONE,
                )
                .unwrap();
                assert_eq!(actual.iter().map(|value| value.to_bits()).collect::<Vec<_>>(), expected.iter().map(|value| value.to_bits()).collect::<Vec<_>>());
                let raster = output(document, &actual, &Interrupt::NONE).unwrap();
                let native = raster.try_read_interleaved_with(document.bounds(), &Interrupt::NONE).unwrap();
                let expected_native: Vec<u8> = expected.iter().flat_map(|value| [if *value == 0. { 0 } else { 255 }, 255]).collect();
                assert_eq!(native, expected_native);
            }
            assert_eq!(source.try_read_interleaved_with(document.bounds(), &Interrupt::NONE).unwrap(), original);
        }
    }

    #[test]
    fn cancellation_and_invalid_binary_output_do_not_publish() {
        let session = fixture(16);
        let state = session.active().unwrap();
        let snapshot = state.doc.clone();
        let cancel = || true;
        let control = Interrupt::cancel_only(&cancel);
        assert!(matches!(prepare(&snapshot, BitmapMethod::Diffusion, &control), Err(EngineError::Cancelled)));
        assert!(matches!(output(&snapshot, &[0.; 201], &control), Err(EngineError::Cancelled)));
        assert!(output(&snapshot, &[0.25; 201], &Interrupt::NONE).is_err());
        assert!(state.doc.shares_state(&snapshot));
    }

    #[test]
    fn captured_target_and_unequal_halftone_fail_without_history() {
        let mut session = fixture(8);
        session
            .edit("resolution", |document, _| {
                let mut resolution = photocraft_doc::PhysicalResolution::square(144.);
                resolution.y_dpi = 288.;
                document.physical_resolution = Some(resolution);
                Ok(())
            })
            .unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let history = state.history.past_len();
        let id = state.doc.id.0.to_string();
        let revision = state.revision.to_string();
        assert!(dispatch_preflight(&session, &json!({"method":"halftone","documentId":id,"expectedRevision":revision}), false).is_err());
        assert!(dispatch_preflight(&session, &json!({"method":"threshold","documentId":id,"expectedRevision":"0"}), false).is_err());
        assert!(dispatch_preflight(&session, &json!({"method":"threshold","documentId":id}), false).is_err());
        assert!(dispatch_preflight(&session, &json!({"method":"not-a-method"}), false).is_err());
        assert!(dispatch_preflight(&session, &json!({"method":"threshold"}), false).is_ok());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }

    #[test]
    fn bitmap_has_one_undo_and_preserves_canonical_resolution() {
        let mut session = fixture(16);
        session
            .edit("resolution", |document, _| {
                document.physical_resolution = Some(photocraft_doc::PhysicalResolution::square(144.));
                document.resolution_dpi = 72.;
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        assert_eq!(run(&mut session, &json!({"method":"threshold"})).unwrap(), Value::Null);
        let state = session.active().unwrap();
        assert_eq!(state.history.past_len(), history + 1);
        assert_eq!(state.doc.mode, ColorMode::Bitmap);
        assert_eq!(state.doc.depth, SampleType::U8);
        assert_eq!(state.doc.physical_resolution, before.physical_resolution);
        assert_eq!(state.doc.resolution_dpi, before.resolution_dpi);
        session.execute("edit.undo", json!({})).unwrap();
        let restored = &session.active().unwrap().doc;
        assert_eq!(restored.mode, before.mode);
        assert_eq!(restored.depth, before.depth);
        assert_eq!(
            restored.layers[0].surface().unwrap().try_read_interleaved_with(restored.bounds(), &Interrupt::NONE).unwrap(),
            before.layers[0].surface().unwrap().try_read_interleaved_with(before.bounds(), &Interrupt::NONE).unwrap()
        );
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.mode, ColorMode::Bitmap);
    }
}
