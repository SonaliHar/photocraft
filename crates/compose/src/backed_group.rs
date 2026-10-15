//! Paid isolated Normal groups. Qualification is independent from ownership.
use super::{BackedNormalError as Error, NormalBandLimits};
use crate::{
    backed_normal::{PreparedBackedNormal, ResidentNormalMask},
    backed_normal_stack::OwnedNormalStackBand,
    gimp_composite::WorkingSpace,
};
use photocraft_cms::Profile;
use photocraft_doc::{
    Document, Layer, LayerContent, LayerId, SampleEncoding, SampleType,
    accounted_document::{AccountedDocument, accounted_layer_access},
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::Rect;
use photocraft_raster::{
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
    checked_view::CheckedRasterView,
};

const EPOCH: &str = "own-accounted-isolated-normal-group-v1";
struct Leaf<'a> {
    id: LayerId,
    prepared: PreparedBackedNormal<'a>,
}
struct ScratchBand {
    pixels: Vec<[f64; 4]>,
    lease: QuotaLease,
}
pub(super) struct PreparedGroup<'a> {
    doc: &'a Document,
    leaves: Vec<Leaf<'a>>,
    working: WorkingSpace,
    quota: BackingQuota,
    limits: NormalBandLimits,
    _working: QuotaLease,
    _index: QuotaLease,
}
impl<'a> PreparedGroup<'a> {
    pub(super) fn new(owner: &'a AccountedDocument, profile: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, Error> {
        Self::prepare(owner, profile, limits, control, false)
    }
    pub(super) fn new_assigned(
        owner: &'a AccountedDocument,
        binding: &crate::backed_assigned_current_profile::PreparedAssignedCurrentProfile,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, Error> {
        control.check(1).map_err(Error::Backing)?;
        if !binding.matches(owner, SampleEncoding::NonlinearProfile) {
            return Err(Error::Invalid("isolated group profile binding mismatch"));
        }
        owner.validate_assigned_isolated_normal_group_storage(control).map_err(Error::Backing)?;
        Self::prepare(owner, binding.profile(), limits, control, true)
    }
    fn prepare(owner: &'a AccountedDocument, profile: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>, assigned: bool) -> Result<Self, Error> {
        let doc = owner.document();
        qualify(owner, control)?;
        if limits.max_rows == 0 || limits.max_rows > 256 || limits.max_pixels == 0 || limits.max_pixels > 65536 {
            return Err(Error::Invalid("group band limits"));
        }
        if assigned && (doc.size.width != 17 || doc.size.height != 13) {
            return Err(Error::Unsupported("observed isolated group canvas"));
        }
        control.check(doc.icc_profile.as_ref().map_or(0, |v| v.len()) as u64 * 2).map_err(Error::Backing)?;
        if doc.icc_profile.as_deref().map(Vec::as_slice) != profile.encoded_bytes() || profile.channels() != doc.mode.color_channels() {
            return Err(Error::Invalid("group current ICC mismatch"));
        }
        let quota = owner.quota();
        let mut index =
            quota.reserve(QuotaKind::Index, (6 * std::mem::size_of::<Leaf<'a>>() + std::mem::size_of::<Self>() + 64) as u64).map_err(Error::Quota)?;
        let mut leaves = Vec::new();
        leaves.try_reserve_exact(3).map_err(|_| Error::Allocation)?;
        let actual = leaves
            .capacity()
            .checked_mul(std::mem::size_of::<Leaf<'a>>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .ok_or(Error::Invalid("group index capacity"))?;
        let ceiling = 6usize
            .checked_mul(std::mem::size_of::<Leaf<'a>>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 64))
            .ok_or(Error::Invalid("group index capacity"))?;
        if actual > ceiling {
            return Err(Error::Invalid("group index capacity"));
        }
        index.reconcile_capacity(actual as u64).map_err(Error::Quota)?;
        for layer in &doc.layers {
            match &layer.content {
                LayerContent::BackedRaster(backed) => leaves.push(Leaf {
                    id: layer.id,
                    prepared: PreparedBackedNormal::new_with_composite_space(
                        backed,
                        profile,
                        SampleEncoding::NonlinearProfile,
                        GimpSpace::LinearRgb,
                        backed.descriptor(),
                        limits,
                        control,
                    )?,
                }),
                LayerContent::Group(group) => {
                    for child in &group.children {
                        let LayerContent::BackedRaster(backed) = &child.content else {
                            return Err(Error::Unsupported("nested group context not qualified"));
                        };
                        leaves.push(Leaf {
                            id: child.id,
                            prepared: PreparedBackedNormal::new_with_composite_space(
                                backed,
                                profile,
                                SampleEncoding::NonlinearProfile,
                                GimpSpace::LinearRgb,
                                backed.descriptor(),
                                limits,
                                control,
                            )?,
                        });
                    }
                }
                _ => return Err(Error::Unsupported("group content")),
            }
        }
        // Leaf preparation has already qualified bounded analytic curves and
        // strings. Prepay the same cross-space profile clones before construction.
        let strings = profile.description.len().checked_add(profile.copyright.len()).ok_or(Error::Invalid("group profile size"))? as u64;
        let temporary = quota
            .reserve(QuotaKind::Resident, strings.checked_mul(6).and_then(|n| n.checked_add(16384)).ok_or(Error::Invalid("group profile size"))?)
            .map_err(Error::Quota)?;
        let working_lease = quota
            .reserve(
                QuotaKind::Resident,
                strings
                    .checked_add(std::mem::size_of::<Profile>() as u64)
                    .and_then(|n| n.checked_add(std::mem::size_of::<WorkingSpace>() as u64 + 4096))
                    .ok_or(Error::Invalid("group profile size"))?,
            )
            .map_err(Error::Quota)?;
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(profile).map_err(cms_error)?;
        control
            .check(semantic.checked_mul(2).and_then(|n| n.checked_add(strings + 1)).ok_or(Error::Backing(photocraft_raster::backing::BackingError::Length))?)
            .map_err(Error::Backing)?;
        let working =
            WorkingSpace::new_with_interrupt(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, control.interrupt).map_err(|e| match e {
                crate::gimp_composite::CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled) => {
                    Error::Backing(photocraft_raster::backing::BackingError::Cancelled)
                }
                other => Error::Color(other),
            })?;
        drop(temporary);
        let mut result = Self { doc, leaves, working, quota: quota.clone(), limits, _working: working_lease, _index: index };
        result
            ._index
            .reconcile_capacity((result.leaves.capacity() * std::mem::size_of::<Leaf<'a>>() + std::mem::size_of::<Self>()) as u64)
            .map_err(Error::Quota)?;
        result.validate_operative(control)?;
        control.check(1).map_err(Error::Backing)?;
        Ok(result)
    }
    fn validate_operative(&self, control: &ReadControl<'_>) -> Result<(), Error> {
        let _scratch = self
            .quota
            .reserve(QuotaKind::Resident, (std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() + 40 + 128) as u64)
            .map_err(Error::Quota)?;
        // Qualification fixed this complete tree to three leaves and one group.
        // No visibility, opacity, or mask-enable short circuit is allowed here.
        for layer in &self.doc.layers {
            match &layer.content {
                LayerContent::BackedRaster(v) => scan(v.checked_view(), control)?,
                LayerContent::Group(g) => {
                    for child in &g.children {
                        let LayerContent::BackedRaster(v) = &child.content else {
                            return Err(Error::Unsupported("group child changed"));
                        };
                        scan(v.checked_view(), control)?;
                    }
                    if let Some(mask) = mask(self.doc, layer, control)? {
                        scan(mask.view, control)?;
                    }
                }
                _ => return Err(Error::Unsupported("group content changed")),
            }
        }
        control.check(0).map_err(Error::Backing)
    }
    pub(super) fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNormalStackBand, Error> {
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or(Error::Invalid("group band"))?;
        if rect.is_empty() || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(Error::Invalid("group band"));
        }
        self.render_layers(&self.doc.layers, rect, 0, control)
    }
    fn render_layers(&self, layers: &[Layer], rect: Rect, depth: usize, control: &ReadControl<'_>) -> Result<OwnedNormalStackBand, Error> {
        if depth > 1 {
            return Err(Error::Unsupported("nested group context not qualified"));
        }
        let mut output = self.blank(rect, control)?;
        for layer in layers {
            control.check(1).map_err(Error::Backing)?;
            if !layer.visible || layer.opacity == 0. {
                continue;
            }
            match &layer.content {
                LayerContent::BackedRaster(_) => {
                    let leaf = self.leaves.iter().find(|entry| entry.id == layer.id).ok_or(Error::Invalid("prepared group leaf"))?;
                    let next = leaf.prepared.render_band_over_with_resident_coverage(rect, &output.pixels, f64::from(layer.opacity), None, control)?;
                    // Reuse our already charged parent allocation. Atomicity is
                    // at the complete returned band, never a partial group.
                    if output.pixels.len() != next.pixels().len() {
                        return Err(Error::Invalid("group leaf band length"));
                    }
                    control.check((output.pixels.len() as u64).checked_mul(32).ok_or(Error::Invalid("group band copy work"))?).map_err(Error::Backing)?;
                    for (targets, sources) in output.pixels.chunks_mut(64).zip(next.pixels().chunks(64)) {
                        control.check(0).map_err(Error::Backing)?;
                        targets.copy_from_slice(sources);
                    }
                }
                LayerContent::Group(group) => {
                    let source = self.render_layers(&group.children, rect, depth + 1, control)?;
                    let mask = mask(self.doc, layer, control)?;
                    let _scratch = if mask.is_some_and(|m| m.enabled) {
                        Some(
                            self.quota
                                .reserve(QuotaKind::Resident, std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() as u64 + 128)
                                .map_err(Error::Quota)?,
                        )
                    } else {
                        None
                    };
                    let mut pin: Option<photocraft_raster::checked_view::CheckedTilePin> = None;
                    let extent = item_bounds(self.doc, layer)?;
                    for (index, pixel) in output.pixels.iter_mut().enumerate() {
                        control.check(128).map_err(Error::Backing)?;
                        let x = i64::from(rect.x0) + (index as u64 % u64::from(rect.width())) as i64;
                        let y = i64::from(rect.y0) + (index as u64 / u64::from(rect.width())) as i64;
                        let x = i32::try_from(x).map_err(|_| Error::Invalid("group sample coordinate"))?;
                        let y = i32::try_from(y).map_err(|_| Error::Invalid("group sample coordinate"))?;
                        if !extent.contains(x, y) {
                            continue;
                        }
                        let coverage = if let Some(mask) = mask.filter(|m| m.enabled) {
                            if mask.view.logical_bounds().contains(x, y) {
                                let coordinate = photocraft_geom::TileCoord::containing(x, y);
                                if pin.as_ref().is_none_or(|old| old.coord() != coordinate) {
                                    drop(pin.take());
                                    pin = Some(mask.view.pin_tile(coordinate, control).map_err(Error::View)?);
                                }
                                let format = mask.view.format();
                                let mut raw = [0; 8];
                                let raw = raw.get_mut(..format.bytes_per_pixel()).ok_or(Error::Invalid("group mask sample layout"))?;
                                pin.as_ref().ok_or(Error::Invalid("group mask pin"))?.read_pixel_native_into(x, y, raw, control).map_err(Error::View)?;
                                let value = photocraft_color::try_read_sample_f64(raw, format.sample, 0)
                                    .map_err(|e| Error::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
                                crate::gimp_composite::coverage(value).map_err(Error::Color)?;
                                value
                            } else {
                                mask.outside_coverage
                            }
                        } else {
                            1.
                        };
                        *pixel = self
                            .working
                            .normal(
                                *pixel,
                                *source.pixels().get(index).ok_or(Error::Invalid("group source index"))?,
                                f64::from(layer.opacity) * coverage,
                                GimpOperator::Union,
                            )
                            .map_err(Error::Color)?;
                    }
                }
                _ => return Err(Error::Unsupported("group content changed")),
            }
        }
        control.check(1).map_err(Error::Backing)?;
        Ok(OwnedNormalStackBand::from_accounted_pixels(rect, output.pixels, output.lease))
    }
    fn blank(&self, rect: Rect, control: &ReadControl<'_>) -> Result<ScratchBand, Error> {
        let count = usize::try_from(u64::from(rect.width()) * u64::from(rect.height())).map_err(|_| Error::Invalid("group band allocation"))?;
        let header = std::mem::size_of::<ScratchBand>().max(std::mem::size_of::<OwnedNormalStackBand>()) as u64;
        let lease = self
            .quota
            .reserve(QuotaKind::Resident, (count as u64).checked_mul(64).and_then(|n| n.checked_add(header)).ok_or(Error::Invalid("group band allocation"))?)
            .map_err(Error::Quota)?;
        let mut output = ScratchBand { pixels: Vec::new(), lease };
        output.pixels.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
        let capacity = (output.pixels.capacity() as u64).checked_mul(32).ok_or(Error::Invalid("group band capacity"))?;
        if capacity > (count as u64).checked_mul(64).ok_or(Error::Invalid("group band capacity"))? {
            return Err(Error::Invalid("group band capacity"));
        }
        output.lease.reconcile_capacity(capacity.checked_add(header).ok_or(Error::Invalid("group band header"))?).map_err(Error::Quota)?;
        for _ in 0..count {
            control.check(32).map_err(Error::Backing)?;
            output.pixels.push([0.; 4]);
        }
        Ok(output)
    }
}
fn item_bounds(doc: &Document, layer: &Layer) -> Result<Rect, Error> {
    doc.xcf_state
        .as_ref()
        .and_then(|state| state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)))
        .and_then(|item| item.raster_bounds)
        .ok_or(Error::Invalid("group canonical extent"))
}
fn mask<'a>(doc: &'a Document, layer: &'a Layer, control: &ReadControl<'_>) -> Result<Option<ResidentNormalMask<'a>>, Error> {
    let Some(physical) = &layer.mask else {
        return Ok(None);
    };
    let state = doc.xcf_state.as_ref().ok_or(Error::Invalid("group canonical mask state"))?;
    let mut matches = state.items.iter().filter_map(|item| item.mask.as_ref().filter(|m| m.owner == ItemRef::Layer(layer.id)));
    let metadata = matches.next().ok_or(Error::Invalid("group canonical mask"))?;
    if matches.next().is_some()
        || metadata.enabled != physical.enabled
        || metadata.linked != physical.linked
        || metadata.shown
        || metadata.editing
        || metadata.outside_f64_bits != 0f64.to_bits()
        || metadata.bounds != item_bounds(doc, layer)?
        || metadata.offset != [metadata.bounds.x0, metadata.bounds.y0]
        || physical.density.to_bits() != 1f32.to_bits()
        || physical.feather.to_bits() != 0f32.to_bits()
        || physical.surface.format() != photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, doc.depth, false)
        || physical.surface.tile_count() > 64
    {
        return Err(Error::Unsupported("group mask context"));
    }
    let view = CheckedRasterView::resident_with_control(physical.surface.as_ref(), metadata.bounds, control).map_err(Error::View)?;
    Ok(Some(ResidentNormalMask { view, enabled: physical.enabled, outside_coverage: 0. }))
}
fn qualify(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<(), Error> {
    let doc = owner.document();
    let fail = || Error::Unsupported("isolated Normal group source context");
    let state = doc.xcf_state.as_ref().ok_or_else(fail)?;
    if doc.layers.len() != 2
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc.working_profile_interpretation != photocraft_doc::WorkingProfileInterpretation::Icc
        || !matches!(doc.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale)
        || !matches!(doc.depth, SampleType::U16 | SampleType::F32 | SampleType::F64)
        || doc.size.width == 0
        || doc.size.height == 0
        || doc.size.width > 320
        || doc.size.height > 320
        || !(4..=5).contains(&state.items.len())
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || state.opaque.iter().any(|v| v.rendering_required)
        || doc.icc_profile.as_ref().is_none_or(|v| !(132..=65536).contains(&v.len()))
    {
        return Err(fail());
    }
    let bottom = doc.layers.first().ok_or_else(fail)?;
    let top = doc.layers.get(1).ok_or_else(fail)?;
    if !matches!(bottom.content, LayerContent::BackedRaster(_))
        || bottom.mask.is_some()
        || !bottom.visible
        || bottom.opacity.to_bits() != 1f32.to_bits()
        || item_bounds(doc, bottom)? != doc.bounds()
    {
        return Err(fail());
    }
    let LayerContent::Group(group) = &top.content else {
        return Err(fail());
    };
    if group.children.len() != 2
        || group.artboard.is_some()
        || group.children.iter().any(|child| !matches!(child.content, LayerContent::BackedRaster(_)) || child.mask.is_some())
    {
        return Err(fail());
    }
    let mut union = item_bounds(doc, group.children.first().ok_or_else(fail)?)?;
    for child in group.children.iter().skip(1) {
        union = union.union(&item_bounds(doc, child)?);
    }
    if union != item_bounds(doc, top)? {
        return Err(fail());
    }
    for layer in std::iter::once(bottom).chain(std::iter::once(top)).chain(group.children.iter()) {
        control.check(1).map_err(Error::Backing)?;
        let _access = accounted_layer_access(doc, layer.id, control).map_err(Error::Backing)?.ok_or(Error::Invalid("group physical identity"))?;
        let extent = item_bounds(doc, layer)?;
        if extent.is_empty() || extent.width() > 320 || extent.height() > 320 {
            return Err(fail());
        }
        let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or_else(fail)?;
        let policy = item.composition.as_ref().ok_or_else(fail)?;
        if policy.compatibility_epoch != EPOCH
            || policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || policy.stored_blend_space != 0
            || policy.stored_composite_space != 1
            || policy.stored_operator != 1
            || policy
                .resolved
                .as_ref()
                .is_none_or(|p| p.operator != GimpOperator::Union || p.blend_space != GimpSpace::LinearRgb || p.composite_space != GimpSpace::LinearRgb)
            || item.visible != layer.visible
            || layer.blend != photocraft_doc::BlendMode::Normal
            || !layer.opacity.is_finite()
            || !(0. ..=1.).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.indexed_source.is_some()
            || layer.import_compositing.is_some()
            || layer.video.is_some()
            || layer.fill_cache.is_some()
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
        {
            return Err(fail());
        }
        if let LayerContent::BackedRaster(backed) = &layer.content
            && (backed.format() != doc.pixel_format() || !owner.quota().shares_ledger(backed.quota()) || item_bounds(doc, layer)? != backed.operative_bounds())
        {
            return Err(fail());
        }
    }
    let _ = mask(doc, top, control)?;
    if state.items.len() != 4 + usize::from(top.mask.is_some()) {
        return Err(fail());
    }
    Ok(())
}

fn cms_error(error: photocraft_cms::control::Error) -> Error {
    match error {
        photocraft_cms::control::Error::Cancelled => Error::Backing(photocraft_raster::backing::BackingError::Cancelled),
        photocraft_cms::control::Error::Limit(s) => Error::Unsupported(s),
        photocraft_cms::control::Error::Allocation => Error::Allocation,
        photocraft_cms::control::Error::Cms(e) => Error::Color(crate::gimp_composite::CompositionError::Color(e)),
    }
}
fn scan(view: CheckedRasterView<'_>, control: &ReadControl<'_>) -> Result<(), Error> {
    let format = view.format();
    let bpp = format.bytes_per_pixel();
    for channel in 0..format.channels() {
        control.check(1).map_err(Error::Backing)?;
        let value = photocraft_color::try_read_sample_f64(view.default_bytes(), format.sample, channel)
            .map_err(|e| Error::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(Error::Unsupported("group native default must be finite unit"));
        }
    }
    let bounds = view.logical_bounds();
    for coord in bounds.tiles() {
        let pin = view.pin_tile(coord, control).map_err(Error::View)?;
        let part = coord.rect().intersect(&bounds);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                control.check(format.channels() as u64).map_err(Error::Backing)?;
                let mut raw = [0; 40];
                let bytes = raw.get_mut(..bpp).ok_or(Error::Invalid("group native layout"))?;
                pin.read_pixel_native_into(x, y, bytes, control).map_err(Error::View)?;
                for channel in 0..format.channels() {
                    let value = photocraft_color::try_read_sample_f64(bytes, format.sample, channel)
                        .map_err(|e| Error::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(Error::Unsupported("group native samples must be finite unit"));
                    }
                }
            }
        }
    }
    control.check(0).map_err(Error::Backing)
}
#[cfg(test)]
#[path = "backed_group_tests.rs"]
mod tests;
