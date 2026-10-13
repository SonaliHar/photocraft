//! Explicit missing-identity PNG replacement; bytes and current raster commit together.
use crate::{
    EngineError, Result, Session,
    commands::CommandSpec,
    document_snapshot::WeakDocumentSnapshot,
    xcf_resources::{RecoveryPolicy, ServiceResourceBytes},
};
use photocraft_doc::{
    DocId, Document, LayerContent, LayerId,
    item_identity::{ItemRef, ResourceId},
    xcf_filters::{FilterArgument, FilterValue},
    xcf_items::EditableSource,
    xcf_resources::{ResourceFidelity, ResourceKind, ResourceResolution},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

const ID: &str = "xcf.link.substitute";
const RECOVER_ID: &str = "xcf.link.recover";
const MAX_INPUT: usize = 8 << 20;
const MAX_VISITS: usize = 100_000;
const MAX_DEPTH: usize = 32;

fn bad(code: &str) -> EngineError {
    EngineError::Other(format!("{ID}.{code}"))
}
fn checked(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn operation_error(control: &Interrupt<'_>, code: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(code) }
}

/// Picker identity is ephemeral; locators and serialized JSON cannot recreate this snapshot.
#[derive(Clone, Debug)]
pub struct LinkedSubstitutionTarget {
    document: DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    owner: LayerId,
    resource: ResourceId,
}

/// Exact recovery uses a separate opaque target and requires retained original identity.
#[derive(Clone, Debug)]
pub struct LinkedRecoveryTarget(LinkedSubstitutionTarget);

pub type LinkedRecoveryResult = LinkedSubstitutionResult;

fn recovery_error(error: EngineError) -> EngineError {
    match error {
        EngineError::Other(message) if message.starts_with(ID) => EngineError::Other(message.replacen(ID, RECOVER_ID, 1)),
        error => error,
    }
}

/// Prepared before publication, including the registered response allocation.
#[derive(Debug)]
pub struct LinkedSubstitutionResult {
    pub document: DocId,
    pub owner: LayerId,
    pub resource: ResourceId,
    pub payload_hash: String,
    pub fidelity: ResourceFidelity,
    pub applied: bool,
    response: Value,
}
impl LinkedSubstitutionResult {
    pub fn into_response(self) -> Value {
        self.response
    }
}

fn context(session: &Session) -> Result<()> {
    if session.coalesce_request.is_some() {
        return Err(bad("coalescing"));
    }
    if session.color_restrict.is_some() || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() {
        return Err(bad("context"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.floating.is_some() {
        return Err(bad("floating"));
    }
    if state.doc.slices.list.len() > MAX_VISITS {
        return Err(bad("limit"));
    }
    layer_bounds(&state.doc)?;
    if state.doc.accounted().is_some()
        || state.revision == u64::MAX
        || state.doc.slices.list.iter().any(|slice| slice.origin == photocraft_doc::slices::SliceOrigin::Layer)
    {
        return Err(bad("context"));
    }
    state.doc.validate_resident().map_err(|_| bad("context"))?;
    Ok(())
}

fn layer_bounds(document: &Document) -> Result<()> {
    let mut frames: [Option<std::slice::Iter<'_, photocraft_doc::Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    let mut depth = 0usize;
    *frames.first_mut().ok_or_else(|| bad("limit"))? = Some(document.layers.iter());
    let mut visits = 0usize;
    loop {
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth = depth.checked_sub(1).ok_or_else(|| bad("limit"))?;
            continue;
        };
        visits = visits.checked_add(1).filter(|count| *count <= 10_000).ok_or_else(|| bad("limit"))?;
        if let LayerContent::Group(children) = &layer.content {
            depth = depth.checked_add(1).filter(|value| *value <= photocraft_doc::MAX_GROUP_DEPTH).ok_or_else(|| bad("limit"))?;
            *frames.get_mut(depth).ok_or_else(|| bad("limit"))? = Some(children.children.iter());
        }
    }
    Ok(())
}

/// Called by the NEW-ID dispatcher before floating normalization/transient assignment.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    dispatch_with_policy(session, params, background, "substitute")
}

pub(crate) fn dispatch_recovery_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    dispatch_with_policy(session, params, background, "exact").map_err(recovery_error)
}

fn dispatch_with_policy(session: &Session, params: &Value, background: bool, policy: &str) -> Result<()> {
    if params.as_object().is_some_and(|object| object.contains_key("coalesce")) {
        return Err(bad("coalescing"));
    }
    if background {
        return Err(bad("context"));
    }
    context(session)?;
    if policy == "substitute" { envelope(params) } else { envelope_with_policy(params, policy) }
}

fn relation(document: &Document, owner: LayerId, resource: ResourceId) -> Result<()> {
    relation_with_policy(document, owner, resource, RecoveryPolicy::AllowSubstitution)
}

fn relation_with_policy(document: &Document, owner: LayerId, resource: ResourceId, policy: RecoveryPolicy) -> Result<()> {
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("unsupported"))?;
    if state.items.len() > 10_000 || state.source_items.len() > MAX_VISITS || state.resources.len() > MAX_VISITS {
        return Err(bad("limit"));
    }
    let source = state.source_items.iter().find(|source| source.owner == ItemRef::Layer(owner)).ok_or_else(|| bad("stale-target"))?;
    if !matches!(source.source, EditableSource::Linked { resource: linked, .. } if linked == resource) {
        return Err(bad("stale-target"));
    }
    let retained = state.resources.iter().find(|entry| entry.id == resource).ok_or_else(|| bad("stale-target"))?;
    if retained.kind != ResourceKind::LinkedImage {
        return Err(bad("unsupported"));
    }
    if policy == RecoveryPolicy::AllowSubstitution && retained.content_hash.is_some() {
        return Err(bad("original-identity"));
    }
    if policy == RecoveryPolicy::Exact
        && retained.content_hash.as_ref().is_none_or(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(bad("original-identity"));
    }
    let layer = document.layer(owner).ok_or_else(|| bad("stale-target"))?;
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Err(bad("unsupported"));
    }
    crate::xcf_item_locks::effective_layer_locks(document, owner)
        .and_then(|locks| locks.require(crate::xcf_item_locks::EditKind::Content))
        .map_err(|_| bad("locked"))?;
    Ok(())
}

pub fn capture_target(session: &Session, owner: LayerId, resource: ResourceId) -> Result<LinkedSubstitutionTarget> {
    context(session)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    relation(&state.doc, owner, resource)?;
    Ok(LinkedSubstitutionTarget { document: state.doc.id, revision: state.revision, snapshot: state.doc.downgrade(), owner, resource })
}

fn validate_target(session: &Session, target: &LinkedSubstitutionTarget) -> Result<()> {
    validate_target_with_policy(session, target, RecoveryPolicy::AllowSubstitution)
}

fn validate_target_with_policy(session: &Session, target: &LinkedSubstitutionTarget, policy: RecoveryPolicy) -> Result<()> {
    context(session)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let old = target.snapshot.upgrade().ok_or_else(|| bad("stale-target"))?;
    if state.doc.id != target.document || state.revision != target.revision || !old.shares_state(&state.doc) {
        return Err(bad("stale-target"));
    }
    relation_with_policy(&state.doc, target.owner, target.resource, policy)
}

pub fn capture_recovery_target(session: &Session, owner: LayerId, resource: ResourceId) -> Result<LinkedRecoveryTarget> {
    (|| {
        context(session)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        relation_with_policy(&state.doc, owner, resource, RecoveryPolicy::Exact)?;
        Ok(LinkedRecoveryTarget(LinkedSubstitutionTarget {
            document: state.doc.id,
            revision: state.revision,
            snapshot: state.doc.downgrade(),
            owner,
            resource,
        }))
    })()
    .map_err(recovery_error)
}

/// Guard a completed picker, then build a bounded request for immediate registered dispatch.
/// This performs no edit, refocus, authority acquisition or locator read. Do not await
/// between this call and executing `xcf.link.substitute`; dispatch repeats current guards.
pub fn request_for_target(session: &Session, target: &LinkedSubstitutionTarget, bytes: &[u8]) -> Result<Value> {
    validate_target(session, target)?;
    request(target, bytes, "substitute")
}

/// No await between this completed-picker guard and immediate registered exact dispatch.
pub fn request_for_recovery_target(session: &Session, target: &LinkedRecoveryTarget, bytes: &[u8]) -> Result<Value> {
    validate_target_with_policy(session, &target.0, RecoveryPolicy::Exact).and_then(|()| request(&target.0, bytes, "exact")).map_err(recovery_error)
}

fn request(target: &LinkedSubstitutionTarget, bytes: &[u8], policy: &str) -> Result<Value> {
    if bytes.is_empty() || bytes.len() > MAX_INPUT {
        return Err(bad("limit"));
    }
    let length = bytes.len().checked_mul(2).ok_or_else(|| bad("limit"))?;
    let mut hex = String::new();
    hex.try_reserve_exact(length).map_err(|_| bad("limit"))?;
    for byte in bytes {
        for nibble in [byte >> 4, byte & 15] {
            hex.push(char::from(if nibble < 10 { b'0' + nibble } else { b'a' + (nibble - 10) }));
        }
    }
    Ok(json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string(),
        "layerId":target.owner.0.to_string(),"resourceId":target.resource.0.to_string(),"policy":policy,"payloadHex":hex}))
}

