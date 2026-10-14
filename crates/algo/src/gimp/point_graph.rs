//! Explicit caller-bound checked point callback for fixed-domain graphs.
//! The caller supplies exact schema/version/current CMS interpretation. No node
//! name, source precision, absent profile or opaque argument confers admission.
use super::{
    graph_execute::{ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    point::{self, PointOperation},
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{ExactNumber, FilterValue, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, checked_region_byte_len};
#[path = "point_graph/red_eye_binding.rs"]
mod red_eye_binding;
#[path = "point_graph/color_exchange_binding.rs"]
mod color_exchange_binding;

/// Independently qualified caller interpretation of this exact node. Supplying
/// arbitrary source metadata is not schema or profile qualification.
pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub domain: ExecutionDomain,
    /// Complete authoritative source extent, including default pixels.
    pub full_extent: Rect,
    pub operation: PointOperation,
}

/// Use this predicate while planning all incoming/outgoing graph edges.
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    matches!((direction, port), (PortDirection::Input, "input") | (PortDirection::Output, "output"))
}

pub fn evaluate(binding: Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if binding.node.0 == 0
        || binding.node != request.node.id
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || binding.identity != &request.node.operation
        || binding.version_origin != request.node.version_origin
    {
        return Err(GraphExecuteError::Operation("point callback requires exact caller-bound node/schema/version origin"));
    }
    if binding.domain != request.domain {
        return Err(GraphExecuteError::Domain);
    }
    if binding.full_extent != request.roi {
        return Err(GraphExecuteError::Operation("point graph requires the whole fixed input ROI"));
    }
    validate_operation(&binding, request.node)?;
    let prepared = binding.operation.prepare().map_err(GraphExecuteError::Raster)?;
    if request.inputs.len() != 1 {
        return Err(GraphExecuteError::Operation("point graph requires exactly one input"));
    }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if !accepts_port(PortDirection::Input, input.input_port) || !accepts_port(PortDirection::Output, input.output_port) {
        return Err(GraphExecuteError::Operation("point graph supports exact input/output ports only"));
    }
    let source = input.surface;
    let format = source.format();
    if input.domain != request.domain || format != request.domain.format || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(GraphExecuteError::Domain);
    }
    let bytes = checked_region_byte_len(request.roi, format).map_err(GraphExecuteError::Raster)? as u64;
    let tiles = tile_count(request.roi)?;
    let source_tiles = source.tile_count() as u64;
    if source_tiles > tiles {
        return Err(GraphExecuteError::Operation("point source contains unrelated outside-ROI tiles"));
    }
    let bpp = format.bytes_per_pixel() as u64;
    let tile_bytes = (TILE_SIZE as u64).checked_pow(2).and_then(|n| n.checked_mul(bpp)).ok_or(GraphExecuteError::ResidentLimit)?;
    let source_charge = source_tiles.checked_mul(tile_bytes + 256).and_then(|n| n.checked_add(256 + bpp)).ok_or(GraphExecuteError::ResidentLimit)?;
    let output_charge = tiles.checked_mul(tile_bytes + 256).and_then(|n| n.checked_add(256 + bpp)).ok_or(GraphExecuteError::ResidentLimit)?;
    let index = source_tiles.checked_add(tiles).and_then(|n| n.checked_mul(512)).and_then(|n| n.checked_add(1024)).ok_or(GraphExecuteError::IndexLimit)?;
    let scratch = source_charge
        .checked_mul(2)
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(4096))
        .ok_or(GraphExecuteError::ResidentLimit)?;
    let work = bytes
        .checked_mul(16)
        .and_then(|n| n.checked_add(request.roi.size().area().checked_mul(format.channels() as u64)?.checked_mul(8)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(source_tiles.checked_add(tiles)?)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(source_tiles.checked_mul(256)?))
        .ok_or(GraphExecuteError::WorkLimit)?;
    // Includes a stack-only sample/output preflight and growing map visits.
    budget.charge_work(work)?;
    let control = || budget.poll().is_err();
    source.try_validate_tile_origins_with(&Interrupt::cancel_only(&control)).map_err(GraphExecuteError::Raster)?;
    for (tile, _) in source.tiles() {
        budget.poll()?;
        if tile.rect().intersect(&request.roi).is_empty() {
            return Err(GraphExecuteError::Operation("point source contains unrelated outside-ROI tiles"));
        }
    }
    // Reject nonfinite/unsupported sample domains before allocating private output.
    // Alpha threshold and Float mixer kernels require complete RGBA arity.
    // Mixers copy Float alpha exactly; color-only families retain its encoded
    // bytes through try_apply without interpreting it as a color component.
    let colors = format.mode.color_channels();
    let evaluated = if matches!(
        binding.operation,
        PointOperation::ThresholdAlpha { .. } | PointOperation::ChannelMixer { .. } | PointOperation::MonoMixerSigned { .. }
            | PointOperation::RedEyeRemoval { .. }
            | PointOperation::ColorExchange { .. }
    ) { format.channels() } else { colors };
    for y in request.roi.y0..request.roi.y1 {
        budget.poll()?;
        for x in request.roi.x0..request.roi.x1 {
            if x.abs_diff(request.roi.x0).is_multiple_of(256) {
                budget.poll()?;
            }
            let mut samples = [0.0; 4];
            source.try_read_pixel_f64(x, y, samples.get_mut(..format.channels()).ok_or(GraphExecuteError::Domain)?).map_err(GraphExecuteError::Raster)?;
            if samples.iter().take(format.channels()).any(|value| !value.is_finite())
                || format.alpha && samples.get(colors).is_none_or(|alpha| !(0.0..=1.0).contains(alpha))
            {
                return Err(GraphExecuteError::Operation("point graph requires finite colors and finite unit alpha"));
            }
            let mut result = [0.0; 4];
            prepared
                .try_eval_pixel(
                    samples.get(..evaluated).ok_or(GraphExecuteError::Domain)?,
                    colors,
                    result.get_mut(..evaluated).ok_or(GraphExecuteError::Domain)?,
                )
                .map_err(GraphExecuteError::Raster)?;
            if result.iter().take(evaluated).any(|value| !value.is_finite() || format.sample == SampleType::F32 && !(*value as f32).is_finite()) {
                return Err(GraphExecuteError::Operation("point graph output is outside finite native sample domain"));
            }
        }
    }
    budget.reserve_index(index)?;
    budget.reserve_output(output_charge)?;
    budget.reserve_scratch(scratch)?;
    let resident = output_charge.checked_add(index).and_then(|n| n.checked_add(scratch)).ok_or(GraphExecuteError::ResidentLimit)?.min(MAX_CHECKED_RASTER_BYTES);
    let output = {
        let cancelled = || budget.poll().is_err();
        point::try_apply(source, request.roi, binding.operation, resident, work, &Interrupt::cancel_only(&cancelled)).map_err(GraphExecuteError::Raster)?
    };
    budget.finish_output(output, request.roi, request.domain)
}

