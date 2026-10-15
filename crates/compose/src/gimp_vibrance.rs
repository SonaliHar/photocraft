//! Paid owned physical Lab/LCh compatibility recipe. The declared ICC/polar
//! schedule is not exact Babl/RGB rounding or producer projection acceptance.
use crate::{
    backed_normal::BackedNormalError as E,
    backed_stock_point_context::PlanBudget,
    gimp_composite::{CompositionError, Result},
};
use photocraft_algo::gimp::{
    hue_chroma,
    point_vibrance::{self, Parameters},
};
use photocraft_cms::{Builtin, Profile, control::Limits, owned_lab::Parts, owned_linear_srgb_lab::Requirements};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, FilterOutputPrecision, LayerId, SampleEncoding,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError};

pub const EPOCH: &str = "xcf22-gegl-vibrance-linear-lch-float-v1";
// Fixed arithmetic schedules are separate from actual CMS stage facts. Polar
// entry covers finite checks, hypot, atan2, degrees/modulo and four Float casts;
// exit covers finite checks, radians, sin/cos and products. Each transcendental
// receives the same conservative128-unit scalar charge as analytical CMS work.
const POLAR_ENTRY_WORK: u64 = 3 * 128 + 32;
const POLAR_EXIT_WORK: u64 = 2 * 128 + 32;
const LCH_STAGE_WORK: u64 = 64;
const OUTPUT_WORK: u64 = 16;
/// Two simultaneous MAX-wide CMS arrays plus native/Lab/LCh/output temporaries.
pub(crate) const PIXEL_SCRATCH_BYTES: u64 = (2 * photocraft_cms::pipeline::MAX * std::mem::size_of::<f64>() + 12 * std::mem::size_of::<[f64; 4]>()) as u64;
fn scope() -> CompositionError {
    CompositionError::Unsupported("closed paid Vibrance recipe")
}
fn length() -> E {
    E::Invalid("owned Vibrance size or work overflow")
}
fn cms(error: photocraft_cms::control::Error) -> E {
    match error {
        photocraft_cms::control::Error::Cancelled => E::Backing(photocraft_raster::backing::BackingError::Cancelled),
        photocraft_cms::control::Error::Allocation => E::Allocation,
        photocraft_cms::control::Error::Limit(v) => E::Unsupported(v),
        photocraft_cms::control::Error::Cms(v) => E::Color(CompositionError::Color(v)),
    }
}
pub(crate) fn charge(budget: &mut impl PlanBudget, control: &Interrupt<'_>, mut units: u64) -> std::result::Result<(), E> {
    control.check().map_err(|_| E::Backing(photocraft_raster::backing::BackingError::Cancelled))?;
    budget.check(0)?;
    while units != 0 {
        let part = units.min(4096);
        budget.check(part)?;
        units -= part;
        control.check().map_err(|_| E::Backing(photocraft_raster::backing::BackingError::Cancelled))?;
    }
    Ok(())
}
pub(crate) fn claims(node: &FilterNode) -> bool {
    node.operation.name == "gegl:vibrance" || node.operation.schema_epoch == EPOCH
}
pub(crate) fn parameters(arguments: &[FilterArgument]) -> Result<Parameters> {
    if arguments.len() != 2 {
        return Err(scope());
    }
    let mut words = [None; 2];
    for arg in arguments {
        let index = match arg.name.as_str() {
            "vibrance" => 0,
            "saturation" => 1,
            _ => return Err(scope()),
        };
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = &arg.value else { return Err(scope()) };
        let slot = words.get_mut(index).ok_or_else(scope)?;
        if slot.replace(f64::from(f32::from_bits(*bits))).is_some() {
            return Err(scope());
        }
    }
    let p = Parameters { vibrance: words.first().copied().flatten().ok_or_else(scope)?, saturation: words.get(1).copied().flatten().ok_or_else(scope)? };
    if !p.vibrance.is_finite() || !(-100.0..=100.0).contains(&p.vibrance) || !p.saturation.is_finite() || !(0.0..=10.0).contains(&p.saturation) {
        return Err(scope());
    }
    Ok(p)
}
pub(crate) fn canonical(node: &FilterNode, stored: SampleEncoding, version: u32, extent: Rect) -> Result<Parameters> {
    if version != 22
        || node.operation.name != "gegl:vibrance"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
        || stored != SampleEncoding::NonlinearProfile
        || extent != Rect::new(0, 0, 8, 6)
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(ItemRef::Mask(_)))
        || node.composition.as_ref().is_none_or(|p| p.compatibility_epoch != EPOCH || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some())
    {
        return Err(scope());
    }
    // The three typed raw fields remain opaque provenance under this own exact
    // envelope; no enum resolution, alias or universal GIMP validity is inferred.
    parameters(&node.arguments)
}
pub(crate) fn qualify_context(doc: &Document, extent: Rect, node: &FilterNode) -> Result<Parameters> {
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    if doc.mode != ColorMode::Rgb
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc.size != photocraft_doc::Size::new(8, 6)
        || extent != doc.bounds()
        || !matches!((doc.depth, source.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750))
        || source.original_icc.is_some()
        || doc.xcf_state.as_ref().is_none_or(|s| s.current_filter_output_precision != FilterOutputPrecision::Canonical)
    {
        return Err(scope());
    }
    canonical(node, SampleEncoding::NonlinearProfile, 22, extent)
}
/// Allocation-free planner facts only. The stock caller must reserve its whole
/// combined peak before constructing either the current profile or these Parts.
pub(crate) struct Preparation {
    requirements: Requirements,
}
impl Preparation {
    pub(crate) fn plan(profile: &Profile, control: &Interrupt<'_>, budget: &mut impl PlanBudget) -> std::result::Result<Self, E> {
        charge(budget, control, 1)?;
        if profile != Builtin::Srgb.profile() {
            return Err(E::Color(scope()));
        }
        let planning = photocraft_cms::owned_linear_srgb_lab::planning_work_ceiling();
        charge(budget, control, planning)?;
        let requirements = photocraft_cms::owned_linear_srgb_lab::requirements_with_control(Limits::default(), &|| control.cancelled()).map_err(cms)?;
        Ok(Self { requirements })
    }
    pub(crate) fn peak_bytes(&self) -> std::result::Result<u64, E> {
        u64::try_from(self.requirements.peak_resident_bytes).map_err(|_| length())
    }
    pub(crate) fn construct(
        self,
        profile: &Profile,
        parameters: Parameters,
        control: &Interrupt<'_>,
        budget: &mut impl PlanBudget,
    ) -> std::result::Result<OwnedVibrance, E> {
        charge(budget, control, self.requirements.work_units)?;
        let limits = Limits { resident_bytes: self.requirements.peak_resident_bytes, work_units: self.requirements.work_units };
        let parts = photocraft_cms::owned_linear_srgb_lab::construct_with_control(limits, &|| control.cancelled()).map_err(cms)?;
        // One independent NL<->Linear transfer holder, not a borrowed profile
        // alias. Fixed builtin analytical curves have no nested sample tables.
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(profile).map_err(cms)?;
        charge(budget, control, semantic.checked_mul(3).ok_or_else(length)?)?;
        let transfer = crate::gimp_composite::WorkingSpace::new_with_interrupt(
            profile,
            SampleEncoding::NonlinearProfile,
            photocraft_doc::import_compositing::GimpSpace::LinearRgb,
            control,
        )
        .map_err(E::Color)?;
        // Repeated census/work-fact traversals are new work, independently
        // prepaid before calls. Fullconstructor peak remains retained afterward.
        let census = self.peak_bytes()?.checked_add(512).ok_or_else(length)?;
        charge(budget, control, census)?;
        let retained = parts.retained_bytes_with_control(Limits::default(), &|| control.cancelled()).map_err(cms)?;
        if retained > self.requirements.peak_resident_bytes {
            return Err(E::Allocation);
        }
        use photocraft_cms::pipeline::Stage;
        if parts.physical_forward.stages.iter().any(|s| !matches!(s, Stage::Curves(_) | Stage::Matrix { .. } | Stage::XyzToLab))
            || parts.physical_reverse.stages.iter().any(|s| !matches!(s, Stage::InvCurves(_) | Stage::Matrix { .. } | Stage::LabToXyz))
        {
            return Err(E::Color(scope()));
        }
        // Physical D50 sides contain no device/XYZ unit clamp or normalized Lab
        // encoding. The extra normalized Transform pair remains paid overhead.
        let forward = parts.physical_forward.evaluation_work_units_with_control(Limits::default(), &|| control.cancelled()).map_err(cms)?;
        let reverse = parts.physical_reverse.evaluation_work_units_with_control(Limits::default(), &|| control.cancelled()).map_err(cms)?;
        let validation_work = forward.checked_add(POLAR_ENTRY_WORK).and_then(|n| n.checked_add(3 * 128 + 4 + 128 + 16)).ok_or_else(length)?;
        let evaluation_work = validation_work
            .checked_add(LCH_STAGE_WORK)
            .and_then(|n| n.checked_add(POLAR_EXIT_WORK))
            .and_then(|n| n.checked_add(reverse))
            .and_then(|n| n.checked_add(OUTPUT_WORK + 3 * 128 + 4 + 16))
            .ok_or_else(length)?;
        let nested_bytes = retained.checked_sub(std::mem::size_of::<Parts>()).ok_or_else(length)? as u64;
        charge(budget, control, 0)?;
        Ok(OwnedVibrance { parts, transfer, parameters, validation_work, evaluation_work, nested_bytes })
    }
}
/// Every nested pipeline/Transform owner is independent, with no borrowed
/// profile/self-reference or retained callback. Enclosing StockPointPlan owns
/// current profile and source custody and keeps the combined peak fee last.
pub(crate) struct OwnedVibrance {
    parts: Parts,
    transfer: crate::gimp_composite::WorkingSpace,
    parameters: Parameters,
    validation_work: u64,
    evaluation_work: u64,
    nested_bytes: u64,
}
impl OwnedVibrance {
    pub(crate) fn holder_peak_bytes() -> std::result::Result<u64, E> {
        (std::mem::size_of::<(Self, bool)>() as u64).checked_mul(2).ok_or_else(length)
    }
    pub(crate) fn nested_bytes(&self) -> u64 {
        self.nested_bytes
    }
    pub(crate) fn validation_work(&self) -> u64 {
        self.validation_work
    }
    pub(crate) fn evaluation_work(&self, enabled: bool) -> u64 {
        if enabled { self.evaluation_work } else { self.validation_work }
    }
    fn input_lch(&self, pixel: [f64; 4], control: &Interrupt<'_>, work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>) -> Result<[f32; 4]> {
        work(0)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        if pixel.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(scope());
        }
        let linear = self.transfer.convert([pixel[0], pixel[1], pixel[2]], true)?;
        let linear = [linear[0] as f32, linear[1] as f32, linear[2] as f32, pixel[3] as f32];
        let saturated = point_vibrance::saturation_linear_float(linear, self.parameters.saturation)?;
        work(0)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let mut lab = [0.; 3];
        self.parts.physical_forward.try_eval_f64(&[f64::from(saturated[0]), f64::from(saturated[1]), f64::from(saturated[2])], &mut lab)?;
        work(0)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        crate::gimp_hue_chroma_color_candidate::lab_to_lch_float(lab, saturated[3]).map_err(Into::into)
    }
    pub(crate) fn validate_input(
        &self,
        pixel: [f64; 4],
        control: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<()> {
        self.input_lch(pixel, control, work).map(|_| ())
    }
    pub(crate) fn apply(
        &self,
        pixel: [f64; 4],
        enabled: bool,
        control: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        let input = self.input_lch(pixel, control, work)?;
        if !enabled {
            return Ok(pixel);
        }
        let changed = hue_chroma::evaluate_lch_float(input, hue_chroma::Parameters { hue: 0., chroma: self.parameters.vibrance, lightness: 0. })?;
        work(0)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let lab = crate::gimp_hue_chroma_color_candidate::lch_to_lab(changed)?;
        let mut output = [0.; 3];
        self.parts.physical_reverse.try_eval_f64(&lab, &mut output)?;
        work(0)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let linear_float = output.map(|v| f64::from(v as f32));
        let native = self.transfer.convert(linear_float, false)?;
        let rgb = native.map(|v| f64::from(v as f32));
        let result = [rgb[0], rgb[1], rgb[2], f64::from(changed[3])];
        if result.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&result[3]) {
            return Err(scope());
        }
        // Finite signed/HDR RGB is operative, without unit clamp. Even enabled
        // neutral or alpha0/1 traverses both physical directions.
        Ok(result)
    }
}
/// Own Vibrance mask envelope, not an alias to another operation's raw policy.
pub(crate) fn qualify_mask(state: &photocraft_doc::XcfState, node: &FilterNode, owner: LayerId, sample: SampleType, control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    let Some(ItemRef::Mask(id)) = node.mask else { return Err(scope()) };
    let [mask] = state.filter_masks.as_slice() else { return Err(scope()) };
    if state.items.len() != 2
        || mask.id != id
        || mask.owner != node.id
        || mask.bounds != Rect::new(0, 0, 8, 6)
        || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, sample, false)
        || mask.surface.tile_count() > 1
    {
        return Err(scope());
    }
    let item = state.items.iter().find(|i| i.item == ItemRef::Mask(id)).ok_or_else(scope)?;
    let m = item.mask.as_ref().ok_or_else(scope)?;
    if !item.visible
        || m.owner != ItemRef::Layer(owner)
        || m.bounds != mask.bounds
        || m.offset != [0, 0]
        || m.outside_f64_bits != 0f64.to_bits()
        || !m.enabled
        || m.shown
        || m.editing
        || m.linked
    {
        return Err(scope());
    }
    let size = sample.bytes();
    let check = |bytes: &[u8]| -> Result<()> {
        if bytes.len() % size != 0 {
            return Err(scope());
        }
        for block in bytes.chunks(size * 64) {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
            for raw in block.chunks_exact(size) {
                let v = photocraft_color::try_read_sample_f64(raw, sample, 0).map_err(|_| scope())?;
                if !v.is_finite() || v != 0. {
                    return Err(scope());
                }
            }
        }
        Ok(())
    };
    check(mask.surface.default_bytes())?;
    for (coordinate, tile) in mask.surface.tiles() {
        if coordinate.tx != 0 || coordinate.ty != 0 {
            return Err(scope());
        }
        check(tile.bytes())?;
    }
    Ok(())
}
