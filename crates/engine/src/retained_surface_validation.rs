//! Nonmutating retained-surface admission before command geometry/history traversal.
use photocraft_doc::{Document, Layer, LayerContent, MAX_GROUP_DEPTH};
use photocraft_raster::{Interrupt, RasterInputError, Surface};

/// Inspect every current document Surface, including hidden/disabled caches.
/// No samples are decoded and no linked/embedded source files are opened.
/// The caller installs this before enabled predicates, floating drops and command execution.
pub fn validate_document(document: &Document, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    optional(document.selection.as_ref(), control)?;
    for channel in document.channels.iter().chain(document.quick_mask.iter()) {
        channel.surface.try_validate_tile_origins_with(control)?;
    }
    for pattern in &document.patterns {
        pattern.surface.try_validate_tile_origins_with(control)?;
    }
    if let Some(state) = &document.xcf_state {
        for mask in &state.filter_masks {
            mask.surface.try_validate_tile_origins_with(control)?;
        }
    }
    // Allocation-free borrowed DFS: native/engine's existing group depth ceiling
    // also bounds this stack. Do not use recursive Document::walk on a forged tree.
    let mut frames: [Option<std::slice::Iter<'_, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    let mut depth = 0_usize;
    *frames.first_mut().ok_or(RasterInputError::LimitExceeded)? = Some(document.layers.iter());
    loop {
        if control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth = depth.checked_sub(1).ok_or(RasterInputError::LimitExceeded)?;
            continue;
        };
        optional(layer.mask.as_ref().map(|mask| mask.surface.as_ref()), control)?;
        optional(layer.fill_cache.as_ref().map(|cache| &cache.surface), control)?;
        if let Some(video) = &layer.video {
            for frame in &video.frames {
                frame.try_validate_tile_origins_with(control)?;
            }
        }
        match &layer.content {
            LayerContent::BackedRaster(_) => return Err(RasterInputError::UnsupportedConversion),
            LayerContent::Raster(surface) => surface.try_validate_tile_origins_with(control)?,
            LayerContent::Text(text) => optional(text.cache.as_ref(), control)?,
            LayerContent::Shape(shape) => optional(shape.cache.as_ref(), control)?,
            LayerContent::Smart(smart) => {
                optional(smart.cache.as_ref(), control)?;
                optional(smart.filter_mask.as_ref().map(|mask| mask.surface.as_ref()), control)?;
            }
            LayerContent::Group(group) => {
                if depth >= MAX_GROUP_DEPTH {
                    return Err(RasterInputError::LimitExceeded);
                }
                if !group.children.is_empty() {
                    depth = depth.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
                    *frames.get_mut(depth).ok_or(RasterInputError::LimitExceeded)? = Some(group.children.iter());
                }
            }
            LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
        }
    }
    Ok(())
}

fn optional(surface: Option<&Surface>, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if let Some(surface) = surface {
        surface.try_validate_tile_origins_with(control)?;
    }
    Ok(())
}