fn validate_operation(binding: &Binding<'_>, node: &photocraft_doc::xcf_filters::GraphNode) -> Result<(), GraphExecuteError> {
    // These retained mixer kernels consume unassociated Linear RGB Float.
    // The fingerprint is caller-established identity, never an inferred sRGB
    // alias or permission to convert Gray/native-wide input here.
    if matches!(binding.operation, PointOperation::ChannelMixer { .. } | PointOperation::MonoMixerSigned { .. })
        && (binding.domain.format.mode != ColorMode::Rgb
            || binding.domain.format.sample != SampleType::F32
            || !binding.domain.format.alpha
            || binding.domain.encoding != SampleEncoding::Linear
            || binding.domain.profile_fingerprint.is_none_or(|value| value == [0; 32]))
    {
        return Err(GraphExecuteError::Operation("mixer graph requires explicit profiled Linear RGB Float with alpha"));
    }
    let rgb_only = matches!(
        binding.operation,
        PointOperation::ValueInvert
            | PointOperation::Sepia { .. }
            | PointOperation::Saturation { .. }
            | PointOperation::Desaturate { .. }
            | PointOperation::MonoMixer { .. }
            | PointOperation::RgbClip { .. }
            | PointOperation::ColorTemperature { .. }
    );
    if rgb_only && binding.domain.format.mode != ColorMode::Rgb {
        return Err(GraphExecuteError::Operation("coupled point family requires explicit RGB working samples"));
    }
    // Native Gray/integer temperature has a separate qualified policy. Do not
    // silently substitute the generic wide RGB equation for that domain.
    if matches!(binding.operation, PointOperation::ColorTemperature { .. }) && !matches!(binding.domain.format.sample, SampleType::F32 | SampleType::F64) {
        return Err(GraphExecuteError::Operation("temperature graph adapter requires floating RGB samples"));
    }
    if matches!(binding.operation, PointOperation::ThresholdAlpha { .. }) && !binding.domain.format.alpha {
        return Err(GraphExecuteError::Operation("alpha threshold graph requires a native alpha channel"));
    }
    match binding.operation {
        PointOperation::ColorExchange { from, to, thresholds } => return color_exchange_binding::validate(binding, node, from, to, thresholds),
        PointOperation::RedEyeRemoval { threshold } => return red_eye_binding::validate(binding, node, threshold),
        PointOperation::ChannelMixer { gains, preserve_luminosity } => {
            let [[rr, rg, rb], [gr, gg, gb], [br, bg, bb]] = gains;
            return validate_values(
                binding, node, "gegl:channel-mixer", "0:0", SampleEncoding::Linear,
                &[("rr-gain", SourceValue::Float(rr)), ("rg-gain", SourceValue::Float(rg)), ("rb-gain", SourceValue::Float(rb)),
                  ("gr-gain", SourceValue::Float(gr)), ("gg-gain", SourceValue::Float(gg)), ("gb-gain", SourceValue::Float(gb)),
                  ("br-gain", SourceValue::Float(br)), ("bg-gain", SourceValue::Float(bg)), ("bb-gain", SourceValue::Float(bb)),
                  ("preserve-luminosity", SourceValue::Bool(preserve_luminosity))],
            );
        }
        PointOperation::MonoMixerSigned { red, green, blue, preserve_luminosity } => {
            return validate_values(
                binding, node, "gegl:mono-mixer", "0:0", SampleEncoding::Linear,
                &[("red", SourceValue::Float(red)), ("green", SourceValue::Float(green)), ("blue", SourceValue::Float(blue)),
                  ("preserve-luminosity", SourceValue::Bool(preserve_luminosity))],
            );
        }
        PointOperation::ValueInvert => return validate_values(binding, node, "gegl:value-invert", "0:0", SampleEncoding::NonlinearProfile, &[]),
        PointOperation::Posterize { levels } => {
            return validate_values(
                binding,
                node,
                "gimp:posterize",
                "0:0",
                SampleEncoding::NonlinearProfile,
                &[("levels", SourceValue::Signed32(i32::from(levels)))],
            );
        }
        PointOperation::Sepia { scale, srgb } => {
            return validate_values(
                binding,
                node,
                "gegl:sepia",
                "0:0",
                if srgb { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear },
                &[("scale", SourceValue::Float(scale)), ("srgb", SourceValue::Bool(srgb))],
            );
        }
        PointOperation::Saturation { scale, colorspace } => {
            return validate_values(
                binding,
                node,
                "gegl:saturation",
                "0:0",
                SampleEncoding::Linear,
                &[("scale", SourceValue::Float(scale)), ("colorspace", SourceValue::Enum("GeglSaturationType", i32::from(colorspace)))],
            );
        }
        PointOperation::Desaturate { mode } => {
            return validate_values(
                binding,
                node,
                "gimp:desaturate",
                "0:0",
                if mode == 3 { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile },
                &[("mode", SourceValue::Enum("GimpDesaturateMode", i32::from(mode)))],
            );
        }
        PointOperation::MonoMixer { red, green, blue } => {
            return validate_values(
                binding,
                node,
                "gegl:mono-mixer",
                "0:0",
                SampleEncoding::Linear,
                &[
                    ("red", SourceValue::Float(red)),
                    ("green", SourceValue::Float(green)),
                    ("blue", SourceValue::Float(blue)),
                    ("preserve-luminosity", SourceValue::Bool(false)),
                ],
            );
        }
        PointOperation::RgbClip { clip_low, low_limit, clip_high, high_limit } => {
            return validate_values(
                binding,
                node,
                "gegl:rgb-clip",
                "0:0",
                SampleEncoding::Linear,
                &[
                    ("clip-low", SourceValue::Bool(clip_low)),
                    ("low-limit", SourceValue::Float(low_limit)),
                    ("clip-high", SourceValue::Bool(clip_high)),
                    ("high-limit", SourceValue::Float(high_limit)),
                ],
            );
        }
        PointOperation::ColorTemperature { original, intended } => {
            return validate_scalars(
                binding,
                node,
                "gegl:color-temperature",
                "0:0",
                SampleEncoding::Linear,
                &[("original-temperature", original), ("intended-temperature", intended)],
            );
        }
        PointOperation::ThresholdAlpha { value } => {
            return validate_scalars(binding, node, "gimp:threshold-alpha", "0:0", SampleEncoding::Linear, &[("value", value)]);
        }
        PointOperation::Threshold { channel, low, high } if channel != 4 => {
            return validate_values(
                binding,
                node,
                "gimp:threshold",
                "0:0",
                SampleEncoding::NonlinearProfile,
                &[
                    ("channel", SourceValue::Enum("GimpHistogramChannel", i32::from(channel))),
                    ("low", SourceValue::Float(low)),
                    ("high", SourceValue::Float(high)),
                ],
            );
        }
        _ => {}
    }
    let (name, version, encoding, parameters): (&str, &str, SampleEncoding, &[(&str, f64)]) = match binding.operation {
        PointOperation::BrightnessContrast { brightness, contrast } => {
            return validate_scalars(
                binding,
                node,
                "gegl:brightness-contrast",
                "0:0",
                SampleEncoding::Linear,
                &[("brightness", brightness), ("contrast", contrast)],
            );
        }
        PointOperation::Exposure { stops, black_level } => {
            return validate_scalars(binding, node, "gegl:exposure", "1:0", SampleEncoding::Linear, &[("exposure", stops), ("black-level", black_level)]);
        }
        PointOperation::Levels { input_low, input_high, output_low, output_high } => {
            return validate_scalars(
                binding,
                node,
                "gegl:levels",
                "0:0",
                SampleEncoding::Linear,
                &[("in-low", input_low), ("in-high", input_high), ("out-low", output_low), ("out-high", output_high)],
            );
        }
        PointOperation::Invert if binding.identity.name == "gegl:invert-linear" => ("gegl:invert-linear", "0:0", SampleEncoding::Linear, &[]),
        PointOperation::Invert if binding.identity.name == "gegl:invert-gamma" => ("gegl:invert-gamma", "0:0", SampleEncoding::NonlinearProfile, &[]),
        _ => return Err(GraphExecuteError::Operation("point family requires a separate qualified alpha/geometry/domain adapter")),
    };
    validate_scalars(binding, node, name, version, encoding, parameters)
}

