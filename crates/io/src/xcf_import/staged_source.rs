//! Private type-selected staged source paths; never a default loader retry.
//! Rc/map publication uses bounded prepaid standard allocations, not a complete
//! fallible allocator or process RSS guarantee. No consumed storage DTO escapes.
use super::*;
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_xcf::{BudgetKind, PlaneDescriptor, PlaneRole, PlaneSink, PlaneSinkControl, Reservation};
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

// Private type-selected factories only: file data and callers cannot opt an
// already decoded plain plane into a different admission policy.
#[path = "staged_residual_point.rs"]
mod residual;
use residual::ResidualPointFamily;
#[path = "staged_configured_point.rs"]
mod configured;
use configured::ConfiguredPointFamily;
#[path = "staged_point_stack.rs"]
mod point_stack;
#[path = "staged_legacy_scalar.rs"]
mod legacy_scalar;
use legacy_scalar::LegacyScalarFamily;
#[path = "staged_neighborhood.rs"]
mod neighborhood;
use neighborhood::NeighborhoodSourceFamily;
#[path = "staged_analytic_geometry.rs"]
mod analytic_geometry;
#[path = "staged_gaussian_group.rs"]
mod gaussian_group;
#[path = "staged_separable_blend.rs"]
mod separable_blend;

trait StagePolicy {
    const THRESHOLD_ALPHA: bool;
    const SOURCE_SELECTED: bool = false;
    fn accepts(d: &PlaneDescriptor) -> bool;
    fn family_controlled<Q:ExactParsedPlane>(file:&XcfFile<Q>,_:&Interrupt<'_>,_:Option<&photocraft_xcf::Budget<'_>>)->Result<SelectedStagedFamily,IoError>{Ok(Self::family(file))}
    fn family<Q: ExactParsedPlane>(_: &XcfFile<Q>) -> SelectedStagedFamily {
        if Self::THRESHOLD_ALPHA { SelectedStagedFamily::ThresholdAlpha } else { SelectedStagedFamily::Plain }
    }
}
/// Internal successful source-route identity, never persisted or caller-set.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectedStagedFamily {
    Plain,
    SeparableBlend,
    AnalyticGeometry,
    IsolatedNormalGroup,
    IsolatedGaussianGroup,
    ThresholdAlpha,
    Threshold,
    Point(crate::xcf_filters::PointReplaceFamily),
    ResidualPoint(ResidualPointFamily),
    ConfiguredPoint(ConfiguredPointFamily),
    PointStackEdits,
    LegacyScalar(LegacyScalarFamily),
    Neighborhood(NeighborhoodSourceFamily),
}
impl SelectedStagedFamily {
    pub(super) fn assigned_point_storage(self)->bool { matches!(self,Self::Neighborhood(NeighborhoodSourceFamily::BoxGeometryAssignedF32 | NeighborhoodSourceFamily::BoxAssignedF32 | NeighborhoodSourceFamily::MedianAssignedF32 | NeighborhoodSourceFamily::GaussianAssignedF32)) }
}
impl std::fmt::Debug for SelectedStagedFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Plain => "Plain",
            Self::SeparableBlend => "SeparableBlend",
            Self::AnalyticGeometry => "AnalyticGeometry",
            Self::IsolatedNormalGroup => "IsolatedNormalGroup",
            Self::IsolatedGaussianGroup => "IsolatedGaussianGroup",
            Self::ThresholdAlpha => "ThresholdAlpha",
            Self::Threshold => "Threshold",
            Self::Point(_) => "Point",
            Self::ResidualPoint(_) => "ResidualPoint",
            Self::ConfiguredPoint(_) => "ConfiguredPoint",
            Self::PointStackEdits => "PointStackEdits",
            Self::LegacyScalar(_) => "LegacyScalar",
            Self::Neighborhood(_) => "Neighborhood",
        })
    }
}
struct PlainPolicy;
struct ThresholdAlphaPolicy;
struct SourceSelectedPolicy;
impl StagePolicy for SourceSelectedPolicy {
    const THRESHOLD_ALPHA: bool = false;
    const SOURCE_SELECTED: bool = true;
    fn accepts(d: &PlaneDescriptor) -> bool {
        PlainPolicy::accepts(d) || ThresholdAlphaPolicy::accepts(d) || point_plane(d)
    }
    fn family_controlled<Q:ExactParsedPlane>(file:&XcfFile<Q>,ctl:&Interrupt<'_>,shared:Option<&photocraft_xcf::Budget<'_>>)->Result<SelectedStagedFamily,IoError>{
        let old=Self::family(file);
        if old!=SelectedStagedFamily::Plain{return Ok(old)}
        let budget=shared.ok_or_else(||IoError::Unsupported("selected source requires accounting".into()))?;
        if separable_blend::general_hint(file,ctl,budget)?{Ok(SelectedStagedFamily::SeparableBlend)}else{Ok(old)}
    }
    fn family<Q: ExactParsedPlane>(file: &XcfFile<Q>) -> SelectedStagedFamily {
        // Select exactly one old gate, before mapping or mutation. A refusal
        // never retries the other gate. This is not eligibility by name/epoch.
        if separable_blend::hint(file) { return SelectedStagedFamily::SeparableBlend; }
        if gaussian_group::hint(file) { return SelectedStagedFamily::IsolatedGaussianGroup; }
        if staged_accounted_group::hint(file) { return SelectedStagedFamily::IsolatedNormalGroup; }
        if analytic_geometry::hint(file) { return SelectedStagedFamily::AnalyticGeometry; }
        if file.layers.len() == 1 && file.layers.first().is_some_and(|layer| !layer.effects.is_empty()) {
            // This chooses a strict validator only. admissions() must obtain
            // the SAME family from the original whole-file classifier before
            // mapping; completion cannot return this route after a refusal.
            if let Some(family) = NeighborhoodSourceFamily::hint(file) {
                return SelectedStagedFamily::Neighborhood(family);
            }
            if point_stack::hint(file) {
                return SelectedStagedFamily::PointStackEdits;
            }
            if let Some(family) = LegacyScalarFamily::hint(file) {
                return SelectedStagedFamily::LegacyScalar(family);
            }
            if let Some(family) = ConfiguredPointFamily::hint(file) {
                return SelectedStagedFamily::ConfiguredPoint(family);
            }
            if let Some(family) = ResidualPointFamily::hint(file) {
                return SelectedStagedFamily::ResidualPoint(family);
            }
            let point = crate::xcf_point_replace_source::classify_family_hint(file);
            if let Some(family) = point {
                return SelectedStagedFamily::Point(family);
            }
            if file.layers.first().and_then(|layer| layer.effects.first()).is_some_and(|effect| effect.operation == "gimp:threshold") {
                SelectedStagedFamily::Threshold
            } else {
                SelectedStagedFamily::ThresholdAlpha
            }
        } else {
            SelectedStagedFamily::Plain
        }
    }
}
impl StagePolicy for PlainPolicy {
    const THRESHOLD_ALPHA: bool = false;
    fn accepts(d: &PlaneDescriptor) -> bool {
        d.role == PlaneRole::Raster
            && d.sample == decoded_sample(d.source_sample)
            && d.alpha
            && matches!(d.model, Model::Rgb | Model::Gray)
            && d.channels == if d.model == Model::Gray { 2 } else { 4 }
    }
}
impl StagePolicy for ThresholdAlphaPolicy {
    const THRESHOLD_ALPHA: bool = true;
    fn accepts(d: &PlaneDescriptor) -> bool {
        d.sample == decoded_sample(d.source_sample)
            && matches!(d.source_sample, Sample::U16 | Sample::F32 | Sample::F64)
            && match d.role {
                PlaneRole::Raster => d.alpha && matches!(d.model, Model::Rgb | Model::Gray) && d.channels == if d.model == Model::Gray { 2 } else { 4 },
                PlaneRole::Mask => !d.alpha && d.model == Model::Gray && d.channels == 1,
                PlaneRole::Channel => false,
            }
    }
}

