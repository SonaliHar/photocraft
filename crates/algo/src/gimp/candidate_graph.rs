//! Required-caller-schema bridge for fixed-domain standalone candidates.
//! No GraphNode is decoded/admitted automatically. The caller binds an exact
//! schema/argument interpretation to its prepared Candidate and CMS domain.
use super::{
    candidate_dispatch::{Candidate, ExecutionDomain, Kind, Limits},
    edge_sobel, generator_checkerboard, global_stretch_contrast,
    graph_execute::{GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};

/// Required planning port predicate, alongside the caller's exact schema check.
/// The executor's NodeRequest has incoming bindings only; outgoing declarations
/// must be validated with this predicate when constructing its structural plan.
pub fn accepts_port(candidate: &Candidate, direction: super::graph_plan::PortDirection, port: &str) -> bool {
    match direction {
        super::graph_plan::PortDirection::Output => port == "output",
        super::graph_plan::PortDirection::Input => candidate.kind() != Kind::Checkerboard && port == "input",
    }
}

/// Independently validated by the caller against its public/observed schema.
/// `arguments` must be the exact source values from which the caller prepared
/// the Candidate (including any explicit CMS conversion of named colors).
/// Echoing an arbitrary node's values is not a qualified schema resolution.
pub struct CallerSchema<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub arguments: &'a [FilterArgument],
}

pub enum KernelDomain {
    Checkerboard,
    Sobel { full_extent: Rect, transfer: edge_sobel::SobelTransfer, abyss: edge_sobel::SobelAbyss },
    StretchContrast { full_extent: Rect },
}