fn validate_scalars(
    binding: &Binding<'_>,
    node: &photocraft_doc::xcf_filters::GraphNode,
    name: &str,
    version: &str,
    encoding: SampleEncoding,
    parameters: &[(&str, f64)],
) -> Result<(), GraphExecuteError> {
    let mut values = [("", SourceValue::Float(0.0)); 4];
    if parameters.len() > values.len() {
        return Err(GraphExecuteError::Operation("point scalar count is unsupported"));
    }
    for (destination, (name, value)) in values.iter_mut().zip(parameters) {
        *destination = (*name, SourceValue::Float(*value));
    }
    validate_values(binding, node, name, version, encoding, values.get(..parameters.len()).ok_or(GraphExecuteError::InvalidPlan)?)
}

#[derive(Clone, Copy)]
enum SourceValue<'a> {
    Float(f64),
    Bool(bool),
    Signed32(i32),
    Enum(&'a str, i32),
}

fn validate_values(
    binding: &Binding<'_>,
    node: &photocraft_doc::xcf_filters::GraphNode,
    name: &str,
    version: &str,
    encoding: SampleEncoding,
    parameters: &[(&str, SourceValue<'_>)],
) -> Result<(), GraphExecuteError> {
    if binding.identity.name != name
        || binding.identity.stored_version != version
        || binding.domain.encoding != encoding
        || node.arguments.len() != parameters.len()
    {
        return Err(GraphExecuteError::Operation("point identity/version/encoding/configuration does not match typed operation"));
    }
    let mut seen = [false; 10];
    for argument in &node.arguments {
        let index = parameters.iter().position(|(name, _)| *name == argument.name).ok_or(GraphExecuteError::Operation("unknown point scalar argument"))?;
        let (name, expected) = parameters.get(index).ok_or(GraphExecuteError::InvalidPlan)?;
        let slot = seen.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
        if std::mem::replace(slot, true) || argument.name != *name {
            return Err(GraphExecuteError::Operation("duplicate point scalar argument"));
        }
        let matches = match (&argument.value, expected) {
            (FilterValue::Number(ExactNumber::Float64Bits(bits)), SourceValue::Float(value)) => *bits == value.to_bits(),
            (FilterValue::Number(ExactNumber::Float32Bits(bits)), SourceValue::Float(value)) => f64::from(f32::from_bits(*bits)).to_bits() == value.to_bits(),
            (FilterValue::Bool(value), SourceValue::Bool(expected)) => value == expected,
            (FilterValue::Number(ExactNumber::Signed { width: 32, value }), SourceValue::Signed32(expected)) => *value == i64::from(*expected),
            (FilterValue::Enum { domain, value }, SourceValue::Enum(expected_domain, expected_value)) => {
                domain.as_str() == *expected_domain && value == expected_value
            }
            _ => false,
        };
        if !matches {
            return Err(GraphExecuteError::Operation("point requires exact typed source scalar/Boolean/enum values"));
        }
    }
    Ok(())
}

fn tile_count(roi: Rect) -> Result<u64, GraphExecuteError> {
    if roi.is_empty() {
        return Err(GraphExecuteError::Roi);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last = TileCoord::containing(roi.x1.checked_sub(1).ok_or(GraphExecuteError::Roi)?, roi.y1.checked_sub(1).ok_or(GraphExecuteError::Roi)?);
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| GraphExecuteError::Roi)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| GraphExecuteError::Roi)?;
    width.checked_mul(height).ok_or(GraphExecuteError::ResidentLimit)
}

#[cfg(test)]
#[path = "point_graph/mixer_binding_tests.rs"]
mod mixer_binding_tests;

#[cfg(test)]
mod tests {
    use super::super::{
        graph_execute::{ExecutionLimits, execute_graph},
        graph_plan::plan_graph,
    };
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::xcf_filters::{FilterGraph, GraphEdge, GraphNode};
    use photocraft_raster::Surface;

    fn graph() -> FilterGraph {
        let node = |id| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "gegl:invert-linear".into(), stored_version: "0:0".into(), schema_epoch: "own-explicit-linear-test".into() },
            version_origin: GraphVersionOrigin::Stored,
            arguments: Vec::new(),
        };
        let edge = |from, to| GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), input_port: "input".into(), output_port: "output".into() };
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(1), node(2)],
            edges: vec![edge(9, 1), edge(1, 2)],
            output: GraphNodeId(2),
            original_pipeline: None,
        }
    }

    #[test]
    fn two_explicit_inverts_preserve_native_alpha_and_source_deterministically() {
        let graph = graph();
        let plan = plan_graph(&graph, 2, &Interrupt::NONE, |_, direction, port| accepts_port(direction, port)).unwrap();
        let roi = Rect::new(0, 0, 2, 1);
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut input = Surface::new(format);
            let mut encoded = vec![0; checked_region_byte_len(roi, format).unwrap()];
            for (pixel, values) in encoded.chunks_exact_mut(format.bytes_per_pixel()).zip([[0.0, 1.0, 0.0, 0.5], [1.0, 0.0, 1.0, -0.0]]) {
                for (channel, value) in values.into_iter().enumerate() {
                    photocraft_color::try_write_sample_f64(pixel, sample, channel, value).unwrap();
                }
            }
            input.try_write_interleaved_with(roi, &encoded, &Interrupt::NONE).unwrap();
            let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: None };
            for _ in 0..2 {
                let output = execute_graph(&plan, Some(&input), roi, domain, ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
                    let binding = Binding {
                        node: request.node.id,
                        identity: &request.node.operation,
                        version_origin: request.node.version_origin,
                        domain,
                        full_extent: roi,
                        operation: PointOperation::Invert,
                    };
                    evaluate(binding, request, budget)
                })
                .unwrap();
                assert_eq!(output.surface().try_read_interleaved_with(roi, &Interrupt::NONE).unwrap(), encoded);
                assert_eq!(input.try_read_interleaved_with(roi, &Interrupt::NONE).unwrap(), encoded);
            }
        }
    }

    #[test]
    fn cancellation_after_private_node_and_low_budget_leave_source_unchanged() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let graph = graph();
        let plan = plan_graph(&graph, 2, &Interrupt::NONE, |_, direction, port| accepts_port(direction, port)).unwrap();
        let roi = Rect::new(0, 0, 1, 1);
        let raw: Vec<_> = [0.25_f32, 0.5, 0.75, 0.5].into_iter().flat_map(f32::to_ne_bytes).collect();
        let input = Surface::try_with_default_bytes(PixelFormat::RGBA32F, &raw).unwrap();
        let domain = ExecutionDomain { format: input.format(), encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let cancelled = AtomicBool::new(false);
        let cancel = || cancelled.load(Ordering::Relaxed);
        let result = execute_graph(&plan, Some(&input), roi, domain, ExecutionLimits::default(), &Interrupt::cancel_only(&cancel), |request, budget| {
            let binding = Binding {
                node: request.node.id,
                identity: &request.node.operation,
                version_origin: request.node.version_origin,
                domain,
                full_extent: roi,
                operation: PointOperation::Invert,
            };
            let result = evaluate(binding, request, budget)?;
            cancelled.store(true, Ordering::Relaxed);
            Ok(result)
        });
        assert!(matches!(result, Err(GraphExecuteError::Cancelled)));
        let result =
            execute_graph(&plan, Some(&input), roi, domain, ExecutionLimits { work_units: 1, ..Default::default() }, &Interrupt::NONE, |request, budget| {
                let binding = Binding {
                    node: request.node.id,
                    identity: &request.node.operation,
                    version_origin: request.node.version_origin,
                    domain,
                    full_extent: roi,
                    operation: PointOperation::Invert,
                };
                evaluate(binding, request, budget)
            });
        assert!(matches!(result, Err(GraphExecuteError::WorkLimit)));
        assert_eq!(input.default_bytes(), raw);
        assert_eq!(input.tile_count(), 0);
    }

    #[test]
    fn exact_typed_boolean_enum_and_float32_source_bindings_refuse_aliases() {
        use photocraft_doc::xcf_filters::FilterArgument;
        let mut node = GraphNode {
            id: GraphNodeId(1),
            operation: OperationIdentity { name: "gegl:sepia".into(), stored_version: "0:0".into(), schema_epoch: "own-explicit-sepia-test".into() },
            version_origin: GraphVersionOrigin::Stored,
            arguments: vec![
                FilterArgument { name: "scale".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5_f32.to_bits())) },
                FilterArgument { name: "srgb".into(), value: FilterValue::Bool(true) },
            ],
        };
        let domain = ExecutionDomain { format: PixelFormat::RGBA32F, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        {
            let binding = Binding {
                node: node.id,
                identity: &node.operation,
                version_origin: node.version_origin,
                domain,
                full_extent: Rect::new(0, 0, 1, 1),
                operation: PointOperation::Sepia { scale: 0.5, srgb: true },
            };
            validate_operation(&binding, &node).unwrap();
        }
        node.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(1.0_f64.to_bits()));
        let binding = Binding {
            node: node.id,
            identity: &node.operation,
            version_origin: node.version_origin,
            domain,
            full_extent: Rect::new(0, 0, 1, 1),
            operation: PointOperation::Sepia { scale: 0.5, srgb: true },
        };
        assert!(validate_operation(&binding, &node).is_err());
    }

    #[test]
    fn alpha_threshold_graph_preserves_color_words_and_source() {
        use photocraft_doc::xcf_filters::FilterArgument;
        let mut graph = graph();
        for node in &mut graph.nodes {
            node.operation.name = "gimp:threshold-alpha".into();
            node.arguments = vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5_f32.to_bits())) }];
        }
        let plan = plan_graph(&graph, 2, &Interrupt::NONE, |_, direction, port| accepts_port(direction, port)).unwrap();
        let raw: Vec<_> = [(-0.0_f32), 0.5, 0.75, 0.25].into_iter().flat_map(f32::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA32F, &raw).unwrap();
        let roi = Rect::new(0, 0, 1, 1);
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let output = execute_graph(&plan, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
            let binding = Binding {
                node: request.node.id,
                identity: &request.node.operation,
                version_origin: request.node.version_origin,
                domain,
                full_extent: roi,
                operation: PointOperation::ThresholdAlpha { value: 0.5 },
            };
            evaluate(binding, request, budget)
        })
        .unwrap();
        let actual = output.surface().try_read_interleaved_with(roi, &Interrupt::NONE).unwrap();
        assert_eq!(&actual[..12], &raw[..12]);
        assert_eq!(&actual[12..], &0.0_f32.to_ne_bytes());
        assert_eq!(source.default_bytes(), raw);
        assert_eq!(source.tile_count(), 0);
    }
}
