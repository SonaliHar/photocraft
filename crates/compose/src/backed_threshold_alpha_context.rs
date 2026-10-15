//! Explicit CURRENT operative/default Threshold Alpha recipe over a real
//! read-only accounted storage owner. This is not original-source equivalence,
//! source eligibility, a full raw-padding proof or a default renderer enrollment.
//! Native samples are validated only in requested bands; raw off-plane backing
//! bytes stay inert and unchanged. The existing resident renderer keeps its full
//! Surface/default/tile/unit qualification separately.
use crate::{
    backed_normal::{BackedNormalError, NormalBandLimits},
    backed_point_filter::{BackedPointError, OwnedNativePointBand, PreparedBackedThresholdAlpha},
    gimp_composite::CompositionError,
};
use photocraft_cms::{Builtin, Profile};
use photocraft_doc::{
    ColorMode, LayerContent, LayerId, SampleEncoding, WorkingProfileInterpretation,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::{FilterId, ItemRef},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::backing::{
    NativePlaneDescriptor, ReadControl,
    quota::{QuotaKind, QuotaLease},
};

/// Borrows the actual immutable storage owner and checks an encoded current-profile
/// witness during preparation. No profile is parsed, encoded, cloned or inferred from source epochs.
/// There is one enabled filter, an empty physical selection mask and Normal/Union
/// at opacity one. Returned bands are straight native current owner samples,
/// not a display composite, Surface, persistence receipt or editable result.
pub struct PreparedBackedThresholdAlphaContext<'a> {
    native: PreparedBackedThresholdAlpha<'a>,
    _owner: &'a AccountedDocument,
    layer: LayerId,
    node: FilterId,
    descriptor: NativePlaneDescriptor,
    _lease: QuotaLease,
}
fn scope() -> BackedNormalError {
    BackedNormalError::Unsupported("closed enabled current backed Threshold Alpha operative/default recipe")
}
fn paid_bytes_equal(a: &[u8], b: &[u8], control: &ReadControl<'_>) -> Result<bool, BackedNormalError> {
    control.check(1).map_err(BackedNormalError::Backing)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.chunks(4096).zip(b.chunks(4096)) {
        control.check(a.len() as u64).map_err(BackedNormalError::Backing)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn profile(owner: &AccountedDocument, supplied: &Profile, control: &ReadControl<'_>) -> Result<&'static Profile, BackedNormalError> {
    let doc = owner.document();
    let current = doc.icc_profile.as_ref().ok_or_else(scope)?;
    let retained = supplied.encoded_bytes().ok_or(BackedNormalError::Unsupported("cold current Profile encoding is not admitted"))?;
    if !(132..=16 << 20).contains(&current.len())
        || supplied.channels() != if doc.mode == ColorMode::Rgb { 3 } else { 1 }
        || !paid_bytes_equal(current, retained, control)?
    {
        return Err(scope());
    }
    let builtin = match (&doc.working_profile_interpretation, doc.mode) {
        (WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb) => Builtin::Srgb,
        (WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale) => Builtin::SGray,
        _ => return Err(scope()),
    };
    // Existing marker validation hashes the bound current bytes. Prepay before
    // entering it; cancellation remains the real caller token.
    for chunk in current.chunks(4096) {
        control.check(chunk.len() as u64).map_err(BackedNormalError::Backing)?;
    }
    let valid = doc.working_profile_interpretation.validate_with_interrupt(Some(current), doc.mode, control.interrupt);
    control.check(0).map_err(BackedNormalError::Backing)?;
    valid.map_err(|e| if e.code == "cancelled" { BackedNormalError::Backing(photocraft_raster::backing::BackingError::Cancelled) } else { scope() })?;
    // Builtin OnceLock initialization is an explicitly outer-polled provider
    // boundary. Only actual retained encoded bytes are compared; no date/name,
    // transfer-curve or absent-profile equivalence is substituted.
    control.check(1).map_err(BackedNormalError::Backing)?;
    let known = builtin.profile();
    control.check(1).map_err(BackedNormalError::Backing)?;
    if !paid_bytes_equal(current, known.encoded_bytes().ok_or_else(scope)?, control)? {
        return Err(scope());
    }
    Ok(known)
}
fn encoding(space: GimpSpace) -> Result<SampleEncoding, BackedNormalError> {
    match space {
        GimpSpace::LinearRgb => Ok(SampleEncoding::Linear),
        GimpSpace::ProfileRgb => Ok(SampleEncoding::NonlinearProfile),
        GimpSpace::PerceptualRgb => Ok(SampleEncoding::Perceptual),
        GimpSpace::Lab => Err(scope()),
    }
}
impl<'a> PreparedBackedThresholdAlphaContext<'a> {
    pub fn new(owner: &'a AccountedDocument, supplied: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        // Genuine runtime class + actual same-ledger storage/model census. A
        // persisted operation/schema/hash cannot supply this owner identity.
        owner.validate_readonly_threshold_alpha_storage(control).map_err(BackedNormalError::Backing)?;
        let doc = owner.document();
        let layer = doc.layers.first().ok_or_else(scope)?;
        let LayerContent::BackedRaster(backed) = &layer.content else {
            return Err(scope());
        };
        let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
        let stack = state.filter_stacks.first().ok_or_else(scope)?;
        let node = stack.nodes.first().ok_or_else(scope)?;
        let stored = doc.sample_encoding.ok_or_else(scope)?;
        if !owner.quota().shares_ledger(backed.quota())
            || layer.blend != photocraft_color::blend::BlendMode::Normal
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
            || !node.enabled
            || stack.owner != ItemRef::Layer(layer.id)
        {
            return Err(scope());
        }
        let header = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(scope)?;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(BackedNormalError::Quota)?;
        let known = profile(owner, supplied, control)?;
        // Closed class has <=2 items/one node/one typed scalar. This prepays
        // bounded metadata/string/registry comparisons, not a CPU-cycle claim.
        control.check(4096).map_err(BackedNormalError::Backing)?;
        crate::gimp_render::qualify_generic_point_storage_metadata(
            doc,
            layer,
            state,
            stack,
            backed.operative_bounds(),
            stored,
            backed.format(),
            backed.default_bytes(),
        )
        .map_err(BackedNormalError::Color)?;
        let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
        let resolved = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if resolved.operator != GimpOperator::Union
            || physical.pass_through
            || physical.blend_encoding != encoding(resolved.blend_space)?
            || physical.composite_encoding != encoding(resolved.composite_space)?
        {
            return Err(scope());
        }
        // This shared pure validator is also called by PointStack's enabled
        // Threshold Alpha branch. Alpha-only execution needs no color transfer.
        let operation = crate::gimp_filters::validated_enabled_threshold_alpha(&stack.nodes, known.channels(), stored, state.provenance.format_version)
            .map_err(BackedNormalError::Color)?;
        let mask = state.filter_masks.first().ok_or_else(scope)?;
        if node.mask != Some(ItemRef::Mask(mask.id)) || mask.owner != node.id {
            return Err(scope());
        }
        let physical = u64::try_from(mask.surface.tile_count())
            .ok()
            .and_then(|n| n.checked_mul(TILE_SIZE as u64))
            .and_then(|n| n.checked_mul(TILE_SIZE as u64))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(scope)?;
        let logical = u64::from(mask.bounds.width()).checked_mul(u64::from(mask.bounds.height())).ok_or_else(scope)?;
        let mut work = physical.max(logical).checked_mul(32).ok_or_else(scope)?;
        while work != 0 {
            let part = work.min(4096);
            control.check(part).map_err(BackedNormalError::Backing)?;
            work -= part;
        }
        let empty = mask.is_empty_selection_with(control.interrupt);
        control.check(0).map_err(BackedNormalError::Backing)?;
        if !empty.map_err(|e| {
            if e.code == "cancelled" {
                BackedNormalError::Backing(photocraft_raster::backing::BackingError::Cancelled)
            } else {
                BackedNormalError::Color(CompositionError::Unsupported("invalid current Threshold Alpha physical mask"))
            }
        })? {
            return Err(scope());
        }
        let descriptor = backed.descriptor();
        let native = PreparedBackedThresholdAlpha::new(backed, descriptor, operation, limits, control).map_err(|e| match e {
            BackedPointError::Backing(e) => BackedNormalError::Backing(e),
            _ => scope(),
        })?;
        let prepared = Self { native, _owner: owner, layer: layer.id, node: node.id, descriptor, _lease: lease };
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(prepared)
    }
    pub fn layer_id(&self) -> LayerId {
        self.layer
    }
    pub fn node_id(&self) -> FilterId {
        self.node
    }
    pub fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    /// Reads/validates only the requested contained operative band. It neither
    /// scans nor certifies raw backing padding/default extension beyond that ROI.
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNativePointBand, BackedPointError> {
        self.native.render_band(rect, control)
    }
}

#[cfg(test)]
#[path = "backed_threshold_alpha_context_tests.rs"]
mod tests;
