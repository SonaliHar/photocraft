//! Explicit current indexed ordinal/alpha edit. Immutable indexed source is untouched.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{ColorMode, Document, EditableIndexedRaster, LayerContent, LayerId, SampleType, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};

pub const SET: &str = "xcf.indexed.pixel.set";
const MAX_ITEMS: usize = 10_000;
const MAX_WORK: u64 = 1 << 30;
fn bad(code: &str) -> EngineError {
    EngineError::Other(format!("{SET}.{code}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, code: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(code) }
}
#[derive(Clone, Copy)]
struct Request {
    document: u64,
    revision: u64,
    layer: LayerId,
    x: i32,
    y: i32,
    index: u8,
    alpha: u8,
}
fn decimal(params: &Value, key: &str, zero: bool) -> Result<u64> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) || text.len() > 1 && text.starts_with('0') {
        return Err(bad("parameters"));
    }
    let value = text.parse::<u64>().map_err(|_| bad("parameters"))?;
    if value == 0 && !zero {
        return Err(bad("parameters"));
    }
    Ok(value)
}
fn request(params: &Value) -> Result<Request> {
    let object = params.as_object().ok_or_else(|| bad("parameters"))?;
    const KEYS: [&str; 7] = ["documentId", "expectedRevision", "layerId", "x", "y", "index", "alpha"];
    if object.len() != KEYS.len() || object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(bad("parameters"));
    }
    let coordinate = |key| i32::try_from(params.get(key).and_then(Value::as_i64).ok_or_else(|| bad("parameters"))?).map_err(|_| bad("parameters"));
    let byte = |key| u8::try_from(params.get(key).and_then(Value::as_u64).ok_or_else(|| bad("parameters"))?).map_err(|_| bad("parameters"));
    Ok(Request {
        document: decimal(params, "documentId", false)?,
        revision: decimal(params, "expectedRevision", true)?,
        layer: LayerId(decimal(params, "layerId", false)?),
        x: coordinate("x")?,
        y: coordinate("y")?,
        index: byte("index")?,
        alpha: byte("alpha")?,
    })
}
pub(crate) fn context(session: &Session) -> Result<()> {
    if session.coalesce_request.is_some() {
        return Err(bad("coalescing"));
    }
    if session.color_restrict.is_some() || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() {
        return Err(bad("context"));
    }
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
        return Err(bad("floating"));
    }
    if active.doc.accounted().is_some() || active.revision == u64::MAX {
        return Err(bad("context"));
    }
    active.doc.validate_resident().map_err(|_| bad("resident"))?;
    Ok(())
}
/// Root dispatcher calls this before generic floating normalization/job/coalesce mutation.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    if background || params.as_object().is_some_and(|object| object.contains_key("coalesce")) {
        return Err(bad("context"));
    }
    context(session)?;
    request(params)?;
    Ok(())
}
pub(crate) fn tree(document: &Document, control: &Interrupt<'_>) -> Result<std::collections::HashSet<LayerId>> {
    let mut seen = std::collections::HashSet::new();
    let mut rasters = std::collections::HashSet::new();
    let mut frames: [Option<std::slice::Iter<'_, photocraft_doc::Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *frames.first_mut().ok_or_else(|| bad("bounds"))? = Some(document.layers.iter());
    let mut depth = 0usize;
    loop {
        poll(control)?;
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth = depth.checked_sub(1).ok_or_else(|| bad("bounds"))?;
            continue;
        };
        if layer.id.0 == 0 || seen.len() >= MAX_ITEMS {
            return Err(bad("bounds"));
        }
        seen.try_reserve(1).map_err(|_| bad("allocation"))?;
        if !seen.insert(layer.id) {
            return Err(bad("identity"));
        }
        if !layer.effects.items.is_empty() || layer.effects.psd_raw.is_some() || layer.effects.reference.is_some() {
            return Err(bad("effect-carrier"));
        }
        match &layer.content {
            LayerContent::Raster(surface) if surface.format().mode == ColorMode::Rgb && surface.format().sample == SampleType::U8 => {
                rasters.try_reserve(1).map_err(|_| bad("allocation"))?;
                rasters.insert(layer.id);
            }
            LayerContent::Group(group) => {
                depth = depth.checked_add(1).filter(|n| *n <= photocraft_doc::MAX_GROUP_DEPTH).ok_or_else(|| bad("bounds"))?;
                *frames.get_mut(depth).ok_or_else(|| bad("bounds"))? = Some(group.children.iter());
            }
            _ => return Err(bad("carrier")),
        }
    }
    Ok(rasters)
}
struct Plan {
    indexed: usize,
    item: usize,
    old_index: u8,
    old_alpha: u8,
    changed: bool,
}
fn prepare(document: &Document, request: Request, control: &Interrupt<'_>) -> Result<Plan> {
    poll(control)?;
    let raster_owners = tree(document, control)?;
    if document.id.0 != request.document || document.mode != ColorMode::Indexed || document.depth != SampleType::U8 {
        return Err(bad("document"));
    }
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("indexed-state"))?;
    if state.items.len() > MAX_ITEMS || state.editable_indexed.len() > MAX_ITEMS {
        return Err(bad("limit"));
    }
    if !state.source_items.is_empty() || !state.filter_stacks.is_empty() {
        return Err(bad("source-or-filter-carrier"));
    }
    // Bound the aggregate scan/copy domain before any full-plane validator.
    let mut plane_bytes = 0u64;
    let mut scan_work = 0u64;
    for indexed in &state.editable_indexed {
        poll(control)?;
        plane_bytes = plane_bytes
            .checked_add(indexed.raster.indices().capacity() as u64)
            .and_then(|n| n.checked_add(indexed.raster.alpha().map_or(0, |alpha| alpha.capacity() as u64)))
            .and_then(|n| n.checked_add(indexed.raster.palette_capacity_bytes() as u64))
            .ok_or_else(|| bad("budget"))?;
        scan_work = scan_work.checked_add((indexed.raster.indices().len() as u64).checked_mul(8).ok_or_else(|| bad("budget"))?).ok_or_else(|| bad("budget"))?;
        if plane_bytes > MAX_CHECKED_RASTER_BYTES || scan_work > MAX_WORK {
            return Err(bad("budget"));
        }
    }
    document.validate_xcf_state_with(control).map_err(|_| controlled(control, "canonical-state"))?;
    let table = document.color_table.as_ref().ok_or_else(|| bad("palette"))?;
    let mut target = None;
    let mut owners = std::collections::HashSet::new();
    owners.try_reserve(state.editable_indexed.len()).map_err(|_| bad("allocation"))?;
    for (position, indexed) in state.editable_indexed.iter().enumerate() {
        poll(control)?;
        if indexed.owner.0 == 0 || !owners.insert(indexed.owner) {
            return Err(bad("indexed-identity"));
        }
        indexed.raster.validate_with(control).map_err(|_| controlled(control, "indexed-state"))?;
        if !raster_owners.contains(&indexed.owner) || indexed.raster.palette() != table.colors.as_slice() {
            return Err(bad("indexed-owner-or-palette"));
        }
        if indexed.owner == request.layer {
            target = Some(position);
        }
    }
    if owners.len() != raster_owners.len() {
        return Err(bad("missing-current-indexed-owner"));
    }
    let indexed = target.ok_or_else(|| bad("indexed-owner"))?;
    let raster = &state.editable_indexed.get(indexed).ok_or_else(|| bad("indexed-owner"))?.raster;
    let item = state.items.iter().position(|item| item.item == ItemRef::Layer(request.layer)).ok_or_else(|| bad("canonical-owner"))?;
    if state.items.get(item).and_then(|item| item.raster_bounds) != Some(raster.bounds()) {
        return Err(bad("declared-bounds"));
    }
    if table.colors.as_slice() != raster.palette()
        || usize::from(request.index) >= table.colors.len()
        || table.transparent.is_some_and(|index| usize::from(index) >= table.colors.len())
    {
        return Err(bad("palette"));
    }
    let layer = document.layer(request.layer).ok_or(EngineError::NoLayer(request.layer))?;
    let surface = layer.surface().ok_or_else(|| bad("carrier"))?;
    if !surface.format().alpha && (request.alpha != 255 || raster.alpha().is_some() || table.transparent.is_some()) {
        return Err(bad("alpha-carrier"));
    }
    let bounds = raster.bounds();
    if !bounds.contains(request.x, request.y) {
        return Err(bad("pixel-bounds"));
    }
    photocraft_raster::checked_region_byte_len(bounds, surface.format()).map_err(|_| bad("bounds"))?;
    let offset = usize::try_from(
        (i64::from(request.y) - i64::from(bounds.y0))
            .checked_mul(i64::from(bounds.x1) - i64::from(bounds.x0))
            .and_then(|n| n.checked_add(i64::from(request.x) - i64::from(bounds.x0)))
            .ok_or_else(|| bad("bounds"))?,
    )
    .map_err(|_| bad("bounds"))?;
    let old_index = raster.indices().get(offset).copied().ok_or_else(|| bad("pixel-bounds"))?;
    let old_alpha = raster.alpha().map_or(Ok(255), |alpha| alpha.get(offset).copied().ok_or_else(|| bad("pixel-bounds")))?;
    let locks = crate::xcf_item_locks::effective_layer_locks(document, request.layer)?;
    locks.require(crate::xcf_item_locks::EditKind::Content)?;
    let effective_old = if table.transparent == Some(old_index) { 0 } else { old_alpha };
    let effective_new = if table.transparent == Some(request.index) { 0 } else { request.alpha };
    if old_alpha != request.alpha || effective_old != effective_new {
        locks.require(crate::xcf_item_locks::EditKind::Alpha)?;
    }
    poll(control)?;
    Ok(Plan { indexed, item, old_index, old_alpha, changed: old_index != request.index || old_alpha != request.alpha })
}
pub(crate) fn tile_count(bounds: Rect) -> Result<u64> {
    let last_x = bounds.x1.checked_sub(1).ok_or_else(|| bad("bounds"))?;
    let last_y = bounds.y1.checked_sub(1).ok_or_else(|| bad("bounds"))?;
    let x = u64::try_from(i64::from(last_x.div_euclid(TILE_SIZE)) - i64::from(bounds.x0.div_euclid(TILE_SIZE)) + 1).map_err(|_| bad("bounds"))?;
    let y = u64::try_from(i64::from(last_y.div_euclid(TILE_SIZE)) - i64::from(bounds.y0.div_euclid(TILE_SIZE)) + 1).map_err(|_| bad("bounds"))?;
    x.checked_mul(y).ok_or_else(|| bad("budget"))
}
pub(crate) fn immutable_indexed_retention(layers: &[photocraft_doc::Layer], depth: usize, control: &Interrupt<'_>) -> Result<u64> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("bounds"));
    }
    let mut bytes = 0u64;
    for layer in layers {
        poll(control)?;
        if let Some(source) = &layer.indexed_source {
            bytes = bytes
                .checked_add(source.indices().capacity() as u64)
                .and_then(|n| n.checked_add(source.alpha().as_ref().map_or(0, |alpha| alpha.capacity() as u64)))
                .and_then(|n| n.checked_add((source.palette_capacity_bytes() as u64).checked_mul(2)?))
                .ok_or_else(|| bad("budget"))?;
        }
        if let LayerContent::Group(group) = &layer.content {
            bytes = bytes.checked_add(immutable_indexed_retention(&group.children, depth + 1, control)?).ok_or_else(|| bad("budget"))?;
        }
        if bytes > MAX_CHECKED_RASTER_BYTES {
            return Err(bad("budget"));
        }
    }
    Ok(bytes)
}
/// Direct canonical sync verifies each pixel and every allocated tile tail.
/// Retain fixed private-edit/expansion costs and native tile staging separately.
fn estimated_target_work(count: u64, tiles: u64, tile_bytes: u64) -> Result<u64> {
    // Explicit sync plus Session precommit sync: two independent checks.
    let direct =
        crate::xcf_editing::direct_indexed_sync_work(usize::try_from(count).map_err(|_| bad("budget"))?, usize::try_from(tiles).map_err(|_| bad("budget"))?)?;
    let direct = direct.checked_mul(2).ok_or_else(|| bad("budget"))?;
    count
        .checked_mul(24)
        .and_then(|n| n.checked_add(direct))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes)?.checked_mul(3)?))
        .ok_or_else(|| bad("budget"))
}
fn admit(document: &Document, plan: &Plan, control: &Interrupt<'_>) -> Result<()> {
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("indexed-state"))?;
    let target = state.editable_indexed.get(plan.indexed).ok_or_else(|| bad("indexed-state"))?;
    let surface = document.layer(target.owner).and_then(|layer| layer.surface()).ok_or_else(|| bad("carrier"))?;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(document, control).map_err(|_| controlled(control, "budget"))?;
    let mut resident = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes)
        .and_then(|n| n.checked_add((MAX_ITEMS as u64).checked_mul(256)?))
        .ok_or_else(|| bad("budget"))?;
    // Source planes can differ physically from current edited planes; retain both
    // conservatively even when their Arcs happen to share an allocation today.
    resident = resident.checked_add(immutable_indexed_retention(&document.layers, 0, control)?).ok_or_else(|| bad("budget"))?;
    let mut work = 0u64;
    for indexed in &state.editable_indexed {
        poll(control)?;
        resident = resident
            .checked_add(indexed.raster.indices().capacity() as u64)
            .and_then(|n| n.checked_add(indexed.raster.alpha().map_or(0, |alpha| alpha.capacity() as u64)))
            .and_then(|n| n.checked_add((indexed.raster.palette_capacity_bytes() as u64).checked_mul(2)?))
            .ok_or_else(|| bad("budget"))?;
        work = work.checked_add((indexed.raster.indices().len() as u64).checked_mul(8).ok_or_else(|| bad("budget"))?).ok_or_else(|| bad("budget"))?;
    }
    let count = target.raster.indices().len() as u64;
    let native = count.checked_mul(surface.format().bytes_per_pixel() as u64).ok_or_else(|| bad("budget"))?;
    let tiles = tile_count(target.raster.bounds())?;
    let tile_bytes =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64)).ok_or_else(|| bad("budget"))?;
    // Covers explicit edit planes plus generic indexed sync staging, expanded
    // input/output buffers, retained output tiles/maps and private COW metadata.
    resident = resident
        .checked_add(count.checked_mul(6).ok_or_else(|| bad("budget"))?)
        .and_then(|n| n.checked_add(native.checked_mul(4)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_add(512)?)?.checked_mul(3)?))
        .and_then(|n| n.checked_add((target.raster.palette_capacity_bytes() as u64).checked_mul(3)?))
        .ok_or_else(|| bad("budget"))?;
    work = work.checked_add(estimated_target_work(count, tiles, tile_bytes)?).ok_or_else(|| bad("budget"))?;
    if resident > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(bad("budget"));
    }
    poll(control)
}
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    poll(control)?;
    let request = request(params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.revision != request.revision {
        return Err(bad("stale-target"));
    }
    let fence = active.doc.downgrade();
    let before = active.doc.clone();
    let plan = prepare(&active.doc, request, control)?;
    let revision = request.revision.checked_add(u64::from(plan.changed)).ok_or_else(|| bad("revision"))?;
    let response = json!({"documentId":request.document.to_string(),"revision":revision.to_string(),"layerId":request.layer.0.to_string(),"x":request.x,"y":request.y,"index":request.index,"alpha":request.alpha,"updated":plan.changed});
    admit(&active.doc, &plan, control)?;
    let source = &active.doc.xcf_state.as_ref().and_then(|state| state.editable_indexed.get(plan.indexed)).ok_or_else(|| bad("indexed-state"))?.raster;
    let format = active.doc.layer(request.layer).and_then(|layer| layer.surface()).ok_or_else(|| bad("carrier"))?.format();
    let transparent = active.doc.color_table.as_ref().and_then(|table| table.transparent);
    let baseline = crate::xcf_editing::expanded_with(source, format, transparent, control)?;
    if active.doc.layer(request.layer).and_then(|layer| layer.surface()) != Some(&baseline) {
        return Err(bad("noncanonical-carrier"));
    }
    drop(baseline);
    if !plan.changed {
        poll(control)?;
        return Ok(response);
    }
    let mut palette = Vec::new();
    poll(control)?;
    palette.try_reserve_exact(source.palette().len()).map_err(|_| bad("allocation"))?;
    poll(control)?;
    palette.extend_from_slice(source.palette());
    let mut raster = EditableIndexedRaster::try_new_with_interrupt(source.bounds(), source.indices().clone(), source.alpha().cloned(), palette, control)
        .map_err(|_| controlled(control, "indexed-state"))?;
    raster.try_set_pixel_with_interrupt(request.x, request.y, request.index, request.alpha, control).map_err(|_| controlled(control, "indexed-edit"))?;
    let output = crate::xcf_editing::expanded_with(&raster, format, transparent, control)?;
    context(session)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.doc.id.0 != request.document || active.revision != request.revision || !fence.is_current(&active.doc) {
        return Err(bad("stale-target"));
    }
    poll(control)?;
    session.edit_with_interrupt("Set XCF Indexed Pixel", control, move |document, _| {
        let current = prepare(document, request, control)?;
        if current.indexed != plan.indexed || current.item != plan.item || current.old_index != plan.old_index || current.old_alpha != plan.old_alpha {
            return Err(bad("stale-target"));
        }
        let expected = raster.clone();
        document.layer_mut(request.layer).ok_or(EngineError::NoLayer(request.layer))?.content = LayerContent::Raster(output);
        document
            .xcf_state
            .as_mut()
            .and_then(|state| state.editable_indexed.get_mut(plan.indexed))
            .filter(|item| item.owner == request.layer)
            .ok_or_else(|| bad("indexed-owner"))?
            .raster = raster;
        crate::xcf_editing::synchronize_indexed_with_interrupt(&before, document, control)?;
        if document.xcf_state.as_ref().and_then(|state| state.editable_indexed.get(plan.indexed)).map(|item| &item.raster) != Some(&expected) {
            return Err(bad("ordinal-or-alpha-loss"));
        }
        poll(control)
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session).map_err(|error| error.to_string())?;
    let active = session.active().ok_or_else(|| "no document open".to_string())?;
    if active.doc.mode != ColorMode::Indexed
        || active.doc.depth != SampleType::U8
        || active.doc.xcf_state.as_ref().is_none_or(|state| state.editable_indexed.is_empty())
    {
        return Err("no current editable indexed pixels".into());
    }
    Ok(())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: SET,
        label: "Set XCF Indexed Pixel",
        menu: &[],
        shortcut: None,
        params: r#"{documentId:canonical nonzero decimal string,expectedRevision:canonical decimal string,layerId:canonical nonzero decimal string,x:i32,y:i32,index:u8,alpha:u8} -> current exact ordinal/independent alpha, one undo; ordinary resident Indexed/U8 RGB carrier only"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorTable, EditableIndexedLayer, IndexedSource, Layer, PixelFormat, Size, XcfState};
    use std::sync::Arc;
    fn source() -> Document {
        let bounds = Rect::new(-1, 1, 1, 3);
        let indices = Arc::new(vec![0, 1, 2, 1]);
        let alpha = Arc::new(vec![255, 255, 64, 7]);
        let palette = vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]];
        let raster = EditableIndexedRaster::try_new(bounds, indices.clone(), Some(alpha.clone()), palette.clone()).unwrap();
        let surface = crate::xcf_editing::expanded(&raster, PixelFormat::RGBA8, None).unwrap();
        let mut layer = Layer::new("Exact current ordinals", LayerContent::Raster(surface.clone()));
        layer.indexed_source = Some(IndexedSource::new(indices, Some(alpha), palette.clone(), bounds, &surface).unwrap());
        let owner = layer.id;
        let mut document = Document::new("Indexed pixel edit", Size::new(4, 4), ColorMode::Indexed, SampleType::U8);
        document.color_table = Some(ColorTable { colors: palette, transparent: None });
        document.layers.push(layer);
        let mut state = XcfState::default();
        state.reconcile_layers(&document).unwrap();
        state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(bounds);
        state.editable_indexed.push(EditableIndexedLayer { owner, raster });
        document.xcf_state = Some(state);
        document
    }
    fn params(session: &Session, index: u8, alpha: u8) -> Value {
        let active = session.active().unwrap();
        json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"layerId":active.doc.layers[0].id.0.to_string(),"x":-1,"y":1,"index":index,"alpha":alpha})
    }
    #[test]
    fn explicit_pixel_estimate_charges_direct_verification_and_sparse_tile_tails() {
        assert_eq!(estimated_target_work(1, 2, 65536 * 4).unwrap(), 24 + 8 * 2 + 2 * 65536 * 2 + 2 * 65536 * 4 * 3);
        assert!(estimated_target_work(u64::MAX, 1, 65536).is_err());
        assert!(estimated_target_work(1, MAX_WORK / 65536 + 1, 65536).unwrap() > MAX_WORK);
    }
    #[test]
    fn duplicate_ordinal_native_history_and_immutable_source_survive() {
        let original = source();
        let immutable = original.layers[0].indexed_source.clone();
        let mut session = Session::new();
        session.add_document(original.clone(), None).unwrap();
        let request = params(&session, 1, 255);
        set_with_interrupt(&mut session, &request, &Interrupt::NONE).unwrap();
        let edited = session.active().unwrap().doc.clone();
        let raster = &edited.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert_eq!(raster.indices().as_slice(), &[1, 1, 2, 1]);
        assert_eq!(raster.palette(), original.color_table.as_ref().unwrap().colors);
        assert_eq!(edited.layers[0].indexed_source, immutable);
        let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap();
        assert_eq!(reopened.xcf_state.as_ref().unwrap().editable_indexed[0].raster, *raster);
        assert_eq!(reopened.layers[0].indexed_source, immutable);
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc.xcf_state, original.xcf_state);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.xcf_state, edited.xcf_state);
    }
    #[test]
    fn alpha_lock_allows_ordinal_only_and_refuses_alpha_change() {
        let mut document = source();
        let owner = document.layers[0].id;
        document.layers[0].locks.transparency = true;
        document.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().locks.alpha = true;
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        let ordinal = params(&session, 1, 255);
        set_with_interrupt(&mut session, &ordinal, &Interrupt::NONE).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let alpha = params(&session, 1, 128);
        assert!(set_with_interrupt(&mut session, &alpha, &Interrupt::NONE).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
    }
    #[test]
    fn stale_float_and_cancel_refuse_without_history_or_source_changes() {
        let mut session = Session::new();
        session.add_document(source(), None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let mut malformed = params(&session, 1, 255);
        malformed["x"] = json!(-1.0);
        assert!(set_with_interrupt(&mut session, &malformed, &Interrupt::NONE).is_err());
        let mut stale = params(&session, 1, 255);
        stale["expectedRevision"] = json!("999999");
        assert!(set_with_interrupt(&mut session, &stale, &Interrupt::NONE).is_err());
        let cancel = || true;
        let request = params(&session, 1, 255);
        assert!(matches!(set_with_interrupt(&mut session, &request, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(!session.undo());
    }
}
