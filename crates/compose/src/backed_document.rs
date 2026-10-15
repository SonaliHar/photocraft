//! Checked native Document consumer for fallible, quota-owned storage.
//! Initial explicit slice: flat owned-backed raster layers, Normal/Union, current
//! ICC interpretation, plus a closed accounted plain-import current-storage lane.
//! Unsupported semantics return errors before any pixel reads.
use crate::backed_normal::{BackedNormalError, NormalBandLimits, ResidentNormalMask};
use crate::backed_normal_stack::{NormalStackLayer, NormalStackLimits, OwnedNormalStackBand, PreparedBackedNormalStack, ResidentNormalStackLayer};
use photocraft_cms::Profile;
use photocraft_color::{ColorMode, blend::BlendMode};
use photocraft_doc::{Document, LayerContent, SampleEncoding, WorkingProfileInterpretation, import_compositing::GimpSpace};
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};

#[path = "backed_group.rs"]
mod group;
#[path = "backed_document/plain_import.rs"]
mod plain_import;
// Keep the prepared profile owner inline: both routes charge the compiled
// Document owner header before publication, without an extra infallible Box.
#[allow(clippy::large_enum_variant)]
enum DocumentCompositor<'a> {
    Flat(PreparedBackedNormalStack<'a>),
    Group(group::PreparedGroup<'a>),
    Threshold(crate::backed_threshold_document::PreparedBackedThresholdDocument<'a>),
    ThresholdAlpha(crate::backed_threshold_alpha_document::PreparedBackedThresholdAlphaDocument<'a>),
}

pub struct PreparedBackedDocument<'a> {
    stack: DocumentCompositor<'a>,
    limits: NormalBandLimits,
    quota: BackingQuota,
    bounds: Rect,
    _index: QuotaLease,
}

/// Derived straight-alpha samples in the explicitly prepared current encoding.
/// This display owner is not a native sample store or a write-back authority.
#[derive(Debug)]
pub struct OwnedBackedReducedBuffer {
    rect: Rect,
    pixels: Vec<[f32; 4]>,
    _lease: QuotaLease,
}
impl OwnedBackedReducedBuffer {
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }
    /// Transfer the pixels and their persistent charge together, without copying.
    /// A cache must retain the lease until the buffer is dropped or transferred.
    pub fn into_parts(self) -> (crate::Buffer, QuotaLease) {
        let Self { rect, pixels, _lease } = self;
        (crate::Buffer { rect, px: pixels }, _lease)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BackedDocumentSinkError<E> {
    Render(BackedNormalError),
    Sink(E),
}
impl<'a> PreparedBackedDocument<'a> {
    /// Profile/encoding are explicit current interpretation. The profile ICC
    /// bytes must match the current document, never immutable import provenance.
    pub fn new(doc: &'a Document, profile: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        Self::new_internal(doc, profile, limits, control, None)
    }
    /// Qualified resident masks and closed plain imports require the actual paid
    /// model/source guard. This does not extend ordinary bare-Document admission.
    pub fn new_accounted(
        owner: &'a photocraft_doc::accounted_document::AccountedDocument,
        profile: &Profile,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedNormalError> {
        if owner.is_readonly_threshold_storage() {
            let prepared = crate::backed_threshold_document::PreparedBackedThresholdDocument::new(owner, profile, limits, control)?;
            let index = owner.quota().reserve(QuotaKind::Index, std::mem::size_of::<Self>() as u64).map_err(BackedNormalError::Quota)?;
            let bounds = prepared.bounds();
            control.check(1).map_err(BackedNormalError::Backing)?;
            return Ok(Self { stack: DocumentCompositor::Threshold(prepared), limits, quota: owner.quota().clone(), bounds, _index: index });
        }
        if owner.is_readonly_threshold_alpha_storage() {
            let prepared = crate::backed_threshold_alpha_document::PreparedBackedThresholdAlphaDocument::new(owner, profile, limits, control)?;
            let index = owner.quota().reserve(QuotaKind::Index, std::mem::size_of::<Self>() as u64).map_err(BackedNormalError::Quota)?;
            let bounds = prepared.bounds();
            control.check(1).map_err(BackedNormalError::Backing)?;
            return Ok(Self { stack: DocumentCompositor::ThresholdAlpha(prepared), limits, quota: owner.quota().clone(), bounds, _index: index });
        }
        if owner.document().layers.iter().any(|layer| matches!(layer.content, LayerContent::Group(_))) {
            let prepared = group::PreparedGroup::new(owner, profile, limits, control)?;
            let index = owner.quota().reserve(QuotaKind::Index, std::mem::size_of::<Self>() as u64).map_err(BackedNormalError::Quota)?;
            control.check(1).map_err(BackedNormalError::Backing)?;
            return Ok(Self { stack: DocumentCompositor::Group(prepared), limits, quota: owner.quota().clone(), bounds: owner.document().bounds(), _index: index });
        }
        Self::new_internal(owner.document(), profile, limits, control, Some(owner))
    }
    fn new_internal(
        doc: &'a Document,
        profile: &Profile,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
        retained_source: Option<&'a photocraft_doc::accounted_document::AccountedDocument>,
    ) -> Result<Self, BackedNormalError> {
        if doc.xcf_state.is_some() && retained_source.is_none() {
            return Err(BackedNormalError::Unsupported("ordinary backed masks require accounted source ownership"));
        }
        control.check(1).map_err(BackedNormalError::Backing)?;
        if doc.layers.is_empty() || doc.layers.len() > 10_000 {
            return Err(BackedNormalError::Invalid("backed Document layer count"));
        }
        let plain = if doc.xcf_state.is_some() && !doc.layers.iter().any(|layer| layer.mask.is_some()) {
            Some(plain_import::qualify(
                retained_source.ok_or(BackedNormalError::Unsupported("plain imported stack requires accounted owner"))?,
                profile,
                control,
            )?)
        } else {
            None
        };
        if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || (plain.is_none() && !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::Icc))
            || !doc.channels.is_empty()
        {
            return Err(BackedNormalError::Unsupported("initial backed Document explicit ICC RGB/Gray slice"));
        }
        let ordinary = if plain.is_some() { false } else { qualify_ordinary_mask(doc, control)? };
        let stored = doc.sample_encoding.ok_or(BackedNormalError::Unsupported("backed Document requires explicit current encoding"))?;
        let composite = if let Some(composite) = plain {
            composite
        } else if ordinary {
            GimpSpace::LinearRgb
        } else {
            match stored {
                SampleEncoding::Linear => GimpSpace::LinearRgb,
                SampleEncoding::NonlinearProfile => GimpSpace::ProfileRgb,
                SampleEncoding::Perceptual => GimpSpace::PerceptualRgb,
            }
        };
        if profile.channels() != if doc.mode == ColorMode::Rgb { 3 } else { 1 }
            || (plain.is_none()
                && doc.icc_profile.as_deref().map(Vec::as_slice)
                    != Some(profile.encoded_bytes().ok_or(BackedNormalError::Unsupported("current profile encoding must be retained"))?))
        {
            return Err(BackedNormalError::Invalid("backed Document current profile mismatch"));
        }
        let first = doc
            .layers
            .iter()
            .find_map(|layer| match &layer.content {
                LayerContent::BackedRaster(backed) => Some(backed),
                _ => None,
            })
            .ok_or(BackedNormalError::Unsupported("backed Document requires owned-backed pixels"))?;
        // Validate every layer's operative semantic fields, including hidden
        // layers. This preparation does not infer that hidden unsupported state
        // is harmless, and it performs no source tile reads in this pass.
        for layer in &doc.layers {
            control.check(1).map_err(BackedNormalError::Backing)?;
            let LayerContent::BackedRaster(backed) = &layer.content else {
                return Err(BackedNormalError::Unsupported("mixed resident/backed Document not migrated"));
            };
            if backed.format().sample != doc.depth
                || backed.format().mode != doc.mode
                || !first.quota().shares_ledger(backed.quota())
                || layer.blend != BlendMode::Normal
                || !layer.opacity.is_finite()
                || !(0.0..=1.0).contains(&layer.opacity)
                || layer.fill_opacity.to_bits() != 1f32.to_bits()
                || layer.clipped
                || (layer.mask.is_some() && !ordinary)
                || layer.vector_mask.is_some()
                || !layer.effects.items.is_empty()
                || layer.indexed_source.is_some()
                || (plain.is_none()
                    && layer.import_compositing.is_some_and(|policy| {
                        !ordinary
                            || policy.pass_through
                            || policy.blend_encoding != SampleEncoding::Linear
                            || policy.composite_encoding != SampleEncoding::Linear
                    }))
                || layer.video.is_some()
                || layer.fill_cache.is_some()
                || layer.excluded_channels != 0
                || layer.blend_if != Default::default()
                || layer.advanced != Default::default()
            {
                return Err(BackedNormalError::Unsupported("backed Document semantics not migrated"));
            }
        }
        let capacity = doc
            .layers
            .len()
            .checked_mul(std::mem::size_of::<ResidentNormalStackLayer<'a>>())
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(64))
            .and_then(|v| v.checked_add(std::mem::size_of::<Self>()))
            .ok_or(BackedNormalError::Invalid("backed Document index size"))?;
        let mut index = first.quota().reserve(QuotaKind::Index, capacity as u64).map_err(BackedNormalError::Quota)?;
        let mut inputs = Vec::new();
        inputs.try_reserve_exact(doc.layers.len()).map_err(|_| BackedNormalError::Allocation)?;
        index
            .reconcile_capacity(
                (inputs.capacity() as u64)
                    .checked_mul(std::mem::size_of::<ResidentNormalStackLayer<'a>>() as u64)
                    .and_then(|n| n.checked_add(std::mem::size_of::<Self>() as u64))
                    .ok_or(BackedNormalError::Invalid("backed Document index capacity"))?,
            )
            .map_err(BackedNormalError::Quota)?;
        // Both Document layers and the explicit stack are bottom-to-top.
        for layer in &doc.layers {
            let LayerContent::BackedRaster(backed) = &layer.content else {
                return Err(BackedNormalError::Unsupported("backed Document content changed"));
            };
            let mask = if let Some(mask) = &layer.mask {
                let state = doc.xcf_state.as_ref().ok_or(BackedNormalError::Unsupported("resident mask requires qualified source context"))?;
                let metadata = state
                    .items
                    .iter()
                    .find_map(|item| item.mask.as_ref().filter(|m| m.owner == photocraft_doc::item_identity::ItemRef::Layer(layer.id)))
                    .ok_or(BackedNormalError::Invalid("missing operative mask metadata"))?;
                let view = photocraft_raster::checked_view::CheckedRasterView::resident_with_control(mask.surface.as_ref(), metadata.bounds, control)
                    .map_err(BackedNormalError::View)?;
                Some(ResidentNormalMask { view, enabled: mask.enabled, outside_coverage: f64::from_bits(metadata.outside_f64_bits) })
            } else {
                None
            };
            inputs.push(ResidentNormalStackLayer {
                input: NormalStackLayer { layer: backed, visible: layer.visible, opacity: f64::from(layer.opacity), mask: None },
                mask,
            });
        }
        let stack = PreparedBackedNormalStack::new_with_resident_masks(
            &inputs,
            first.quota(),
            profile,
            stored,
            composite,
            NormalStackLimits { band: limits, max_layers: 10_000 },
            control,
        )?;
        drop(inputs);
        // The construction-only vector is gone; retain only owner metadata.
        index.reconcile_capacity(std::mem::size_of::<Self>() as u64).map_err(BackedNormalError::Quota)?;
        control.check(1).map_err(BackedNormalError::Backing)?;
        Ok(Self { stack: DocumentCompositor::Flat(stack), limits, quota: first.quota().clone(), bounds: doc.bounds(), _index: index })
    }
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNormalStackBand, BackedNormalError> {
        match &self.stack {
            DocumentCompositor::Flat(stack) => stack.render_band(rect, [0.; 4], control),
            DocumentCompositor::Group(group) => group.render_band(rect, control),
            DocumentCompositor::Threshold(prepared) => prepared.render_band(rect, control),
            DocumentCompositor::ThresholdAlpha(prepared) => prepared.render_band(rect, control),
        }
    }

    /// Bounded area reduction of the document canvas, averaging premultiplied
    /// samples in the explicit current encoding. Source pixels stream in bands;
    /// the retained output and temporary accumulator share the source ledger.
    pub fn render_reduced(&self, width: u32, height: u32, control: &ReadControl<'_>) -> Result<OwnedBackedReducedBuffer, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let source_w =
            u64::try_from(i64::from(self.bounds.x1) - i64::from(self.bounds.x0)).map_err(|_| BackedNormalError::Invalid("reduction canvas width"))?;
        let source_h =
            u64::try_from(i64::from(self.bounds.y1) - i64::from(self.bounds.y0)).map_err(|_| BackedNormalError::Invalid("reduction canvas height"))?;
        if source_w == 0 || source_h == 0 || width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(BackedNormalError::Invalid("bounded reduction dimensions"));
        }
        let width = u64::from(width).min(source_w);
        let height = u64::from(height).min(source_h);
        let count = usize::try_from(width.checked_mul(height).ok_or(BackedNormalError::Invalid("reduction size"))?)
            .map_err(|_| BackedNormalError::Invalid("reduction size"))?;
        let bytes = (count as u64)
            .checked_mul(48)
            .and_then(|v| v.checked_add(std::mem::size_of::<OwnedBackedReducedBuffer>() as u64))
            .ok_or(BackedNormalError::Invalid("reduction capacity"))?;
        let mut lease = self.quota.reserve(QuotaKind::Resident, bytes).map_err(BackedNormalError::Quota)?;
        let mut sums = Vec::<[f64; 4]>::new();
        let mut pixels = Vec::<[f32; 4]>::new();
        sums.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
        pixels.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
        let retained = (pixels.capacity() as u64)
            .checked_mul(16)
            .and_then(|v| v.checked_add(std::mem::size_of::<OwnedBackedReducedBuffer>() as u64))
            .ok_or(BackedNormalError::Invalid("reduction capacity"))?;
        lease
            .reconcile_capacity(
                retained
                    .checked_add((sums.capacity() as u64).checked_mul(32).ok_or(BackedNormalError::Invalid("reduction capacity"))?)
                    .ok_or(BackedNormalError::Invalid("reduction capacity"))?,
            )
            .map_err(BackedNormalError::Quota)?;
        sums.resize(count, [0.0; 4]);
        self.render_bands(self.bounds, control, |band| -> Result<(), BackedNormalError> {
            let rect = band.rect();
            let band_w = usize::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| BackedNormalError::Invalid("reduction band"))?;
            for (i, rgba) in band.pixels().iter().enumerate() {
                if i % 256 == 0 {
                    control.check(256).map_err(BackedNormalError::Backing)?;
                }
                let x =
                    u64::try_from(i64::from(rect.x0) - i64::from(self.bounds.x0)).map_err(|_| BackedNormalError::Invalid("reduction x"))? + (i % band_w) as u64;
                let y =
                    u64::try_from(i64::from(rect.y0) - i64::from(self.bounds.y0)).map_err(|_| BackedNormalError::Invalid("reduction y"))? + (i / band_w) as u64;
                let bx = ((x + 1) * width - 1) / source_w;
                let by = ((y + 1) * height - 1) / source_h;
                let sum = sums
                    .get_mut(usize::try_from(by * width + bx).map_err(|_| BackedNormalError::Invalid("reduction index"))?)
                    .ok_or(BackedNormalError::Invalid("reduction index"))?;
                let [r, g, b, a] = *rgba;
                let [sr, sg, sb, sa] = sum;
                *sr += r * a;
                *sg += g * a;
                *sb += b * a;
                *sa += a;
                if !sum.iter().all(|v| v.is_finite()) {
                    return Err(BackedNormalError::Invalid("non-finite reduction"));
                }
            }
            Ok(())
        })
        .map_err(|e| match e {
            BackedDocumentSinkError::Render(e) | BackedDocumentSinkError::Sink(e) => e,
        })?;
        for (i, [r, g, b, a]) in sums.iter().copied().enumerate() {
            if i % 256 == 0 {
                control.check(256).map_err(BackedNormalError::Backing)?;
            }
            let x = i as u64 % width;
            let y = i as u64 / width;
            let area = ((x + 1) * source_w / width - x * source_w / width) * ((y + 1) * source_h / height - y * source_h / height);
            let rgba = if a == 0.0 { [0.0; 4] } else { [(r / a) as f32, (g / a) as f32, (b / a) as f32, (a / area as f64) as f32] };
            if !rgba.iter().all(|v| v.is_finite()) {
                return Err(BackedNormalError::Unsupported("reduced samples exceed display precision"));
            }
            pixels.push(rgba);
        }
        drop(sums);
        lease.reconcile_capacity(retained).map_err(BackedNormalError::Quota)?;
        control.check(1).map_err(BackedNormalError::Backing)?;
        Ok(OwnedBackedReducedBuffer { rect: Rect::new(0, 0, width as i32, height as i32), pixels, _lease: lease })
    }

    /// Stream complete bounded native bands; wide regions split into columns.
    /// Each returned owner remains charged while retained by the sink. Consumers
    /// that publish a whole image must stage it until this operation succeeds.
    pub fn render_bands<E>(
        &self,
        rect: Rect,
        control: &ReadControl<'_>,
        mut sink: impl FnMut(OwnedNormalStackBand) -> Result<(), E>,
    ) -> Result<(), BackedDocumentSinkError<E>> {
        use BackedDocumentSinkError as Error;
        control.check(1).map_err(|e| Error::Render(BackedNormalError::Backing(e)))?;
        if rect.x1 < rect.x0 || rect.y1 < rect.y0 {
            return Err(Error::Render(BackedNormalError::Invalid("streaming region bounds")));
        }
        if rect.is_empty() {
            return Ok(());
        }
        let width = u64::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| Error::Render(BackedNormalError::Invalid("streaming region width")))?;
        // A cache may retain only one native tile. Wide thin bands would reload
        // full physical tiles for successive rows; bounded tile columns allow
        // each source tile to be consumed once without pin-all staging.
        let columns = width.min(self.limits.max_pixels).min(photocraft_geom::TILE_SIZE as u64);
        let rows = (self.limits.max_pixels / columns).min(u64::from(self.limits.max_rows));
        let columns = i32::try_from(columns).map_err(|_| Error::Render(BackedNormalError::Invalid("streaming columns")))?;
        let rows = i32::try_from(rows).map_err(|_| Error::Render(BackedNormalError::Invalid("streaming rows")))?;
        let mut y = rect.y0;
        while y < rect.y1 {
            let tile_rows = photocraft_geom::TILE_SIZE - y.rem_euclid(photocraft_geom::TILE_SIZE);
            let end_y = y.saturating_add(rows.min(tile_rows)).min(rect.y1);
            let mut x = rect.x0;
            while x < rect.x1 {
                let tile_columns = photocraft_geom::TILE_SIZE - x.rem_euclid(photocraft_geom::TILE_SIZE);
                let end_x = x.saturating_add(columns.min(tile_columns)).min(rect.x1);
                let band = self.render_band(Rect::new(x, y, end_x, end_y), control).map_err(Error::Render)?;
                control.check(1).map_err(|e| Error::Render(BackedNormalError::Backing(e)))?;
                sink(band).map_err(Error::Sink)?;
                x = end_x;
            }
            y = end_y;
        }
        control.check(1).map_err(|e| Error::Render(BackedNormalError::Backing(e)))?;
        Ok(())
    }
}

fn qualify_ordinary_mask(doc: &Document, control: &ReadControl<'_>) -> Result<bool, BackedNormalError> {
    use photocraft_doc::{
        SampleType,
        import_compositing::{GimpOperator, XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH},
        item_identity::ItemRef,
    };
    let Some(state) = &doc.xcf_state else {
        return Ok(false);
    };
    let scope = || BackedNormalError::Unsupported("backed ordinary Normal mask source context");
    control.check(32).map_err(BackedNormalError::Backing)?;
    let canvas_pixels = u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(scope)?;
    if doc.layers.len() != 2
        || state.items.len() != 3
        || doc.layers.first().zip(doc.layers.get(1)).is_none_or(|(a, b)| a.id == b.id)
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(doc.depth, SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        || doc.size.width == 0
        || doc.size.height == 0
        || doc.size.width > 262144
        || doc.size.height > 262144
        || canvas_pixels > 1 << 30
        || doc.icc_profile.as_ref().is_none_or(|bytes| !(132..=65536).contains(&bytes.len()))
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || state.opaque.iter().any(|record| record.rendering_required)
    {
        return Err(scope());
    }
    for (index, layer) in doc.layers.iter().enumerate() {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let LayerContent::BackedRaster(backed) = &layer.content else {
            return Err(scope());
        };
        let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
        if state.items.iter().filter(|item| item.item == ItemRef::Layer(layer.id)).count() != 1 {
            return Err(scope());
        }
        let extent = item.raster_bounds.ok_or_else(scope)?;
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        let resolved = policy.resolved.as_ref().ok_or_else(scope)?;
        if !layer.visible
            || !item.visible
            || item.parent.is_some()
            || extent != backed.operative_bounds()
            || extent.is_empty()
            || extent.width() > 262144
            || extent.height() > 262144
            || policy.compatibility_epoch != XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH
            || policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || resolved.operator != GimpOperator::Union
            || policy.stored_blend_space != 0
            || policy.stored_composite_space != if index == 0 { -1 } else { 1 }
            || policy.stored_operator != if index == 0 { -1 } else { 1 }
        {
            return Err(scope());
        }
        if index == 0 {
            if extent != doc.bounds() || layer.opacity.to_bits() != 1f32.to_bits() || layer.mask.is_some() {
                return Err(scope());
            }
        } else {
            let mask = layer.mask.as_ref().ok_or_else(scope)?;
            let mut records = state.items.iter().filter(|item| item.mask.as_ref().is_some_and(|m| m.owner == ItemRef::Layer(layer.id)));
            let record = records.next().ok_or_else(scope)?;
            if records.next().is_some() || !matches!(record.item, ItemRef::Mask(_)) {
                return Err(scope());
            }
            let metadata = record.mask.as_ref().ok_or_else(scope)?;
            if !mask.enabled
                || mask.density.to_bits() != 1f32.to_bits()
                || mask.feather.to_bits() != 0f32.to_bits()
                || mask.surface.tile_count() > 100_000
                || mask.surface.format() != photocraft_doc::PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
                || metadata.bounds != extent
                || metadata.offset != [extent.x0, extent.y0]
                || metadata.outside_f64_bits != 0f64.to_bits()
                || !metadata.enabled
                || metadata.shown
                || metadata.editing
                || metadata.linked != mask.linked
            {
                return Err(scope());
            }
        }
    }
    Ok(true)
}
