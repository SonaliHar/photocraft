//! Ephemeral service-authorized resource bytes. This is not a resource evaluator.
//! Callers must acquire explicit picker/root/upload authority and issue unguessable handles.
//! No type here is serializable; retained locators and JSON cannot mint a grant.
use crate::document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot};
use photocraft_doc::{item_identity::ResourceId, xcf_resources::ResourceKind};
use photocraft_raster::Interrupt;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResourceHandle([u8; 32]);
impl ResourceHandle {
    /// A trusted service supplies cryptographically random bytes, never a source-stored token.
    pub fn from_service_nonce(nonce: [u8; 32]) -> Result<Self, RecoveryError> {
        if nonce.iter().all(|b| *b == 0) { Err(RecoveryError::Denied) } else { Ok(Self(nonce)) }
    }
}
impl std::fmt::Debug for ResourceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResourceHandle(<redacted>)")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    Denied,
    Cancelled,
    Limit,
    Allocation,
    UnknownHandle,
    DuplicateHandle,
    StaleDocument,
    UnknownResource,
    KindMismatch,
    MissingHash,
    HashMismatch,
    InvalidState,
    UnsupportedFont,
}
impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XCF resource grant: {self:?}")
    }
}
impl std::error::Error for RecoveryError {}
fn poll(ctl: &Interrupt<'_>) -> Result<(), RecoveryError> {
    ctl.check().map_err(|_| RecoveryError::Cancelled)
}
fn hash(bytes: &[u8], ctl: &Interrupt<'_>) -> Result<String, RecoveryError> {
    let mut hash = blake3::Hasher::new();
    for part in bytes.chunks(32 << 10) {
        poll(ctl)?;
        hash.update(part);
    }
    poll(ctl)?;
    Ok(hash.finalize().to_hex().to_string())
}
/// Bytes supplied by an already-authorized service, not by deserialized XCF metadata.
pub struct ServiceResourceBytes {
    kind: ResourceKind,
    bytes: Vec<u8>,
}
impl ServiceResourceBytes {
    pub(crate) fn kind(&self) -> &ResourceKind {
        &self.kind
    }
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn from_authorized_service(kind: ResourceKind, bytes: Vec<u8>, granted: bool) -> Result<Self, RecoveryError> {
        if !granted {
            return Err(RecoveryError::Denied);
        }
        if kind == ResourceKind::Font {
            return Err(RecoveryError::UnsupportedFont);
        }
        if bytes.len() > 64 << 20 {
            return Err(RecoveryError::Limit);
        }
        Ok(Self { kind, bytes })
    }
}
struct Entry {
    handle: ResourceHandle,
    document: WeakDocumentSnapshot,
    resource: ResourceId,
    kind: ResourceKind,
    expected_hash: String,
    bytes: Vec<u8>,
}
/// A hash-bound payload still requires a typed decoder and qualified consumer before activation.
pub struct HashBoundResourceBytes {
    resource: ResourceId,
    kind: ResourceKind,
    expected_hash: String,
    bytes: Vec<u8>,
}
impl HashBoundResourceBytes {
    pub fn resource(&self) -> ResourceId {
        self.resource
    }
    pub fn kind(&self) -> &ResourceKind {
        &self.kind
    }
    pub fn expected_hash(&self) -> &str {
        &self.expected_hash
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
impl std::fmt::Debug for HashBoundResourceBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashBoundResourceBytes").field("resource", &self.resource).field("kind", &self.kind).field("byte_length", &self.bytes.len()).finish()
    }
}
/// Bounded handles expire on close, native reopen or any document COW edit.
pub struct ResourceCapabilities {
    entries: Vec<Entry>,
    issued: Vec<ResourceHandle>,
    component: usize,
    total: usize,
    slots: usize,
}
impl Default for ResourceCapabilities {
    fn default() -> Self {
        Self { entries: Vec::new(), issued: Vec::new(), component: 8 << 20, total: 32 << 20, slots: 32 }
    }
}
impl ResourceCapabilities {
    /// Caller-selected lower bounds; hard ceilings are 64MiB/component,128MiB total,64 grants.
    pub fn with_limits(component: usize, total: usize, slots: usize) -> Result<Self, RecoveryError> {
        if component > 64 << 20 || total > 128 << 20 || slots > 64 {
            return Err(RecoveryError::Limit);
        }
        Ok(Self { entries: Vec::new(), issued: Vec::new(), component, total, slots })
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn clear(&mut self) {
        self.entries.clear()
    }
    pub fn issue(
        &mut self,
        handle: ResourceHandle,
        document: &DocumentSnapshot,
        resource: ResourceId,
        input: ServiceResourceBytes,
        ctl: &Interrupt<'_>,
    ) -> Result<(), RecoveryError> {
        poll(ctl)?;
        if self.issued.contains(&handle) {
            return Err(RecoveryError::DuplicateHandle);
        }
        if self.issued.len() >= 4096 {
            return Err(RecoveryError::Limit);
        }
        let used = self.entries.iter().try_fold(0usize, |sum, entry| sum.checked_add(entry.bytes.len()).ok_or(RecoveryError::Limit))?;
        if self.entries.len() >= self.slots || input.bytes.len() > self.component || used.checked_add(input.bytes.len()).is_none_or(|n| n > self.total) {
            return Err(RecoveryError::Limit);
        }
        document.validate_xcf_state_with(ctl).map_err(|_| if ctl.cancelled() { RecoveryError::Cancelled } else { RecoveryError::InvalidState })?;
        let target =
            document.xcf_state.as_ref().and_then(|state| state.resources.iter().find(|entry| entry.id == resource)).ok_or(RecoveryError::UnknownResource)?;
        if target.kind != input.kind {
            return Err(RecoveryError::KindMismatch);
        }
        let expected = target.content_hash.as_ref().ok_or(RecoveryError::MissingHash)?;
        if !hash(&input.bytes, ctl)?.eq_ignore_ascii_case(expected) {
            return Err(RecoveryError::HashMismatch);
        }
        self.entries.try_reserve(1).map_err(|_| RecoveryError::Allocation)?;
        self.issued.try_reserve(1).map_err(|_| RecoveryError::Allocation)?;
        poll(ctl)?;
        self.issued.push(handle);
        self.entries.push(Entry { handle, document: document.downgrade(), resource, kind: input.kind, expected_hash: expected.clone(), bytes: input.bytes });
        Ok(())
    }
    /// Consume once only after identity/hash/cancellation checks; denial leaves all state unchanged.
    pub fn consume(&mut self, handle: ResourceHandle, document: &DocumentSnapshot, ctl: &Interrupt<'_>) -> Result<HashBoundResourceBytes, RecoveryError> {
        poll(ctl)?;
        let index = self.entries.iter().position(|entry| entry.handle == handle).ok_or(RecoveryError::UnknownHandle)?;
        let entry = self.entries.get(index).ok_or(RecoveryError::UnknownHandle)?;
        if !entry.document.is_current(document) {
            return Err(RecoveryError::StaleDocument);
        }
        document.validate_xcf_state_with(ctl).map_err(|_| if ctl.cancelled() { RecoveryError::Cancelled } else { RecoveryError::InvalidState })?;
        let target =
            document.xcf_state.as_ref().and_then(|state| state.resources.iter().find(|r| r.id == entry.resource)).ok_or(RecoveryError::UnknownResource)?;
        if target.kind != entry.kind {
            return Err(RecoveryError::KindMismatch);
        }
        if target.content_hash.as_deref() != Some(entry.expected_hash.as_str()) || !hash(&entry.bytes, ctl)?.eq_ignore_ascii_case(&entry.expected_hash) {
            return Err(RecoveryError::HashMismatch);
        }
        poll(ctl)?;
        let entry = self.entries.remove(index);
        Ok(HashBoundResourceBytes { resource: entry.resource, kind: entry.kind, expected_hash: entry.expected_hash, bytes: entry.bytes })
    }
}

/// An explicit service policy; an absent original digest cannot establish exact identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryPolicy {
    Exact,
    AllowSubstitution,
}

