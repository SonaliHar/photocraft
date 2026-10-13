//! Resident raster retention only. Does not estimate all Document metadata allocations.
use photocraft_doc::{Document, Layer, LayerContent, MAX_GROUP_DEPTH};
use photocraft_raster::{Interrupt, Surface, backing::BackingError};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RasterRetention {
    pub retained: u64,
    pub clone_metadata: u64,
}

struct Counter<'a> {
    size: RasterRetention,
    visits: usize,
    limit: u64,
    control: &'a Interrupt<'a>,
}
impl Counter<'_> {
    fn visit(&mut self) -> Result<(), BackingError> {
        self.control.check().map_err(|_| BackingError::Cancelled)?;
        self.visits = self.visits.checked_add(1).filter(|n| *n <= 100_000).ok_or(BackingError::Quota)?;
        Ok(())
    }
    fn surface(&mut self, surface: &Surface) -> Result<(), BackingError> {
        self.visit()?;
        let tile_bytes = (photocraft_geom::TILE_SIZE as usize)
            .checked_mul(photocraft_geom::TILE_SIZE as usize)
            .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel()))
            .ok_or(BackingError::Quota)?;
        if surface.default_bytes().len() != surface.format().bytes_per_pixel() {
            return Err(BackingError::Length);
        }
        let envelope = (surface.tile_count() as u64)
            .checked_mul((tile_bytes as u64).checked_add(256).ok_or(BackingError::Quota)?)
            .and_then(|n| n.checked_add(surface.default_bytes().len() as u64))
            .ok_or(BackingError::Quota)?;
        if self.size.retained.checked_add(envelope).ok_or(BackingError::Quota)? > self.limit {
            return Err(BackingError::Quota);
        }
        for (index, (_, tile)) in surface.tiles().enumerate() {
            if index.is_multiple_of(256) {
                self.control.check().map_err(|_| BackingError::Cancelled)?;
            }
            if tile.bytes().len() != tile_bytes {
                return Err(BackingError::Length);
            }
        }
        let retained = surface.resident_accounted_bytes().map_err(|_| BackingError::Quota)?;
        self.size.retained = self.size.retained.checked_add(retained).filter(|n| *n <= self.limit).ok_or(BackingError::Quota)?;
        let copied =
            (surface.tile_count() as u64).checked_mul(256).and_then(|n| n.checked_add(surface.default_bytes().len() as u64)).ok_or(BackingError::Quota)?;
        self.size.clone_metadata = self.size.clone_metadata.checked_add(copied).ok_or(BackingError::Quota)?;
        surface.try_validate_tile_origins_with(self.control).map_err(|error| match error {
            photocraft_raster::RasterInputError::Cancelled => BackingError::Cancelled,
            _ => BackingError::Bounds,
        })
    }
    fn optional(&mut self, surface: Option<&Surface>) -> Result<(), BackingError> {
        if let Some(surface) = surface {
            self.surface(surface)?;
        }
        Ok(())
    }
}

/// Conservative per-occurrence accounting: shared payloads may be counted twice,
/// never omitted. No native backing IO, decoded samples, ambient files or heap DFS.
pub(super) fn resident(document: &Document, limit: u64, control: &Interrupt<'_>) -> Result<RasterRetention, BackingError> {
    let mut counter = Counter { size: RasterRetention::default(), visits: 0, limit, control };
    counter.visit()?;
    counter.optional(document.selection.as_ref())?;
    for channel in document.channels.iter().chain(document.quick_mask.iter()) {
        counter.surface(&channel.surface)?;
    }
    for pattern in &document.patterns {
        counter.surface(&pattern.surface)?;
    }
    if let Some(state) = &document.xcf_state {
        for mask in &state.filter_masks {
            counter.surface(&mask.surface)?;
        }
    }
    let mut frames: [Option<std::slice::Iter<'_, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *frames.first_mut().ok_or(BackingError::Bounds)? = Some(document.layers.iter());
    let mut depth = 0usize;
    loop {
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth = depth.checked_sub(1).ok_or(BackingError::Bounds)?;
            continue;
        };
        counter.visit()?;
        counter.optional(layer.mask.as_ref().map(|mask| mask.surface.as_ref()))?;
        counter.optional(layer.fill_cache.as_ref().map(|cache| &cache.surface))?;
        if let Some(video) = &layer.video {
            for frame in &video.frames {
                counter.surface(frame)?;
            }
        }
        match &layer.content {
            LayerContent::BackedRaster(_) => return Err(BackingError::Unsupported),
            LayerContent::Raster(surface) => counter.surface(surface)?,
            LayerContent::Text(text) => counter.optional(text.cache.as_ref())?,
            LayerContent::Shape(shape) => counter.optional(shape.cache.as_ref())?,
            LayerContent::Smart(smart) => {
                counter.optional(smart.cache.as_ref())?;
                counter.optional(smart.filter_mask.as_ref().map(|mask| mask.surface.as_ref()))?;
            }
            LayerContent::Group(group) => {
                if depth >= MAX_GROUP_DEPTH {
                    return Err(BackingError::Bounds);
                }
                if !group.children.is_empty() {
                    depth = depth.checked_add(1).ok_or(BackingError::Bounds)?;
                    *frames.get_mut(depth).ok_or(BackingError::Bounds)? = Some(group.children.iter());
                }
            }
            LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
        }
    }
    control.check().map_err(|_| BackingError::Cancelled)?;
    Ok(counter.size)
}
