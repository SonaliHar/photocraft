//! Closed current-model profile/tuple admission for raw active backed sampling.
//! This does not qualify an XCF compositor, runtime Babl space or source renderer.
use crate::{DocumentSnapshot, EngineError, Result};
use photocraft_doc::{
    Document, Layer, LayerContent, SampleEncoding, SampleType, WorkingProfileInterpretation,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_raster::backing::{BackingError, ReadControl, quota::QuotaKind};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Policy {
    SourceFree,
    OriginalExact,
    MaterializedImplicitSrgb,
}
fn bad(msg: &str) -> EngineError {
    EngineError::Other(format!("backed imported sampling: {msg}"))
}
/// Bounded metadata only. Original bytes and implicit hashes are checked by the
/// worker before source pixels; mere presence of source metadata is no grant.
pub(crate) fn metadata(doc: &Document) -> Result<Policy> {
    let (state, context) = match (&doc.xcf_state, &doc.source_color_context) {
        (None, None) => return Ok(Policy::SourceFree),
        (Some(state), Some(context)) => (state, context),
        _ => return Err(bad("incomplete imported context")),
    };
    if doc.mode != photocraft_doc::ColorMode::Rgb
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!((context.source_precision, doc.depth), (150, SampleType::U8) | (250, SampleType::U16) | (650, SampleType::F32))
        || doc.selection_channel_id.is_some()
        || doc.quick_mask.is_some()
        || !doc.channels.is_empty()
        || doc.layers.is_empty()
        || doc.layers.len() > 4096
        || state.items.len() != doc.layers.len()
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
    {
        return Err(bad("unsupported current imported owner/precision"));
    }
    let current = doc.icc_profile.as_ref().filter(|p| !p.is_empty() && p.len() <= 16 << 20).ok_or_else(|| bad("current ICC missing or unbounded"))?;
    match (&context.original_icc, &doc.working_profile_interpretation) {
        (Some(original), WorkingProfileInterpretation::Icc) if !original.is_empty() && original.len() <= 16 << 20 && original.len() == current.len() => {
            Ok(Policy::OriginalExact)
        }
        (None, WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 }) if icc_blake3.len() == 64 && icc_blake3.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(Policy::MaterializedImplicitSrgb)
        }
        _ => Err(bad("explicit original or materialized implicit ICC witness required")),
    }
}
fn encoding(space: GimpSpace) -> Result<SampleEncoding> {
    match space {
        GimpSpace::LinearRgb => Ok(SampleEncoding::Linear),
        GimpSpace::ProfileRgb => Ok(SampleEncoding::NonlinearProfile),
        GimpSpace::PerceptualRgb => Ok(SampleEncoding::Perceptual),
        GimpSpace::Lab => Err(bad("Lab tuple is unsupported")),
    }
}
/// Retain the exact current tuple without evaluating its blend/operator. Active
/// sampling reads base pixels just as the upstream resident source_pixel route.
pub(crate) fn layer(doc: &Document, layer: &Layer) -> Result<()> {
    if metadata(doc)? == Policy::SourceFree {
        if layer.import_compositing.is_some() {
            return Err(bad("source-free imported tuple"));
        }
        return Ok(());
    }
    let LayerContent::BackedRaster(backed) = &layer.content else {
        return Err(bad("native layer required"));
    };
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("state missing"))?;
    let mut matches = state.items.iter().filter(|item| item.item == ItemRef::Layer(layer.id));
    let item = matches.next().ok_or_else(|| bad("canonical layer missing"))?;
    if matches.next().is_some() || item.parent.is_some() || item.mask.is_some() || item.raster_bounds != Some(backed.operative_bounds()) {
        return Err(bad("canonical layer geometry/identity mismatch"));
    }
    let stored = item.composition.as_ref().ok_or_else(|| bad("composition witness missing"))?;
    if !matches!(stored.source_mode, 0 | 28) || stored.pass_through || stored.legacy != (stored.source_mode == 0) || stored.compatibility_epoch.len() > 256 {
        return Err(bad("plain source tuple required"));
    }
    let resolved = stored.checked_resolved().map_err(|_| bad("invalid retained composition tuple"))?;
    if resolved.operator != GimpOperator::Union {
        return Err(bad("plain Normal union tuple required"));
    }
    let actual = layer.import_compositing.as_ref().ok_or_else(|| bad("current tuple missing"))?;
    if actual.pass_through || actual.blend_encoding != encoding(resolved.blend_space)? || actual.composite_encoding != encoding(resolved.composite_space)? {
        return Err(bad("current tuple contradicts retained resolution"));
    }
    Ok(())
}
/// Complete typed copier admission and profile proof, before any native read.
pub(crate) fn verify(source: &DocumentSnapshot, policy: Policy, control: &ReadControl<'_>) -> Result<()> {
    control.check(1)?;
    if metadata(source)? != policy {
        return Err(bad("captured profile policy changed"));
    }
    if policy == Policy::SourceFree {
        return Ok(());
    }
    source.accounted().ok_or(BackingError::Unsupported)?.validate_selection_replacement(control)?;
    let context = source.source_color_context.as_ref().ok_or_else(|| bad("source context missing"))?;
    let current = source.icc_profile.as_ref().ok_or_else(|| bad("ICC missing"))?;
    if policy == Policy::OriginalExact {
        let original = context.original_icc.as_ref().ok_or_else(|| bad("original ICC missing"))?;
        if original.len() != current.len() {
            return Err(bad("original/current ICC differ"));
        }
        for (a, b) in original.chunks(32768).zip(current.chunks(32768)) {
            control.check((a.len() as u64) * 2)?;
            if a != b {
                return Err(bad("original/current ICC differ"));
            }
        }
        control.check(1)?;
        return Ok(());
    }
    control.check((current.len() as u64).checked_add(256).ok_or(BackingError::Length)?)?;
    source
        .working_profile_interpretation
        .validate_with_interrupt(Some(current), source.mode, control.interrupt)
        .map_err(|_| if control.interrupt.cancelled() { EngineError::Cancelled } else { bad("implicit marker is stale") })?;
    verify_implicit(current, source, control)
}
fn cms_error(error: photocraft_cms::control::Error) -> EngineError {
    if matches!(error, photocraft_cms::control::Error::Cancelled) { EngineError::Cancelled } else { bad(&error.to_string()) }
}
fn verify_implicit(current: &[u8], source: &DocumentSnapshot, control: &ReadControl<'_>) -> Result<()> {
    let cancelled = || control.interrupt.cancelled();
    let maximum = photocraft_cms::control::Limits::default();
    // This fixed-shape prospective helper inspects actual builtin capacities;
    // it neither infers an implicit profile nor evaluates curve tables.
    control.check(4096)?;
    let ceiling = photocraft_cms::xcf_color::effective_implicit_rgb_nonlinear_planner_work_with_control(
        current.len(),
        photocraft_cms::control::Limits { work_units: 4096, ..maximum },
        &cancelled,
    )
    .map_err(cms_error)?;
    let lower = photocraft_cms::control::Limits { work_units: ceiling, ..maximum };
    control.check(ceiling)?;
    let requirement = photocraft_cms::xcf_color::effective_implicit_profile_requirements_with_control(
        current,
        photocraft_cms::ColorSpace::Rgb,
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        lower,
        &cancelled,
    )
    .map_err(cms_error)?;
    if requirement.work_units > ceiling {
        return Err(bad("implicit requirement exceeded prospective work"));
    }
    let lease = source
        .quota()
        .ok_or(BackingError::Unsupported)?
        .reserve(QuotaKind::Resident, requirement.peak_resident_bytes as u64)
        .map_err(|_| BackingError::Quota)?;
    // Actual constructor re-runs implicit/effective admission, then clones and
    // compares bytes. Each nested phase is prepaid on the same caller ledger.
    control.check(ceiling.checked_mul(3).ok_or(BackingError::Length)?)?;
    let actual = photocraft_cms::xcf_color::effective_implicit_profile_with_control(
        current,
        photocraft_cms::ColorSpace::Rgb,
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        photocraft_cms::control::Limits { resident_bytes: requirement.peak_resident_bytes, work_units: requirement.work_units },
        &cancelled,
    )
    .map_err(cms_error)?;
    control.check(ceiling)?;
    let actual_requirement = actual
        .clone_requirements_with_control(
            photocraft_cms::control::Limits { resident_bytes: requirement.peak_resident_bytes, work_units: requirement.work_units },
            &cancelled,
        )
        .map_err(cms_error)?;
    if actual_requirement.retained_bytes > requirement.retained_bytes || actual_requirement.work_units > requirement.work_units {
        return Err(bad("implicit actual profile exceeded admitted envelope"));
    }
    // The verifier compares its actual generated retained bytes internally.
    // Keep its temporary Profile alive until before the physical lease drops.
    control.check(1)?;
    drop(actual);
    drop(lease);
    Ok(())
}
#[cfg(test)]
#[path = "imported_backed_sampling_tests.rs"]
mod tests;