/// Shared nonmutating identity and consumer-lock qualification.
pub(crate) fn prepare_recovery_identity(
    document: &crate::document_snapshot::DocumentSnapshot,
    resource: ResourceId,
    input: &ServiceResourceBytes,
    policy: RecoveryPolicy,
    digest: String,
    ctl: &Interrupt<'_>,
) -> crate::Result<(String, photocraft_doc::xcf_resources::ResourceFidelity)> {
    use photocraft_doc::xcf_resources::ResourceFidelity;
    let bad = |e: RecoveryError| crate::EngineError::Other(e.to_string());
    poll(ctl).map_err(bad)?;
    document
        .validate_resident()
        .map_err(|_| crate::EngineError::Other("XCF recovery requires a resident document; accounted linked edit adoption is not available".into()))?;
    document.validate_xcf_state_with(ctl).map_err(|e| crate::EngineError::Other(e.to_string()))?;
    let target = document
        .xcf_state
        .as_ref()
        .and_then(|state| state.resources.iter().find(|entry| entry.id == resource))
        .ok_or_else(|| bad(RecoveryError::UnknownResource))?;
    if target.kind != input.kind {
        return Err(bad(RecoveryError::KindMismatch));
    }
    if let Some(state) = &document.xcf_state {
        for source in &state.source_items {
            let uses_resource = match source.source {
                photocraft_doc::xcf_items::EditableSource::Linked { resource: linked, .. } => linked == resource,
                photocraft_doc::xcf_items::EditableSource::Text { font, .. } => font == resource,
                _ => false,
            };
            if uses_resource {
                if !matches!(source.owner, photocraft_doc::item_identity::ItemRef::Layer(_)) {
                    return Err(bad(RecoveryError::InvalidState));
                }
                let effective = crate::xcf_item_locks::effective_item_locks(document, source.owner)?;
                let locks = effective.imported;
                let ordinary = effective.native;
                if locks.content || ordinary.all || ordinary.pixels {
                    return Err(crate::EngineError::Other("resource consumer content is locked".into()));
                }
            }
        }
    }
    let fidelity = if target.content_hash.as_deref().is_some_and(|expected| expected.eq_ignore_ascii_case(&digest)) {
        ResourceFidelity::Exact
    } else if policy == RecoveryPolicy::AllowSubstitution {
        ResourceFidelity::Substituted
    } else {
        return Err(bad(if target.content_hash.is_none() { RecoveryError::MissingHash } else { RecoveryError::HashMismatch }));
    };
    Ok((digest, fidelity))
}

