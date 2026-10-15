//! Separate CURRENT assigned-ICC Normal geometry qualification.
//! Original geometry epochs and raw policies are preserved verbatim. This
//! borrows an actual paid owner and grants no source/profile interpretation.
use crate::{backed_normal::BackedNormalError as E, gimp_composite::CompositionError};
use photocraft_cms::Profile;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    LayerContent, SampleEncoding, WorkingProfileInterpretation,
    accounted_document::AccountedDocument,
    import_compositing::{
        GimpOperator, GimpSpace, XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
    },
    item_identity::ItemRef,
};
use photocraft_raster::{
    backing::{BackingError, ReadControl, quota::QuotaKind},
    checked_view::CheckedRasterView,
};
/// Bounded route selection only; matching epochs grant no renderer/source proof.
pub(crate) fn is_candidate(doc: &photocraft_doc::Document, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    c.check(1)?;
    let Some(state) = doc.xcf_state.as_ref() else { return Ok(false) };
    if state.items.len() != 3 {
        return Ok(false);
    }
    let mut found = false;
    for item in &state.items {
        c.check(1)?;
        if let Some(policy) = &item.composition {
            if policy.compatibility_epoch.len() > 128 {
                return Ok(false);
            }
            c.check(policy.compatibility_epoch.len() as u64)?;
            found |= matches!(
                policy.compatibility_epoch.as_str(),
                XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
                    | XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
                    | XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH
                    | XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH
            );
        }
    }
    Ok(found)
}
fn scope() -> E {
    E::Unsupported("closed assigned nonlinear Normal Union geometry")
}
fn equal(a: &[u8], b: &[u8], c: &ReadControl<'_>) -> Result<bool, E> {
    c.check(1).map_err(E::Backing)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.chunks(4096).zip(b.chunks(4096)) {
        c.check(a.len() as u64 * 2).map_err(E::Backing)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn bounded(r: photocraft_geom::Rect) -> Result<u64, E> {
    let count = u64::from(r.width()).checked_mul(u64::from(r.height())).ok_or_else(scope)?;
    if r.is_empty() || count > 4096 || [r.x0, r.y0, r.x1, r.y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) {
        return Err(scope());
    }
    Ok(count)
}
fn encoding(v: GimpSpace) -> Result<SampleEncoding, E> {
    match v {
        GimpSpace::LinearRgb => Ok(SampleEncoding::Linear),
        GimpSpace::ProfileRgb => Ok(SampleEncoding::NonlinearProfile),
        _ => Err(scope()),
    }
}
fn scan(view: CheckedRasterView<'_>, c: &ReadControl<'_>) -> Result<(), E> {
    let bounds = view.logical_bounds();
    let format = view.format();
    let bpp = format.bytes_per_pixel();
    for coord in bounds.tiles() {
        c.check(1).map_err(E::Backing)?;
        let pin = view.pin_tile(coord, c).map_err(E::View)?;
        let part = coord.rect().intersect(&bounds);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                c.check(format.channels() as u64).map_err(E::Backing)?;
                let mut raw = [0; 40];
                let encoded = raw.get_mut(..bpp).ok_or_else(scope)?;
                pin.read_pixel_native_into(x, y, encoded, c).map_err(E::View)?;
                for channel in 0..format.channels() {
                    let value = photocraft_color::try_read_sample_f64(encoded, format.sample, channel)
                        .map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(scope());
                    }
                }
            }
        }
    }
    Ok(())
}
/// Exact CURRENT slice of the existing nonlinear assigned-ICC geometry policy.
/// The returned stored resolution is used by the existing NormalStack equations;
/// it is never recomputed from raw Auto words or immutable provenance.
pub(crate) fn qualify(owner: &AccountedDocument, profile: &Profile, c: &ReadControl<'_>) -> Result<GimpSpace, E> {
    qualify_inner(owner, profile, None, c)
}
/// Typed assigned original/effective-current relation. Matching the immutable
/// model and ledger is required; encoded bytes or epoch hints cannot fabricate
/// this binding. The generated effective definition supplies the renderer math.
/// CURRENT layer activity may differ from the source recipe; every layer and
/// mask is still validated, including hidden or zero-opacity native content.
pub(crate) fn qualify_assigned(
    owner: &AccountedDocument,
    binding: &crate::backed_assigned_current_profile::PreparedAssignedCurrentProfile,
    c: &ReadControl<'_>,
) -> Result<GimpSpace, E> {
    qualify_inner(owner, binding.profile(), Some(binding), c)
}
fn qualify_inner(
    owner: &AccountedDocument,
    profile: &Profile,
    binding: Option<&crate::backed_assigned_current_profile::PreparedAssignedCurrentProfile>,
    c: &ReadControl<'_>,
) -> Result<GimpSpace, E> {
    c.check(1).map_err(E::Backing)?;
    let sample_encoding = owner.document().sample_encoding.ok_or_else(scope)?;
    if let Some(binding) = binding {
        if !binding.matches(owner, sample_encoding) {
            return Err(scope());
        }
    } else if sample_encoding != SampleEncoding::NonlinearProfile {
        return Err(scope());
    }
    if binding.is_some() {
        owner.validate_assigned_analytic_geometry_storage(c).map_err(E::Backing)?;
    } else {
        owner.validate_masked_analytic_normal_storage(c).map_err(E::Backing)?;
    }
    let doc = owner.document();
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    c.check(state.opaque.len() as u64).map_err(E::Backing)?;
    if doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(
            (doc.depth, source.source_precision, sample_encoding),
            (SampleType::U16, 250, SampleEncoding::NonlinearProfile)
                | (SampleType::U32, 350, SampleEncoding::NonlinearProfile)
                | (SampleType::F32, 650, SampleEncoding::NonlinearProfile)
                | (SampleType::F64, 750, SampleEncoding::NonlinearProfile)
                | (SampleType::F32, 600, SampleEncoding::Linear)
                | (SampleType::F64, 700, SampleEncoding::Linear)
                | (SampleType::F32, 675, SampleEncoding::Perceptual)
                | (SampleType::F64, 775, SampleEncoding::Perceptual)
        )
        || state.provenance.format_version != 12
        || doc.layers.len() != 2
        || state.items.len() != 3
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || state.opaque.iter().any(|r| r.rendering_required)
    {
        return Err(scope());
    }
    let current = doc.icc_profile.as_deref().ok_or_else(scope)?;
    let original = source.original_icc.as_deref().ok_or_else(scope)?;
    if !(132..=65536).contains(&current.len())
        || !(132..=65536).contains(&original.len())
        || profile.channels() != doc.mode.color_channels()
        || profile.description.len() > 65536
        || profile.copyright.len() > 65536
        || (binding.is_none() && (!equal(current, original, c)? || !equal(current, profile.encoded_bytes().ok_or_else(scope)?, c)?))
    {
        return Err(scope());
    }
    if binding.is_none() {
        // Existing bare-profile route remains nonlinear and exactly byte-bound.
        let work = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(profile).map_err(|error| match error {
            photocraft_cms::control::Error::Cancelled => E::Backing(BackingError::Cancelled),
            photocraft_cms::control::Error::Limit(message) => E::Unsupported(message),
            photocraft_cms::control::Error::Allocation => E::Allocation,
            photocraft_cms::control::Error::Cms(error) => E::Color(CompositionError::Color(error)),
        })?;
        c.check(work).map_err(E::Backing)?;
        photocraft_cms::xcf_color::qualify_effective_analytic_profile(profile, photocraft_cms::xcf_color::Encoding::NonlinearProfile)
            .map_err(|e| E::Color(CompositionError::Color(e)))?;
    }
    // Typed preparation already qualified both the original and the generated
    // effective definition. Do not reparse current bytes or recompute a relation.
    let expected = match (doc.depth, sample_encoding) {
        (SampleType::U16 | SampleType::U32, SampleEncoding::NonlinearProfile) => XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH,
        (_, SampleEncoding::NonlinearProfile) => XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
        (_, SampleEncoding::Linear) => XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        (_, SampleEncoding::Perceptual) => XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
    };
    let canvas = bounded(doc.bounds())?;
    let mut aggregate = canvas;
    let mut composite = None;
    // All profile/model checks above precede this real pin/scratch/header fee.
    let _scratch = owner
        .quota()
        .reserve(QuotaKind::Resident, (std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() + 40 + 128) as u64)
        .map_err(E::Quota)?;
    for (index, layer) in doc.layers.iter().enumerate() {
        c.check(4096).map_err(E::Backing)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
        if !owner.quota().shares_ledger(backed.quota()) || backed.format() != doc.pixel_format() || !backed.format().alpha {
            return Err(scope());
        }
        let extent = backed.operative_bounds();
        let pixels = bounded(extent)?;
        let item = state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        let resolved = policy.checked_resolved().map_err(|_| scope())?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if item.parent.is_some()
            || item.raster_bounds != Some(extent)
            || item.visible != layer.visible
            || (binding.is_none() && !layer.visible)
            || policy.compatibility_epoch != expected
            || policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || (policy.stored_blend_space, policy.stored_composite_space) != (0, if index == 0 { -1 } else { 1 })
            || if index == 0 {
                policy.stored_operator != -1
            } else if binding.is_some() {
                !(1..=4).contains(&policy.stored_operator)
            } else {
                policy.stored_operator != 1
            }
            || ((index == 0 || binding.is_none()) && resolved.operator != GimpOperator::Union)
            || resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || physical.pass_through
            || physical.blend_encoding != encoding(resolved.blend_space)?
            || physical.composite_encoding != encoding(resolved.composite_space)?
            || layer.blend != photocraft_color::blend::BlendMode::Normal
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
            || layer.indexed_source.is_some()
            || layer.video.is_some()
            || layer.fill_cache.is_some()
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
        {
            return Err(scope());
        }
        if composite.is_some_and(|space| space != resolved.composite_space) {
            return Err(scope());
        }
        composite = Some(resolved.composite_space);
        if index == 0 {
            if extent != doc.bounds() || (binding.is_none() && layer.opacity.to_bits() != 1f32.to_bits()) || layer.mask.is_some() {
                return Err(scope());
            }
        } else {
            aggregate = aggregate.checked_add(pixels.checked_mul(2).ok_or_else(scope)?).ok_or_else(scope)?;
            if aggregate > 8192 {
                return Err(scope());
            }
            let mask = layer.mask.as_ref().ok_or_else(scope)?;
            let mut records = state.items.iter().filter(|i| i.mask.as_ref().is_some_and(|m| m.owner == ItemRef::Layer(layer.id)));
            let record = records.next().ok_or_else(scope)?;
            if records.next().is_some() || !matches!(record.item, ItemRef::Mask(_)) {
                return Err(scope());
            }
            let meta = record.mask.as_ref().ok_or_else(scope)?;
            if !mask.enabled
                || mask.density.to_bits() != 1f32.to_bits()
                || mask.feather.to_bits() != 0f32.to_bits()
                || mask.surface.format() != photocraft_color::PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
                || meta.bounds != extent
                || meta.offset != [extent.x0, extent.y0]
                || meta.outside_f64_bits != 0f64.to_bits()
                || !meta.enabled
                || meta.shown
                || meta.editing
                || meta.linked != mask.linked
            {
                return Err(scope());
            }
            let view = CheckedRasterView::resident_with_control(&mask.surface, meta.bounds, c).map_err(E::View)?;
            scan(view, c)?;
        }
        scan(backed.checked_view(), c)?;
    }
    c.check(0).map_err(E::Backing)?;
    composite.ok_or_else(scope)
}
#[cfg(test)]
#[path = "backed_analytic_geometry_tests.rs"]
mod tests;
