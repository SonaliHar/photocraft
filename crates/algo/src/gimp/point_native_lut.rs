//! Caller-owned exact native-code LUT primitive, not a GEGL/schema/profile evaluator.
//! Dense integer lookup and strictly ordered exact sparse code matching only.
//! No interpolation, gamma, clamp, float narrowing or source admission is inferred.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::Arc;
const MAX_WORK: u64 = 1 << 30;
const MAX_KNOTS: usize = 65536;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMarker {
    Linear,
    Nonlinear,
    Perceptual,
    Opaque(u64),
}
/// Equality markers are supplied by the CMS/context owner; they do not prove an ICC alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub format: PixelFormat,
    pub transfer: TransferMarker,
    pub profile_marker: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupPolicy {
    ExactCodesOnly,
}
#[derive(Clone, Copy)]
pub enum NativeTable<'a> {
    DenseU8(&'a Vec<u8>),
    DenseU16(&'a Vec<u16>),
    ExactU32(&'a Vec<(u32, u32)>),
    ExactF32Bits(&'a Vec<(u32, u32)>),
    ExactF64Bits(&'a Vec<(u64, u64)>),
}
#[derive(Clone, Copy)]
pub struct ChannelTable<'a> {
    pub channel: usize,
    pub table: NativeTable<'a>,
}
#[derive(Clone, Copy, Debug)]
pub struct LutLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
/// Conservative borrowed-input and private-output admission; not allocator/RSS.
/// Index bytes include the one cloned sparse map and temporary per-tile builder
/// envelopes. Existing upstream input charges are never refunded by this estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub parameter_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
pub struct PreparedNativeLut<'a> {
    context: WorkingContext,
    tables: &'a [ChannelTable<'a>],
    retained: u64,
    validation_work: u64,
    lookup_work: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn limits(limits: LutLimits) -> Result<(), RasterInputError> {
    if limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}
impl NativeTable<'_> {
    fn descriptor(self) -> Result<(SampleType, usize, u64, u64), RasterInputError> {
        let descriptor = match self {
            Self::DenseU8(v) if v.len() == 256 => (SampleType::U8, v.len(), v.capacity(), 1usize, 1),
            Self::DenseU16(v) if v.len() == 65536 => (SampleType::U16, v.len(), v.capacity(), 2, 1),
            Self::ExactU32(v) if !v.is_empty() && v.len() <= MAX_KNOTS => (SampleType::U32, v.len(), v.capacity(), 8, 17),
            Self::ExactF32Bits(v) if !v.is_empty() && v.len() <= MAX_KNOTS => (SampleType::F32, v.len(), v.capacity(), 8, 18),
            Self::ExactF64Bits(v) if !v.is_empty() && v.len() <= MAX_KNOTS => (SampleType::F64, v.len(), v.capacity(), 16, 18),
            _ => return Err(RasterInputError::LengthMismatch),
        };
        let (sample, count, capacity, width, lookup) = descriptor;
        let retained = (capacity as u64).checked_mul(width as u64).ok_or(RasterInputError::LimitExceeded)?;
        Ok((sample, count, retained, lookup))
    }
    fn validate(self, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
        match self {
            Self::DenseU8(_) | Self::DenseU16(_) => {}
            Self::ExactU32(v) | Self::ExactF32Bits(v) => {
                let float = matches!(self, Self::ExactF32Bits(_));
                let mut previous = None;
                for &(key, value) in v {
                    poll(control)?;
                    if previous.is_some_and(|old| old >= key) {
                        return Err(RasterInputError::InvalidSample);
                    }
                    if float && (!f32::from_bits(key).is_finite() || !f32::from_bits(value).is_finite()) {
                        return Err(RasterInputError::InvalidSample);
                    }
                    previous = Some(key);
                }
            }
            Self::ExactF64Bits(v) => {
                let mut previous = None;
                for &(key, value) in v {
                    poll(control)?;
                    if previous.is_some_and(|old| old >= key) || !f64::from_bits(key).is_finite() || !f64::from_bits(value).is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    previous = Some(key);
                }
            }
        }
        poll(control)
    }
    fn map(self, input: &[u8], output: &mut [u8]) -> Result<(), RasterInputError> {
        let width = match self {
            Self::DenseU8(_) => 1,
            Self::DenseU16(_) => 2,
            Self::ExactU32(_) | Self::ExactF32Bits(_) => 4,
            Self::ExactF64Bits(_) => 8,
        };
        if input.len() != width || output.len() != width {
            return Err(RasterInputError::LengthMismatch);
        }
        match self {
            Self::DenseU8(v) => {
                let key = *input.first().filter(|_| input.len() == 1).ok_or(RasterInputError::LengthMismatch)?;
                let value = *v.get(usize::from(key)).ok_or(RasterInputError::LengthMismatch)?;
                *output.first_mut().ok_or(RasterInputError::LengthMismatch)? = value;
            }
            Self::DenseU16(v) => {
                let key = u16::from_ne_bytes(input.try_into().map_err(|_| RasterInputError::LengthMismatch)?);
                let value = v.get(usize::from(key)).ok_or(RasterInputError::LengthMismatch)?;
                output.copy_from_slice(&value.to_ne_bytes());
            }
            Self::ExactU32(v) | Self::ExactF32Bits(v) => {
                let key = u32::from_ne_bytes(input.try_into().map_err(|_| RasterInputError::LengthMismatch)?);
                if matches!(self, Self::ExactF32Bits(_)) && !f32::from_bits(key).is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                let position = v.binary_search_by_key(&key, |entry| entry.0).map_err(|_| RasterInputError::UnsupportedConversion)?;
                let value = v.get(position).ok_or(RasterInputError::InvalidSample)?.1;
                output.copy_from_slice(&value.to_ne_bytes());
            }
            Self::ExactF64Bits(v) => {
                let key = u64::from_ne_bytes(input.try_into().map_err(|_| RasterInputError::LengthMismatch)?);
                if !f64::from_bits(key).is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                let position = v.binary_search_by_key(&key, |entry| entry.0).map_err(|_| RasterInputError::UnsupportedConversion)?;
                let value = v.get(position).ok_or(RasterInputError::InvalidSample)?.1;
                output.copy_from_slice(&value.to_ne_bytes());
            }
        }
        Ok(())
    }
}
impl<'a> PreparedNativeLut<'a> {
    /// Validate borrowed tables once. Vec capacities (not only lengths) remain charged
    /// as caller-retained inputs; this object neither copies nor owns their payloads.
    pub fn prepare(
        context: WorkingContext,
        tables: &'a [ChannelTable<'a>],
        policy: LookupPolicy,
        budget: LutLimits,
        control: &Interrupt<'_>,
    ) -> Result<Self, RasterInputError> {
        poll(control)?;
        limits(budget)?;
        if !matches!(context.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || !matches!(context.format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        match policy {
            LookupPolicy::ExactCodesOnly => {}
        }
        if tables.is_empty() || tables.len() > context.format.channels() {
            return Err(RasterInputError::InvalidSample);
        }
        let mut seen = [false; 4];
        let mut retained = 0u64;
        let mut validation_work = 0u64;
        let mut lookup_work = 0u64;
        for selected in tables {
            poll(control)?;
            let slot = seen.get_mut(selected.channel).filter(|_| selected.channel < context.format.channels()).ok_or(RasterInputError::InvalidSample)?;
            if *slot {
                return Err(RasterInputError::InvalidSample);
            }
            *slot = true;
            let (sample, count, bytes, work) = selected.table.descriptor()?;
            if sample != context.format.sample {
                return Err(RasterInputError::UnsupportedConversion);
            }
            retained = retained.checked_add(bytes).ok_or(RasterInputError::LimitExceeded)?;
            validation_work = validation_work.checked_add(count as u64).ok_or(RasterInputError::LimitExceeded)?;
            lookup_work =
                lookup_work.checked_add(work).and_then(|n| n.checked_add(context.format.sample.bytes() as u64)).ok_or(RasterInputError::LimitExceeded)?;
        }
        if retained > budget.resident_bytes || validation_work > budget.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        for selected in tables {
            selected.table.validate(control)?;
        }
        poll(control)?;
        Ok(Self { context, tables, retained, validation_work, lookup_work })
    }
    pub fn format(&self) -> PixelFormat {
        self.context.format
    }
    pub fn context(&self) -> WorkingContext {
        self.context
    }
    /// Metadata-only checked requirements. No raster sample reads, table copies,
    /// rendering or hashing. Invalid retained tile layouts refuse before execution.
    pub fn requirements(&self, source: &Surface, roi: Rect, current: WorkingContext) -> Result<Requirements, RasterInputError> {
        self.requirements_with_interrupt(source, roi, current, &Interrupt::NONE)
    }
    pub fn requirements_with_interrupt(
        &self,
        source: &Surface,
        roi: Rect,
        current: WorkingContext,
        control: &Interrupt<'_>,
    ) -> Result<Requirements, RasterInputError> {
        poll(control)?;
        if current != self.context || source.format() != self.context.format {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let format = source.format();
        let bpp = format.bytes_per_pixel();
        let length = checked_region_byte_len(roi, format)?;
        let output_tiles = tile_count(roi)?;
        let source_tiles = source.tile_count() as u64;
        let tile_bytes = 65536u64.checked_mul(bpp as u64).ok_or(RasterInputError::LimitExceeded)?;
        let source_retained_bytes = source_tiles
            .checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|n| n.checked_add(bpp as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        // The standard source envelope bounds the metadata traversal even if the
        // eventual graph supplies a smaller aggregate cap. It never raises a cap.
        if source_retained_bytes > MAX_CHECKED_RASTER_BYTES {
            return Err(RasterInputError::LimitExceeded);
        }
        if source.default_bytes().len() != bpp {
            return Err(RasterInputError::LengthMismatch);
        }
        for (_, tile) in source.tiles() {
            poll(control)?;
            if tile.bytes().len() as u64 != tile_bytes {
                return Err(RasterInputError::LengthMismatch);
            }
        }
        source.try_validate_tile_origins_with(control)?;
        let output_retained_bytes = output_tiles.checked_mul(tile_bytes).and_then(|n| n.checked_add(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
        let output_index_bytes =
            source_tiles.checked_mul(256).and_then(|n| n.checked_add(output_tiles.checked_mul(1024)?)).ok_or(RasterInputError::LimitExceeded)?;
        let scratch_bytes = if roi.is_empty() { 0 } else { tile_bytes.checked_add(tile_bytes / 16).ok_or(RasterInputError::LimitExceeded)? };
        let peak_resident_bytes = source_retained_bytes
            .checked_add(self.retained)
            .and_then(|n| n.checked_add(output_retained_bytes))
            .and_then(|n| n.checked_add(output_index_bytes))
            .and_then(|n| n.checked_add(scratch_bytes))
            .ok_or(RasterInputError::LimitExceeded)?;
        let work_units = (length as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(((length / bpp) as u64).checked_mul(self.lookup_work)?))
            .and_then(|n| n.checked_add(output_tiles.checked_mul(tile_bytes)?.checked_mul(8)?))
            .and_then(|n| n.checked_add(source_tiles.checked_mul(4)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        poll(control)?;
        Ok(Requirements {
            source_retained_bytes,
            parameter_retained_bytes: self.retained,
            output_retained_bytes,
            output_index_bytes,
            scratch_bytes,
            peak_resident_bytes,
            work_units,
        })
    }
    pub fn retained_table_bytes(&self) -> u64 {
        self.retained
    }
    pub fn validation_work(&self) -> u64 {
        self.validation_work
    }
    /// Apply only ROI; exact default/outside/unselected channels (including alpha)
    /// remain unchanged. Float sparse tables match finite native bits, not neighbors.
    /// This is an explicit operation call, never per-frame implicit validation.
    pub fn apply(&self, source: &Surface, roi: Rect, current: WorkingContext, budget: LutLimits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
        poll(control)?;
        limits(budget)?;
        if current != self.context || source.format() != self.context.format {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let format = source.format();
        let bpp = format.bytes_per_pixel();
        let length = checked_region_byte_len(roi, format)?;
        let tiles = tile_count(roi)?;
        let source_tiles = source.tile_count() as u64;
        let tile_bytes = 65536u64.checked_mul(bpp as u64).ok_or(RasterInputError::LimitExceeded)?;
        let pixels = (length / bpp) as u64;
        let work = (length as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(pixels.checked_mul(self.lookup_work)?))
            .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes)?.checked_mul(8)?))
            .and_then(|n| n.checked_add(source_tiles.checked_mul(4)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        let source_lower =
            source_tiles.checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        if work > budget.work_units || source_lower > budget.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        if source.default_bytes().len() != bpp {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut retained = source.default_bytes().len() as u64;
        for (_, tile) in source.tiles() {
            poll(control)?;
            if tile.bytes().len() as u64 != tile_bytes {
                return Err(RasterInputError::LengthMismatch);
            }
            retained = retained.checked_add(tile.bytes().len() as u64).and_then(|n| n.checked_add(256)).ok_or(RasterInputError::LimitExceeded)?;
        }
        source.try_validate_tile_origins_with(control)?;
        let maps = source_tiles.checked_mul(256).and_then(|n| n.checked_add(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
        // Every touched source-grid tile is staged once, including its unchanged border.
        // Reserve all retained replacements/map nodes up front; scratch is tile bounded.
        let staging = tiles.checked_mul(tile_bytes.checked_add(1024).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        let base = retained
            .checked_add(self.retained)
            .and_then(|n| n.checked_add(maps))
            .and_then(|n| n.checked_add(staging))
            .ok_or(RasterInputError::LimitExceeded)?;
        if base > budget.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        if roi.is_empty() {
            poll(control)?;
            let result = source.clone();
            poll(control)?;
            return Ok(result);
        }
        let cell_bytes = tile_bytes / 16;
        if base.checked_add(tile_bytes).and_then(|n| n.checked_add(cell_bytes)).is_none_or(|n| n > budget.resident_bytes) {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut raw = scratch(tile_bytes, control)?;
        let mut cell = scratch(cell_bytes, control)?;
        if base.checked_add(raw.capacity() as u64).and_then(|n| n.checked_add(cell.capacity() as u64)).is_none_or(|n| n > budget.resident_bytes) {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut prepaid = Prepaid { bytes: staging, work: budget.work_units };
        prepaid.work(source_tiles.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?)?;
        poll(control)?;
        // One sparse-map COW clone; per-tile insertion never clones the whole map.
        // Existing std map/Arc allocations are bounded/prepaid, not fallible allocator proof.
        let mut result = source.clone();
        for coordinate in roi.tiles() {
            poll(control)?;
            let x0 = coordinate.tx.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
            let y0 = coordinate.ty.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
            let bounds = Rect::new(
                x0,
                y0,
                x0.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?,
                y0.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?,
            );
            checked_region_byte_len(bounds, format)?;
            prepaid.work(tile_bytes.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?)?;
            source.try_read_interleaved_into_with(bounds, &mut raw, control)?;
            let part = bounds.intersect(&roi);
            let mut encoded = [0u8; 8];
            for y in part.y0..part.y1 {
                poll(control)?;
                prepaid.work((part.width() as u64).checked_mul(self.lookup_work).ok_or(RasterInputError::LimitExceeded)?)?;
                for x in part.x0..part.x1 {
                    let row = usize::try_from(i64::from(y) - i64::from(y0)).map_err(|_| RasterInputError::InvalidBounds)?;
                    let col = usize::try_from(i64::from(x) - i64::from(x0)).map_err(|_| RasterInputError::InvalidBounds)?;
                    let start = row
                        .checked_mul(TILE_SIZE as usize)
                        .and_then(|n| n.checked_add(col))
                        .and_then(|n| n.checked_mul(bpp))
                        .ok_or(RasterInputError::LengthMismatch)?;
                    let native = raw.get_mut(start..start.checked_add(bpp).ok_or(RasterInputError::LengthMismatch)?).ok_or(RasterInputError::LengthMismatch)?;
                    for selected in self.tables {
                        let start = selected.channel.checked_mul(format.sample.bytes()).ok_or(RasterInputError::LengthMismatch)?;
                        let end = start.checked_add(format.sample.bytes()).ok_or(RasterInputError::LengthMismatch)?;
                        let output = encoded.get_mut(..format.sample.bytes()).ok_or(RasterInputError::LengthMismatch)?;
                        selected.table.map(native.get(start..end).ok_or(RasterInputError::LengthMismatch)?, output)?;
                        native.get_mut(start..end).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(output);
                    }
                }
            }
            let mut builder = StagedSurfaceBuilder::new(format, source.default_bytes(), bounds, &mut prepaid, control)?;
            for cy in (0..TILE_SIZE).step_by(64) {
                for cx in (0..TILE_SIZE).step_by(64) {
                    poll(control)?;
                    let row_bytes = 64usize.checked_mul(bpp).ok_or(RasterInputError::LimitExceeded)?;
                    for dy in 0..64usize {
                        poll(control)?;
                        let src = (cy as usize)
                            .checked_add(dy)
                            .and_then(|n| n.checked_mul(TILE_SIZE as usize))
                            .and_then(|n| n.checked_add(cx as usize))
                            .and_then(|n| n.checked_mul(bpp))
                            .ok_or(RasterInputError::LimitExceeded)?;
                        let dst = dy.checked_mul(row_bytes).ok_or(RasterInputError::LimitExceeded)?;
                        cell.get_mut(dst..dst.checked_add(row_bytes).ok_or(RasterInputError::LimitExceeded)?)
                            .ok_or(RasterInputError::LengthMismatch)?
                            .copy_from_slice(
                                raw.get(src..src.checked_add(row_bytes).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?,
                            );
                    }
                    builder.push_region(Rect::new(x0 + cx, y0 + cy, x0 + cx + 64, y0 + cy + 64), &cell, &mut prepaid, control)?;
                }
            }
            let (replacement, ()) = builder.finalize(&mut prepaid, control)?.into_parts();
            poll(control)?;
            result.put_tiles(replacement.tiles().map(|(coordinate, tile)| (*coordinate, Arc::clone(tile))));
        }
        poll(control)?;
        Ok(result)
    }
}
fn tile_count(rect: Rect) -> Result<u64, RasterInputError> {
    if rect.is_empty() {
        return Ok(0);
    }
    let last_x = rect.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let last_y = rect.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let span = |first: i32, last: i32| {
        u64::try_from(i64::from(last.div_euclid(TILE_SIZE)) - i64::from(first.div_euclid(TILE_SIZE)) + 1).map_err(|_| RasterInputError::InvalidBounds)
    };
    span(rect.x0, last_x)?.checked_mul(span(rect.y0, last_y)?).ok_or(RasterInputError::LimitExceeded)
}

// Unit leases do not refund: aggregate retained output admission is owned by this call.
struct Prepaid {
    bytes: u64,
    work: u64,
}
impl StagingBudget for Prepaid {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> Result<(), RasterInputError> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> Result<(), RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn scratch(bytes: u64, ctl: &Interrupt<'_>) -> Result<Vec<u8>, RasterInputError> {
    poll(ctl)?;
    let len = usize::try_from(bytes).map_err(|_| RasterInputError::LimitExceeded)?;
    let mut output = Vec::new();
    output.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
    poll(ctl)?;
    if output.capacity() != len {
        return Err(RasterInputError::AllocationFailed);
    }
    while output.len() < len {
        poll(ctl)?;
        let next = output.len().checked_add(32768).ok_or(RasterInputError::LimitExceeded)?.min(len);
        output.resize(next, 0);
    }
    poll(ctl)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(format: PixelFormat) -> WorkingContext {
        WorkingContext { format, transfer: TransferMarker::Nonlinear, profile_marker: [7; 32] }
    }
    fn budget() -> LutLimits {
        LutLimits { resident_bytes: 128 << 20, work_units: MAX_WORK }
    }
    #[test]
    fn dense_u16_preserves_unselected_alpha_default_and_boundary_tile() {
        let format = PixelFormat::RGBA16;
        let original: Vec<u8> = [1u16, 2, 3, 0x1234].into_iter().flat_map(u16::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(format, &original).unwrap();
        let mut table: Vec<u16> = (0..=u16::MAX).collect();
        table[1] = 65534;
        let selectors = [ChannelTable { channel: 0, table: NativeTable::DenseU16(&table) }];
        let lut = PreparedNativeLut::prepare(context(format), &selectors, LookupPolicy::ExactCodesOnly, budget(), &Interrupt::NONE).unwrap();
        let output = lut.apply(&source, Rect::new(-1, -1, 1, 1), context(format), budget(), &Interrupt::NONE).unwrap();
        let mut actual = [0u8; 8];
        output.try_read_pixel_bytes(-1, -1, &mut actual).unwrap();
        let mut expected = original.clone();
        expected[..2].copy_from_slice(&65534u16.to_ne_bytes());
        assert_eq!(actual.as_slice(), expected);
        output.try_read_pixel_bytes(-2, -1, &mut actual).unwrap();
        assert_eq!(actual.as_slice(), original);
        assert_eq!(output.default_bytes(), source.default_bytes());
        assert_eq!(source.tile_count(), 0);
    }
    #[test]
    fn exact_u32_missing_key_and_context_refuse_without_source_mutation() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let source = Surface::try_with_default_bytes(format, &0x10000001u32.to_ne_bytes()).unwrap();
        let table = vec![(0x10000000, 1)];
        let selectors = [ChannelTable { channel: 0, table: NativeTable::ExactU32(&table) }];
        let lut = PreparedNativeLut::prepare(context(format), &selectors, LookupPolicy::ExactCodesOnly, budget(), &Interrupt::NONE).unwrap();
        assert!(matches!(lut.apply(&source, Rect::new(0, 0, 1, 1), context(format), budget(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
        let mut changed = context(format);
        changed.profile_marker[0] = 8;
        assert!(lut.apply(&source, Rect::new(0, 0, 1, 1), changed, budget(), &Interrupt::NONE).is_err());
        assert_eq!(source.default_bytes(), &0x10000001u32.to_ne_bytes());
        assert_eq!(source.tile_count(), 0);
    }
    #[test]
    fn exact_f64_preserves_low_bits_signed_zero_and_unselected_nan_alpha() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let key = 1.0f64.to_bits() + 1;
        let alpha = 0x7ff8000000000012u64;
        let original: Vec<u8> = [key, alpha].into_iter().flat_map(u64::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(format, &original).unwrap();
        let table = vec![(key, (-0.0f64).to_bits())];
        let selectors = [ChannelTable { channel: 0, table: NativeTable::ExactF64Bits(&table) }];
        let lut = PreparedNativeLut::prepare(context(format), &selectors, LookupPolicy::ExactCodesOnly, budget(), &Interrupt::NONE).unwrap();
        let output = lut.apply(&source, Rect::new(0, 0, 1, 1), context(format), budget(), &Interrupt::NONE).unwrap();
        let mut actual = [0u8; 16];
        output.try_read_pixel_bytes(0, 0, &mut actual).unwrap();
        assert_eq!(&actual[..8], &(-0.0f64).to_ne_bytes());
        assert_eq!(&actual[8..], &alpha.to_ne_bytes());
        assert_eq!(source.default_bytes(), original);
        let cancelled = || true;
        assert!(matches!(
            lut.apply(&source, Rect::new(0, 0, 1, 1), context(format), budget(), &Interrupt::new(&cancelled, &|_| {})),
            Err(RasterInputError::Cancelled)
        ));
        assert!(
            lut.apply(&source, Rect::new(0, 0, 1, 100000), context(format), LutLimits { resident_bytes: 128 << 20, work_units: 1 }, &Interrupt::NONE).is_err()
        );
    }
}

#[cfg(test)]
mod requirements_tests {
    use super::*;
    #[test]
    fn requirements_are_borrowed_exact_preflight_and_smaller_work_refuses() {
        let format = PixelFormat::GRAY8;
        let context = WorkingContext { format, transfer: TransferMarker::Nonlinear, profile_marker: [1; 32] };
        let table: Vec<u8> = (0..=u8::MAX).collect();
        let channels = [ChannelTable { channel: 0, table: NativeTable::DenseU8(&table) }];
        let limits = LutLimits { resident_bytes: 64 << 20, work_units: MAX_WORK };
        let lut = PreparedNativeLut::prepare(context, &channels, LookupPolicy::ExactCodesOnly, limits, &Interrupt::NONE).unwrap();
        let source = Surface::try_with_default_bytes(format, &[17]).unwrap();
        let roi = Rect::new(-1, -1, 1, 1);
        let req = lut.requirements(&source, roi, context).unwrap();
        assert_eq!(req.parameter_retained_bytes, table.capacity() as u64);
        assert_eq!(
            req.peak_resident_bytes,
            req.source_retained_bytes + req.parameter_retained_bytes + req.output_retained_bytes + req.output_index_bytes + req.scratch_bytes
        );
        assert_eq!(lut.format(), format);
        assert_eq!(lut.context(), context);
        assert!(
            lut.apply(&source, roi, context, LutLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units - 1 }, &Interrupt::NONE).is_err()
        );
        let cancelled = || true;
        assert!(matches!(lut.requirements_with_interrupt(&source, roi, context, &Interrupt::cancel_only(&cancelled)), Err(RasterInputError::Cancelled)));
        assert_eq!(source.tile_count(), 0);
        assert_eq!(source.default_bytes(), &[17]);
    }
}
