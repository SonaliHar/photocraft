//! Bounded saved/work path storage. Geometry is decoded by the existing parser.
use super::{bad, is_work, op_from, parse_path, path_json, preflight_path_value, saved_path_id};
use crate::{EngineError, Result, Session};
use photocraft_doc::item_identity::{ItemRef, PathId};
use photocraft_doc::xcf_items::EditableSource;
use photocraft_doc::{Document, NamedPath, Path, PathOp, Subpath};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
const NAME_LIMIT: usize = 2048;
const PATH_LIMIT: usize = 10000;
const KNOT_LIMIT: usize = 100000;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(command: &str, control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(command, error.to_string()) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}
#[derive(Clone, Copy)]
enum Selector<'a> {
    Id(PathId),
    Work,
    Name(&'a str),
}
fn selector<'a>(params: &'a Value) -> Result<Selector<'a>> {
    if let Some(id) = saved_path_id(params)? {
        return Ok(Selector::Id(id));
    }
    let name = params.get("name").and_then(Value::as_str);
    Ok(if is_work(name) { Selector::Work } else { Selector::Name(name.unwrap_or_default()) })
}
fn selected(selector: Selector<'_>, path: &NamedPath) -> bool {
    match selector {
        Selector::Id(id) => path.id == id,
        Selector::Name(name) => path.name == name,
        Selector::Work => false,
    }
}
fn metadata_shape(doc: &Document, command: &str, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if let Some(state) = &doc.xcf_state {
        let count = [
            state.items.len(),
            state.source_items.len(),
            state.filter_stacks.len(),
            state.opaque.len(),
            state.item_sets.len(),
            state.selection.selected.len(),
            state.selection.selected_paths.len(),
        ]
        .into_iter()
        .try_fold(0usize, |count, len| count.checked_add(len));
        if count.is_none_or(|count| count > 100000) {
            return Err(bad(command, "path metadata count limit"));
        }
        let mut count = count.ok_or_else(|| bad(command, "path metadata count overflow"))?;
        for set in &state.item_sets {
            poll(control)?;
            count = count.checked_add(set.members.len()).filter(|n| *n <= 100000).ok_or_else(|| bad(command, "path membership work limit"))?;
        }
        for stack in &state.filter_stacks {
            poll(control)?;
            count = count.checked_add(stack.nodes.len()).filter(|n| *n <= 100000).ok_or_else(|| bad(command, "path filter work limit"))?;
            for node in &stack.nodes {
                poll(control)?;
                count = count.checked_add(node.auxiliary.len()).filter(|n| *n <= 100000).ok_or_else(|| bad(command, "path auxiliary work limit"))?;
            }
        }
    }
    poll(control)
}
// A locally reconciled path self-row has no imported semantics to preserve.
// This is only a restrictive deletion discriminator, never Source authority.
// The actual path is removed privately by the existing job; the existing
// edit_validation -> reconcile_layers_with_interrupt transaction removes its
// exact missing-path row and validates the complete staged model before publish.
fn same_path_name(a: &str, b: &str, control: &Interrupt<'_>) -> Result<bool> {
    poll(control)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (left, right) in a.as_bytes().chunks(32 << 10).zip(b.as_bytes().chunks(32 << 10)) {
        poll(control)?;
        if left != right {
            return Ok(false);
        }
    }
    poll(control)?;
    Ok(true)
}
fn ordinary_current_path_row(doc: &Document, path: &NamedPath, control: &Interrupt<'_>) -> Result<Option<usize>> {
    let Some(state) = &doc.xcf_state else { return Ok(None) };
    if doc.source_color_context.is_some()
        || state.provenance != Default::default()
        || !state.source_items.is_empty()
        || !state.resources.is_empty()
        || !state.opaque.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
    {
        return Ok(None);
    }
    let reference = ItemRef::Path(path.id);
    let mut found = None;
    for (index, item) in state.items.iter().enumerate() {
        poll(control)?;
        if item.parent == Some(reference) || item.mask.as_ref().is_some_and(|mask| mask.owner == reference) {
            return Ok(None);
        }
        if item.item == reference {
            if found.is_some()
                || !same_path_name(&item.name, &path.name, control)?
                || !item.custom_name
                || item.visible
                || item.locks != Default::default()
                || item.parent.is_some()
                || item.raster_bounds.is_some()
                || item.mask.is_some()
                || item.composition.is_some()
                || item.source_tattoo.is_some()
                || item.source_flags != 0
            {
                return Ok(None);
            }
            found = Some(index);
        }
    }
    Ok(found)
}
fn target_policy(doc: &Document, selector: Selector<'_>, delete: bool, command: &str, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    metadata_shape(doc, command, control)?;
    let mut matches = 0usize;
    for path in &doc.paths {
        poll(control)?;
        if !selected(selector, path) {
            continue;
        }
        matches += 1;
        if path.id.0 == 0 {
            return Err(bad(command, "zero saved path identity"));
        }
        let mut identities = 0usize;
        for other in &doc.paths {
            poll(control)?;
            if other.id == path.id {
                identities += 1;
            }
        }
        if identities != 1 {
            return Err(bad(command, "ambiguous saved path identity"));
        }
        if let Some(state) = &doc.xcf_state {
            let reference = ItemRef::Path(path.id);
            if state.items.iter().any(|item| item.item == reference) {
                if delete {
                    // The admitted self-row has default locks and no parent or
                    // live owner. Do not allocate a general lock index for this
                    // new preflight lane; imported rows still refuse outright.
                    if ordinary_current_path_row(doc, path, control)?.is_none() {
                        return Err(bad(command, "imported canonical path deletion requires dependency management"));
                    }
                } else {
                    let locks = crate::xcf_item_locks::effective_item_locks(doc, reference)?;
                    locks.require(crate::xcf_item_locks::EditKind::Content)?;
                    locks.require(crate::xcf_item_locks::EditKind::Position)?;
                }
            }
            for source in &state.source_items {
                poll(control)?;
                if matches!(source.source,EditableSource::Vector{path:bound,..} if bound==path.id) {
                    if delete {
                        return Err(bad(command, "live vector path dependency prevents deletion"));
                    }
                    let locks = crate::xcf_item_locks::effective_item_locks(doc, source.owner)?;
                    locks.require(crate::xcf_item_locks::EditKind::Content)?;
                    locks.require(crate::xcf_item_locks::EditKind::Position)?;
                }
            }
            if delete
                && (state.selection.selected.contains(&reference)
                    || state.selection.active == Some(reference)
                    || state.selection.selected_paths.contains(&reference)
                    || state.selection.mask_target == Some(reference)
                    || state.item_sets.iter().any(|set| set.members.contains(&reference))
                    || state.opaque.iter().any(|record| record.owner == Some(reference)))
            {
                return Err(bad(command, "retained path reference prevents deletion"));
            }
        }
    }
    if matches!(selector, Selector::Id(_)) && matches != 1 {
        return Err(bad(command, "no unique saved path with pathId"));
    }
    if !delete && matches!(selector, Selector::Name(_)) && matches == 0 && doc.paths.len() >= PATH_LIMIT {
        return Err(bad(command, "saved path count limit"));
    }
    if delete {
        match selector {
            Selector::Work if doc.work_path.is_none() => return Err(bad(command, "no work path")),
            Selector::Name(_) if matches == 0 => return Err(bad(command, "no saved path with name")),
            _ => {}
        }
    }
    poll(control)
}
pub(super) fn dispatch_preflight(session: &Session, command: &str, params: &Value, _background: bool) -> Result<()> {
    if !matches!(command, "path.set" | "path.delete") {
        return Err(bad(command, "unsupported path storage command"));
    }
    if !params.is_null() {
        let object = params.as_object().filter(|map| map.len() <= 6).ok_or_else(|| bad(command, "expected bounded path parameters"))?;
        if !object.keys().all(|key| {
            matches!(key.as_str(), "pathId" | "name" | "documentId" | "expectedRevision") || (command == "path.set" && matches!(key.as_str(), "path" | "op"))
        }) {
            return Err(bad(command, "unknown path storage parameter"));
        }
    }
    if let Some(value) = params.get("name")
        && !value.as_str().is_some_and(|name| name.len() <= NAME_LIMIT && !name.contains('\0'))
    {
        return Err(bad(command, "bounded UTF-8 path name required"));
    }
    if command == "path.set" {
        preflight_path_value(params.get("path").ok_or_else(|| bad(command, "missing path"))?).map_err(|e| bad(command, e))?;
    }
    if let Some(value) = params.get("op") {
        let text = value.as_str().filter(|text| text.len() <= 16).ok_or_else(|| bad(command, "bounded path operation required"))?;
        if op_from(text).is_none() {
            return Err(bad(command, "unknown path operation"));
        }
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.paths.len() > PATH_LIMIT || state.revision == u64::MAX || state.doc.accounted().is_some() {
        return Err(bad(command, "path count/revision/resident limit"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if state.doc.id.0 != 0 && decimal(id) == Some(state.doc.id.0) && decimal(revision) == Some(state.revision) => {}
        _ => return Err(bad(command, "captured path target changed or incomplete")),
    }
    if state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad(command, "finish jobs or gestures before path storage edits"));
    }
    target_policy(&state.doc, selector(params)?, command == "path.delete", command, &Interrupt::NONE)
}
fn path_census(path: &Path, command: &str, control: &Interrupt<'_>, count: &mut usize) -> Result<u64> {
    poll(control)?;
    if path.subpaths.len() > PATH_LIMIT {
        return Err(bad(command, "subpath count limit"));
    }
    let mut bytes =
        (path.subpaths.capacity() as u64).checked_mul(std::mem::size_of::<Subpath>() as u64 + 256).ok_or_else(|| bad(command, "path capacity overflow"))?;
    for subpath in &path.subpaths {
        poll(control)?;
        *count = count.checked_add(subpath.knots.len()).filter(|n| *n <= KNOT_LIMIT).ok_or_else(|| bad(command, "aggregate knot limit"))?;
        bytes = bytes
            .checked_add(
                (subpath.knots.capacity() as u64)
                    .checked_mul(std::mem::size_of::<photocraft_doc::Knot>() as u64)
                    .ok_or_else(|| bad(command, "knot capacity overflow"))?,
            )
            .ok_or_else(|| bad(command, "path capacity overflow"))?;
        for knot in &subpath.knots {
            poll(control)?;
            for point in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
                if !point.x.is_finite() || !point.y.is_finite() || point.x.abs() > 1000000. || point.y.abs() > 1000000. {
                    return Err(bad(command, "retained path coordinate is unsupported"));
                }
            }
        }
    }
    Ok(bytes)
}
#[derive(Clone, Copy, Default)]
struct InputCost {
    bytes: u64,
    knots: usize,
}
fn input_cost(value: &Value, command: &str) -> Result<InputCost> {
    let subpaths = value.get("subpaths").and_then(Value::as_array).ok_or_else(|| bad(command, "missing subpaths"))?;
    let mut knots = 0usize;
    for subpath in subpaths {
        let count = subpath.get("knots").and_then(Value::as_array).ok_or_else(|| bad(command, "missing knots"))?.len();
        knots = knots.checked_add(count).filter(|n| *n <= KNOT_LIMIT).ok_or_else(|| bad(command, "incoming knot limit"))?;
    }
    let bytes = (knots as u64)
        .checked_mul(std::mem::size_of::<photocraft_doc::Knot>() as u64)
        .and_then(|n| n.checked_add((subpaths.len() as u64).checked_mul(std::mem::size_of::<Subpath>() as u64 + 256)?))
        .and_then(|n| n.checked_mul(2))
        .ok_or_else(|| bad(command, "incoming allocation envelope"))?;
    Ok(InputCost { bytes, knots })
}
fn admission(doc: &Document, incoming: Option<&Path>, extra: InputCost, command: &str, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    metadata_shape(doc, command, control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(command, control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(command, control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let mut knots = extra.knots;
    let mut paths = extra.bytes;
    for saved in &doc.paths {
        poll(control)?;
        paths = paths.checked_add(path_census(&saved.path, command, control, &mut knots)?).ok_or_else(|| bad(command, "path model capacity"))?;
    }
    if let Some(path) = &doc.work_path {
        paths = paths.checked_add(path_census(path, command, control, &mut knots)?).ok_or_else(|| bad(command, "work path capacity"))?;
    }
    if let Some(path) = incoming {
        paths = paths.checked_add(path_census(path, command, control, &mut knots)?).ok_or_else(|| bad(command, "incoming path capacity"))?;
    }
    let retained = payload
        .checked_add(model.checked_mul(5).ok_or_else(|| bad(command, "model capacity"))?)
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(paths.checked_mul(8)?))
        .and_then(|n| n.checked_add((knots as u64).checked_mul(2048)?))
        .and_then(|n| n.checked_add(1 << 20));
    if retained.is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad(command, "path storage resident admission"));
    }
    let identities = (doc.paths.len() as u64)
        .checked_add(doc.xcf_state.as_ref().map_or(0, |state| state.items.len() as u64 + state.source_items.len() as u64))
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| bad(command, "identity work count"))?;
    let work = payload
        .checked_add(model)
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(identities.checked_pow(2)?.checked_mul(64)?))
        .and_then(|n| n.checked_add((knots as u64).checked_mul(128)?));
    if work.is_none_or(|n| n > 8_000_000_000) {
        return Err(bad(command, "path storage work admission"));
    }
    poll(control)
}
fn copy_path(path: &Path, command: &str, control: &Interrupt<'_>) -> Result<Path> {
    poll(control)?;
    let mut subpaths = Vec::new();
    subpaths.try_reserve_exact(path.subpaths.len()).map_err(|_| bad(command, "path allocation"))?;
    for old in &path.subpaths {
        poll(control)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(old.knots.len()).map_err(|_| bad(command, "knot allocation"))?;
        for chunk in old.knots.chunks(512) {
            poll(control)?;
            knots.extend_from_slice(chunk);
        }
        subpaths.push(Subpath { closed: old.closed, op: old.op, knots });
    }
    poll(control)?;
    Ok(Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted })
}
fn owned_name(text: &str, command: &str, control: &Interrupt<'_>) -> Result<String> {
    poll(control)?;
    let mut name = String::new();
    name.try_reserve_exact(text.len()).map_err(|_| bad(command, "name allocation"))?;
    name.push_str(text);
    poll(control)?;
    Ok(name)
}
fn merge(old: Option<&Path>, mut incoming: Path, operation: Option<PathOp>, command: &str, control: &Interrupt<'_>) -> Result<Path> {
    if let (Some(old), Some(operation)) = (old, operation) {
        if old.subpaths.len().checked_add(incoming.subpaths.len()).is_none_or(|count| count > PATH_LIMIT) {
            return Err(bad(command, "appended subpath count limit"));
        }
        let mut output = copy_path(old, command, control)?;
        output.subpaths.try_reserve_exact(incoming.subpaths.len()).map_err(|_| bad(command, "append path allocation"))?;
        for mut subpath in incoming.subpaths.drain(..) {
            poll(control)?;
            subpath.op = operation;
            output.subpaths.push(subpath);
        }
        poll(control)?;
        Ok(output)
    } else {
        poll(control)?;
        Ok(incoming)
    }
}
fn fresh(doc: &Document, command: &str, control: &Interrupt<'_>) -> Result<PathId> {
    let metadata = doc.xcf_state.as_ref().map_or(0, |state| state.items.len() + state.source_items.len());
    let attempts = doc.paths.len().checked_add(metadata).and_then(|n| n.checked_add(1)).ok_or_else(|| bad(command, "path identity attempts"))?;
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
            for source in &state.source_items {
                poll(control)?;
                if matches!(source.source,EditableSource::Vector{path,..} if path==id) {
                    used = true;
                    break;
                }
            }
        }
        if !used {
            return Ok(id);
        }
    }
    Err(bad(command, "path identities exhausted"))
}
enum Prepared {
    Set { path: Path, id: Option<PathId>, name: Option<String>, work: bool, new_id: Option<PathId>, response: Value },
    Delete { ids: Vec<PathId>, work: bool },
}
pub(super) fn run(session: &mut Session, command: &str, params: &Value) -> Result<Value> {
    dispatch_preflight(session, command, params, false)?;
    let command = if command == "path.set" { "path.set" } else { "path.delete" };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let weak = snapshot.downgrade();
    let id = saved_path_id(params)?;
    let borrowed_name = params.get("name").and_then(Value::as_str);
    let work = id.is_none() && is_work(borrowed_name);
    let name = borrowed_name.map(|text| owned_name(text, command, &Interrupt::NONE)).transpose()?;
    let extra =
        if command == "path.set" { input_cost(params.get("path").ok_or_else(|| bad(command, "missing path"))?, command)? } else { InputCost::default() };
    // Admit the aggregate retained model and prospective parser vectors before
    // the checked parser allocates them; the worker rechecks actual capacities.
    admission(&snapshot, None, extra, command, &Interrupt::NONE)?;
    // The parser already admits shape/coordinates before its bounded fallible vectors.
    let incoming = if command == "path.set" {
        Some(parse_path(params.get("path").ok_or_else(|| bad(command, "missing path"))?).map_err(|e| bad(command, e))?)
    } else {
        None
    };
    let operation = params.get("op").and_then(Value::as_str).and_then(op_from);
    let worker = snapshot.clone();
    let label = if command == "path.delete" {
        "Delete Path"
    } else if is_work(borrowed_name) {
        "Work Path"
    } else {
        "Save Path"
    };
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                admission(&worker, incoming.as_ref(), InputCost::default(), command, control)?;
                let select = if let Some(id) = id {
                    Selector::Id(id)
                } else if work {
                    Selector::Work
                } else {
                    Selector::Name(name.as_deref().unwrap_or_default())
                };
                target_policy(&worker, select, command == "path.delete", command, control)?;
                let prepared = if let Some(incoming) = incoming {
                    let previous =
                        if work { worker.work_path.as_ref() } else { worker.paths.iter().find(|path| selected(select, path)).map(|path| &path.path) };
                    let replacement = merge(previous, incoming, operation, command, control)?;
                    let new_id = if !work && previous.is_none() {
                        if worker.paths.len() >= PATH_LIMIT {
                            return Err(bad(command, "saved path count limit"));
                        }
                        Some(fresh(&worker, command, control)?)
                    } else {
                        None
                    };
                    poll(control)?;
                    let response = json!({"name":name.as_deref().unwrap_or("work"),"path":path_json(&replacement)});
                    poll(control)?;
                    Prepared::Set { path: replacement, id, name, work, new_id, response }
                } else {
                    let mut ids = Vec::new();
                    ids.try_reserve_exact(worker.paths.len()).map_err(|_| bad(command, "delete identity allocation"))?;
                    for path in &worker.paths {
                        poll(control)?;
                        if selected(select, path) {
                            ids.push(path.id);
                        }
                    }
                    Prepared::Delete { ids, work }
                };
                poll(control)?;
                Ok((prepared, ctx.clone()))
            })
        },
        move |session, (prepared, ctx)| {
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                let current = session.active().ok_or(EngineError::NoDocument)?;
                if current.doc.id != snapshot.id
                    || current.revision != revision
                    || !current.doc.shares_state(&snapshot)
                    || !weak.is_current(&current.doc)
                    || current.floating.is_some()
                    || current.coalesce.is_some()
                    || session.command_has_job_on(snapshot.id)
                    || session.command_has_pending_job()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                {
                    return Err(bad(command, "path storage target changed"));
                }
                session.edit_with_interrupt(label, control, move |doc, _| {
                    poll(control)?;
                    match prepared {
                        Prepared::Set { path, id, name, work, new_id, response } => {
                            if work {
                                doc.work_path = Some(path);
                            } else if let Some(id) = id {
                                doc.paths.iter_mut().find(|saved| saved.id == id).ok_or_else(|| bad(command, "saved target disappeared"))?.path = path;
                            } else if let Some(new_id) = new_id {
                                doc.paths.try_reserve_exact(1).map_err(|_| bad(command, "saved path allocation"))?;
                                doc.paths.push(NamedPath { id: new_id, name: name.unwrap_or_default(), path, psd_raw: None });
                            } else {
                                let name = name.unwrap_or_default();
                                doc.paths.iter_mut().find(|saved| saved.name == name).ok_or_else(|| bad(command, "named target disappeared"))?.path = path;
                            }
                            poll(control)?;
                            Ok(response)
                        }
                        Prepared::Delete { ids, work } => {
                            if work {
                                doc.work_path = None;
                            } else {
                                // Clone is private; cancellation never exposes partially removed paths.
                                let mut index = 0usize;
                                while index < doc.paths.len() {
                                    poll(control)?;
                                    let remove = doc.paths.get(index).is_some_and(|path| ids.contains(&path.id));
                                    if remove {
                                        let removed = doc.paths.remove(index);
                                        if doc.clipping_path.as_ref().is_some_and(|clip| clip.name == removed.name) {
                                            doc.clipping_path = None;
                                        }
                                    } else {
                                        index += 1;
                                    }
                                }
                            }
                            poll(control)?;
                            Ok(Value::Null)
                        }
                    }
                })
            })
        },
    )
}