// Source-selected sink storage acceptance is broader than source authority.
// Only admissions() can enroll the supported Point intersection after decode.
fn point_plane(d: &PlaneDescriptor) -> bool {
    d.sample == decoded_sample(d.source_sample)
        && matches!(d.source_sample, Sample::U8 | Sample::U16 | Sample::U32 | Sample::F16 | Sample::F32 | Sample::F64)
        && match d.role {
            PlaneRole::Raster => d.alpha && matches!(d.model, Model::Rgb | Model::Gray) && d.channels == if d.model == Model::Gray { 2 } else { 4 },
            PlaneRole::Mask => !d.alpha && d.model == Model::Gray && d.channels == 1,
            PlaneRole::Channel => false,
        }
}

type Token<'b, 'a> = Rc<LeaseToken<'b, 'a>>;
struct LeaseToken<'b, 'a> {
    _surface: Reservation<'b, 'a>,
    _metadata: Reservation<'b, 'a>,
    budget: &'b photocraft_xcf::Budget<'a>,
}
enum Storage {
    Ready(Surface),
    Transferred,
}
struct StagedPlane<'b, 'a, P: StagePolicy = PlainPolicy> {
    descriptor: PlaneDescriptor,
    // Actual raster must die before its last reservation token.
    storage: Storage,
    _token: Token<'b, 'a>,
    _policy: PhantomData<P>,
}
/// Parser widens finite Half exactly; all other native widths are unchanged.
fn decoded_sample(source: Sample) -> Sample {
    if source == Sample::F16 { Sample::F32 } else { source }
}
fn pixel_bytes(d: &PlaneDescriptor) -> Result<usize, XcfError> {
    d.channels.checked_mul(d.sample.source_bytes()).filter(|n| (1..=32).contains(n)).ok_or_else(|| stage_error(ErrorKind::Structure, "staged pixel width"))
}
fn stage_error(kind: ErrorKind, message: impl Into<String>) -> XcfError {
    XcfError { kind, offset: 0, item: String::new(), message: message.into() }
}
fn raster_error(error: photocraft_raster::RasterInputError) -> XcfError {
    stage_error(
        match error {
            photocraft_raster::RasterInputError::Cancelled => ErrorKind::Cancelled,
            photocraft_raster::RasterInputError::LimitExceeded | photocraft_raster::RasterInputError::AllocationFailed => ErrorKind::Limit,
            photocraft_raster::RasterInputError::UnsupportedConversion => ErrorKind::Unsupported,
            _ => ErrorKind::Structure,
        },
        format!("staged XCF raster: {error}"),
    )
}
struct BuilderBudget<'b, 'a> {
    budget: &'b photocraft_xcf::Budget<'a>,
    failure: Option<XcfError>,
}
impl<'b, 'a> StagingBudget for BuilderBudget<'b, 'a> {
    type Lease = Reservation<'b, 'a>;
    fn reserve(&mut self, bytes: u64) -> Result<Self::Lease, photocraft_raster::RasterInputError> {
        self.budget.reserve(BudgetKind::Resident, bytes, "staged XCF surface").map_err(|e| {
            let error = if e.kind == ErrorKind::Cancelled {
                photocraft_raster::RasterInputError::Cancelled
            } else {
                photocraft_raster::RasterInputError::LimitExceeded
            };
            self.failure = Some(e);
            error
        })
    }
    fn work(&mut self, units: u64) -> Result<(), photocraft_raster::RasterInputError> {
        self.budget.consume_work(units, "staged XCF raster initialization/copy").map_err(|e| {
            let error = if e.kind == ErrorKind::Cancelled {
                photocraft_raster::RasterInputError::Cancelled
            } else {
                photocraft_raster::RasterInputError::LimitExceeded
            };
            self.failure = Some(e);
            error
        })
    }
}
struct Sink<'b, 'a, 'c, 'i, P: StagePolicy = PlainPolicy> {
    // All completed storage returned to parser drops before this final registry.
    active: Option<(PlaneDescriptor, StagedSurfaceBuilder<Reservation<'b, 'a>>)>,
    tokens: Vec<Token<'b, 'a>>,
    registry_lease: Option<Reservation<'b, 'a>>,
    budget: &'b photocraft_xcf::Budget<'a>,
    interrupt: &'c Interrupt<'i>,
    _policy: PhantomData<P>,
}
impl<'b, 'a, P: StagePolicy> PlaneSink for Sink<'b, 'a, '_, '_, P> {
    type Plane = StagedPlane<'b, 'a, P>;
    fn begin(&mut self, d: &PlaneDescriptor, control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        control.poll()?;
        if self.active.is_some() || !P::accepts(d) {
            return Err(stage_error(
                ErrorKind::Unsupported,
                if P::THRESHOLD_ALPHA || P::SOURCE_SELECTED {
                    "staged Threshold Alpha endpoint does not admit this native plane role/format"
                } else {
                    "staged endpoint requires alpha-bearing RGB/Gray native plain raster"
                },
            ));
        }
        let width = i32::try_from(d.width).map_err(|_| stage_error(ErrorKind::Limit, "staged width"))?;
        let height = i32::try_from(d.height).map_err(|_| stage_error(ErrorKind::Limit, "staged height"))?;
        let right = d.offset.0.checked_add(width).ok_or_else(|| stage_error(ErrorKind::Limit, "staged horizontal extent"))?;
        let bottom = d.offset.1.checked_add(height).ok_or_else(|| stage_error(ErrorKind::Limit, "staged vertical extent"))?;
        if width <= 0 || height <= 0 || [d.offset.0, d.offset.1, right, bottom].iter().any(|n| !(-(1 << 29)..=(1 << 29)).contains(n)) {
            return Err(stage_error(ErrorKind::Limit, "staged extent"));
        }
        let format = PixelFormat::new(if d.model == Model::Gray { ColorMode::Grayscale } else { ColorMode::Rgb }, depth(d.sample), d.alpha);
        let mut adapter = BuilderBudget { budget: self.budget, failure: None };
        let default = [0u8; 32];
        let builder = StagedSurfaceBuilder::new(
            format,
            default.get(..pixel_bytes(d)?).ok_or_else(|| stage_error(ErrorKind::Structure, "staged default descriptor"))?,
            Rect::new(d.offset.0, d.offset.1, right, bottom),
            &mut adapter,
            self.interrupt,
        )
        .map_err(|e| adapter.failure.take().unwrap_or_else(|| raster_error(e)))?;
        self.active = Some((d.clone(), builder));
        Ok(())
    }
    fn tile(&mut self, x: u32, y: u32, width: u32, height: u32, bytes: &[u8], control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        control.poll()?;
        let (descriptor, builder) = self.active.as_mut().ok_or_else(|| stage_error(ErrorKind::Structure, "staged tile without plane"))?;
        let x = descriptor
            .offset
            .0
            .checked_add(i32::try_from(x).map_err(|_| stage_error(ErrorKind::Limit, "staged cell x"))?)
            .ok_or_else(|| stage_error(ErrorKind::Limit, "staged cell x"))?;
        let y = descriptor
            .offset
            .1
            .checked_add(i32::try_from(y).map_err(|_| stage_error(ErrorKind::Limit, "staged cell y"))?)
            .ok_or_else(|| stage_error(ErrorKind::Limit, "staged cell y"))?;
        let width = i32::try_from(width).map_err(|_| stage_error(ErrorKind::Limit, "staged cell width"))?;
        let height = i32::try_from(height).map_err(|_| stage_error(ErrorKind::Limit, "staged cell height"))?;
        let mut adapter = BuilderBudget { budget: self.budget, failure: None };
        builder
            .push_region(
                Rect::new(
                    x,
                    y,
                    x.checked_add(width).ok_or_else(|| stage_error(ErrorKind::Limit, "staged cell right"))?,
                    y.checked_add(height).ok_or_else(|| stage_error(ErrorKind::Limit, "staged cell bottom"))?,
                ),
                bytes,
                &mut adapter,
                self.interrupt,
            )
            .map_err(|e| adapter.failure.take().unwrap_or_else(|| raster_error(e)))
    }
    fn finish(&mut self, control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<Self::Plane, XcfError> {
        control.poll()?;
        let (descriptor, builder) = self.active.take().ok_or_else(|| stage_error(ErrorKind::Structure, "staged finish without plane"))?;
        let mut adapter = BuilderBudget { budget: self.budget, failure: None };
        let staged = builder.finalize(&mut adapter, self.interrupt).map_err(|e| adapter.failure.take().unwrap_or_else(|| raster_error(e)))?;
        // Prepay bounded Rc bookkeeping before allocation. Standard Rc has no
        // stable fallible constructor, matching the documented raster Arc limit.
        let metadata = self.budget.reserve(BudgetKind::Resident, 128, "staged reservation token")?;
        let count = self.tokens.len().checked_add(1).ok_or_else(|| stage_error(ErrorKind::Limit, "staged lease registry count"))?;
        let requested = count
            .checked_mul(std::mem::size_of::<Token<'b, 'a>>())
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| stage_error(ErrorKind::Limit, "staged lease registry capacity"))?;
        if let Some(lease) = &mut self.registry_lease {
            lease.resize(requested as u64, "staged lease registry allocation")?;
        } else {
            self.registry_lease = Some(self.budget.reserve(BudgetKind::Resident, requested as u64, "staged lease registry allocation")?);
        }
        self.tokens.try_reserve_exact(1).map_err(|_| stage_error(ErrorKind::Limit, "staged lease registry allocation"))?;
        let actual = self
            .tokens
            .capacity()
            .checked_mul(std::mem::size_of::<Token<'b, 'a>>())
            .ok_or_else(|| stage_error(ErrorKind::Limit, "staged lease registry capacity"))?;
        if actual > requested {
            return Err(stage_error(ErrorKind::Limit, "staged lease registry allocator capacity"));
        }
        if let Some(lease) = &mut self.registry_lease {
            lease.resize(actual as u64, "staged lease registry retained capacity")?;
        }
        let (surface, lease) = staged.into_parts();
        let token = Rc::new(LeaseToken { _surface: lease, _metadata: metadata, budget: self.budget });
        self.tokens.push(Rc::clone(&token));
        Ok(StagedPlane { descriptor, storage: Storage::Ready(surface), _token: token, _policy: PhantomData })
    }
    fn abort(&mut self) {
        self.active.take();
    }
}
impl<P: StagePolicy> StagedPlane<'_, '_, P> {
    fn descriptor(&self) -> plane::ParsedPlaneDescriptor {
        plane::ParsedPlaneDescriptor {
            width: self.descriptor.width,
            height: self.descriptor.height,
            channels: self.descriptor.channels,
            sample: self.descriptor.sample,
        }
    }
    fn visit_blocks<const BYTES: usize>(&self, ctl: &Interrupt<'_>, visit: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        let Storage::Ready(surface) = &self.storage else { return Err(IoError::Unsupported("native access after staged Surface transfer".into())) };
        let d = &self.descriptor;
        if d.sample != decoded_sample(d.source_sample) || !(1..=4).contains(&d.channels) {
            return Err(IoError::Unsupported("staged native descriptor".into()));
        }
        let bpp = pixel_bytes(d).map_err(accounting_error)?;
        let _scratch = self._token.budget.reserve(BudgetKind::Resident, BYTES as u64, "staged exact visitor block").map_err(accounting_error)?;
        let mut block = [0u8; BYTES];
        let mut used = 0usize;
        for y in 0..d.height {
            let mut x = 0u32;
            while x < d.width {
                if used == 0 {
                    ctl.check().map_err(|_| IoError::Cancelled)?;
                    let completed = u64::from(y) * u64::from(d.width) + u64::from(x);
                    let remaining = u64::from(d.width) * u64::from(d.height) - completed;
                    self._token.budget.consume_work(remaining.min(64) * bpp as u64, "staged exact native visitor").map_err(accounting_error)?;
                }
                // Fill a single native block across row boundaries without
                // changing callback order or its prepaid 64-pixel work charge.
                let count = (d.width - x).min(u32::try_from(64 - used).map_err(|_| IoError::Unsupported("staged native count".into()))?);
                let next = used.checked_add(count as usize).ok_or_else(|| IoError::Unsupported("staged native count".into()))?;
                let at = used.checked_mul(bpp).ok_or_else(|| IoError::Unsupported("staged native block".into()))?;
                let end = next.checked_mul(bpp).ok_or_else(|| IoError::Unsupported("staged native block".into()))?;
                let output = block.get_mut(at..end).ok_or_else(|| IoError::Unsupported("staged native block".into()))?;
                let left = d
                    .offset
                    .0
                    .checked_add(i32::try_from(x).map_err(|_| IoError::Unsupported("staged native x".into()))?)
                    .ok_or_else(|| IoError::Unsupported("staged native x".into()))?;
                let top = d
                    .offset
                    .1
                    .checked_add(i32::try_from(y).map_err(|_| IoError::Unsupported("staged native y".into()))?)
                    .ok_or_else(|| IoError::Unsupported("staged native y".into()))?;
                let right = left
                    .checked_add(i32::try_from(count).map_err(|_| IoError::Unsupported("staged native width".into()))?)
                    .ok_or_else(|| IoError::Unsupported("staged native right".into()))?;
                let bottom = top.checked_add(1).ok_or_else(|| IoError::Unsupported("staged native bottom".into()))?;
                surface.try_read_interleaved_into_with(Rect::new(left, top, right, bottom), output, ctl).map_err(|e| {
                    if e == photocraft_raster::RasterInputError::Cancelled {
                        IoError::Cancelled
                    } else {
                        IoError::Unsupported(format!("staged native read: {e}"))
                    }
                })?;
                used = next;
                x = x.checked_add(count).ok_or_else(|| IoError::Unsupported("staged native next x".into()))?;
                if used == 64 {
                    visit(
                        block
                            .get(..used.checked_mul(bpp).ok_or_else(|| IoError::Unsupported("staged visitor length".into()))?)
                            .ok_or_else(|| IoError::Unsupported("staged visitor length".into()))?,
                    )?;
                    used = 0;
                }
            }
        }
        if used > 0 {
            visit(
                block
                    .get(..used.checked_mul(bpp).ok_or_else(|| IoError::Unsupported("staged visitor length".into()))?)
                    .ok_or_else(|| IoError::Unsupported("staged visitor length".into()))?,
            )?;
        }
        Ok(())
    }
}
impl<P: StagePolicy> ExactParsedPlane for StagedPlane<'_, '_, P> {
    fn descriptor(&self) -> plane::ParsedPlaneDescriptor {
        StagedPlane::descriptor(self)
    }
    fn visit_native(&self, ctl: &Interrupt<'_>, visit: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        match self.descriptor.sample {
            Sample::U8 => self.visit_blocks::<256>(ctl, visit),
            Sample::U16 => self.visit_blocks::<512>(ctl, visit),
            Sample::U32 | Sample::F32 => self.visit_blocks::<1024>(ctl, visit),
            Sample::F64 => self.visit_blocks::<2048>(ctl, visit),
            Sample::F16 => Err(IoError::Unsupported("staged Half must be widened exactly by the decoder".into())),
        }
    }
}
impl<P: StagePolicy> MappingPlane for StagedPlane<'_, '_, P> {
    fn requires_budgeted_rendering_preparation() -> bool {
        P::THRESHOLD_ALPHA
    }
    fn requires_budgeted_rendering_preparation_for(threshold_alpha_domain: bool) -> bool {
        if P::SOURCE_SELECTED { threshold_alpha_domain } else { Self::requires_budgeted_rendering_preparation() }
    }
    fn requires_budgeted_point_preparation(point_domain: bool) -> bool {
        P::SOURCE_SELECTED && point_domain
    }
    fn requires_budgeted_observed_box_geometry_preparation(domain:bool)->bool{P::SOURCE_SELECTED && domain}
    fn requires_budgeted_assigned_box_preparation(domain: bool) -> bool {P::SOURCE_SELECTED && domain}
    fn requires_budgeted_assigned_median_preparation(domain: bool) -> bool {P::SOURCE_SELECTED && domain}
    fn requires_budgeted_assigned_gaussian_preparation(domain: bool) -> bool {P::SOURCE_SELECTED && domain}
    fn requires_budgeted_neighborhood_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn requires_budgeted_gaussian_group_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn requires_budgeted_lab_separable_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn requires_budgeted_alpha_separable_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn requires_budgeted_component_separable_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn requires_budgeted_legacy_separable_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn requires_budgeted_separable_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn requires_budgeted_general_separable_preparation(domain: bool) -> bool { P::SOURCE_SELECTED && domain }
    fn effective_profile(file: &XcfFile<Self>, space: photocraft_cms::ColorSpace, ce: photocraft_cms::xcf_color::Encoding, ctl: &Interrupt<'_>, shared: Option<&photocraft_xcf::Budget<'_>>) -> Result<photocraft_cms::Profile, IoError> {
        if matches!(P::family(file), SelectedStagedFamily::AnalyticGeometry | SelectedStagedFamily::IsolatedNormalGroup | SelectedStagedFamily::Neighborhood(NeighborhoodSourceFamily::BoxGeometryAssignedF32 | NeighborhoodSourceFamily::BoxAssignedF32 | NeighborhoodSourceFamily::MedianAssignedF32 | NeighborhoodSourceFamily::GaussianAssignedF32)) {
            analytic_geometry::effective(file,space,ce,ctl,shared.ok_or_else(||IoError::Unsupported("geometry profile needs accounting".into()))?)
        }else{
            photocraft_cms::xcf_color::effective_profile(if file.icc.is_empty(){None}else{Some(&file.icc)},space,ce).map_err(|e|IoError::Unsupported(format!("XCF source profile/encoding: {e}")))
        }
    }
    fn admissions(file: &XcfFile<Self>, ctl: &Interrupt<'_>, shared: Option<&photocraft_xcf::Budget<'_>>) -> Result<admission::AdmissionDomains, IoError> {
        let family = P::family_controlled(file,ctl,shared)?;
        let separable = family == SelectedStagedFamily::SeparableBlend;
        let separable_domain = if separable {
            Some(separable_blend::qualify_selected(file,ctl,shared.ok_or_else(||IoError::Unsupported("separable source requires accounting".into()))?)?
                .ok_or_else(||IoError::Unsupported("separable source outside exact original predicate".into()))?)
        } else { None };
        let gaussian_group = family == SelectedStagedFamily::IsolatedGaussianGroup;
        if gaussian_group && !gaussian_group::qualify(file, ctl, shared.ok_or_else(||IoError::Unsupported("Gaussian group requires persistent accounting".into()))?)? {
            return Err(IoError::Unsupported("staged Gaussian group is outside its exact original source predicate".into()));
        }
        let geometry = family == SelectedStagedFamily::AnalyticGeometry;
        let group = family == SelectedStagedFamily::IsolatedNormalGroup;
        if group && !accounted_group::qualifies_with(file, ctl, Some(shared.ok_or_else(|| IoError::Unsupported("group requires persistent source accounting".into()))?))? {
            return Err(IoError::Unsupported("staged group is outside its exact original source predicate".into()));
        }
        if geometry && !analytic_geometry::qualify(file, ctl, shared.ok_or_else(|| IoError::Unsupported("geometry requires persistent source accounting".into()))?)? {
            return Err(IoError::Unsupported("staged geometry is outside exact original analytic geometry predicate".into()));
        }
        let histogram = family == SelectedStagedFamily::Threshold;
        let threshold = family == SelectedStagedFamily::ThresholdAlpha || histogram;
        let point = match family {
            SelectedStagedFamily::Point(expected) => {
                let actual = crate::xcf_point_replace_source::classify(file, ctl, shared)?;
                if actual != Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(expected)) {
                    return Err(IoError::Unsupported("staged Point source is outside its exact whole-file predicate".into()));
                }
                actual
            }
            SelectedStagedFamily::ResidualPoint(expected) => {
                if !expected.qualify(file, ctl, shared)? {
                    return Err(IoError::Unsupported("staged residual Point source is outside its exact whole-file predicate".into()));
                }
                Some(expected.domain())
            }
            SelectedStagedFamily::ConfiguredPoint(expected) => {
                if !expected.qualify(file, ctl, shared)? {
                    return Err(IoError::Unsupported("staged configured Point source is outside its exact whole-file predicate".into()));
                }
                Some(expected.domain())
            }
            SelectedStagedFamily::PointStackEdits => {
                if !point_stack::qualify(file, ctl, shared)? {
                    return Err(IoError::Unsupported("staged Point stack source is outside its exact whole-file predicate".into()));
                }
                Some(crate::xcf_filters::NeighborhoodDomain::PointStackEdits)
            }
            _ => None,
        };
        let owned_point = if let SelectedStagedFamily::LegacyScalar(family) = family {
            if !family.qualify(file, ctl, shared)? {
                return Err(IoError::Unsupported("staged legacy scalar source is outside its exact whole-file predicate".into()));
            }
            Some(family.domain())
        } else { None };
        let neighborhood = if let SelectedStagedFamily::Neighborhood(expected) = family {
            if !expected.qualify(file, ctl, shared)? {
                return Err(IoError::Unsupported("staged neighborhood source is outside its exact whole-file predicate".into()));
            }
            Some(expected)
        } else { None };
        if separable || gaussian_group || group || geometry || point.is_some() || owned_point.is_some() || neighborhood.is_some() {
            // Full original source, precision, geometry, mask flags and unit
            // domains were just checked by the unchanged classifier policy.
        } else if histogram {
            if file.model != Model::Rgb
                || file.encoding != Encoding::NonlinearProfile
                || (file.width, file.height) != (8, 6)
                || !matches!((file.precision, file.sample), (250, Sample::U16) | (650, Sample::F32) | (750, Sample::F64))
                || file.layers.first().and_then(|layer| layer.effects.first()).is_none_or(|effect| effect.mask.is_none())
                || !precision_point_source::qualify(file, "gimp:threshold", false, false, ctl, shared)?
            {
                return Err(IoError::Unsupported("staged Histogram Threshold source is outside the exact RGB nonlinear 8x6 precision predicate".into()));
            }
        } else if threshold {
            if !precision_point_source::qualify_threshold_geometry(file, ctl, shared)? {
                return Err(IoError::Unsupported("staged Threshold Alpha source is outside the existing precision-point predicate".into()));
            }
        } else if file.encoding != Encoding::NonlinearProfile
            || !matches!(file.model, Model::Rgb | Model::Gray)
            || !file.icc.is_empty()
            || !file.palette.is_empty()
            || !file.channels.is_empty()
            || !file.paths.is_empty()
            || file.layers.is_empty()
        {
            return Err(IoError::Unsupported("staged endpoint only supports implicit nonlinear plain RGB/Gray layers".into()));
        }
        for d in &file.layers {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if !threshold
                && !geometry
                && !group
                && !gaussian_group
                && !separable
                && point.is_none()
                && owned_point.is_none()
                && neighborhood.is_none()
                && (d.group
                    || d.path.len() > 1
                    || d.model != file.model
                    || !d.alpha
                    || !matches!(d.mode, 0 | 28)
                    || d.mask.is_some()
                    || !d.effects.is_empty()
                    || d.text.is_some()
                    || d.vector.is_some()
                    || d.link.is_some()
                    || d.transform.is_some()
                    || d.floating_attachment.is_some())
            {
                return Err(unsupported(d, "staged endpoint unsupported owner family"));
            }
            // The parser deliberately skips the non-authoritative group projection.
            // The exact shared predicate above requires it absent; only leaves
            // and the optional physical mask retain real staged storage.
            if (group || gaussian_group) && d.group { continue; }
            let p = d.plane.as_ref().ok_or_else(|| unsupported(d, "staged owner missing pixels"))?;
            if p.descriptor.width != d.width
                || p.descriptor.height != d.height
                || p.descriptor.offset != d.offset
                || p.descriptor.model != d.model
                || p.descriptor.source_sample != file.sample
                || p.descriptor.sample != decoded_sample(file.sample)
                || p.descriptor.channels != if d.model == Model::Gray { 2 } else { 4 }
            {
                return Err(unsupported(d, "staged owner descriptor mismatch"));
            }
            p.visit_native(ctl, &mut |bytes| {
                if let Some(shared) = shared {
                    shared.consume_work(bytes.len() as u64, "staged source native validation").map_err(accounting_error)?;
                }
                Ok(())
            })?;
            validate_drawable_alpha(d, point.is_some(), false, ctl, 0)?;
        }
        let mut domains = admission::AdmissionDomains {
            lab_separable_domain: separable_domain == Some(separable_blend::SourceKind::LabStrict),
            alpha_separable_domain: separable_domain == Some(separable_blend::SourceKind::AlphaStrict),
            component_separable_domain: separable_domain == Some(separable_blend::SourceKind::ComponentStrict),
            legacy_separable_domain: separable_domain == Some(separable_blend::SourceKind::LegacyStrict),
            general_separable_domain: separable_domain == Some(separable_blend::SourceKind::General),
            composition_command_domain: separable_domain == Some(separable_blend::SourceKind::General),
            multiply_spaces_domain: separable_domain == Some(separable_blend::SourceKind::Strict(separable_blend::Domain::MultiplySpaces)),
            gaussian_group_domain: gaussian_group,
            masked_group_domain: gaussian_group,
            accounted_group_domain: group,
            analytic_geometry_domain: geometry,
            normal_mask_domain: geometry,
            analytic_normal_domain: geometry,
            point_replace_domain: point,
            owned_point_domain: owned_point,
            unpremultiply_domain: matches!(family, SelectedStagedFamily::LegacyScalar(LegacyScalarFamily::Unpremultiply)),
            ..Default::default()
        };
        if let Some(family) = neighborhood { family.set_admission(&mut domains); }
        Ok(domains)
    }
    fn late_admissions(file: &XcfFile<Self>) -> admission::LateAdmissionDomains {
        // Generic mapping always runs admissions before this side-effect-free
        // domain handoff. Only the private Threshold factory can select this
        // type, and its admissions above propagate the shared exact predicate.
        let mut domains = admission::LateAdmissionDomains::default();
        if P::family(file) == SelectedStagedFamily::SeparableBlend {
            if separable_blend::legacy_domain(file) || separable_blend::component_domain(file) || separable_blend::alpha_domain(file) || separable_blend::lab_domain(file) { domains.basic_blend_domain = true; }
            match separable_blend::domain(file) {
                Some(separable_blend::Domain::Basic) => domains.basic_blend_domain=true,
                Some(separable_blend::Domain::MultiplyAuto) => domains.multiply_domain=true,
                _ => {},
            }
        }
        if let SelectedStagedFamily::ResidualPoint(family) = P::family(file) {
            family.set_late(&mut domains);
        }
        if let SelectedStagedFamily::ConfiguredPoint(family) = P::family(file) {
            family.set_late(&mut domains);
        }
        if P::family(file) == SelectedStagedFamily::PointStackEdits {
            domains.point_stack_edits_domain = true;
        }
        if let SelectedStagedFamily::LegacyScalar(family) = P::family(file) {
            family.set_late(&mut domains);
        }
        admission::LateAdmissionDomains {
            threshold_alpha_domain: P::family(file) == SelectedStagedFamily::ThresholdAlpha,
            threshold_domain: P::family(file) == SelectedStagedFamily::Threshold,
            ..domains
        }
    }
    fn eager_original(&self) -> Result<&Plane, IoError> {
        Err(IoError::Unsupported("staged storage has no eager Plane".into()))
    }
    fn into_original(self) -> Result<Plane, IoError> {
        Err(IoError::Unsupported("staged storage cannot become a source cache".into()))
    }
    fn retained_source_bytes(&self) -> usize {
        0
    }
    fn current_layer_mask_surface(&mut self, rect: Rect, budget: &mut Budget<'_>, ctl: &Interrupt<'_>) -> Result<Surface, IoError> {
        let d = &self.descriptor;
        if !P::SOURCE_SELECTED || d.role != PlaneRole::Mask || d.offset != (0,0) || !point_plane(d)
            || rect.is_empty() || rect.width()!=d.width || rect.height()!=d.height
            || u64::from(d.width).checked_mul(u64::from(d.height)).is_none_or(|n|n>4096) {
            return Err(IoError::Unsupported("staged ordinary mask placement".into()));
        }
        let format = PixelFormat::new(ColorMode::Grayscale, depth(d.sample), false);
        let length = usize::try_from(self.native_byte_len().ok_or_else(||IoError::Unsupported("mask native length".into()))?).map_err(|_|IoError::Unsupported("mask length".into()))?;
        // Both the old local Surface and this world-positioned Surface stay
        // charged concurrently. Include actual padded tiles, map nodes and
        // ingestion temporary buffers before constructing either destination.
        let tiles=rect.tiles().count();
        let tile_bytes=65536usize.checked_mul(format.bytes_per_pixel()).ok_or_else(||IoError::Unsupported("mask tile bytes".into()))?;
        let envelope=tiles.checked_mul(tile_bytes.checked_mul(3).and_then(|n|n.checked_add(1024)).ok_or_else(||IoError::Unsupported("mask envelope".into()))?)
            .and_then(|n|n.checked_add(4096)).ok_or_else(||IoError::Unsupported("mask envelope".into()))?;
        budget.charge(envelope)?;
        budget.charge(length.checked_mul(2).and_then(|n|n.checked_add(std::mem::size_of::<Vec<u8>>())).ok_or_else(||IoError::Unsupported("mask capture fee".into()))?)?;
        let mut bytes=Vec::new();
        bytes.try_reserve_exact(length).map_err(|_|IoError::Unsupported("mask capture allocation".into()))?;
        if bytes.capacity()>length.checked_mul(2).ok_or_else(||IoError::Unsupported("mask capture capacity".into()))? {return Err(IoError::Unsupported("mask capture capacity".into()));}

        analytic_geometry::pay(self._token.budget,length as u64)?;
        for _ in 0..length.div_ceil(65536) { ctl.check().map_err(|_|IoError::Cancelled)?; let n=length.saturating_sub(bytes.len()).min(65536);bytes.resize(bytes.len()+n,0); }
        let mut cursor=0usize;
        analytic_geometry::pay(self._token.budget,length as u64)?;
        self.visit_native(ctl,&mut |block|{
            let end=cursor.checked_add(block.len()).ok_or_else(||IoError::Unsupported("mask copy overflow".into()))?;
            bytes.get_mut(cursor..end).ok_or_else(||IoError::Unsupported("mask copy bounds".into()))?.copy_from_slice(block);
            cursor=end;Ok(())
        })?;
        if cursor!=length {return Err(IoError::Unsupported("mask capture incomplete".into()));}
        let physical=(tiles as u64).checked_mul(tile_bytes as u64).and_then(|n|n.checked_mul(4)).and_then(|n|n.checked_add(length as u64)).ok_or_else(||IoError::Unsupported("mask ingestion work".into()))?;
        analytic_geometry::pay(self._token.budget,physical)?;
        let result=Surface::try_from_interleaved_with(format,rect,&bytes,ctl).map_err(|e| if e==photocraft_raster::RasterInputError::Cancelled {IoError::Cancelled}else{IoError::Unsupported(format!("mask world ingestion: {e}"))})?;
        if result.tile_count()>tiles || result.resident_accounted_bytes().map_err(|_|IoError::Unsupported("mask capacity census".into()))?>envelope as u64 {return Err(IoError::Unsupported("mask actual capacity".into()));}
        Ok(result)
    }
    fn current_filter_mask_surface(&mut self, rect: Rect, _: &mut Budget<'_>, ctl: &Interrupt<'_>) -> Result<Surface, IoError> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let d = &self.descriptor;
        if !(P::THRESHOLD_ALPHA || P::SOURCE_SELECTED)
            || d.role != PlaneRole::Mask
            || !(ThresholdAlphaPolicy::accepts(d) || P::SOURCE_SELECTED && point_plane(d))
        {
            return Err(IoError::Unsupported("plain staged endpoint has no filter-mask transfer".into()));
        }
        let width = i32::try_from(d.width).map_err(|_| IoError::Unsupported("staged mask width".into()))?;
        let height = i32::try_from(d.height).map_err(|_| IoError::Unsupported("staged mask height".into()))?;
        let expected = Rect::new(
            d.offset.0,
            d.offset.1,
            d.offset.0.checked_add(width).ok_or_else(|| IoError::Unsupported("staged mask right".into()))?,
            d.offset.1.checked_add(height).ok_or_else(|| IoError::Unsupported("staged mask bottom".into()))?,
        );
        let Storage::Ready(surface) = &self.storage else { return Err(IoError::Unsupported("staged mask Surface transferred twice".into())) };
        if rect != expected || surface.format() != PixelFormat::new(ColorMode::Grayscale, depth(d.sample), false) {
            return Err(IoError::Unsupported("staged mask operative native descriptor mismatch".into()));
        }
        // The common mapper next scans the current mask's default and complete
        // resident tile samples, including padding. Pay that physical schedule
        // on the real staging ledger before transfer/scan, not merely its small
        // logical rectangle. Preserve the existing conservative per-sample32
        // allowance; it is a logical work envelope, not CPU-cycle measurement.
        let side = u64::try_from(photocraft_geom::TILE_SIZE).map_err(|_| IoError::Unsupported("staged mask tile side".into()))?;
        let physical = (surface.tile_count() as u64)
            .checked_mul(side)
            .and_then(|n| n.checked_mul(side))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| IoError::Unsupported("staged mask physical scan work".into()))?;
        let logical =
            u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(|| IoError::Unsupported("staged mask logical scan work".into()))?;
        let work = physical.max(logical).checked_mul(32).ok_or_else(|| IoError::Unsupported("staged mask scan work overflow".into()))?;
        let budget = self._token.budget;
        budget
            .check(
                BudgetKind::Work,
                budget.used(BudgetKind::Work).checked_add(work).ok_or_else(|| IoError::Unsupported("staged mask cumulative work".into()))?,
                "staged mask prospective emptiness scan",
            )
            .map_err(accounting_error)?;
        let mut left = work;
        while left != 0 {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let part = left.min(4096);
            budget.consume_work(part, "staged mask default/full-tile emptiness scan").map_err(accounting_error)?;
            left -= part;
        }
        let storage = std::mem::replace(&mut self.storage, Storage::Transferred);
        let Storage::Ready(surface) = storage else { return Err(IoError::Unsupported("staged mask transfer state".into())) };
        // Original Sink registry owns the actual pixel/map reservations after
        // this plane token disappears, through CompletedStagedImport's drop.
        ctl.check().map_err(|_| IoError::Cancelled)?;
        Ok(surface)
    }
    fn current_owner(
        mut owner: Drawable<Self>,
        _: &[[u8; 3]],
        _: &mut Budget<'_>,
        ctl: &Interrupt<'_>,
    ) -> Result<(Drawable<Self>, Layer, Option<IndexedSource>), IoError> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if owner.group {
            if !P::SOURCE_SELECTED || owner.plane.is_some() {
                return Err(IoError::Unsupported("staged group projection is not authoritative".into()));
            }
            let layer = Layer::group(&owner.name, Vec::new());
            return Ok((owner, layer, None));
        }
        let p = owner.plane.as_mut().ok_or_else(|| IoError::Unsupported("staged owner missing pixels".into()))?;
        let storage = std::mem::replace(&mut p.storage, Storage::Transferred);
        let Storage::Ready(surface) = storage else { return Err(IoError::Unsupported("staged Surface transferred twice".into())) };
        let layer = Layer::new(&owner.name, LayerContent::Raster(surface));
        Ok((owner, layer, None))
    }
    fn legacy_point_candidate(owner: &Drawable<Self>, _: bool, _: bool, _: Size) -> Result<bool, IoError> {
        if !(P::THRESHOLD_ALPHA || P::SOURCE_SELECTED) && !owner.effects.is_empty() {
            return Err(unsupported(owner, "staged effect evaluation unsupported"));
        }
        Ok(false)
    }
}

