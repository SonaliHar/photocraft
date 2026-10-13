//! Owned-cache-only linked affine placement candidate. No serialized interpolation
//! mapping, resource fetch, source mutation, CMS default or admitted dispatcher.
use photocraft_algo::gimp::{
    geometry_bilinear as bilinear, geometry_nearest as nearest,
    geometry_sampling_roi::{self as footprint, SourceMetadata, SourceRequirement},
    wide_point::WorkingContext,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
use photocraft_doc::{LayerId, SampleEncoding, item_identity::ResourceId};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeDomain {
    pub format: PixelFormat,
    pub encoding: SampleEncoding,
    pub profile_identity: [u8; 32],
}
pub struct OwnedCache<'a> {
    pub owner: LayerId,
    pub resource: ResourceId,
    pub cache_revision: u64,
    pub raster: &'a Arc<Surface>,
    pub extent: Rect,
    pub domain: NativeDomain,
}
/// Explicit output pixel-center -> cache spatial map. No inverse/determinant or
/// interpretation of a retained GIMP transform is guessed by this consumer.
#[derive(Clone, Copy, Debug)]
pub struct OutputToCache {
    pub xx: f64,
    pub xy: f64,
    pub yx: f64,
    pub yy: f64,
    pub tx: f64,
    pub ty: f64,
}
#[derive(Clone, Copy)]
pub enum Interpolation<'a> {
    RawNearest {
        ties: nearest::Ties,
        abyss: nearest::Abyss,
        output_default: nearest::NativePixel,
    },
    /// Existing straight-channel F32/F64 math, not GIMP's linked Linear epoch.
    StraightBilinear {
        alpha: bilinear::Alpha,
        abyss: bilinear::Abyss<'a>,
        output_default: &'a [u8],
    },
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub map: OutputToCache,
    pub output_extent: Rect,
    pub roi: Rect,
    pub output_domain: NativeDomain,
    pub interpolation: Interpolation<'a>,
}
#[derive(Clone, Copy, Debug)]
pub enum ReadCoverage {
    NoPixelsRequired,
    Rectangle(Rect),
    ConservativeFullCache(Rect),
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub coverage: ReadCoverage,
    pub source_retained_bytes: u64,
    pub output_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
pub struct PlacedCache {
    pub raster: Surface,
    pub roi: Rect,
    pub owner: LayerId,
    pub resource: ResourceId,
    pub cache_revision: u64,
    pub coverage: ReadCoverage,
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn nearest_error(e: nearest::Error) -> RasterInputError {
    match e {
        nearest::Error::Raster(e) => e,
        _ => RasterInputError::UnsupportedConversion,
    }
}
fn bilinear_error(e: bilinear::Error) -> RasterInputError {
    match e {
        bilinear::Error::Raster(e) => e,
        _ => RasterInputError::UnsupportedConversion,
    }
}
fn footprint_error(e: footprint::Error) -> RasterInputError {
    match e {
        footprint::Error::Raster(e) => e,
        _ => RasterInputError::InvalidBounds,
    }
}
fn params(request: Request<'_>, ties: nearest::Ties, abyss: nearest::Abyss, output_default: nearest::NativePixel) -> nearest::Params {
    let map = request.map;
    nearest::Params { map: nearest::Affine { xx: map.xx, xy: map.xy, yx: map.yx, yy: map.yy, tx: map.tx, ty: map.ty }, ties, abyss, output_default }
}
fn tile_count(roi: Rect) -> Result<u64, RasterInputError> {
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
    )
}
fn bilinear_pixel(bytes: &[u8], format: PixelFormat) -> Result<(), RasterInputError> {
    if bytes.len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    for index in 0..format.channels() {
        let value = try_read_sample_f64(bytes, format.sample, index).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() || (format.alpha && index == format.mode.color_channels() && !(0.0..=1.0).contains(&value)) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(())
}
fn validate(cache: &OwnedCache<'_>, request: Request<'_>, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    poll(control)?;
    if cache.owner.0 == 0
        || cache.resource.0 == 0
        || cache.raster.format() != cache.domain.format
        || cache.domain != request.output_domain
        || cache.extent.is_empty()
        || request.output_extent.is_empty()
        || request.roi.is_empty()
        || request.roi.intersect(&request.output_extent) != request.roi
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if [request.map.xx, request.map.xy, request.map.yx, request.map.yy, request.map.tx, request.map.ty].iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidBounds);
    }
    let last_x = request.roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let last_y = request.roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    for (x, y) in [(request.roi.x0, request.roi.y0), (last_x, request.roi.y0), (request.roi.x0, last_y), (last_x, last_y)] {
        poll(control)?;
        let x = f64::from(x) + 0.5;
        let y = f64::from(y) + 0.5;
        let m = request.map;
        let xx = m.xx * x;
        let xy = m.xy * y;
        let yx = m.yx * x;
        let yy = m.yy * y;
        let sx = xx + xy;
        let sy = yx + yy;
        let sx = sx + m.tx;
        let sy = sy + m.ty;
        if [xx, xy, yx, yy, sx, sy, sx - 0.5, sy - 0.5].iter().any(|value| !value.is_finite()) {
            return Err(RasterInputError::InvalidBounds);
        }
    }
    checked_region_byte_len(cache.extent, cache.domain.format)?;
    checked_region_byte_len(request.output_extent, request.output_domain.format)?;
    checked_region_byte_len(request.roi, request.output_domain.format)?;
    Ok(())
}
fn cached_resident(cache: &OwnedCache<'_>, control: &Interrupt<'_>) -> Result<u64, RasterInputError> {
    poll(control)?;
    let bpp = cache.domain.format.bytes_per_pixel();
    if bpp == 0 || bpp > 40 || cache.raster.default_bytes().len() != bpp {
        return Err(RasterInputError::LengthMismatch);
    }
    let payload = mul(65536, bpp as u64)?;
    let upper = add(mul(cache.raster.tile_count() as u64, add(payload, 256)?)?, bpp as u64)?;
    if upper > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    cache.raster.try_validate_tile_origins_with(control)?;
    for (_, tile) in cache.raster.tiles() {
        poll(control)?;
        if tile.bytes().len() as u64 != payload {
            return Err(RasterInputError::LengthMismatch);
        }
    }
    poll(control)?;
    Ok(upper)
}
/// Bounded native cache footprint/admission; retained cache memory is charged
/// even when nearest proves no reads, never treated as an upstream refund.
pub fn requirements(cache: &OwnedCache<'_>, request: Request<'_>, control: &Interrupt<'_>) -> Result<Requirements, RasterInputError> {
    validate(cache, request, control)?;
    let source_retained = cached_resident(cache, control)?;
    match request.interpolation {
        Interpolation::RawNearest { ties, abyss, output_default } => {
            let parameters = params(request, ties, abyss, output_default);
            let needed = footprint::plan_nearest(
                SourceMetadata { format: cache.domain.format, extent: cache.extent },
                request.roi,
                parameters,
                footprint::Limits { work_units: 4096 },
                control,
            )
            .map_err(footprint_error)?;
            let admitted = match needed {
                SourceRequirement::None => nearest::requirements_no_pixels_with_interrupt(cache.domain.format, cache.extent, request.roi, parameters, control)
                    .map_err(nearest_error)?,
                _ => nearest::requirements_with_interrupt(cache.raster, cache.extent, request.roi, parameters, control).map_err(nearest_error)?,
            };
            let source = source_retained;
            let peak = add(admitted.peak_resident_bytes, source)?;
            if peak > MAX_CHECKED_RASTER_BYTES {
                return Err(RasterInputError::LimitExceeded);
            }
            let coverage = match needed {
                SourceRequirement::None => ReadCoverage::NoPixelsRequired,
                SourceRequirement::Rectangle(rect) => ReadCoverage::Rectangle(rect),
                SourceRequirement::FullExtentForUncertainDisplacement(_) => return Err(RasterInputError::UnsupportedConversion),
            };
            Ok(Requirements {
                coverage,
                source_retained_bytes: source,
                output_bytes: admitted.output_retained_bytes,
                index_bytes: admitted.output_index_bytes,
                scratch_bytes: admitted.scratch_bytes,
                peak_resident_bytes: peak,
                work_units: add(admitted.work_units, 4096)?,
            })
        }
        Interpolation::StraightBilinear { abyss, output_default, .. } => {
            let format = cache.domain.format;
            if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            bilinear_pixel(output_default, format)?;
            if let bilinear::Abyss::ConstantNative(pixel) = abyss {
                bilinear_pixel(pixel, format)?;
            }
            let payload = mul(65536, format.bytes_per_pixel() as u64)?;
            let count = tile_count(request.roi)?;
            let output = mul(count, add(payload, 1024)?)?;
            let scratch = add(mul(payload, 8)?, 128)?;
            let source = source_retained;
            let index = mul(count, 512)?;
            let peak = add(source, add(output, add(scratch, index)?)?)?;
            if peak > MAX_CHECKED_RASTER_BYTES {
                return Err(RasterInputError::LimitExceeded);
            }
            let work = bilinear::required_work(cache.raster, request.roi).map_err(bilinear_error)?;
            Ok(Requirements {
                coverage: ReadCoverage::ConservativeFullCache(cache.extent),
                source_retained_bytes: source,
                output_bytes: output,
                index_bytes: index,
                scratch_bytes: scratch,
                peak_resident_bytes: peak,
                work_units: work,
            })
        }
    }
}
/// Private output from already owned pixels only. Cache/source provenance is
/// returned unchanged; caller still owns final document/revision qualification.
pub fn place(cache: &OwnedCache<'_>, request: Request<'_>, limits: nearest::Limits, control: &Interrupt<'_>) -> Result<PlacedCache, RasterInputError> {
    let admitted = requirements(cache, request, control)?;
    if limits.resident_bytes == 0
        || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES
        || admitted.peak_resident_bytes > limits.resident_bytes
        || admitted.work_units > limits.work_units
    {
        return Err(RasterInputError::LimitExceeded);
    }
    poll(control)?;
    let raster = match request.interpolation {
        Interpolation::RawNearest { ties, abyss, output_default } => {
            let parameters = params(request, ties, abyss, output_default);
            match admitted.coverage {
                ReadCoverage::NoPixelsRequired => {
                    nearest::try_map_no_pixels(cache.domain.format, cache.extent, request.roi, parameters, limits, control).map_err(nearest_error)?
                }
                _ => nearest::try_map(cache.raster, cache.extent, request.roi, parameters, limits, control).map_err(nearest_error)?,
            }
        }
        Interpolation::StraightBilinear { alpha, abyss, output_default } => {
            let m = request.map;
            let configuration = bilinear::Sampler {
                map: bilinear::Affine { xx: m.xx, xy: m.xy, x_offset: m.tx, yx: m.yx, yy: m.yy, y_offset: m.ty },
                source_extent: cache.extent,
                output_extent: request.output_extent,
                output_format: request.output_domain.format,
                alpha,
                abyss,
                output_default,
            };
            let context = WorkingContext { encoding: cache.domain.encoding, profile_identity: cache.domain.profile_identity };
            bilinear::try_apply(
                cache.raster,
                &configuration,
                request.roi,
                context,
                context,
                bilinear::Limits { resident_bytes: limits.resident_bytes, work_units: limits.work_units },
                control,
            )
            .map_err(bilinear_error)?
        }
    };
    poll(control)?;
    Ok(PlacedCache {
        raster,
        roi: request.roi,
        owner: cache.owner,
        resource: cache.resource,
        cache_revision: cache.cache_revision,
        coverage: admitted.coverage,
    })
}
