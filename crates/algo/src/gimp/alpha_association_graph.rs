//! Exact caller-schema graph adapter for the existing native alpha kernel.
//! No metadata-only admission, fallback pixels, CMS or source encoding guesses.
use super::{
    graph_execute::{self, AvailableNodeRequest, GraphExecuteError as Error, InputPixels, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    point_alpha_association as kernel,
    wide_point::WorkingContext,
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{FilterArgument, FilterValue, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use std::sync::Arc;
type Result<T> = std::result::Result<T, Error>;
pub type ArgumentValidator = fn(&[FilterArgument], kernel::Params) -> Result<()>;
pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub arguments: &'a [FilterArgument],
    pub validate_arguments: ArgumentValidator,
    pub endpoint: Endpoint,
    pub extent: Rect,
    pub context: WorkingContext,
    pub params: kernel::Params,
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => port == "source",
        PortDirection::Output => port == "output",
    }
}
pub(super) fn bounded(arguments: &[FilterArgument]) -> Result<()> {
    if arguments.len() > 16 {
        return Err(Error::Operation("alpha/coverage graph argument count"));
    }
    let mut bytes = 0usize;
    for (index, a) in arguments.iter().enumerate() {
        if a.name.is_empty() || a.name.len() > 128 || a.name.contains('\0') || arguments.iter().take(index).any(|p| p.name == a.name) {
            return Err(Error::Operation("alpha/coverage graph unique names"));
        }
        let n = match &a.value {
            FilterValue::Bool(_) | FilterValue::Number(_) => 32,
            FilterValue::Enum { domain, .. } if !domain.is_empty() && domain.len() <= 256 && !domain.contains('\0') => domain.len(),
            FilterValue::String(s) if s.len() <= 4096 && !s.contains('\0') => s.len(),
            _ => return Err(Error::Operation("alpha/coverage graph requires flat typed arguments")),
        };
        bytes = bytes
            .checked_add(a.name.len())
            .and_then(|v| v.checked_add(n))
            .filter(|v| *v <= 16 << 10)
            .ok_or(Error::Operation("alpha/coverage graph argument bytes"))?;
    }
    Ok(())
}
pub(super) fn schema(
    node: GraphNodeId,
    identity: &OperationIdentity,
    origin: GraphVersionOrigin,
    args: &[FilterArgument],
    request: &NodeRequest<'_>,
    name: &str,
) -> Result<()> {
    for id in [identity, &request.node.operation] {
        if id.name.is_empty()
            || id.name.len() > 256
            || id.stored_version.is_empty()
            || id.stored_version.len() > 64
            || id.schema_epoch.is_empty()
            || id.schema_epoch.len() > 256
            || [&id.name, &id.stored_version, &id.schema_epoch].iter().any(|s| s.contains('\0'))
        {
            return Err(Error::Operation("alpha/coverage graph identity"));
        }
    }
    if node.0 == 0
        || node != request.node.id
        || identity != &request.node.operation
        || identity.name != name
        || identity.stored_version != "1"
        || origin != request.node.version_origin
    {
        return Err(Error::Operation("alpha/coverage graph exact caller schema/version"));
    }
    bounded(args)?;
    bounded(&request.node.arguments)?;
    if args != request.node.arguments.as_slice() {
        return Err(Error::Operation("alpha/coverage graph captured arguments changed"));
    }
    Ok(())
}
pub(super) fn endpoint_valid(e: Endpoint) -> bool {
    match e {
        Endpoint::ExternalInput(id) => id.0 != 0,
        Endpoint::Node { id, index } => id.0 != 0 && index < 4096,
    }
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(Error::ResidentLimit)
}
/// Conservative kernel-matching envelope. Sources remain charged upstream too;
/// no refund or process-RSS claim. No sample math is duplicated here.
pub(super) fn precharge(sources: &[&Surface], roi: Rect, factor: u64, budget: &mut NodeBudget<'_, '_>) -> Result<(u64, u64)> {
    budget.poll()?;
    let first = *sources.first().ok_or(Error::MissingInput)?;
    let format = first.format();
    photocraft_raster::checked_region_byte_len(roi, format).map_err(Error::Raster)?;
    if roi.is_empty() {
        return Err(Error::Roi);
    }
    let nx = u64::try_from(i64::from(roi.x1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(roi.x0).div_euclid(i64::from(TILE_SIZE)) + 1)
        .map_err(|_| Error::Roi)?;
    let ny = u64::try_from(i64::from(roi.y1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(roi.y0).div_euclid(i64::from(TILE_SIZE)) + 1)
        .map_err(|_| Error::Roi)?;
    let tiles = mul(nx, ny)?;
    let payload = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, format.bytes_per_pixel() as u64)?;
    let mut retained = 0u64;
    let mut defaults = 0u64;
    for s in sources {
        budget.poll()?;
        if s.format() != format {
            return Err(Error::Domain);
        }
        retained = add(retained, s.tile_count() as u64)?;
        defaults = add(defaults, s.default_bytes().len() as u64)?;
    }
    let source_bytes = add(mul(retained, payload)?, defaults)?;
    let staging = add(mul(payload, 4)?, 8192)?;
    let maps = mul(add(retained, tiles)?, 1024)?;
    let output = add(add(256, format.bytes_per_pixel() as u64)?, mul(tiles, add(payload, 256)?)?)?;
    let extra = add(add(mul(tiles, add(payload, 512)?)?, maps)?, add(staging, add(payload, mul(4096, format.bytes_per_pixel() as u64)?)?)?)?;
    let lower_resident = add(source_bytes, extra)?;
    let lower_work = add(mul(tiles, mul(payload, factor)?)?, add(mul(retained, 32)?, mul(tiles, mul(add(retained, tiles)?, 512)?)?)?)?;
    let trim = mul(add(retained, tiles)?, 256)?;
    budget.charge_work(add(lower_work, add(mul(retained, 128)?, trim)?)?)?;
    let index = add(add(maps, mul(tiles, 1024)?)?, 1024)?;
    let scratch = add(source_bytes, add(staging, add(payload, mul(4096, format.bytes_per_pixel() as u64)?)?)?)?;
    let aggregate = add(output, add(index, scratch)?)?;
    let remaining = budget.remaining();
    if lower_resident > MAX_CHECKED_RASTER_BYTES || aggregate > remaining.resident_bytes {
        return Err(Error::ResidentLimit);
    }
    if output > remaining.cache_bytes {
        return Err(Error::CacheLimit);
    }
    if index > remaining.index_bytes {
        return Err(Error::IndexLimit);
    }
    budget.reserve_index(index)?;
    budget.reserve_output(output)?;
    budget.reserve_scratch(scratch)?;
    Ok((lower_resident, lower_work))
}
pub(super) fn finish(staged: Surface, request: &NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    let mut trimmed = Surface::try_with_default_bytes(staged.format(), staged.default_bytes()).map_err(Error::Raster)?;
    for (coordinate, tile) in staged.tiles() {
        budget.poll()?;
        if !coordinate.rect().intersect(&request.roi).is_empty() {
            trimmed.put_tiles(std::iter::once((*coordinate, Arc::clone(tile))));
        }
    }
    drop(staged);
    budget.poll()?;
    budget.finish_output(trimmed, request.roi, request.domain)
}
fn error(e: kernel::Error) -> Error {
    match e {
        kernel::Error::Raster(r) => Error::Raster(r),
        _ => Error::Operation("native alpha association policy/domain"),
    }
}
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    schema(binding.node, binding.identity, binding.version_origin, binding.arguments, &request, "own:alpha-association")?;
    (binding.validate_arguments)(binding.arguments, binding.params)?;
    (binding.validate_arguments)(&request.node.arguments, binding.params)?;
    if request.domain.encoding != binding.context.encoding || request.domain.profile_fingerprint != Some(binding.context.profile_identity) {
        return Err(Error::Domain);
    }
    if request.inputs.len() != 1 || binding.extent != request.roi || !endpoint_valid(binding.endpoint) {
        return Err(Error::Roi);
    }
    let input = request.inputs.first().ok_or(Error::MissingInput)?;
    if input.input_port != "source"
        || input.output_port != "output"
        || input.endpoint != binding.endpoint
        || input.roi != request.roi
        || input.domain != request.domain
        || input.surface.format() != request.domain.format
        || input.footprint.is_some()
    {
        return Err(Error::Domain);
    }
    let (resident, work) = precharge(&[input.surface], request.roi, 128, budget)?;
    let staged = {
        let cancel = || budget.poll().is_err();
        kernel::try_apply(
            input.surface,
            request.roi,
            binding.context,
            binding.context,
            binding.params,
            kernel::Limits { resident_bytes: resident, work_units: work },
            &Interrupt::cancel_only(&cancel),
        )
        .map_err(error)?
    };
    finish(staged, &request, budget)
}
/// These operators need a materialized native source/default; absence is not zero.
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    if request.inputs.len() != 1 || request.inputs.iter().any(|i| matches!(i.pixels, InputPixels::NoPixelsRequired)) {
        return Err(Error::MissingInput);
    }
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}
