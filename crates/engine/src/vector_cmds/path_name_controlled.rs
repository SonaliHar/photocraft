//! Controlled path.rename only; naming never evaluates or transforms geometry.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_doc::{
    Document, NamedPath, Path, Subpath,
    item_identity::{ItemRef, PathId},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
const ID: &str = "path.rename";
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{ID}: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn exact(v: &Value, nonzero: bool) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| error("captured IDs must be canonical decimal strings"))?;
    let n = s.parse::<u64>().map_err(|_| error("captured ID exceeds range"))?;
    if nonzero && n == 0 {
        return Err(error("documentId must be nonzero"));
    }
    Ok(n)
}
#[derive(Clone, Copy)]
enum Target {
    Work,
    Saved(PathId),
}
struct Request<'a> {
    target: Target,
    to: &'a str,
    label: &'static str,
}
fn request<'a>(session: &Session, p: &'a Value, early: bool) -> Result<Request<'a>> {
    if !p.as_object().is_some_and(|p| p.len() <= 5 && p.keys().all(|k| matches!(k.as_str(), "pathId" | "name" | "to" | "documentId" | "expectedRevision"))) {
        return Err(error("expected bounded path rename parameters"));
    }
    let name = match p.get("name") {
        None => None,
        Some(v) => Some(v.as_str().filter(|s| s.len() <= 4096 && !s.as_bytes().contains(&0)).ok_or_else(|| error("name must be bounded UTF8 without NUL"))?),
    };
    let to = p
        .get("to")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.as_bytes().contains(&0))
        .ok_or_else(|| error("missing or invalid to name"))?;
    if super::is_work(Some(to)) {
        return Err(error("to cannot name the work path"));
    }
    let id = super::saved_path_id(p)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if exact(id, true)? == state.doc.id.0 && exact(rev, false)? == state.revision => {}
        _ => return Err(error("captured target changed or incomplete")),
    }
    if (state.floating.is_some() && !(early && p.get("documentId").is_none()))
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
        || state.revision == u64::MAX
        || state.doc.accounted().is_some()
    {
        return Err(error("finish transient or pending work before renaming a path"));
    }
    if state.doc.paths.len() > 10_000 || state.doc.xcf_state.as_ref().is_some_and(|s| s.items.len() > 10_000 || s.source_items.len() > 10_000) {
        return Err(error("path metadata work limit"));
    }
    let target = if let Some(id) = id {
        let mut found = state.doc.paths.iter().filter(|path| path.id == id);
        if found.next().is_none() || found.next().is_some() {
            return Err(error("missing or ambiguous saved pathId"));
        }
        Target::Saved(id)
    } else if super::is_work(name) {
        if state.doc.work_path.is_none() {
            return Err(error("no work path"));
        }
        Target::Work
    } else {
        let mut found = state.doc.paths.iter().filter(|path| path.name == name.unwrap_or_default());
        let path = found.next().ok_or_else(|| error("no saved path with name"))?;
        if found.next().is_some() {
            return Err(error("ambiguous saved path name; use pathId"));
        }
        Target::Saved(path.id)
    };
    for path in &state.doc.paths {
        if path.name == to && !matches!(target,Target::Saved(id) if id==path.id) {
            return Err(error(format!("a path named \"{to}\" already exists")));
        }
    }
    if let Target::Saved(id) = target
        && let Some(imported) = &state.doc.xcf_state
    {
        let mut matched = imported.items.iter().filter(|item| item.item == ItemRef::Path(id));
        if let Some(item) = matched.next() {
            if matched.next().is_some() {
                return Err(error("ambiguous imported path identity"));
            }
            let current = state.doc.paths.iter().find(|path| path.id == id).ok_or_else(|| error("missing path"))?;
            if item.name != current.name {
                return Err(error("native/current imported path names disagree"));
            }
        }
    }
    Ok(Request { target, to, label: if super::is_work(name) { "Save Path" } else { "Rename Path" } })
}
pub(super) fn dispatch_preflight(session: &Session, id: &str, p: &Value, _background: bool) -> Result<bool> {
    if id != ID {
        return Ok(false);
    }
    let _ = request(session, p, true)?;
    Ok(true)
}
fn is_noop(doc: &Document, target: Target, to: &str) -> bool {
    let Target::Saved(id) = target else {
        return false;
    };
    doc.paths.iter().find(|path| path.id == id).is_some_and(|path| path.name == to)
        && doc
            .xcf_state
            .as_ref()
            .is_none_or(|state| state.items.iter().filter(|item| item.item == ItemRef::Path(id)).all(|item| item.name == to && item.custom_name))
}
fn string_copy(s: &str) -> Result<String> {
    let mut copy = String::new();
    copy.try_reserve_exact(s.len()).map_err(|_| error("name allocation"))?;
    copy.push_str(s);
    Ok(copy)
}
fn path_sizes(path: &Path, control: &Interrupt<'_>) -> Result<u64> {
    if path.subpaths.len() > 100_000 {
        return Err(error("path component count limit"));
    }
    let mut knots = 0usize;
    let mut bytes = (path.subpaths.capacity() as u64).checked_mul(std::mem::size_of::<Subpath>() as u64).ok_or_else(|| error("path size overflow"))?;
    for component in &path.subpaths {
        poll(control)?;
        knots = knots.checked_add(component.knots.len()).filter(|n| *n <= 1_000_000).ok_or_else(|| error("path knot count limit"))?;
        bytes = bytes
            .checked_add(
                (component.knots.capacity() as u64)
                    .checked_mul(std::mem::size_of::<photocraft_doc::Knot>() as u64)
                    .ok_or_else(|| error("knot size overflow"))?,
            )
            .ok_or_else(|| error("path size overflow"))?;
    }
    Ok(bytes)
}
fn prepare(snapshot: &DocumentSnapshot, target: Target, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    snapshot.validate_resident()?;
    snapshot.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, preparation, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
    let model = photocraft_io::retained_native_model_bytes(snapshot, preparation, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    let mut geometry = 0u64;
    for path in &snapshot.paths {
        poll(control)?;
        geometry = geometry.checked_add(path_sizes(&path.path, control)?).ok_or_else(|| error("path model overflow"))?;
    }
    if let Some(path) = &snapshot.work_path {
        geometry = geometry.checked_add(path_sizes(path, control)?).ok_or_else(|| error("work path model overflow"))?;
    }
    let bytes = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.retained_bytes))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(geometry.checked_mul(2)?))
        .and_then(|n| n.checked_add(32_768))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("model/source/path COW staging limit"))?;
    let identities = snapshot
        .paths
        .len()
        .checked_add(snapshot.xcf_state.as_ref().map_or(0, |s| s.items.len() + s.source_items.len()))
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| error("identity work overflow"))?;
    let work =
        geometry.checked_mul(8).and_then(|n| n.checked_add((identities as u64).checked_pow(2)?.checked_mul(16)?)).ok_or_else(|| error("path work overflow"))?;
    if work > 1 << 32 || bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(error("path preparation work limit"));
    }
    if let Target::Saved(id) = target
        && !snapshot.paths.iter().any(|path| path.id == id)
    {
        return Err(error("saved target vanished"));
    }
    poll(control)
}
fn copy_path(path: &Path, control: &Interrupt<'_>) -> Result<Path> {
    let mut subpaths = Vec::new();
    subpaths.try_reserve_exact(path.subpaths.len()).map_err(|_| error("component allocation"))?;
    if subpaths.capacity() != path.subpaths.len() {
        return Err(error("component capacity"));
    }
    for component in &path.subpaths {
        poll(control)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(component.knots.len()).map_err(|_| error("knot allocation"))?;
        if knots.capacity() != component.knots.len() {
            return Err(error("knot capacity"));
        }
        for chunk in component.knots.chunks(256) {
            poll(control)?;
            knots.extend_from_slice(chunk);
        }
        subpaths.push(Subpath { closed: component.closed, knots, op: component.op });
    }
    Ok(Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted })
}
fn fresh_id(doc: &Document, control: &Interrupt<'_>) -> Result<PathId> {
    let attempts = doc
        .paths
        .len()
        .checked_add(doc.xcf_state.as_ref().map_or(0, |s| s.items.len() + s.source_items.len()))
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| error("identity attempt overflow"))?;
    for _ in 0..attempts {
        poll(control)?;
        let id = PathId::fresh();
        if id.0 == 0 {
            continue;
        }
        let mut used = false;
        for path in &doc.paths {
            poll(control)?;
            if path.id == id {
                used = true;
                break;
            }
        }
        if let Some(state) = &doc.xcf_state {
            for item in &state.items {
                poll(control)?;
                if item.item == ItemRef::Path(id) {
                    used = true;
                    break;
                }
            }
            for item in &state.source_items {
                poll(control)?;
                if matches!(item.source,photocraft_doc::xcf_items::EditableSource::Vector{path,..} if path==id) {
                    used = true;
                    break;
                }
            }
        }
        if !used {
            return Ok(id);
        }
    }
    Err(error("unable to allocate a unique saved path identity"))
}
fn stage(snapshot: &DocumentSnapshot, target: Target, to: &str, control: &Interrupt<'_>) -> Result<Document> {
    prepare(snapshot, target, control)?;
    poll(control)?;
    let copied =
        if matches!(target, Target::Work) { Some(copy_path(snapshot.work_path.as_ref().ok_or_else(|| error("no work path"))?, control)?) } else { None };
    let mut doc = (**snapshot).clone();
    match target {
        Target::Work => {
            let id = fresh_id(&doc, control)?;
            doc.paths.try_reserve(1).map_err(|_| error("saved path allocation"))?;
            doc.paths.push(NamedPath { id, name: string_copy(to)?, path: copied.ok_or_else(|| error("missing prepared work path"))?, psd_raw: None });
            doc.work_path = None;
        }
        Target::Saved(id) => {
            let path = doc.paths.iter_mut().find(|path| path.id == id).ok_or_else(|| error("saved target missing"))?;
            let old = string_copy(&path.name)?;
            path.name = string_copy(to)?;
            if let Some(clipping) = &mut doc.clipping_path {
                poll(control)?;
                if clipping.name == old {
                    clipping.name = string_copy(to)?;
                }
            }
            if let Some(imported) = &mut doc.xcf_state {
                for item in &mut imported.items {
                    poll(control)?;
                    if item.item == ItemRef::Path(id) {
                        item.name = string_copy(to)?;
                        item.custom_name = true;
                    }
                }
            }
        }
    }
    poll(control)?;
    Ok(doc)
}
pub(super) fn path_rename(session: &mut Session, p: &Value) -> Result<Value> {
    let parsed = request(session, p, false)?;
    let target = parsed.target;
    let to = string_copy(parsed.to)?;
    let label = parsed.label;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if is_noop(&state.doc, target, &to) {
        return Ok(json!({"name":to}));
    }
    let baseline = state.doc.clone();
    let revision = state.revision;
    let weak = baseline.downgrade();
    let work = baseline.clone();
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, label, |control| {
                let doc = stage(&work, target, &to, control)?;
                let reply = json!({"name":to});
                poll(control)?;
                Ok((doc, reply, ctx.clone()))
            })
        },
        move |session, (doc, reply, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.floating.is_some()
                || current.coalesce.is_some()
                || current.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
                || session.command_has_job_on(current.doc.id)
            {
                return Err(error("target changed during path naming"));
            }
            ctx.stage(0.95, 1.0, label, |control| {
                poll(control)?;
                session.edit_with_interrupt(label, control, move |current, _| {
                    poll(control)?;
                    *current = doc;
                    Ok(())
                })?;
                Ok(reply)
            })
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_id_name_clipping_and_work_geometry_are_preserved() {
        let path = Path::new(vec![Subpath::polyline(&[(-0.0, 1.25), (3.5, 4.75)])]);
        let mut document = Document::new("Own names", photocraft_doc::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let saved = NamedPath::new("Old", path.clone());
        let id = saved.id;
        document.paths.push(saved);
        document.work_path = Some(path.clone());
        document.clipping_path = Some(photocraft_doc::ClippingPath { name: "Old".into(), flatness: 0.5 });
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(document)).unwrap();
        let renamed = stage(&snapshot, Target::Saved(id), "New", &Interrupt::NONE).unwrap();
        assert_eq!(renamed.paths[0].id, id);
        assert_eq!(renamed.paths[0].path, path);
        assert_eq!(renamed.clipping_path.as_ref().unwrap().name, "New");
        assert_eq!(renamed.paths[0].psd_raw, snapshot.paths[0].psd_raw);
        let saved = stage(&snapshot, Target::Work, "Saved work", &Interrupt::NONE).unwrap();
        assert!(saved.work_path.is_none());
        assert_ne!(saved.paths[1].id, id);
        assert_eq!(saved.paths[1].path.subpaths[0].knots[0].anchor.x.to_bits(), (-0.0f64).to_bits());
        let cancel = || true;
        assert!(matches!(stage(&snapshot, Target::Work, "Never", &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
    }
    #[test]
    fn canonical_path_name_sets_custom_flag_without_source_or_geometry_changes() {
        let mut document =
            Document::new("Own canonical name", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let saved = NamedPath::new("Original name", Path::default());
        let id = saved.id;
        document.paths.push(saved);
        let mut state = photocraft_doc::XcfState::default();
        state.items.push(photocraft_doc::xcf_state::ItemState {
            item: ItemRef::Path(id),
            name: "Original name".into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: Some(9),
            source_flags: 0x8000_0000,
            raster_bounds: None,
            mask: None,
            composition: None,
        });
        document.xcf_state = Some(state);
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(document)).unwrap();
        let mut expected = snapshot.xcf_state.as_ref().unwrap().clone();
        expected.items[0].name = "Owned name".into();
        expected.items[0].custom_name = true;
        let edited = stage(&snapshot, Target::Saved(id), "Owned name", &Interrupt::NONE).unwrap();
        assert_eq!(edited.xcf_state.as_ref(), Some(&expected));
        assert_eq!(edited.paths[0].id, id);
        assert_eq!(edited.paths[0].path, snapshot.paths[0].path);
        assert_eq!(edited.paths[0].psd_raw, snapshot.paths[0].psd_raw);
        assert_eq!(edited.icc_profile, snapshot.icc_profile);
        assert_eq!(edited.physical_resolution, snapshot.physical_resolution);
    }
    #[test]
    fn exact_noop_does_not_consume_real_redo_and_path_id_has_precedence() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":1,"height":1})).unwrap();
        session
            .edit("seed", |d, _| {
                d.paths.push(NamedPath::new("Stable", Path::default()));
                Ok(())
            })
            .unwrap();
        let id = session.active().unwrap().doc.paths[0].id;
        session
            .edit("name", |d, _| {
                d.name = "redo seed".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let revision = session.active().unwrap().revision;
        path_rename(&mut session, &json!({"pathId":id.0.to_string(),"name":"work","to":"Stable"})).unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
    }
}
