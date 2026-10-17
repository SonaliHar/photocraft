//! Explicit Named saved-set creation; no retained search expression is evaluated.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerContent, LayerId, XcfState,
    item_identity::{ChannelId, ItemRef, ItemSetId, PathId},
    xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
use std::collections::HashSet;

pub const CREATE: &str = "xcf.itemSet.create";
const MAX_ITEMS: usize = 10_000;
const MAX_MEMBERS: usize = 1000;
const MAX_VISITS: usize = 100_000;
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CREATE.into(), msg: message.into() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn canonical(value: &Value) -> Result<u64> {
    let text = value.as_str().ok_or_else(|| bad("identity must be a canonical nonzero decimal string"))?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad("identity must be a canonical nonzero decimal string"));
    }
    text.parse::<u64>().map_err(|_| bad("identity overflow"))
}
fn family(text: &str) -> Result<ItemFamily> {
    match text {
        "layer" => Ok(ItemFamily::Layer),
        "channel" => Ok(ItemFamily::Channel),
        "path" => Ok(ItemFamily::Path),
        _ => Err(bad("family must be layer, channel or path")),
    }
}
fn member(value: &Value, required: ItemFamily) -> Result<ItemRef> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 2 && object.contains_key("kind") && object.contains_key("id"))
        .ok_or_else(|| bad("member requires exactly kind and id"))?;
    let kind = family(object.get("kind").and_then(Value::as_str).ok_or_else(|| bad("member kind must be a string"))?)?;
    if kind != required {
        return Err(bad("member belongs to a different family"));
    }
    let raw = canonical(object.get("id").ok_or_else(|| bad("missing member id"))?)?;
    Ok(match kind {
        ItemFamily::Layer => ItemRef::Layer(LayerId(raw)),
        ItemFamily::Channel => ItemRef::Channel(ChannelId(raw)),
        ItemFamily::Path => ItemRef::Path(PathId(raw)),
    })
}
struct Request<'a> {
    id: ItemSetId,
    name: &'a str,
    family: ItemFamily,
    values: &'a [Value],
}
fn envelope(params: &Value) -> Result<Request<'_>> {
    let object = params.as_object().filter(|object| object.len() == 4).ok_or_else(|| bad("expected exactly setId, name, family and members"))?;
    if object.keys().any(|key| !matches!(key.as_str(), "setId" | "name" | "family" | "members")) {
        return Err(bad("unknown creation parameter"));
    }
    let id = ItemSetId(canonical(object.get("setId").ok_or_else(|| bad("missing setId"))?)?);
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= 1 << 20 && !name.contains('\0') && !name.trim().is_empty())
        .ok_or_else(|| bad("name must be bounded nonempty UTF-8 text"))?;
    let family = family(object.get("family").and_then(Value::as_str).ok_or_else(|| bad("family must be a string"))?)?;
    let values = object
        .get("members")
        .and_then(Value::as_array)
        .filter(|values| values.len() <= MAX_MEMBERS)
        .ok_or_else(|| bad("members must contain at most 1000 typed references"))?;
    // Before the dispatcher copies parameters, check every shape/type without allocation.
    for value in values {
        member(value, family)?;
    }
    Ok(Request { id, name, family, values })
}
fn context(session: &Session, background: bool) -> Result<()> {
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if session.documents().iter().any(|document| document.floating.is_some() || document.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())) {
        return Err(bad("complete or cancel floating selections before creating a saved set"));
    }
    if active.revision == u64::MAX {
        return Err(bad("document revision exhausted"));
    }
    if session.has_jobs() {
        return Err(bad("finish active jobs before creating a saved set"));
    }
    if background || session.command_is_background() {
        return Err(bad("set creation requires synchronous dispatch"));
    }
    if session.coalesce_request.is_some() || session.color_restrict.is_some() {
        return Err(bad("inherited edit context is unsupported"));
    }
    Ok(())
}
/// Root calls before floating normalization, parameter cloning and transient setup.
pub fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    context(session, background)?;
    envelope(params)?;
    Ok(())
}
fn add_visits(visits: &mut usize, count: usize) -> Result<()> {
    *visits = visits.checked_add(count).filter(|count| *count <= MAX_VISITS).ok_or_else(|| bad("current reference work limit"))?;
    Ok(())
}
fn reference_bounds(state: &XcfState, added: usize, control: &Interrupt<'_>) -> Result<()> {
    let mut visits = 0;
    for count in
        [state.items.len(), state.source_items.len(), state.filter_stacks.len(), state.filter_masks.len(), state.item_sets.len(), state.editable_indexed.len()]
    {
        if count > MAX_ITEMS {
            return Err(bad("canonical identity limit"));
        }
        add_visits(&mut visits, count)?;
    }
    add_visits(&mut visits, state.opaque.len())?;
    add_visits(&mut visits, state.selection.selected.len())?;
    add_visits(&mut visits, state.selection.selected_paths.len())?;
    add_visits(&mut visits, usize::from(state.selection.active.is_some()) + usize::from(state.selection.mask_target.is_some()))?;
    add_visits(&mut visits, usize::from(state.floating.is_some()) * 2)?;
    add_visits(&mut visits, state.filter_masks.len())?;
    for item in &state.items {
        poll(control)?;
        add_visits(&mut visits, usize::from(item.parent.is_some()) + usize::from(item.mask.is_some()))?;
    }
    for stack in &state.filter_stacks {
        poll(control)?;
        add_visits(&mut visits, stack.nodes.len())?;
        for node in &stack.nodes {
            poll(control)?;
            add_visits(&mut visits, usize::from(node.mask.is_some()))?;
            add_visits(&mut visits, node.auxiliary.len())?;
        }
    }
    for set in &state.item_sets {
        poll(control)?;
        add_visits(&mut visits, set.members.len())?;
    }
    add_visits(&mut visits, added.checked_add(1).ok_or_else(|| bad("current reference work limit"))?)
}
fn validate_current(document: &Document, id: ItemSetId, members: &[ItemRef], control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing canonical imported state"))?;
    if state.item_sets.len() >= MAX_ITEMS {
        return Err(bad("saved-set identity limit"));
    }
    reference_bounds(state, members.len(), control)?;
    let mut sets = HashSet::new();
    sets.try_reserve(state.item_sets.len()).map_err(|_| bad("set identity allocation limit"))?;
    for set in &state.item_sets {
        poll(control)?;
        if set.id.0 == 0 || !sets.insert(set.id) {
            return Err(bad("zero or duplicate saved-set identity"));
        }
        if set.id == id {
            return Err(bad("setId already exists"));
        }
    }
    let mut items = HashSet::new();
    items.try_reserve(state.items.len()).map_err(|_| bad("item identity allocation limit"))?;
    for item in &state.items {
        poll(control)?;
        if item.item.raw() == 0 || !items.insert(item.item) {
            return Err(bad("zero or duplicate canonical item"));
        }
    }
    let mut actual = HashSet::new();
    actual.try_reserve(MAX_ITEMS).map_err(|_| bad("current identity allocation limit"))?;
    let mut pending = Vec::new();
    pending.try_reserve_exact(MAX_ITEMS).map_err(|_| bad("hierarchy allocation limit"))?;
    if document.layers.len() > MAX_ITEMS {
        return Err(bad("current layer limit"));
    }
    pending.extend(document.layers.iter().map(|layer| (layer, 0usize)));
    let mut layer_count = 0usize;
    while let Some((layer, depth)) = pending.pop() {
        poll(control)?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH || layer.id.0 == 0 || !actual.insert(ItemRef::Layer(layer.id)) {
            return Err(bad("invalid current layer identity or depth"));
        }
        layer_count = layer_count.checked_add(1).filter(|count| *count <= MAX_ITEMS).ok_or_else(|| bad("current layer limit"))?;
        if let LayerContent::Group(group) = &layer.content {
            if pending.len().checked_add(layer_count).and_then(|count| count.checked_add(group.children.len())).is_none_or(|count| count > MAX_ITEMS) {
                return Err(bad("current layer limit"));
            }
            let depth = depth.checked_add(1).ok_or_else(|| bad("layer depth"))?;
            pending.extend(group.children.iter().map(|child| (child, depth)));
        }
    }
    if document.channels.len() > MAX_ITEMS || document.paths.len() > MAX_ITEMS {
        return Err(bad("current channel/path limit"));
    }
    if layer_count.checked_add(document.channels.len()).and_then(|count| count.checked_add(document.paths.len())).is_none_or(|count| count > MAX_ITEMS) {
        return Err(bad("current identity limit"));
    }
    actual
        .try_reserve(document.channels.len().checked_add(document.paths.len()).ok_or_else(|| bad("current identity limit"))?)
        .map_err(|_| bad("current identity allocation limit"))?;
    for channel in &document.channels {
        poll(control)?;
        if channel.id.0 == 0 || !actual.insert(ItemRef::Channel(channel.id)) {
            return Err(bad("invalid current channel identity"));
        }
    }
    for path in &document.paths {
        poll(control)?;
        if path.id.0 == 0 || !actual.insert(ItemRef::Path(path.id)) {
            return Err(bad("invalid current path identity"));
        }
    }
    for member in members {
        poll(control)?;
        if !items.contains(member) || !actual.contains(member) {
            return Err(bad("member is missing canonical metadata or current target"));
        }
    }
    document.validate_xcf_state_with(control).map_err(|error| bad(error.to_string()))?;
    Ok(())
}
fn prepared_charge(name: usize, members: usize, identity_slots: usize) -> Result<u64> {
    let name = u64::try_from(name).map_err(|_| bad("new set allocation limit"))?;
    let members = u64::try_from(members).map_err(|_| bad("new set allocation limit"))?;
    let slots = u64::try_from(identity_slots).map_err(|_| bad("new set allocation limit"))?;
    name.checked_add(members.checked_mul(std::mem::size_of::<ItemRef>() as u64).ok_or_else(|| bad("new set allocation limit"))?)
        .and_then(|bytes| slots.checked_mul(64).and_then(|slots| bytes.checked_add(slots)))
        .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<ItemSet>() as u64))
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or_else(|| bad("new set allocation limit"))
}
/// Explicit Named metadata creation; registered dispatch supplies authorization and
/// the pre-floating/context guard. No source locator, rule or raster is interpreted.
pub fn create_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    context(session, false)?;
    poll(control)?;
    let request = envelope(params)?;
    let document = &session.active().ok_or(EngineError::NoDocument)?.doc;
    document.validate_resident().map_err(|error| bad(format!("resident metadata editing required: {error}")))?;
    // Before prepared allocation: retained raster plus COW bookkeeping and a
    // conservative name/member/duplicate-index/new-record reservation.
    let footprint = document
        .resident_raster_footprint_with(control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(format!("retained raster admission: {error}")) })?;
    let raster_peak = footprint.retained_bytes.checked_add(footprint.clone_metadata_bytes).ok_or_else(|| bad("retained raster/COW limit"))?;
    let anticipated = prepared_charge(request.name.len(), request.values.len(), request.values.len())?;
    if raster_peak.checked_add(anticipated).is_none_or(|bytes| bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("new set and retained raster/COW limit"));
    }
    let mut members = Vec::new();
    members.try_reserve_exact(request.values.len()).map_err(|_| bad("membership allocation limit"))?;
    let mut selected = HashSet::new();
    selected.try_reserve(request.values.len()).map_err(|_| bad("membership allocation limit"))?;
    for value in request.values {
        poll(control)?;
        let target = member(value, request.family)?;
        if !selected.insert(target) {
            return Err(bad("duplicate member"));
        }
        members.push(target);
    }
    validate_current(document, request.id, &members, control)?;
    let mut name = String::new();
    name.try_reserve_exact(request.name.len()).map_err(|_| bad("name allocation limit"))?;
    name.push_str(request.name);
    let allocated = prepared_charge(name.capacity(), members.capacity(), selected.capacity())?;
    if raster_peak.checked_add(allocated).is_none_or(|bytes| bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("new set and retained raster/COW limit"));
    }
    drop(selected);
    // This raster+new-record charge is not complete config/resource accounting
    // or a process RSS guarantee. Prepared buffers move into the new record.
    let prepared = ItemSet { id: request.id, name, family: request.family, rule: ItemSetRule::Named, members };
    let response = json!({"setId":request.id.0,"setIdString":request.id.0.to_string(),"created":true,"members":prepared.members.len()});
    session.edit_with_interrupt("Create Saved Item Set", control, |document, _| {
        validate_current(document, prepared.id, &prepared.members, control)?;
        poll(control)?;
        let state = document.xcf_state.as_mut().ok_or_else(|| bad("missing canonical imported state"))?;
        state.item_sets.try_reserve_exact(1).map_err(|_| bad("set allocation limit"))?;
        state.item_sets.push(prepared);
        document.validate_xcf_state_with(control).map_err(|error| bad(error.to_string()))?;
        poll(control)?;
        Ok(())
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session, false).map_err(|error| error.to_string())?;
    session.active().and_then(|active| active.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no canonical imported state".into())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    create_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CREATE,
        label: "Create Saved Item Set",
        menu: &[],
        shortcut: None,
        params: r#"{setId:canonical nonzero decimal string,name:bounded UTF-8 string,family:layer|channel|path,members:[{kind:family,id:canonical string}]} -> {setId:integer,setIdString:string,created:true,members:count} (Named only, at most 1000 ordered unique current members; explicit unused setId)"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, PixelFormat, SampleType, Size, xcf_state::ItemState};

    fn fixture() -> (Session, [LayerId; 2]) {
        use photocraft_doc::{
            item_identity::ResourceId,
            xcf_items::{EditableSource, SourceCache, SourceItem},
            xcf_resources::{ResourceKind, ResourceReference},
        };
        let first = Layer::raster("Own first", PixelFormat::RGBA16);
        let second = Layer::raster("Own second", PixelFormat::RGBA16);
        let ids = [first.id, second.id];
        let item = |id, name: &str, tattoo| ItemState {
            item: ItemRef::Layer(id),
            name: name.into(),
            custom_name: true,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: Some(tattoo),
            source_flags: 0x40,
            raster_bounds: None,
            mask: None,
            composition: None,
        };
        let mut document = Document::new("Own set creation", Size::new(2, 2), ColorMode::Rgb, SampleType::U16);
        document.layers = vec![first, second];
        document.xcf_state = Some(XcfState {
            items: vec![item(ids[0], "Own first", 41), item(ids[1], "Own second", 42)],
            item_sets: vec![ItemSet {
                id: ItemSetId(7),
                name: "Retained set".into(),
                family: ItemFamily::Layer,
                rule: ItemSetRule::Named,
                members: vec![ItemRef::Layer(ids[0])],
            }],
            resources: vec![ResourceReference {
                id: ResourceId(1),
                kind: ResourceKind::LinkedImage,
                name: "Own inert source".into(),
                collection: None,
                internal: false,
                locator: Some("own:never-read".into()),
                content_hash: None,
                owned_bytes: None,
                resolution: None,
            }],
            source_items: vec![SourceItem {
                owner: ItemRef::Layer(ids[0]),
                source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: Vec::new(), current: None },
                transform: None,
                cache: Some(SourceCache {
                    bytes: vec![1, 0, 2, 0, 3, 0, 255, 255].into(),
                    blake3: blake3::hash(&[1, 0, 2, 0, 3, 0, 255, 255]).to_hex().to_string(),
                    width: 1,
                    height: 1,
                    format: PixelFormat::RGBA16,
                }),
            }],
            ..Default::default()
        });
        document.validate_xcf_state().unwrap();
        let mut session = Session::new();
        session.open_document(document, None).unwrap();
        (session, ids)
    }
    fn request(id: u64, ids: [LayerId; 2]) -> Value {
        json!({"setId":id.to_string(),"name":"Own ordered set","family":"layer","members":[{"kind":"layer","id":ids[1].0.to_string()},{"kind":"layer","id":ids[0].0.to_string()}]})
    }
    #[test]
    fn ordered_creation_preserves_existing_document_and_native_one_undo_redo() {
        let (mut session, ids) = fixture();
        let before = session.active().unwrap().doc.clone();
        // History labels include the legitimate current Open state before any edit.
        let baseline_labels = session.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
        let baseline_past = session.active().unwrap().history.past_len();
        assert_eq!(baseline_labels, vec!["Open"]);
        assert_eq!(baseline_past, 0);
        let result = create_with_interrupt(&mut session, &request(9, ids), &Interrupt::NONE).unwrap();
        assert_eq!(result["setIdString"], "9");
        let after = session.active().unwrap().doc.clone();
        let set = after.xcf_state.as_ref().unwrap().item_sets.last().unwrap();
        assert_eq!(set.rule, ItemSetRule::Named);
        assert_eq!(set.members, vec![ItemRef::Layer(ids[1]), ItemRef::Layer(ids[0])]);
        let mut expected = (*before).clone();
        expected.xcf_state.as_mut().unwrap().item_sets.push(set.clone());
        assert_eq!(*after, expected);
        let mut expected_labels = baseline_labels.clone();
        expected_labels.push("Create Saved Item Set".into());
        assert_eq!(session.active().unwrap().history.entry_labels().collect::<Vec<_>>(), expected_labels.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(session.active().unwrap().history.past_len(), baseline_past + 1);
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened, *after);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.past_len(), baseline_past);
        assert_eq!(session.active().unwrap().history.entry_labels().collect::<Vec<_>>(), baseline_labels.iter().map(String::as_str).collect::<Vec<_>>());
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&after));
        assert_eq!(session.active().unwrap().history.past_len(), baseline_past + 1);
        assert_eq!(session.active().unwrap().history.entry_labels().collect::<Vec<_>>(), expected_labels.iter().map(String::as_str).collect::<Vec<_>>());
    }
    #[test]
    fn collision_malformed_missing_and_duplicate_members_preserve_real_redo() {
        let (mut session, ids) = fixture();
        create_with_interrupt(&mut session, &request(9, ids), &Interrupt::NONE).unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let baseline_labels = session.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
        let baseline_past = session.active().unwrap().history.past_len();
        let baseline_redo = session.active().unwrap().history.can_redo();
        let revision = session.active().unwrap().revision;
        let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let mut duplicate = request(10, ids);
        duplicate["members"] = json!([{"kind":"layer","id":ids[0].0.to_string()},{"kind":"layer","id":ids[0].0.to_string()}]);
        let mut missing = request(10, ids);
        missing["members"] = json!([{"kind":"layer","id":"18446744073709551615"}]);
        for params in [
            request(7, ids),
            duplicate,
            missing,
            json!({"setId":"09","name":"Own","family":"layer","members":[]}),
            json!({"setId":"10","name":"Own","family":"layer","members":[{"kind":"channel","id":"1"}]}),
            json!({"setId":"10","name":"Own","family":"layer","members":[],"rule":"regex"}),
        ] {
            assert!(create_with_interrupt(&mut session, &params, &Interrupt::NONE).is_err());
            let active = session.active().unwrap();
            assert!(active.doc.shares_state(&before));
            assert_eq!(active.revision, revision);
            assert!(active.history.can_redo());
            assert_eq!(active.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), baseline_labels);
            assert_eq!(active.history.past_len(), baseline_past);
            assert_eq!(active.history.can_redo(), baseline_redo);
            assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), bytes);
        }
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().item_sets.last().unwrap().id, ItemSetId(9));
    }
    #[test]
    fn late_cancel_preserves_native_state_revision_and_real_redo() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let seed = |session: &mut Session, ids| {
            create_with_interrupt(session, &request(9, ids), &Interrupt::NONE).unwrap();
            session.execute("edit.undo", json!({})).unwrap();
        };
        let (mut probe, ids) = fixture();
        seed(&mut probe, ids);
        let polls = AtomicUsize::new(0);
        let count = || {
            polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        create_with_interrupt(&mut probe, &request(10, ids), &Interrupt::cancel_only(&count)).unwrap();
        let last = polls.load(Ordering::Relaxed).checked_sub(1).unwrap();
        let (mut session, ids) = fixture();
        seed(&mut session, ids);
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= last;
        assert!(create_with_interrupt(&mut session, &request(10, ids), &Interrupt::cancel_only(&cancel)).is_err());
        let active = session.active().unwrap();
        assert!(active.doc.shares_state(&before));
        assert_eq!(active.revision, revision);
        assert!(active.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), bytes);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().item_sets.last().unwrap().id, ItemSetId(9));
    }

    #[test]
    fn exhausted_revision_refuses_set_creation_before_history_change() {
        let (mut session, ids) = fixture();
        session.active_mut().unwrap().revision = u64::MAX;
        let before = session.active().unwrap().doc.clone();
        let baseline_labels = session.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
        let baseline_past = session.active().unwrap().history.past_len();
        let baseline_redo = session.active().unwrap().history.can_redo();
        assert!(session.execute(CREATE, request(10, ids)).is_err());
        let active = session.active().unwrap();
        assert!(active.doc.shares_state(&before));
        assert_eq!(active.revision, u64::MAX);
        assert_eq!(active.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), baseline_labels);
        assert_eq!(active.history.past_len(), baseline_past);
        assert_eq!(active.history.can_redo(), baseline_redo);
    }
}
