//! Storage-only census for a sealed, already source-qualified Point completion.
//! This cannot construct original-source, operation or renderer authority.
use super::super::owned_plain_source::census::{self as shared, Census};
use super::*;
use photocraft_doc::{item_identity::ItemRef, xcf_filters::{ExactNumber, FilterArgument, FilterValue}};

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
    // Reuse L1's exact operation-independent bounded storage shape and debit
    // every real validation step to the enclosing XCF ledger, without a second
    // counter/reset or asserted peak. The following walker counts actual spare
    // capacities (including the top Vec, whose API here is a borrowed slice).
    let failure = std::cell::RefCell::new(None);
    let debit = |units| budget.consume_work(units, "Point argument storage shape").map_err(|error| {
        let kind = if error.kind == ErrorKind::Cancelled { BackingError::Cancelled } else { BackingError::Work };
        *failure.borrow_mut() = Some(error);
        kind
    });
    let checked = photocraft_doc::accounted_document::point_argument_shape_with(&node.arguments, control, &debit);
    if let Some(error) = failure.into_inner() { return Err(accounting_error(error)); }
    checked.map_err(backing)?;
    finite_arguments(&node.arguments, budget, control)?;
    Ok(())
}

pub(super) fn measure(doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<Census, IoError> {
    shape(doc, budget, control)?;
    // Narrow shared exhaustive walker entry is registered with the Point lane.
    // Existing Plain/Alpha/Histogram shape gates and fees remain unchanged.
    shared::measure_point_storage(doc, budget, control)
}

// Shared L1 validation establishes bounded depth/counts first. This additional
// source lane keeps its pre-existing finite scalar/unit Color policy and the
// configured mapper's finite F64/curve words; raw L1 storage alone has no such
// numerical/source authority.
fn finite_arguments(arguments: &[FilterArgument], budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<(), IoError> {
    for argument in arguments {
        control.check().map_err(|_| IoError::Cancelled)?;
        budget.consume_work(1, "Point mapped finite argument").map_err(accounting_error)?;
        match &argument.value {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) if !f32::from_bits(*bits).is_finite() => return Err(invalid("Point finite scalar")),
            FilterValue::Number(ExactNumber::Float64Bits(bits)) if !f64::from_bits(*bits).is_finite() => return Err(invalid("Point finite configured scalar")),
            FilterValue::Color { components, .. } => {
                budget.consume_work(components.len() as u64, "Point mapped finite Color").map_err(accounting_error)?;
                if components.iter().any(|v| !matches!(v, ExactNumber::Float32Bits(bits) if f32::from_bits(*bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(*bits)))) {
                    return Err(invalid("Point finite unit Color"));
                }
            }
            FilterValue::Config(fields) => finite_arguments(fields, budget, control)?,
            FilterValue::Curve { points_f64_bits, samples_f64_bits, .. } => {
                for part in points_f64_bits.chunks(32) {
                    budget.consume_work(part.len() as u64 * 2, "Point mapped finite curve points").map_err(accounting_error)?;
                    control.check().map_err(|_| IoError::Cancelled)?;
                    if part.iter().flatten().any(|bits| !f64::from_bits(*bits).is_finite()) { return Err(invalid("Point finite curve point")); }
                }
                for part in samples_f64_bits.chunks(64) {
                    budget.consume_work(part.len() as u64, "Point mapped finite curve samples").map_err(accounting_error)?;
                    control.check().map_err(|_| IoError::Cancelled)?;
                    if part.iter().any(|bits| !f64::from_bits(*bits).is_finite()) { return Err(invalid("Point finite curve sample")); }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
