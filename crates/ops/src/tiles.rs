//! Tile references owned by history snapshots. The snapshots keep these addresses alive;
//! COW edits cannot change a retained tile in place. Count occurrences (including aliases
//! within one document), so removing a state releases a tile only after its last reference.
use photocraft_doc::{Document, LayerContent, Surface};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub(crate) struct Tiles {
    refs: HashMap<usize, (u128, usize)>,
    bytes: u128,
}

pub(crate) fn surfaces(doc: &Document, mut visit: impl FnMut(&Surface)) {
    for (_, _, layer) in doc.walk() {
        if let Some(surface) = layer.surface() {
            visit(surface);
        }
        if let Some(mask) = &layer.mask {
            visit(&mask.surface);
        }
        if let Some(cache) = &layer.fill_cache {
            visit(&cache.surface);
        }
        if let LayerContent::Smart(smart) = &layer.content
            && let Some(mask) = &smart.filter_mask
        {
            visit(&mask.surface);
        }
        if let Some(video) = &layer.video {
            // Undo retains the whole clip, not only the displayed frame. Visit aliases too:
            // Tiles tracks every owner and counts their shared allocations only once.
            for frame in video.frames.iter().chain(video.original_frames.iter().flatten()) {
                visit(frame);
            }
        }
    }
    for channel in doc.channels.iter().chain(&doc.quick_mask) {
        visit(&channel.surface);
    }
    // A pattern fill or layer style copies its pattern into the document.
    for pattern in &doc.patterns {
        visit(&pattern.surface);
    }
    if let Some(selection) = &doc.selection {
        visit(selection);
    }
}

impl Tiles {
    pub(crate) fn capacity(&self) -> usize {
        self.refs.capacity()
    }

