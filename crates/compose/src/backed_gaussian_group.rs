//! Closed CURRENT isolated small-Auto Gaussian group. Source admission is separate.
use crate::gimp_group_gaussian as gaussian;
use crate::{
    backed_normal::{BackedNormalError as E, NormalBandLimits},
    backed_normal_stack::OwnedNormalStackBand,
    gimp_composite::{CompositionError, WorkingSpace},
};
use photocraft_cms::{Builtin, WideWork};
use photocraft_doc::{
    Document, Layer, LayerContent, SampleEncoding, WorkingProfileInterpretation,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Surface,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
    checked_view::NativeTileBytes,
};
fn scope() -> E {
    E::Unsupported("closed isolated small-Auto Gaussian group")
}
fn size() -> E {
    E::Invalid("Gaussian group resource envelope")
}
fn color(e: CompositionError) -> E {
    match e {
        CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled) => E::Backing(BackingError::Cancelled),
        other => E::Color(other),
    }
}
struct Material {
    surface: Surface,
    _fee: QuotaLease,
}
struct Pixels {
    values: Vec<[f64; 4]>,
    fee: QuotaLease,
}
struct Prepared {
    filter: gaussian::GroupGaussian,
    linear: WorkingSpace,
    _fee: QuotaLease,
}
struct Shared<'a, B> {
    budget: &'a mut B,
}
impl<B: crate::gimp_render::PreparationBudget> crate::backed_stock_point_context::PlanBudget for Shared<'_, B> {
    type Lease = B::Lease;
    fn reserve(&mut self, n: u64) -> Result<Self::Lease, E> {
        self.budget.reserve_prepared_bytes(n).map_err(E::Color)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.budget.consume_preparation_work(n).map_err(E::Color)
    }
}
/// Stage/profile preparation for an independently admitted original source.
/// The genuine source/model lease and preparation fee outlive every heap here.
/// This performs no native materialization or output evaluation.
pub struct LeasedGaussianGroupPipeline<S, L> {
    _filter: gaussian::GroupGaussian,
    _working: WorkingSpace,
    _current_profile: photocraft_cms::Profile,
    _source: S,
    _lease: L,
}
impl<S, L> LeasedGaussianGroupPipeline<S, L> {
    pub fn new_budgeted<B: crate::gimp_render::PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        use crate::backed_stock_point_context::PlanBudget;
        let result = (|| -> Result<Self, E> {
            let mut b = Shared { budget };
            b.check(32768)?;
            let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
            let [_, layer] = doc.layers.as_slice() else { return Err(scope()) };
            let LayerContent::Group(group) = &layer.content else { return Err(scope()) };
            if group.children.len() != 2
                || state.items.len() != 6
                || state.filter_stacks.len() != 1
                || state.filter_masks.len() != 1
                || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
            {
                return Err(scope());
            }
            let lease = b.reserve(3 * 65536 + std::mem::size_of::<Self>() as u64 + 4096)?;
            let (current, profile) = crate::backed_stock_point_context::bound_profile(doc, None, &mut b, interrupt)?;
            let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(profile).map_err(|_| scope())?;
            b.check(semantic.checked_mul(6).ok_or_else(size)?)?;
            for mask in &state.filter_masks {
                b.check((mask.surface.tile_count() as u64 + 1).checked_mul(65536 * 4).ok_or_else(size)?)?;
            }
            let filter = gaussian::GroupGaussian::new_owned(
                doc,
                layer,
                state,
                state.filter_stacks.first().ok_or_else(scope)?,
                (bounds(doc, layer)?, profile, SampleEncoding::NonlinearProfile),
                interrupt,
            )
            .map_err(color)?;
            let working = WorkingSpace::new_with_interrupt(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, interrupt).map_err(color)?;
            b.check(0)?;
            Ok(Self { _filter: filter, _working: working, _current_profile: current, _source: source_lease, _lease: lease })
        })();
        result.map_err(|e| match e {
            E::Color(e) => e,
            E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
            _ => CompositionError::Unsupported("isolated Gaussian group preparation"),
        })
    }
}
/// Complete derived canvas, retaining the genuine native/model owner and real fee.
/// No native writeback or source correspondence is implied by these samples.
pub struct PreparedBackedGaussianGroupContext<'a> {
    output: Pixels,
    owner: &'a AccountedDocument,
    limits: NormalBandLimits,
}
fn unit(raw: &[u8], format: photocraft_doc::PixelFormat, c: &ReadControl<'_>) -> Result<(), E> {
    c.check(format.channels() as u64).map_err(E::Backing)?;
    for channel in 0..format.channels() {
        let v = photocraft_color::try_read_sample_f64(raw, format.sample, channel)
            .map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(scope());
        }
    }
    Ok(())
}
fn bounds(doc: &Document, layer: &Layer) -> Result<Rect, E> {
    doc.xcf_state.as_ref().and_then(|s| s.items.iter().find(|i| i.item == ItemRef::Layer(layer.id))).and_then(|i| i.raster_bounds).ok_or_else(scope)
}
fn material(layer: &Layer, q: &BackingQuota, c: &ReadControl<'_>) -> Result<Material, E> {
    let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
    if !backed.quota().shares_ledger(q) {
        return Err(scope());
    }
    let view = backed.checked_view();
    let format = view.format();
    unit(view.default_bytes(), format, c)?;
    let tiles = view.logical_bounds().tiles().count() as u64;
    let bytes = tiles.checked_mul(256 * 256 * format.bytes_per_pixel() as u64 * 3 + 1024).and_then(|v| v.checked_add(1024)).ok_or_else(size)?;
    let fee = q.reserve(QuotaKind::Resident, bytes).map_err(E::Quota)?;
    c.check(bytes).map_err(E::Backing)?;
    let mut out = Material { surface: Surface::try_with_default_bytes(format, view.default_bytes()).map_err(|e| color(e.into()))?, _fee: fee };
    for coord in view.logical_bounds().tiles() {
        let pin = view.pin_tile(coord, c).map_err(E::View)?;
        for y in coord.rect().intersect(&view.logical_bounds()).y0..coord.rect().intersect(&view.logical_bounds()).y1 {
            for x in coord.rect().intersect(&view.logical_bounds()).x0..coord.rect().intersect(&view.logical_bounds()).x1 {
                let mut raw = [0; 16];
                pin.read_pixel_native_into(x, y, &mut raw, c).map_err(E::View)?;
                unit(&raw, format, c)?;
            }
        }
        if let NativeTileBytes::FullTile(raw) = pin.encoded() {
            out.surface.try_write_interleaved_with(coord.rect(), raw, c.interrupt).map_err(|e| color(e.into()))?
        }
    }
    if out.surface.resident_payload_bytes().map_err(|e| color(e.into()))? > bytes {
        return Err(E::Allocation);
    }
    Ok(out)
}
fn validate_mask(surface: &Surface, c: &ReadControl<'_>) -> Result<(), E> {
    let f = surface.format();
    unit(surface.default_bytes(), f, c)?;
    for (_, tile) in surface.tiles() {
        for words in tile.bytes().chunks(1024 * f.bytes_per_pixel()) {
            c.check(words.len() as u64).map_err(E::Backing)?;
            for p in words.chunks_exact(f.bytes_per_pixel()) {
                unit(p, f, c)?
            }
        }
    }
    Ok(())
}
fn allocate(q: &BackingQuota, count: usize, c: &ReadControl<'_>) -> Result<Pixels, E> {
    let bytes = (count as u64).checked_mul(64).and_then(|n| n.checked_add(512)).ok_or_else(size)?;
    let fee = q.reserve(QuotaKind::Resident, bytes).map_err(E::Quota)?;
    c.check(bytes).map_err(E::Backing)?;
    let mut p = Pixels { values: Vec::new(), fee };
    p.values.try_reserve_exact(count).map_err(|_| E::Allocation)?;
    if p.values.capacity() > count.checked_mul(2).ok_or_else(size)? {
        return Err(E::Allocation);
    }
    p.fee.reconcile_capacity((p.values.capacity() as u64).checked_mul(32).and_then(|v| v.checked_add(512)).ok_or_else(size)?).map_err(E::Quota)?;
    Ok(p)
}
fn pixel(surface: &Surface, x: i32, y: i32) -> Result<[f64; 4], E> {
    let mut p = [0.; 4];
    surface.try_read_pixel_f64(x, y, &mut p).map_err(|e| color(e.into()))?;
    Ok(p)
}
fn policy<'a>(doc: &'a Document, layer: &Layer) -> Result<&'a photocraft_doc::import_compositing::GimpComposition, E> {
    doc.xcf_state.as_ref().and_then(|s| s.items.iter().find(|i| i.item == ItemRef::Layer(layer.id))).and_then(|i| i.composition.as_ref()).ok_or_else(scope)
}
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
fn compose(doc: &Document, layer: &Layer, p: &Prepared, bg: [f64; 4], source: [f64; 4], opacity: f64, seed: bool, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
    c.check(128).map_err(E::Backing)?;
    let policy = policy(doc, layer)?;
    let resolved = policy.checked_resolved().map_err(|_| scope())?;
    let working = match resolved.composite_space {
        GimpSpace::LinearRgb => &p.linear,
        GimpSpace::ProfileRgb => return Err(scope()),
        _ => return Err(scope()),
    };
    let blend = match resolved.blend_space {
        GimpSpace::LinearRgb => &p.linear,
        GimpSpace::ProfileRgb => return Err(scope()),
        _ => return Err(scope()),
    };
    let mode = crate::gimp_blend::mode(policy.source_mode).map_err(color)?;
    let operator = if seed && mode == crate::gimp_blend::GimpBlendMode::Multiply && resolved.operator == GimpOperator::ClipToBackdrop {
        GimpOperator::Union
    } else {
        resolved.operator
    };
    if mode == crate::gimp_blend::GimpBlendMode::Normal {
        working.normal(bg, source, opacity, operator).map_err(color)
    } else {
        working.separable(blend, bg, source, opacity, operator, mode).map_err(color)
    }
}
fn children(doc: &Document, layers: &[Layer], native: &[Material; 3], p: &Prepared, x: i32, y: i32, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
    let mut value = [0.; 4];
    let mut first = true;
    for (index, layer) in layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        let source = pixel(&native.get(index + 1).ok_or_else(scope)?.surface, x, y)?;
        value = compose(doc, layer, p, value, source, f64::from(layer.opacity) * f64::from(layer.fill_opacity), first, c)?;
        first = false;
    }
    Ok(value)
}
fn map_group(doc: &Document, layers: &[Layer], native: &[Material; 3], p: &Prepared, q: &BackingQuota, c: &ReadControl<'_>) -> Result<Material, E> {
    let extent = p.filter.extent;
    let count = u64::from(extent.width()) * u64::from(extent.height());
    let bytes = (extent.tiles().count() as u64).checked_mul(256 * 256 * 32 * 3 + 1024).and_then(|v| v.checked_add(count * 64 + 1024)).ok_or_else(size)?;
    let fee = q.reserve(QuotaKind::Resident, bytes).map_err(E::Quota)?;
    c.check(bytes).map_err(E::Backing)?;
    let mut out = Material { surface: Surface::new(photocraft_doc::PixelFormat::RGBA64F), _fee: fee };
    let mut samples = Vec::new();
    samples.try_reserve_exact(count as usize * 4).map_err(|_| E::Allocation)?;
    if samples.capacity() > count as usize * 8 {
        return Err(E::Allocation);
    }
    for y in extent.y0..extent.y1 {
        for x in extent.x0..extent.x1 {
            c.check(128).map_err(E::Backing)?;
            samples.extend_from_slice(&p.filter.linear(children(doc, layers, native, p, x, y, c)?).map_err(color)?);
        }
    }
    out.surface.try_write_region_f64_with(extent, &samples, c.interrupt).map_err(|e| color(e.into()))?;
    Ok(out)
}
impl<'a> PreparedBackedGaussianGroupContext<'a> {
    pub fn new(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        owner.validate_gaussian_group_storage(c).map_err(E::Backing)?;
        if limits.max_rows == 0 || limits.max_rows > 256 || limits.max_pixels == 0 || limits.max_pixels > 65536 {
            return Err(scope());
        }
        let doc = owner.document();
        let q = owner.quota();
        if !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. }) {
            return Err(scope());
        }
        c.check(1).map_err(E::Backing)?;
        let profile = Builtin::Srgb.profile();
        c.check(1).map_err(E::Backing)?;
        c.check(doc.icc_profile.as_ref().map_or(0, |v| v.len()) as u64 * 2).map_err(E::Backing)?;
        doc.working_profile_interpretation.validate_with(doc.icc_profile.as_deref().map(Vec::as_slice), doc.mode).map_err(|_| scope())?;
        if doc.icc_profile.as_deref().map(Vec::as_slice) != profile.encoded_bytes() {
            return Err(scope());
        }
        let base = doc.layers.first().ok_or_else(scope)?;
        let group_layer = doc.layers.get(1).ok_or_else(scope)?;
        let LayerContent::Group(group) = &group_layer.content else { return Err(scope()) };
        let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
        let stack = state.filter_stacks.first().ok_or_else(scope)?;
        let preparation = q.reserve(QuotaKind::Resident, 2 * 65536 + std::mem::size_of::<Prepared>() as u64 + 4096).map_err(E::Quota)?;
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(profile).map_err(|_| scope())?;
        c.check(semantic.checked_mul(6).ok_or_else(size)?).map_err(E::Backing)?;
        // Full physical/default mask scans precede any activation shortcut.
        let physical = group_layer.mask.as_ref().ok_or_else(scope)?;
        validate_mask(physical.surface.as_ref(), c)?;
        for mask in &state.filter_masks {
            validate_mask(&mask.surface, c)?
        }
        // Selection-mask qualification repeats a full physical/default scan.
        for mask in &state.filter_masks {
            c.check((mask.surface.tile_count() as u64 + 1).checked_mul(65536 * 4).ok_or_else(size)?).map_err(E::Backing)?;
        }
        let prepared = Prepared {
            filter: gaussian::GroupGaussian::new_current_owned(
                doc,
                group_layer,
                state,
                stack,
                (bounds(doc, group_layer)?, profile, SampleEncoding::NonlinearProfile),
                c.interrupt,
            )
            .map_err(color)?,
            linear: WorkingSpace::new_with_interrupt(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, c.interrupt).map_err(color)?,
            _fee: preparation,
        };
        let native =
            [material(base, q, c)?, material(group.children.first().ok_or_else(scope)?, q, c)?, material(group.children.get(1).ok_or_else(scope)?, q, c)?];
        let filtered = if prepared.filter.enabled {
            let input = map_group(doc, &group.children, &native, &prepared, q, c)?;
            use photocraft_algo::gimp::neighborhood as n;
            c.check((input.surface.tile_count() as u64 + 1) * 1024).map_err(E::Backing)?;
            let params = prepared.filter.parameters();
            let req =
                n::gaussian_finite_requirements(&input.surface, prepared.filter.extent, prepared.filter.extent, params, n::GaussianFilter::Auto, c.interrupt)
                    .map_err(|e| color(e.into()))?;
            if req.peak_resident_bytes > 64 * 1024 * 1024 {
                return Err(scope());
            }
            let envelope = req
                .peak_resident_bytes
                .checked_mul(2)
                .and_then(|v| v.checked_add((prepared.filter.extent.tiles().count() as u64) * (256 * 256 * 32 * 3 + 1024) + 16384))
                .ok_or_else(size)?;
            let fee = q.reserve(QuotaKind::Resident, envelope).map_err(E::Quota)?;
            c.check(req.work_units.checked_add(envelope).ok_or_else(size)?).map_err(E::Backing)?;
            let mut failure = None;
            let mut poll = |work| {
                let r = match work {
                    WideWork::SampleUnits(v) => c.check(v),
                    _ => Err(BackingError::Integrity),
                };
                if let Err(e) = r {
                    failure = Some(e);
                    return Err(CompositionError::Invalid("Gaussian group caller work"));
                }
                Ok(())
            };
            let result = prepared.filter.evaluate_owned(
                &input.surface,
                &mut poll,
                c.interrupt,
                n::GaussianLimits { resident_bytes: 64 * 1024 * 1024, work_units: req.work_units },
            );
            if let Some(e) = failure {
                return Err(E::Backing(e));
            }
            let result = Material { surface: result.map_err(color)?, _fee: fee };
            if result.surface.resident_payload_bytes().map_err(|e| color(e.into()))? > envelope {
                return Err(E::Allocation);
            }
            Some(result)
        } else {
            None
        };
        let mut output = allocate(q, 192, c)?;
        let mask_state = state
            .items
            .iter()
            .filter(|item| !state.filter_masks.iter().any(|mask| item.item == ItemRef::Mask(mask.id)))
            .filter_map(|item| item.mask.as_ref())
            .find(|mask| mask.owner == ItemRef::Layer(group_layer.id))
            .ok_or_else(scope)?;
        for y in 0..12 {
            for x in 0..16 {
                let mut value = [0.; 4];
                if base.visible {
                    value = compose(
                        doc,
                        base,
                        &prepared,
                        value,
                        pixel(&native[0].surface, x, y)?,
                        f64::from(base.opacity) * f64::from(base.fill_opacity),
                        false,
                        c,
                    )?;
                }
                if group_layer.visible && (!prepared.filter.enabled || prepared.filter.extent.contains(x, y)) {
                    let source = if let Some(filtered) = &filtered {
                        pixel(&filtered.surface, x, y)?
                    } else {
                        children(doc, &group.children, &native, &prepared, x, y, c)?
                    };
                    let coverage = if physical.enabled {
                        let mut m = [0.];
                        if mask_state.bounds.contains(x, y) {
                            physical.surface.try_read_pixel_f64(x, y, &mut m).map_err(|e| color(e.into()))?
                        } else {
                            m[0] = f64::from_bits(mask_state.outside_f64_bits)
                        }
                        1. - f64::from(physical.density) * (1. - m[0])
                    } else {
                        1.
                    };
                    value = compose(
                        doc,
                        group_layer,
                        &prepared,
                        value,
                        source,
                        f64::from(group_layer.opacity) * f64::from(group_layer.fill_opacity) * coverage,
                        false,
                        c,
                    )?;
                }
                if value.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&value[3]) {
                    return Err(scope());
                }
                output.values.push(value);
            }
        }
        c.check(0).map_err(E::Backing)?;
        Ok(Self { output, owner, limits })
    }
    pub fn bounds(&self) -> Rect {
        self.owner.document().bounds()
    }
    pub fn validate_operative(&self, c: &ReadControl<'_>) -> Result<(), E> {
        for p in &self.output.values {
            c.check(4).map_err(E::Backing)?;
            if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) {
                return Err(scope());
            }
        }
        Ok(())
    }
    pub fn render_band(&self, rect: Rect, c: &ReadControl<'_>) -> Result<OwnedNormalStackBand, E> {
        let count = u64::from(rect.width()) * u64::from(rect.height());
        if rect.is_empty() || !self.bounds().contains_rect(&rect) || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(scope());
        }
        let mut out = allocate(self.owner.quota(), count as usize, c)?;
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                c.check(32).map_err(E::Backing)?;
                let index = (y as usize) * 16 + x as usize;
                out.values.push(*self.output.values.get(index).ok_or_else(size)?);
            }
        }
        let Pixels { values, fee } = out;
        Ok(OwnedNormalStackBand::from_accounted_pixels(rect, values, fee))
    }
}

#[cfg(test)]
#[path = "backed_gaussian_group_tests.rs"]
mod tests;