/// Explicit opt-in plain source endpoint. This preserves original native bytes
/// while constructing current COW tiles directly; other families return errors.
/// CallerReserved accounting assertions are trusted contracts, not IO authority.
// Payload fields precede all receipts on both success and error. Only the
// opt-in owned endpoint may consume this private transaction before those actual
// reservations die; the existing resident API intentionally retains its scope.
pub(super) struct CompletedStagedImport<'b, 'a> {
    pub(super) result: ImportResult,
    tokens: Vec<Token<'b, 'a>>,
    registry_lease: Option<Reservation<'b, 'a>>,
    reservations: photocraft_xcf::DecodeReservations<'b, 'a>,
}
pub(super) struct OwnedStagedImport {
    pub(super) result: ImportResult,
    pub(super) encoded: Arc<Vec<u8>>,
    pub(super) source_batch: Option<photocraft_xcf::OwnedReservationBatch>,
    pub(super) surface_batch: photocraft_xcf::OwnedReservationBatch,
}
impl<'b, 'a> CompletedStagedImport<'b, 'a> {
    pub(super) fn detach_owned(
        self,
        encoded: Arc<Vec<u8>>,
        extra: &mut [&mut Reservation<'b, 'a>],
        budget: &'b photocraft_xcf::Budget<'a>,
    ) -> Result<OwnedStagedImport, IoError> {
        // If a later transfer fails, the complete result and encoded owner die
        // BEFORE either the old borrowed or the earlier owned receipts refund.
        struct Transfer<'b, 'a> {
            completed: CompletedStagedImport<'b, 'a>,
            encoded: Arc<Vec<u8>>,
            source_batch: Option<photocraft_xcf::OwnedReservationBatch>,
        }
        let mut transfer = Transfer { completed: self, encoded, source_batch: None };
        if transfer.completed.tokens.len() > 4096 {
            return Err(IoError::Unsupported("owned staged plane count".into()));
        }
        let count = transfer
            .completed
            .tokens
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(usize::from(transfer.completed.registry_lease.is_some())))
            .and_then(|n| n.checked_add(extra.len()))
            .ok_or_else(|| IoError::Unsupported("owned staged receipt count".into()))?;
        let requested = count
            .checked_mul(std::mem::size_of::<&mut Reservation<'b, 'a>>())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(128))
            .ok_or_else(|| IoError::Unsupported("owned staged handle index".into()))?;
        let mut handle_lease = budget.reserve(BudgetKind::Index, requested as u64, "owned staged receipt references").map_err(accounting_error)?;
        let mut references = Vec::new();
        references.try_reserve_exact(count).map_err(|_| accounting_error(stage_error(ErrorKind::Allocation, "owned staged receipt allocation")))?;
        let actual = references
            .capacity()
            .checked_mul(std::mem::size_of::<&mut Reservation<'b, 'a>>())
            .and_then(|n| n.checked_add(128))
            .ok_or_else(|| IoError::Unsupported("owned staged receipt capacity".into()))?;
        if actual > requested {
            return Err(IoError::Unsupported("owned staged receipt allocator capacity".into()));
        }
        handle_lease.resize(actual as u64, "owned staged receipt actual capacity").map_err(accounting_error)?;
        transfer.source_batch = Some(transfer.completed.reservations.detach_owned().map_err(accounting_error)?);
        for token in &mut transfer.completed.tokens {
            budget.consume_work(1, "owned staged receipt traversal").map_err(accounting_error)?;
            let token = Rc::get_mut(token).ok_or_else(|| IoError::Unsupported("owned staged Surface token escaped transaction".into()))?;
            references.push(&mut token._surface);
            references.push(&mut token._metadata);
        }
        if let Some(lease) = &mut transfer.completed.registry_lease {
            references.push(lease);
        }
        for lease in extra {
            references.push(&mut **lease);
        }
        let surface_batch = Reservation::detach_owned_batch(&mut references).map_err(accounting_error)?;
        drop(references);
        drop(handle_lease);
        let Transfer { completed, encoded, source_batch } = transfer;
        let CompletedStagedImport { result, .. } = completed;
        Ok(OwnedStagedImport { result, encoded, source_batch, surface_batch })
    }
}
pub fn import_staged_plain_source_with_budget(
    name: &str,
    source: &mut dyn photocraft_xcf::Source,
    control: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    budget: &photocraft_xcf::Budget<'_>,
    accounting: photocraft_xcf::PlainSourceAccounting,
) -> Result<ImportResult, IoError> {
    Ok(complete_staged_plain_source(name, source, control, options, budget, accounting)?.result)
}