fn visit(count: &mut usize, additional: usize, control: &Interrupt<'_>) -> Result<()> {
    checked(control)?;
    *count = count.checked_add(additional).filter(|total| *total <= MAX_VISITS).ok_or_else(|| bad("limit"))?;
    Ok(())
}
fn arguments(arguments: &[FilterArgument], resource: ResourceId, depth: usize, visits: &mut usize, control: &Interrupt<'_>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(bad("limit"));
    }
    visit(visits, arguments.len(), control)?;
    for argument in arguments {
        checked(control)?;
        match &argument.value {
            FilterValue::Resource(id) if *id == resource => return Err(bad("shared-resource")),
            FilterValue::Color { profile: Some(id), .. } if *id == resource => return Err(bad("shared-resource")),
            FilterValue::Config(values) => self::arguments(values, resource, depth + 1, visits, control)?,
            FilterValue::Graph(graph) => {
                visit(visits, graph.nodes.len(), control)?;
                visit(visits, graph.edges.len(), control)?;
                for node in &graph.nodes {
                    self::arguments(&node.arguments, resource, depth + 1, visits, control)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn single_consumer(document: &Document, target: &LinkedSubstitutionTarget, control: &Interrupt<'_>) -> Result<()> {
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("unsupported"))?;
    let mut visits = 0;
    visit(&mut visits, state.source_items.len(), control)?;
    visit(&mut visits, state.filter_stacks.len(), control)?;
    let mut found = false;
    for source in &state.source_items {
        checked(control)?;
        let configuration = match &source.source {
            EditableSource::Linked { resource, configuration, .. } => {
                if *resource == target.resource {
                    if found || source.owner != ItemRef::Layer(target.owner) {
                        return Err(bad("shared-resource"));
                    }
                    found = true;
                }
                configuration
            }
            EditableSource::Text { font, configuration, .. } => {
                if *font == target.resource {
                    return Err(bad("shared-resource"));
                }
                configuration
            }
            EditableSource::Vector { configuration, .. } => configuration,
        };
        arguments(configuration, target.resource, 0, &mut visits, control)?;
    }
    for stack in &state.filter_stacks {
        visit(&mut visits, stack.nodes.len(), control)?;
        for node in &stack.nodes {
            arguments(&node.arguments, target.resource, 0, &mut visits, control)?;
        }
    }
    if !found {
        return Err(bad("stale-target"));
    }
    Ok(())
}

/// One synchronous resident transaction; no locator reads, queue, nested edit or prior upload.
pub fn substitute_with_interrupt(
    session: &mut Session,
    target: LinkedSubstitutionTarget,
    input: ServiceResourceBytes,
    control: &Interrupt<'_>,
) -> Result<LinkedSubstitutionResult> {
    apply_with_interrupt(session, target, input, RecoveryPolicy::AllowSubstitution, control)
}

/// Original resource bytes and qualified current raster publish in one resident edit.
/// Exact resource identity does not establish whole-document fidelity.
pub fn recover_with_interrupt(
    session: &mut Session,
    target: LinkedRecoveryTarget,
    input: ServiceResourceBytes,
    control: &Interrupt<'_>,
) -> Result<LinkedRecoveryResult> {
    apply_with_interrupt(session, target.0, input, RecoveryPolicy::Exact, control).map_err(recovery_error)
}

fn apply_with_interrupt(
    session: &mut Session,
    target: LinkedSubstitutionTarget,
    input: ServiceResourceBytes,
    policy: RecoveryPolicy,
    control: &Interrupt<'_>,
) -> Result<LinkedSubstitutionResult> {
    checked(control)?;
    validate_target_with_policy(session, &target, policy)?;
    if input.kind() != &ResourceKind::LinkedImage {
        return Err(bad("unsupported"));
    }
    if input.len() == 0 || input.len() > MAX_INPUT {
        return Err(bad("limit"));
    }
    let document = &session.active().ok_or(EngineError::NoDocument)?.doc;
    single_consumer(document, &target, control)?;
    // Admit retained raster backing and the COW metadata clone before editing.
    // Decoder/metadata budgets remain separate; this is not an RSS guarantee.
    let footprint = document.resident_raster_footprint_with(control).map_err(|_| operation_error(control, "limit"))?;
    let retained_peak = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes)
        .and_then(|bytes| bytes.checked_add(input.len() as u64))
        .ok_or_else(|| bad("limit"))?;
    if retained_peak > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(bad("limit"));
    }
    document.validate_xcf_state_with(control).map_err(|_| operation_error(control, "unsupported"))?;
    crate::xcf_resources::check_recovery_total(document, target.resource, input.len()).map_err(|_| operation_error(control, "limit"))?;
    let digest = crate::xcf_resources::hash_recovery_input(&input, control).map_err(|_| operation_error(control, "unsupported"))?;
    if policy == RecoveryPolicy::Exact
        && document
            .xcf_state
            .as_ref()
            .and_then(|state| state.resources.iter().find(|resource| resource.id == target.resource))
            .and_then(|resource| resource.content_hash.as_deref())
            .is_none_or(|expected| !expected.eq_ignore_ascii_case(&digest))
    {
        return Err(bad("hash-mismatch"));
    }
    let (digest, fidelity) = crate::xcf_resources::prepare_recovery_identity(document, target.resource, &input, policy, digest, control)
        .map_err(|_| operation_error(control, "unsupported"))?;
    if fidelity
        != match policy {
            RecoveryPolicy::Exact => ResourceFidelity::Exact,
            RecoveryPolicy::AllowSubstitution => ResourceFidelity::Substituted,
        }
    {
        return Err(bad("original-identity"));
    }
    validate_target_with_policy(session, &target, policy)?;
    let label = match policy {
        RecoveryPolicy::Exact => "Recover Exact XCF Linked Image",
        RecoveryPolicy::AllowSubstitution => "Replace XCF Linked Image (Reduced)",
    };
    session.edit_with_interrupt(label, control, move |document, _| {
        let retained = document
            .xcf_state
            .as_mut()
            .and_then(|state| state.resources.iter_mut().find(|entry| entry.id == target.resource))
            .ok_or_else(|| bad("stale-target"))?;
        retained.owned_bytes = Some(input.into_bytes().into());
        retained.resolution = Some(ResourceResolution { fidelity, payload_hash: digest.clone() });
        let source = document
            .xcf_state
            .as_ref()
            .and_then(|state| state.source_items.iter().find(|source| source.owner == ItemRef::Layer(target.owner)))
            .ok_or_else(|| bad("stale-target"))?;
        let (owner, current) =
            photocraft_compose::gimp_link_items::prepare_current_source(document, source, control).map_err(|_| operation_error(control, "unsupported"))?;
        if owner != target.owner {
            return Err(bad("stale-target"));
        }
        let layer = document.layer_mut(owner).ok_or_else(|| bad("stale-target"))?;
        if !matches!(layer.content, LayerContent::Raster(_)) {
            return Err(bad("unsupported"));
        }
        layer.content = LayerContent::Raster(current);
        let source = document
            .xcf_state
            .as_mut()
            .and_then(|state| state.source_items.iter_mut().find(|source| source.owner == ItemRef::Layer(owner)))
            .ok_or_else(|| bad("stale-target"))?;
        let EditableSource::Linked { last_verified_hash, .. } = &mut source.source else {
            return Err(bad("unsupported"));
        };
        *last_verified_hash = Some(digest.clone());
        document.validate_xcf_state_with(control).map_err(|_| operation_error(control, "unsupported"))?;
        photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(document, control).map_err(|_| operation_error(control, "unsupported"))?;
        let response = match policy {
            RecoveryPolicy::AllowSubstitution => {
                json!({"documentId":target.document.0.to_string(),"layerId":owner.0.to_string(),"resourceId":target.resource.0.to_string(),
                "byteIdentity":"Substituted","assessment":"Reduced","payloadHash":digest,"applied":true})
            }
            RecoveryPolicy::Exact => {
                json!({"documentId":target.document.0.to_string(),"layerId":owner.0.to_string(),"resourceId":target.resource.0.to_string(),
                "byteIdentity":"Exact","assessmentCommand":"xcf.assess","payloadHash":digest,"applied":true})
            }
        };
        Ok(LinkedSubstitutionResult { document: target.document, owner, resource: target.resource, payload_hash: digest, fidelity, applied: true, response })
    })
}

fn decimal(params: &Value, name: &str, zero: bool) -> Result<u64> {
    let text = params.get(name).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad("parameters"));
    }
    let value = text.parse::<u64>().map_err(|_| bad("parameters"))?;
    if (!zero && value == 0) || value.to_string() != text {
        return Err(bad("parameters"));
    }
    Ok(value)
}
fn nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(bad("parameters")),
    }
}
fn envelope(params: &Value) -> Result<()> {
    envelope_with_policy(params, "substitute")
}

