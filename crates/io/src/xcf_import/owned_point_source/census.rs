//! Storage-only census for a sealed, already source-qualified Point completion.
//! This cannot construct original-source, operation or renderer authority.
use super::super::owned_plain_source::census::{self as shared, Census};
use super::*;
use photocraft_doc::{
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterValue},
};

/// The actual current ItemState carries the original drawable rectangle. Never
/// infer it from sparse tile occupancy, default samples or the canvas rectangle.
pub(super) fn raster_bounds(doc: &Document, layer: photocraft_doc::LayerId) -> Result<Rect, IoError> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("Point state missing"))?;
    if doc.layers.len() != 1 || state.items.len() != 2 {
        return Err(invalid("Point singleton layer/mask shape"));
    }
    let mut found = None;
    let mut seen = false;
    for item in &state.items {
        if item.item == ItemRef::Layer(layer) {
            if seen || item.parent.is_some() || item.mask.is_some() {
                return Err(invalid("Point duplicate or nested layer item"));
            }
            seen = true;
            found = item.raster_bounds;
        }
    }
    let bounds = found.ok_or_else(|| invalid("Point operative layer bounds missing"))?;
    if bounds.is_empty()
        || bounds.width() > 262144
        || bounds.height() > 262144
        || [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().any(|n| !(-(1 << 29)..=(1 << 29)).contains(n))
    {
        return Err(invalid("Point operative layer bounds"));
    }
    Ok(bounds)
}

pub(super) fn shape(doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<(), IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    budget.consume_work(64, "Point closed storage shape").map_err(accounting_error)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("Point state missing"))?;
    if doc.size.width == 0
        || doc.size.height == 0
        || doc.size.width > 262144
        || doc.size.height > 262144
        || doc.layers.len() != 1
        || state.items.len() != 2
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() != 1
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(doc.depth, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
    {
        return Err(invalid("Point closed current storage shape"));
    }
    let layer = doc.layers.first().ok_or_else(|| invalid("Point layer missing"))?;
    raster_bounds(doc, layer.id)?;
    let stack = state.filter_stacks.first().ok_or_else(|| invalid("Point stack missing"))?;
    if stack.owner != ItemRef::Layer(layer.id) || stack.nodes.len() != 1 {
        return Err(invalid("Point singleton stack owner"));
    }
    let node = stack.nodes.first().ok_or_else(|| invalid("Point node missing"))?;
    let mask = state.filter_masks.first().ok_or_else(|| invalid("Point mask missing"))?;
    if node.arguments.len() > 10
        || !node.auxiliary.is_empty()
        || node.mask != Some(ItemRef::Mask(mask.id))
        || mask.owner != node.id
        || mask.bounds != doc.bounds()
        || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
    {
        return Err(invalid("Point flat arguments or native full-canvas mask"));
    }
    for argument in &node.arguments {
        match &argument.value {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) if f32::from_bits(*bits).is_finite() => {}
            FilterValue::Bool(_) | FilterValue::Enum { .. } => {}
            FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None }
                if components.len() == 4
                    && components.iter().all(
                        |v| matches!(v, ExactNumber::Float32Bits(bits) if f32::from_bits(*bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(*bits))),
                    ) => {}
            _ => return Err(invalid("Point bounded argument storage")),
        }
    }
    Ok(())
}

pub(super) fn measure(doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<Census, IoError> {
    shape(doc, budget, control)?;
    // Narrow shared exhaustive walker entry is registered with the Point lane.
    // Existing Plain/Alpha/Histogram shape gates and fees remain unchanged.
    shared::measure_point_storage(doc, budget, control)
}
