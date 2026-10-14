//! Explicit retained Vibrance -> controlled Linear Float conversion consumer.
//! No source/default dispatcher admission or original outer filter compositing.
//! Native source conversion into the declared Linear Float stage is caller-owned.
use crate::{gimp_vibrance_candidate as source, gimp_vibrance_color_candidate as color};
use photocraft_algo::gimp::point_vibrance::{Context, FloatConversions, Parameters};
use photocraft_cms::Profile;
use photocraft_doc::xcf_filters::FilterNode;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError as E};
pub use source::{EvaluationScope, RgbFloatView, RgbFloatWindow, SourceBinding};
use std::mem::size_of;
const BIND_WORK: u64 = 32;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        let provider = color::Limits::default();
        let stage = source::Limits::default();
        Self { resident_bytes: provider.resident_bytes.min(stage.resident_bytes), work_units: provider.work_units.min(stage.work_units) }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PreparationRequirements {
    pub profile: color::PreparationRequirements,
    pub consumer_bytes: u64,
    pub peak_bytes: u64,
    pub work_units: u64,
}
pub struct PreparedSource<'a> {
    bound: source::BoundSource<'a>,
    conversions: color::PreparedLinearConversions<'a>,
    ceiling: Limits,
    provider_ceiling: color::Limits,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::LimitExceeded)
}
fn consumer_bytes() -> Result<u64, E> {
    u64::try_from(size_of::<PreparedSource<'_>>()).map_err(|_| E::LimitExceeded)
}
fn constructor_limits(limits: Limits) -> Result<color::Limits, E> {
    let max = Limits::default();
    if limits.resident_bytes > max.resident_bytes || limits.work_units > max.work_units {
        return Err(E::LimitExceeded);
    }
    Ok(color::Limits {
        resident_bytes: limits.resident_bytes.checked_sub(consumer_bytes()?).ok_or(E::LimitExceeded)?,
        work_units: limits.work_units.checked_sub(BIND_WORK).ok_or(E::LimitExceeded)?,
    })
}
fn profile_plan(original: &Profile, context: Context, limits: Limits, c: &Interrupt<'_>) -> Result<(PreparationRequirements, color::Limits), E> {
    let provider_limits = constructor_limits(limits)?;
    let profile = color::preparation_requirements(original, context, provider_limits, c)?;
    let consumer_bytes = consumer_bytes()?;
    let peak_bytes = add(profile.peak_bytes, consumer_bytes)?;
    let work_units = add(profile.work_units, BIND_WORK)?;
    if peak_bytes > limits.resident_bytes || work_units > limits.work_units {
        return Err(E::LimitExceeded);
    }
    poll(c)?;
    Ok((PreparationRequirements { profile, consumer_bytes, peak_bytes, work_units }, provider_limits))
}
/// Borrowed schema binding precedes the profile census; no profile/config copies.
/// The borrowed original FilterNode payload backing is still caller-accounted.
pub fn preparation_requirements(
    node: &FilterNode,
    binding: SourceBinding<'_>,
    original: &Profile,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<PreparationRequirements, E> {
    poll(c)?;
    constructor_limits(limits)?;
    let _bound = source::bind_source_node(node, binding, c)?;
    profile_plan(original, binding.context, limits, c).map(|(plan, _)| plan)
}
impl<'a> PreparedSource<'a> {
    pub fn prepare(node: &'a FilterNode, binding: SourceBinding<'a>, original: &'a Profile, limits: Limits, c: &Interrupt<'_>) -> Result<Self, E> {
        poll(c)?;
        constructor_limits(limits)?;
        // Strict retained scalar binding is complete before private CMS construction.
        let bound = source::bind_source_node(node, binding, c)?;
        let (plan, provider_ceiling) = profile_plan(original, binding.context, limits, c)?;
        let conversions = color::PreparedLinearConversions::prepare(original, binding.context, provider_ceiling, c)?;
        let retained = conversions.requirements(c)?;
        if add(add(retained.retained_bytes, retained.scratch_bytes)?, plan.consumer_bytes)? > limits.resident_bytes
            || add(retained.preparation_work_units, BIND_WORK)? > limits.work_units
        {
            return Err(E::LimitExceeded);
        }
        poll(c)?;
        Ok(Self { bound, conversions, ceiling: limits, provider_ceiling })
    }
    pub fn original_node(&self) -> &'a FilterNode {
        self.bound.original_node()
    }
    pub fn original_profile(&self) -> &'a Profile {
        self.conversions.original_profile()
    }
    pub fn context(&self) -> Context {
        self.conversions.context()
    }
    pub fn parameters(&self) -> Parameters {
        self.bound.parameters()
    }
    pub fn scope(&self) -> EvaluationScope {
        self.bound.scope()
    }
    fn evaluation_limits(&self, limits: Limits, c: &Interrupt<'_>) -> Result<source::Limits, E> {
        poll(c)?;
        if limits.resident_bytes > self.ceiling.resident_bytes || limits.work_units > self.ceiling.work_units {
            return Err(E::LimitExceeded);
        }
        // The converter independently retains its immutable construction ceiling.
        // The enclosing request never resets its construction or steady pair budget.
        let retained = self.conversions.requirements(c)?;
        if add(retained.retained_bytes, retained.scratch_bytes)? > self.provider_ceiling.resident_bytes
            || add(retained.preparation_work_units, retained.work_units)? > self.provider_ceiling.work_units
        {
            return Err(E::LimitExceeded);
        }
        Ok(source::Limits { resident_bytes: limits.resident_bytes.checked_sub(consumer_bytes()?).ok_or(E::LimitExceeded)?, work_units: limits.work_units })
    }
    /// Full request includes consumer ownership plus the existing source/provider
    /// envelope. Preparation work is already charged once by BoundSource::plan.
    pub fn requirements(&self, input: RgbFloatView<'_>, roi: Rect, limits: Limits, c: &Interrupt<'_>) -> Result<source::Requirements, E> {
        let lower = self.evaluation_limits(limits, c)?;
        let mut required = self.bound.requirements(input, roi, &self.conversions, lower, c)?;
        required.peak_bytes = add(required.peak_bytes, consumer_bytes()?)?;
        if required.peak_bytes > limits.resident_bytes || required.work_units > limits.work_units {
            return Err(E::LimitExceeded);
        }
        poll(c)?;
        Ok(required)
    }
    pub fn evaluate(&self, input: RgbFloatView<'_>, roi: Rect, limits: Limits, c: &Interrupt<'_>) -> Result<RgbFloatWindow, E> {
        let lower = self.evaluation_limits(limits, c)?;
        // Existing candidate admits/validates every selected sample before reserve,
        // and accounts provider prep exactly once plus all steady per-pixel work.
        self.bound.evaluate(input, roi, &self.conversions, lower, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::gimp::{
        configured::{Transfer, WorkingContext},
        hue_chroma::LchFloatContext,
    };
    use photocraft_doc::{
        item_identity::FilterId,
        xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
    };
    fn context() -> Context {
        Context {
            rgb: WorkingContext { transfer: Transfer::Linear, profile_identity: [1; 32], color_channels: 3, alpha: true },
            lch: LchFloatContext { profile_identity: [1; 32], space_identity: [2; 32] },
        }
    }
    fn binding() -> SourceBinding<'static> {
        SourceBinding { expected_epoch: source::EPOCH_326, schema_sha256: source::SCHEMA_326, context: context() }
    }
    fn node() -> FilterNode {
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:vibrance".into(), stored_version: "0:0".into(), schema_epoch: source::EPOCH_326.into() },
            source_display_name: Some("Retained disabled Vibrance".into()),
            source_icon: None,
            enabled: false,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: None,
            arguments: vec![
                FilterArgument { name: "vibrance".into(), value: FilterValue::Number(ExactNumber::Float32Bits(5.0f32.to_bits())) },
                FilterArgument { name: "saturation".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
            ],
        }
    }
    #[test]
    fn real_controlled_provider_consumer_matches_direct_stage_and_preserves_source_owners() {
        let node = node();
        let before = node.clone();
        let original = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let original_bytes = original.to_bytes();
        let limits = Limits::default();
        let prepared = PreparedSource::prepare(&node, binding(), original, limits, &Interrupt::NONE).unwrap();
        let pixels = [[0.25, 0.5, 0.75, -0.0], [0.5, 0.25, 0.75, 1.0]];
        let before_pixels = pixels.map(|p| p.map(f32::to_bits));
        let view = RgbFloatView { extent: Rect::new(-1, 2, 1, 3), pixels: &pixels, context: context() };
        let output = prepared.evaluate(view, view.extent, limits, &Interrupt::NONE).unwrap();
        let direct = source::bind_source_node(&node, binding(), &Interrupt::NONE)
            .unwrap()
            .evaluate(
                view,
                view.extent,
                &prepared.conversions,
                source::Limits { resident_bytes: limits.resident_bytes - consumer_bytes().unwrap(), work_units: limits.work_units },
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(
            output.pixels.iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>(),
            direct.pixels.iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>()
        );
        assert_eq!(output.pixels[0][3].to_bits(), (-0.0f32).to_bits());
        assert_eq!(prepared.original_node(), &before);
        assert_eq!(prepared.scope(), EvaluationScope::PointStagesOnly);
        assert_eq!(original.to_bytes().as_slice(), original_bytes.as_slice());
        assert_eq!(pixels.map(|p| p.map(f32::to_bits)), before_pixels);
    }
    #[test]
    fn preparation_and_request_caps_never_raise_and_constructor_work_is_not_repeated_per_pixel() {
        let node = node();
        let original = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let limits = Limits::default();
        let prepared = PreparedSource::prepare(&node, binding(), original, limits, &Interrupt::NONE).unwrap();
        let pixels = [[0.25, 0.5, 0.75, 1.0]; 2];
        let view = RgbFloatView { extent: Rect::new(0, 0, 2, 1), pixels: &pixels, context: context() };
        let one = prepared.requirements(view, Rect::new(0, 0, 1, 1), limits, &Interrupt::NONE).unwrap();
        let two = prepared.requirements(view, view.extent, limits, &Interrupt::NONE).unwrap();
        assert_eq!(two.work_units - one.work_units, 400);
        assert!(matches!(
            prepared.evaluate(view, view.extent, Limits { resident_bytes: limits.resident_bytes, work_units: limits.work_units + 1 }, &Interrupt::NONE),
            Err(E::LimitExceeded)
        ));
        assert!(matches!(
            prepared.requirements(view, view.extent, Limits { resident_bytes: limits.resident_bytes + 1, work_units: limits.work_units }, &Interrupt::NONE),
            Err(E::LimitExceeded)
        ));
        assert!(matches!(
            prepared.evaluate(view, view.extent, Limits { resident_bytes: two.peak_bytes, work_units: two.work_units - 1 }, &Interrupt::NONE),
            Err(E::LimitExceeded)
        ));
    }
    #[test]
    fn malformed_retained_schema_precedes_profile_construction_and_cancel_is_not_fallback() {
        let mut malformed = node();
        malformed.operation.schema_epoch = "unknown".into();
        let original = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let before = original.to_bytes();
        assert!(matches!(PreparedSource::prepare(&malformed, binding(), original, Limits::default(), &Interrupt::NONE), Err(E::UnsupportedConversion)));
        let node = node();
        let stop = || true;
        assert!(matches!(PreparedSource::prepare(&node, binding(), original, Limits::default(), &Interrupt::cancel_only(&stop)), Err(E::Cancelled)));
        assert!(matches!(
            PreparedSource::prepare(&node, binding(), original, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
            Err(E::LimitExceeded)
        ));
        assert_eq!(original.to_bytes().as_slice(), before.as_slice());
    }
}