pub(super) fn complete_staged_plain_source<'b, 'a>(
    name: &str,
    source: &mut dyn photocraft_xcf::Source,
    control: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    budget: &'b photocraft_xcf::Budget<'a>,
    accounting: photocraft_xcf::PlainSourceAccounting,
) -> Result<CompletedStagedImport<'b, 'a>, IoError> {
    complete_staged_source::<PlainPolicy>(name, source, control, options, budget, accounting).map(|(completed, _)| completed)
}

pub(super) fn complete_staged_threshold_alpha_source<'b, 'a>(
    name: &str,
    source: &mut dyn photocraft_xcf::Source,
    control: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    budget: &'b photocraft_xcf::Budget<'a>,
    accounting: photocraft_xcf::PlainSourceAccounting,
) -> Result<CompletedStagedImport<'b, 'a>, IoError> {
    complete_staged_source::<ThresholdAlphaPolicy>(name, source, control, options, budget, accounting).map(|(completed, _)| completed)
}

pub(super) fn complete_staged_selected_source<'b, 'a>(
    name: &str,
    source: &mut dyn photocraft_xcf::Source,
    control: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    budget: &'b photocraft_xcf::Budget<'a>,
    accounting: photocraft_xcf::PlainSourceAccounting,
) -> Result<(CompletedStagedImport<'b, 'a>, SelectedStagedFamily), IoError> {
    complete_staged_source::<SourceSelectedPolicy>(name, source, control, options, budget, accounting)
}

