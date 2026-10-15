//! Observational Box source preparation. Raw Replace words are retained opaque;
//! this closed envelope is not a general interpretation of those words.
use crate::{
    backed_normal::BackedNormalError as E,
    backed_stock_point_context::{PlanBudget, bound_profile},
    gimp_composite::{CompositionError, WorkingSpace},
    gimp_render::PreparationBudget,
};
use photocraft_cms::{
    Profile,
    xcf_color::{self, Encoding},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
use photocraft_doc::{
    Document, LayerContent, SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::backing::BackingError;
use photocraft_raster::{Interrupt, RasterInputError};
pub const BOX_GEOMETRY_EPOCH: &str = "xcf22-observed-box-geometry-v1";
fn scope() -> E {
    E::Unsupported("observed Box geometry envelope")
}
fn length() -> E {
    E::Backing(BackingError::Length)
}
fn check(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
/// Shared sealed observational descriptor check. Source activity is deliberately
/// exact; the first observational lane has no activation/metadata edit grant.
pub(crate) fn metadata(doc: &Document, b: &mut impl PlanBudget, i: &Interrupt<'_>) -> Result<(Rect, u32), E> {
    check(b, i, 32768)?;
    let [layer] = doc.layers.as_slice() else { return Err(scope()) };
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    if state.items.len() != 2 || state.filter_masks.len() != 1 {
        return Err(scope());
    }
    let [stack] = state.filter_stacks.as_slice() else { return Err(scope()) };
    let [node] = stack.nodes.as_slice() else { return Err(scope()) };
    let item = state.items.iter().find(|v| v.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
    check(b, i, (item.name.len() as u64).checked_add(layer.name.len() as u64).ok_or_else(length)?)?;
    let (s, format, default) = match &layer.content {
        LayerContent::Raster(v) => (item.raster_bounds.ok_or_else(scope)?, v.format(), v.default_bytes()),
        LayerContent::BackedRaster(v) => (v.operative_bounds(), v.format(), v.default_bytes()),
        _ => return Err(scope()),
    };
    if doc.mode != ColorMode::Rgb
        || !matches!(doc.depth, SampleType::F32 | SampleType::F64)
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc
            .source_color_context
            .as_ref()
            .is_none_or(|v| !matches!((doc.depth, v.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750)) || v.original_icc.is_some())
        || state.provenance.format_version != 22
        || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
        || state.items.len() != 2
        || state.filter_masks.len() != 1
        || !state.editable_indexed.is_empty()
        || stack.owner != ItemRef::Layer(layer.id)
        || !layer.visible
        || layer.opacity.to_bits() != 1f32.to_bits()
        || !node.enabled
        || item.name != layer.name
        || item.raster_bounds != Some(s)
        || item.parent.is_some()
        || item.mask.is_some()
    {
        return Err(scope());
    }
    crate::gimp_render::qualify_current_point_optional_mask_metadata(doc, layer, state, stack, s, SampleEncoding::NonlinearProfile, format, default)
        .map_err(E::Color)?;
    let r = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
    let p = layer.import_compositing.as_ref().ok_or_else(scope)?;
    if r.operator != GimpOperator::Union
        || r.blend_space != GimpSpace::LinearRgb
        || r.composite_space != GimpSpace::LinearRgb
        || p.pass_through
        || p.blend_encoding != SampleEncoding::Linear
        || p.composite_encoding != SampleEncoding::Linear
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
    let radius = crate::gimp_neighborhood::box_blur::radius_for_observed_geometry(node, 22).map_err(E::Color)?;
    let mask = state.filter_masks.first().ok_or_else(scope)?;
    let mi = state.items.iter().find(|v| v.item == ItemRef::Mask(mask.id)).ok_or_else(scope)?;
    let m = mi.mask.as_ref().ok_or_else(scope)?;
    if node.mask != Some(ItemRef::Mask(mask.id))
        || mask.owner != node.id
        || mask.bounds != doc.bounds()
        || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
        || !mi.visible
        || mi.composition.is_some()
        || mi.source_flags != 0
        || m.owner != ItemRef::Layer(layer.id)
        || m.bounds != doc.bounds()
        || m.offset != [0, 0]
        || m.outside_f64_bits != 0f64.to_bits()
        || !m.enabled
        || m.shown
        || m.editing
        || m.linked
        || mask.surface.default_bytes().iter().any(|v| *v != 0)
    {
        return Err(scope());
    }
    let mut bytes = mask.surface.default_bytes().len() as u64;
    zero(mask.surface.default_bytes(), doc.depth, b, i)?;
    for (_, tile) in mask.surface.tiles() {
        bytes = bytes.checked_add(tile.bytes().len() as u64).ok_or_else(length)?;
        if bytes > 16 << 20 {
            return Err(scope());
        }
        zero(tile.bytes(), doc.depth, b, i)?;
    }
    check(b, i, 1024)?;
    photocraft_algo::gimp::neighborhood::BoxBlurTilePlan::new(s, TileCoord::containing(s.x0, s.y0), radius, i).map_err(|e| E::Color(e.into()))?;
    Ok((s, radius))
}
fn zero(bytes: &[u8], sample_type: SampleType, b: &mut impl PlanBudget, i: &Interrupt<'_>) -> Result<(), E> {
    if !bytes.len().is_multiple_of(sample_type.bytes()) {
        return Err(scope());
    }
    for part in bytes.chunks(1024) {
        check(b, i, part.len() as u64)?;
        for sample in part.chunks_exact(sample_type.bytes()) {
            if try_read_sample_f64(sample, sample_type, 0).map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))? != 0. {
                return Err(scope());
            }
        }
    }
    Ok(())
}
struct Shared<'a, B> {
    budget: &'a mut B,
    held: u64,
}
impl<B: PreparationBudget> PlanBudget for Shared<'_, B> {
    type Lease = B::Lease;
    fn reserve(&mut self, n: u64) -> Result<Self::Lease, E> {
        let total = self.held.checked_add(n).ok_or_else(length)?;
        if total > 32 << 20 {
            return Err(scope());
        }
        let fee = self.budget.reserve_prepared_bytes(n).map_err(E::Color)?;
        self.held = total;
        Ok(fee)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.budget.consume_preparation_work(n).map_err(E::Color)
    }
}
/// Profile/Space payloads drop before their preparation fee. Source custody is
/// retained separately; no decoded copy or whole filtered Surface is produced.
pub struct LeasedStreamedBoxPipeline<S, L> {
    _working: WorkingSpace,
    _profile: Profile,
    _source: S,
    _lease: L,
}
impl<S, L> LeasedStreamedBoxPipeline<S, L> {
    pub fn new_geometry_budgeted<B: PreparationBudget<Lease = L>>(
        doc: &Document,
        i: &Interrupt<'_>,
        budget: &mut B,
        source: S,
    ) -> Result<Self, CompositionError> {
        Self::prepare(doc, i, budget, source).map_err(|e| match e {
            E::Color(e) => e,
            E::Backing(BackingError::Cancelled) => RasterInputError::Cancelled.into(),
            E::Allocation => RasterInputError::AllocationFailed.into(),
            _ => CompositionError::Unsupported("observed Box source preparation"),
        })
    }
    fn prepare<B: PreparationBudget<Lease = L>>(doc: &Document, i: &Interrupt<'_>, budget: &mut B, source: S) -> Result<Self, E> {
        let mut b = Shared { budget, held: 0 };
        let (_s, _radius) = metadata(doc, &mut b, i)?;
        let [layer] = doc.layers.as_slice() else { return Err(scope()) };
        let LayerContent::Raster(surface) = &layer.content else { return Err(scope()) };
        native(surface.default_bytes(), doc.depth, &mut b, i)?;
        for (_, tile) in surface.tiles() {
            native(tile.bytes(), doc.depth, &mut b, i)?;
        }
        let envelope = crate::gimp_neighborhood::scalar_preparation_bytes(2)
            .map_err(E::Color)?
            .checked_add(2 * 65536)
            .and_then(|v| v.checked_add(std::mem::size_of::<Self>() as u64 + 128))
            .ok_or_else(length)?;
        let fee = b.reserve(envelope)?;
        let (profile, basis) = bound_profile(doc, None, &mut b, i)?;
        let semantic = xcf_color::assigned_effective_planning_work_ceiling(basis, Encoding::NonlinearProfile).map_err(|_| scope())?;
        check(&mut b, i, semantic.checked_mul(3).ok_or_else(length)?)?;
        check(&mut b, i, envelope.checked_mul(64).ok_or_else(length)?)?;
        let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, i).map_err(E::Color)?;
        let out = Self { _working: working, _profile: profile, _lease: fee, _source: source };
        check(&mut b, i, 0)?;
        Ok(out)
    }
}
fn native(bytes: &[u8], sample_type: SampleType, b: &mut impl PlanBudget, i: &Interrupt<'_>) -> Result<(), E> {
    let pixel_bytes = sample_type.bytes().checked_mul(4).ok_or_else(length)?;
    if !bytes.len().is_multiple_of(pixel_bytes) {
        return Err(scope());
    }
    for block in bytes.chunks(1024) {
        check(b, i, block.len() as u64)?;
        for p in block.chunks_exact(pixel_bytes) {
            for channel in 0..4 {
                let v = try_read_sample_f64(p, sample_type, channel).map_err(|e| E::View(photocraft_raster::checked_view::RasterViewError::Sample(e)))?;
                if !v.is_finite() || channel == 3 && !(0.0..=1.0).contains(&v) {
                    return Err(scope());
                }
            }
        }
    }
    Ok(())
}
