//! Current editable indexed pixels. This is separate from immutable import provenance.
use crate::{LayerId, Rect, XcfValidationError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type Result<T> = std::result::Result<T, XcfValidationError>;
fn invalid() -> XcfValidationError {
    XcfValidationError::new("indexed_state", "invalid editable indexed pixels or palette")
}
const MAX_PIXELS: usize = 64 << 20;
const COPY_CHUNK: usize = 32 << 10;
fn poll(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| crate::XcfValidationError::new("cancelled", "indexed operation cancelled"))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "IndexedWire")]
pub struct EditableIndexedRaster {
    bounds: Rect,
    indices: Arc<Vec<u8>>,
    alpha: Option<Arc<Vec<u8>>>,
    palette: Vec<[u8; 3]>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexedWire {
    bounds: Rect,
    indices: Arc<Vec<u8>>,
    alpha: Option<Arc<Vec<u8>>>,
    palette: Vec<[u8; 3]>,
}
impl TryFrom<IndexedWire> for EditableIndexedRaster {
    type Error = XcfValidationError;
    fn try_from(wire: IndexedWire) -> Result<Self> {
        Self::try_new(wire.bounds, wire.indices, wire.alpha, wire.palette)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableIndexedLayer {
    pub owner: LayerId,
    pub raster: EditableIndexedRaster,
}
impl EditableIndexedRaster {
    pub fn try_new(bounds: Rect, indices: Arc<Vec<u8>>, alpha: Option<Arc<Vec<u8>>>, palette: Vec<[u8; 3]>) -> Result<Self> {
        Self::try_new_with_interrupt(bounds, indices, alpha, palette, &photocraft_raster::Interrupt::NONE)
    }
    pub fn try_new_with_interrupt(
        bounds: Rect,
        indices: Arc<Vec<u8>>,
        alpha: Option<Arc<Vec<u8>>>,
        palette: Vec<[u8; 3]>,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Self> {
        poll(control)?;
        let value = Self { bounds, indices, alpha, palette };
        value.validate_with(control)?;
        poll(control)?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_with(&photocraft_raster::Interrupt::NONE)
    }
    pub fn validate_with(&self, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        poll(control)?;
        let width = i64::from(self.bounds.x1) - i64::from(self.bounds.x0);
        let height = i64::from(self.bounds.y1) - i64::from(self.bounds.y0);
        let count = width.checked_mul(height).and_then(|n| usize::try_from(n).ok()).ok_or_else(invalid)?;
        if width <= 0
            || height <= 0
            || width > 262144
            || height > 262144
            || count > MAX_PIXELS
            || self.indices.len() != count
            || self.alpha.as_ref().is_some_and(|a| a.len() != count)
            || self.palette.is_empty()
            || self.palette.len() > 256
        {
            return Err(invalid());
        }
        for chunk in self.indices.chunks(32 << 10) {
            poll(control)?;
            if chunk.iter().any(|i| usize::from(*i) >= self.palette.len()) {
                return Err(invalid());
            }
            poll(control)?;
        }
        poll(control)?;
        Ok(())
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    pub fn indices(&self) -> &Arc<Vec<u8>> {
        &self.indices
    }
    pub fn alpha(&self) -> Option<&Arc<Vec<u8>>> {
        self.alpha.as_ref()
    }
    /// Retained palette allocation capacity, without cloning or interpreting palette entries.
    pub fn palette_capacity_bytes(&self) -> usize {
        self.palette.capacity().saturating_mul(std::mem::size_of::<[u8; 3]>())
    }

    pub fn palette(&self) -> &[[u8; 3]] {
        &self.palette
    }
    fn offset(&self, x: i32, y: i32) -> Result<usize> {
        if x < self.bounds.x0 || x >= self.bounds.x1 || y < self.bounds.y0 || y >= self.bounds.y1 {
            return Err(invalid());
        }
        let width = i64::from(self.bounds.x1) - i64::from(self.bounds.x0);
        let offset = (i64::from(y) - i64::from(self.bounds.y0))
            .checked_mul(width)
            .and_then(|n| n.checked_add(i64::from(x) - i64::from(self.bounds.x0)))
            .ok_or_else(invalid)?;
        usize::try_from(offset).map_err(|_| invalid())
    }
    pub fn expanded_pixel(&self, x: i32, y: i32) -> Result<[u8; 4]> {
        let offset = self.offset(x, y)?;
        let index = self.indices.get(offset).ok_or_else(invalid)?;
        let color = self.palette.get(usize::from(*index)).ok_or_else(invalid)?;
        let alpha = self.alpha.as_ref().map_or(Ok(255), |a| a.get(offset).copied().ok_or_else(invalid))?;
        Ok([color[0], color[1], color[2], alpha])
    }
    /// Produces a checked COW edit; failure leaves the current state unchanged.
    pub fn try_set_pixel(&mut self, x: i32, y: i32, index: u8, alpha: u8) -> Result<()> {
        self.try_set_pixel_with_interrupt(x, y, index, alpha, &photocraft_raster::Interrupt::NONE)
    }
    pub fn try_set_pixel_with_interrupt(&mut self, x: i32, y: i32, index: u8, alpha: u8, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        self.validate_with(control)?;
        let offset = self.offset(x, y)?;
        if usize::from(index) >= self.palette.len() {
            return Err(invalid());
        }
        let mut indices = copy_with_interrupt(&self.indices, control)?;
        *indices.get_mut(offset).ok_or_else(invalid)? = index;
        let new_alpha = if let Some(plane) = &self.alpha {
            let mut bytes = copy_with_interrupt(plane, control)?;
            *bytes.get_mut(offset).ok_or_else(invalid)? = alpha;
            Some(Arc::new(bytes))
        } else if alpha != 255 {
            let mut bytes = Vec::new();
            poll(control)?;
            let allocation = bytes.try_reserve_exact(indices.len());
            poll(control)?;
            allocation.map_err(|_| invalid())?;
            while bytes.len() < indices.len() {
                poll(control)?;
                let end = bytes.len().checked_add(COPY_CHUNK).ok_or_else(invalid)?.min(indices.len());
                bytes.resize(end, 255);
                poll(control)?;
            }
            *bytes.get_mut(offset).ok_or_else(invalid)? = alpha;
            Some(Arc::new(bytes))
        } else {
            None
        };
        let new_indices = Arc::new(indices);
        // All allocations and both private planes are complete before publication.
        poll(control)?;
        self.indices = new_indices;
        self.alpha = new_alpha;
        Ok(())
    }
    /// Palette order and duplicate entries are meaningful and are never deduplicated.
    pub fn try_set_palette(&mut self, palette: Vec<[u8; 3]>) -> Result<()> {
        self.try_set_palette_with_interrupt(palette, &photocraft_raster::Interrupt::NONE)
    }
    pub fn try_set_palette_with_interrupt(&mut self, palette: Vec<[u8; 3]>, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        self.validate_with(control)?;
        if palette.is_empty() || palette.len() > 256 {
            return Err(invalid());
        }
        for chunk in self.indices.chunks(COPY_CHUNK) {
            poll(control)?;
            if chunk.iter().any(|i| usize::from(*i) >= palette.len()) {
                return Err(invalid());
            }
            poll(control)?;
        }
        // Incoming palette is already owned, so moving it retains exact order and
        // capacity without an unnecessary clone or a second allocation.
        poll(control)?;
        self.palette = palette;
        Ok(())
    }
    /// Move the declared extent without resampling or changing any plane bytes.
    pub fn translate_bounds(&mut self, dx: i32, dy: i32) -> Result<()> {
        let x0 = self.bounds.x0.checked_add(dx).ok_or_else(invalid)?;
        let x1 = self.bounds.x1.checked_add(dx).ok_or_else(invalid)?;
        let y0 = self.bounds.y0.checked_add(dy).ok_or_else(invalid)?;
        let y1 = self.bounds.y1.checked_add(dy).ok_or_else(invalid)?;
        self.bounds = Rect::new(x0, y0, x1, y1);
        Ok(())
    }
}
fn copy_with_interrupt(bytes: &[u8], control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>> {
    poll(control)?;
    let mut result = Vec::new();
    let allocation = result.try_reserve_exact(bytes.len());
    poll(control)?;
    allocation.map_err(|_| invalid())?;
    for chunk in bytes.chunks(COPY_CHUNK) {
        poll(control)?;
        result.extend_from_slice(chunk);
        poll(control)?;
    }
    poll(control)?;
    Ok(result)
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn palette_capacity_reports_spare_owned_allocation() {
        let mut palette = Vec::with_capacity(1024);
        palette.extend([[0, 0, 0], [255, 255, 255]]);
        let capacity = palette.capacity() * std::mem::size_of::<[u8; 3]>();
        let value = EditableIndexedRaster::try_new(Rect::from_xywh(0, 0, 1, 1), std::sync::Arc::new(vec![0]), None, palette).unwrap();
        assert_eq!(value.palette_capacity_bytes(), capacity);
        assert_eq!(value.palette().len(), 2);
    }
}

#[cfg(test)]
mod controlled_edit_tests {
    use super::*;
    #[test]
    fn late_cow_copy_cancel_preserves_both_source_planes_and_identity() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let indices = Arc::new(vec![0; 70000]);
        let alpha = Arc::new(vec![123; 70000]);
        let mut raster =
            EditableIndexedRaster::try_new(Rect::new(-10, -1, 69990, 0), indices.clone(), Some(alpha.clone()), vec![[0, 0, 0], [255, 0, 0]]).unwrap();
        let before = raster.clone();
        let calls = AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 14;
        let control = photocraft_raster::Interrupt::cancel_only(&cancel);
        assert!(raster.try_set_pixel_with_interrupt(-9, -1, 1, 222, &control).is_err());
        assert_eq!(raster, before);
        assert!(Arc::ptr_eq(raster.indices(), &indices));
        assert!(Arc::ptr_eq(raster.alpha().unwrap(), &alpha));
    }
    #[test]
    fn duplicate_palette_ordinals_and_implicit_opaque_plane_are_preserved() {
        let source = Arc::new(vec![0, 1]);
        let duplicate = [12, 34, 56];
        let mut raster = EditableIndexedRaster::try_new_with_interrupt(
            Rect::new(-1, 0, 1, 1),
            source.clone(),
            None,
            vec![duplicate, duplicate],
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap();
        raster.try_set_pixel_with_interrupt(-1, 0, 1, 255, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(raster.indices().as_slice(), &[1, 1]);
        assert_eq!(source.as_slice(), &[0, 1]);
        assert_eq!(raster.palette(), &[duplicate, duplicate]);
        assert!(raster.alpha().is_none());
    }
    #[test]
    fn palette_cancel_and_missing_ordinal_refuse_atomically() {
        let indices = Arc::new(vec![1]);
        let mut raster = EditableIndexedRaster::try_new(Rect::new(0, 0, 1, 1), indices.clone(), None, vec![[0, 0, 0], [1, 1, 1]]).unwrap();
        let before = raster.clone();
        let cancel = || true;
        assert!(raster.try_set_palette_with_interrupt(vec![[2, 2, 2], [3, 3, 3]], &photocraft_raster::Interrupt::cancel_only(&cancel)).is_err());
        assert_eq!(raster, before);
        assert!(raster.try_set_palette_with_interrupt(vec![[2, 2, 2]], &photocraft_raster::Interrupt::NONE).is_err());
        assert_eq!(raster, before);
        assert!(Arc::ptr_eq(raster.indices(), &indices));
    }
}
