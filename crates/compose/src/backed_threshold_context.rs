//! Explicit CURRENT operative/default Histogram Threshold recipe over a real
//! read-only accounted storage owner. This is not original-source equivalence,
//! source eligibility, a full raw-padding proof or a default renderer enrollment.
//! Native samples are validated only in requested bands; raw off-plane backing
//! bytes stay inert and unchanged. The existing resident renderer keeps its full
//! Surface/default/tile/unit qualification separately.
use crate::{
    backed_normal::{BackedNormalError, NormalBandLimits},
    backed_threshold::{BackedThresholdError, OwnedNativeThresholdBand, PreparedBackedThreshold},
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
/// There is one enabled or disabled filter, an empty physical selection mask and Normal/Union
/// with CURRENT finite-unit layer coverage. Returned bands are straight native current owner samples,
/// not a display composite, Surface, persistence receipt or editable result.
/// This initial current recipe remains fixed at the class's 8-by-6 RGB extent.
pub struct PreparedBackedThresholdContext<'a> {
    native: PreparedBackedThreshold<'a>,
    _owner: &'a AccountedDocument,
    layer: LayerId,
    node: FilterId,
    descriptor: NativePlaneDescriptor,
    limits: NormalBandLimits,
    visible: bool,
    opacity: f64,
    _lease: QuotaLease,
}
fn scope() -> BackedNormalError {
    BackedNormalError::Unsupported("closed current backed Histogram Threshold operative/default recipe")
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
    if doc.mode != ColorMode::Rgb || !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. }) {
        return Err(scope());
    }
    let builtin = Builtin::Srgb;
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
impl<'a> PreparedBackedThresholdContext<'a> {
    pub fn new(owner: &'a AccountedDocument, supplied: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        // Genuine runtime class + actual same-ledger storage/model census. A
        // persisted operation/schema/hash cannot supply this owner identity.
        owner.validate_readonly_threshold_storage(control).map_err(BackedNormalError::Backing)?;
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
            || stack.owner != ItemRef::Layer(layer.id)
        {
            return Err(scope());
        }
        let header = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(scope)?;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(BackedNormalError::Quota)?;
        let _known = profile(owner, supplied, control)?;
        // Closed class has <=2 items/one node/three typed arguments. This prepays
        // bounded metadata/string/registry comparisons, not a CPU-cycle claim.
        control.check(4096).map_err(BackedNormalError::Backing)?;
        crate::gimp_render::qualify_current_point_layer_coverage_metadata(
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
        // Exact current typed recipe; this does not enroll original source storage.
        let (operation, enabled) =
            crate::gimp_threshold_current::validate(&stack.nodes, doc.mode, stored, state.provenance.format_version).map_err(BackedNormalError::Color)?;
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
                BackedNormalError::Color(CompositionError::Unsupported("invalid current Histogram Threshold physical mask"))
            }
        })? {
            return Err(scope());
        }
        let descriptor = backed.descriptor();
        let native = PreparedBackedThreshold::new_with_enabled(backed, descriptor, stored, operation, enabled, limits, control).map_err(|e| match e {
            BackedThresholdError::Backing(e) => BackedNormalError::Backing(e),
            _ => scope(),
        })?;
        let prepared = Self {
            native,
            _owner: owner,
            layer: layer.id,
            node: node.id,
            descriptor,
            limits,
            visible: layer.visible,
            opacity: f64::from(layer.opacity),
            _lease: lease,
        };
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(prepared)
    }
    /// Display-only layer state; native filter bands remain unscaled.
    pub fn layer_visible(&self) -> bool {
        self.visible
    }
    pub fn layer_opacity(&self) -> f64 {
        self.opacity
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
    /// Validate the complete operative extent using one same-ledger native band
    /// at a time. The exact caller-lowered limits are retained from preparation;
    /// there is no full-image output, new work budget or retained rendering grant.
    /// Only operative samples are inspected: backing padding remains uncertified.
    pub fn validate_operative(&self, control: &ReadControl<'_>) -> Result<(), BackedThresholdError> {
        visit_operative_bands(self.descriptor.bounds(), self.limits, control, |rect| {
            let band = self.render_band(rect, control)?;
            drop(band);
            Ok(())
        })
    }
    /// Reads/validates only the requested contained operative band. It neither
    /// scans nor certifies raw backing padding/default extension beyond that ROI.
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNativeThresholdBand, BackedThresholdError> {
        self.native.render_band(rect, control)
    }
}

// Allocation-free scheduling only. This private helper supplies neither storage
// admission nor source authority; the context's immutable owner supplies both
// the operative bounds and already-validated lower limits.
fn visit_operative_bands(
    rect: Rect,
    limits: NormalBandLimits,
    control: &ReadControl<'_>,
    mut visit: impl FnMut(Rect) -> Result<(), BackedThresholdError>,
) -> Result<(), BackedThresholdError> {
    control.check(1).map_err(BackedThresholdError::Backing)?;
    let maximum = NormalBandLimits::default();
    if rect.is_empty()
        || rect.x1 < rect.x0
        || rect.y1 < rect.y0
        || limits.max_rows == 0
        || limits.max_rows > maximum.max_rows
        || limits.max_pixels == 0
        || limits.max_pixels > maximum.max_pixels
    {
        return Err(BackedThresholdError::Invalid("operative validation bounds or limits"));
    }
    let width = u64::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| BackedThresholdError::Invalid("operative validation width"))?;
    let columns = width.min(limits.max_pixels).min(TILE_SIZE as u64);
    let rows = limits.max_pixels.checked_div(columns).ok_or(BackedThresholdError::Invalid("operative validation columns"))?.min(u64::from(limits.max_rows));
    let columns = i64::try_from(columns).map_err(|_| BackedThresholdError::Invalid("operative validation columns"))?;
    let rows = i64::try_from(rows).map_err(|_| BackedThresholdError::Invalid("operative validation rows"))?;
    if columns == 0 || rows == 0 {
        return Err(BackedThresholdError::Invalid("operative validation progress"));
    }
    let mut y = rect.y0;
    while y < rect.y1 {
        let tile_rows = i64::from(TILE_SIZE - y.rem_euclid(TILE_SIZE));
        let end_y =
            i64::from(y).checked_add(rows.min(tile_rows)).ok_or(BackedThresholdError::Invalid("operative validation row overflow"))?.min(i64::from(rect.y1));
        let end_y = i32::try_from(end_y).map_err(|_| BackedThresholdError::Invalid("operative validation row range"))?;
        let mut x = rect.x0;
        while x < rect.x1 {
            control.check(1).map_err(BackedThresholdError::Backing)?;
            let tile_columns = i64::from(TILE_SIZE - x.rem_euclid(TILE_SIZE));
            let end_x = i64::from(x)
                .checked_add(columns.min(tile_columns))
                .ok_or(BackedThresholdError::Invalid("operative validation column overflow"))?
                .min(i64::from(rect.x1));
            let end_x = i32::try_from(end_x).map_err(|_| BackedThresholdError::Invalid("operative validation column range"))?;
            if end_x <= x || end_y <= y {
                return Err(BackedThresholdError::Invalid("operative validation progress"));
            }
            visit(Rect::new(x, y, end_x, end_y))?;
            // The validation caller has destroyed the band before this poll.
            control.check(1).map_err(BackedThresholdError::Backing)?;
            x = end_x;
        }
        y = end_y;
    }
    control.check(1).map_err(BackedThresholdError::Backing)?;
    Ok(())
}

#[cfg(test)]
#[path = "backed_threshold_context_tests.rs"]
pub(crate) mod tests;
