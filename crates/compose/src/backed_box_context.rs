//! Additive streamed CURRENT float Box over genuine native ownership.
//! Rendering bounds extra renderer memory, not total input/store/cache custody.
//! Source admission and the old eager/public constructors remain independent.
use crate::{
    backed_neighborhood_context::assigned_profile::BoundAssignedNeighborhoodProfile,
    backed_normal::{BackedNormalError as E, NormalBandLimits},
    backed_stock_point_context::{OwnedStockPointWideBand, PlanBudget, bound_profile},
    gimp_composite::{CompositionError, WorkingSpace},
};
use photocraft_algo::gimp::neighborhood::{BoxBlurTilePlan, GaussianLimits, try_box_blur_packed_tile};
use photocraft_cms::{
    Profile,
    xcf_color::{self, Encoding},
};
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::{
    BackedLayer, Document, LayerContent, LayerId, SampleEncoding,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::RasterInputError;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
const CACHE: u64 = 16 << 20;
const JOB: u64 = 32 << 20;
fn scope() -> E {
    E::Unsupported("closed streamed CURRENT float Box recipe")
}
fn size() -> E {
    E::Backing(BackingError::Length)
}
fn lower(e: RasterInputError) -> E {
    match e {
        RasterInputError::Cancelled => E::Backing(BackingError::Cancelled),
        RasterInputError::AllocationFailed => E::Allocation,
        other => E::Color(other.into()),
    }
}
fn cms(e: photocraft_cms::control::Error) -> E {
    match e {
        photocraft_cms::control::Error::Cancelled => E::Backing(BackingError::Cancelled),
        photocraft_cms::control::Error::Allocation => E::Allocation,
        photocraft_cms::control::Error::Limit(_) => E::Backing(BackingError::Quota),
        photocraft_cms::control::Error::Cms(e) => E::Color(CompositionError::Color(e)),
    }
}
/// Allocation-free routing hint only. Malformed selected recipes must fail in
/// `new`; this does not authorize a source or an implicit fallback.
pub fn is_candidate(doc: &Document, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    c.check(1)?;
    if doc.mode != ColorMode::Rgb || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile) || !matches!(doc.depth, SampleType::F32 | SampleType::F64) {
        return Ok(false);
    }
    let Some(state) = doc.xcf_state.as_ref() else { return Ok(false) };
    let [stack] = state.filter_stacks.as_slice() else { return Ok(false) };
    let [node] = stack.nodes.as_slice() else { return Ok(false) };
    if node.operation.name.len() > 128 {
        return Ok(false);
    }
    c.check(128)?;
    Ok(node.operation.name == "gegl:box-blur")
}
struct Budget<'a, 'i> {
    q: &'a BackingQuota,
    c: &'a ReadControl<'i>,
    held: u64,
}
impl PlanBudget for Budget<'_, '_> {
    type Lease = QuotaLease;
    fn reserve(&mut self, n: u64) -> Result<QuotaLease, E> {
        let total = self.held.checked_add(n).ok_or_else(size)?;
        if total > JOB {
            return Err(E::Backing(BackingError::Quota));
        }
        let fee = self.q.reserve(QuotaKind::Resident, n).map_err(E::Quota)?;
        self.held = total;
        Ok(fee)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.c.check(n).map_err(E::Backing)
    }
}
enum ProfileOwner {
    Implicit(Profile),
    Assigned(BoundAssignedNeighborhoodProfile<QuotaLease>),
}
impl ProfileOwner {
    fn profile(&self) -> &Profile {
        match self {
            Self::Implicit(p) => p,
            Self::Assigned(p) => p.profile(),
        }
    }
}
struct Binding {
    working: WorkingSpace,
    _profile: ProfileOwner,
    bytes: u64,
    _lease: QuotaLease,
}
impl Binding {
    fn new(owner: &AccountedDocument, assigned: bool, base_bytes: u64, c: &ReadControl<'_>) -> Result<Self, E> {
        let envelope = crate::gimp_neighborhood::scalar_preparation_bytes(2)
            .map_err(E::Color)?
            .checked_add(2 * 65536)
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() as u64 + 128))
            .ok_or_else(size)?;
        let mut b = Budget { q: owner.quota(), c, held: base_bytes };
        let fee = b.reserve(envelope)?;
        let (profile, implicit_basis, extra) = if assigned {
            let p = BoundAssignedNeighborhoodProfile::prepare(owner.document(), c.interrupt, &mut b)?;
            let extra = u64::try_from(p.held_peak()?)
                .map_err(|_| size())?
                .checked_add(std::mem::size_of::<BoundAssignedNeighborhoodProfile<QuotaLease>>() as u64)
                .ok_or_else(size)?;
            (ProfileOwner::Assigned(p), None, extra)
        } else {
            let (p, basis) = bound_profile(owner.document(), None, &mut b, c.interrupt)?;
            (ProfileOwner::Implicit(p), Some(basis), 0)
        };
        let bytes = envelope.checked_add(extra).ok_or_else(size)?;
        if bytes > JOB {
            return Err(scope());
        }
        let basis: &Profile = match implicit_basis {
            Some(p) => p,
            None => profile.profile(),
        };
        let semantic = xcf_color::assigned_effective_planning_work_ceiling(basis, Encoding::NonlinearProfile).map_err(cms)?;
        c.check(semantic.checked_mul(3).ok_or_else(size)?).map_err(E::Backing)?;
        c.check(envelope.checked_mul(64).ok_or_else(size)?).map_err(E::Backing)?;
        let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, c.interrupt).map_err(E::Color)?;
        let out = Self { working, _profile: profile, bytes, _lease: fee };
        c.check(0).map_err(E::Backing)?;
        Ok(out)
    }
}
struct Packed {
    pixels: Vec<[f64; 4]>,
    _lease: QuotaLease,
}
struct Tile {
    raw: Vec<u8>,
    rect: Rect,
    _lease: QuotaLease,
}
struct Output {
    pixels: Vec<[f64; 4]>,
    lease: QuotaLease,
}
/// All source data remains borrowed from the exact immutable accounted model.
/// No Surface materialization or callback chain is retained by this context.
pub struct PreparedBackedBoxContext<'a> {
    binding: Binding,
    owner: &'a AccountedDocument,
    layer: &'a BackedLayer,
    source: Rect,
    output: Rect,
    radius: u32,
    enabled: bool,
    layer_id: LayerId,
    limits: NormalBandLimits,
    prepared_bytes: u64,
    _lease: QuotaLease,
}
impl<'a> PreparedBackedBoxContext<'a> {
    pub fn new(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        Self::new_with_overlap(owner, limits, 0, c)
    }
    pub(crate) fn new_with_overlap(owner: &'a AccountedDocument, limits: NormalBandLimits, overlap: u64, c: &ReadControl<'_>) -> Result<Self, E> {
        c.check(1).map_err(E::Backing)?;
        let maximum = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > maximum.max_rows || limits.max_pixels == 0 || limits.max_pixels > maximum.max_pixels {
            return Err(scope());
        }
        let assigned = owner.is_readonly_assigned_point_storage();
        if assigned { owner.validate_readonly_assigned_point_storage(c) } else { owner.validate_readonly_point_storage(c) }.map_err(E::Backing)?;
        let doc = owner.document();
        c.check(32768).map_err(E::Backing)?;
        let [layer] = doc.layers.as_slice() else { return Err(scope()) };
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
        let source = backed.operative_bounds();
        let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
        let [stack] = state.filter_stacks.as_slice() else { return Err(scope()) };
        let [node] = stack.nodes.as_slice() else { return Err(scope()) };
        let source_color = doc.source_color_context.as_ref().ok_or_else(scope)?;
        if doc.mode != ColorMode::Rgb
            || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
            || !matches!((doc.depth, source_color.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750))
            || assigned && doc.depth != SampleType::F32
            || assigned != source_color.original_icc.is_some()
            || state.provenance.format_version != 22
            || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
            || state.items.len() != 2
            || state.filter_masks.len() != 1
            || stack.owner != ItemRef::Layer(layer.id)
            || !state.editable_indexed.is_empty()
            || node.composition.as_ref().is_none_or(|p| p.compatibility_epoch != "xcf22-rgb-box-replace-v1")
        {
            return Err(scope());
        }
        crate::gimp_render::qualify_current_point_optional_mask_metadata(
            doc,
            layer,
            state,
            stack,
            source,
            SampleEncoding::NonlinearProfile,
            backed.format(),
            backed.default_bytes(),
        )
        .map_err(E::Color)?;
        let item = state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
        let resolved = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if resolved.operator != GimpOperator::Union
            || resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || physical.pass_through
            || physical.blend_encoding != SampleEncoding::Linear
            || physical.composite_encoding != SampleEncoding::Linear
            || layer.blend != photocraft_color::blend::BlendMode::Normal
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
        {
            return Err(scope());
        }
        let radius = crate::gimp_neighborhood::box_blur::radius_for_current_float(node, 22).map_err(E::Color)?;
        let mask = state.filter_masks.first().ok_or_else(scope)?;
        if node.mask != Some(ItemRef::Mask(mask.id))
            || mask.owner != node.id
            || mask.bounds != doc.bounds()
            || mask.surface.format() != photocraft_color::PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
        {
            return Err(scope());
        }
        let maskwork = (mask.surface.tile_count() as u64)
            .checked_add(1)
            .and_then(|n| n.checked_mul(65536))
            .and_then(|n| n.checked_mul(doc.depth.bytes() as u64))
            .ok_or_else(size)?;
        c.check(maskwork).map_err(E::Backing)?;
        let empty = mask.is_empty_selection_with(c.interrupt);
        c.check(0).map_err(E::Backing)?;
        if !empty.map_err(|_| scope())? {
            return Err(scope());
        }
        let header = std::mem::size_of::<Self>() as u64 + 128;
        let fee = owner.quota().reserve(QuotaKind::Index, header).map_err(E::Quota)?;
        let binding = Binding::new(owner, assigned, header.checked_add(overlap).ok_or_else(size)?, c)?;
        let prepared_bytes = binding.bytes.checked_add(header).ok_or_else(size)?;
        if prepared_bytes > JOB {
            return Err(scope());
        }
        c.check(1024).map_err(E::Backing)?;
        let plan = BoxBlurTilePlan::new(source, TileCoord::containing(source.x0, source.y0), radius, c.interrupt).map_err(lower)?;
        let output = if node.enabled { plan.output_extent() } else { source };
        let out =
            Self { binding, owner, layer: backed, source, output, radius, enabled: node.enabled, layer_id: layer.id, limits, prepared_bytes, _lease: fee };
        out.validate_native(c)?;
        c.check(0).map_err(E::Backing)?;
        Ok(out)
    }
    pub fn bounds(&self) -> Rect {
        self.output
    }
    pub fn layer_id(&self) -> LayerId {
        self.layer_id
    }
    pub fn layer_visible(&self) -> bool {
        self.owner.document().layers.first().is_some_and(|v| v.visible)
    }
    pub fn layer_opacity(&self) -> f64 {
        self.owner.document().layers.first().map_or(0., |v| f64::from(v.opacity))
    }
    pub(crate) fn admit(&self, extra: u64) -> Result<(), E> {
        if self.prepared_bytes.checked_add(extra).ok_or_else(size)? > JOB { Err(scope()) } else { Ok(()) }
    }
    fn validate_native(&self, c: &ReadControl<'_>) -> Result<(), E> {
        let format = self.layer.format();
        c.check(format.bytes_per_pixel() as u64).map_err(E::Backing)?;
        let mut default = [0.; 4];
        for (channel, v) in default.iter_mut().enumerate() {
            *v = try_read_sample_f64(self.layer.default_bytes(), format.sample, channel)
                .map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
        }
        valid(default)?;
        let view = self.layer.checked_view();
        for coordinate in self.source.tiles() {
            let pin = view.pin_tile(coordinate, c).map_err(E::View)?;
            let part = coordinate.rect().intersect(&self.source);
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    c.check(4).map_err(E::Backing)?;
                    let p = read_pin(&pin, x, y, format, c)?;
                    valid(p)?;
                }
            }
        }
        Ok(())
    }
    fn packed(&self, plan: &BoxBlurTilePlan, c: &ReadControl<'_>) -> Result<Packed, E> {
        let count = plan.packed_input_pixels().map_err(lower)?;
        let cap = count.checked_mul(2).ok_or_else(size)?;
        let envelope = cap.checked_mul(32).and_then(|n| n.checked_add(std::mem::size_of::<Packed>() + 128)).ok_or_else(size)? as u64;
        let fee = self.owner.quota().reserve(QuotaKind::Resident, envelope).map_err(E::Quota)?;
        let mut out = Packed { pixels: Vec::new(), _lease: fee };
        c.check((count as u64).checked_mul(4).ok_or_else(size)?).map_err(E::Backing)?;
        out.pixels.try_reserve_exact(count).map_err(|_| E::Allocation)?;
        if out.pixels.capacity() > cap {
            return Err(E::Allocation);
        }
        out.pixels.resize(count, [0.; 4]);
        let read = plan.source_read();
        let view = self.layer.checked_view();
        for coordinate in read.tiles() {
            let pin = view.pin_tile(coordinate, c).map_err(E::View)?;
            let part = coordinate.rect().intersect(&read);
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    c.check(4 + 3 * 128).map_err(E::Backing)?;
                    let p = read_pin(&pin, x, y, self.layer.format(), c)?;
                    valid(p)?;
                    let rgb = self.binding.working.convert([p[0], p[1], p[2]], true).map_err(E::Color)?;
                    let index = index(read, x, y)?;
                    *out.pixels.get_mut(index).ok_or_else(size)? = [rgb[0], rgb[1], rgb[2], p[3]];
                }
            }
        }
        let actual = (out.pixels.capacity() as u64).checked_mul(32).and_then(|n| n.checked_add(std::mem::size_of::<Packed>() as u64 + 128)).ok_or_else(size)?;
        out._lease.reconcile_capacity(actual).map_err(E::Quota)?;
        Ok(out)
    }
    fn tile(&self, coordinate: TileCoord, band_bytes: u64, c: &ReadControl<'_>) -> Result<Tile, E> {
        c.check(1024).map_err(E::Backing)?;
        let plan = BoxBlurTilePlan::new(self.source, coordinate, self.radius, c.interrupt).map_err(lower)?;
        let req = plan.requirements(c.interrupt).map_err(lower)?;
        let packedbytes = (plan.packed_input_pixels().map_err(lower)? as u64)
            .checked_mul(64)
            .and_then(|n| n.checked_add(std::mem::size_of::<Packed>() as u64 + 128))
            .ok_or_else(size)?;
        let kernelbytes = req.peak_resident_bytes.checked_add(std::mem::size_of::<Tile>() as u64 + 128).ok_or_else(size)?;
        let private = packedbytes.checked_add(kernelbytes).ok_or_else(size)?;
        if private > CACHE {
            return Err(scope());
        }
        self.admit(private.checked_add(band_bytes).ok_or_else(size)?)?;
        let packed = self.packed(&plan, c)?;
        let fee = self.owner.quota().reserve(QuotaKind::Resident, kernelbytes).map_err(E::Quota)?;
        c.check(req.work_units).map_err(E::Backing)?;
        let raw = try_box_blur_packed_tile(
            &plan,
            &packed.pixels,
            GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
            c.interrupt,
        )
        .map_err(lower)?;
        let mut tile = Tile { raw, rect: plan.tile_output(), _lease: fee };
        drop(packed);
        let actual = (tile.raw.capacity() as u64).checked_add(std::mem::size_of::<Tile>() as u64 + 128).ok_or_else(size)?;
        if actual > kernelbytes || tile.raw.len() != plan.output_bytes().map_err(lower)? {
            return Err(E::Allocation);
        }
        tile._lease.reconcile_capacity(actual).map_err(E::Quota)?;
        for bytes in tile.raw.chunks_exact_mut(32) {
            c.check(4 + 3 * 128).map_err(E::Backing)?;
            let p = decode(bytes)?;
            valid(p)?;
            let rgb = self.binding.working.convert([p[0], p[1], p[2]], false).map_err(E::Color)?;
            let p = [rgb[0], rgb[1], rgb[2], p[3]];
            valid(p)?;
            for (channel, value) in p.into_iter().enumerate() {
                try_write_sample_f64(bytes, SampleType::F64, channel, value)
                    .map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
            }
        }
        c.check(0).map_err(E::Backing)?;
        Ok(tile)
    }
    /// Full expanded operative validation is required before publication, even if
    /// the canvas, layer activity or requested ROI would hide a failing word.
    pub fn validate_operative(&self, c: &ReadControl<'_>) -> Result<(), E> {
        self.validate_with_overlap(0, c)
    }
    pub(crate) fn validate_with_overlap(&self, overlap: u64, c: &ReadControl<'_>) -> Result<(), E> {
        self.admit(overlap)?;
        c.check(1).map_err(E::Backing)?;
        if self.enabled {
            for coordinate in self.output.tiles() {
                drop(self.tile(coordinate, overlap, c)?);
            }
        }
        c.check(0).map_err(E::Backing)
    }
    pub fn render_wide_band(&self, rect: Rect, c: &ReadControl<'_>) -> Result<OwnedStockPointWideBand, E> {
        self.render_with_overlap(rect, 0, c)
    }
    pub(crate) fn render_with_overlap(&self, rect: Rect, overlap: u64, c: &ReadControl<'_>) -> Result<OwnedStockPointWideBand, E> {
        c.check(1).map_err(E::Backing)?;
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(size)?;
        if rect.is_empty() || !self.output.contains_rect(&rect) || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(scope());
        }
        let count = usize::try_from(count).map_err(|_| size())?;
        let cap = count.checked_mul(2).ok_or_else(size)?;
        let envelope = cap
            .checked_mul(32)
            .and_then(|n| n.checked_add(std::mem::size_of::<Output>() + std::mem::size_of::<OwnedStockPointWideBand>() + 128))
            .ok_or_else(size)? as u64;
        self.admit(envelope.checked_add(overlap).ok_or_else(size)?)?;
        let fee = self.owner.quota().reserve(QuotaKind::Resident, envelope).map_err(E::Quota)?;
        let mut out = Output { pixels: Vec::new(), lease: fee };
        c.check((count as u64).checked_mul(4).ok_or_else(size)?).map_err(E::Backing)?;
        out.pixels.try_reserve_exact(count).map_err(|_| E::Allocation)?;
        if out.pixels.capacity() > cap {
            return Err(E::Allocation);
        }
        out.pixels.resize(count, [0.; 4]);
        if !self.enabled {
            let view = self.layer.checked_view();
            for coordinate in rect.tiles() {
                let pin = view.pin_tile(coordinate, c).map_err(E::View)?;
                let part = coordinate.rect().intersect(&rect);
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        c.check(4).map_err(E::Backing)?;
                        let p = read_pin(&pin, x, y, self.layer.format(), c)?;
                        valid(p)?;
                        *out.pixels.get_mut(index(rect, x, y)?).ok_or_else(size)? = p;
                    }
                }
            }
        } else {
            for coordinate in rect.tiles() {
                let tile = self.tile(coordinate, envelope.checked_add(overlap).ok_or_else(size)?, c)?;
                let part = tile.rect.intersect(&rect);
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        c.check(32).map_err(E::Backing)?;
                        let offset = index(tile.rect, x, y)?.checked_mul(32).ok_or_else(size)?;
                        let p = decode(tile.raw.get(offset..offset.checked_add(32).ok_or_else(size)?).ok_or_else(size)?)?;
                        *out.pixels.get_mut(index(rect, x, y)?).ok_or_else(size)? = p;
                    }
                }
            }
        }
        c.check(0).map_err(E::Backing)?;
        let Output { pixels, lease } = out;
        Ok(OwnedStockPointWideBand::from_accounted_pixels(rect, pixels, lease))
    }
}
fn valid(p: [f64; 4]) -> Result<(), E> {
    if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) { Err(scope()) } else { Ok(()) }
}
fn decode(bytes: &[u8]) -> Result<[f64; 4], E> {
    let mut p = [0.; 4];
    for (channel, v) in p.iter_mut().enumerate() {
        *v = try_read_sample_f64(bytes, SampleType::F64, channel).map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
    }
    Ok(p)
}
fn index(rect: Rect, x: i32, y: i32) -> Result<usize, E> {
    if !rect.contains(x, y) {
        return Err(size());
    }
    usize::try_from(i64::from(y) - i64::from(rect.y0))
        .map_err(|_| size())?
        .checked_mul(rect.width() as usize)
        .and_then(|n| n.checked_add(usize::try_from(i64::from(x) - i64::from(rect.x0)).ok()?))
        .ok_or_else(size)
}

fn read_pin(
    pin: &photocraft_raster::checked_view::CheckedTilePin,
    x: i32,
    y: i32,
    format: photocraft_color::PixelFormat,
    c: &ReadControl<'_>,
) -> Result<[f64; 4], E> {
    let mut raw = [0; 40];
    let bytes = raw.get_mut(..format.bytes_per_pixel()).ok_or_else(size)?;
    pin.read_pixel_native_into(x, y, bytes, c).map_err(E::View)?;
    let mut p = [0.; 4];
    for (channel, value) in p.iter_mut().enumerate() {
        *value = try_read_sample_f64(bytes, format.sample, channel).map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
    }
    Ok(p)
}