/// A same-ROI bridge, deliberately refusing larger global/halo extents. The
/// executor retains upstream owners; their already-debited capacity is never
/// returned or silently reused. This cooperative ledger does not measure RSS.
pub fn evaluate(
    candidate: &Candidate,
    schema: CallerSchema<'_>,
    domain: KernelDomain,
    request: NodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    let (operation, version) = candidate.identity();
    if schema.node.0 == 0
        || schema.node != request.node.id
        || schema.identity.name != operation
        || schema.identity.stored_version != version
        || schema.identity.schema_epoch.is_empty()
        || schema.identity.schema_epoch.len() > 256
        || schema.identity != &request.node.operation
        || schema.version_origin != request.node.version_origin
    {
        return Err(GraphExecuteError::Operation("candidate requires an exact caller-bound node schema"));
    }
    validate_arguments(candidate, schema.arguments)?;
    validate_arguments(candidate, &request.node.arguments)?;
    if schema.arguments != request.node.arguments.as_slice() {
        return Err(GraphExecuteError::Operation("candidate source arguments differ from caller schema"));
    }
    let source = match candidate.kind() {
        Kind::Checkerboard if request.inputs.is_empty() => None,
        Kind::Sobel | Kind::StretchContrast if request.inputs.len() == 1 => {
            let binding = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
            if binding.domain != request.domain { return Err(GraphExecuteError::Domain); }
            if binding.input_port != "input" || binding.output_port != "output" {
                return Err(GraphExecuteError::Operation("candidate requires exact input/output ports; auxiliary inputs are unsupported"));
            }
            Some(binding.surface)
        }
        _ => return Err(GraphExecuteError::Operation("candidate input-port count is unsupported")),
    };
    let execution = match (candidate.kind(), domain, source) {
        (Kind::Checkerboard, KernelDomain::Checkerboard, None) => {
            ExecutionDomain::Checkerboard { format: request.domain.format, encoding: request.domain.encoding }
        }
        (Kind::Sobel, KernelDomain::Sobel { full_extent, transfer, abyss }, Some(source)) => {
            if full_extent != request.roi {
                return Err(GraphExecuteError::Operation("fixed-ROI candidate bridge does not provide a larger Sobel input extent"));
            }
            let encoding = match transfer {
                edge_sobel::SobelTransfer::Linear => SampleEncoding::Linear,
                edge_sobel::SobelTransfer::Nonlinear => SampleEncoding::NonlinearProfile,
            };
            if request.domain.encoding != encoding {
                return Err(GraphExecuteError::Domain);
            }
            ExecutionDomain::Sobel { input: edge_sobel::SobelWorkingInput { source, extent: full_extent, transfer }, abyss }
        }
        (Kind::StretchContrast, KernelDomain::StretchContrast { full_extent }, Some(source)) => {
            if full_extent != request.roi {
                return Err(GraphExecuteError::Operation("fixed-ROI candidate bridge cannot reduce a larger authoritative global source extent"));
            }
            if request.domain.encoding != SampleEncoding::Linear {
                return Err(GraphExecuteError::Domain);
            }
            ExecutionDomain::StretchContrast { source, full_extent, encoding: request.domain.encoding }
        }
        _ => return Err(GraphExecuteError::Operation("candidate execution domain does not match its prepared configuration")),
    };
    let format = request.domain.format;
    candidate.validate_domain(&execution, request.roi).map_err(GraphExecuteError::Raster)?;
    if source.is_some_and(|source| source.format() != format) {
        return Err(GraphExecuteError::Domain);
    }
    let tiles = tile_count(request.roi)?;
    let bpp = format.bytes_per_pixel() as u64;
    let tile_bytes = (TILE_SIZE as u64).checked_pow(2).and_then(|n| n.checked_mul(bpp)).ok_or(GraphExecuteError::ResidentLimit)?;
    let output = tiles
        .checked_mul(tile_bytes.checked_add(256).ok_or(GraphExecuteError::ResidentLimit)?)
        .and_then(|n| n.checked_add(256 + bpp))
        .ok_or(GraphExecuteError::ResidentLimit)?;
    let source_tiles = source.map_or(0, |source| source.tile_count() as u64);
    let remaining = budget.remaining();
    if source_tiles > tiles || source_tiles > remaining.work_units {
        return Err(GraphExecuteError::Operation("candidate source retains unrelated fixed-ROI tiles"));
    }
    // Bounded metadata inspection precedes any source clone/output allocation.
    budget.charge_work(source_tiles)?;
    if let Some(source) = source {
        for (tile, _) in source.tiles() {
            budget.poll()?;
            if tile.rect().intersect(&request.roi).is_empty() {
                return Err(GraphExecuteError::Operation("candidate source retains outside-ROI tiles"));
            }
        }
    }
    let source_charge = source_tiles.checked_mul(tile_bytes + 256).and_then(|n| n.checked_add(256 + bpp)).ok_or(GraphExecuteError::ResidentLimit)?;
    let index = source_tiles.checked_add(tiles).and_then(|n| n.checked_mul(512)).and_then(|n| n.checked_add(1024)).ok_or(GraphExecuteError::IndexLimit)?;
    let scratch = source_charge
        .checked_mul(2)
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(4096))
        .ok_or(GraphExecuteError::ResidentLimit)?;
    let pixels = request.roi.size().area();
    let work = match candidate.kind() {
        Kind::Checkerboard => pixels
            .checked_mul(bpp)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(pixels.checked_mul(16)?))
            .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(256)?)),
        Kind::Sobel => Some(edge_sobel::required_work(source.ok_or(GraphExecuteError::MissingInput)?, request.roi).map_err(GraphExecuteError::Raster)?),
        Kind::StretchContrast => Some(
            global_stretch_contrast::required_work(source.ok_or(GraphExecuteError::MissingInput)?, request.roi, request.roi)
                .map_err(GraphExecuteError::Raster)?,
        ),
    }
    .ok_or(GraphExecuteError::WorkLimit)?;
    // Reserve every callback-owned category before invoking allocating kernels.
    // Output/index overlap is conservatively charged twice, never credited back.
    budget.reserve_index(index)?;
    budget.reserve_output(output)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(work)?;
    let kernel_resident = output.checked_add(scratch).and_then(|n| n.checked_add(index)).ok_or(GraphExecuteError::ResidentLimit)?;
    let kernel_resident = kernel_resident.min(MAX_CHECKED_RASTER_BYTES);
    let limits = match candidate.kind() {
        Kind::Checkerboard => Limits::Checkerboard(generator_checkerboard::Limits { resident_bytes: kernel_resident, work_units: work }),
        Kind::Sobel => Limits::Sobel(edge_sobel::SobelLimits { resident_bytes: kernel_resident, work_units: work }),
        Kind::StretchContrast => Limits::StretchContrast(global_stretch_contrast::GlobalLimits { resident_bytes: kernel_resident, work_units: work }),
    };
    // Use the ledger's actual cancellation token, not an unrelated caller token.
    let surface = {
        let cancelled = || budget.poll().is_err();
        candidate.execute(execution, request.roi, limits, &Interrupt::cancel_only(&cancelled)).map_err(GraphExecuteError::Raster)?
    };
    budget.finish_output(surface, request.roi, request.domain)
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

