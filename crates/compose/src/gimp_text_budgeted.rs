//! Caller-leased preparation of the existing single v19 exact-font text domain.
//! The source lease remains responsible for the complete borrowed document.
#[cfg(test)]
#[path = "gimp_text_budgeted_tests.rs"]
mod tests;
use super::{PreparationBudget, Prepared, PreparedRenderer, WorkingSpace};
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{
    ColorMode, Document, SampleEncoding, SampleType, WorkingProfileInterpretation,
    xcf_filters::{FilterArgument, FilterValue},
    xcf_items::EditableSource,
};
use photocraft_raster::{Interrupt, RasterInputError};
use photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont;

// This is the actual cap used by both existing IO text preparation and the
// compositor's rectilinear/underline preparation, not a font-size estimate.
const TEXT_BYTES: u64 = 8 << 20;
fn invalid() -> CompositionError {
    CompositionError::Unsupported("budgeted single v19 exact-font text preparation")
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(invalid)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(invalid)
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| CompositionError::Raster(RasterInputError::Cancelled))
}
pub(super) fn supports(doc: &Document) -> bool {
    doc.layers.len() == 1
        && doc.mode == ColorMode::Rgb
        && doc.depth == SampleType::U8
        && doc.sample_encoding == Some(SampleEncoding::NonlinearProfile)
        && matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. })
        && doc.xcf_state.as_ref().is_some_and(|s| {
            s.provenance.format_version == 19
                && s.source_items.len() == 1
                && matches!(s.source_items.first().map(|s| &s.source), Some(EditableSource::Text { source_modified: false, .. }))
        })
}
// The admitted v19 Normal28 Auto tuple (0,-1,-1) resolves to Linear in
// the existing importer. Stored nonlinear pixel encoding is a separate role.
fn normal_text_resolution(policy: &photocraft_doc::import_compositing::GimpComposition) -> bool {
    policy.resolved.as_ref().is_some_and(|resolved| {
        resolved.blend_space == photocraft_doc::import_compositing::GimpSpace::LinearRgb
            && resolved.composite_space == photocraft_doc::import_compositing::GimpSpace::LinearRgb
            && resolved.operator == photocraft_doc::import_compositing::GimpOperator::Union
    })
}
fn text_bytes(text: &str, budget: &mut impl PreparationBudget) -> Result<u64> {
    if text.len() > 1 << 20 {
        return Err(invalid());
    }
    budget.consume_preparation_work(mul(text.len() as u64, 4)?)?;
    Ok(text.len() as u64)
}
fn arguments(values: &[FilterArgument], depth: usize, visits: &mut usize, budget: &mut impl PreparationBudget, ctl: &Interrupt<'_>) -> Result<u64> {
    if depth > 8 || values.len() > 128 {
        return Err(invalid());
    }
    let mut bytes = 0;
    for arg in values {
        budget.consume_preparation_work(128)?;
        poll(ctl)?;
        *visits = visits.checked_add(1).filter(|n| *n <= 4096).ok_or_else(invalid)?;
        bytes = add(bytes, text_bytes(&arg.name, budget)?)?;
        bytes = add(
            bytes,
            match &arg.value {
                FilterValue::String(v) | FilterValue::Enum { domain: v, .. } => text_bytes(v, budget)?,
                FilterValue::Config(v) => arguments(v, depth + 1, visits, budget, ctl)?,
                FilterValue::Number(_) | FilterValue::Bool(_) | FilterValue::Resource(_) => 0,
                FilterValue::Color { components, .. } if components.len() <= 4 => 0,
                // Text has no graph/curve or uninterpreted native-color consumer.
                _ => return Err(invalid()),
            },
        )?;
    }
    Ok(bytes)
}
pub(super) fn reserve<B: PreparationBudget>(doc: &Document, font: &PreparedRuntimeFont<'_>, budget: &mut B, ctl: &Interrupt<'_>) -> Result<B::Lease> {
    poll(ctl)?;
    if !supports(doc) {
        return Err(invalid());
    }
    let state = doc.xcf_state.as_ref().ok_or_else(invalid)?;
    if state.items.len() != 1
        || state.resources.len() != 1
        || state.opaque.len() > 256
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.selection.is_some()
        || doc.quick_mask.is_some()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
    {
        return Err(invalid());
    }
    let source = state.source_items.first().ok_or_else(invalid)?;
    let layer = doc.layers.first().ok_or_else(invalid)?;
    let surface = layer.surface().ok_or_else(invalid)?;
    if surface.tile_count() > 3 || !matches!(layer.content, photocraft_doc::LayerContent::Raster(_)) {
        return Err(invalid());
    }
    let policy = state.items.first().and_then(|item| item.composition.as_ref()).ok_or_else(invalid)?;
    if !normal_text_resolution(policy) {
        return Err(invalid());
    }
    let EditableSource::Text { content, markup, configuration, .. } = &source.source else { return Err(invalid()) };
    let cache = source.cache.as_ref().ok_or_else(invalid)?;
    if cache.width > 512 || cache.height > 102 || cache.bytes.len() > 512 * 102 * 4 {
        return Err(invalid());
    }
    let resource = state.resources.first().ok_or_else(invalid)?;
    if resource.owned_bytes.is_some() {
        return Err(invalid());
    }
    // The namespace validators allocate sets for these bounded actual records.
    // Charge index/header room before they can run; source payload stays borrowed.
    let records = add(add(state.items.len() as u64, state.resources.len() as u64)?, add(state.source_items.len() as u64, state.opaque.len() as u64)?)?;
    budget.consume_preparation_work(mul(records, 128)?)?;
    let mut input = add(cache.bytes.capacity() as u64, text_bytes(content, budget)?)?;
    for value in [
        &resource.name,
        &state.items.first().ok_or_else(invalid)?.name,
        &state.provenance.format,
        &state.provenance.container,
        &state.provenance.source_label,
        &cache.blake3,
    ] {
        input = add(input, text_bytes(value, budget)?)?;
    }
    for value in [
        resource.collection.as_ref(),
        resource.locator.as_ref(),
        resource.content_hash.as_ref(),
        resource.resolution.as_ref().map(|r| &r.payload_hash),
        state.provenance.producer.as_ref(),
        state.provenance.release_id.as_ref(),
        state.provenance.source_hash.as_ref(),
        state.provenance.original_locator.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        input = add(input, text_bytes(value, budget)?)?;
    }
    if state.provenance.protected_source_fingerprints.len() > 256 {
        return Err(invalid());
    }
    for value in &state.provenance.protected_source_fingerprints {
        poll(ctl)?;
        input = add(input, text_bytes(value, budget)?)?;
    }
    if let Some(markup) = markup {
        input = add(input, text_bytes(markup, budget)?)?;
    }
    let mut visits = 0;
    input = add(input, arguments(configuration, 0, &mut visits, budget, ctl)?)?;
    for raw in &state.opaque {
        poll(ctl)?;
        budget.consume_preparation_work(add(128, mul(raw.bytes.len() as u64, 4)?)?)?;
        input = add(input, raw.bytes.capacity() as u64)?;
        for value in [&raw.namespace, &raw.identity, &raw.blake3] {
            input = add(input, text_bytes(value, budget)?)?;
        }
    }
    let icc = doc.icc_profile.as_ref().ok_or_else(invalid)?;
    if !(132..=65536).contains(&icc.len()) {
        return Err(invalid());
    }
    let limits = photocraft_cms::control::Limits::default();
    // Prospective CMS traversal uses the same caller's work budget, before it
    // traverses the builtin definition. No new independent aggregate is made.
    budget.consume_preparation_work(limits.work_units)?;
    let cms = photocraft_cms::xcf_color::effective_implicit_profile_requirements_with_control(
        icc,
        photocraft_cms::ColorSpace::Rgb,
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        limits,
        &|| ctl.cancelled(),
    )
    .map_err(|e| {
        if ctl.cancelled() {
            CompositionError::Raster(RasterInputError::Cancelled)
        } else {
            match e {
                photocraft_cms::control::Error::Cms(e) => CompositionError::Color(e),
                _ => invalid(),
            }
        }
    })?;
    let req = font.requirements();
    // At most sixteen glyphs across two lines; four qualification/preparation
    // passes are conservatively priced against the verified encoded face bound.
    budget
        .consume_preparation_work(add(mul(req.work_units, 64)?, add(mul(cms.work_units, 4)?, add(mul(cache.bytes.len() as u64, 8)?, 3 * 256 * 256 * 8)?)?)?)?;
    let index = mul(add(records, visits as u64)?, 256)?;
    let headers = (std::mem::size_of::<PreparedRenderer<'_>>()
        + 2 * std::mem::size_of::<Prepared<'_>>()
        + 4 * std::mem::size_of::<(photocraft_doc::import_compositing::GimpSpace, WorkingSpace)>()
        + 4096) as u64;
    let bytes = add(TEXT_BYTES, add(req.peak_bytes, add(mul(cms.peak_resident_bytes as u64, 4)?, add(input, add(index, headers)?)?)?)?)?;
    budget.reserve_prepared_bytes(bytes)
}
/// Repeats the existing IO builder checks only after the caller lease exists.
pub(super) fn validate_text(doc: &Document, font: &PreparedRuntimeFont<'_>, budget: &mut impl PreparationBudget, ctl: &Interrupt<'_>) -> Result<()> {
    let source = doc.xcf_state.as_ref().and_then(|s| s.source_items.first()).ok_or_else(invalid)?;
    let used = std::cell::Cell::new(0u64);
    let mut charge = |n| {
        let next = used.get().checked_add(n).filter(|n| *n <= TEXT_BYTES).ok_or(photocraft_text::xcf::TextSourceError::Budget)?;
        used.set(next);
        Ok(())
    };
    let result = if matches!(source.source, EditableSource::Text { markup: Some(_), .. }) {
        let q = crate::gimp_text_items::qualify_underlined_source_with_authorized_font(doc, source, font, ctl)?;
        let mut failure = None;
        let prepared = photocraft_text::xcf::underline::prepare_underlined_with_charge(font.exact_font(), q, TEXT_BYTES, ctl, &mut charge, &mut |n| {
            budget.consume_preparation_work(n).map_err(|e| {
                failure = Some(e);
                photocraft_text::xcf::TextSourceError::Budget
            })
        });
        if let Some(error) = failure {
            return Err(error);
        }
        prepared
    } else {
        let q = crate::gimp_text_items::qualify_layout_source_with_authorized_font(doc, source, font, ctl)?;
        photocraft_text::xcf::prepare_qualified_rectilinear_with_charge(font.exact_font(), q, TEXT_BYTES, ctl, &mut charge)
    };
    let _prepared = result
        .map_err(|e| if e == photocraft_text::xcf::TextSourceError::Cancelled { CompositionError::Raster(RasterInputError::Cancelled) } else { invalid() })?;
    poll(ctl)
}