fn envelope_with_policy(params: &Value, policy: &str) -> Result<()> {
    let object = params.as_object().ok_or_else(|| bad("parameters"))?;
    if object.len() != 6
        || object.keys().any(|key| !matches!(key.as_str(), "documentId" | "expectedRevision" | "layerId" | "resourceId" | "policy" | "payloadHex"))
        || params.get("policy").and_then(Value::as_str) != Some(policy)
    {
        return Err(bad("parameters"));
    }
    for name in ["documentId", "layerId", "resourceId"] {
        decimal(params, name, false)?;
    }
    decimal(params, "expectedRevision", true)?;
    let hex = params.get("payloadHex").and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if hex.len() > MAX_INPUT * 2 {
        return Err(bad("limit"));
    }
    if hex.is_empty() || hex.len() % 2 != 0 {
        return Err(bad("parameters"));
    }
    Ok(())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    run_with_policy(session, params, RecoveryPolicy::AllowSubstitution)
}

fn run_recovery(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_recovery_preflight(session, params, false)?;
    run_with_policy(session, params, RecoveryPolicy::Exact).map_err(recovery_error)
}

fn run_with_policy(session: &mut Session, params: &Value, policy: RecoveryPolicy) -> Result<Value> {
    let policy_name = match policy {
        RecoveryPolicy::Exact => "exact",
        RecoveryPolicy::AllowSubstitution => "substitute",
    };
    dispatch_with_policy(session, params, false, policy_name)?;
    let object = params.as_object().ok_or_else(|| bad("parameters"))?;
    if object.len() != 6
        || object.keys().any(|key| !matches!(key.as_str(), "documentId" | "expectedRevision" | "layerId" | "resourceId" | "policy" | "payloadHex"))
        || params.get("policy").and_then(Value::as_str) != Some(policy_name)
    {
        return Err(bad("parameters"));
    }
    let document = DocId(decimal(params, "documentId", false)?);
    let revision = decimal(params, "expectedRevision", true)?;
    let owner = LayerId(decimal(params, "layerId", false)?);
    let resource = ResourceId(decimal(params, "resourceId", false)?);
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id != document || state.revision != revision {
        return Err(bad("stale-target"));
    }
    let target = match policy {
        RecoveryPolicy::Exact => capture_recovery_target(session, owner, resource)?.0,
        RecoveryPolicy::AllowSubstitution => capture_target(session, owner, resource)?,
    };
    let hex = params.get("payloadHex").and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if hex.is_empty() || hex.len() % 2 != 0 {
        return Err(bad("parameters"));
    }
    if hex.len() > MAX_INPUT * 2 {
        return Err(bad("limit"));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(hex.len() / 2).map_err(|_| bad("limit"))?;
    for &[hi, lo] in hex.as_bytes().as_chunks::<2>().0 {
        bytes.push(nibble(hi)? * 16 + nibble(lo)?);
    }
    let input = ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, bytes, true).map_err(|_| bad("unsupported"))?;
    apply_with_interrupt(session, target, input, policy, &Interrupt::NONE).map(LinkedSubstitutionResult::into_response)
}

fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session).map_err(|error| error.to_string())
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Replace XCF Linked Image (Reduced)",
        menu: &[],
        shortcut: None,
        params: r#"{"documentId":"canonical nonzero U64","expectedRevision":"canonical U64","layerId":"canonical nonzero U64","resourceId":"canonical nonzero U64","policy":"substitute","payloadHex":"bounded PNG hex"} → one synchronous resident Reduced replacement; no locator reads or journal payload"#,
        enabled,
        run,
        journal: false,
    }]
}

fn recovery_enabled(session: &Session) -> std::result::Result<(), String> {
    context(session).map_err(recovery_error).map_err(|error| error.to_string())
}

/// Registered separately so transport/UI joins can retain their dedicated early guards.
pub fn recover_specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: RECOVER_ID,
        label: "Recover Exact XCF Linked Image",
        menu: &[],
        shortcut: None,
        params: r#"{"documentId":"canonical nonzero U64","expectedRevision":"canonical U64","layerId":"canonical nonzero U64","resourceId":"canonical nonzero U64","policy":"exact","payloadHex":"bounded PNG hex"} → one synchronous atomic original-hash recovery; byteIdentity Exact is not whole-document fidelity; query xcf.assess"#,
        enabled: recovery_enabled,
        run: run_recovery,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_target() -> LinkedSubstitutionTarget {
        LinkedSubstitutionTarget { document: DocId(1), revision: 0, snapshot: WeakDocumentSnapshot::new(), owner: LayerId(1), resource: ResourceId(1) }
    }
    fn input() -> ServiceResourceBytes {
        ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, vec![1], true).unwrap()
    }
    #[test]
    fn inherited_private_transients_are_refused_before_target_work_and_preserved() {
        let mut session = Session::new();
        session.coalesce_request = Some("retained private key".into());
        let before = session.coalesce_request.clone();
        let error = substitute_with_interrupt(&mut session, empty_target(), input(), &Interrupt::NONE).unwrap_err();
        assert_eq!(error.to_string(), "xcf.link.substitute.coalescing");
        assert_eq!(session.coalesce_request, before);
        assert_eq!(dispatch_preflight(&session, &Value::Null, false).unwrap_err().to_string(), "xcf.link.substitute.coalescing");
        session.coalesce_request = None;
        session.color_restrict = Some(1);
        let error = substitute_with_interrupt(&mut session, empty_target(), input(), &Interrupt::NONE).unwrap_err();
        assert_eq!(error.to_string(), "xcf.link.substitute.context");
        assert_eq!(session.color_restrict, Some(1));
        assert_eq!(dispatch_preflight(&session, &Value::Null, false).unwrap_err().to_string(), "xcf.link.substitute.context");
        assert!(session.documents().is_empty());
        assert!(session.journal.is_empty());
    }
    #[test]
    fn encoded_envelope_refuses_large_or_wrong_typed_values_before_decode() {
        let mut params = json!({"documentId":"1","expectedRevision":"0","layerId":"1","resourceId":"1","policy":"substitute","payloadHex":"00"});
        assert!(envelope(&params).is_ok());
        params["payloadHex"] = Value::String("0".repeat(MAX_INPUT * 2 + 1));
        assert_eq!(envelope(&params).unwrap_err().to_string(), "xcf.link.substitute.limit");
        params["payloadHex"] = json!({"untrusted": ["large nested value is not cloned by preflight"]});
        assert_eq!(envelope(&params).unwrap_err().to_string(), "xcf.link.substitute.parameters");
        params["payloadHex"] = json!("00");
        params["documentId"] = json!("01");
        assert_eq!(envelope(&params).unwrap_err().to_string(), "xcf.link.substitute.parameters");
    }
}