// Actual regression drafts, deliberately unrun in the implementation-only wave.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, FillRule, Knot, Layer, SampleType, Size, XcfState};
    fn geometry() -> Value {
        json!({"subpaths":[{"closed":false,"op":"subtract","knots":[{"anchor":[-2.5,3.25],"in":[-3.,2.],"out":[1.,4.],"smooth":true}]}],"fillRule":"evenodd","inverted":true})
    }
    fn fixture() -> Session {
        let mut doc = Document::new("Own paths", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let mut saved = NamedPath::new("Stored", Path::new(vec![Subpath::polygon(&[(0., 0.), (1., 1.)])]));
        saved.id = PathId(u64::MAX);
        doc.paths.push(saved);
        doc.work_path = Some(Path::new(vec![Subpath::polygon(&[(2., 2.)])]));
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn maximum_identity_precedes_work_name_and_preserves_exact_native_geometry_history() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.clone();
        let decoded = parse_path(&geometry()).unwrap();
        run(&mut session, "path.set", &json!({"pathId":u64::MAX.to_string(),"name":"work","path":geometry()})).unwrap();
        let current = &session.active().unwrap().doc;
        assert_eq!(current.paths[0].id, PathId(u64::MAX));
        assert_eq!(current.paths[0].path, decoded);
        assert_eq!(current.paths[0].name, "Stored");
        assert_eq!(current.work_path, before.work_path);
        let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap();
        // MAX cannot reserve a future runtime identity; native loading rekeys the
        // complete identity set. Compare every persisted path field under its bijection.
        let current_paths = current.paths.clone();
        assert_eq!(reopened.paths.len(), current_paths.len());
        assert_eq!(reopened.work_path, current.work_path);
        let mut remapped_paths = reopened.paths.clone();
        let mut loaded_ids = std::collections::BTreeSet::new();
        for (loaded, original) in remapped_paths.iter_mut().zip(&current_paths) {
            assert_ne!(loaded.id.0, 0);
            assert_ne!(loaded.id, PathId(u64::MAX));
            assert!(loaded_ids.insert(loaded.id.0));
            loaded.id = original.id;
        }
        assert_eq!(remapped_paths, current_paths);
        session.execute("edit.undo", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().doc.paths, before.paths);
        session.execute("edit.redo", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().doc.paths, current_paths);
    }
    #[test]
    fn append_retains_old_path_metadata_and_overrides_only_appended_subpath_op() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.paths[0].path.clone();
        run(&mut session, "path.set", &json!({"name":"Stored","path":geometry(),"op":"xor"})).unwrap();
        let current = &session.active().unwrap().doc.paths[0].path;
        assert_eq!(current.fill_rule, before.fill_rule);
        assert_eq!(current.inverted, before.inverted);
        assert_eq!(current.subpaths[0], before.subpaths[0]);
        assert_eq!(current.subpaths[1].op, PathOp::Exclude);
        assert!(!current.subpaths[1].closed);
        assert_eq!(current.subpaths[1].knots[0], parse_path(&geometry()).unwrap().subpaths[0].knots[0]);
        run(&mut session, "path.set", &json!({"path":geometry()})).unwrap();
        assert_eq!(session.active().unwrap().doc.work_path.as_ref().unwrap().fill_rule, FillRule::EvenOdd);
    }
    #[test]
    fn name_delete_removes_all_ordinary_matches_and_clipping_but_work_policy_stays_separate() {
        let mut session = fixture();
        session
            .edit("Setup", |doc, _| {
                doc.paths.push(NamedPath::new("Stored", Path::default()));
                doc.clipping_path = Some(photocraft_doc::ClippingPath { name: "Stored".into(), flatness: 0. });
                Ok(())
            })
            .unwrap();
        let work = session.active().unwrap().doc.work_path.clone();
        run(&mut session, "path.delete", &json!({"name":"Stored"})).unwrap();
        assert!(session.active().unwrap().doc.paths.is_empty());
        assert!(session.active().unwrap().doc.clipping_path.is_none());
        assert_eq!(session.active().unwrap().doc.work_path, work);
        session.execute("edit.undo", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().doc.paths.len(), 2);
        run(&mut session, "path.delete", &Value::Null).unwrap();
        assert!(session.active().unwrap().doc.work_path.is_none());
        assert_eq!(session.active().unwrap().doc.paths.len(), 2);
    }
    #[test]
    fn imported_dependency_refuses_before_mutation_and_keeps_source_provenance() {
        let mut doc = Document::new("Bound path", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let owner = Layer::raster("Owner", doc.pixel_format());
        let owner_id = owner.id;
        doc.layers.push(owner);
        let saved = NamedPath::new("Bound", Path::default());
        let path_id = saved.id;
        doc.paths.push(saved);
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.source_items.push(photocraft_doc::xcf_items::SourceItem {
            owner: ItemRef::Layer(owner_id),
            source: EditableSource::Vector { path: path_id, source_modified: false, configuration: Vec::new() },
            transform: None,
            cache: None,
        });
        doc.xcf_state = Some(state);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        assert!(run(&mut session, "path.delete", &json!({"pathId":path_id.0.to_string()})).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().doc.xcf_state, before.xcf_state);
    }
    #[test]
    fn ordinary_self_row_delete_never_grants_source_locked_or_referenced_path_deletion() {
        for defect in 0..5 {
            let mut session = fixture();
            session
                .edit("Ordinary current canonical paths", |doc, _| {
                    // MAX rekey is covered separately; this test isolates row authority.
                    doc.paths[0].id = PathId::fresh();
                    doc.xcf_state = Some(XcfState::default());
                    Ok(())
                })
                .unwrap();
            let id = session.active().unwrap().doc.paths[0].id;
            session
                .edit("Retained row obligation", |doc, _| {
                    let state = doc.xcf_state.as_mut().unwrap();
                    let item = state.items.iter_mut().find(|item| item.item == ItemRef::Path(id)).unwrap();
                    match defect {
                        0 => state.provenance.format = "xcf".into(),
                        1 => item.source_tattoo = Some(42),
                        2 => item.locks.position = true,
                        3 => state.selection.selected_paths.push(ItemRef::Path(id)),
                        _ => {
                            item.source_flags = 1;
                        }
                    }
                    Ok(())
                })
                .unwrap();
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let past = session.active().unwrap().history.past_len();
            assert!(run(&mut session, "path.delete", &json!({"pathId":id.0.to_string()})).is_err(), "defect={defect}");
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().doc, before);
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.past_len(), past);
        }
    }
    #[test]
    fn stale_bad_input_and_late_private_copy_cancel_preserve_real_redo() {
        let mut session = fixture();
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let redo = session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        for params in [
            json!({"path":geometry(),"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
            json!({"path":geometry(),"name":"x".repeat(NAME_LIMIT+1)}),
            json!({"path":{"subpaths":[{"knots":[[1,"bad"]]}]}}),
            json!({"path":geometry(),"pathId":"01"}),
        ] {
            assert!(run(&mut session, "path.set", &params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        }
        let mut path = Path::new(vec![Subpath { closed: false, op: PathOp::Combine, knots: vec![Knot::corner(1., 2.); 2048] }]);
        path.inverted = true;
        let preserved = path.clone();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 4;
        let control = Interrupt::cancel_only(&cancelled);
        assert!(matches!(copy_path(&path, "path.set", &control), Err(EngineError::Cancelled)));
        assert_eq!(path, preserved);
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