    fn snapshot_map(
        slots: usize,
        m: &mut crate::resident_history_controlled::Meter<'_, '_>,
    ) -> Result<HashMap<usize, (u128, usize)>, photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        let envelope = (slots as u64).checked_mul(256).and_then(|n| n.checked_add(4096)).ok_or(E::LimitExceeded)?;
        m.allocate(envelope)?;
        m.step(envelope)?;
        let mut refs = HashMap::new();
        refs.try_reserve(slots).map_err(|_| E::AllocationFailed)?;
        let actual = (refs.capacity() as u64).checked_mul(128).and_then(|n| n.checked_add(std::mem::size_of::<Self>() as u64)).ok_or(E::LimitExceeded)?;
        if actual > envelope {
            return Err(E::LimitExceeded);
        }
        Ok(refs)
    }

    /// Snapshot-only private clone: its whole prospective table is charged
    /// before reserve, and no insertion can grow it implicitly.
    pub(crate) fn snapshot_clone_with(&self, m: &mut crate::resident_history_controlled::Meter<'_, '_>) -> Result<Self, photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        let mut refs = Self::snapshot_map(self.refs.capacity(), m)?;
        for (key, value) in &self.refs {
            m.step(128)?;
            if refs.len() >= refs.capacity() {
                return Err(E::LimitExceeded);
            }
            refs.insert(*key, *value);
        }
        Ok(Self { refs, bytes: self.bytes })
    }

    /// Complete census slots are reserved once before walking the documents;
    /// aliases are counted without any later allocation or rehash growth.
    pub(crate) fn snapshot_build_with<'a>(
        documents: impl Iterator<Item = &'a Document>,
        slots: usize,
        m: &mut crate::resident_history_controlled::Meter<'_, '_>,
    ) -> Result<Self, photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        let mut result = Self { refs: Self::snapshot_map(slots, m)?, bytes: 0 };
        for document in documents {
            m.step(1)?;
            surfaces_with(document, m, |surface, m| {
                for (_, tile) in surface.tiles() {
                    m.step(128)?;
                    let key = Arc::as_ptr(tile) as usize;
                    if let Some(value) = result.refs.get_mut(&key) {
                        value.0 = value.0.checked_add(1).ok_or(E::LimitExceeded)?;
                    } else {
                        if result.refs.len() >= result.refs.capacity() {
                            return Err(E::LimitExceeded);
                        }
                        result.bytes = result.bytes.checked_add(tile.bytes().len() as u128).ok_or(E::LimitExceeded)?;
                        result.refs.insert(key, (1, tile.bytes().len()));
                    }
                }
                Ok(())
            })?;
        }
        Ok(result)
    }

    pub(crate) fn try_clone_with(&self, m: &mut crate::resident_history_controlled::Meter<'_, '_>) -> Result<Self, photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        let slots = self.refs.capacity();
        m.allocate((slots as u64).checked_mul(128).ok_or(E::LimitExceeded)?)?;
        let mut refs = HashMap::new();
        refs.try_reserve(slots).map_err(|_| E::AllocationFailed)?;
        m.allocate((refs.capacity().saturating_sub(slots) as u64).checked_mul(128).ok_or(E::LimitExceeded)?)?;
        for (key, value) in &self.refs {
            m.step(1)?;
            refs.insert(*key, *value);
        }
        Ok(Self { refs, bytes: self.bytes })
    }
    pub(crate) fn add_with(
        &mut self,
        doc: &Document,
        m: &mut crate::resident_history_controlled::Meter<'_, '_>,
    ) -> Result<(), photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        let mut count = 0usize;
        surfaces_with(doc, m, |surface, m| {
            m.step(1)?;
            count = count.checked_add(surface.tile_count()).ok_or(E::LimitExceeded)?;
            Ok(())
        })?;
        let old = self.refs.capacity();
        let needed = self.refs.len().checked_add(count).ok_or(E::LimitExceeded)?;
        let delta = needed.saturating_sub(old);
        m.allocate((delta as u64).checked_mul(128).ok_or(E::LimitExceeded)?)?;
        self.refs.try_reserve(count).map_err(|_| E::AllocationFailed)?;
        m.allocate((self.refs.capacity().saturating_sub(old.checked_add(delta).ok_or(E::LimitExceeded)?) as u64).checked_mul(128).ok_or(E::LimitExceeded)?)?;
        surfaces_with(doc, m, |surface, m| {
            for (_, tile) in surface.tiles() {
                m.step(1)?;
                let key = Arc::as_ptr(tile) as usize;
                if let Some(value) = self.refs.get_mut(&key) {
                    value.0 = value.0.checked_add(1).ok_or(E::LimitExceeded)?;
                } else {
                    self.bytes = self.bytes.checked_add(tile.bytes().len() as u128).ok_or(E::LimitExceeded)?;
                    self.refs.insert(key, (1, tile.bytes().len()));
                }
            }
            Ok(())
        })
    }
    pub(crate) fn remove_with(
        &mut self,
        doc: &Document,
        m: &mut crate::resident_history_controlled::Meter<'_, '_>,
    ) -> Result<(), photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        surfaces_with(doc, m, |surface, m| {
            for (_, tile) in surface.tiles() {
                m.step(1)?;
                let key = Arc::as_ptr(tile) as usize;
                if let Some(value) = self.refs.get_mut(&key) {
                    value.0 = value.0.checked_sub(1).ok_or(E::InvalidBounds)?;
                    if value.0 == 0 {
                        self.bytes = self.bytes.checked_sub(value.1 as u128).ok_or(E::InvalidBounds)?;
                        self.refs.remove(&key);
                    }
                }
            }
            Ok(())
        })
    }
    pub(crate) fn fits_with(
        &self,
        doc: &Document,
        budget: usize,
        seen: &mut HashSet<usize>,
        m: &mut crate::resident_history_controlled::Meter<'_, '_>,
    ) -> Result<bool, photocraft_raster::RasterInputError> {
        if self.bytes > budget as u128 {
            m.step(1)?;
            return Ok(false);
        }
        let mut total = self.bytes;
        surfaces_with(doc, m, |surface, m| {
            for (_, tile) in surface.tiles() {
                m.step(1)?;
                let key = Arc::as_ptr(tile) as usize;
                if !self.refs.contains_key(&key) && seen.insert(key) {
                    total = total.checked_add(tile.bytes().len() as u128).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
                }
            }
            Ok(())
        })?;
        Ok(total <= budget as u128)
    }
    pub(crate) fn add(&mut self, doc: &Document) {
        surfaces(doc, |surface| {
            for (_, tile) in surface.tiles() {
                let count = self.refs.entry(Arc::as_ptr(tile) as usize).or_insert_with(|| {
                    self.bytes += tile.bytes().len() as u128;
                    (0, tile.bytes().len())
                });
                count.0 += 1;
            }
        });
    }

    pub(crate) fn remove(&mut self, doc: &Document) {
        surfaces(doc, |surface| {
            for (_, tile) in surface.tiles() {
                let key = Arc::as_ptr(tile) as usize;
                if let Some(count) = self.refs.get_mut(&key) {
                    count.0 -= 1;
                    if count.0 == 0 {
                        self.bytes -= count.1 as u128;
                        self.refs.remove(&key);
                    }
                }
            }
        });
    }

    /// Retained bytes are incremental; only current tiles need examining below the limit.
    /// Current can change without recording (coalescing), and tile transfers can carry a
    /// different byte length, so a tile-count estimate must not underestimate its memory.
    pub(crate) fn fits(&self, current: &Document, budget: usize) -> bool {
        self.bytes <= budget as u128 && self.bytes_with(current) <= budget as u128
    }

    pub(crate) fn bytes_with(&self, current: &Document) -> u128 {
        let mut total = self.bytes;
        let mut seen = HashSet::new();
        surfaces(current, |surface| {
            for (_, tile) in surface.tiles() {
                let key = Arc::as_ptr(tile) as usize;
                if !self.refs.contains_key(&key) && seen.insert(key) {
                    total += tile.bytes().len() as u128;
                }
            }
        });
        total
    }
}

/// Same owner order/scope as legacy `surfaces`, with bounded model walks and no sample interpretation.
pub(crate) fn surfaces_with(
    doc: &Document,
    m: &mut crate::resident_history_controlled::Meter<'_, '_>,
    mut visit: impl FnMut(&Surface, &mut crate::resident_history_controlled::Meter<'_, '_>) -> Result<(), photocraft_raster::RasterInputError>,
) -> Result<(), photocraft_raster::RasterInputError> {
    let control = m.ctl;
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
        m.step(1)?;
        if matches!(layer.content, photocraft_doc::LayerContent::BackedRaster(_)) {
            return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
        }
        if let Some(surface) = layer.surface() {
            visit(surface, m)?;
        }
        if let Some(mask) = &layer.mask {
            visit(&mask.surface, m)?;
        }
        if let Some(cache) = &layer.fill_cache {
            visit(&cache.surface, m)?;
        }
        if let LayerContent::Smart(smart) = &layer.content
            && let Some(mask) = &smart.filter_mask
        {
            visit(&mask.surface, m)?;
        }
        if let Some(video) = &layer.video {
            for frame in video.frames.iter().chain(video.original_frames.iter().flatten()) {
                m.step(1)?;
                visit(frame, m)?;
            }
        }
        Ok(true)
    })?;
    for channel in doc.channels.iter().chain(&doc.quick_mask) {
        m.step(1)?;
        visit(&channel.surface, m)?;
    }
    for pattern in &doc.patterns {
        m.step(1)?;
        visit(&pattern.surface, m)?;
    }
    if let Some(selection) = &doc.selection {
        m.step(1)?;
        visit(selection, m)?;
    }
    m.step(1)
}
