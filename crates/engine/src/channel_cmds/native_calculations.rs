//! Explicit current native unit-coverage arithmetic; no GIMP/source qualification.
use super::{bad, calculation_pixels, calculation_poll};
use crate::{EngineError, Result, Session};
use photocraft_algo::gimp::multi_input_coverage as coverage;
use photocraft_color::{
    ColorMode,
    checked_arithmetic::{Domain, Operand, Policy, Rounding, Storage, UnitOverflow, decode_finite},
};
use photocraft_doc::{
    AlphaChannel, Document,
    item_identity::{ChannelId, ItemRef},
    xcf_state::ItemState,
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface};
use serde_json::{Value, json};
use std::collections::HashSet;
const CMD: &str = "image.calculations";
const MAX_CHANNELS: usize = 10000;
pub(super) struct Plan {
    first: usize,
    second: usize,
    first_id: ChannelId,
    second_id: ChannelId,
    area: Rect,
    operation: coverage::Operation,
    document: u64,
    revision: u64,
}
fn identity(value: &Value) -> Result<u64> {
    let text = value.as_str().ok_or_else(|| bad(CMD, "native identities must be canonical decimal strings"))?;
    if text.is_empty() || text.len() > 20 || text.len() > 1 && text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad(CMD, "noncanonical native identity"));
    }
    text.parse().map_err(|_| bad(CMD, "native identity overflow"))
}
fn selector(p: &Value, key: &str) -> Result<ChannelId> {
    let object = p.get(key).and_then(Value::as_object).ok_or_else(|| bad(CMD, "native source requires {channelId}"))?;
    if object.len() != 1 {
        return Err(bad(CMD, "native source accepts only channelId"));
    }
    let id = identity(object.get("channelId").ok_or_else(|| bad(CMD, "missing channelId"))?)?;
    if id == 0 {
        return Err(bad(CMD, "zero channel identity"));
    }
    Ok(ChannelId(id))
}
fn metadata(doc: &Document, id: ChannelId, area: Rect, ctl: &Interrupt<'_>) -> Result<usize> {
    calculation_poll(ctl)?;
    let mut index = None;
    for (position, channel) in doc.channels.iter().enumerate() {
        calculation_poll(ctl)?;
        if channel.id == id && index.replace(position).is_some() {
            return Err(bad(CMD, "duplicate saved channel identity"));
        }
    }
    let index = index.ok_or_else(|| bad(CMD, "missing saved channel"))?;
    let channel = doc.channels.get(index).ok_or_else(|| bad(CMD, "missing saved channel"))?;
    let format = channel.surface.format();
    if format.mode != ColorMode::Grayscale || format.alpha || format.sample != doc.depth || channel.spot.is_some() {
        return Err(bad(CMD, "native coverage requires same-depth Gray/no-alpha saved channels"));
    }
    if channel.surface.default_bytes().len() != format.bytes_per_pixel() {
        return Err(bad(CMD, "native default width mismatch"));
    }
    // The actual native default has a declared unit meaning, never inferred from
    // tile presence or normalized to a different outside value.
    decode_finite(Operand { bytes: channel.surface.default_bytes(), storage: Storage::Native(format.sample), index: 0 }, Domain::Unit)
        .map_err(|e| bad(CMD, e.to_string()))?;
    if let Some(state) = &doc.xcf_state {
        if state.items.len() > 10000 || state.source_items.len() > 10000 || state.filter_stacks.len() > 10000 || state.opaque.len() > 10000 {
            return Err(bad(CMD, "imported native context limit"));
        }
        let owner = ItemRef::Channel(id);
        let mut item = None;
        for current in &state.items {
            calculation_poll(ctl)?;
            if current.item == owner && item.replace(current).is_some() {
                return Err(bad(CMD, "ambiguous channel metadata"));
            }
        }
        let item = item.ok_or_else(|| bad(CMD, "native imported channel geometry unavailable"))?;
        if item.raster_bounds != Some(area)
            || item.source_flags != 0
            || item.parent.is_some()
            || item.mask.is_some()
            || item.composition.is_some()
            || item.name != channel.name
        {
            return Err(bad(CMD, "unqualified imported native channel geometry/flags"));
        }
        for source in &state.source_items {
            calculation_poll(ctl)?;
            if source.owner == owner {
                return Err(bad(CMD, "native channel source-item context unsupported"));
            }
        }
        for stack in &state.filter_stacks {
            calculation_poll(ctl)?;
            if stack.owner == owner {
                return Err(bad(CMD, "native channel filter context unsupported"));
            }
        }
        for opaque in &state.opaque {
            calculation_poll(ctl)?;
            if opaque.rendering_required {
                return Err(bad(CMD, "required opaque rendering context unsupported"));
            }
        }
    }
    Ok(index)
}
pub(super) fn preflight(s: &Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Plan> {
    calculation_poll(ctl)?;
    let object = p.as_object().ok_or_else(|| bad(CMD, "native coverage requires explicit parameters"))?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "nativeCoverage"
                | "coverageContext"
                | "documentId"
                | "expectedRevision"
                | "source1"
                | "source2"
                | "blending"
                | "opacity"
                | "scale"
                | "offset"
                | "result"
                | "name"
        )
    }) {
        return Err(bad(CMD, "unsupported native coverage parameter"));
    }
    if p.get("nativeCoverage").and_then(Value::as_bool) != Some(true) || p.get("coverageContext").and_then(Value::as_str) != Some("unit") {
        return Err(bad(CMD, "explicit unit coverage context required"));
    }
    for (key, expected) in [("opacity", 100.0), ("scale", 1.0), ("offset", 0.0)] {
        if let Some(value) = p.get(key)
            && value.as_f64() != Some(expected)
        {
            return Err(bad(CMD, "native coverage requires opacity100/scale1/offset0"));
        }
    }
    if p.get("result").is_some_and(|value| value.as_str() != Some("newChannel")) {
        return Err(bad(CMD, "native coverage supports newChannel only"));
    }
    let operation = match p.get("blending").and_then(Value::as_str).unwrap_or("multiply") {
        "multiply" => coverage::Operation::Multiply,
        "darken" => coverage::Operation::IntersectionMin,
        "lighten" => coverage::Operation::UnionMax,
        "subtract" => coverage::Operation::SubtractSaturating,
        _ => return Err(bad(CMD, "unsupported native coverage blend")),
    };
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let document = identity(p.get("documentId").ok_or_else(|| bad(CMD, "captured documentId required"))?)?;
    let revision = identity(p.get("expectedRevision").ok_or_else(|| bad(CMD, "captured revision required"))?)?;
    if document == 0 || document != active.doc.id.0 || revision != active.revision {
        return Err(bad(CMD, "stale native coverage target"));
    }
    if s.color_restrict.is_some() || active.floating.is_some() || active.coalesce.is_some() || s.has_jobs() || s.coalesce_request.is_some() {
        return Err(bad(CMD, "native coverage requires ordinary idle context"));
    }
    active.doc.validate_resident()?;
    if active.revision == u64::MAX {
        return Err(bad(CMD, "revision headroom exhausted"));
    }
    if active.doc.channels.len() >= MAX_CHANNELS {
        return Err(bad(CMD, "saved channel headroom exhausted"));
    }
    let area = active.doc.bounds();
    if area.is_empty() {
        return Err(bad(CMD, "native coverage requires nonempty canvas"));
    }
    calculation_pixels(area)?;
    let first_id = selector(p, "source1")?;
    let second_id = selector(p, "source2")?;
    let first = metadata(&active.doc, first_id, area, ctl)?;
    let second = metadata(&active.doc, second_id, area, ctl)?;
    Ok(Plan { first, second, first_id, second_id, area, operation, document, revision })
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| bad(CMD, "native coverage byte overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| bad(CMD, "native coverage charge overflow"))
}
fn tiles(area: Rect) -> Result<u64> {
    let span =
        |a: i32, b: i32| u64::try_from(i64::from(b.div_euclid(TILE_SIZE)) - i64::from(a.div_euclid(TILE_SIZE)) + 1).map_err(|_| bad(CMD, "native tile span"));
    mul(
        span(area.x0, area.x1.checked_sub(1).ok_or_else(|| bad(CMD, "native tile edge"))?)?,
        span(area.y0, area.y1.checked_sub(1).ok_or_else(|| bad(CMD, "native tile edge"))?)?,
    )
}
fn validate_surface(surface: &Surface, ctl: &Interrupt<'_>) -> Result<()> {
    let format = surface.format();
    let payload = 65536usize.checked_mul(format.bytes_per_pixel()).ok_or_else(|| bad(CMD, "native tile width"))?;
    for (coordinate, tile) in surface.tiles() {
        calculation_poll(ctl)?;
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|x| x.checked_add(TILE_SIZE)).ok_or_else(|| bad(CMD, "native tile origin"))?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|y| y.checked_add(TILE_SIZE)).ok_or_else(|| bad(CMD, "native tile origin"))?;
        if tile.bytes().len() != payload {
            return Err(bad(CMD, "native tile payload mismatch"));
        }
        for index in 0..65536usize {
            if index % 4096 == 0 {
                calculation_poll(ctl)?;
            }
            decode_finite(Operand { bytes: tile.bytes(), storage: Storage::Native(format.sample), index }, Domain::Unit)
                .map_err(|e| bad(CMD, e.to_string()))?;
        }
    }
    Ok(())
}
fn render_guard(doc: &Document, ctl: &Interrupt<'_>) -> Result<()> {
    if doc.xcf_state.is_some() {
        doc.validate_xcf_state_with(ctl).map_err(|e| if e.code == "cancelled" { EngineError::Cancelled } else { bad(CMD, e.to_string()) })?;
        photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, ctl)
            .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(CMD, format!("unqualified imported native coverage context: {e}")) })?;
    }
    calculation_poll(ctl)
}
fn kernel_error(error: coverage::Error) -> EngineError {
    match error {
        coverage::Error::Raster(RasterInputError::Cancelled) => EngineError::Cancelled,
        other => bad(CMD, other.to_string()),
    }
}
pub(super) fn run(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let plan = preflight(s, p, ctl)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let baseline = active.doc.clone();
    let fence = baseline.downgrade();
    let first = &baseline.channels.get(plan.first).ok_or_else(|| bad(CMD, "source disappeared"))?.surface;
    let second = &baseline.channels.get(plan.second).ok_or_else(|| bad(CMD, "source disappeared"))?.surface;
    let bpp = first.format().bytes_per_pixel() as u64;
    let payload = mul(65536, bpp)?;
    let output_tiles = tiles(plan.area)?;
    let retained = add(first.tile_count() as u64, second.tile_count() as u64)?;
    // Match the existing coverage kernel envelope, then debit whole-document COW
    // and the generated channel simultaneously. No shared-input memory refund.
    let staging = add(mul(payload, 4)?, 8192)?;
    let maps = mul(add(retained, output_tiles)?, 1024)?;
    let extra = add(add(mul(output_tiles, add(payload, 512)?)?, maps)?, add(staging, add(payload, mul(4096, bpp)?)?)?)?;
    let resident = add(mul(retained, payload)?, mul(bpp, 2)?)?;
    let kernel_peak = add(resident, extra)?;
    let work = add(mul(output_tiles, mul(payload, 192)?)?, add(mul(retained, 32)?, mul(output_tiles, mul(add(retained, output_tiles)?, 512)?)?)?)?;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(&baseline, ctl)?;
    let output_charge = add(mul(add(second.tile_count() as u64, output_tiles)?, add(payload, 256)?)?, bpp)?;
    let peak = add(add(add(mul(footprint.retained_bytes, 2)?, mul(footprint.clone_metadata_bytes, 3)?)?, kernel_peak)?, add(output_charge, 3 << 20)?)?;
    if peak > MAX_CHECKED_RASTER_BYTES || kernel_peak > MAX_CHECKED_RASTER_BYTES {
        return Err(bad(CMD, "native coverage aggregate raster/COW limit"));
    }
    render_guard(&baseline, ctl)?;
    validate_surface(first, ctl)?;
    validate_surface(second, ctl)?;
    let output = coverage::try_apply(
        coverage::Input { surface: second, extent: plan.area, outside: coverage::Outside::Reject },
        coverage::Input { surface: first, extent: plan.area, outside: coverage::Outside::Reject },
        plan.area,
        coverage::Params {
            operation: plan.operation,
            policy: Policy { domain: Domain::Unit, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Nearest },
        },
        coverage::Limits { resident_bytes: kernel_peak, work_units: work },
        ctl,
    )
    .map_err(kernel_error)?;
    let name = p.get("name").and_then(Value::as_str).unwrap_or("Native Calculations");
    let mut owned = String::new();
    owned.try_reserve_exact(name.len()).map_err(|_| bad(CMD, "native name allocation"))?;
    if owned.capacity() > 1 << 20 {
        return Err(bad(CMD, "native name capacity"));
    }
    owned.push_str(name);
    calculation_poll(ctl)?;
    super::calculation_dispatch_preflight(s, CMD, p, false)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.doc.id.0 != plan.document || active.revision != plan.revision || !fence.is_current(&active.doc) {
        return Err(bad(CMD, "stale native channel snapshot"));
    }
    let index = s.edit_with_interrupt("Calculations", ctl, move |doc, _| {
        if doc.id.0 != plan.document
            || doc.channels.get(plan.first).map(|c| c.id) != Some(plan.first_id)
            || doc.channels.get(plan.second).map(|c| c.id) != Some(plan.second_id)
        {
            return Err(bad(CMD, "stale native channel identities"));
        }
        metadata(doc, plan.first_id, plan.area, ctl)?;
        metadata(doc, plan.second_id, plan.area, ctl)?;
        let mut ids = HashSet::new();
        ids.try_reserve(doc.channels.len()).map_err(|_| bad(CMD, "channel identity allocation"))?;
        for channel in &doc.channels {
            calculation_poll(ctl)?;
            if channel.id.0 == 0 || !ids.insert(channel.id.0) {
                return Err(bad(CMD, "ambiguous current channel identities"));
            }
        }
        let limit = u64::try_from(doc.channels.len())
            .map_err(|_| bad(CMD, "channel identity limit"))?
            .checked_add(1)
            .ok_or_else(|| bad(CMD, "channel identity limit"))?;
        let mut new_id = None;
        for candidate in 1..=limit {
            calculation_poll(ctl)?;
            if !ids.contains(&candidate) {
                new_id = Some(ChannelId(candidate));
                break;
            }
        }
        let id = new_id.ok_or_else(|| bad(CMD, "no free channel identity"))?;
        let mut channel = AlphaChannel::new(owned, output);
        channel.id = id;
        doc.channels.try_reserve(1).map_err(|_| bad(CMD, "native channel allocation"))?;
        if let Some(state) = &mut doc.xcf_state {
            if state.items.len() >= 10000 {
                return Err(bad(CMD, "canonical channel headroom"));
            }
            state.items.try_reserve(1).map_err(|_| bad(CMD, "canonical channel allocation"))?;
            state.items.push(ItemState {
                item: ItemRef::Channel(id),
                name: channel.name.clone(),
                custom_name: true,
                visible: false,
                locks: Default::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(plan.area),
                mask: None,
                composition: None,
            });
        }
        calculation_poll(ctl)?;
        doc.channels.push(channel);
        doc.validate_xcf_state_with(ctl).map_err(|e| if e.code == "cancelled" { EngineError::Cancelled } else { bad(CMD, e.to_string()) })?;
        render_guard(doc, ctl)?;
        calculation_poll(ctl)?;
        Ok(doc.channels.len() - 1)
    })?;
    Ok(json!({"channel":index,"nativeCoverage":true}))
}
