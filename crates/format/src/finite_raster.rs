//! Explicit operative extent for future bounded native raster persistence.
//! This additive metadata validator is not a Document content kind or a loader.
//! The optional owned parser prepays and retains bounded metadata capacity.
//! Selective native reader floors and document load/save remain separate joins;
//! standard SurfaceM semantics stay unchanged.
use crate::{FormatError, Result, manifest::TileRef};
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::backing::{BackingError, NativePlaneDescriptor, ReadControl};
use serde::{Deserialize, Serialize};

#[path = "finite_raster/parse.rs"]
mod parse;
pub use parse::OwnedFiniteRasterMetadata;

/// Raw canonical tile frames, with exact defaults and transparent logical exterior.
/// Do not reinterpret the unbounded legacy SurfaceM representation as this shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiniteRasterM {
    pub format: PixelFormat,
    pub bounds: Rect,
    /// Exactly one little-endian pixel encoded as lowercase hexadecimal.
    pub default: String,
    /// Strict coordinate order, unique, and intersecting the operative extent.
    pub tiles: Vec<TileRef>,
}

/// Prospective domain limits; validation does not allocate or read tile objects.
#[derive(Clone, Copy, Debug)]
pub struct FiniteRasterLimits {
    pub max_logical_bytes: u64,
    pub max_tiles: usize,
}

impl FiniteRasterM {
    pub fn descriptor_with(&self, limits: FiniteRasterLimits, control: &ReadControl<'_>) -> Result<NativePlaneDescriptor> {
        poll(control, 1)?;
        if self.tiles.len() > limits.max_tiles.min(4096) {
            return Err(FormatError::LimitExceeded("finite raster tile references".into()));
        }
        let bpp = self.format.bytes_per_pixel();
        if bpp == 0 || bpp > 40 || self.default.len() != bpp.checked_mul(2).ok_or_else(|| FormatError::corrupt("finite default length"))? {
            return Err(FormatError::corrupt("finite default is not exactly one pixel"));
        }
        let mut default = [0; 40];
        for (out, pair) in default.iter_mut().zip(self.default.as_bytes().as_chunks::<2>().0.iter()) {
            poll(control, 2)?;
            let high = nibble(*pair.first().ok_or_else(|| FormatError::corrupt("finite default byte"))?)?;
            let low = nibble(*pair.get(1).ok_or_else(|| FormatError::corrupt("finite default byte"))?)?;
            *out = (high << 4) | low;
        }
        let native = default.get_mut(..bpp).ok_or_else(|| FormatError::corrupt("finite default layout"))?;
        if cfg!(target_endian = "big") {
            for sample in native.chunks_exact_mut(self.format.sample.bytes()) {
                sample.reverse();
            }
        }
        let descriptor =
            NativePlaneDescriptor::new(self.format, self.bounds, native).map_err(|_| FormatError::corrupt("finite raster extent or byte length"))?;
        if descriptor.logical_bytes() > limits.max_logical_bytes {
            return Err(FormatError::LimitExceeded("finite raster logical bytes".into()));
        }
        let first = TileCoord::containing(self.bounds.x0, self.bounds.y0);
        let last = TileCoord::containing(
            self.bounds.x1.checked_sub(1).ok_or_else(|| FormatError::corrupt("finite x edge"))?,
            self.bounds.y1.checked_sub(1).ok_or_else(|| FormatError::corrupt("finite y edge"))?,
        );
        let mut previous = None;
        for tile in &self.tiles {
            poll(control, 1)?;
            let coord = TileCoord { tx: tile.tx, ty: tile.ty };
            if previous.is_some_and(|p| p >= coord) || coord.tx < first.tx || coord.tx > last.tx || coord.ty < first.ty || coord.ty > last.ty {
                return Err(FormatError::corrupt("finite tile order, duplicate, or outside extent"));
            }
            if tile.hash.len() != 64 {
                return Err(FormatError::corrupt("finite tile hash length"));
            }
            for byte in tile.hash.bytes() {
                poll(control, 1)?;
                nibble(byte)?;
            }
            previous = Some(coord);
        }
        poll(control, 1)?;
        Ok(descriptor)
    }
}

fn nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(FormatError::corrupt("noncanonical finite hexadecimal")),
    }
}
fn poll(control: &ReadControl<'_>, units: u64) -> Result<()> {
    control.check(units).map_err(|error| {
        if error == BackingError::Cancelled { FormatError::Cancelled } else { FormatError::LimitExceeded(format!("finite metadata control: {error}")) }
    })
}
