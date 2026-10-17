//! Bounded canonical XCF metadata inspection. Persisted locators grant no authority.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{item_identity::ItemRef, xcf_items::EditableSource};
use serde_json::{Value, json};
const ID: &str = "xcf.inspect";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
pub(crate) fn short(text: &str) -> String {
    text.chars().take(256).collect()
}
pub(crate) fn reference(item: ItemRef) -> Value {
    let kind = match item {
        ItemRef::Layer(_) => "layer",
        ItemRef::Channel(_) => "channel",
        ItemRef::Mask(_) => "mask",
        ItemRef::Path(_) => "path",
    };
    json!({"kind":kind,"id":item.raw().to_string()})
}
pub(crate) fn page<T>(values: &[T], offset: usize, limit: usize, map: impl Fn(&T) -> Value) -> Value {
    let entries: Vec<_> = values.iter().skip(offset).take(limit).map(map).collect();
    json!({"total":values.len(),"offset":offset,"limit":limit,"entries":entries})
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    session.active().and_then(|state| state.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no imported XCF state".into())
}
fn inspect(session: &mut Session, params: &Value) -> Result<Value> {
    let params = params.as_object().ok_or_else(|| bad("expected an object"))?;
    if params.keys().any(|key| !matches!(key.as_str(), "offset" | "limit")) {
        return Err(bad("unknown inspection parameter"));
    }
    let integer = |key: &str, default, max| -> Result<usize> {
        let value = match params.get(key) {
            None => default,
            Some(value) => value.as_u64().ok_or_else(|| bad(format!("{key} must be an unsigned integer")))?,
        };
        if value > max {
            return Err(bad(format!("{key} exceeds inspection bound")));
        }
        usize::try_from(value).map_err(|_| bad("pagination overflow"))
    };
    let offset = integer("offset", 0, 100000)?;
    let limit = integer("limit", 64, 128)?;
    if limit == 0 {
        return Err(bad("limit must be positive"));
    }
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    doc.validate_xcf_state().map_err(|error| bad(error.to_string()))?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing imported XCF state"))?;
    let resolution = doc.effective_resolution();
    resolution.validate().map_err(|error| bad(error.to_string()))?;
    build_inspect_reply(doc, state, &resolution, offset, limit, None)
}
#[derive(Clone, Copy)]
pub(crate) struct ItemTextFacts<'a> {
    pub(crate) item: &'a photocraft_doc::xcf_state::ItemState,
    pub(crate) name_truncated: bool,
    pub(crate) epoch_truncated: bool,
}
fn truncated(item: &photocraft_doc::xcf_state::ItemState, epoch: bool, facts: Option<&[Option<ItemTextFacts<'_>>]>) -> bool {
    if let Some(fact) = facts.and_then(|facts| facts.iter().flatten().find(|fact| std::ptr::eq(fact.item, item))) {
        return if epoch { fact.epoch_truncated } else { fact.name_truncated };
    }
    // The resident caller supplies no facts, retaining its original full scan.
    // A paid caller's complete page coverage is checked before any reply copy.
    if epoch { item.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch.chars().count() > 256) } else { item.name.chars().count() > 256 }
}
#[derive(Clone, Copy)]
pub(crate) struct SetTextFacts<'a> {
    pub(crate) set: &'a photocraft_doc::xcf_editing::ItemSet,
    pub(crate) rule_truncated: bool,
}
fn copied_metadata_text(text: &str, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<String> {
    use photocraft_raster::backing::BackingError;
    if text.len() > 1 << 20 {
        return Err(BackingError::Length.into());
    }
    control.check(1)?;
    let mut output = String::new();
    output.try_reserve_exact(text.len()).map_err(|_| BackingError::Allocation)?;
    if output.capacity() > text.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    let mut start = 0usize;
    while start < text.len() {
        let mut end = start.checked_add(32 * 1024).ok_or(BackingError::Length)?.min(text.len());
        while !text.is_char_boundary(end) {
            end = end.checked_sub(1).ok_or(BackingError::Length)?;
        }
        if end <= start {
            return Err(BackingError::Length.into());
        }
        let part = text.get(start..end).ok_or(BackingError::Length)?;
        control.check(part.len() as u64 + 1)?;
        output.push_str(part);
        start = end;
    }
    control.check(1)?;
    Ok(output)
}
fn controlled_page<T>(
    values: &[T],
    offset: usize,
    limit: usize,
    control: &photocraft_raster::backing::ReadControl<'_>,
    mut map: impl FnMut(&T) -> Result<Value>,
) -> Result<Value> {
    use photocraft_raster::backing::BackingError;
    if limit == 0 || limit > 128 || offset > 100000 {
        return Err(BackingError::Length.into());
    }
    let count = values.len().saturating_sub(offset).min(limit);
    control.check(1)?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
    if entries.capacity() > count.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for value in values.iter().skip(offset).take(limit) {
        control.check(128)?;
        entries.push(map(value)?);
    }
    let mut result = json!({"total":values.len(),"offset":offset,"limit":limit});
    result.as_object_mut().ok_or(BackingError::Integrity)?.insert("entries".into(), Value::Array(entries));
    Ok(result)
}
fn controlled_resources(state: &photocraft_doc::XcfState, offset: usize, limit: usize, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<Value> {
    use photocraft_raster::backing::BackingError;
    controlled_page(&state.resources, offset, limit, control, |resource| {
        let hash = resource.content_hash.as_deref().map(|text| copied_metadata_text(text, control)).transpose()?;
        let mut row = json!({"id":resource.id.0.to_string(),"kind":resource.kind,"name":short(&resource.name),"internal":resource.internal,"hasLocator":resource.locator.is_some(),"ownedBytes":resource.owned_bytes.as_ref().map(|bytes|bytes.len()),"accessGranted":false});
        // Move the single bounded copy into the response; json!(hash) would
        // serialize/clone the entire temporary String again without chunk polls.
        row.as_object_mut().ok_or(BackingError::Integrity)?.insert("contentHash".into(), hash.map(Value::String).unwrap_or(Value::Null));
        Ok(row)
    })
}
fn controlled_sets(
    state: &photocraft_doc::XcfState,
    offset: usize,
    limit: usize,
    facts: &[SetTextFacts<'_>],
    control: &photocraft_raster::backing::ReadControl<'_>,
) -> Result<Value> {
    use photocraft_doc::xcf_editing::ItemSetRule;
    use photocraft_raster::backing::BackingError;
    controlled_page(&state.item_sets, offset, limit, control, |set| {
        let fact = facts.iter().find(|fact| std::ptr::eq(fact.set, set)).ok_or(BackingError::Integrity)?;
        let (kind, expression) = match &set.rule {
            ItemSetRule::Named => ("named", None),
            ItemSetRule::Text(text) => ("text", Some(text)),
            ItemSetRule::Regex(text) => ("regex", Some(text)),
            ItemSetRule::Glob(text) => ("glob", Some(text)),
        };
        let members = controlled_page(&set.members, offset, limit, control, |item| Ok(reference(*item)))?;
        let mut row = json!({"id":set.id.0.to_string(),"name":short(&set.name),"family":set.family,"memberCount":set.members.len(),"ruleKind":kind,"ruleExpression":expression.map(|text|short(text)),"ruleTruncated":fact.rule_truncated});
        row.as_object_mut().ok_or(BackingError::Integrity)?.insert("members".into(), members);
        Ok(row)
    })
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_plain_inspect_reply(
    doc: &photocraft_doc::Document,
    state: &photocraft_doc::XcfState,
    resolution: &photocraft_doc::PhysicalResolution,
    offset: usize,
    limit: usize,
    item_facts: &[Option<ItemTextFacts<'_>>],
    set_facts: &[SetTextFacts<'_>],
    control: &photocraft_raster::backing::ReadControl<'_>,
) -> Result<Value> {
    for set in state.item_sets.iter().skip(offset).take(limit) {
        control.check(1)?;
        if !set_facts.iter().any(|fact| std::ptr::eq(fact.set, set)) {
            return Err(bad("missing controlled set text facts"));
        }
    }
    build_inspect_reply_inner(doc, state, resolution, offset, limit, Some(item_facts), Some((set_facts, control)))
}
/// Exact canonical metadata response; callers independently validate the model.
/// Paid accounted callers must reserve/census all reply and serde temporaries.
pub(crate) fn build_inspect_reply(
    doc: &photocraft_doc::Document,
    state: &photocraft_doc::XcfState,
    resolution: &photocraft_doc::PhysicalResolution,
    offset: usize,
    limit: usize,
    facts: Option<&[Option<ItemTextFacts<'_>>]>,
) -> Result<Value> {
    build_inspect_reply_inner(doc, state, resolution, offset, limit, facts, None)
}
#[allow(clippy::too_many_arguments)]
fn build_inspect_reply_inner(
    doc: &photocraft_doc::Document,
    state: &photocraft_doc::XcfState,
    resolution: &photocraft_doc::PhysicalResolution,
    offset: usize,
    limit: usize,
    facts: Option<&[Option<ItemTextFacts<'_>>]>,
    plain: Option<(&[SetTextFacts<'_>], &photocraft_raster::backing::ReadControl<'_>)>,
) -> Result<Value> {
    if let Some(facts) = facts {
        for item in state.items.iter().skip(offset).take(limit) {
            if !facts.iter().flatten().any(|fact| std::ptr::eq(fact.item, item)) {
                return Err(bad("missing controlled item text facts"));
            }
        }
    }
    let items = page(&state.items, offset, limit, |item| {
        let composition=item.composition.as_ref().map(|policy|json!({"compatibilityEpoch":short(&policy.compatibility_epoch),"epochTruncated":truncated(item,true,facts),"sourceMode":policy.source_mode,"legacy":policy.legacy,"storedBlendSpace":policy.stored_blend_space,"storedCompositeSpace":policy.stored_composite_space,"storedOperator":policy.stored_operator,"resolved":policy.resolved,"passThrough":policy.pass_through}));
        let mask=item.mask.as_ref().map(|mask|json!({"owner":reference(mask.owner),"bounds":mask.bounds,"offset":mask.offset,"outsideFloat64Bits":format!("0x{:016x}",mask.outside_f64_bits),"enabled":mask.enabled,"shown":mask.shown,"editing":mask.editing,"linked":mask.linked}));
        json!({"item":reference(item.item),"name":short(&item.name),"nameTruncated":truncated(item,false,facts),"customName":item.custom_name,"visible":item.visible,"locks":item.locks,"parent":item.parent.map(reference),"sourceTattoo":item.source_tattoo,"sourceFlags":item.source_flags,"rasterBounds":item.raster_bounds,"hasComposition":item.composition.is_some(),"hasMask":item.mask.is_some(),"composition":composition,"mask":mask})
    });
    let paths =
        page(&doc.paths, offset, limit, |path| json!({"item":reference(ItemRef::Path(path.id)),"name":short(&path.name),"subpaths":path.path.subpaths.len()}));
    let sources = page(&state.source_items, offset, limit, |item| {
        let source = match &item.source {
            EditableSource::Text { content, markup, font, direction, source_modified, configuration, current_origin } => {
                json!({"kind":"text","characters":content.chars().count(),"content":short(content),"contentTruncated":content.chars().nth(256).is_some(),
                    "markup":markup.as_deref().map(short),"markupTruncated":markup.as_deref().is_some_and(|text|text.chars().nth(256).is_some()),
                    "direction":direction,"fontId":font.0.to_string(),"sourceModified":source_modified,"configurationMembers":configuration.len(),"currentOriginPx":current_origin})
            }
            EditableSource::Vector { path, source_modified, configuration } => {
                json!({"kind":"vector","pathId":path.0.to_string(),"sourceModified":source_modified,"configurationMembers":configuration.len()})
            }
            EditableSource::Linked { resource, configuration, current, .. } => {
                json!({"kind":"linked","resourceId":resource.0.to_string(),"configurationMembers":configuration.len(),
                    "current":current.as_ref().map(crate::xcf_link_geometry_cmds::current_geometry_json)})
            }
        };
        let cache = item
            .cache
            .as_ref()
            .map(|cache| json!({"blake3":cache.blake3,"width":cache.width,"height":cache.height,"format":cache.format,"bytes":cache.bytes.len()}));
        json!({"owner":reference(item.owner),"source":source,"hasTransform":item.transform.is_some(),"cache":cache})
    });
    let resources = if let Some((_, control)) = plain {
        controlled_resources(state, offset, limit, control)?
    } else {
        page(
            &state.resources,
            offset,
            limit,
            |resource| json!({"id":resource.id.0.to_string(),"kind":resource.kind,"name":short(&resource.name),"internal":resource.internal,"hasLocator":resource.locator.is_some(),"contentHash":resource.content_hash,"ownedBytes":resource.owned_bytes.as_ref().map(|bytes|bytes.len()),"accessGranted":false}),
        )
    };
    let stacks = page(
        &state.filter_stacks,
        offset,
        limit,
        |stack| json!({"owner":reference(stack.owner),"revision":stack.revision.to_string(),"nodes":stack.nodes.len()}),
    );
    let total = state.filter_stacks.iter().try_fold(0usize, |total, stack| total.checked_add(stack.nodes.len())).ok_or_else(|| bad("filter count overflow"))?;
    let entries: Vec<_>=state.filter_stacks.iter().flat_map(|stack|stack.nodes.iter().map(move |node|(stack.owner,node))).skip(offset).take(limit).map(|(owner,node)|json!({"id":node.id.0.to_string(),"owner":reference(owner),"operation":short(&node.operation.name),"storedVersion":short(&node.operation.stored_version),"schemaEpoch":short(&node.operation.schema_epoch),"enabled":node.enabled,"arguments":node.arguments.len(),"hasMask":node.mask.is_some(),"auxiliaryPorts":node.auxiliary.len(),"inspectCommand":"xcf.filter.inspect"})).collect();
    let filters = json!({"total":total,"offset":offset,"limit":limit,"entries":entries});
    let sets = if let Some((facts, control)) = plain {
        controlled_sets(state, offset, limit, facts, control)?
    } else {
        page(&state.item_sets, offset, limit, |set| {
            use photocraft_doc::xcf_editing::ItemSetRule;
            let (kind, expression) = match &set.rule {
                ItemSetRule::Named => ("named", None),
                ItemSetRule::Text(text) => ("text", Some(text)),
                ItemSetRule::Regex(text) => ("regex", Some(text)),
                ItemSetRule::Glob(text) => ("glob", Some(text)),
            };
            json!({"id":set.id.0.to_string(),"name":short(&set.name),"family":set.family,"memberCount":set.members.len(),"members":page(&set.members,offset,limit,|item|reference(*item)),"ruleKind":kind,"ruleExpression":expression.map(|text|short(text)),"ruleTruncated":expression.is_some_and(|text|text.chars().count()>256)})
        })
    };
    let selection = json!({"selected":page(&state.selection.selected,offset,limit,|item|reference(*item)),"paths":page(&state.selection.selected_paths,offset,limit,|item|reference(*item)),"active":state.selection.active.map(reference),"maskTarget":state.selection.mask_target.map(reference)});
    let floating = state.floating.as_ref().map(|floating| json!({"layerId":floating.layer.0.to_string(),"target":reference(floating.target)}));
    Ok(
        json!({"format":"xcf","formatVersion":state.provenance.format_version,"container":short(&state.provenance.container),"sampleType":format!("{:?}",doc.depth),"colorMode":format!("{:?}",doc.mode),"sampleEncoding":doc.sample_encoding,
        "resolution":{"xDpi":resolution.x_dpi,"yDpi":resolution.y_dpi,"customUnit":matches!(resolution.unit,photocraft_doc::PhysicalUnit::Custom(_))},
        "profiles":{"originalSourceIccBytes":doc.source_color_context.as_ref().and_then(|context|context.original_icc.as_ref()).map(|bytes|bytes.len()),"effectiveWorkingIccBytes":doc.icc_profile.as_ref().map(|bytes|bytes.len())},
        "items":items,"paths":paths,"sources":sources,"resources":resources,"filterStacks":stacks,"filters":filters,"savedSets":sets,"selection":selection,"floating":floating,"opaqueRecords":state.opaque.len(),
        "admission":"canonical metadata only; current rendering and editing require separate qualification","recoveryActions":[]}),
    )
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: ID,
            label: "Inspect Imported GIMP Project",
            menu: &[],
            shortcut: None,
            params: "{offset?:unsigned integer<=100000,limit?:1..128} -> bounded canonical metadata; IDs are decimal strings",
            enabled,
            run: inspect,
            journal: false,
        },
        CommandSpec {
            id: "xcf.source.inspect",
            label: "Inspect Imported GIMP Source Details",
            menu: &[],
            shortcut: None,
            params: "{owner:{kind:layer|channel|mask|path,id:canonical nonzero U64 string},offset?:0..100000,limit?:1..64} -> exact typed source metadata; bounded reply without resource authority",
            enabled,
            run: inspect_source,
            journal: false,
        },
    ]
}

fn inspect_source(session: &mut Session, params: &Value) -> Result<Value> {
    let (owner, offset, limit) = crate::inspect_xcf_typed::selector(params)?;
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    crate::inspect_xcf_typed::source_with_interrupt(doc, owner, offset, limit, &photocraft_raster::Interrupt::NONE)
}