fn complete_staged_source<'b, 'a, P: StagePolicy>(
    name: &str,
    source: &mut dyn photocraft_xcf::Source,
    control: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    budget: &'b photocraft_xcf::Budget<'a>,
    accounting: photocraft_xcf::PlainSourceAccounting,
) -> Result<(CompletedStagedImport<'b, 'a>, SelectedStagedFamily), IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    // Sink registry is declared before decoded/mapped storage so errors destroy
    // actual Surfaces before their final token/registry refunds.
    let mut sink = Sink::<P> { active: None, tokens: Vec::new(), registry_lease: None, budget, interrupt: control, _policy: PhantomData };
    let strict = DecodeOptions { limits: options.limits.clone(), allow_finite_drawable_alpha: false, cancelled: options.cancelled, progress: options.progress };
    let photocraft_xcf::BudgetDecodedImport { decoded, reservations } =
        photocraft_xcf::decode_plain_source_into_sink_with_budget(source, &strict, budget, accounting, &mut sink).map_err(accounting_error)?;
    let mut file = decoded.file;
    let family = P::family_controlled(&file,control,Some(budget))?;
    let encoded = usize::try_from(decoded.encoded_bytes).map_err(|_| IoError::Unsupported("staged source address space".into()))?;
    file.charged_bytes = file.charged_bytes.checked_sub(encoded).ok_or_else(|| IoError::Unsupported("staged source accounting mismatch".into()))?;
    let previous = std::cell::Cell::new(file.charged_bytes);
    let leases = RefCell::new(reservations);
    let account = |used: usize| {
        if used > previous.get() {
            budget.consume_work((used - previous.get()) as u64, "staged mapping metadata").map_err(accounting_error)?;
        }
        leases
            .try_borrow_mut()
            .map_err(|_| IoError::Unsupported("staged accounting borrowed".into()))?
            .resize_retained(used as u64, "staged mapping metadata")
            .map_err(accounting_error)?;
        previous.set(used);
        Ok(())
    };
    let maximum = options
        .limits
        .resident_bytes
        .min(usize::try_from(budget.limits().get(BudgetKind::Resident)).map_err(|_| IoError::Unsupported("staged resident address space".into()))?);
    let (mut result, _) = map_accounted(name, file, control, maximum, Some(&account), Some((budget, &leases)))?;
    provenance::stamp(&mut result, source, decoded.encoded_bytes, control, budget, previous.get(), &account)?;
    // Mapping and provenance have finished. The completed transaction owns the
    // actual surface registry and source/mapping receipts until explicit transfer.
    let reservations = leases.into_inner();
    let Sink { active, tokens, registry_lease, .. } = sink;
    if active.is_some() {
        return Err(IoError::Unsupported("staged import left an active plane".into()));
    }
    Ok((CompletedStagedImport { result, tokens, registry_lease, reservations }, family))
}

