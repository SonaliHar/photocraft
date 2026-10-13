//! Checked native extraction for ordinary neutral wide raster layers.
use crate::{
    EngineError, Result,
    doc::{ColorMode, Document, Layer, LayerContent, LayerId, Rect, SampleType, Size, WorkingProfileInterpretation},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};

/// Aggregate preparation limits, including retained source pixels.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub peak_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { peak_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }
}
fn refusal(message: &str) -> EngineError {
    EngineError::Other(format!("wide layer export: {message}"))
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| refusal("preparation limit exceeded"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| refusal("preparation limit exceeded"))
}
fn surface_charge(surface: &Surface) -> Result<u64> {
    add(surface.resident_payload_bytes()?, mul(surface.tiles().count() as u64, 256)?)
}

/// Extract exact native words. Unsupported contexts never enter the f32 compositor.
pub fn extract(doc: &Document, id: LayerId, limits: Limits, control: &Interrupt<'_>) -> Result<Document> {
    if control.cancelled() {
        return Err(refusal("cancelled"));
    }
    doc.effective_resolution().validate().map_err(|e| refusal(&e.to_string()))?;
    let limits = Limits { peak_bytes: limits.peak_bytes.min(MAX_CHECKED_RASTER_BYTES), work_units: limits.work_units };
    if doc.layers.len() > 10_000 {
        return Err(refusal("preparation limit exceeded"));
    }
    let layer = doc.layers.iter().find(|l| l.id == id).ok_or(EngineError::NoLayer(id))?;
    if doc.mode != ColorMode::Rgb
        || !matches!(doc.depth, SampleType::U32 | SampleType::F64)
        || doc.icc_profile.is_some()
        || doc.sample_encoding.is_some()
        || doc.source_color_context.is_some()
        || doc.xcf_state.is_some()
        || !doc.patterns.is_empty()
        || doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
    {
        return Err(refusal("unsupported source color context"));
    }
    let neutral = |l: &Layer| {
        l.opacity.to_bits() == 1f32.to_bits()
            && l.fill_opacity.to_bits() == 1f32.to_bits()
            && l.mask.is_none()
            && l.vector_mask.is_none()
            && l.effects.items.is_empty()
            && l.effects.psd_raw.is_none()
            && l.effects.reference.is_none()
            && l.fill_cache.is_none()
            && l.indexed_source.is_none()
            && l.import_compositing.is_none()
            && l.excluded_channels == 0
            && l.blend_if == Default::default()
            && l.advanced == Default::default()
            && l.psd_blocks.is_empty()
            && l.video.is_none()
    };
    let LayerContent::Raster(source) = &layer.content else {
        return Err(refusal("unsupported layer content"));
    };
    if !neutral(layer) || source.format() != doc.pixel_format() || source.channels() != 4 || source.default_bytes().iter().any(|v| *v != 0) {
        return Err(refusal("unsupported nonneutral raster"));
    }
    // All retained raster payloads participate in the aggregate peak. Other storage
    // families need their own accounting before they can enter this lane.
    let mut resident = 0;
    if doc.layers.len() > 10_000 || doc.channels.len() > 10_000 {
        return Err(refusal("preparation limit exceeded"));
    }
    for l in &doc.layers {
        if control.cancelled() {
            return Err(refusal("cancelled"));
        }
        let LayerContent::Raster(s) = &l.content else {
            return Err(refusal("unsupported retained layer storage"));
        };
        if !neutral(l) {
            return Err(refusal("unsupported retained layer storage"));
        }
        resident = add(resident, surface_charge(s)?)?;
    }
    for channel in &doc.channels {
        if control.cancelled() {
            return Err(refusal("cancelled"));
        }
        resident = add(resident, surface_charge(&channel.surface)?)?;
    }
    if let Some(s) = &doc.selection {
        resident = add(resident, surface_charge(s)?)?;
    }
    if let Some(s) = &doc.quick_mask {
        resident = add(resident, surface_charge(&s.surface)?)?;
    }
    if resident > limits.peak_bytes {
        return Err(refusal("preparation peak limit exceeded"));
    }
    source.try_validate_tile_origins_with(control)?;
    let canvas = doc.bounds();
    photocraft_raster::checked_region_byte_len(canvas, source.format())?;
    let tile_bytes = mul(256 * 256, source.format().bytes_per_pixel() as u64)?;
    let scratch = mul(tile_bytes, 6)?;
    if add(resident, scratch)? > limits.peak_bytes {
        return Err(refusal("preparation peak limit exceeded"));
    }
    let mut bounds = Rect::EMPTY;
    let mut work = 0;
    for (coord, tile) in source.tiles() {
        let x0 = coord.tx.checked_mul(256).ok_or_else(|| refusal("invalid tile origin"))?;
        let y0 = coord.ty.checked_mul(256).ok_or_else(|| refusal("invalid tile origin"))?;
        let origin = Rect::new(
            x0,
            y0,
            x0.checked_add(256).ok_or_else(|| refusal("invalid tile origin"))?,
            y0.checked_add(256).ok_or_else(|| refusal("invalid tile origin"))?,
        );
        photocraft_raster::checked_region_byte_len(origin, source.format())?;
        let area = origin.intersect(&canvas);
        if area.is_empty() {
            continue;
        }
        let pixels = mul(area.width() as u64, area.height() as u64)?;
        work = add(work, mul(pixels, 4)?)?;
        if work > limits.work_units {
            return Err(refusal("preparation work limit exceeded"));
        }
        let bpp = source.format().bytes_per_pixel();
        for y in area.y0..area.y1 {
            if control.cancelled() {
                return Err(refusal("cancelled"));
            }
            for x in area.x0..area.x1 {
                let index = ((y - origin.y0) as usize * 256 + (x - origin.x0) as usize).checked_mul(bpp).ok_or_else(|| refusal("invalid tile extent"))?;
                let raw = tile
                    .bytes()
                    .get(index..index.checked_add(bpp).ok_or_else(|| refusal("invalid tile extent"))?)
                    .ok_or_else(|| refusal("invalid tile payload"))?;
                let opaque = match doc.depth {
                    SampleType::U32 => {
                        let alpha = raw.get(12..16).and_then(|s| <[u8; 4]>::try_from(s).ok()).ok_or_else(|| refusal("invalid native pixel"))?;
                        match u32::from_ne_bytes(alpha) {
                            0 => false,
                            u32::MAX => true,
                            _ => return Err(refusal("unsupported partial alpha")),
                        }
                    }
                    SampleType::F64 => {
                        let mut alpha = 0.;
                        for (i, word) in raw.as_chunks::<8>().0.iter().enumerate() {
                            let value = f64::from_ne_bytes(*word);
                            if !value.is_finite() {
                                return Err(refusal("unsupported nonfinite sample"));
                            }
                            if i == 3 {
                                alpha = value;
                            }
                        }
                        if alpha == 1. {
                            true
                        } else if alpha == 0. {
                            false
                        } else {
                            return Err(refusal("unsupported partial alpha"));
                        }
                    }
                    _ => return Err(refusal("unsupported sample type")),
                };
                if !opaque && raw.iter().any(|v| *v != 0) {
                    return Err(refusal("unsupported hidden color or nonzero transparent words"));
                }
                if opaque {
                    bounds = bounds.union(&Rect::new(
                        x,
                        y,
                        x.checked_add(1).ok_or_else(|| refusal("invalid crop"))?,
                        y.checked_add(1).ok_or_else(|| refusal("invalid crop"))?,
                    ));
                }
            }
        }
    }
    if bounds.is_empty() {
        return Err(EngineError::Other("the layer is empty".into()));
    }
    let destination = Rect::from_xywh(0, 0, bounds.width(), bounds.height());
    let tiles = mul(u64::from(bounds.width()).div_ceil(256), u64::from(bounds.height()).div_ceil(256))?;
    // Payload plus conservative BTreeMap/Arc bookkeeping for source and staged copies.
    let output = mul(tiles, add(tile_bytes, 256)?)?;
    if add(add(resident, output)?, scratch)? > limits.peak_bytes {
        return Err(refusal("preparation peak limit exceeded"));
    }
    let copy_tiles = bounds.tiles().count() as u64;
    // Reads plus up to four destination replacement tiles per source band;
    // cloned tile-map entries are also charged for every staged band.
    work = add(work, add(mul(copy_tiles, 5 * 256 * 256 * 4)?, mul(copy_tiles, tiles)?)?)?;
    if work > limits.work_units {
        return Err(refusal("preparation work limit exceeded"));
    }
    photocraft_raster::checked_region_byte_len(destination, source.format())?;
    let dx = bounds.x0.checked_neg().ok_or_else(|| refusal("invalid crop origin"))?;
    let dy = bounds.y0.checked_neg().ok_or_else(|| refusal("invalid crop origin"))?;
    let surface: Surface = photocraft_algo::wide::try_translate(source, bounds, dx, dy, control)?;
    let mut out = Document::new(layer.name.clone(), Size::new(bounds.width(), bounds.height()), doc.mode, doc.depth);
    out.resolution_dpi = doc.resolution_dpi;
    out.physical_resolution = doc.physical_resolution.clone();
    out.layers.push(Layer::new(layer.name.clone(), LayerContent::Raster(surface)));
    Ok(out)
}
