//! Exact current layer-mask crop and placement; no source/filter-mask geometry inference.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerContent, LayerId,
    item_identity::{ItemRef, MaskId},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
use std::sync::Arc;
pub const SET: &str = "xcf.mask.geometry.set";
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
    mask: MaskId,
    crop: Rect,
    offset: [i32; 2],
}
fn decimal(p: &Value, key: &str, zero: bool) -> Result<u64> {
    let s = p.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) || s.len() > 1 && s.starts_with('0') {
        return Err(bad("parameters"));
    }
    let n = s.parse::<u64>().map_err(|_| bad("parameters"))?;
    if n == 0 && !zero {
        return Err(bad("parameters"));
    }
    Ok(n)
}
fn request(p: &Value) -> Result<Request> {
    let o = p.as_object().ok_or_else(|| bad("parameters"))?;
    const KEYS: [&str; 5] = ["documentId", "expectedRevision", "maskId", "crop", "offset"];
    if o.len() != 5 || o.keys().any(|k| !KEYS.contains(&k.as_str())) {
        return Err(bad("parameters"));
    }
    fn array<const N: usize>(p: &Value, key: &str) -> Result<[i32; N]> {
        let a = p.get(key).and_then(Value::as_array).filter(|a| a.len() == N).ok_or_else(|| bad("parameters"))?;
        let mut out = [0; N];
        for (to, v) in out.iter_mut().zip(a) {
            *to = i32::try_from(v.as_i64().ok_or_else(|| bad("parameters"))?).map_err(|_| bad("parameters"))?;
        }
        Ok(out)
    }
    let [x0, y0, x1, y1] = array(p, "crop")?;
    Ok(Request {
        document: decimal(p, "documentId", false)?,
        revision: decimal(p, "expectedRevision", true)?,
        mask: MaskId(decimal(p, "maskId", false)?),
        crop: Rect::new(x0, y0, x1, y1),
        offset: array(p, "offset")?,
    })
}
pub(crate) fn dispatch_preflight(session: &Session, p: &Value, background: bool) -> Result<()> {
    request(p)?;
    if background || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() {
        return Err(bad("context"));
    }
    if session.coalesce_request.is_some() || p.as_object().is_some_and(|o| o.contains_key("coalesce")) {
        return Err(bad("coalescing"));
    }
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some()) {
        return Err(bad("floating"));
    }
    if active.revision == u64::MAX {
        return Err(bad("revision"));
    }
    active.doc.validate_resident().map_err(|_| bad("accounted"))?;
    Ok(())
}
struct Target {
    owner: LayerId,
    bounds: Rect,
    surface: Arc<Surface>,
}
fn qualify(doc: &Document, id: MaskId, control: &Interrupt<'_>) -> Result<Target> {
    poll(control)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("unsupported"))?;
    if state.items.len() > MAX_ITEMS || state.filter_masks.len() > MAX_ITEMS {
        return Err(bad("limit"));
    }
    if state.filter_masks.iter().any(|m| m.id == id) {
        return Err(bad("unsupported-filter-mask"));
    }
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control).map_err(|_| failure(control, "budget"))?;
    if footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes.checked_mul(2).ok_or_else(|| bad("budget"))?)
        .is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("budget"));
    }
    let mut found = state.items.iter().filter(|i| i.item == ItemRef::Mask(id));
    let item = found.next().ok_or_else(|| bad("missing-mask"))?;
    if found.next().is_some() {
        return Err(bad("identity"));
    }
    let metadata = item.mask.as_ref().ok_or_else(|| bad("unsupported"))?;
    let ItemRef::Layer(owner) = metadata.owner else {
        return Err(bad("unsupported-owner"));
    };
    let owner_locks = crate::xcf_item_locks::effective_layer_locks(doc, owner)?;
    owner_locks.require(crate::xcf_item_locks::EditKind::Content)?;
    owner_locks.require(crate::xcf_item_locks::EditKind::Position)?;
    let mask_locks = crate::xcf_item_locks::effective_item_locks(doc, ItemRef::Mask(id))?;
    mask_locks.require(crate::xcf_item_locks::EditKind::Content)?;
    mask_locks.require(crate::xcf_item_locks::EditKind::Position)?;
    let layer = doc.layer(owner).ok_or_else(|| bad("missing-owner"))?;
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Err(bad("unsupported-owner"));
    }
    let mask = layer.mask.as_ref().ok_or_else(|| bad("missing-native-mask"))?;
    let format = mask.surface.format();
    if metadata.bounds.is_empty()
        || metadata.offset != [metadata.bounds.x0, metadata.bounds.y0]
        || item.raster_bounds != Some(metadata.bounds)
        || format.mode != photocraft_doc::ColorMode::Grayscale
        || format.alpha
        || format.sample != doc.depth
        || metadata.enabled != mask.enabled
        || metadata.linked
        || mask.linked
        || metadata.shown
        || metadata.editing
        || mask.density != 1.0
        || mask.feather != 0.0
    {
        return Err(bad("unsupported-mask-domain"));
    }
    if state
        .items
        .iter()
        .filter(|i| {
            i.mask.as_ref().is_some_and(|m| m.owner == ItemRef::Layer(owner))
                && !matches!(i.item,ItemRef::Mask(mid) if state.filter_masks.iter().any(|m|m.id==mid))
        })
        .count()
        != 1
    {
        return Err(bad("ambiguous-owner"));
    }
    photocraft_raster::checked_region_byte_len(metadata.bounds, format)?;
    let default = photocraft_color::try_read_sample_f64(mask.surface.default_bytes(), format.sample, 0).map_err(|_| bad("default"))?;
    if default.to_bits() != metadata.outside_f64_bits || !default.is_finite() || !(0.0..=1.0).contains(&default) {
        return Err(bad("default"));
    }
    let tile_bytes =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or_else(|| bad("budget"))?;
    let sample_work = (mask.surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_mul(4)).ok_or_else(|| bad("budget"))?;
    if sample_work > MAX_WORK {
        return Err(bad("work-limit"));
    }
    mask.surface.try_validate_tile_origins_with(control)?;
    for (_, tile) in mask.surface.tiles() {
        poll(control)?;
        if tile.bytes().len() as u64 != tile_bytes {
            return Err(bad("tile-length"));
        }
        for n in 0..65536usize {
            if n.is_multiple_of(4096) {
                poll(control)?;
            }
            let value = photocraft_color::try_read_sample_f64(tile.bytes(), format.sample, n).map_err(|_| bad("sample"))?;
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(bad("sample"));
            }
        }
    }
    doc.validate_xcf_state_with(control).map_err(|_| failure(control, "canonical-state"))?;
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control).map_err(|_| failure(control, "unsupported-current"))?;
    poll(control)?;
    Ok(Target { owner, bounds: metadata.bounds, surface: mask.surface.clone() })
}
fn destination(r: Request, target: &Target) -> Result<Rect> {
    if r.crop.is_empty() || r.crop.intersect(&target.bounds) != r.crop {
        return Err(bad("crop-bounds"));
    }
    let width = r.crop.x1.checked_sub(r.crop.x0).ok_or_else(|| bad("bounds"))?;
    let height = r.crop.y1.checked_sub(r.crop.y0).ok_or_else(|| bad("bounds"))?;
    Ok(Rect::new(
        r.offset[0],
        r.offset[1],
        r.offset[0].checked_add(width).ok_or_else(|| bad("bounds"))?,
        r.offset[1].checked_add(height).ok_or_else(|| bad("bounds"))?,
    ))
}
fn admit(doc: &Document, r: Request, target: &Target, to: Rect, control: &Interrupt<'_>) -> Result<()> {
    let format = target.surface.format();
    let copied = photocraft_raster::checked_region_byte_len(to, format)? as u64;
    let tiles = crate::xcf_indexed_pixel_cmds::tile_count(to)?;
    let tile_bytes =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or_else(|| bad("budget"))?;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control).map_err(|_| failure(control, "budget"))?;
    let peak = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes.checked_mul(2).ok_or_else(|| bad("budget"))?)
        .and_then(|n| n.checked_add(copied.checked_mul(2)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_add(512)?)?.checked_mul(3)?))
        .ok_or_else(|| bad("budget"))?;
    let work = copied
        .checked_mul(4)
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes)?.checked_mul(3)?))
        .and_then(|n| n.checked_add((target.surface.tile_count() as u64).checked_mul(65536)?.checked_mul(4)?))
        .ok_or_else(|| bad("budget"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(bad("budget"));
    }
    photocraft_raster::checked_region_byte_len(r.crop, format)?;
    poll(control)
}
fn install(doc: &mut Document, r: Request, target: &Target, to: Rect, output: Arc<Surface>, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    doc.layer_mut(target.owner).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("missing-mask"))?.surface = output;
    let item = doc.xcf_state.as_mut().and_then(|s| s.items.iter_mut().find(|i| i.item == ItemRef::Mask(r.mask))).ok_or_else(|| bad("missing-mask"))?;
    let metadata = item.mask.as_mut().ok_or_else(|| bad("missing-mask"))?;
    metadata.bounds = to;
    metadata.offset = [to.x0, to.y0];
    item.raster_bounds = Some(to);
    poll(control)
}
pub fn set_with_interrupt(session: &mut Session, p: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, p, false)?;
    let r = request(p)?;
    poll(control)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.doc.id.0 != r.document || active.revision != r.revision {
        return Err(bad("stale-target"));
    }
    let target = qualify(&active.doc, r.mask, control)?;
    let to = destination(r, &target)?;
    admit(&active.doc, r, &target, to, control)?;
    let changed = r.crop != target.bounds || to != target.bounds;
    let revision = r.revision.checked_add(u64::from(changed)).ok_or_else(|| bad("revision"))?;
    let response = json!({"documentId":r.document.to_string(),"revision":revision.to_string(),"maskId":r.mask.0.to_string(),"layerId":target.owner.0.to_string(),"bounds":[to.x0,to.y0,to.x1,to.y1],"updated":changed});
    if !changed {
        poll(control)?;
        return Ok(response);
    }
    let mut output = Surface::try_with_default_bytes(target.surface.format(), target.surface.default_bytes())?;
    output.try_copy_native_region_from(&target.surface, r.crop, to, control)?;
    let output = Arc::new(output);
    let fence = active.doc.downgrade();
    let before = active.doc.clone();
    let mut staged = before.clone();
    let doc = staged.try_resident_mut_with_raster_limit(MAX_CHECKED_RASTER_BYTES, control)?;
    install(doc, r, &target, to, output.clone(), control)?;
    qualify(doc, r.mask, control)?;
    drop(staged);
    dispatch_preflight(session, p, false)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.doc.id.0 != r.document || active.revision != r.revision || !fence.is_current(&active.doc) {
        return Err(bad("stale-target"));
    }
    session.edit_with_interrupt("Set XCF Layer Mask Geometry", control, move |doc, _| {
        let current = qualify(doc, r.mask, control)?;
        if current.owner != target.owner || current.bounds != target.bounds || !Arc::ptr_eq(&current.surface, &target.surface) {
            return Err(bad("stale-target"));
        }
        install(doc, r, &target, to, output, control)?;
        qualify(doc, r.mask, control)?;
        poll(control)
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    session.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn run(session: &mut Session, p: &Value) -> Result<Value> {
    set_with_interrupt(session, p, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: SET,
        label: "Set XCF Layer Mask Geometry",
        menu: &[],
        shortcut: None,
        params: r#"{documentId:canonical nonzero decimal string,expectedRevision:canonical decimal string,maskId:canonical nonzero decimal string,crop:[i32 x0,y0,x1,y1],offset:[i32 x,y]} -> crop inside current layer mask, then raw placement; unlinked finite unit Gray mask, native/default bits and source preserved, one undo"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        LayerMask, PixelFormat, SampleEncoding, XcfState,
        xcf_state::{ItemState, MaskState},
    };
    fn fixture() -> (Session, MaskId) {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":4,"height":3})).unwrap();
        let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let owner = doc.layers[0].id;
        let id = MaskId(1);
        let bounds = Rect::new(-1, 0, 3, 2);
        let mut surface = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[255]).unwrap();
        surface.try_write_interleaved(bounds, &[0, 17, 128, 255, 9, 31, 127, 254]).unwrap();
        doc.layer_mut(owner).unwrap().mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: false, density: 1.0, feather: 0.0 });
        let item = |reference, name: &str| ItemState {
            item: reference,
            name: name.into(),
            custom_name: true,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(bounds),
            mask: None,
            composition: None,
        };
        let mut metadata = item(ItemRef::Mask(id), "Independent mask");
        metadata.mask = Some(MaskState {
            owner: ItemRef::Layer(owner),
            bounds,
            offset: [-1, 0],
            outside_f64_bits: 1f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        });
        let mut owner_item = item(ItemRef::Layer(owner), "Owner");
        owner_item.raster_bounds = Some(doc.bounds());
        doc.xcf_state = Some(XcfState { items: vec![owner_item, metadata], ..Default::default() });
        (session, id)
    }
    fn params(session: &Session, id: MaskId) -> Value {
        let active = session.active().unwrap();
        json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"maskId":id.0.to_string(),"crop":[0,0,2,2],"offset":[-2,1]})
    }
    #[test]
    fn actual_raw_crop_placement_metadata_native_and_one_history_step() {
        let (mut session, id) = fixture();
        let before = session.active().unwrap().doc.clone();
        let p = params(&session, id);
        set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
        let after = session.active().unwrap().doc.clone();
        let native = after.layers[0].mask.as_ref().unwrap();
        assert_eq!(native.surface.default_bytes(), &[255]);
        assert_eq!(native.surface.try_read_interleaved(Rect::new(-2, 1, 0, 3)).unwrap(), [17, 128, 31, 127]);
        let metadata = after.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Mask(id)).unwrap();
        assert_eq!(metadata.mask.as_ref().unwrap().bounds, Rect::new(-2, 1, 0, 3));
        assert_eq!(metadata.mask.as_ref().unwrap().offset, [-2, 1]);
        assert_eq!(after.layers[0].indexed_source, before.layers[0].indexed_source);
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *after);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, after);
    }
    #[test]
    fn overflow_external_float_and_cancel_leave_snapshot_unchanged() {
        let (mut session, id) = fixture();
        let before = session.active().unwrap().doc.clone();
        for edit in [json!({"crop":[-2,0,2,2]}), json!({"offset":[2147483647,0]}), json!({"crop":[0.5,0,2,2]})] {
            let mut p = params(&session, id);
            for (k, v) in edit.as_object().unwrap() {
                p[k] = v.clone();
            }
            assert!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        }
        let p = params(&session, id);
        let stop = || true;
        assert!(set_with_interrupt(&mut session, &p, &Interrupt::cancel_only(&stop)).is_err());
        assert_eq!(session.active().unwrap().doc, before);
    }
}
