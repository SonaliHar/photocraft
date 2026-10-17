//! Explicit numeric displacement followed by raw native nearest sampling.
//! No GEGL displacement convention, channel bias, normalization, CMS, source
//! precision/version admission or default orientation is inferred.
use super::{
    geometry_nearest::{self, Abyss, Affine, Limits, NativePixel, Params, Ties},
    wide_point::WorkingContext,
};
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    backing::{BackingError, ReadControl},
    checked_region_byte_len,
    checked_view::{CheckedRasterView, RasterViewError},
    staged::{SOURCE_CELL_SIZE, StagedSurfaceBuilder, StagingBudget},
};

/// Field pixel `(x,y)` describes the output pixel center `(x+0.5,y+0.5)`.
/// The field is not resampled or transformed to a different grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldCoordinates {
    OutputPixelGrid,
}
/// Scaled vectors are added in source spatial pixel units, after base affine
/// evaluation and before nearest selection. No output-axis scaling is inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorUnits {
    SourcePixels,
}
#[derive(Clone, Copy, Debug)]
pub struct FieldParams {
    /// Explicit x/y channels; the same channel may intentionally drive both.
    /// Any present channel, including alpha, can be explicitly selected.
    pub channels: [usize; 2],
    pub scales: [f64; 2],
    /// Numeric vector used outside the field's own logical extent, before scale.
    pub outside: [f64; 2],
    pub coordinates: FieldCoordinates,
    pub units: VectorUnits,
    pub context: WorkingContext,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub field_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
/// Inclusive numeric bounds for the selected in-field channels, before scale.
#[derive(Clone, Copy, Debug)]
pub struct VectorBounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    View(RasterViewError),
    FieldContext,
    FieldFormat,
    NonFinite,
    Coordinate,
    VectorBounds,
    MissingPixels,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native nearest displacement: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<RasterInputError> for Error {
    fn from(error: RasterInputError) -> Self {
        Self::Raster(error)
    }
}
impl From<RasterViewError> for Error {
    fn from(error: RasterViewError) -> Self {
        if matches!(
            error,
            RasterViewError::Resident(RasterInputError::Cancelled)
                | RasterViewError::Control(BackingError::Cancelled)
                | RasterViewError::Backed(BackingError::Cancelled)
        ) {
            Self::Raster(RasterInputError::Cancelled)
        } else {
            Self::View(error)
        }
    }
}
impl From<geometry_nearest::Error> for Error {
    fn from(error: geometry_nearest::Error) -> Self {
        match error {
            geometry_nearest::Error::Raster(error) => Self::Raster(error),
            geometry_nearest::Error::View(error) => Self::from(error),
            geometry_nearest::Error::Coordinate => Self::Coordinate,
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn limit() -> Error {
    RasterInputError::LimitExceeded.into()
}
fn bounds() -> Error {
    RasterInputError::InvalidBounds.into()
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn finite(value: f64) -> Result<f64> {
    if value.is_finite() { Ok(value) } else { Err(Error::NonFinite) }
}
fn native(pixel: &NativePixel, bpp: usize) -> Result<&[u8]> {
    if usize::from(pixel.length) != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    pixel.bytes.get(..bpp).ok_or_else(|| RasterInputError::LengthMismatch.into())
}
fn valid_rect(rect: Rect, nonempty: bool) -> Result<()> {
    if rect.x1 < rect.x0 || rect.y1 < rect.y0 || (nonempty && rect.is_empty()) {
        return Err(bounds());
    }
    Ok(())
}
fn base(map: Affine, x: i32, y: i32) -> Result<(f64, f64)> {
    let x = f64::from(x) + 0.5;
    let y = f64::from(y) + 0.5;
    let xx = finite(map.xx * x)?;
    let xy = finite(map.xy * y)?;
    let sx0 = finite(xx + xy)?;
    let sx = finite(sx0 + map.tx)?;
    let yx = finite(map.yx * x)?;
    let yy = finite(map.yy * y)?;
    let sy0 = finite(yx + yy)?;
    let sy = finite(sy0 + map.ty)?;
    Ok((sx, sy))
}
fn nearest(coordinate: f64, ties: Ties) -> Result<i32> {
    let shifted = finite(coordinate - 0.5)?;
    let lower = shifted.floor();
    let fraction = finite(shifted - lower)?;
    let upper = fraction > 0.5
        || (fraction == 0.5
            && match ties {
                Ties::Lower => false,
                Ties::Upper => true,
                Ties::ToEven => lower.rem_euclid(2.0) != 0.0,
            });
    let selected = finite(if upper { lower + 1.0 } else { lower })?;
    if selected < f64::from(i32::MIN) || selected > f64::from(i32::MAX) {
        return Err(Error::Coordinate);
    }
    Ok(selected as i32)
}
fn field_vector(view: &CheckedRasterView<'_>, extent: Rect, x: i32, y: i32, params: FieldParams, read: &ReadControl<'_>) -> Result<[f64; 2]> {
    if !extent.contains(x, y) {
        return Ok(params.outside);
    }
    let format = view.format();
    let mut raw = [0; 32];
    let bytes = raw.get_mut(..format.bytes_per_pixel()).ok_or(Error::FieldFormat)?;
    view.read_logical_pixel_native_into(x, y, bytes, read)?;
    let dx = try_read_sample_f64(bytes, format.sample, params.channels[0]).map_err(|_| Error::FieldFormat)?;
    let dy = try_read_sample_f64(bytes, format.sample, params.channels[1]).map_err(|_| Error::FieldFormat)?;
    // Selected channels only: unselected channels/alpha are never interpreted.
    Ok([finite(dx)?, finite(dy)?])
}
fn tile_count(roi: Rect) -> Result<u64> {
    if roi.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last = TileCoord::containing(roi.x1.checked_sub(1).ok_or_else(bounds)?, roi.y1.checked_sub(1).ok_or_else(bounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(bounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(bounds)?;
    }
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| bounds())?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| bounds())?;
    width.checked_mul(height).ok_or_else(limit)
}
fn resident(surface: &Surface, cap: u64, control: &Interrupt<'_>) -> Result<u64> {
    let mut bytes = (surface.tile_count() as u64).checked_mul(512).and_then(|n| n.checked_add(surface.default_bytes().len() as u64)).ok_or_else(limit)?;
    if bytes > cap {
        return Err(limit());
    }
    for (index, (_, tile)) in surface.tiles().enumerate() {
        if index.is_multiple_of(256) {
            poll(control)?;
        }
        bytes = bytes.checked_add(tile.bytes().len() as u64).ok_or_else(limit)?;
        if bytes > cap {
            return Err(limit());
        }
    }
    poll(control)?;
    Ok(bytes)
}
struct Budget {
    resident: u64,
    work: u64,
}
impl StagingBudget for Budget {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), RasterInputError> {
        self.resident = self.resident.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn scratch(length: usize, control: &Interrupt<'_>) -> Result<Vec<u8>> {
    poll(control)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    if bytes.capacity() > length.checked_mul(2).ok_or_else(limit)? {
        return Err(limit());
    }
    while bytes.len() < length {
        poll(control)?;
        bytes.resize(bytes.len().checked_add(8192).ok_or_else(limit)?.min(length), 0);
    }
    Ok(bytes)
}
/// Includes both resident metadata walks, field decoding, explicit unfused
/// coordinate arithmetic, raw source reads and existing nearest staging charge.
pub fn required_work(source: &Surface, field: &Surface, roi: Rect) -> Result<u64> {
    valid_rect(roi, false)?;
    let pixels = u64::from(roi.width()).checked_mul(u64::from(roi.height())).ok_or_else(limit)?;
    geometry_nearest::required_work(source, roi)?
        .checked_add(pixels.checked_mul(96).ok_or_else(limit)?)
        .and_then(|n| n.checked_add((field.tile_count() as u64).checked_mul(8)?))
        .ok_or_else(limit)
}

/// Shared actual sampler preflight: format/context/geometry/native-default
/// validation and both cancellable retained-tile metadata censuses, no field or
/// source sample scans. Graph callers precharge known metadata work first.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn requirements_with_interrupt(
    source: &Surface,
    source_extent: Rect,
    field: &Surface,
    field_extent: Rect,
    roi: Rect,
    field_params: FieldParams,
    expected_field_context: WorkingContext,
    geometry: Params,
    control: &Interrupt<'_>,
) -> Result<Requirements> {
    poll(control)?;
    valid_rect(source_extent, true)?;
    valid_rect(field_extent, true)?;
    valid_rect(roi, false)?;
    if field_params.context != expected_field_context {
        return Err(Error::FieldContext);
    }
    match (field_params.coordinates, field_params.units) {
        (FieldCoordinates::OutputPixelGrid, VectorUnits::SourcePixels) => {}
    }
    let field_format = field.format();
    if !matches!(field_format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(field_format.sample, SampleType::F32 | SampleType::F64)
        || field_params.channels.iter().any(|channel| *channel >= field_format.channels())
    {
        return Err(Error::FieldFormat);
    }
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    checked_region_byte_len(source_extent, format)?;
    checked_region_byte_len(field_extent, field_format)?;
    checked_region_byte_len(roi, format)?;
    native(&geometry.output_default, bpp)?;
    if let Abyss::ConstantNative(pixel) = &geometry.abyss {
        native(pixel, bpp)?;
    }
    for value in [
        geometry.map.xx,
        geometry.map.xy,
        geometry.map.yx,
        geometry.map.yy,
        geometry.map.tx,
        geometry.map.ty,
        field_params.scales[0],
        field_params.scales[1],
        field_params.outside[0],
        field_params.outside[1],
    ] {
        finite(value)?;
    }
    if !roi.is_empty() {
        let last_x = roi.x1.checked_sub(1).ok_or_else(bounds)?;
        let last_y = roi.y1.checked_sub(1).ok_or_else(bounds)?;
        for (x, y) in [(roi.x0, roi.y0), (last_x, roi.y0), (roi.x0, last_y), (last_x, last_y)] {
            base(geometry.map, x, y)?;
        }
    }
    let work_units = required_work(source, field, roi)?;
    let source_retained_bytes = resident(source, MAX_CHECKED_RASTER_BYTES, control)?;
    let field_retained_bytes = resident(field, MAX_CHECKED_RASTER_BYTES.checked_sub(source_retained_bytes).ok_or_else(limit)?, control)?;
    source.try_validate_tile_origins_with(control)?;
    field.try_validate_tile_origins_with(control)?;
    let tiles = tile_count(roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let output_retained_bytes = tiles.checked_mul(tile_bytes).and_then(|n| n.checked_add(bpp as u64)).and_then(|n| n.checked_add(256)).ok_or_else(limit)?;
    let output_index_bytes = tiles.checked_mul(512).ok_or_else(limit)?;
    let scratch_bytes = if roi.is_empty() {
        0
    } else {
        (SOURCE_CELL_SIZE as u64)
            .checked_mul(SOURCE_CELL_SIZE as u64)
            .and_then(|n| n.checked_mul(bpp as u64))
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(limit)?
    };
    let peak_resident_bytes = source_retained_bytes
        .checked_add(field_retained_bytes)
        .and_then(|n| n.checked_add(output_retained_bytes))
        .and_then(|n| n.checked_add(output_index_bytes))
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or_else(limit)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(limit());
    }
    poll(control)?;
    Ok(Requirements { source_retained_bytes, field_retained_bytes, output_retained_bytes, output_index_bytes, scratch_bytes, peak_resident_bytes, work_units })
}

/// Creates private same-source-format output, defaulted outside `roi`. Field
/// channels are numeric native F32/F64 values, widened without f32 intermediates;
/// source samples are never decoded, including NaN, HDR, signed zero and alpha.
/// Per axis: base affine multiply/add stages, then field * scale, then + base,
/// all separate unfused binary64 operations, then explicit nearest/tie policy.
/// Field context equality asserts caller-established interpretation, not CMS.
/// The source and field remain unchanged on every refusal/cancellation path.
/// Staged builder capacities are fallibly reserved; its bounded map/Arc standard
/// library publication is prepaid, not a complete allocator/RSS guarantee.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn try_displace(
    source: &Surface,
    source_extent: Rect,
    field: &Surface,
    field_extent: Rect,
    roi: Rect,
    field_params: FieldParams,
    expected_field_context: WorkingContext,
    geometry: Params,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    apply(source, source_extent, field, field_extent, roi, field_params, expected_field_context, geometry, None, limits, control)
}
/// Graph-facing variant verifies the caller's ROI planning range at every
/// actually sampled in-field vector before reading a source pixel. A false
/// range refuses atomically instead of accepting missing upstream coverage as
/// a sparse default. Outside vectors remain independently explicit.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn try_displace_with_vector_bounds(
    source: &Surface,
    source_extent: Rect,
    field: &Surface,
    field_extent: Rect,
    roi: Rect,
    field_params: FieldParams,
    expected_field_context: WorkingContext,
    geometry: Params,
    bounds: VectorBounds,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    apply(source, source_extent, field, field_extent, roi, field_params, expected_field_context, geometry, Some(bounds), limits, control)
}
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
fn apply(
    source: &Surface,
    source_extent: Rect,
    field: &Surface,
    field_extent: Rect,
    roi: Rect,
    field_params: FieldParams,
    expected_field_context: WorkingContext,
    geometry: Params,
    vector_bounds: Option<VectorBounds>,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    poll(control)?;
    if let Some(bounds) = vector_bounds {
        for axis in 0..2 {
            if !bounds.min[axis].is_finite() || !bounds.max[axis].is_finite() || bounds.min[axis] > bounds.max[axis] {
                return Err(Error::VectorBounds);
            }
        }
    }
    let work = required_work(source, field, roi)?;
    if work > limits.work_units {
        return Err(limit());
    }
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    let map_floor = (source.tile_count() as u64).checked_add(field.tile_count() as u64).and_then(|n| n.checked_mul(512)).ok_or_else(limit)?;
    if map_floor > cap {
        return Err(limit());
    }
    let requirements = requirements_with_interrupt(source, source_extent, field, field_extent, roi, field_params, expected_field_context, geometry, control)?;
    if requirements.peak_resident_bytes > cap {
        return Err(limit());
    }
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let output_default = native(&geometry.output_default, bpp)?;
    let overhead = requirements
        .source_retained_bytes
        .checked_add(requirements.field_retained_bytes)
        .and_then(|n| n.checked_add(requirements.scratch_bytes))
        .and_then(|n| n.checked_add(256))
        .ok_or_else(limit)?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let source_view = CheckedRasterView::resident_with_control(source, source_extent, &read)?;
    let field_view = CheckedRasterView::resident_with_control(field, field_extent, &read)?;
    if roi.is_empty() {
        poll(control)?;
        return Ok(Surface::try_with_default_bytes(format, output_default)?);
    }
    let pixels = u64::from(roi.width()).checked_mul(u64::from(roi.height())).ok_or_else(limit)?;
    let metadata_tiles = (source.tile_count() as u64).checked_add(field.tile_count() as u64).ok_or_else(limit)?;
    let map_read_work = pixels.checked_mul(160).and_then(|n| n.checked_add(metadata_tiles.checked_mul(8)?)).ok_or_else(limit)?;
    let mut budget = Budget { resident: cap.checked_sub(overhead).ok_or_else(limit)?, work: limits.work_units.checked_sub(map_read_work).ok_or_else(limit)? };
    let mut output = StagedSurfaceBuilder::new(format, output_default, roi, &mut budget, control)?;
    let mut cell_y = roi.y0;
    while cell_y < roi.y1 {
        let end_y = cell_y.checked_add(SOURCE_CELL_SIZE).ok_or_else(bounds)?.min(roi.y1);
        let mut cell_x = roi.x0;
        while cell_x < roi.x1 {
            poll(control)?;
            let end_x = cell_x.checked_add(SOURCE_CELL_SIZE).ok_or_else(bounds)?.min(roi.x1);
            let part = Rect::new(cell_x, cell_y, end_x, end_y);
            let length = checked_region_byte_len(part, format)?;
            budget.work(length as u64)?;
            let mut encoded = scratch(length, control)?;
            let width = usize::try_from(part.width()).map_err(|_| limit())?;
            for (index, pixel) in encoded.chunks_exact_mut(bpp).enumerate() {
                if index.is_multiple_of(256) {
                    poll(control)?;
                }
                let row = index.checked_div(width).ok_or_else(bounds)?;
                let column = index.checked_rem(width).ok_or_else(bounds)?;
                let x = i32::try_from(i64::from(part.x0) + i64::try_from(column).map_err(|_| bounds())?).map_err(|_| bounds())?;
                let y = i32::try_from(i64::from(part.y0) + i64::try_from(row).map_err(|_| bounds())?).map_err(|_| bounds())?;
                let vector = field_vector(&field_view, field_extent, x, y, field_params, &read)?;
                match vector_bounds {
                    Some(bounds)
                        if field_extent.contains(x, y)
                            && vector.iter().enumerate().any(|(axis, value)| *value < bounds.min[axis] || *value > bounds.max[axis]) =>
                    {
                        return Err(Error::VectorBounds);
                    }
                    _ => {}
                }
                let (base_x, base_y) = base(geometry.map, x, y)?;
                let offset_x = finite(vector[0] * field_params.scales[0])?;
                let offset_y = finite(vector[1] * field_params.scales[1])?;
                let sx = nearest(finite(base_x + offset_x)?, geometry.ties)?;
                let sy = nearest(finite(base_y + offset_y)?, geometry.ties)?;
                match &geometry.abyss {
                    Abyss::ConstantNative(constant) if !source_extent.contains(sx, sy) => {
                        pixel.copy_from_slice(native(constant, bpp)?);
                        continue;
                    }
                    _ => {}
                }
                let sx = sx.clamp(source_extent.x0, source_extent.x1.checked_sub(1).ok_or_else(bounds)?);
                let sy = sy.clamp(source_extent.y0, source_extent.y1.checked_sub(1).ok_or_else(bounds)?);
                source_view.read_logical_pixel_native_into(sx, sy, pixel, &read)?;
            }
            poll(control)?;
            output.push_region(part, &encoded, &mut budget, control)?;
            cell_x = end_x;
        }
        cell_y = end_y;
    }
    poll(control)?;
    let (surface, ()) = output.finalize(&mut budget, control)?.into_parts();
    poll(control)?;
    Ok(surface)
}

// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
fn availability_plan(
    source: Option<&Surface>,
    source_metadata: super::geometry_sampling_roi::SourceMetadata,
    field: Option<&Surface>,
    field_metadata: super::geometry_sampling_roi::FieldMetadata,
    roi: Rect,
    field_params: FieldParams,
    expected_context: WorkingContext,
    geometry: Params,
    bounds: VectorBounds,
    control: &Interrupt<'_>,
) -> Result<()> {
    use super::geometry_sampling_roi::{self as planning, ProvenVectorBounds, SourceRequirement, UncertaintyPolicy};
    if source.is_some_and(|surface| surface.format() != source_metadata.format) || field.is_some_and(|surface| surface.format() != field_metadata.format) {
        return Err(Error::FieldFormat);
    }
    let plan = planning::plan_displacement(
        source_metadata,
        field_metadata,
        roi,
        geometry,
        field_params,
        expected_context,
        ProvenVectorBounds { min: bounds.min, max: bounds.max },
        UncertaintyPolicy::Reject,
        planning::Limits { work_units: 128 },
        control,
    )
    .map_err(|error| match error {
        planning::Error::Raster(error) => Error::Raster(error),
        planning::Error::FieldContext => Error::FieldContext,
        planning::Error::FieldFormat => Error::FieldFormat,
        planning::Error::VectorBounds => Error::VectorBounds,
        _ => Error::Coordinate,
    })?;
    if source.is_none() && plan.source != SourceRequirement::None {
        return Err(Error::MissingPixels);
    }
    if field.is_none() && plan.field.is_some() {
        return Err(Error::MissingPixels);
    }
    Ok(())
}
fn availability_work(source: Option<&Surface>, field: Option<&Surface>, format: photocraft_color::PixelFormat, roi: Rect) -> Result<u64> {
    let bytes = checked_region_byte_len(roi, format)? as u64;
    let pixels = u64::from(roi.width()).checked_mul(u64::from(roi.height())).ok_or_else(limit)?;
    let metadata = (source.map_or(0, Surface::tile_count) as u64).checked_add(field.map_or(0, Surface::tile_count) as u64).ok_or_else(limit)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or_else(limit)?;
    bytes
        .checked_mul(8)
        .and_then(|n| n.checked_add(pixels.checked_mul(160)?))
        .and_then(|n| n.checked_add(metadata.checked_mul(8)?))
        .and_then(|n| n.checked_add(tile_count(roi).ok()?.checked_mul(tile_bytes.checked_add(512)?)?))
        .and_then(|n| n.checked_add(256))
        .ok_or_else(limit)
}
/// Optional pixel owners are admitted only by an independently recomputed
/// no-read plan. No zero/default sentinel or upstream materialization occurs.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn requirements_available_with_interrupt(
    source: Option<&Surface>,
    source_metadata: super::geometry_sampling_roi::SourceMetadata,
    field: Option<&Surface>,
    field_metadata: super::geometry_sampling_roi::FieldMetadata,
    roi: Rect,
    field_params: FieldParams,
    expected_context: WorkingContext,
    geometry: Params,
    bounds: VectorBounds,
    control: &Interrupt<'_>,
) -> Result<Requirements> {
    poll(control)?;
    availability_plan(source, source_metadata, field, field_metadata, roi, field_params, expected_context, geometry, bounds, control)?;
    let source_retained_bytes = match source {
        Some(surface) => {
            surface.try_validate_tile_origins_with(control)?;
            resident(surface, MAX_CHECKED_RASTER_BYTES, control)?
        }
        None => 0,
    };
    let field_retained_bytes = match field {
        Some(surface) => {
            surface.try_validate_tile_origins_with(control)?;
            resident(surface, MAX_CHECKED_RASTER_BYTES.checked_sub(source_retained_bytes).ok_or_else(limit)?, control)?
        }
        None => 0,
    };
    let tiles = tile_count(roi)?;
    let bpp = source_metadata.format.bytes_per_pixel() as u64;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp)).ok_or_else(limit)?;
    let output_retained_bytes = tiles.checked_mul(tile_bytes).and_then(|n| n.checked_add(bpp)).and_then(|n| n.checked_add(256)).ok_or_else(limit)?;
    let output_index_bytes = tiles.checked_mul(512).ok_or_else(limit)?;
    let scratch_bytes = if roi.is_empty() {
        0
    } else {
        (SOURCE_CELL_SIZE as u64).checked_mul(SOURCE_CELL_SIZE as u64).and_then(|n| n.checked_mul(bpp)).and_then(|n| n.checked_mul(2)).ok_or_else(limit)?
    };
    let peak_resident_bytes = source_retained_bytes
        .checked_add(field_retained_bytes)
        .and_then(|n| n.checked_add(output_retained_bytes))
        .and_then(|n| n.checked_add(output_index_bytes))
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or_else(limit)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(limit());
    }
    let work_units = availability_work(source, field, source_metadata.format, roi)?;
    poll(control)?;
    Ok(Requirements { source_retained_bytes, field_retained_bytes, output_retained_bytes, output_index_bytes, scratch_bytes, peak_resident_bytes, work_units })
}
/// No-read execution still validates every actual needed field vector, range,
/// unfused finite coordinate stage and abyss decision. A field outside vector is
/// used only outside its extent; missing source pixels never become a default.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn try_displace_available(
    source: Option<&Surface>,
    source_metadata: super::geometry_sampling_roi::SourceMetadata,
    field: Option<&Surface>,
    field_metadata: super::geometry_sampling_roi::FieldMetadata,
    roi: Rect,
    field_params: FieldParams,
    expected_context: WorkingContext,
    geometry: Params,
    range: VectorBounds,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    poll(control)?;
    let work = availability_work(source, field, source_metadata.format, roi)?;
    if work > limits.work_units {
        return Err(limit());
    }
    let requirements =
        requirements_available_with_interrupt(source, source_metadata, field, field_metadata, roi, field_params, expected_context, geometry, range, control)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if requirements.peak_resident_bytes > cap {
        return Err(limit());
    }
    let format = source_metadata.format;
    let bpp = format.bytes_per_pixel();
    let default = native(&geometry.output_default, bpp)?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let source_view = source.map(|surface| CheckedRasterView::resident_with_control(surface, source_metadata.extent, &read)).transpose()?;
    let field_view = field.map(|surface| CheckedRasterView::resident_with_control(surface, field_metadata.extent, &read)).transpose()?;
    if roi.is_empty() {
        poll(control)?;
        return Ok(Surface::try_with_default_bytes(format, default)?);
    }
    let overhead = requirements
        .source_retained_bytes
        .checked_add(requirements.field_retained_bytes)
        .and_then(|n| n.checked_add(requirements.scratch_bytes))
        .and_then(|n| n.checked_add(256))
        .ok_or_else(limit)?;
    let metadata = (source.map_or(0, Surface::tile_count) as u64).checked_add(field.map_or(0, Surface::tile_count) as u64).ok_or_else(limit)?;
    let map_work = u64::from(roi.width())
        .checked_mul(u64::from(roi.height()))
        .and_then(|n| n.checked_mul(160))
        .and_then(|n| n.checked_add(metadata.checked_mul(8)?))
        .and_then(|n| n.checked_add(256))
        .ok_or_else(limit)?;
    let mut budget = Budget { resident: cap.checked_sub(overhead).ok_or_else(limit)?, work: limits.work_units.checked_sub(map_work).ok_or_else(limit)? };
    let mut output = StagedSurfaceBuilder::new(format, default, roi, &mut budget, control)?;
    let mut cell_y = roi.y0;
    while cell_y < roi.y1 {
        let end_y = cell_y.checked_add(SOURCE_CELL_SIZE).ok_or_else(bounds)?.min(roi.y1);
        let mut cell_x = roi.x0;
        while cell_x < roi.x1 {
            poll(control)?;
            let end_x = cell_x.checked_add(SOURCE_CELL_SIZE).ok_or_else(bounds)?.min(roi.x1);
            let part = Rect::new(cell_x, cell_y, end_x, end_y);
            let length = checked_region_byte_len(part, format)?;
            let width = part.width() as usize;
            budget.work(length as u64)?;
            let mut encoded = scratch(length, control)?;
            for (index, pixel) in encoded.chunks_exact_mut(bpp).enumerate() {
                if index.is_multiple_of(256) {
                    poll(control)?;
                }
                let x = i32::try_from(i64::from(part.x0) + i64::try_from(index.checked_rem(width).ok_or_else(bounds)?).map_err(|_| bounds())?)
                    .map_err(|_| bounds())?;
                let y = i32::try_from(i64::from(part.y0) + i64::try_from(index.checked_div(width).ok_or_else(bounds)?).map_err(|_| bounds())?)
                    .map_err(|_| bounds())?;
                let vector = if field_metadata.extent.contains(x, y) {
                    let view = field_view.as_ref().ok_or(Error::MissingPixels)?;
                    let vector = field_vector(view, field_metadata.extent, x, y, field_params, &read)?;
                    if vector.iter().enumerate().any(|(axis, value)| *value < range.min[axis] || *value > range.max[axis]) {
                        return Err(Error::VectorBounds);
                    }
                    vector
                } else {
                    field_params.outside
                };
                let (bx, by) = base(geometry.map, x, y)?;
                let dx = finite(vector[0] * field_params.scales[0])?;
                let dy = finite(vector[1] * field_params.scales[1])?;
                let sx = nearest(finite(bx + dx)?, geometry.ties)?;
                let sy = nearest(finite(by + dy)?, geometry.ties)?;
                match &geometry.abyss {
                    Abyss::ConstantNative(constant) if !source_metadata.extent.contains(sx, sy) => {
                        pixel.copy_from_slice(native(constant, bpp)?);
                        continue;
                    }
                    _ => {}
                }
                let view = source_view.as_ref().ok_or(Error::MissingPixels)?;
                let sx = sx.clamp(source_metadata.extent.x0, source_metadata.extent.x1.checked_sub(1).ok_or_else(bounds)?);
                let sy = sy.clamp(source_metadata.extent.y0, source_metadata.extent.y1.checked_sub(1).ok_or_else(bounds)?);
                view.read_logical_pixel_native_into(sx, sy, pixel, &read)?;
            }
            poll(control)?;
            output.push_region(part, &encoded, &mut budget, control)?;
            cell_x = end_x;
        }
        cell_y = end_y;
    }
    poll(control)?;
    let (surface, ()) = output.finalize(&mut budget, control)?.into_parts();
    poll(control)?;
    Ok(surface)
}