#[cfg(test)]
#[path = "staged_selected_source_tests.rs"]
mod selected_source_tests;

#[cfg(test)]
#[path = "staged_threshold_alpha_tests.rs"]
mod threshold_alpha_tests;

#[cfg(test)]
mod tests {
    use super::super::plane::fixture;
    use super::*;
    struct Bytes(Vec<u8>);
    impl photocraft_xcf::Source for Bytes {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8], _: &photocraft_xcf::Budget<'_>) -> Result<(), XcfError> {
            let start = offset as usize;
            out.copy_from_slice(&self.0[start..start + out.len()]);
            Ok(())
        }
    }
    #[test]
    fn bulk_visitor_keeps_blocks_work_scratch_and_cancels_inside_short_block() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for width in [3, 17, 65, 129] {
            let bytes: Vec<_> = (0..width * 3).flat_map(|i| [i as u8, 19, 97, 0]).collect();
            let encoded = fixture::image(12, 150, 0, width, 3, &bytes);
            let eager = photocraft_xcf::decode(&encoded, &DecodeOptions::default()).unwrap();
            let budget = photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None);
            let mut source = Bytes(encoded);
            let mut sink = Sink::<PlainPolicy> {
                active: None,
                tokens: Vec::new(),
                registry_lease: None,
                budget: &budget,
                interrupt: &Interrupt::NONE,
                _policy: PhantomData,
            };
            let parsed = photocraft_xcf::decode_plain_source_into_sink_with_budget(
                &mut source,
                &DecodeOptions::default(),
                &budget,
                photocraft_xcf::PlainSourceAccounting::BorrowedMemory,
                &mut sink,
            )
            .unwrap();
            let p = parsed.decoded.file.layers[0].plane.as_ref().unwrap();
            let resident = budget.used(BudgetKind::Resident);
            let before_work = budget.used(BudgetKind::Work);
            let mut expected = Vec::new();
            let mut actual = Vec::new();
            eager.layers[0]
                .plane
                .as_ref()
                .unwrap()
                .visit_native(&Interrupt::NONE, &mut |b| {
                    expected.push(b.to_vec());
                    Ok(())
                })
                .unwrap();
            p.visit_native(&Interrupt::NONE, &mut |b| {
                assert_eq!(budget.used(BudgetKind::Resident), resident + 256);
                actual.push(b.to_vec());
                Ok(())
            })
            .unwrap();
            assert_eq!(actual, expected, "same row-major 64-pixel callbacks at width{width}");
            assert_eq!(budget.used(BudgetKind::Work) - before_work, bytes.len() as u64);
            assert_eq!(budget.used(BudgetKind::Resident), resident);
            if width == 17 {
                let calls = AtomicUsize::new(0);
                let stop = || calls.fetch_add(1, Ordering::Relaxed) >= 3;
                let mut callbacks = 0;
                assert!(matches!(
                    p.visit_native(&Interrupt::cancel_only(&stop), &mut |_| {
                        callbacks += 1;
                        Ok(())
                    }),
                    Err(IoError::Cancelled)
                ));
                assert_eq!(callbacks, 0, "incomplete canceled block is never delivered");
                assert_eq!(budget.used(BudgetKind::Resident), resident);
            }
            drop(parsed);
            drop(sink);
            assert_eq!(budget.used(BudgetKind::Resident), 0);
        }
    }
    #[test]
    fn actual_surface_move_keeps_token_after_owner_drop_and_late_native_reads_fail() {
        let budget = photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::NativeStandard).unwrap(), None);
        let mut source = Bytes(fixture::image(12, 150, 0, 1, 1, &[11, 31, 97, 0]));
        let mut sink =
            Sink::<PlainPolicy> { active: None, tokens: Vec::new(), registry_lease: None, budget: &budget, interrupt: &Interrupt::NONE, _policy: PhantomData };
        let mut parsed = photocraft_xcf::decode_plain_source_into_sink_with_budget(
            &mut source,
            &DecodeOptions::default(),
            &budget,
            photocraft_xcf::PlainSourceAccounting::BorrowedMemory,
            &mut sink,
        )
        .unwrap();
        let owner = parsed.decoded.file.layers.remove(0);
        let plane = owner.plane.as_ref().unwrap();
        let Storage::Ready(before) = &plane.storage else { panic!() };
        let pointer = Arc::as_ptr(before.tiles().next().unwrap().1);
        let used = budget.used(BudgetKind::Resident);
        let mut local = Budget { used: 0, max: usize::MAX, peak: 0, shared: None };
        let (owner, layer, _) = StagedPlane::current_owner(owner, &[], &mut local, &Interrupt::NONE).unwrap();
        let LayerContent::Raster(after) = &layer.content else { panic!() };
        assert_eq!(Arc::as_ptr(after.tiles().next().unwrap().1), pointer, "authoritative tile Arc moved without publication clone");
        let mut delivered = 0;
        let error = owner.plane.as_ref().unwrap().visit_native(&Interrupt::NONE, &mut |_| {
            delivered += 1;
            Ok(())
        });
        assert!(error.is_err());
        assert_eq!(delivered, 0);
        assert_eq!(budget.used(BudgetKind::Resident), used);
        drop(owner);
        assert_eq!(budget.used(BudgetKind::Resident), used, "owner drop cannot refund a Surface now in Layer");
        drop(layer);
        assert_eq!(budget.used(BudgetKind::Resident), used, "endpoint registry retains charge through final hash even after late staging teardown");
        drop(parsed);
        assert!(budget.used(BudgetKind::Resident) >= 256 * 256 * 4);
        drop(sink);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
    }
}

#[cfg(test)]
#[path = "staged_point_source_tests.rs"]
mod point_source_tests;
