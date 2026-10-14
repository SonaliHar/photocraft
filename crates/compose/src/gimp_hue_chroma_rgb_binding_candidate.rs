//! Source-preserving, explicitly caller-bound RGB Hue-Chroma candidate.
//!
//! This composes the retained typed argument/schema binding with the analytical
//! RGB candidate. It is a point-stage interface, not an imported-filter renderer:
//! enabled/opacity/mask/auxiliary/composition/region/clipping remain original data
//! and are neither interpreted nor normalized. No opaque space tuple is aliased.
//! RGB ingress/egress schedule remains a candidate. Display acceptance is the
//! existing matched-profile contract; this module embeds no tolerance or grant.
use crate::{gimp_hue_chroma_candidate as typed, gimp_hue_chroma_color_candidate as rgb};
use photocraft_cms::profile::Profile;
use photocraft_doc::xcf_filters::FilterNode;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};

const BINDING_WORK: u64 = 32;

/// Both contexts must be established by the caller. The intermediate LCh space
/// identity and RGB working-space identity are distinct roles, never synonyms.
#[derive(Clone, Copy)]
pub struct Binding<'a> {
    pub source: typed::SourceBinding<'a>,
    pub rgb_context: rgb::WorkingContext,
}

/// Borrows immutable source configuration/profile; owns only prepared pipelines.
/// Holding this value never authorizes use as the original filter's compositor.
pub struct BoundRgbPoint<'a> {
    source: typed::BoundSource<'a>,
    prepared: rgb::PreparedRgb<'a>,
    limits: rgb::Limits,
}

fn lower(limits: rgb::Limits) -> Result<rgb::Limits, E> {
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > 1 << 30 {
        return Err(E::LimitExceeded);
    }
    Ok(rgb::Limits { resident_bytes: limits.resident_bytes, work_units: limits.work_units.checked_sub(BINDING_WORK).ok_or(E::LimitExceeded)? })
}

/// Explicit invocation scope. Outer serialized records remain unconsumed even
/// when the original filter is disabled or has opacity/masks/auxiliary inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationScope {
    PointStageOnly,
}

fn binding_valid(binding: Binding<'_>, control: &Interrupt<'_>) -> Result<(), E> {
    control.check().map_err(|_| E::Cancelled)?;
    if binding.source.context.profile_identity == [0; 32]
        || binding.source.context.space_identity == [0; 32]
        || binding.source.context.profile_identity != binding.rgb_context.profile_identity
    {
        return Err(E::UnsupportedConversion);
    }
    Ok(())
}

/// Allocation-free typed/config/profile admission. Profile parsing and identities
/// are caller-owned; original node payload/resident accounting remains caller-owned.
pub fn preparation_requirements(
    node: &FilterNode,
    binding: Binding<'_>,
    profile: &Profile,
    limits: rgb::Limits,
    control: &Interrupt<'_>,
) -> Result<rgb::PreparationRequirements, E> {
    preparation_requirements_with_trc_policy(node, binding, profile, rgb::TrcPolicy::StrictAnalytic, limits, control)
}
/// Explicit analytical-pair policy, still point-stage only and not admission.
pub fn preparation_requirements_with_trc_policy(
    node: &FilterNode,
    binding: Binding<'_>,
    profile: &Profile,
    policy: rgb::TrcPolicy,
    limits: rgb::Limits,
    control: &Interrupt<'_>,
) -> Result<rgb::PreparationRequirements, E> {
    binding_valid(binding, control)?;
    let source = typed::bind_source_node(node, binding.source, control)?;
    let mut result = rgb::preparation_requirements_with_trc_policy(profile, binding.rgb_context, source.parameters(), policy, lower(limits)?, control)?;
    result.inspection_work_units = result.inspection_work_units.checked_add(BINDING_WORK).ok_or(E::LimitExceeded)?;
    result.work_units = result.work_units.checked_add(BINDING_WORK).ok_or(E::LimitExceeded)?;
    control.check().map_err(|_| E::Cancelled)?;
    Ok(result)
}

pub fn bind_source_rgb<'a>(
    node: &'a FilterNode,
    binding: Binding<'a>,
    profile: &'a Profile,
    limits: rgb::Limits,
    control: &Interrupt<'_>,
) -> Result<BoundRgbPoint<'a>, E> {
    bind_source_rgb_with_trc_policy(node, binding, profile, rgb::TrcPolicy::StrictAnalytic, limits, control)
}
/// Explicit opt-in; source curves, outer records and caller contexts stay original.
pub fn bind_source_rgb_with_trc_policy<'a>(
    node: &'a FilterNode,
    binding: Binding<'a>,
    profile: &'a Profile,
    policy: rgb::TrcPolicy,
    limits: rgb::Limits,
    control: &Interrupt<'_>,
) -> Result<BoundRgbPoint<'a>, E> {
    binding_valid(binding, control)?;
    let source = typed::bind_source_node(node, binding.source, control)?;
    let prepared = rgb::PreparedRgb::prepare_with_trc_policy(profile, binding.rgb_context, source.parameters(), policy, lower(limits)?, control)?;
    control.check().map_err(|_| E::Cancelled)?;
    Ok(BoundRgbPoint { source, prepared, limits })
}

