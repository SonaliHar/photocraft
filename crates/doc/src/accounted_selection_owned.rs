//! Consuming trusted-codec ownership transfer; never an asserted byte census.
use super::*;
use std::any::Any;

impl AccountedSelection {
    /// The codec guard must own the existing Surface map/default/native tile
    /// allocations on this quota. Those allocations are moved, not charged or
    /// copied again. Only the new selection owner/Arc headers are reserved here.
    /// This crate-private boundary conveys no source or rendering authority.
    pub(crate) fn adopt_codec_surface(
        surface: Surface,
        canvas: Rect,
        retention: Arc<dyn Any + Send + Sync>,
        quota: &BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            surface: Surface,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { surface, retention };
        control.check(1)?;
        let count = bounds(canvas)?;
        let format = incoming.surface.format();
        valid(format)?;
        let bpp = format.bytes_per_pixel();
        let tile_bytes = mul(65536, bpp as u64)?;
        let tiles = incoming.surface.tile_count() as u64;
        if tiles > count || add(mul(tiles, tile_bytes)?, bpp as u64)? > MAX_CHECKED_RASTER_BYTES {
            return Err(BackingError::Length);
        }
        if incoming.surface.default_bytes().len() != bpp {
            return Err(BackingError::Integrity);
        }
        coverage(format, incoming.surface.default_bytes(), control)?;
        for (coord, tile) in incoming.surface.tiles() {
            control.check(1024)?;
            let x0 = coord.tx.checked_mul(256).ok_or(BackingError::Bounds)?;
            let y0 = coord.ty.checked_mul(256).ok_or(BackingError::Bounds)?;
            let rect = Rect::new(x0, y0, x0.checked_add(256).ok_or(BackingError::Bounds)?, y0.checked_add(256).ok_or(BackingError::Bounds)?);
            if rect.intersect(&canvas).is_empty() || tile.bytes().len() as u64 != tile_bytes {
                return Err(BackingError::Integrity);
            }
            // Every stored word, including padded tails, is retained verbatim.
            // Float selections require finite unit coverage, as fresh owners do.
            coverage(format, tile.bytes(), control)?;
        }
        let header = add(std::mem::size_of::<SelectionOwner>() as u64, std::mem::size_of::<QuotaLease>() as u64 + 256)?;
        let allocation = quota.reserve(QuotaKind::Index, header).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Incoming { surface, retention } = incoming;
        Ok(Self {
            owner: Arc::new(SelectionOwner { surface, bounds: canvas, quota: quota.clone(), _retention: Some(retention), _payload: None, _index: None }),
            _allocation: Arc::new(allocation),
        })
    }
}

#[cfg(test)]
#[path = "accounted_selection_owned_tests.rs"]
mod tests;
