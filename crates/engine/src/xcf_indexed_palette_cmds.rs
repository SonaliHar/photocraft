//! One exact RGB palette slot, document-wide; no invented per-slot alpha.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{ColorMode, Document, EditableIndexedRaster, LayerContent, LayerId, SampleType, item_identity::ItemRef};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
pub const SET: &str = "xcf.indexed.palette.set";
const MAX_ITEMS: usize = 10_000;
const MAX_WORK: u64 = 1 << 30;
fn bad(code: &str) -> EngineError {
    EngineError::Other(format!("{SET}.{code}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn failure(control: &Interrupt<'_>, code: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(code) }
}
#[derive(Clone, Copy)]
struct Request {
    document: u64,
    revision: u64,
    layer: LayerId,
    slot: u8,
    rgb: [u8; 3],
}
fn decimal(params: &Value, key: &str, zero: bool) -> Result<u64> {
    let s = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) || s.len() > 1 && s.starts_with('0') {
        return Err(bad("parameters"));
    }
    let n = s.parse::<u64>().map_err(|_| bad("parameters"))?;
    if n == 0 && !zero {
        return Err(bad("parameters"));
    }
    Ok(n)
}
fn request(params: &Value) -> Result<Request> {
    let o = params.as_object().ok_or_else(|| bad("parameters"))?;
    const KEYS: [&str; 5] = ["documentId", "expectedRevision", "layerId", "slot", "rgb"];
    if o.len() != KEYS.len() || o.keys().any(|k| !KEYS.contains(&k.as_str())) {
        return Err(bad("parameters"));
    }
    let a = params.get("rgb").and_then(Value::as_array).filter(|a| a.len() == 3).ok_or_else(|| bad("parameters"))?;
    let mut rgb = [0; 3];
    for (target, value) in rgb.iter_mut().zip(a) {
        *target = u8::try_from(value.as_u64().ok_or_else(|| bad("parameters"))?).map_err(|_| bad("parameters"))?;
    }
    Ok(Request {
        document: decimal(params, "documentId", false)?,
        revision: decimal(params, "expectedRevision", true)?,
        layer: LayerId(decimal(params, "layerId", false)?),
        slot: u8::try_from(params.get("slot").and_then(Value::as_u64).ok_or_else(|| bad("parameters"))?).map_err(|_| bad("parameters"))?,
        rgb,
    })
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    if background || params.as_object().is_some_and(|o| o.contains_key("coalesce")) {
        return Err(bad("context"));
    }
    crate::xcf_indexed_pixel_cmds::context(session)?;
    request(params)?;
    Ok(())
}
/// Explicit palette installation changes canonical data, so shared sync uses
/// the verified direct lane. Keep native expansion/staged qualification costs;
/// no nearest-palette search is estimated for this lane.
fn estimated_owner_work(count: u64, tiles: u64, channels: u64) -> Result<u64> {
    // Explicit sync plus Session precommit sync: two independent checks.
    let direct =
        crate::xcf_editing::direct_indexed_sync_work(usize::try_from(count).map_err(|_| bad("budget"))?, usize::try_from(tiles).map_err(|_| bad("budget"))?)?;
    let direct = direct.checked_mul(2).ok_or_else(|| bad("budget"))?;
    count
        .checked_mul(24)
        .and_then(|n| n.checked_add(direct))
        .and_then(|n| n.checked_add(tiles.checked_mul(65536)?.checked_mul(channels)?.checked_mul(3)?))
        .ok_or_else(|| bad("budget"))
}
fn qualify(doc: &Document, r: Request, control: &Interrupt<'_>) -> Result<bool> {
    poll(control)?;
    let owners = crate::xcf_indexed_pixel_cmds::tree(doc, control)?;
    if doc.id.0 != r.document || doc.mode != ColorMode::Indexed || doc.depth != SampleType::U8 {
        return Err(bad("document"));
    }
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("indexed-state"))?;
    let table = doc.color_table.as_ref().ok_or_else(|| bad("palette"))?;
    let previous = *table.colors.get(usize::from(r.slot)).ok_or_else(|| bad("palette-slot"))?;
    if state.items.len() > MAX_ITEMS || state.editable_indexed.len() > MAX_ITEMS || !state.source_items.is_empty() || !state.filter_stacks.is_empty() {
        return Err(bad("carrier"));
    }
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control).map_err(|_| failure(control, "budget"))?;
    let mut resident = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes.checked_mul(2).ok_or_else(|| bad("budget"))?)
        .and_then(|n| n.checked_add((MAX_ITEMS as u64) * 256))
        .ok_or_else(|| bad("budget"))?;
    resident = resident.checked_add(crate::xcf_indexed_pixel_cmds::immutable_indexed_retention(&doc.layers, 0, control)?).ok_or_else(|| bad("budget"))?;
    let mut work = 0u64;
    let mut seen = std::collections::HashSet::new();
    seen.try_reserve(state.editable_indexed.len()).map_err(|_| bad("allocation"))?;
    for item in &state.editable_indexed {
        poll(control)?;
        if !owners.contains(&item.owner) || !seen.insert(item.owner) {
            return Err(bad("indexed-owner"));
        }
        let raster = &item.raster;
        let layer = doc.layer(item.owner).ok_or_else(|| bad("indexed-owner"))?;
        let surface = layer.surface().ok_or_else(|| bad("carrier"))?;
        let count = raster.indices().len() as u64;
        let channels = surface.format().bytes_per_pixel() as u64;
        let tiles = crate::xcf_indexed_pixel_cmds::tile_count(raster.bounds())?;
        resident = resident
            .checked_add(raster.indices().capacity() as u64)
            .and_then(|n| n.checked_add(raster.alpha().map_or(0, |v| v.capacity() as u64)))
            .and_then(|n| n.checked_add((raster.palette_capacity_bytes() as u64).checked_mul(5)?))
            .and_then(|n| n.checked_add(count.checked_mul(channels.checked_mul(4)?.checked_add(6)?)?))
            .and_then(|n| n.checked_add(tiles.checked_mul((65536u64).checked_mul(channels)?.checked_add(512)?)?.checked_mul(3)?))
            .ok_or_else(|| bad("budget"))?;
        work = work.checked_add(estimated_owner_work(count, tiles, channels)?).ok_or_else(|| bad("budget"))?;
        if resident > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
            return Err(bad("budget"));
        }
    }
    if seen != owners || !seen.contains(&r.layer) {
        return Err(bad("indexed-owner"));
    }
    doc.validate_xcf_state_with(control).map_err(|_| failure(control, "canonical-state"))?;
    for item in &state.editable_indexed {
        poll(control)?;
        let raster = &item.raster;
        if raster.palette() != table.colors.as_slice() {
            return Err(bad("palette"));
        }
        let metadata = state.items.iter().find(|entry| entry.item == ItemRef::Layer(item.owner)).ok_or_else(|| bad("canonical-owner"))?;
        if metadata.raster_bounds != Some(raster.bounds()) {
            return Err(bad("declared-bounds"));
        }
        let surface = doc.layer(item.owner).and_then(|layer| layer.surface()).ok_or_else(|| bad("carrier"))?;
        if !surface.format().alpha && (raster.alpha().is_some() || table.transparent.is_some()) {
            return Err(bad("alpha-carrier"));
        }
        let expected = crate::xcf_editing::expanded_with(raster, surface.format(), table.transparent, control)?;
        if surface != &expected {
            return Err(bad("noncanonical-carrier"));
        }
        // Slot RGB changes only affect owners whose ordinal plane actually uses it.
        // Target content locks are still checked even if its slot is currently unused.
        let mut affected = item.owner == r.layer;
        for chunk in raster.indices().chunks(32768) {
            poll(control)?;
            affected |= chunk.contains(&r.slot);
        }
        if affected {
            crate::xcf_item_locks::effective_layer_locks(doc, item.owner)?.require(crate::xcf_item_locks::EditKind::Content)?;
        }
        // Alpha plane and transparent ordinal cannot change through this RGB-only API.
    }
    poll(control)?;
    Ok(previous != r.rgb)
}
struct Update {
    owner: LayerId,
    raster: EditableIndexedRaster,
    surface: Surface,
}
fn updates(doc: &Document, r: Request, control: &Interrupt<'_>) -> Result<Vec<Update>> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("indexed-state"))?;
    let table = doc.color_table.as_ref().ok_or_else(|| bad("palette"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(state.editable_indexed.len()).map_err(|_| bad("allocation"))?;
    for item in &state.editable_indexed {
        poll(control)?;
        let old = &item.raster;
        let mut palette = Vec::new();
        palette.try_reserve_exact(old.palette().len()).map_err(|_| bad("allocation"))?;
        poll(control)?;
        palette.extend_from_slice(old.palette());
        let mut raster = EditableIndexedRaster::try_new_with_interrupt(old.bounds(), old.indices().clone(), old.alpha().cloned(), palette, control)
            .map_err(|_| failure(control, "indexed-state"))?;
        let mut replacement = Vec::new();
        replacement.try_reserve_exact(raster.palette().len()).map_err(|_| bad("allocation"))?;
        poll(control)?;
        replacement.extend_from_slice(raster.palette());
        *replacement.get_mut(usize::from(r.slot)).ok_or_else(|| bad("palette-slot"))? = r.rgb;
        raster.try_set_palette_with_interrupt(replacement, control).map_err(|_| failure(control, "palette"))?;
        let format = doc.layer(item.owner).and_then(|layer| layer.surface()).ok_or_else(|| bad("carrier"))?.format();
        let surface = crate::xcf_editing::expanded_with(&raster, format, table.transparent, control)?;
        out.push(Update { owner: item.owner, raster, surface });
    }
    poll(control)?;
    Ok(out)
}
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    poll(control)?;
    let r = request(params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.revision != r.revision {
        return Err(bad("stale-target"));
    }
    let changed = qualify(&active.doc, r, control)?;
    let revision = r.revision.checked_add(u64::from(changed)).ok_or_else(|| bad("revision"))?;
    let response = json!({"documentId":r.document.to_string(),"revision":revision.to_string(),"layerId":r.layer.0.to_string(),"slot":r.slot,"rgb":r.rgb,"updated":changed});
    if !changed {
        poll(control)?;
        return Ok(response);
    }
    let fence = active.doc.downgrade();
    let before = active.doc.clone();
    let prepared = updates(&active.doc, r, control)?;
    crate::xcf_indexed_pixel_cmds::context(session)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.doc.id.0 != r.document || active.revision != r.revision || !fence.is_current(&active.doc) {
        return Err(bad("stale-target"));
    }
    session.edit_with_interrupt("Set XCF Indexed Palette Slot", control, move |doc, _| {
        if !qualify(doc, r, control)? {
            return Err(bad("stale-target"));
        }
        *doc.color_table.as_mut().and_then(|table| table.colors.get_mut(usize::from(r.slot))).ok_or_else(|| bad("palette-slot"))? = r.rgb;
        for update in &prepared {
            poll(control)?;
            doc.layer_mut(update.owner).ok_or_else(|| bad("indexed-owner"))?.content = LayerContent::Raster(update.surface.clone());
            let item = doc
                .xcf_state
                .as_mut()
                .and_then(|state| state.editable_indexed.iter_mut().find(|item| item.owner == update.owner))
                .ok_or_else(|| bad("indexed-owner"))?;
            item.raster = update.raster.clone();
        }
        crate::xcf_editing::synchronize_indexed_with_interrupt(&before, doc, control)?;
        for update in &prepared {
            poll(control)?;
            if doc.xcf_state.as_ref().and_then(|state| state.editable_indexed.iter().find(|item| item.owner == update.owner)).map(|item| &item.raster)
                != Some(&update.raster)
                || doc.layer(update.owner).and_then(|layer| layer.surface()) != Some(&update.surface)
            {
                return Err(bad("ordinal-alpha-or-carrier-loss"));
            }
        }
        qualify(doc, r, control)?;
        poll(control)
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    crate::xcf_indexed_pixel_cmds::context(session).map_err(|e| e.to_string())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: SET,
        label: "Set XCF Indexed Palette Slot",
        menu: &[],
        shortcut: None,
        params: r#"{documentId:canonical nonzero decimal string,expectedRevision:canonical decimal string,layerId:canonical nonzero decimal string,slot:u8,rgb:[u8,u8,u8]} -> document-wide exact current RGB slot; ordinal/alpha planes and transparent ordinal unchanged; one undo"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorTable, EditableIndexedLayer, IndexedSource, Layer, PixelFormat, Size, XcfState};
    use photocraft_geom::Rect;
    use std::sync::Arc;
    fn fixture() -> Session {
        let bounds = Rect::new(-1, 1, 1, 3);
        let indices = Arc::new(vec![0, 1, 1, 2]);
        let alpha = Arc::new(vec![255, 64, 7, 255]);
        let colors = vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]];
        let raster = EditableIndexedRaster::try_new(bounds, indices.clone(), Some(alpha.clone()), colors.clone()).unwrap();
        let surface = crate::xcf_editing::expanded(&raster, PixelFormat::RGBA8, Some(1)).unwrap();
        let mut layer = Layer::new("Duplicate ordinal", LayerContent::Raster(surface.clone()));
        // Immutable source fingerprint uses the stored alpha, before current palette transparency.
        let original_surface = crate::xcf_editing::expanded(&raster, PixelFormat::RGBA8, None).unwrap();
        layer.indexed_source = Some(IndexedSource::new(indices, Some(alpha), colors.clone(), bounds, &original_surface).unwrap());
        let owner = layer.id;
        let mut doc = Document::new("Palette exact", Size::new(4, 4), ColorMode::Indexed, SampleType::U8);
        doc.layers = vec![layer];
        doc.color_table = Some(ColorTable { colors, transparent: Some(1) });
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner)).unwrap().raster_bounds = Some(bounds);
        state.editable_indexed.push(EditableIndexedLayer { owner, raster });
        doc.xcf_state = Some(state);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    fn params(session: &Session) -> Value {
        let active = session.active().unwrap();
        json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"layerId":active.doc.layers[0].id.0.to_string(),"slot":1,"rgb":[0,255,0]})
    }
    #[test]
    fn direct_estimate_keeps_padded_tile_work_without_legacy_palette_search() {
        let pixels = 4u64 << 20;
        let tiles = pixels / 65536;
        let work = estimated_owner_work(pixels, tiles, 4).unwrap();
        assert_eq!(work, pixels * (24 + 8 * 2) + tiles * 65536 * (2 + 4 * 3));
        assert!(work <= MAX_WORK);
        assert!(pixels * (256 + 32) > MAX_WORK);
        assert!(estimated_owner_work(u64::MAX, u64::MAX, 4).is_err());
        assert!(estimated_owner_work(1, MAX_WORK / 65536 + 1, 4).unwrap() > MAX_WORK);
    }
    #[test]
    fn transparent_duplicate_slot_preserves_ordinals_alpha_source_native_and_history() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.clone();
        let p = params(&session);
        set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
        let after = session.active().unwrap().doc.clone();
        let old = &before.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        let new = &after.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert_eq!(new.indices(), old.indices());
        assert_eq!(new.alpha(), old.alpha());
        assert_eq!(new.palette(), &[[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
        assert_eq!(after.color_table.as_ref().unwrap().transparent, Some(1));
        assert_eq!(after.layers[0].indexed_source, before.layers[0].indexed_source);
        let native = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *after);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, after);
        let revision = session.active().unwrap().revision;
        let p = params(&session);
        assert_eq!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap()["updated"], false);
        assert_eq!(session.active().unwrap().revision, revision);
    }
    #[test]
    fn stale_float_and_cancel_requests_preserve_snapshot() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.clone();
        let mut p = params(&session);
        p["expectedRevision"] = json!("999");
        assert!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        let mut p = params(&session);
        p["rgb"] = json!([0.5, 255, 0]);
        assert!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        let p = params(&session);
        let stop = || true;
        assert!(set_with_interrupt(&mut session, &p, &Interrupt::cancel_only(&stop)).is_err());
        assert_eq!(session.active().unwrap().doc, before);
    }
}