/// No recursive/opaque values are compared. This bounds the exact source-value
/// comparison; faithful typed/CMS preparation remains caller schema ownership.
fn validate_arguments(candidate: &Candidate, arguments: &[FilterArgument]) -> Result<(), GraphExecuteError> {
    let kind = candidate.kind();
    let names: &[&str] = match kind {
        Kind::Checkerboard => &["x", "y", "x-offset", "y-offset", "color1", "color2"],
        Kind::Sobel => &["horizontal", "vertical", "keep-sign"],
        Kind::StretchContrast => &["keep-colors", "perceptual"],
    };
    if arguments.len() != names.len() {
        return Err(GraphExecuteError::Operation("candidate requires exact typed named source arguments"));
    }
    let mut seen = [false; 6];
    for argument in arguments {
        let index = names.iter().position(|name| *name == argument.name).ok_or(GraphExecuteError::Operation("unknown candidate source argument"))?;
        let slot = seen.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
        if std::mem::replace(slot, true) {
            return Err(GraphExecuteError::Operation("duplicate candidate source argument"));
        }
        if matches!(&argument.value, FilterValue::StoredColor(_)) { return Err(GraphExecuteError::Operation("StoredColor candidate evaluation is unsupported")); }
        let valid = match (kind, argument.name.as_str(), &argument.value) {
            (Kind::Sobel | Kind::StretchContrast, name, FilterValue::Bool(value)) => candidate.matches_source_boolean(name, *value),
            (Kind::Checkerboard, name @ ("x" | "y" | "x-offset" | "y-offset"), FilterValue::Number(ExactNumber::Signed { width: 32, value })) => {
                candidate.matches_source_integer(name, *value)
            }
            (Kind::Checkerboard, "color1" | "color2", FilterValue::Color { components, .. }) if components.len() == 4 => {
                components.iter().all(|number| match number {
                    ExactNumber::Float32Bits(bits) => f32::from_bits(*bits).is_finite(),
                    ExactNumber::Float64Bits(bits) => f64::from_bits(*bits).is_finite(),
                    _ => false,
                })
            }
            _ => false,
        };
        if !valid {
            return Err(GraphExecuteError::Operation("opaque or unsupported candidate source value type"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{candidate_dispatch::Configuration, global_stretch_contrast::BoolArgument, graph_plan::PortDirection};
    use super::*;

    fn stretch() -> Candidate {
        Candidate::from_named(
            "gegl:stretch-contrast",
            "0:0",
            Configuration::StretchContrast {
                arguments: &[BoolArgument { name: "keep-colors", value: false }, BoolArgument { name: "perceptual", value: false }],
            },
        )
        .unwrap()
    }

    #[test]
    fn ports_and_source_scalars_must_match_the_prepared_candidate() {
        let candidate = stretch();
        assert!(accepts_port(&candidate, PortDirection::Input, "input"));
        assert!(accepts_port(&candidate, PortDirection::Output, "output"));
        for port in ["aux", "input ", "Output", ""] {
            assert!(!accepts_port(&candidate, PortDirection::Input, port));
            assert!(!accepts_port(&candidate, PortDirection::Output, port));
        }
        let mut arguments = vec![
            FilterArgument { name: "keep-colors".into(), value: FilterValue::Bool(false) },
            FilterArgument { name: "perceptual".into(), value: FilterValue::Bool(false) },
        ];
        validate_arguments(&candidate, &arguments).unwrap();
        arguments[0].value = FilterValue::Bool(true);
        assert!(validate_arguments(&candidate, &arguments).is_err());
        arguments[0].value = FilterValue::Config(Vec::new());
        assert!(validate_arguments(&candidate, &arguments).is_err());
        arguments[0].value = FilterValue::Bool(false);
        arguments[1].name = "keep-colors".into();
        assert!(validate_arguments(&candidate, &arguments).is_err());
    }
}
