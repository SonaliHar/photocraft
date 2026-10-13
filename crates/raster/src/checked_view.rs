//! Explicit checked access to resident and owned-backed native pixels.
//! A missing resident tile is a verified sparse default; a backing cache miss is
//! fallible IO. This view never materializes a Surface or infers color authority.
use crate::backing::{BackedRaster, BackingError, NativePlaneDescriptor, ReadControl, TilePin};
use crate::{RasterInputError, Surface, Tile};
use photocraft_color::{PixelFormat, SampleAccessError, try_read_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterViewError {
    Resident(RasterInputError),
    Backed(BackingError),
    Control(BackingError),
    Sample(SampleAccessError),
    Descriptor(BackingError),
    Length,
    Bounds,
}
impl std::fmt::Display for RasterViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "checked raster view: {self:?}")
    }
}
impl std::error::Error for RasterViewError {}

#[derive(Clone, Copy, Debug)]
enum Storage<'a> {
    Resident(&'a Surface),
    Backed(&'a BackedRaster),
}
/// Borrowed exact raster access. The descriptor owns only its fixed-size default
/// copy, no heap. For resident input the caller supplies operative logical bounds;
/// this does not erase retained resident bytes outside those bounds.
#[derive(Clone, Copy, Debug)]
pub struct CheckedRasterView<'a> {
    storage: Storage<'a>,
    descriptor: NativePlaneDescriptor,
}
impl<'a> CheckedRasterView<'a> {
    /// Synchronous convenience constructor. Background callers should use the
    /// metered, cancellable resident_with_control constructor.
    pub fn resident(surface: &'a Surface, logical_bounds: Rect) -> Result<Self, RasterViewError> {
        fn no_work(_: u64) -> Result<(), BackingError> {
            Ok(())
        }
        Self::resident_with_control(surface, logical_bounds, &ReadControl { interrupt: &crate::Interrupt::NONE, work: &no_work })
    }
    pub fn resident_with_control(surface: &'a Surface, logical_bounds: Rect, control: &ReadControl<'_>) -> Result<Self, RasterViewError> {
        control.check(surface.tile_count() as u64).map_err(RasterViewError::Control)?;
        surface.try_validate_tile_origins_with(control.interrupt).map_err(RasterViewError::Resident)?;
        let descriptor = NativePlaneDescriptor::new(surface.format(), logical_bounds, surface.default_bytes()).map_err(RasterViewError::Descriptor)?;
        control.check(1).map_err(RasterViewError::Control)?;
        Ok(Self { storage: Storage::Resident(surface), descriptor })
    }
    pub fn backed(raster: &'a BackedRaster) -> Self {
        Self { storage: Storage::Backed(raster), descriptor: raster.descriptor() }
    }
    pub fn format(&self) -> PixelFormat {
        self.descriptor.format()
    }
    pub fn default_bytes(&self) -> &[u8] {
        self.descriptor.default_bytes()
    }
    pub fn logical_bounds(&self) -> Rect {
        self.descriptor.bounds()
    }

