//! Bounded paid CURRENT descriptors, independent of the original source selector.
use super::*;
use photocraft_doc::import_compositing::GimpComposition;

pub(super) struct Metadata<'a> {
    pub(super) inputs: Vec<SeparableStackLayer<'a>>,
    pub(super) recipes: Vec<Recipe>,
    _fee: QuotaLease,
}
fn explicit_space(raw: i32) -> Option<GimpSpace> {
    match raw {
        1 => Some(GimpSpace::LinearRgb),
        2 => Some(GimpSpace::ProfileRgb),
        4 => Some(GimpSpace::PerceptualRgb),
        _ => None,
    }
}
fn explicit_operator(raw: i32) -> Option<GimpOperator> {
    match raw {
        1 => Some(GimpOperator::Union),
        2 => Some(GimpOperator::ClipToBackdrop),
        3 => Some(GimpOperator::ClipToLayer),
        4 => Some(GimpOperator::Intersection),
        _ => None,
    }
}
fn policy(policy: &GimpComposition, version: u32, c: &ReadControl<'_>) -> Result<Recipe, E> {
    if policy.compatibility_epoch.is_empty()
        || policy.compatibility_epoch.len() > 128
        || policy.pass_through
        || !(policy.source_mode == 28
            || lower::is_supported_mode(policy.source_mode)
            || lower::is_supported_legacy_mode(policy.source_mode)
            || lower::is_supported_component_mode(policy.source_mode)
            || lower::is_supported_alpha_mode(policy.source_mode))
    {
        return Err(scope());
    }
    c.check(policy.compatibility_epoch.len() as u64 + 1).map_err(E::Backing)?;
    if policy.legacy {
        use photocraft_doc::import_compositing::{XCF12_FLAT_LEGACY_SOURCE_EPOCH, flat_legacy_source_rule};
        if policy.compatibility_epoch != XCF12_FLAT_LEGACY_SOURCE_EPOCH {
            return Err(scope());
        }
        let rule = flat_legacy_source_rule(version, policy.source_mode, policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator)
            .ok_or_else(scope)?;
        if policy.checked_resolved().map_err(|_| scope())? != &rule.resolved() {
            return Err(scope());
        }
        return Ok(Recipe { mode: policy.source_mode, blend: rule.blend_space, composite: rule.composite_space, operator: rule.operator });
    }
    if lower::is_supported_legacy_mode(policy.source_mode) {
        return Err(scope());
    }
    if lower::is_supported_component_mode(policy.source_mode) {
        use photocraft_doc::import_compositing::{XCF12_FLAT_COMPONENT_SOURCE_EPOCH, flat_component_source_rule};
        if policy.compatibility_epoch != XCF12_FLAT_COMPONENT_SOURCE_EPOCH {
            return Err(scope());
        }
        let rule = flat_component_source_rule(version, policy.source_mode, policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator)
            .ok_or_else(scope)?;
        if policy.checked_resolved().map_err(|_| scope())? != &rule.resolved() {
            return Err(scope());
        }
        return Ok(Recipe { mode: policy.source_mode, blend: rule.blend_space, composite: rule.composite_space, operator: rule.operator });
    }
    if lower::is_supported_alpha_mode(policy.source_mode) {
        use photocraft_doc::import_compositing::{XCF12_FLAT_ALPHA_SOURCE_EPOCH, flat_alpha_source_rule};
        if policy.compatibility_epoch != XCF12_FLAT_ALPHA_SOURCE_EPOCH {
            return Err(scope());
        }
        let rule = flat_alpha_source_rule(version, policy.source_mode, policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator)
            .ok_or_else(scope)?;
        if policy.checked_resolved().map_err(|_| scope())? != &rule.resolved() {
            return Err(scope());
        }
        return Ok(Recipe { mode: policy.source_mode, blend: rule.blend_space, composite: rule.composite_space, operator: rule.operator });
    }
    // These existing epochs select separate assigned-profile or signed-alpha
    // recipes in the resident renderer. Fully explicit fields do not erase
    // those semantic prerequisites or grant either specialized lane here.
    if photocraft_doc::import_compositing::analytic_normal_encoding(&policy.compatibility_epoch).is_some()
        || policy.compatibility_epoch == photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH
    {
        return Err(scope());
    }
    let resolved = policy.checked_resolved().map_err(|_| scope())?;
    if policy.compatibility_epoch == photocraft_doc::import_compositing::XCF_FLAT_MODERN_IMPLICIT_SOURCE_EPOCH {
        let rule = photocraft_doc::import_compositing::flat_modern_source_rule(
            version,
            policy.source_mode,
            policy.stored_blend_space,
            policy.stored_composite_space,
            policy.stored_operator,
        )
        .ok_or_else(scope)?;
        if resolved != &rule.resolved() {
            return Err(scope());
        }
        return Ok(Recipe { mode: policy.source_mode, blend: rule.blend_space, composite: rule.composite_space, operator: rule.operator });
    }
    if let (Some(blend), Some(composite), Some(operator)) =
        (explicit_space(policy.stored_blend_space), explicit_space(policy.stored_composite_space), explicit_operator(policy.stored_operator))
    {
        // The existing explicit CURRENT wire has no ordinary Layer.blend
        // equivalent for grain. Retained authored Auto grain remains below.
        if matches!(policy.source_mode, 46 | 47) {
            return Err(scope());
        }
        // Existing xcf.composition.set writes all three fields explicitly and
        // preserves the prior epoch even when changing mode. The retained epoch
        // is bounded inert metadata here, never negative-Auto/source authority.
        if resolved.blend_space != blend || resolved.composite_space != composite || resolved.operator != operator {
            return Err(scope());
        }
        return Ok(Recipe { mode: policy.source_mode, blend, composite, operator });
    }
    // Negative/Auto fields get only the exact already qualified version/mode/
    // epoch/raw resolution. checked_resolved alone is not an Auto interpreter.
    let (blend, composite, operator, epoch) = if policy.source_mode == 28 {
        if (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1) {
            return Err(scope());
        }
        (
            GimpSpace::LinearRgb,
            GimpSpace::LinearRgb,
            GimpOperator::Union,
            if version == 12 {
                "xcf-12-normal"
            } else if version == 23 {
                "xcf-23-normal"
            } else {
                return Err(scope());
            },
        )
    } else {
        if policy.stored_operator != -2 {
            return Err(scope());
        }
        let (blend, composite) = match (version, policy.source_mode, policy.stored_blend_space, policy.stored_composite_space) {
            (12, 30, -1, -1) => (GimpSpace::LinearRgb, GimpSpace::LinearRgb),
            (12, 30, 2, -1) => (GimpSpace::ProfileRgb, GimpSpace::LinearRgb),
            (23, 30, 4, -1) => (GimpSpace::PerceptualRgb, GimpSpace::LinearRgb),
            (12, 30, -1, 2) => (GimpSpace::LinearRgb, GimpSpace::ProfileRgb),
            (23, 30, -1, 4) => (GimpSpace::LinearRgb, GimpSpace::PerceptualRgb),
            (23, 23 | 31 | 32 | 42..=53, -4, -1) => (GimpSpace::PerceptualRgb, GimpSpace::LinearRgb),
            (12, 33 | 34 | 41, -1, -1) => (GimpSpace::LinearRgb, GimpSpace::LinearRgb),
            (12, 35 | 36, 0, -1) => (GimpSpace::ProfileRgb, GimpSpace::LinearRgb),
            _ => return Err(scope()),
        };
        let epoch = match (version, policy.source_mode) {
            (12, 35 | 36) => "xcf-12-authored-equivalent-rgb-space",
            (12, _) => "xcf-12-authored-blend-auto",
            (23, _) => "xcf-23-authored-blend-auto",
            _ => return Err(scope()),
        };
        (blend, composite, GimpOperator::ClipToBackdrop, epoch)
    };
    if policy.compatibility_epoch != epoch || resolved.blend_space != blend || resolved.composite_space != composite || resolved.operator != operator {
        return Err(scope());
    }
    Ok(Recipe { mode: policy.source_mode, blend, composite, operator })
}
impl<'a> Metadata<'a> {
    pub(super) fn new(owner: &'a AccountedDocument, c: &ReadControl<'_>) -> Result<Self, E> {
        let doc = owner.document();
        let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
        let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
        let n = doc.layers.len();
        if n == 0
            || n > 4096
            || state.items.len() != n
            || doc.bounds().is_empty()
            || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || doc.depth != SampleType::F32
            || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
            || source.source_precision != 650
            || source.original_icc.is_some()
            || !matches!(state.provenance.format_version, 12 | 23)
            || !doc.channels.is_empty()
            || !doc.paths.is_empty()
            || !state.filter_stacks.is_empty()
            || !state.filter_masks.is_empty()
            || !state.source_items.is_empty()
            || !state.editable_indexed.is_empty()
            || state.floating.is_some()
        {
            return Err(scope());
        }
        c.check(1).map_err(E::Backing)?;
        let profile = implicit_basis(doc)?;
        c.check(1).map_err(E::Backing)?;
        let current = doc.icc_profile.as_deref().map(Vec::as_slice).ok_or_else(scope)?;
        if !equal(current, profile.encoded_bytes().ok_or_else(scope)?, &mut |n| c.check(n).map_err(E::Backing))? {
            return Err(scope());
        }
        c.check(current.len() as u64 + 64).map_err(E::Backing)?;
        doc.working_profile_interpretation
            .validate_with_interrupt(Some(current), doc.mode, c.interrupt)
            .map_err(|_| if c.interrupt.cancelled() { E::Backing(BackingError::Cancelled) } else { scope() })?;
        for opaque in &state.opaque {
            c.check(1).map_err(E::Backing)?;
            if opaque.rendering_required {
                return Err(scope());
            }
        }
        let each = std::mem::size_of::<Recipe>().checked_add(std::mem::size_of::<SeparableStackLayer<'a>>()).ok_or_else(scope)?;
        let bytes = n.checked_mul(each).and_then(|v| v.checked_mul(2)).and_then(|v| v.checked_add(std::mem::size_of::<Self>())).ok_or_else(scope)?;
        let fee = owner.quota().reserve(QuotaKind::Index, bytes as u64).map_err(E::Quota)?;
        let mut out = Self { inputs: Vec::new(), recipes: Vec::new(), _fee: fee };
        c.check(bytes as u64).map_err(E::Backing)?;
        let ceiling = n.checked_mul(2).ok_or_else(scope)?;
        out.inputs.try_reserve_exact(n).map_err(|_| E::Allocation)?;
        if out.inputs.capacity() > ceiling {
            return Err(E::Allocation);
        }
        out.recipes.try_reserve_exact(n).map_err(|_| E::Allocation)?;
        if out.recipes.capacity() > ceiling {
            return Err(E::Allocation);
        }
        let actual = out
            .inputs
            .capacity()
            .checked_mul(std::mem::size_of::<SeparableStackLayer<'a>>())
            .and_then(|v| out.recipes.capacity().checked_mul(std::mem::size_of::<Recipe>()).and_then(|r| v.checked_add(r)))
            .and_then(|v| v.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(scope)?;
        out._fee.reconcile_capacity(actual as u64).map_err(E::Quota)?;
        let mut has_alpha = false;
        for layer in &doc.layers {
            c.check(1).map_err(E::Backing)?;
            let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
            let mut item = None;
            // Canonical rows may be reordered. This bounded O(n²) walk has an
            // actual debit at every visited row; no unpriced iterator.find.
            for row in &state.items {
                c.check(1).map_err(E::Backing)?;
                if row.item == ItemRef::Layer(layer.id) {
                    if item.is_some() {
                        return Err(scope());
                    }
                    item = Some(row);
                }
            }
            let item = item.ok_or_else(scope)?;
            if item.parent.is_some()
                || item.visible != layer.visible
                || item.raster_bounds != Some(backed.operative_bounds())
                || !owner.quota().shares_ledger(backed.quota())
                || backed.format() != doc.pixel_format()
                || !backed.format().alpha
                || layer.mask.is_some()
                || layer.vector_mask.is_some()
                || layer.clipped
                || !layer.effects.items.is_empty()
                || layer.blend_if != photocraft_doc::BlendIf::default()
                || layer.excluded_channels != 0
                || layer.advanced != photocraft_doc::AdvancedBlending::default()
                || layer.fill_opacity.to_bits() != 1f32.to_bits()
                || !layer.opacity.is_finite()
                || !(0.0..=1.0).contains(&layer.opacity)
            {
                return Err(scope());
            }
            let recipe = policy(item.composition.as_ref().ok_or_else(scope)?, state.provenance.format_version, c)?;
            if doc.mode == ColorMode::Grayscale && recipe.mode != 28 && !lower::is_supported_alpha_mode(recipe.mode) {
                return Err(scope());
            }
            has_alpha |= lower::is_supported_alpha_mode(recipe.mode);
            let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
            if physical.pass_through || physical.blend_encoding != transfer(recipe.blend)? || physical.composite_encoding != transfer(recipe.composite)? {
                return Err(scope());
            }
            // Physical Layer.blend stays inert, exactly as in the resident path.
            out.inputs.push(SeparableStackLayer {
                layer: backed,
                visible: layer.visible,
                opacity: f64::from(layer.opacity),
                source_mode: recipe.mode,
                blend_space: recipe.blend,
                composite_space: recipe.composite,
                operator: recipe.operator,
            });
            out.recipes.push(recipe);
        }
        if doc.mode == ColorMode::Grayscale && !has_alpha {
            return Err(scope());
        }
        c.check(0).map_err(E::Backing)?;
        Ok(out)
    }
}
pub(super) fn validate_bands(stack: &PreparedBackedSeparableStack<'_>, bounds: Rect, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<(), E> {
    if bounds.is_empty() {
        return Err(scope());
    }
    let columns = u64::from(bounds.width()).min(limits.max_pixels).min(256);
    if columns == 0 {
        return Err(scope());
    }
    let rows = (limits.max_pixels / columns).min(u64::from(limits.max_rows));
    let columns = i32::try_from(columns).map_err(|_| scope())?;
    let rows = i32::try_from(rows).map_err(|_| scope())?;
    if rows == 0 {
        return Err(scope());
    }
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let end_y = y.saturating_add(rows.min(256 - y.rem_euclid(256))).min(bounds.y1);
        let mut x = bounds.x0;
        while x < bounds.x1 {
            let end_x = x.saturating_add(columns.min(256 - x.rem_euclid(256))).min(bounds.x1);
            c.check(1).map_err(E::Backing)?;
            drop(stack.render_band(Rect::new(x, y, end_x, end_y), [0.; 4], c)?);
            x = end_x;
        }
        y = end_y;
    }
    c.check(0).map_err(E::Backing)
}
