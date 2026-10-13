//! Current enabled-state consumption for an explicitly qualified empty selection.
//! This does not qualify a filter/epoch, nonempty coverage, profile, or source envelope.
use crate::gimp_composite::CompositionError;
pub use crate::gimp_filters::{RGB_CLIP_HDR_EPOCH, VALUE_INVERT_PRECISION_EPOCH, BRIGHTNESS_CONTRAST_PRECISION_EPOCH};
use photocraft_doc::{
    LayerId, PixelFormat, XcfState,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::FilterNode,
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct MaskStateLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptySelectionDisposition {
    EnabledEmptySelection,
    Disabled,
}
/// The caller must already qualify that source_region=0's empty selection means
/// unrestricted operation support for this exact operation/context.
#[derive(Clone, Copy, Debug)]
pub struct PreparedEmptySelection {
    id: MaskId,
    filter: FilterId,
    disposition: EmptySelectionDisposition,
}
impl PreparedEmptySelection {
    pub fn id(self) -> MaskId {
        self.id
    }
    pub fn filter(self) -> FilterId {
        self.filter
    }
    pub fn disposition(self) -> EmptySelectionDisposition {
        self.disposition
    }
    /// Disabled masks do not restrict operation support. The enabled branch is
    /// unrestricted ONLY because this token proves a qualified empty selection.
    /// No arbitrary zero/nonzero mask is converted into a general opacity factor.
    pub fn permits_unrestricted_support(self) -> bool {
        match self.disposition {
            EmptySelectionDisposition::Disabled => true,
            EmptySelectionDisposition::EnabledEmptySelection => true,
        }
    }
}
fn invalid(code: &'static str) -> CompositionError {
    CompositionError::Invalid(code)
}
/// Cheap UI declaration hint only: no tile samples, evaluator, profile or epoch
/// are qualified here. A candidate can still fail the checked engine command.
pub fn declared_empty_selection_candidate(state: &XcfState, node: &FilterNode, owner: LayerId) -> bool {
    if state.items.len() > 10_000 || state.filter_masks.len() > 10_000 || !node.enabled || node.source_region != Some(0) || owner.0 == 0 || node.id.0 == 0 {
        return false;
    }
    let Some(ItemRef::Mask(id)) = node.mask else {
        return false;
    };
    if id.0 == 0 {
        return false;
    }
    let mut masks = state.filter_masks.iter().filter(|mask| mask.id == id);
    let Some(mask) = masks.next() else {
        return false;
    };
    let format = mask.surface.format();
    if masks.next().is_some()
        || mask.owner != node.id
        || format.mode != photocraft_doc::ColorMode::Grayscale
        || format.alpha
        || mask.surface.tile_count() > 2
        || mask.surface.default_bytes().len() != format.bytes_per_pixel()
        || mask.surface.default_bytes().iter().any(|byte| *byte != 0)
        || checked_region_byte_len(mask.bounds, format).is_err()
    {
        return false;
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Mask(id));
    let Some(item) = items.next() else {
        return false;
    };
    let Some(metadata) = &item.mask else {
        return false;
    };
    items.next().is_none()
        && metadata.owner == ItemRef::Layer(owner)
        && metadata.bounds == mask.bounds
        && metadata.offset == [mask.bounds.x0, mask.bounds.y0]
        && metadata.outside_f64_bits == 0f64.to_bits()
        && item.raster_bounds.is_none_or(|bounds| bounds == mask.bounds)
        && !metadata.shown
        && !metadata.editing
        && !metadata.linked
}
fn poll(control: &Interrupt<'_>) -> Result<(), CompositionError> {
    control.check().map_err(|_| CompositionError::from(photocraft_raster::RasterInputError::Cancelled))
}
/// Native mask payloads are validated in both states, so disabling never conceals
/// malformed data that a later enable would encounter. Existing operation-specific
/// raw tuple/schema/profile/format/extent and operative-stack relationship guards
/// remain the caller's authority. The standalone mask qualifier does not discover
/// or admit a filter stack.
pub fn prepare_empty_selection(
    state: &XcfState,
    node: &FilterNode,
    owner: LayerId,
    expected_bounds: Rect,
    expected_format: PixelFormat,
    limits: MaskStateLimits,
    control: &Interrupt<'_>,
) -> Result<PreparedEmptySelection, CompositionError> {
    poll(control)?;
    if state.items.len() > 10_000 || state.filter_masks.len() > 10_000 || state.filter_stacks.len() > 10_000 {
        return Err(invalid("filter mask state inventory limit"));
    }
    let Some(ItemRef::Mask(id)) = node.mask else {
        return Err(invalid("filter mask state reference"));
    };
    if id.0 == 0 || node.id.0 == 0 || owner.0 == 0 || node.source_region != Some(0) {
        return Err(invalid("filter mask state identity or region"));
    }
    let mut masks = state.filter_masks.iter().filter(|mask| mask.id == id);
    let mask = masks.next().ok_or_else(|| invalid("filter mask state payload missing"))?;
    if masks.next().is_some()
        || mask.owner != node.id
        || mask.bounds != expected_bounds
        || mask.surface.format() != expected_format
        || expected_format.mode != photocraft_doc::ColorMode::Grayscale
        || expected_format.alpha
        || expected_bounds.is_empty()
    {
        return Err(invalid("filter mask state native domain"));
    }
    checked_region_byte_len(expected_bounds, expected_format)?;
    let tile_samples = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).ok_or_else(|| invalid("filter mask state budget"))?;
    let tile_bytes = tile_samples.checked_mul(expected_format.bytes_per_pixel() as u64).ok_or_else(|| invalid("filter mask state budget"))?;
    let tiles = mask.surface.tile_count() as u64;
    let resident = tiles
        .checked_mul(tile_bytes.checked_add(256).ok_or_else(|| invalid("filter mask state budget"))?)
        .and_then(|n| n.checked_add(mask.surface.default_bytes().len() as u64))
        .ok_or_else(|| invalid("filter mask state budget"))?;
    let work = tiles.checked_mul(tile_samples).and_then(|n| n.checked_add(1)).ok_or_else(|| invalid("filter mask state budget"))?;
    if limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || resident > limits.resident_bytes || work > limits.work_units {
        return Err(invalid("filter mask state budget"));
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Mask(id));
    let item = items.next().ok_or_else(|| invalid("filter mask state metadata missing"))?;
    let metadata = item.mask.as_ref().ok_or_else(|| invalid("filter mask state metadata missing"))?;
    if items.next().is_some()
        || metadata.owner != ItemRef::Layer(owner)
        || metadata.bounds != mask.bounds
        || metadata.offset != [mask.bounds.x0, mask.bounds.y0]
        || item.raster_bounds.is_some_and(|bounds| bounds != mask.bounds)
        || metadata.shown
        || metadata.editing
        || metadata.linked
    {
        return Err(invalid("filter mask state metadata relation"));
    }
    let outside =
        photocraft_color::try_read_sample_f64(mask.surface.default_bytes(), expected_format.sample, 0).map_err(|_| invalid("filter mask state default"))?;
    if outside.to_bits() != metadata.outside_f64_bits || outside.to_bits() != 0.0f64.to_bits() {
        return Err(invalid("filter mask state exact outside"));
    }
    mask.surface.try_validate_tile_origins_with(control)?;
    for (_, tile) in mask.surface.tiles() {
        poll(control)?;
        if tile.bytes().len() as u64 != tile_bytes {
            return Err(invalid("filter mask state tile length"));
        }
    }
    if !mask.is_empty_selection_with(control).map_err(|_| {
        if control.cancelled() { CompositionError::from(photocraft_raster::RasterInputError::Cancelled) } else { invalid("filter mask state samples") }
    })? {
        return Err(CompositionError::Unsupported("nonempty filter mask coverage is not qualified"));
    }
    poll(control)?;
    Ok(PreparedEmptySelection {
        id,
        filter: node.id,
        disposition: if metadata.enabled { EmptySelectionDisposition::EnabledEmptySelection } else { EmptySelectionDisposition::Disabled },
    })
}