// Unrun synthetic geometry/bit-level drafts; no stock/source admission claim.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::SampleEncoding;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [37; 32] }
    }
    fn field_params() -> FieldParams {
        FieldParams {
            channels: [0, 0],
            scales: [1.0, 0.0],
            outside: [0.0, 0.0],
            coordinates: FieldCoordinates::OutputPixelGrid,
            units: VectorUnits::SourcePixels,
            context: context(),
        }
    }
    fn geometry(format: PixelFormat) -> Params {
        Params {
            map: Affine { xx: 1.0, xy: 0.0, yx: 0.0, yy: 1.0, tx: 0.0, ty: 0.0 },
            ties: Ties::Lower,
            abyss: Abyss::Clamp,
            output_default: NativePixel::from_slice(&vec![0; format.bytes_per_pixel()]).unwrap(),
        }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 40 }
    }
    fn field(depth: SampleType) -> Surface {
        Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false))
    }
    fn raw(surface: &Surface, x: i32, y: i32) -> Vec<u8> {
        surface.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap()
    }
    #[test]
    fn zero_field_agrees_with_nearest_and_preserves_all_raw_models_depths() {
        let extent = Rect::new(-2, -1, 1, 1);
        for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in SampleType::ALL {
                let format = PixelFormat::new(mode, depth, true);
                let mut source = Surface::new(format);
                let bytes = (0..6 * format.bytes_per_pixel()).map(|n| (n * 19 + 7) as u8).collect::<Vec<_>>();
                source.try_write_interleaved(extent, &bytes).unwrap();
                let g = geometry(format);
                let vector = field(SampleType::F64);
                let output = try_displace(&source, extent, &vector, extent, extent, field_params(), context(), g, limits(), &Interrupt::NONE).unwrap();
                let nearest_output = geometry_nearest::try_map(&source, extent, extent, g, limits(), &Interrupt::NONE).unwrap();
                assert_eq!(output, nearest_output);
                assert_eq!(output.try_read_interleaved(extent).unwrap(), bytes);
            }
        }
    }
    #[test]
    fn signed_field_samples_and_explicit_channel_order_select_exact_u32_pixels() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let mut source = Surface::new(format);
        let extent = Rect::new(-2, -1, 2, 1);
        let bytes = (16_777_216u32..16_777_224).flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved(extent, &bytes).unwrap();
        for depth in [SampleType::F32, SampleType::F64] {
            let mut vectors = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, false));
            let field_bytes = if depth == SampleType::F32 {
                [0.0f32, -1.0, 0.0, 0.0, 1.0, -1.0].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>()
            } else {
                [0.0f64, -1.0, 0.0, 0.0, 1.0, -1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()
            };
            let roi = Rect::new(-1, 0, 1, 1);
            vectors.try_write_interleaved(roi, &field_bytes).unwrap();
            let mut p = field_params();
            p.channels = [1, 2];
            p.scales = [1.0, 1.0];
            let output = try_displace(&source, extent, &vectors, roi, roi, p, context(), geometry(format), limits(), &Interrupt::NONE).unwrap();
            assert_eq!(raw(&output, -1, 0), raw(&source, -2, 0));
            assert_eq!(raw(&output, 0, 0), raw(&source, 1, -1));
        }
    }
    #[test]
    fn field_outside_vector_is_explicit_and_source_abyss_remains_raw() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut source = Surface::new(format);
        let extent = Rect::new(-1, 0, 1, 1);
        let bytes = [0x8000000000000000u64, 0x7ff8000000000019].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved(extent, &bytes).unwrap();
        let vectors = field(SampleType::F64);
        let mut p = field_params();
        p.outside = [-1.0, 0.0];
        let mut g = geometry(format);
        let roi = Rect::new(0, 0, 1, 1);
        let field_extent = Rect::new(10, 10, 11, 11);
        let output = try_displace(&source, extent, &vectors, field_extent, roi, p, context(), g, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(raw(&output, 0, 0), raw(&source, -1, 0));
        p.outside = [100.0, 0.0];
        g.abyss = Abyss::ConstantNative(NativePixel::from_slice(&0x7ff0000000000000u64.to_ne_bytes()).unwrap());
        let output = try_displace(&source, extent, &vectors, field_extent, roi, p, context(), g, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(raw(&output, 0, 0), 0x7ff0000000000000u64.to_ne_bytes());
    }
    #[test]
    fn f64_offset_above_tie_is_not_narrowed_and_empty_roi_has_no_tiles() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let mut source = Surface::new(format);
        let extent = Rect::new(0, 0, 2, 1);
        let roi = Rect::new(0, 0, 1, 1);
        source.try_write_interleaved(extent, &[16_777_217u32, 16_777_218].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()).unwrap();
        for depth in [SampleType::F32, SampleType::F64] {
            let mut vectors = field(depth);
            let bytes = if depth == SampleType::F32 { 0.5f32.to_ne_bytes().to_vec() } else { 0.5000000000000002f64.to_ne_bytes().to_vec() };
            vectors.try_write_interleaved(roi, &bytes).unwrap();
            let output = try_displace(&source, extent, &vectors, roi, roi, field_params(), context(), geometry(format), limits(), &Interrupt::NONE).unwrap();
            assert_eq!(raw(&output, 0, 0), raw(&source, if depth == SampleType::F64 { 1 } else { 0 }, 0));
            let empty =
                try_displace(&source, extent, &vectors, roi, Rect::new(0, 0, 0, 0), field_params(), context(), geometry(format), limits(), &Interrupt::NONE)
                    .unwrap();
            assert_eq!(empty.tile_count(), 0);
        }
    }
    #[test]
    fn invalid_field_context_channels_nonfinite_and_coordinate_overflow_are_atomic() {
        let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        let extent = Rect::new(0, 0, 1, 1);
        let mut vectors = field(SampleType::F64);
        let original = source.clone();
        let mut expected = context();
        expected.profile_identity[0] = 18;
        assert!(matches!(
            try_displace(&source, extent, &vectors, extent, extent, field_params(), expected, geometry(source.format()), limits(), &Interrupt::NONE),
            Err(Error::FieldContext)
        ));
        let mut p = field_params();
        p.channels[0] = usize::MAX;
        assert!(matches!(
            try_displace(&source, extent, &vectors, extent, extent, p, context(), geometry(source.format()), limits(), &Interrupt::NONE),
            Err(Error::FieldFormat)
        ));
        for value in [f64::NAN, f64::INFINITY, f64::MAX] {
            vectors.try_write_interleaved(extent, &value.to_ne_bytes()).unwrap();
            p = field_params();
            p.scales[0] = 2.0;
            assert!(try_displace(&source, extent, &vectors, extent, extent, p, context(), geometry(source.format()), limits(), &Interrupt::NONE).is_err());
            assert_eq!(source, original);
        }
        let wrong = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U16, false));
        assert!(matches!(
            try_displace(&source, extent, &wrong, extent, extent, field_params(), context(), geometry(source.format()), limits(), &Interrupt::NONE),
            Err(Error::FieldFormat)
        ));
    }
    #[test]
    fn source_and_field_combined_resident_and_work_limits_refuse_before_output() {
        let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        let vectors = field(SampleType::F64);
        let extent = Rect::new(0, 0, 1, 1);
        for restricted in [Limits { resident_bytes: 1, ..limits() }, Limits { work_units: 1, ..limits() }] {
            assert!(matches!(
                try_displace(&source, extent, &vectors, extent, extent, field_params(), context(), geometry(source.format()), restricted, &Interrupt::NONE),
                Err(Error::Raster(RasterInputError::LimitExceeded))
            ));
        }
    }
    #[test]
    fn cancellation_keeps_both_inputs_and_drops_private_staging() {
        let source = Surface::new(PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true));
        let vectors = field(SampleType::F32);
        let extent = Rect::new(-257, -1, 257, 1);
        let source_before = source.clone();
        let field_before = vectors.clone();
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) > 40;
        assert!(matches!(
            try_displace(
                &source,
                extent,
                &vectors,
                extent,
                extent,
                field_params(),
                context(),
                geometry(source.format()),
                limits(),
                &Interrupt::cancel_only(&cancel)
            ),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source, source_before);
        assert_eq!(vectors, field_before);
    }
    #[test]
    fn shared_preflight_exact_peak_and_false_planning_range_preserve_both_inputs() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let mut source = Surface::new(format);
        let mut vectors = field(SampleType::F64);
        let extent = Rect::new(0, 0, 2, 1);
        let roi = Rect::new(0, 0, 1, 1);
        source.try_write_interleaved(extent, &[16_777_217u32, 16_777_218].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()).unwrap();
        vectors.try_write_interleaved(roi, &1.0f64.to_ne_bytes()).unwrap();
        let before_source = source.clone();
        let before_field = vectors.clone();
        let p = field_params();
        let g = geometry(format);
        let requirements = requirements_with_interrupt(&source, extent, &vectors, roi, roi, p, context(), g, &Interrupt::NONE).unwrap();
        assert_eq!(
            requirements.peak_resident_bytes,
            requirements.source_retained_bytes
                + requirements.field_retained_bytes
                + requirements.output_retained_bytes
                + requirements.output_index_bytes
                + requirements.scratch_bytes
        );
        let exact = Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units };
        let false_range = VectorBounds { min: [0.0; 2], max: [0.0; 2] };
        assert!(matches!(
            try_displace_with_vector_bounds(&source, extent, &vectors, roi, roi, p, context(), g, false_range, exact, &Interrupt::NONE),
            Err(Error::VectorBounds)
        ));
        let output = try_displace_with_vector_bounds(
            &source,
            extent,
            &vectors,
            roi,
            roi,
            p,
            context(),
            g,
            VectorBounds { min: [1.0; 2], max: [1.0; 2] },
            exact,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(raw(&output, 0, 0), raw(&source, 1, 0));
        assert_eq!(source, before_source);
        assert_eq!(vectors, before_field);
    }
    #[test]
    fn absent_owners_require_independent_no_read_proof_and_keep_constant_raw_bits() {
        use super::super::geometry_sampling_roi::{FieldMetadata, SourceMetadata};
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let extent = Rect::new(0, 0, 1, 1);
        let source = SourceMetadata { format, extent };
        let field_metadata = FieldMetadata { format: field(SampleType::F32).format(), extent: Rect::new(10, 10, 11, 11) };
        let mut g = geometry(format);
        g.map.tx = 100.0;
        g.abyss = Abyss::ConstantNative(NativePixel::from_slice(&0x7ff8000000000047u64.to_ne_bytes()).unwrap());
        let range = VectorBounds { min: [-1.0; 2], max: [1.0; 2] };
        let requirements =
            requirements_available_with_interrupt(None, source, None, field_metadata, extent, field_params(), context(), g, range, &Interrupt::NONE).unwrap();
        assert_eq!(requirements.source_retained_bytes, 0);
        assert_eq!(requirements.field_retained_bytes, 0);
        let output = try_displace_available(
            None,
            source,
            None,
            field_metadata,
            extent,
            field_params(),
            context(),
            g,
            range,
            Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(raw(&output, 0, 0), 0x7ff8000000000047u64.to_ne_bytes());
        let mut clamp = g;
        clamp.abyss = Abyss::Clamp;
        assert!(matches!(
            try_displace_available(None, source, None, field_metadata, extent, field_params(), context(), clamp, range, limits(), &Interrupt::NONE),
            Err(Error::MissingPixels)
        ));
        let needed_field = FieldMetadata { extent, ..field_metadata };
        assert!(matches!(
            try_displace_available(None, source, None, needed_field, extent, field_params(), context(), g, range, limits(), &Interrupt::NONE),
            Err(Error::MissingPixels)
        ));
    }
    #[test]
    fn constant_source_none_still_validates_actual_required_field_samples() {
        use super::super::geometry_sampling_roi::{FieldMetadata, SourceMetadata};
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let extent = Rect::new(0, 0, 1, 1);
        let source = SourceMetadata { format, extent };
        let mut vectors = field(SampleType::F32);
        vectors.try_write_interleaved(extent, &f32::NAN.to_ne_bytes()).unwrap();
        let before = vectors.clone();
        let field_metadata = FieldMetadata { format: vectors.format(), extent };
        let mut g = geometry(format);
        g.map.tx = 100.0;
        g.abyss = Abyss::ConstantNative(NativePixel::from_slice(&16_777_217u32.to_ne_bytes()).unwrap());
        assert!(matches!(
            try_displace_available(
                None,
                source,
                Some(&vectors),
                field_metadata,
                extent,
                field_params(),
                context(),
                g,
                VectorBounds { min: [-1.0; 2], max: [1.0; 2] },
                limits(),
                &Interrupt::NONE
            ),
            Err(Error::NonFinite)
        ));
        assert_eq!(vectors, before);
    }
}