    /// Raw storage read, including retained resident pixels outside logical_bounds.
    /// It must not be used to infer canonical drawable coverage. Use the logical
    /// read below for drawable pixels. Caller output is unchanged on failure.
    pub fn read_pixel_native_into(&self, x: i32, y: i32, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), RasterViewError> {
        control.check(1).map_err(RasterViewError::Control)?;
        let n = self.format().bytes_per_pixel();
        if output.len() != n {
            return Err(RasterViewError::Length);
        }
        let mut raw = [0; 40];
        let pixel = raw.get_mut(..n).ok_or(RasterViewError::Length)?;
        // Validate the complete selected resident tile length, not merely the
        // requested byte range: put_tiles can otherwise forge a mixed layout.
        self.pin_tile(TileCoord::containing(x, y), control)?.read_pixel_native_into(x, y, pixel, control)?;
        control.check(n as u64).map_err(RasterViewError::Control)?;
        output.copy_from_slice(pixel);
        Ok(())
    }
    /// Raw storage f64 read: U32 never passes through f32,
    /// F64 bits/signed zero are retained. No color conversion or sample policy is
    /// inferred here. Caller output remains unchanged on failure.
    pub fn read_pixel_f64_into(&self, x: i32, y: i32, output: &mut [f64], control: &ReadControl<'_>) -> Result<(), RasterViewError> {
        control.check(1).map_err(RasterViewError::Control)?;
        let format = self.format();
        if output.len() != format.channels() {
            return Err(RasterViewError::Length);
        }
        let mut raw = [0; 40];
        let pixel = raw.get_mut(..format.bytes_per_pixel()).ok_or(RasterViewError::Length)?;
        self.read_pixel_native_into(x, y, pixel, control)?;
        let mut staged = [0.0; 5];
        for (i, value) in staged.get_mut(..output.len()).ok_or(RasterViewError::Length)?.iter_mut().enumerate() {
            *value = try_read_sample_f64(pixel, format.sample, i).map_err(RasterViewError::Sample)?;
        }
        control.check(output.len() as u64).map_err(RasterViewError::Control)?;
        output.copy_from_slice(staged.get(..output.len()).ok_or(RasterViewError::Length)?);
        Ok(())
    }
    /// Canonical logical pixel access. Outside coordinates are an explicit error;
    /// callers choose coverage/clipping without erasing retained physical bytes.
    pub fn read_logical_pixel_native_into(&self, x: i32, y: i32, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), RasterViewError> {
        control.check(1).map_err(RasterViewError::Control)?;
        if !self.logical_bounds().contains(x, y) {
            return Err(RasterViewError::Bounds);
        }
        self.read_pixel_native_into(x, y, output, control)
    }
    pub fn read_logical_pixel_f64_into(&self, x: i32, y: i32, output: &mut [f64], control: &ReadControl<'_>) -> Result<(), RasterViewError> {
        control.check(1).map_err(RasterViewError::Control)?;
        if !self.logical_bounds().contains(x, y) {
            return Err(RasterViewError::Bounds);
        }
        self.read_pixel_f64_into(x, y, output, control)
    }
    /// Resident sparse defaults require no heap allocation; resident tiles share
    /// their existing Arc. All pin kinds outlive the view. Backing pins retain the
    /// physical quota lease independently of cache eviction and source lifetime.
    pub fn pin_tile(&self, coord: TileCoord, control: &ReadControl<'_>) -> Result<CheckedTilePin, RasterViewError> {
        control.check(1).map_err(RasterViewError::Control)?;
        tile_rect(coord)?;
        let storage = match self.storage {
            Storage::Resident(surface) => {
                if let Some(tile) = surface.tile(coord) {
                    let expected = (TILE_SIZE as usize)
                        .checked_mul(TILE_SIZE as usize)
                        .and_then(|n| n.checked_mul(self.format().bytes_per_pixel()))
                        .ok_or(RasterViewError::Bounds)?;
                    if tile.bytes().len() != expected {
                        return Err(RasterViewError::Resident(RasterInputError::LengthMismatch));
                    }
                    PinStorage::Resident(Arc::clone(tile))
                } else {
                    PinStorage::Default(self.descriptor)
                }
            }
            Storage::Backed(raster) => PinStorage::Backed(raster.pin_tile(coord, control).map_err(RasterViewError::Backed)?),
        };
        control.check(1).map_err(RasterViewError::Control)?;
        Ok(CheckedTilePin { storage, format: self.format(), coord })
    }
}
#[derive(Clone, Debug)]
enum PinStorage {
    Resident(Arc<Tile>),
    Default(NativePlaneDescriptor),
    Backed(TilePin),
}
/// Explicit payload shape: DefaultPixel is one pixel, never a whole tile buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTileBytes<'a> {
    FullTile(&'a [u8]),
    DefaultPixel(&'a [u8]),
}
#[derive(Clone, Debug)]
pub struct CheckedTilePin {
    storage: PinStorage,
    format: PixelFormat,
    coord: TileCoord,
}
impl CheckedTilePin {
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn coord(&self) -> TileCoord {
        self.coord
    }
    pub fn encoded(&self) -> NativeTileBytes<'_> {
        match &self.storage {
            PinStorage::Resident(tile) => NativeTileBytes::FullTile(tile.bytes()),
            PinStorage::Default(pixel) => NativeTileBytes::DefaultPixel(pixel.default_bytes()),
            PinStorage::Backed(pin) => NativeTileBytes::FullTile(pin.bytes()),
        }
    }
    pub fn read_pixel_native_into(&self, x: i32, y: i32, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), RasterViewError> {
        control.check(1).map_err(RasterViewError::Control)?;
        let bpp = self.format.bytes_per_pixel();
        if output.len() != bpp {
            return Err(RasterViewError::Length);
        }
        let rect = tile_rect(self.coord)?;
        if !rect.contains(x, y) {
            return Err(RasterViewError::Bounds);
        }
        let pixel = match self.encoded() {
            NativeTileBytes::DefaultPixel(pixel) => pixel,
            NativeTileBytes::FullTile(bytes) => {
                let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| RasterViewError::Bounds)?;
                let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| RasterViewError::Bounds)?;
                let start =
                    row.checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_add(col)).and_then(|n| n.checked_mul(bpp)).ok_or(RasterViewError::Bounds)?;
                bytes.get(start..start.checked_add(bpp).ok_or(RasterViewError::Bounds)?).ok_or(RasterViewError::Length)?
            }
        };
        if pixel.len() != bpp {
            return Err(RasterViewError::Length);
        }
        control.check(bpp as u64).map_err(RasterViewError::Control)?;
        output.copy_from_slice(pixel);
        Ok(())
    }
}
fn tile_rect(coord: TileCoord) -> Result<Rect, RasterViewError> {
    let x = coord.tx.checked_mul(TILE_SIZE).ok_or(RasterViewError::Bounds)?;
    let y = coord.ty.checked_mul(TILE_SIZE).ok_or(RasterViewError::Bounds)?;
    Ok(Rect::new(x, y, x.checked_add(TILE_SIZE).ok_or(RasterViewError::Bounds)?, y.checked_add(TILE_SIZE).ok_or(RasterViewError::Bounds)?))
}
