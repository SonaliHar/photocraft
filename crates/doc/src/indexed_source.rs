//! Immutable indexed-source provenance. Validity is checked on demand, never each frame.
use crate::{Rect, Surface};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedSource {
    indices: Arc<Vec<u8>>,
    alpha: Option<Arc<Vec<u8>>>,
    palette: Vec<[u8; 3]>,
    bounds: Rect,
    fingerprint: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedSourceError;
impl std::fmt::Display for IndexedSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid indexed-source provenance")
    }
}
impl std::error::Error for IndexedSourceError {}

impl IndexedSource {
    pub fn new(
        indices: Arc<Vec<u8>>,
        alpha: Option<Arc<Vec<u8>>>,
        palette: Vec<[u8; 3]>,
        bounds: Rect,
        expanded: &Surface,
    ) -> Result<Self, IndexedSourceError> {
        Self::new_with_interrupt(indices, alpha, palette, bounds, expanded, &photocraft_raster::Interrupt::NONE)
    }
    pub fn new_with_interrupt(
        indices: Arc<Vec<u8>>,
        alpha: Option<Arc<Vec<u8>>>,
        palette: Vec<[u8; 3]>,
        bounds: Rect,
        expanded: &Surface,
        interrupt: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Self, IndexedSourceError> {
        let fingerprint = Self::fingerprint(expanded, bounds, interrupt)?;
        Self::from_parts_with_interrupt(indices, alpha, palette, bounds, fingerprint, interrupt)
    }
    /// Reconstitute a persisted original snapshot, which need not match edited current pixels.
    pub fn from_parts(
        indices: Arc<Vec<u8>>,
        alpha: Option<Arc<Vec<u8>>>,
        palette: Vec<[u8; 3]>,
        bounds: Rect,
        fingerprint: [u8; 32],
    ) -> Result<Self, IndexedSourceError> {
        Self::from_parts_with_interrupt(indices, alpha, palette, bounds, fingerprint, &photocraft_raster::Interrupt::NONE)
    }
    pub fn from_parts_with_interrupt(
        indices: Arc<Vec<u8>>,
        alpha: Option<Arc<Vec<u8>>>,
        palette: Vec<[u8; 3]>,
        bounds: Rect,
        fingerprint: [u8; 32],
        interrupt: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Self, IndexedSourceError> {
        if interrupt.cancelled() {
            return Err(IndexedSourceError);
        }
        let n = usize::try_from(u64::from(bounds.width()) * u64::from(bounds.height())).map_err(|_| IndexedSourceError)?;
        if bounds.is_empty()
            || bounds.width() > 262144
            || bounds.height() > 262144
            || n > (1 << 30)
            || indices.len() != n
            || alpha.as_ref().is_some_and(|a| a.len() != n)
            || palette.is_empty()
            || palette.len() > 256
            || fingerprint == [0; 32]
        {
            return Err(IndexedSourceError);
        }
        for chunk in indices.chunks(4096) {
            if interrupt.cancelled() || chunk.iter().any(|i| usize::from(*i) >= palette.len()) {
                return Err(IndexedSourceError);
            }
        }
        if [bounds.x0, bounds.x1, bounds.y0, bounds.y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) {
            return Err(IndexedSourceError);
        }
        // Recompute the original expanded snapshot using one bounded scratch tile. This checks
        // the persisted fingerprint without retaining a second full expanded raster.
        let mut h = blake3::Hasher::new();
        let format = crate::PixelFormat::new(crate::ColorMode::Rgb, crate::SampleType::U8, true);
        h.update(format!("{format:?}").as_bytes());
        for v in [bounds.x0, bounds.y0, bounds.x1, bounds.y1] {
            h.update(&v.to_le_bytes());
        }
        h.update(&[0; 4]);
        let first = photocraft_geom::TileCoord::containing(bounds.x0, bounds.y0);
        let last = photocraft_geom::TileCoord::containing(bounds.x1 - 1, bounds.y1 - 1);
        let mut tile = Vec::new();
        tile.try_reserve_exact(256 * 256 * 4).map_err(|_| IndexedSourceError)?;
        tile.resize(256 * 256 * 4, 0);
        for tx in first.tx..=last.tx {
            for ty in first.ty..=last.ty {
                if interrupt.cancelled() {
                    return Err(IndexedSourceError);
                }
                tile.fill(0);
                let coord = photocraft_geom::TileCoord::new(tx, ty);
                let r = coord.rect().intersect(&bounds);
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        let i = (y - bounds.y0) as usize * bounds.width() as usize + (x - bounds.x0) as usize;
                        let color = indices.get(i).and_then(|i| palette.get(usize::from(*i))).ok_or(IndexedSourceError)?;
                        let a = alpha.as_ref().map_or(Ok(255), |a| a.get(i).copied().ok_or(IndexedSourceError))?;
                        let offset = ((y - ty * 256) as usize * 256 + (x - tx * 256) as usize) * 4;
                        tile.get_mut(offset..offset + 4).ok_or(IndexedSourceError)?.copy_from_slice(&[color[0], color[1], color[2], a]);
                    }
                }
                h.update(&tx.to_le_bytes());
                h.update(&ty.to_le_bytes());
                h.update(&tile);
            }
        }
        if *h.finalize().as_bytes() != fingerprint {
            return Err(IndexedSourceError);
        }
        Ok(Self { indices, alpha, palette, bounds, fingerprint })
    }
    pub fn indices(&self) -> &Arc<Vec<u8>> {
        &self.indices
    }
    pub fn alpha(&self) -> &Option<Arc<Vec<u8>>> {
        &self.alpha
    }
    /// Retained palette allocation capacity, without cloning or interpreting palette entries.
    pub fn palette_capacity_bytes(&self) -> usize {
        self.palette.capacity().saturating_mul(std::mem::size_of::<[u8; 3]>())
    }

