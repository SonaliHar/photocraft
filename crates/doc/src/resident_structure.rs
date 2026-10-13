//! Cooperative structural traversal; no raster sample validation or source access.
use crate::{Document, Layer, LayerContent, MAX_GROUP_DEPTH};
use photocraft_raster::{Interrupt, RasterInputError};
const MAX_VISITS: usize = 100_000;
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
/// Preorder traversal with the existing group-depth and model-census visit ceilings.
/// Returning false from the visitor stops traversal, after a final cancellation poll.
pub fn visit_layers_with(
    document: &Document,
    control: &Interrupt<'_>,
    mut visitor: impl FnMut(&Layer) -> Result<bool, RasterInputError>,
) -> Result<(), RasterInputError> {
    poll(control)?;
    let mut frames: [Option<std::slice::Iter<'_, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *frames.first_mut().ok_or(RasterInputError::InvalidBounds)? = Some(document.layers.iter());
    let mut depth = 0usize;
    let mut visits = 0usize;
    loop {
        poll(control)?;
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth = depth.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
            continue;
        };
        visits = visits.checked_add(1).filter(|n| *n <= MAX_VISITS).ok_or(RasterInputError::LimitExceeded)?;
        if !visitor(layer)? {
            return poll(control);
        }
        poll(control)?;
        if let LayerContent::Group(group) = &layer.content {
            depth = depth.checked_add(1).filter(|n| *n <= MAX_GROUP_DEPTH).ok_or(RasterInputError::LimitExceeded)?;
            *frames.get_mut(depth).ok_or(RasterInputError::InvalidBounds)? = Some(group.children.iter());
        }
    }
    poll(control)
}
/// Same backed-storage separation as Document::has_backed_rasters, cooperatively.
/// Other current raster owners contain resident Surface, not BackedLayer; their
/// owner traversal is bounded/polled without scanning samples or tile payloads.
pub fn has_backed_rasters_with(document: &Document, control: &Interrupt<'_>) -> Result<bool, RasterInputError> {
    let mut backed = false;
    visit_layers_with(document, control, |layer| {
        if matches!(layer.content, LayerContent::BackedRaster(_)) {
            backed = true;
            return Ok(false);
        }
        if layer.mask.is_some() {
            poll(control)?;
        }
        match &layer.content {
            LayerContent::Smart(smart) => {
                if smart.cache.is_some() {
                    poll(control)?;
                }
                if smart.filter_mask.is_some() {
                    poll(control)?;
                }
            }
            LayerContent::Text(text) if text.cache.is_some() => poll(control)?,
            _ => {}
        }
        Ok(true)
    })?;
    if backed {
        return Ok(true);
    }
    let mut owners = 0usize;
    for count in [
        document.channels.len(),
        document.patterns.len(),
        usize::from(document.quick_mask.is_some()),
        usize::from(document.selection.is_some()),
        document.xcf_state.as_ref().map_or(0, |state| state.filter_masks.len()),
    ] {
        owners = owners.checked_add(count).filter(|n| *n <= MAX_VISITS).ok_or(RasterInputError::LimitExceeded)?;
    }
    for _ in &document.channels {
        poll(control)?;
    }
    for _ in &document.patterns {
        poll(control)?;
    }
    if document.quick_mask.is_some() {
        poll(control)?;
    }
    if document.selection.is_some() {
        poll(control)?;
    }
    if let Some(state) = &document.xcf_state {
        for _ in &state.filter_masks {
            poll(control)?;
        }
    }
    poll(control)?;
    Ok(false)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorMode, LayerContent, SampleType, Size};
    #[test]
    fn cancel_is_observed_before_traversal_and_after_a_visited_owner() {
        let mut doc = Document::new("Own tree", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(Layer::new("Own layer", LayerContent::Raster(photocraft_raster::Surface::new(crate::PixelFormat::RGBA8))));
        let stop = || true;
        assert_eq!(has_backed_rasters_with(&doc, &Interrupt::cancel_only(&stop)), Err(RasterInputError::Cancelled));
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::Relaxed);
        assert_eq!(
            visit_layers_with(&doc, &Interrupt::cancel_only(&check), |_| {
                cancelled.store(true, Ordering::Relaxed);
                Ok(true)
            }),
            Err(RasterInputError::Cancelled)
        );
    }
}