impl<'a> BoundRgbPoint<'a> {
    pub fn trc_policy(&self) -> rgb::TrcPolicy {
        self.prepared.trc_policy()
    }
    pub fn scope(&self) -> EvaluationScope {
        EvaluationScope::PointStageOnly
    }
    pub fn original_node(&self) -> &'a FilterNode {
        self.source.original_node()
    }
    pub fn context(&self) -> rgb::WorkingContext {
        self.prepared.context()
    }
    pub fn parameters(&self) -> photocraft_algo::gimp::hue_chroma::Parameters {
        self.prepared.parameters()
    }
    pub fn requirements(&self, input: rgb::RgbFloatView<'_>, roi: Rect, limits: rgb::Limits, control: &Interrupt<'_>) -> Result<rgb::Requirements, E> {
        if limits.resident_bytes > self.limits.resident_bytes || limits.work_units > self.limits.work_units {
            return Err(E::LimitExceeded);
        }
        let mut result = self.prepared.requirements(input, roi, lower(limits)?, control)?;
        result.preparation_work_units = result.preparation_work_units.checked_add(BINDING_WORK).ok_or(E::LimitExceeded)?;
        result.work_units = result.work_units.checked_add(BINDING_WORK).ok_or(E::LimitExceeded)?;
        control.check().map_err(|_| E::Cancelled)?;
        Ok(result)
    }
    /// Returns a private point-stage window. Callers must separately decide whether
    /// and how original composition/masks/region/enabled state can be consumed.
    pub fn render_roi(&self, input: rgb::RgbFloatView<'_>, roi: Rect, limits: rgb::Limits, control: &Interrupt<'_>) -> Result<rgb::RgbFloatWindow, E> {
        self.requirements(input, roi, limits, control)?;
        self.prepared.render_roi(input, roi, lower(limits)?, control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::gimp::hue_chroma::LchFloatContext;
    use photocraft_doc::{
        SampleEncoding,
        item_identity::{FilterId, ItemRef},
        xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
    };

    fn binding() -> Binding<'static> {
        Binding {
            source: typed::SourceBinding {
                expected_epoch: "gimp-3.2.6-windows-0c253517c76eee04",
                schema_sha256: typed::SCHEMA_326_320,
                context: LchFloatContext { profile_identity: [7; 32], space_identity: [9; 32] },
            },
            rgb_context: rgb::WorkingContext { profile_identity: [7; 32], working_space_identity: [11; 32], encoding: SampleEncoding::NonlinearProfile },
        }
    }
    fn node() -> FilterNode {
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:hue-chroma".into(), stored_version: "0:0".into(), schema_epoch: binding().source.expected_epoch.into() },
            source_display_name: Some("Original label".into()),
            source_icon: None,
            arguments: [("hue", 37.5_f64), ("chroma", -12.5), ("lightness", 8.25)]
                .into_iter()
                .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(value.to_bits())) })
                .collect(),
            enabled: false,
            opacity_f64_bits: 0.25_f64.to_bits(),
            source_clipping: Some(17),
            source_region: Some(29),
            mask: None,
            auxiliary: Vec::<(String, ItemRef)>::new(),
            composition: None,
        }
    }
    #[test]
    fn source_outer_records_remain_original_and_contexts_are_distinct() {
        let original = node();
        let before = original.clone();
        let profile = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let bound = bind_source_rgb(&original, binding(), profile, rgb::Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(bound.scope(), EvaluationScope::PointStageOnly);
        assert!(std::ptr::eq(bound.original_node(), &original));
        let pixels = [[0.25, 0.5, 0.75, -0.0]];
        let input = rgb::RgbFloatView { extent: Rect::new(-3, 7, -2, 8), pixels: &pixels, context: binding().rgb_context };
        let output = bound.render_roi(input, input.extent, rgb::Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(output.pixels[0][3].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(original, before);
        let wrong = rgb::RgbFloatView { context: rgb::WorkingContext { working_space_identity: [12; 32], ..input.context }, ..input };
        assert_eq!(bound.render_roi(wrong, wrong.extent, rgb::Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
    }
    #[test]
    fn binding_work_cannot_refresh_or_raise_the_prepared_envelope() {
        let original = node();
        let profile = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let req = preparation_requirements(&original, binding(), profile, rgb::Limits::default(), &Interrupt::NONE).unwrap();
        let low = rgb::Limits { work_units: req.work_units - 1, ..rgb::Limits::default() };
        assert_eq!(bind_source_rgb(&original, binding(), profile, low, &Interrupt::NONE).err(), Some(E::LimitExceeded));
        let mut wrong = binding();
        wrong.rgb_context.profile_identity = [8; 32];
        assert_eq!(bind_source_rgb(&original, wrong, profile, rgb::Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
        assert_eq!(bind_source_rgb(&original, binding(), profile, rgb::Limits::default(), &Interrupt::cancel_only(&|| true)).err(), Some(E::Cancelled));
        let raised = rgb::Limits { work_units: (1 << 30) + 1, ..rgb::Limits::default() };
        assert_eq!(bind_source_rgb(&original, binding(), profile, raised, &Interrupt::NONE).err(), Some(E::LimitExceeded));
    }
}
