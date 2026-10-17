//! Additive checked ordinary feather-mask preparation.
use super::masks;
use photocraft_doc::Layer;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
#[derive(Clone, Copy, Debug)]
pub struct FeatherPreparationLimits {
    pub max_bytes: usize,
    pub max_work: u64,
}
impl Default for FeatherPreparationLimits {
    fn default() -> Self {
        Self { max_bytes: 64 << 20, max_work: 1 << 32 }
    }
}
const FORMAT: photocraft_color::PixelFormat =
    photocraft_color::PixelFormat { mode: photocraft_color::ColorMode::Grayscale, sample: photocraft_color::SampleType::U16, alpha: false };
struct Control<'a, 'b> {
    token: &'a Interrupt<'b>,
    limits: FeatherPreparationLimits,
    work: u64,
    since_poll: usize,
}
impl Control<'_, '_> {
    fn tick(&mut self, count: usize) -> Result<(), RasterInputError> {
        self.work = self.work.checked_add(count as u64).ok_or(RasterInputError::LimitExceeded)?;
        if self.work > self.limits.max_work {
            return Err(RasterInputError::LimitExceeded);
        }
        self.since_poll = self.since_poll.saturating_add(count);
        if self.since_poll >= 256 {
            self.since_poll = 0;
            self.token.check().map_err(|_| RasterInputError::Cancelled)?;
        }
        Ok(())
    }
    fn zeros<T: Copy + Default>(&mut self, count: usize) -> Result<Vec<T>, RasterInputError> {
        let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)?;
        if bytes > self.limits.max_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut result = Vec::new();
        result.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
        if result.capacity() != count {
            return Err(RasterInputError::LimitExceeded);
        }
        while result.len() < count {
            let amount = (count - result.len()).min(256);
            self.tick(amount)?;
            result.resize(result.len() + amount, T::default());
        }
        Ok(result)
    }
}
fn bounds(path: &photocraft_doc::Path, cx: &mut Control<'_, '_>) -> Result<Option<(f64, f64, f64, f64)>, RasterInputError> {
    let mut result: Option<(f64, f64, f64, f64)> = None;
    for subpath in &path.subpaths {
        cx.tick(1)?;
        for knot in &subpath.knots {
            for p in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
                cx.tick(1)?;
                if !p.x.is_finite() || !p.y.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                if p.x.abs() > (1 << 29) as f64 || p.y.abs() > (1 << 29) as f64 {
                    return Err(RasterInputError::InvalidBounds);
                }
                result = Some(result.map_or((p.x, p.y, p.x, p.y), |b| (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y))));
            }
        }
    }
    Ok(result)
}
fn content_bounds(surface: &Surface, cx: &mut Control<'_, '_>) -> Result<Rect, RasterInputError> {
    use photocraft_geom::TILE_SIZE;
    let bpp = surface.format().bytes_per_pixel();
    let expected = (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_mul(bpp)).ok_or(RasterInputError::LimitExceeded)?;
    if bpp == 0 || surface.default_bytes().len() != bpp {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut result = Rect::EMPTY;
    for (coordinate, tile) in surface.tiles() {
        cx.tick(1)?;
        let x = coordinate.tx.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        let y = coordinate.ty.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        let origin =
            Rect::new(x, y, x.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?, y.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?);
        if tile.bytes().len() != expected {
            return Err(RasterInputError::LengthMismatch);
        }
        if !result.is_empty() && result.contains_rect(&origin) {
            continue;
        }
        for (row_index, row) in tile.bytes().chunks_exact(TILE_SIZE as usize * bpp).enumerate() {
            let mut first = None;
            let mut last = 0;
            for (column, pixel) in row.chunks_exact(bpp).enumerate() {
                cx.tick(1)?;
                if pixel != surface.default_bytes() {
                    first.get_or_insert(column);
                    last = column + 1;
                }
            }
            if let Some(first) = first {
                let row = y.checked_add(row_index as i32).ok_or(RasterInputError::InvalidBounds)?;
                result = result.union(&Rect::new(x + first as i32, row, x + last as i32, row + 1));
            }
        }
    }
    Ok(result)
}
fn gaussian(values: &mut [f32], width: usize, height: usize, sigma: f32, line: &mut [f32], cx: &mut Control<'_, '_>) -> Result<(), RasterInputError> {
    if sigma <= 0.0 || width == 0 || height == 0 {
        return Ok(());
    }
    // Same three-box variance construction and f32 summation order as the
    // existing ordinary mask feather. This is not GIMP Gaussian admission.
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let radius = (((ideal.floor() as usize) | 1).max(1) - 1) / 2;
    if radius == 0 {
        return Ok(());
    }
    for vertical in [false, true] {
        let (len, count) = if vertical { (height, width) } else { (width, height) };
        for k in 0..count {
            for _ in 0..3 {
                let at = |i: usize| if vertical { i * width + k } else { k * width + i };
                for i in 0..len {
                    cx.tick(1)?;
                    *line.get_mut(i).ok_or(RasterInputError::LengthMismatch)? = *values.get(at(i)).ok_or(RasterInputError::LengthMismatch)?;
                }
                let get = |i: isize| -> Result<f32, RasterInputError> {
                    line.get(i.clamp(0, len as isize - 1) as usize).copied().ok_or(RasterInputError::LengthMismatch)
                };
                let mut acc = 0.0_f32;
                for i in -(radius as isize)..=radius as isize {
                    cx.tick(1)?;
                    acc += get(i)?;
                }
                let n = (2 * radius + 1) as f32;
                for i in 0..len {
                    cx.tick(1)?;
                    *values.get_mut(at(i)).ok_or(RasterInputError::LengthMismatch)? = acc / n;
                    acc += get(i as isize + radius as isize + 1)? - get(i as isize - radius as isize)?;
                }
            }
        }
    }
    Ok(())
}
/// Known payload/work-bounded ordinary mask preparation. Original surfaces,
/// native defaults and vector data are borrowed and never modified. No global
/// successful-result cache is consulted or populated by this checked facade.
pub fn try_combined_mask(layer: &Layer, canvas: Rect, limits: FeatherPreparationLimits, control: &Interrupt<'_>) -> Result<Option<Surface>, RasterInputError> {
    use RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    photocraft_raster::checked_region_byte_len(canvas, FORMAT)?;
    let vm = layer.vector_mask.as_ref().filter(|v| v.enabled);
    let pixel = layer.mask.as_ref().filter(|m| m.enabled);
    let (sv, sp) = (vm.map_or(0.0, |v| masks::feather_sigma(v.feather)), pixel.map_or(0.0, |m| masks::feather_sigma(m.feather)));
    if vm.is_none() && sp <= 0.0 {
        return Ok(None);
    }
    let mut cx = Control {
        token: control,
        limits: FeatherPreparationLimits { max_work: if vm.is_some() { limits.max_work / 4 } else { limits.max_work }, ..limits },
        work: 0,
        since_poll: 0,
    };
    let grow = |r: Rect, sigma: f32| {
        let m = (sigma * 3.0).ceil() as i32 + 2;
        if r.is_empty() { r } else { Rect::new(r.x0.saturating_sub(m), r.y0.saturating_sub(m), r.x1.saturating_add(m), r.y1.saturating_add(m)) }
    };
    let vector_bounds = match vm {
        Some(v) => bounds(&v.path, &mut cx)?,
        None => None,
    };
    let mut area = vector_bounds
        .map_or(Rect::EMPTY, |b| grow(Rect::new(b.0.floor() as i32 - 2, b.1.floor() as i32 - 2, b.2.ceil() as i32 + 2, b.3.ceil() as i32 + 2), sv));
    if pixel.is_some_and(|m| m.surface.channels() == 0 || m.surface.format().bytes_per_pixel() == 0) {
        return Err(E::UnsupportedConversion);
    }
    let p_def = pixel.map_or(1.0, |m| {
        let d = m.surface.default_pixel().first().copied().unwrap_or(1.0);
        1.0 - m.density * (1.0 - d)
    });
    if !p_def.is_finite() {
        return Err(E::InvalidSample);
    }
    // Outside the supplied canvas, retain the exact existing ordinary default
    // observation. Coordinate validation above makes this subtraction total.
    let far = Rect::from_xywh(canvas.x0 - 1_000_000, canvas.y0 - 1_000_000, 1, 1);
    let geometry = match vm {
        Some(mask) => Some(photocraft_vector::CompiledVectorMask::try_new_with_interrupt(
            mask,
            photocraft_vector::VectorPreparationLimits { max_bytes: limits.max_bytes, max_work: limits.max_work / 4, ..Default::default() },
            control,
        )?),
        None => None,
    };
    let geometry_bytes = geometry.as_ref().map_or(0, |g| g.preparation_bytes());
    let v_out = match &geometry {
        Some(g) => g
            .try_render_with_interrupt(
                far,
                photocraft_vector::VectorPreparationLimits { max_bytes: limits.max_bytes, max_work: limits.max_work / 4, ..Default::default() },
                control,
            )?
            .first()
            .copied()
            .ok_or(E::LengthMismatch)?,
        None => 1.0,
    };
    if let Some(m) = pixel {
        m.surface.try_validate_tile_origins_with(control)?;
        let b = content_bounds(&m.surface, &mut cx)?;
        if v_out > 0.0 {
            let b = grow(b, sp);
            area = if area.is_empty() {
                b
            } else if b.is_empty() {
                area
            } else {
                area.union(&b)
            };
        }
    }
    let area = area.intersect(&canvas);
    let bytes = photocraft_raster::checked_region_byte_len(area, FORMAT)?;
    let mut tiles = 0_usize;
    for _ in area.tiles() {
        cx.tick(1)?;
        tiles = tiles.checked_add(1).ok_or(E::LimitExceeded)?;
    }
    // Keep legacy U16 derived-mask quantization; all convolution planes stay F32.
    let working_bytes = bytes.checked_mul(2).ok_or(E::LimitExceeded)?;
    let tile_bytes = tiles.checked_mul(256 * 256 * 2 + 256).ok_or(E::LimitExceeded)?;
    let line_bytes = (area.width().max(area.height()) as usize).checked_mul(4).ok_or(E::LimitExceeded)?;
    let channels = pixel.map_or(1, |m| m.surface.channels());
    let read_bytes = 1024_usize.checked_mul(channels).and_then(|n| n.checked_mul(4)).ok_or(E::LimitExceeded)?;
    let peak = working_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(bytes))
        .and_then(|n| n.checked_add(tile_bytes))
        .and_then(|n| n.checked_add(line_bytes))
        .and_then(|n| n.checked_add(read_bytes))
        .and_then(|n| n.checked_add(geometry_bytes))
        .and_then(|n| n.checked_add(4096))
        .ok_or(E::LimitExceeded)?;
    if peak > limits.max_bytes || peak as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    cx.tick(8)?;
    let mut default = [0_u8; 2];
    photocraft_color::write_sample(&mut default, FORMAT.sample, 0, p_def * v_out);
    let mut output = Surface::try_with_default_bytes(FORMAT, &default)?;
    if !area.is_empty() {
        let (w, h) = (area.width() as usize, area.height() as usize);
        let mut values = match &geometry {
            Some(g) => g.try_render_with_interrupt(
                area,
                photocraft_vector::VectorPreparationLimits {
                    max_bytes: limits.max_bytes - (peak - working_bytes - geometry_bytes),
                    max_work: limits.max_work / 4,
                    ..Default::default()
                },
                control,
            )?,
            None => {
                let mut v = cx.zeros::<f32>(working_bytes / 4)?;
                for chunk in v.chunks_mut(256) {
                    cx.tick(chunk.len())?;
                    chunk.fill(1.0);
                }
                v
            }
        };
        if values.capacity() != working_bytes / 4 {
            return Err(E::LimitExceeded);
        }
        let mut line = cx.zeros::<f32>(w.max(h))?;
        gaussian(&mut values, w, h, sv, &mut line, &mut cx)?;
        if let Some(m) = pixel {
            let mut pm = cx.zeros::<f32>(working_bytes / 4)?;
            let mut read = cx.zeros::<f32>(read_bytes / 4)?;
            for y in area.y0..area.y1 {
                let mut x = area.x0;
                while x < area.x1 {
                    let right = x.saturating_add(1024).min(area.x1);
                    control.check().map_err(|_| E::Cancelled)?;
                    m.surface.read_region_into(Rect::new(x, y, right, y + 1), &mut read);
                    for (i, p) in read.chunks_exact(channels).enumerate() {
                        cx.tick(1)?;
                        let mut value = p.first().copied().ok_or(E::LengthMismatch)?;
                        if m.density < 1.0 {
                            value = 1.0 - m.density * (1.0 - value);
                        }
                        if !value.is_finite() {
                            return Err(E::InvalidSample);
                        }
                        let at = (y - area.y0) as usize * w + (x - area.x0) as usize + i;
                        *pm.get_mut(at).ok_or(E::LengthMismatch)? = value;
                    }
                    x = right;
                }
            }
            gaussian(&mut pm, w, h, sp, &mut line, &mut cx)?;
            for (a, b) in values.iter_mut().zip(pm) {
                cx.tick(1)?;
                *a *= b;
            }
        }
        let mut encoded = cx.zeros::<u8>(bytes)?;
        for (value, out) in values.iter().zip(encoded.as_chunks_mut::<2>().0.iter_mut()) {
            cx.tick(8)?;
            photocraft_color::write_sample(out, FORMAT.sample, 0, *value);
        }
        // The raster writer polls cancellation, while this facade owns its Work
        // envelope: include physical default-filled tiles even for a tiny ROI.
        let mut writer_work = tiles.checked_mul(256 * 256 * 2 + 256).and_then(|n| n.checked_add(bytes)).ok_or(E::LimitExceeded)?;
        while writer_work != 0 {
            let chunk = writer_work.min(65_536);
            cx.tick(chunk)?;
            writer_work -= chunk;
        }
        output.try_write_interleaved_with(area, &encoded, control)?;
    }
    control.check().map_err(|_| E::Cancelled)?;
    Ok(Some(output))
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::LayerMask;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn original() -> Layer {
        let mut layer = Layer::raster("own feather", PixelFormat::RGBA8);
        let mut mask = LayerMask::hide_all();
        std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(64, 64, 192, 192), &[0.73]);
        mask.feather = 16.25;
        layer.mask = Some(mask);
        layer
    }
    #[test]
    fn checked_feathers_match_legacy_pixels_defaults_and_source_at_multiple_depths() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_doc::{Path, Subpath, VectorMask};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32, SampleType::U32, SampleType::F64] {
            for inverted in [false, true] {
                let mut layer = original();
                let mask = layer.mask.as_mut().unwrap();
                let format = PixelFormat { mode: ColorMode::Grayscale, sample: depth, alpha: false };
                mask.surface = std::sync::Arc::new(Surface::with_default(format, &[0.13]));
                std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(-37, 12, 92, 133), &[0.79]);
                mask.density = 0.71;
                mask.feather = 3.75;
                let mut path = Path::new(vec![Subpath::polygon(&[(-30.7, -17.2), (143.2, 3.7), (56.4, 162.8)])]);
                path.inverted = inverted;
                let mut vector = VectorMask::new(path);
                vector.feather = 7.25;
                vector.density = 0.83;
                layer.vector_mask = Some(vector);
                let before = layer.clone();
                let canvas = Rect::new(-48, -24, 176, 168);
                let expected = masks::combined_mask(&layer, canvas).unwrap();
                let actual = try_combined_mask(&layer, canvas, FeatherPreparationLimits::default(), &Interrupt::NONE).unwrap().unwrap();
                assert_eq!(actual, expected, "{depth:?}, inverted={inverted}");
                assert_eq!(layer, before);
            }
        }
    }
    #[test]
    fn checked_feathers_reject_bad_origins_and_work_before_returning_surface() {
        let layer = original();
        let before = layer.clone();
        assert!(matches!(
            try_combined_mask(&layer, Rect::new(0, 0, 256, 256), FeatherPreparationLimits { max_work: 1, ..Default::default() }, &Interrupt::NONE),
            Err(RasterInputError::LimitExceeded)
        ));
        assert!(try_combined_mask(&layer, Rect::new(i32::MAX - 2, 0, i32::MAX, 1), FeatherPreparationLimits::default(), &Interrupt::NONE).is_err());
        assert_eq!(layer, before);
    }
    #[test]
    fn cancellation_is_observed_inside_feather_preparation() {
        let layer = original();
        let before = layer.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        assert!(matches!(
            try_combined_mask(&layer, Rect::new(0, 0, 256, 256), FeatherPreparationLimits::default(), &Interrupt::cancel_only(&cancel)),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(layer, before);
    }
    #[test]
    fn feather_zero_payload_budget_returns_error_instead_of_cached_success() {
        let layer = original();
        let before = layer.clone();
        assert!(matches!(
            try_combined_mask(&layer, Rect::new(0, 0, 256, 256), FeatherPreparationLimits { max_bytes: 0, ..Default::default() }, &Interrupt::NONE),
            Err(RasterInputError::LimitExceeded)
        ));
        assert_eq!(layer, before);
    }
}
