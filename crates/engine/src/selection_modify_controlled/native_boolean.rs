//! Closed native Modify input: Boolean U32 or exactly f32-representable finite unit F64.
//! Every retained word is validated before plane lifting. Arithmetic stays upstream f32;
//! this is not native-f64 math or a source-operation qualification.
//! Generated coverage is encoded directly, with U32 half-up rounding or exact f32-to-f64 widening.
use crate::{EngineError, Result};
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    Interrupt, RasterInputError, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
fn bad(cmd: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
pub(crate) fn is_wide(source: &Surface) -> bool {
    matches!(source.format().sample, SampleType::U32 | SampleType::F64)
}
fn lossless_sample(word: &[u8], sample: SampleType, cmd: &str) -> Result<f32> {
    match sample {
        SampleType::U32 => {
            let bytes: [u8; 4] = word.try_into().map_err(|_| bad(cmd, "native word width"))?;
            match u32::from_ne_bytes(bytes) {
                0 => Ok(0.0),
                u32::MAX => Ok(1.0),
                _ => Err(bad(cmd, "fractional native Modify input is unsupported")),
            }
        }
        SampleType::F64 => {
            let bytes: [u8; 8] = word.try_into().map_err(|_| bad(cmd, "native word width"))?;
            let value = f64::from_ne_bytes(bytes);
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(bad(cmd, "native Modify input must be finite unit coverage"));
            }
            let lifted = value as f32;
            // Bit equality includes signed zero. The cast is only a scalar admission check;
            // it is not used as a plane value unless widening recovers the original word.
            if f64::from(lifted).to_bits() == value.to_bits() {
                Ok(lifted)
            } else {
                Err(bad(cmd, "F64 Modify input is not exactly f32-representable; no lossy lift performed"))
            }
        }
        _ => Err(bad(cmd, "not a supported native selection")),
    }
}
pub(crate) fn validate_default(source: &Surface, cmd: &str) -> Result<()> {
    if is_wide(source) {
        lossless_sample(source.default_bytes(), source.format().sample, cmd)?;
    }
    Ok(())
}
pub(crate) fn validation_work(source: &Surface, cmd: &str, ctl: &Interrupt<'_>) -> Result<u64> {
    if !is_wide(source) {
        return Ok(0);
    }
    let mut work = 1u64;
    for _ in source.tiles() {
        poll(ctl)?;
        work = work.checked_add((TILE_SIZE as u64) * (TILE_SIZE as u64)).filter(|n| *n <= 1 << 30).ok_or_else(|| bad(cmd, "Boolean validation work limit"))?;
    }
    poll(ctl)?;
    Ok(work)
}
/// Census precedes scans; includes the default and every retained word, including off-canvas tails.
pub(crate) fn validate(source: &Surface, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    if !is_wide(source) {
        return Ok(());
    }
    poll(ctl)?;
    if source.format().mode != ColorMode::Grayscale || source.format().alpha {
        return Err(bad(cmd, "native Modify requires Gray without alpha"));
    }
    source.try_validate_tile_origins_with(ctl)?;
    validation_work(source, cmd, ctl)?;
    lossless_sample(source.default_bytes(), source.format().sample, cmd)?;
    let width = source.format().sample.bytes();
    for (_, tile) in source.tiles() {
        poll(ctl)?;
        if tile.bytes().len() % width != 0 {
            return Err(bad(cmd, "native tile word alignment"));
        }
        for block in tile.bytes().chunks(32768) {
            poll(ctl)?;
            for word in block.chunks_exact(width) {
                lossless_sample(word, source.format().sample, cmd)?;
            }
        }
    }
    poll(ctl)
}
pub(crate) fn sample(source: &Surface, x: i32, y: i32, cmd: &str, ctl: &Interrupt<'_>) -> Result<f32> {
    poll(ctl)?;
    let mut word = [0u8; 8];
    let bytes = word.get_mut(..source.format().sample.bytes()).ok_or_else(|| bad(cmd, "native word width"))?;
    source.try_read_pixel_bytes(x, y, bytes)?;
    lossless_sample(bytes, source.format().sample, cmd)
}
struct Budget {
    bytes: u64,
    work: u64,
}
impl StagingBudget for Budget {
    type Lease = ();
    fn reserve(&mut self, n: u64) -> std::result::Result<(), RasterInputError> {
        self.bytes = self.bytes.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, n: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
/// Canvas-only output and zero default retain the existing Modify geometry policy.
// Keep the explicit authority, borrowed payload and original-control proof inputs together.
#[allow(clippy::too_many_arguments)]
pub(crate) fn output(
    source: &Surface,
    bounds: Rect,
    plane: &[f32],
    padded_width: usize,
    pad: usize,
    stage_bytes: u64,
    stage_work: u64,
    cmd: &str,
    ctl: &Interrupt<'_>,
) -> Result<Option<Surface>> {
    let format = source.format();
    let bytes = format.sample.bytes();
    let zero = [0u8; 8];
    let default = zero.get(..bytes).ok_or_else(|| bad(cmd, "output word width"))?;
    let mut out = Surface::try_with_default_bytes(format, default)?;
    let len = (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_mul(bytes)).ok_or_else(|| bad(cmd, "native tile capacity"))?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(len).map_err(|_| bad(cmd, "native tile allocation"))?;
    raw.resize(len, 0);
    let cell_len = 64usize * 64 * bytes;
    let mut cell = Vec::new();
    cell.try_reserve_exact(cell_len).map_err(|_| bad(cmd, "native cell allocation"))?;
    if raw.capacity() > len.saturating_add(64) || cell.capacity() > cell_len.saturating_add(64) {
        return Err(bad(cmd, "native scratch capacity"));
    }
    let mut budget = Budget { bytes: stage_bytes, work: stage_work };
    let mut positive = false;
    for position in bounds.tiles() {
        poll(ctl)?;
        raw.fill(0);
        let tile = position.rect();
        let area = tile.intersect(&bounds);
        for y in area.y0..area.y1 {
            poll(ctl)?;
            for x in area.x0..area.x1 {
                poll(ctl)?;
                let offset = (y as usize)
                    .checked_add(pad)
                    .and_then(|n| n.checked_mul(padded_width))
                    .and_then(|n| n.checked_add((x as usize).checked_add(pad)?))
                    .ok_or_else(|| bad(cmd, "native crop offset"))?;
                let value = *plane.get(offset).ok_or_else(|| bad(cmd, "native crop value"))?;
                if !value.is_finite() {
                    return Err(bad(cmd, "nonfinite generated coverage"));
                }
                let value = f64::from(value.clamp(0.0, 1.0));
                let destination = ((y - tile.y0) as usize)
                    .checked_mul(TILE_SIZE as usize)
                    .and_then(|n| n.checked_add((x - tile.x0) as usize))
                    .and_then(|n| n.checked_mul(bytes))
                    .ok_or_else(|| bad(cmd, "native output offset"))?;
                let target = raw.get_mut(destination..destination + bytes).ok_or_else(|| bad(cmd, "native output word"))?;
                match format.sample {
                    SampleType::U32 => {
                        let code = (value * f64::from(u32::MAX) + 0.5).floor() as u32;
                        positive |= code != 0;
                        target.copy_from_slice(&code.to_ne_bytes());
                    }
                    SampleType::F64 => {
                        positive |= value > 0.0;
                        target.copy_from_slice(&value.to_ne_bytes());
                    }
                    _ => return Err(bad(cmd, "unsupported native output")),
                }
            }
        }
        let mut builder = StagedSurfaceBuilder::new(format, default, tile, &mut budget, ctl)?;
        for cy in (0..TILE_SIZE).step_by(64) {
            for cx in (0..TILE_SIZE).step_by(64) {
                poll(ctl)?;
                cell.clear();
                for row in cy..cy + 64 {
                    let offset = ((row as usize) * (TILE_SIZE as usize) + cx as usize) * bytes;
                    cell.extend_from_slice(raw.get(offset..offset + 64 * bytes).ok_or_else(|| bad(cmd, "native cell row"))?);
                }
                builder.push_region(Rect::new(tile.x0 + cx, tile.y0 + cy, tile.x0 + cx + 64, tile.y0 + cy + 64), &cell, &mut budget, ctl)?;
            }
        }
        let (replacement, _lease) = builder.finalize(&mut budget, ctl)?.into_parts();
        for (position, tile) in replacement.tiles() {
            poll(ctl)?;
            out.put_tiles(std::iter::once((*position, std::sync::Arc::clone(tile))));
        }
    }
    out.try_prune_with(ctl)?;
    poll(ctl)?;
    Ok(positive.then_some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    #[test]
    fn native_lift_is_lossless_and_nonrepresentable_or_nonfinite_refuse() {
        assert_eq!(lossless_sample(&u32::MAX.to_ne_bytes(), SampleType::U32, "draft").unwrap(), 1.0);
        assert_eq!(lossless_sample(&(-0.0f64).to_ne_bytes(), SampleType::F64, "draft").unwrap(), 0.0);
        assert!(lossless_sample(&1u32.to_ne_bytes(), SampleType::U32, "draft").is_err());
        assert_eq!(lossless_sample(&0.5f64.to_ne_bytes(), SampleType::F64, "draft").unwrap(), 0.5);
        let adjacent = f64::from_bits(0.5f64.to_bits() + 1);
        assert!(lossless_sample(&adjacent.to_ne_bytes(), SampleType::F64, "draft").is_err());
        let negative_zero = lossless_sample(&(-0.0f64).to_ne_bytes(), SampleType::F64, "draft").unwrap();
        assert_eq!(negative_zero.to_bits(), (-0.0f32).to_bits());
        let subnormal = f64::from(f32::from_bits(1));
        assert_eq!(f64::from(lossless_sample(&subnormal.to_ne_bytes(), SampleType::F64, "draft").unwrap()).to_bits(), subnormal.to_bits());
        assert!(lossless_sample(&f64::from_bits(1).to_ne_bytes(), SampleType::F64, "draft").is_err());
        assert!(lossless_sample(&f64::NAN.to_ne_bytes(), SampleType::F64, "draft").is_err());
    }
    #[test]
    fn offcanvas_fractional_storage_refuses_without_changing_source() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let mut source = Surface::try_with_default_bytes(format, &0u32.to_ne_bytes()).unwrap();
        source.try_write_interleaved(Rect::new(-7, 9, -6, 10), &1u32.to_ne_bytes()).unwrap();
        let original = source.clone();
        assert!(validate(&source, "draft", &Interrupt::NONE).is_err());
        let mut word = [0u8; 4];
        source.try_read_pixel_bytes(-7, 9, &mut word).unwrap();
        assert_eq!(u32::from_ne_bytes(word), 1);
        assert_eq!(source.default_bytes(), original.default_bytes());
        assert!(matches!(validate(&source, "draft", &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }
}
