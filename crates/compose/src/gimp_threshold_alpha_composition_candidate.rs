//! Explicit alpha-only consumer. No RGB/ICC schedule or source admission is inferred.
use photocraft_algo::gimp::{threshold_alpha::ThresholdAlpha, threshold_alpha_composition as alpha};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
use photocraft_raster::{Interrupt, RasterInputError as E};

/// Exact metadata supplied by a caller that already owns the original node.
/// Matching this structure is not a source-proof or file-reading capability.
pub struct SourceBinding<'a> {
    pub source_version: u32,
    pub operation_epoch: &'a str,
    pub composition_epoch: &'a str,
    pub stored_triplet: [i32; 3],
}
pub struct BoundAlpha<'a> {
    node: &'a FilterNode,
    threshold: ThresholdAlpha,
    mode: alpha::Mode,
    opacity: alpha::ObservedOpacity,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
/// Bind the observed enabled, unmasked, unclipped selection-region-zero domain.
/// Opaque words remain opaque; they are never converted into Auto/resolved enums.
pub fn bind_source<'a>(node: &'a FilterNode, binding: SourceBinding<'_>, c: &Interrupt<'_>) -> Result<BoundAlpha<'a>, E> {
    poll(c)?;
    if binding.source_version != 22
        || binding.operation_epoch.is_empty()
        || binding.operation_epoch.len() > 128
        || binding.operation_epoch.contains('\0')
        || binding.composition_epoch.is_empty()
        || binding.composition_epoch.len() > 128
        || binding.composition_epoch.contains('\0')
        || binding.stored_triplet != [239108689; 3]
        || node.operation.name != "gimp:threshold-alpha"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != binding.operation_epoch
        || !node.enabled
        || node.mask.is_some()
        || !node.auxiliary.is_empty()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || node.arguments.len() != 1
    {
        return Err(E::UnsupportedConversion);
    }
    let comp = node.composition.as_ref().ok_or(E::UnsupportedConversion)?;
    if comp.compatibility_epoch != binding.composition_epoch
        || comp.legacy
        || comp.pass_through
        || comp.resolved.is_some()
        || [comp.stored_operator, comp.stored_composite_space, comp.stored_blend_space] != binding.stored_triplet
    {
        return Err(E::UnsupportedConversion);
    }
    let mode = match comp.source_mode {
        28 => alpha::Mode::Normal,
        62 => alpha::Mode::Replace,
        _ => return Err(E::UnsupportedConversion),
    };
    let opacity = match f64::from_bits(node.opacity_f64_bits) {
        0.5 => alpha::ObservedOpacity::Half,
        1.0 => alpha::ObservedOpacity::Full,
        _ => return Err(E::UnsupportedConversion),
    };
    let argument = node.arguments.first().ok_or(E::LengthMismatch)?;
    let value = match &argument.value {
        // The independent XCF tag45 observer records type3/Float32 source
        // values. Promote exactly to the operation's Double threshold while
        // retaining the original typed argument. Current Float64 values remain
        // an explicit caller-owned point-stage input.
        FilterValue::Number(ExactNumber::Float32Bits(bits)) => f64::from(f32::from_bits(*bits)),
        FilterValue::Number(ExactNumber::Float64Bits(bits)) => f64::from_bits(*bits),
        _ => return Err(E::UnsupportedConversion),
    };
    if argument.name != "value" || !matches!(value, 0.25 | 0.5) {
        return Err(E::UnsupportedConversion);
    }
    poll(c)?;
    Ok(BoundAlpha { node, threshold: ThresholdAlpha { value }, mode, opacity })
}
impl BoundAlpha<'_> {
    pub fn original_node(&self) -> &FilterNode {
        self.node
    }
    pub fn requirements(&self, count: usize, limits: alpha::Limits) -> Result<alpha::Requirements, E> {
        alpha::requirements(count, limits)
    }
    /// Caller-supplied straight Float32 alpha. Color/profile/extent/association
    /// remain explicit outer boundaries; this returns no mislabeled Surface.
    pub fn evaluate(&self, source_alpha: &[f32], limits: alpha::Limits, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
        alpha::evaluate(source_alpha, self.threshold, self.mode, self.opacity, limits, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_outer_binding_preserves_node_and_refuses_auto_alias() {
        use photocraft_doc::{
            import_compositing::GimpComposition,
            item_identity::FilterId,
            xcf_filters::{FilterArgument, OperationIdentity},
        };
        let node = FilterNode {
            id: FilterId(7),
            operation: OperationIdentity { name: "gimp:threshold-alpha".into(), stored_version: "0:0".into(), schema_epoch: "test-leaf".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits())) }],
            enabled: true,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: "test-envelope".into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 239108689,
                stored_composite_space: 239108689,
                stored_operator: 239108689,
                resolved: None,
                pass_through: false,
            }),
        };
        let binding = || SourceBinding { source_version: 22, operation_epoch: "test-leaf", composition_epoch: "test-envelope", stored_triplet: [239108689; 3] };
        let before = node.clone();
        let bound = bind_source(&node, binding(), &Interrupt::NONE).unwrap();
        assert!(std::ptr::eq(bound.original_node(), &node));
        assert_eq!(bound.evaluate(&[0.25], alpha::Limits::default(), &Interrupt::NONE).unwrap(), vec![0.25]);
        let mut altered = node.clone();
        altered.composition.as_mut().unwrap().stored_operator = 0;
        assert!(bind_source(&altered, binding(), &Interrupt::NONE).is_err());
        assert_eq!(node, before);
        let mut stored = node.clone();
        stored.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits()));
        let original_stored = stored.clone();
        let bound_stored = bind_source(&stored, binding(), &Interrupt::NONE).unwrap();
        assert!(std::ptr::eq(bound_stored.original_node(), &stored));
        assert_eq!(bound_stored.evaluate(&[0.25], alpha::Limits::default(), &Interrupt::NONE).unwrap(), vec![0.25]);
        assert_eq!(stored, original_stored);
        stored.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()));
        assert!(bind_source(&stored, binding(), &Interrupt::NONE).is_err());
    }
}
