//! Fixed one-level owned promotion. The sealed source supplied eligibility;
//! these visits transfer actual three-leaf storage without a group projection.
use super::*;

fn layers(doc: &Document) -> impl Iterator<Item = &photocraft_doc::Layer> {
    doc.layers.iter().flat_map(|layer| {
        let children = match &layer.content {
            LayerContent::Group(group) => group.children.as_slice(),
            _ => &[],
        };
        std::iter::once(layer).chain(children)
    })
}
pub(super) fn phase_work(doc: &Document, model: u64, payload: u64, source: u64, budget: &photocraft_xcf::Budget<'_>) -> Result<u64, IoError> {
    let mut native = 0u64;
    let mut masks = 0u64;
    for layer in layers(doc) {
        budget.consume_work(1, "isolated group physical work census").map_err(accounting_error)?;
        if let LayerContent::Raster(surface) = &layer.content {
            let bytes = (surface.tile_count() as u64)
                .checked_mul(65536)
                .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64))
                .and_then(|n| n.checked_add(surface.default_bytes().len() as u64))
                .ok_or_else(|| invalid("isolated group native work"))?;
            native = add(native, bytes)?;
        }
        if let Some(mask) = &layer.mask {
            let bytes = (mask.surface.tile_count() as u64)
                .checked_mul(65536)
                .and_then(|n| n.checked_mul(mask.surface.format().bytes_per_pixel() as u64))
                .and_then(|n| n.checked_add(mask.surface.default_bytes().len() as u64))
                .ok_or_else(|| invalid("isolated group mask work"))?;
            masks = add(masks, bytes)?;
        }
    }
    // Includes physical padding/default checks, repeated complete model/mask
    // custody gates, ID scans and all store/model headers on this fixed shape.
    let retained = add(add(model, payload)?, source)?.checked_mul(8).ok_or_else(|| invalid("isolated group retained work"))?;
    add(
        add(
            native.checked_mul(4).ok_or_else(|| invalid("isolated group native work"))?,
            masks.checked_mul(8).ok_or_else(|| invalid("isolated group mask work"))?,
        )?,
        add(retained, 65536)?,
    )
}
pub(super) fn validate(doc: &Document, read: &ReadControl<'_>, poll: &dyn Fn() -> Result<(), BackingError>) -> Result<(), IoError> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("isolated group state"))?;
    let mut leaves = 0usize;
    for layer in layers(doc) {
        poll().map_err(backing)?;
        read.check(1).map_err(backing)?;
        match &layer.content {
            LayerContent::Raster(surface) => {
                store::validate(surface, layer_bounds(state, layer.id, read, poll).map_err(backing)?, read, poll).map_err(backing)?;
                leaves = leaves.checked_add(1).ok_or_else(|| invalid("isolated group leaf count"))?;
            }
            LayerContent::Group(group)
                if group.children.len() == 2
                    && group.artboard.is_none()
                    && group.children.iter().all(|child| matches!(child.content, LayerContent::Raster(_))) => {}
            _ => return Err(invalid("isolated group authoritative content")),
        }
    }
    if leaves != 3 {
        return Err(invalid("isolated group authoritative leaf count"));
    }
    poll().map_err(backing)?;
    Ok(())
}
pub(super) fn promote(
    roots: &mut [photocraft_doc::Layer],
    state: &photocraft_doc::XcfState,
    retention: &Arc<Retention>,
    quota: &BackingQuota,
    read: &ReadControl<'_>,
    poll: &dyn Fn() -> Result<(), BackingError>,
) -> Result<(), IoError> {
    for root in roots {
        poll().map_err(backing)?;
        if let LayerContent::Group(group) = &mut root.content {
            if group.children.len() != 2 || group.artboard.is_some() {
                return Err(invalid("isolated group promotion shape"));
            }
            for child in &mut group.children {
                leaf(child, state, retention, quota, read, poll)?;
            }
        } else {
            leaf(root, state, retention, quota, read, poll)?;
        }
    }
    poll().map_err(backing)?;
    Ok(())
}
fn leaf(
    layer: &mut photocraft_doc::Layer,
    state: &photocraft_doc::XcfState,
    retention: &Arc<Retention>,
    quota: &BackingQuota,
    read: &ReadControl<'_>,
    poll: &dyn Fn() -> Result<(), BackingError>,
) -> Result<(), IoError> {
    poll().map_err(backing)?;
    read.check(1).map_err(backing)?;
    let bounds = layer_bounds(state, layer.id, read, poll).map_err(backing)?;
    let placeholder = LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: false, artboard: None });
    let LayerContent::Raster(surface) = std::mem::replace(&mut layer.content, placeholder) else {
        return Err(invalid("isolated group leaf moved twice"));
    };
    let descriptor = photocraft_raster::backing::NativePlaneDescriptor::new(surface.format(), bounds, surface.default_bytes()).map_err(backing)?;
    let store = Arc::new(store::SurfaceStore::new(surface, descriptor, retention.clone(), quota.clone()));
    let raster = BackedRaster::new(store, quota.clone(), 1, read).map_err(backing)?;
    layer.content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, read).map_err(|error| match error {
        photocraft_doc::BackedLayerError::Backing(error) => backing(error),
        _ => backing(BackingError::Integrity),
    })?);
    poll().map_err(backing)?;
    Ok(())
}
