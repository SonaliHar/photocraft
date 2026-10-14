//! Complete opt-in configured saved-channel storage, not import/render admission.
//! Every descriptor has one explicit binding. Original decoder/source/current
//! backing and native masks stay held by an exclusive scene-owner borrow. Child
//! quotas are prepaid once against that owner's existing leases; unused quota
//! is never refunded. No source pixels or document state are modified.
use super::{
    brightness_contrast_source_candidate::{self as brightness, BrightnessContrastSourceIdentity},
    configured_source_candidate::{self as curves, CurvesMappingLimits, CurvesMappingRequirements},
    hue_saturation_source_candidate::{self as hue, HueSaturationSourceIdentity},
    levels_source_candidate as levels,
    scene_channel_storage::SceneChannelStorage,
};
use crate::IoError;
use photocraft_compose::gimp_configured_candidate::{CURVES_SCHEMA, LEVELS_SCHEMA, RetainedSourceIdentity};
use photocraft_doc::{
    item_identity::{FilterId, ItemRef},
    xcf_filters::{FilterNode, FilterStack},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, Budget, TypedConfiguration};
use std::{cell::Cell, mem::size_of};

#[derive(Clone, Copy, Debug)]
pub enum ConfiguredEffectIdentity<'a> {
    Curves(RetainedSourceIdentity<'a>),
    Levels(RetainedSourceIdentity<'a>),
    BrightnessContrast(BrightnessContrastSourceIdentity<'a>),
    HueSaturation(HueSaturationSourceIdentity<'a>),
}
#[derive(Clone, Copy, Debug)]
pub struct ConfiguredEffectBinding<'a> {
    pub filter: FilterId,
    pub identity: ConfiguredEffectIdentity<'a>,
    /// Explicit lower quotas, not defaults reset for each effect. Input backing
    /// counted by a child remains conservatively charged again within its quota.
    pub limits: CurvesMappingLimits,
}
#[derive(Clone, Copy, Debug)]
pub struct SceneConfiguredLimits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for SceneConfiguredLimits {
    fn default() -> Self {
        Self { resident_bytes: 16 << 20, index_bytes: 1 << 20, work_units: 16_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SceneConfiguredRequirements {
    pub prepaid_resident_bytes: u64,
    pub prepaid_index_bytes: u64,
    pub prepaid_work_units: u64,
    /// All child input/copy quotas, plus actual outer vector capacities.
    pub charged_resident_bytes: u64,
    pub charged_index_bytes: u64,
    pub charged_work_units: u64,
    pub child_actual_copied_bytes: u64,
    pub child_actual_index_bytes: u64,
    pub child_actual_work_units: u64,
}
/// No extraction API can detach canonical stacks from their real backing leases.
pub struct ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'a> {
    scene: &'o mut SceneChannelStorage<'s, 'd, 'b, 'a>,
    bindings: &'p [ConfiguredEffectBinding<'p>],
    stacks: Vec<FilterStack>,
    requirements: SceneConfiguredRequirements,
}
impl<'o, 'p, 's, 'd, 'b, 'a> ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'a> {
    pub fn stacks(&self) -> &[FilterStack] {
        &self.stacks
    }
    pub fn scene(&self) -> &SceneChannelStorage<'s, 'd, 'b, 'a> {
        self.scene
    }
    pub fn bindings(&self) -> &[ConfiguredEffectBinding<'p>] {
        self.bindings
    }
    pub fn requirements(&self) -> SceneConfiguredRequirements {
        self.requirements
    }
}
fn bad(message: &str) -> IoError {
    IoError::Unsupported(format!("configured channel scene: {message}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size/work overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size/work overflow"))
}
struct Meter<'c, 'i> {
    limits: SceneConfiguredLimits,
    resident: Cell<u64>,
    index: Cell<u64>,
    work: Cell<u64>,
    ctl: &'c Interrupt<'i>,
}
impl Meter<'_, '_> {
    fn step(&self, amount: u64) -> Result<(), IoError> {
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        let value = add(self.work.get(), amount)?;
        if value > self.limits.work_units {
            return Err(bad("prepaid aggregate work exhausted"));
        }
        self.work.set(value);
        Ok(())
    }
    fn reserve(&self, bytes: u64, index: u64) -> Result<(), IoError> {
        self.step(1)?;
        let r = add(self.resident.get(), bytes)?;
        let i = add(self.index.get(), index)?;
        if r > self.limits.resident_bytes || i > self.limits.index_bytes {
            return Err(bad("prepaid aggregate allocation exhausted"));
        }
        self.resident.set(r);
        self.index.set(i);
        Ok(())
    }
    fn vector<T>(&self, count: usize) -> Result<Vec<T>, IoError> {
        let bytes = mul(count as u64, size_of::<T>() as u64)?;
        self.reserve(bytes, bytes)?;
        let mut out = Vec::new();
        out.try_reserve_exact(count).map_err(|_| bad("allocation"))?;
        let extra = mul(out.capacity().saturating_sub(count) as u64, size_of::<T>() as u64)?;
        self.reserve(extra, extra)?;
        Ok(out)
    }
}
fn identity_fields<'a>(identity: ConfiguredEffectIdentity<'a>) -> (u32, &'a str, &'a str) {
    match identity {
        ConfiguredEffectIdentity::Curves(v) | ConfiguredEffectIdentity::Levels(v) => (v.xcf_version, v.expected_epoch, v.schema_sha256),
        ConfiguredEffectIdentity::BrightnessContrast(v) => (v.xcf_version, v.expected_epoch, v.schema_sha256),
        ConfiguredEffectIdentity::HueSaturation(v) => (v.xcf_version, v.expected_epoch, v.schema_sha256),
    }
}
fn retained_typed_configuration<'a>(arguments: &'a [Argument], m: &Meter<'_, '_>) -> Result<&'a TypedConfiguration, IoError> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(bad("bounded configured argument envelope"));
    }
    let mut configuration = None;
    for argument in arguments {
        m.step(1)?;
        if argument.graph.is_some() {
            return Err(bad("graph is not a configured argument"));
        }
        match (argument.name.as_str(), &argument.value) {
            ("config", ArgumentValue::Config(_)) if configuration.is_none() => {
                configuration = Some(
                    argument
                        .configuration
                        .as_ref()
                        .and_then(|value| value.typed.as_ref())
                        .ok_or_else(|| bad("typed configuration on exact config argument required"))?,
                );
            }
            ("trc", ArgumentValue::Enum(_)) if argument.configuration.is_none() => {}
            _ => return Err(bad("unique config argument and scalar TRC association required")),
        }
    }
    configuration.ok_or_else(|| bad("config argument missing"))
}
fn child_actual(r: CurvesMappingRequirements, limit: CurvesMappingLimits) -> Result<(), IoError> {
    if r.reserved_resident_bytes != limit.resident_bytes
        || r.reserved_index_bytes != limit.index_bytes
        || r.reserved_work_units != limit.work_units
        || add(r.selected_input_bytes, r.copied_bytes)? > limit.resident_bytes
        || r.copied_index_bytes > limit.index_bytes
        || r.work_units > limit.work_units
    {
        return Err(bad("child actual charges exceed its prepaid quota"));
    }
    Ok(())
}
/// A failed attempt consumes the one-shot claim too. Only exact known configured
/// operations are admitted; unsupported arguments/operations across the whole
/// scene refuse before any canonical vector/configuration is materialized.
pub fn map_scene_channel_configured<'o, 'p, 's, 'd, 'b, 'a>(
    scene: &'o mut SceneChannelStorage<'s, 'd, 'b, 'a>,
    bindings: &'p [ConfiguredEffectBinding<'p>],
    limits: SceneConfiguredLimits,
    shared: &Budget<'_>,
    ctl: &Interrupt<'_>,
) -> Result<ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'a>, IoError> {
    let standard = SceneConfiguredLimits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.index_bytes > standard.index_bytes || limits.work_units > standard.work_units {
        return Err(bad("lower-only aggregate limits"));
    }
    scene.claim_configured(limits.resident_bytes, limits.index_bytes, limits.work_units, shared, ctl)?;
    let m = Meter { limits, resident: Cell::new(0), index: Cell::new(0), work: Cell::new(0), ctl };
    m.reserve(size_of::<ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'a>>() as u64, 0)?;
    // Caller binding storage is borrowed, but still belongs to this aggregate.
    m.reserve(mul(bindings.len() as u64, size_of::<ConfiguredEffectBinding<'p>>() as u64)?, 0)?;
    let mut position = 0usize;
    let mut child_resident = 0u64;
    let mut child_index = 0u64;
    let mut child_work = 0u64;
    for channel in scene.channels() {
        m.step(1)?;
        for descriptor in &channel.effects {
            m.step(1)?;
            let binding = bindings.get(position).ok_or_else(|| bad("one ordered binding per effect required"))?;
            position = position.checked_add(1).ok_or_else(|| bad("effect count overflow"))?;
            let effect = scene.original_effect(descriptor).ok_or_else(|| bad("exact scene descriptor association"))?;
            let child_max = CurvesMappingLimits::default();
            let (version, epoch, schema) = identity_fields(binding.identity);
            // Bound identity strings before touching their contents. Their exact
            // referenced bytes stay borrowed through the result.
            if epoch.len() > 128 || schema.len() != 64 {
                return Err(bad("bounded epoch/schema identity"));
            }
            m.reserve(add(epoch.len() as u64, schema.len() as u64)?, 0)?;
            m.step(add(epoch.len() as u64, schema.len() as u64)?)?;
            if binding.filter != descriptor.filter
                || version != 22
                || scene.file().version != version
                || epoch.is_empty()
                || epoch.len() > 128
                || epoch.contains('\0')
                || schema.len() != 64
                || effect.version.as_deref() != Some("0:0")
                || binding.limits.resident_bytes > child_max.resident_bytes
                || binding.limits.index_bytes > child_max.index_bytes
                || binding.limits.work_units > child_max.work_units
            {
                return Err(bad("ordered ID/version/epoch/lower quota binding"));
            }
            let config = retained_typed_configuration(&effect.arguments, &m)?;
            let expected = match (binding.identity, config) {
                (ConfiguredEffectIdentity::Curves(v), TypedConfiguration::Curves(_)) => {
                    if v.color_channels != 1 || v.alpha || v.profile_identity == [0; 32] || schema != CURVES_SCHEMA {
                        return Err(bad("explicit Gray Curves profile/schema"));
                    }
                    ("gimp:curves", 2)
                }
                (ConfiguredEffectIdentity::Levels(v), TypedConfiguration::Levels(_)) => {
                    if v.color_channels != 1 || v.alpha || v.profile_identity == [0; 32] || schema != LEVELS_SCHEMA {
                        return Err(bad("explicit Gray Levels profile/schema"));
                    }
                    ("gimp:levels", 2)
                }
                (ConfiguredEffectIdentity::BrightnessContrast(_), TypedConfiguration::BrightnessContrast(_)) => {
                    if !matches!(schema, brightness::BRIGHTNESS_CONTRAST_SCHEMA | brightness::BRIGHTNESS_CONTRAST_SCHEMA_308) {
                        return Err(bad("Brightness Contrast schema"));
                    }
                    ("gimp:brightness-contrast", 1)
                }
                (ConfiguredEffectIdentity::HueSaturation(_), TypedConfiguration::HueSaturation(_)) => {
                    if schema != hue::HUE_SATURATION_SCHEMA {
                        return Err(bad("Hue Saturation schema"));
                    }
                    ("gimp:hue-saturation", 1)
                }
                _ => return Err(bad("unsupported operation/configuration binding")),
            };
            if effect.operation != expected.0 || effect.arguments.len() != expected.1 {
                return Err(bad("unsupported configured operation/argument envelope"));
            }
            let mut configs = 0usize;
            let mut transfers = 0usize;
            for argument in &effect.arguments {
                m.step(1)?;
                match (&argument.value, argument.name.as_str()) {
                    (ArgumentValue::Config(_), "config") => configs += 1,
                    (ArgumentValue::Enum(_), "trc") if expected.1 == 2 => transfers += 1,
                    _ => return Err(bad("unsupported configured argument syntax")),
                }
            }
            if configs != 1 || transfers != expected.1 - 1 {
                return Err(bad("complete unique configured arguments"));
            }
            child_resident = add(child_resident, binding.limits.resident_bytes)?;
            child_index = add(child_index, binding.limits.index_bytes)?;
            child_work = add(child_work, binding.limits.work_units)?;
        }
    }
    if position != bindings.len() {
        return Err(bad("extra binding or omitted scene effect"));
    }
    // No child has allocated yet. All prospective child quotas must fit together
    // with stack/header storage. They will debit this same Meter exactly once.
    let outer_index = add(mul(scene.channels().len() as u64, size_of::<FilterStack>() as u64)?, mul(position as u64, size_of::<FilterNode>() as u64)?)?;
    let future_outer_work = add(add(mul(scene.channels().len() as u64, 4)?, mul(position as u64, 4)?)?, 4)?;
    if add(m.resident.get(), add(child_resident, outer_index)?)? > limits.resident_bytes
        || add(m.index.get(), add(child_index, outer_index)?)? > limits.index_bytes
        || add(m.work.get(), add(child_work, future_outer_work)?)? > limits.work_units
    {
        return Err(bad("complete prospective configured scene exceeds aggregate"));
    }
    let mut stacks = m.vector(scene.channels().len())?;
    let mut actual_copy = 0u64;
    let mut actual_index = 0u64;
    let mut actual_work = 0u64;
    let mut position = 0usize;
    for channel in scene.channels() {
        m.step(1)?;
        let mut nodes = m.vector(channel.effects.len())?;
        for descriptor in &channel.effects {
            m.step(1)?;
            let binding = bindings.get(position).ok_or_else(|| bad("binding position"))?;
            position = position.checked_add(1).ok_or_else(|| bad("binding count"))?;
            let effect = scene.original_effect(descriptor).ok_or_else(|| bad("descriptor association"))?;
            let mut reserve = |r, i| m.reserve(r, i);
            let mut charge = |w| m.step(w);
            let (node, requirement) = match binding.identity {
                ConfiguredEffectIdentity::Curves(id) => {
                    let value = curves::map_retained_curves_source(
                        scene.logical_source(),
                        effect,
                        descriptor.filter,
                        descriptor.mask,
                        id,
                        binding.limits,
                        ctl,
                        &mut reserve,
                        &mut charge,
                    )?;
                    let r = value.requirements();
                    (value.into_node(), r)
                }
                ConfiguredEffectIdentity::Levels(id) => {
                    let value = levels::map_retained_levels_source(
                        scene.logical_source(),
                        effect,
                        descriptor.filter,
                        descriptor.mask,
                        id,
                        binding.limits,
                        ctl,
                        &mut reserve,
                        &mut charge,
                    )?;
                    let r = value.requirements();
                    (value.into_node(), r)
                }
                ConfiguredEffectIdentity::BrightnessContrast(id) => {
                    let value = brightness::map_retained_brightness_contrast_source(
                        scene.logical_source(),
                        effect,
                        descriptor.filter,
                        descriptor.mask,
                        id,
                        binding.limits,
                        ctl,
                        &mut reserve,
                        &mut charge,
                    )?;
                    let r = value.requirements();
                    (value.into_node(), r)
                }
                ConfiguredEffectIdentity::HueSaturation(id) => {
                    let value = hue::map_retained_hue_saturation_source(
                        scene.logical_source(),
                        effect,
                        descriptor.filter,
                        descriptor.mask,
                        id,
                        binding.limits,
                        ctl,
                        &mut reserve,
                        &mut charge,
                    )?;
                    let r = value.requirements();
                    (value.into_node(), r)
                }
            };
            child_actual(requirement, binding.limits)?;
            if node.id != descriptor.filter || node.mask != descriptor.mask.map(ItemRef::Mask) || node.composition.as_ref().is_none_or(|v| v.resolved.is_some())
            {
                return Err(bad("exact IDs/unresolved composition must remain retained"));
            }
            actual_copy = add(actual_copy, requirement.copied_bytes)?;
            actual_index = add(actual_index, requirement.copied_index_bytes)?;
            actual_work = add(actual_work, requirement.work_units)?;
            nodes.push(node);
        }
        stacks.push(FilterStack { owner: ItemRef::Channel(channel.channel), nodes, revision: 0 });
    }
    m.step(1)?;
    let requirements = SceneConfiguredRequirements {
        prepaid_resident_bytes: limits.resident_bytes,
        prepaid_index_bytes: limits.index_bytes,
        prepaid_work_units: limits.work_units,
        charged_resident_bytes: m.resident.get(),
        charged_index_bytes: m.index.get(),
        charged_work_units: m.work.get(),
        child_actual_copied_bytes: actual_copy,
        child_actual_index_bytes: actual_index,
        child_actual_work_units: actual_work,
    };
    Ok(ConfiguredSceneStacks { scene, bindings, stacks, requirements })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    fn meter<'c, 'i>(ctl: &'c Interrupt<'i>) -> Meter<'c, 'i> {
        Meter {
            limits: SceneConfiguredLimits { resident_bytes: 256, index_bytes: 128, work_units: 128 },
            resident: Cell::new(0),
            index: Cell::new(0),
            work: Cell::new(0),
            ctl,
        }
    }
    #[test]
    fn typed_configuration_must_belong_to_the_unique_type6_config_argument() {
        let ctl = Interrupt::NONE;
        let m = meter(&ctl);
        let typed = photocraft_xcf::EffectConfiguration {
            nodes: Vec::new(),
            typed: Some(TypedConfiguration::BrightnessContrast(photocraft_xcf::BrightnessContrastConfiguration {
                time_bits: Some(0),
                brightness_bits: (-0f64).to_bits(),
                contrast_bits: 0,
            })),
        };
        let config = Argument {
            name: "config".into(),
            property_index: 0,
            value: ArgumentValue::Config("own raw config".into()),
            configuration: Some(typed),
            graph: None,
        };
        assert!(matches!(retained_typed_configuration(std::slice::from_ref(&config), &m).unwrap(), TypedConfiguration::BrightnessContrast(_)));
        assert!(retained_typed_configuration(&[config.clone(), config.clone()], &m).is_err());
        let mut misplaced = config;
        misplaced.name = "trc".into();
        misplaced.value = ArgumentValue::Enum(1);
        assert!(retained_typed_configuration(&[misplaced], &m).is_err());
    }
    #[test]
    fn two_child_quotas_cannot_reuse_first_unused_capacity() {
        let ctl = Interrupt::NONE;
        let m = meter(&ctl);
        m.reserve(200, 100).unwrap();
        m.step(100).unwrap();
        assert!(m.reserve(57, 1).is_err());
        assert!(m.reserve(1, 29).is_err());
        assert!(m.step(29).is_err());
        assert_eq!(m.resident.get(), 200);
        assert_eq!(m.index.get(), 100);
        // Failed traversals do not reset a prepaid child's existing work.
        assert!(m.work.get() >= 100);
    }
    #[test]
    fn late_cancellation_refuses_second_private_vector_without_mutating_first() {
        let cancel = AtomicBool::new(false);
        let check = || cancel.load(Ordering::Relaxed);
        let ctl = Interrupt::cancel_only(&check);
        let m = meter(&ctl);
        let mut first = m.vector::<u64>(2).unwrap();
        first.extend([0x8000000000000000, 0x3ff0000009b26224]);
        let retained = m.resident.get();
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(m.vector::<u64>(2), Err(IoError::Cancelled)));
        assert_eq!(first, [0x8000000000000000, 0x3ff0000009b26224]);
        assert_eq!(m.resident.get(), retained);
    }
    #[test]
    fn child_requirement_cannot_smuggle_actual_copy_or_work_above_prepaid_quota() {
        let limit = CurvesMappingLimits { resident_bytes: 100, index_bytes: 50, work_units: 80 };
        let mut r = CurvesMappingRequirements {
            reserved_resident_bytes: 100,
            reserved_index_bytes: 50,
            reserved_work_units: 80,
            selected_input_bytes: 40,
            copied_bytes: 60,
            copied_index_bytes: 50,
            work_units: 80,
        };
        child_actual(r, limit).unwrap();
        r.copied_bytes += 1;
        assert!(child_actual(r, limit).is_err());
        r.copied_bytes = 60;
        r.work_units += 1;
        assert!(child_actual(r, limit).is_err());
        r.work_units = 80;
        r.reserved_index_bytes -= 1;
        assert!(child_actual(r, limit).is_err());
    }
}