pub(crate) fn check_recovery_total(document: &photocraft_doc::Document, resource: ResourceId, input_len: usize) -> crate::Result<()> {
    let bad = |e: RecoveryError| crate::EngineError::Other(e.to_string());
    let current_total = document
        .xcf_state
        .as_ref()
        .ok_or_else(|| bad(RecoveryError::InvalidState))?
        .resources
        .iter()
        .filter(|entry| entry.id != resource)
        .try_fold(0usize, |total, entry| {
            total.checked_add(entry.owned_bytes.as_ref().map_or(0, |bytes| bytes.len())).ok_or_else(|| bad(RecoveryError::Limit))
        })?;
    if current_total.checked_add(input_len).is_none_or(|total| total > 128 << 20) {
        return Err(bad(RecoveryError::Limit));
    }
    Ok(())
}

pub(crate) fn hash_recovery_input(input: &ServiceResourceBytes, ctl: &Interrupt<'_>) -> crate::Result<String> {
    hash(&input.bytes, ctl).map_err(|error| crate::EngineError::Other(error.to_string()))
}

/// Retain explicitly authorized bytes in one undoable transaction. This resolves byte
/// identity only: typed font/pattern/image consumers must independently qualify activation.
/// No locator is consulted, and neither the service grant nor a handle is serialized.
pub fn recover_authorized_bytes(
    session: &mut crate::Session,
    resource: ResourceId,
    input: ServiceResourceBytes,
    policy: RecoveryPolicy,
    ctl: &Interrupt<'_>,
) -> crate::Result<photocraft_doc::xcf_resources::ResourceFidelity> {
    use photocraft_doc::xcf_resources::ResourceResolution;
    let bad = |e: RecoveryError| crate::EngineError::Other(e.to_string());
    poll(ctl).map_err(bad)?;
    let digest = hash_recovery_input(&input, ctl)?;
    let document = &session.active().ok_or(crate::EngineError::NoDocument)?.doc;
    let (digest, fidelity) = prepare_recovery_identity(document, resource, &input, policy, digest, ctl)?;
    let target = document
        .xcf_state
        .as_ref()
        .and_then(|state| state.resources.iter().find(|entry| entry.id == resource))
        .ok_or_else(|| bad(RecoveryError::UnknownResource))?;
    if target.owned_bytes.as_ref().map(|bytes| bytes.as_slice()) == Some(input.bytes.as_slice())
        && target.resolution.as_ref().is_some_and(|resolution| resolution.fidelity == fidelity && resolution.payload_hash == digest)
    {
        poll(ctl).map_err(bad)?;
        return Ok(fidelity);
    }
    check_recovery_total(document, resource, input.bytes.len())?;
    session.edit_with_interrupt("Recover XCF Resource Bytes", ctl, move |document, _| {
        let target = document
            .xcf_state
            .as_mut()
            .and_then(|state| state.resources.iter_mut().find(|entry| entry.id == resource))
            .ok_or_else(|| bad(RecoveryError::UnknownResource))?;
        target.owned_bytes = Some(input.bytes.into());
        target.resolution = Some(ResourceResolution { fidelity, payload_hash: digest });
        document.validate_xcf_state_with(ctl).map_err(|e| crate::EngineError::Other(e.to_string()))?;
        Ok(fidelity)
    })
}
