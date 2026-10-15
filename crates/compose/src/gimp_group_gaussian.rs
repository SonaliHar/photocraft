//! Narrow independent isolated-group small Auto Gaussian member.
use crate::gimp_composite::{CompositionError, Result, WorkingSpace};
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianLimits, GaussianParams, try_gaussian_finite_clamp, try_gaussian_small_auto};
use photocraft_cms::{Profile, WideWork};
use photocraft_doc::{
    Document, Layer, SampleEncoding,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterStack, FilterValue},
    xcf_state::XcfState,
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
#[derive(Clone, Copy)]
enum PreparationPolicy {
    Legacy,
    SourceOwned,
    CurrentOwned,
}
pub(super) struct GroupGaussian {
    pub extent: Rect,
    pub enabled: bool,
    working: WorkingSpace,
    params: GaussianParams,
    policy: PreparationPolicy,
}
impl GroupGaussian {
    pub fn new(
        doc: &Document,
        layer: &Layer,
        state: &XcfState,
        stack: &FilterStack,
        context: (Rect, &Profile, SampleEncoding),
        interrupt: &Interrupt,
    ) -> Result<Self> {
        Self::prepare(doc, layer, state, stack, context, interrupt, PreparationPolicy::Legacy)
    }
    pub(super) fn new_owned(
        doc: &Document,
        layer: &Layer,
        state: &XcfState,
        stack: &FilterStack,
        context: (Rect, &Profile, SampleEncoding),
        interrupt: &Interrupt<'_>,
    ) -> Result<Self> {
        Self::prepare(doc, layer, state, stack, context, interrupt, PreparationPolicy::SourceOwned)
    }
    /// CURRENT-only parameter preparation after the actual runtime class gate.
    /// Original source/legacy entry points retain the literal observed pair.
    pub(super) fn new_current_owned(
        doc: &Document,
        layer: &Layer,
        state: &XcfState,
        stack: &FilterStack,
        context: (Rect, &Profile, SampleEncoding),
        interrupt: &Interrupt<'_>,
    ) -> Result<Self> {
        Self::prepare(doc, layer, state, stack, context, interrupt, PreparationPolicy::CurrentOwned)
    }
    /// The exact retained values drive both prospective requirements and execution.
    pub(super) fn parameters(&self) -> GaussianParams {
        self.params
    }
    fn prepare(
        doc: &Document,
        layer: &Layer,
        state: &XcfState,
        stack: &FilterStack,
        context: (Rect, &Profile, SampleEncoding),
        interrupt: &Interrupt<'_>,
        policy_kind: PreparationPolicy,
    ) -> Result<Self> {
        let owned = !matches!(policy_kind, PreparationPolicy::Legacy);
        let current = matches!(policy_kind, PreparationPolicy::CurrentOwned);
        let (extent, profile, stored) = context;
        // The owned caller has already validated its genuine class/model (or
        // the mapper's exact source predicate). Do not repeat the unrelated
        // global metadata validator and its temporary indexes without payment.
        if !owned {
            state.validate_with(doc, interrupt).map_err(|error| {
                if error.code == "cancelled" {
                    photocraft_raster::RasterInputError::Cancelled.into()
                } else {
                    CompositionError::Invalid("invalid Gaussian group state")
                }
            })?;
        }
        let policy = state
            .items
            .iter()
            .find(|i| i.item == ItemRef::Layer(layer.id))
            .and_then(|i| i.composition.as_ref())
            .ok_or(CompositionError::Invalid("group Gaussian policy"))?;
        if state.provenance.format_version != 22
            || doc.size != photocraft_doc::Size::new(16, 12)
            || doc.mode != photocraft_doc::ColorMode::Rgb
            || doc.depth != photocraft_doc::SampleType::F32
            || stored != SampleEncoding::NonlinearProfile
            || if owned {
                profile.encoded_bytes() != photocraft_cms::Builtin::Srgb.profile().encoded_bytes()
            } else {
                profile.to_bytes() != photocraft_cms::Builtin::Srgb.profile().to_bytes()
            }
            || doc.source_color_context.as_ref().is_none_or(|c| c.original_icc.is_some())
            || policy.source_mode != 28
            || policy.pass_through
            || policy.compatibility_epoch != photocraft_doc::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH
            || !matches!(extent, Rect { x0: 0, y0: -1, x1: 12, y1: 8 } | Rect { x0: -1, y0: 0, x1: 11, y1: 11 })
            || layer.mask.is_none()
            || stack.nodes.len() != 1
        {
            return Err(CompositionError::Unsupported("isolated small Gaussian group source domain"));
        }
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("group Gaussian node"))?;
        let c = node.composition.as_ref().ok_or(CompositionError::Invalid("group Gaussian envelope"))?;
        let stored_spaces = [c.stored_blend_space, c.stored_composite_space, c.stored_operator];
        let qualified_spaces = stored_spaces == [-643996976; 3] || (!node.enabled && stored_spaces == [-1474592175; 3]);
        if node.operation.name != "gegl:gaussian-blur"
            || node.operation.stored_version != "0:0"
            || node.operation.schema_epoch != "xcf22-isolated-small-auto-v1"
            || node.opacity_f64_bits != 1.0f64.to_bits()
            || node.source_clipping != Some(0)
            || node.source_region != Some(if extent.y0 == -1 { 0 } else { 1 })
            || !node.auxiliary.is_empty()
            || c.source_mode != 62
            || c.legacy
            || c.pass_through
            || c.resolved.is_some()
            || !qualified_spaces
            || node.arguments.len() != 5
        {
            return Err(CompositionError::Unsupported("isolated small Gaussian effect envelope"));
        }
        let axis = |name, original_bits| -> Result<f64> {
            let mut arguments = node.arguments.iter().filter(|arg| arg.name == name);
            let value = &arguments.next().ok_or(CompositionError::Unsupported("Gaussian axis absent"))?.value;
            if arguments.next().is_some() {
                return Err(CompositionError::Unsupported("duplicate Gaussian axis"));
            }
            let FilterValue::Number(ExactNumber::Float32Bits(bits)) = value else {
                return Err(CompositionError::Unsupported("Gaussian axis stored type"));
            };
            let value = f32::from_bits(*bits);
            if if current { !value.is_finite() || !(0.0..=15.0).contains(&value) } else { *bits != original_bits } {
                return Err(CompositionError::Unsupported("Gaussian axis parameter domain"));
            }
            Ok(f64::from(value))
        };
        let params = GaussianParams {
            std_dev_x: axis("std-dev-x", 1061997773)?,
            std_dev_y: axis("std-dev-y", 1067869798)?,
            abyss: GaussianAbyss::Clamp,
            clip_extent: true,
        };
        for (name, expected) in [
            ("filter", FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 0 }),
            ("abyss-policy", FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 }),
            ("clip-extent", FilterValue::Bool(true)),
        ] {
            let mut args = node.arguments.iter().filter(|a| a.name == name);
            if args.next().is_none_or(|a| a.value != expected) || args.next().is_some() {
                return Err(CompositionError::Unsupported("isolated small Gaussian argument"));
            }
        }
        let Some(ItemRef::Mask(id)) = node.mask else { return Err(CompositionError::Unsupported("group Gaussian selection mask")) };
        let mask = state.filter_masks.iter().find(|m| m.id == id).ok_or(CompositionError::Invalid("group Gaussian mask"))?;
        let metadata = state
            .items
            .iter()
            .find(|i| i.item == ItemRef::Mask(id))
            .and_then(|i| i.mask.as_ref())
            .ok_or(CompositionError::Invalid("group Gaussian mask state"))?;
        if mask.owner != node.id
            || mask.bounds != doc.bounds()
            || mask.surface.format() != photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::F32, false)
            || metadata.owner != ItemRef::Layer(layer.id)
            || metadata.bounds != mask.bounds
            || metadata.offset != [0, 0]
            || !metadata.enabled
            || metadata.shown
            || metadata.editing
            || metadata.linked
            || metadata.outside_f64_bits != 0.0f64.to_bits()
            || mask.surface.resident_payload_bytes()? > 262144 + 4
            || !mask.is_empty_selection_with(interrupt).map_err(|_| {
                if owned && interrupt.cancelled() {
                    photocraft_raster::RasterInputError::Cancelled.into()
                } else {
                    CompositionError::Invalid("group Gaussian mask validation")
                }
            })?
        {
            return Err(CompositionError::Unsupported("group Gaussian empty selection mask domain"));
        }
        let working = if owned {
            WorkingSpace::new_with_interrupt(profile, stored, photocraft_doc::import_compositing::GimpSpace::LinearRgb, interrupt)?
        } else {
            WorkingSpace::new(profile, stored, photocraft_doc::import_compositing::GimpSpace::LinearRgb)?
        };
        Ok(Self { extent, enabled: node.enabled, working, params, policy: policy_kind })
    }
    pub fn linear(&self, p: [f64; 4]) -> Result<[f64; 4]> {
        if p.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(CompositionError::Unsupported("Gaussian group finite unit source domain"));
        }
        let rgb = self.working.convert([p[0], p[1], p[2]], true)?;
        Ok([rgb[0], rgb[1], rgb[2], p[3]])
    }
    pub fn evaluate(&self, input: &Surface, poll: &mut impl FnMut(WideWork) -> Result<()>, interrupt: &Interrupt<'_>) -> Result<Surface> {
        self.evaluate_inner(input, poll, interrupt, None)
    }
    pub(super) fn evaluate_owned(
        &self,
        input: &Surface,
        poll: &mut impl FnMut(WideWork) -> Result<()>,
        interrupt: &Interrupt<'_>,
        limits: GaussianLimits,
    ) -> Result<Surface> {
        self.evaluate_inner(input, poll, interrupt, Some(limits))
    }
    fn evaluate_inner(
        &self,
        input: &Surface,
        poll: &mut impl FnMut(WideWork) -> Result<()>,
        interrupt: &Interrupt<'_>,
        owned_limits: Option<GaussianLimits>,
    ) -> Result<Surface> {
        let units = u64::from(self.extent.width()) * u64::from(self.extent.height()) * 4 * 128;
        poll(WideWork::SampleUnits(units))?;
        let limits = owned_limits.unwrap_or(GaussianLimits { resident_bytes: 64 * 1024 * 1024, work_units: units });
        let filtered = match self.policy {
            PreparationPolicy::CurrentOwned => try_gaussian_finite_clamp(input, self.extent, self.extent, self.params, GaussianFilter::Auto, limits, interrupt),
            PreparationPolicy::Legacy | PreparationPolicy::SourceOwned => {
                try_gaussian_small_auto(input, self.extent, self.extent, self.params, limits, interrupt)
            }
        }?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact((self.extent.width() * self.extent.height() * 4) as usize)
            .map_err(|_| CompositionError::Invalid("group Gaussian output allocation"))?;
        if owned_limits.is_some() && samples.capacity() > (self.extent.width() * self.extent.height() * 8) as usize {
            return Err(CompositionError::Invalid("group Gaussian output capacity"));
        }
        for y in self.extent.y0..self.extent.y1 {
            poll(WideWork::SampleUnits(u64::from(self.extent.width()) * 128))?;
            for x in self.extent.x0..self.extent.x1 {
                if interrupt.cancelled() {
                    return Err(photocraft_raster::RasterInputError::Cancelled.into());
                }
                let mut p = [0.; 4];
                filtered.try_read_pixel_f64(x, y, &mut p)?;
                let rgb = self.working.convert([p[0], p[1], p[2]], false)?;
                samples.extend_from_slice(&[rgb[0], rgb[1], rgb[2], p[3]]);
            }
        }
        let mut output = Surface::new(photocraft_doc::PixelFormat::RGBA64F);
        output.try_write_region_f64_with(self.extent, &samples, interrupt)?;
        Ok(output)
    }
}