    pub fn palette(&self) -> &[[u8; 3]] {
        &self.palette
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    pub fn expanded_fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
    /// Compare expanded storage, palette and bounds with the original snapshot.
    /// Duplicate palette colors can expand identically after index edits. Callers proving
    /// current index identity must also compare the editable index and alpha planes.
    pub fn current_matches(&self, surface: &Surface, palette: &[[u8; 3]], bounds: Rect) -> bool {
        self.bounds == bounds
            && self.palette == palette
            && Self::fingerprint(surface, bounds, &photocraft_raster::Interrupt::NONE).is_ok_and(|f| self.fingerprint == f)
    }
    fn fingerprint(s: &Surface, bounds: Rect, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<[u8; 32], IndexedSourceError> {
        if interrupt.cancelled() {
            return Err(IndexedSourceError);
        }
        let mut h = blake3::Hasher::new();
        h.update(format!("{:?}", s.format()).as_bytes());
        for v in [bounds.x0, bounds.y0, bounds.x1, bounds.y1] {
            h.update(&v.to_le_bytes());
        }
        // Canonical little endian preserves float bit patterns across architectures.
        let update = |h: &mut blake3::Hasher, data: &[u8]| match s.format().sample {
            crate::SampleType::U8 => {
                h.update(data);
            }
            crate::SampleType::U16 => {
                for c in data.as_chunks::<2>().0 {
                    h.update(&u16::from_ne_bytes(*c).to_le_bytes());
                }
            }
            crate::SampleType::U32 | crate::SampleType::F32 => {
                for c in data.as_chunks::<4>().0 {
                    h.update(&u32::from_ne_bytes(*c).to_le_bytes());
                }
            }
            crate::SampleType::F64 => {
                for c in data.as_chunks::<8>().0 {
                    h.update(&u64::from_ne_bytes(*c).to_le_bytes());
                }
            }
        };
        update(&mut h, s.default_bytes());
        for (coord, tile) in s.tiles() {
            if interrupt.cancelled() {
                return Err(IndexedSourceError);
            }
            h.update(&coord.tx.to_le_bytes());
            h.update(&coord.ty.to_le_bytes());
            update(&mut h, tile.bytes());
        }
        Ok(*h.finalize().as_bytes())
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn palette_capacity_reports_spare_owned_allocation() {
        let mut palette = Vec::with_capacity(1024);
        palette.extend([[0, 0, 0], [255, 255, 255]]);
        let capacity = palette.capacity() * std::mem::size_of::<[u8; 3]>();
        let bounds = Rect::from_xywh(0, 0, 1, 1);
        let expanded = Surface::try_from_interleaved(crate::PixelFormat::RGBA8, bounds, &[0, 0, 0, 255]).unwrap();
        let value = IndexedSource::new(std::sync::Arc::new(vec![0]), None, palette, bounds, &expanded).unwrap();
        assert_eq!(value.palette_capacity_bytes(), capacity);
        assert_eq!(value.palette().len(), 2);
    }
}
