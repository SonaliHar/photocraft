//! Explicit owned-XCF current interpretation. Ordinary Open remains unchanged.
use super::{Filesystem, Headless};
use crate::{AutomationError, files};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{QuotaKind, QuotaLease},
};
use serde_json::{Value, json};
use std::path::Path;

/// Bounded summary and its actual catalog charge. Borrow the value while this
/// carrier remains alive; it conveys no persistent read authority.
pub struct OwnedXcfOpenReply {
    value: Value,
    warnings: Vec<String>,
    _lease: QuotaLease,
}
impl OwnedXcfOpenReply {
    pub fn value(&self) -> &Value {
        &self.value
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}
impl Headless {
    /// CLI convenience with the same finite lower work policy as explicit UI
    /// Open. Controlled callers should supply their own cumulative ReadControl.
    pub fn open_owned_xcf_keep_current_bounded(&mut self, path: &Path) -> Result<OwnedXcfOpenReply, AutomationError> {
        self.open_owned_xcf_bounded(path, None)
    }
    pub(crate) fn open_owned_xcf_rpc_bounded(&mut self, path: &Path, id: &Value) -> Result<OwnedXcfOpenReply, AutomationError> {
        self.open_owned_xcf_bounded(path, Some(id))
    }
    fn open_owned_xcf_bounded(&mut self, path: &Path, id: Option<&Value>) -> Result<OwnedXcfOpenReply, AutomationError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        let used = AtomicU64::new(0);
        let charge = |units: u64| {
            used.try_update(Ordering::Relaxed, Ordering::Relaxed, |old| old.checked_add(units).filter(|next| *next <= 8_000_000_000))
                .map_err(|_| BackingError::Work)?;
            Ok(())
        };
        self.open_owned_xcf_impl(path, id, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &charge })
    }
    /// Explicitly keep the current interpretation of one eligible original XCF.
    /// The installed authority is selected once; a failed reader never retries.
    pub fn open_owned_xcf_keep_current(&mut self, path: &Path, control: &ReadControl<'_>) -> Result<OwnedXcfOpenReply, AutomationError> {
        self.open_owned_xcf_impl(path, None, control)
    }
    fn open_owned_xcf_impl(&mut self, path: &Path, id: Option<&Value>, control: &ReadControl<'_>) -> Result<OwnedXcfOpenReply, AutomationError> {
        // Bound hostile request IDs before reading any source. This is an
        // allocation-free recipe for the later owned copy, not an input fee.
        let id_bytes = id.map(|value| id_copy_bound(value, control)).transpose().map_err(ReplyError::into_automation)?;
        let path = path.to_str().ok_or_else(|| AutomationError::BadRequest("owned XCF path must be UTF-8".into()))?;
        if path.is_empty() || path.len() > 4096 || !Path::new(path).extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("xcf")) {
            return Err(AutomationError::BadRequest("explicit keepCurrent requires a bounded .xcf path".into()));
        }
        control.check(path.len() as u64).map_err(photocraft_engine::EngineError::from)?;
        if path.contains('\0') {
            return Err(AutomationError::BadRequest("owned XCF path contains NUL".into()));
        }
        if matches!(self.filesystem, Filesystem::Denied) {
            return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into()));
        }
        if self.session.active_job().is_some() {
            return Err(AutomationError::BadRequest("owned XCF Open requires an idle session".into()));
        }
        let quota = match self.session.accounted_open_quota() {
            Some(quota) => quota.clone(),
            None => photocraft_io::accounted_native::new_accounted_native_open_quota(control).map_err(|e| match e {
                photocraft_io::accounted_native::AccountedNativeError::Backing(error) => AutomationError::Engine(error.into()),
                other => AutomationError::Other(other.to_string()),
            })?,
        };
        let imported = match &self.filesystem {
            Filesystem::TrustedLocal => files::open_owned_current_xcf_selected_path(path, quota.clone(), control)?,
            Filesystem::Workspace(workspace) => workspace.open_owned_current_xcf_with_read_control(path, quota.clone(), control)?,
            Filesystem::Denied => return Err(AutomationError::BadRequest("read authority is absent".into())),
        };
        if !quota.shares_ledger(imported.document().quota()) {
            return Err(AutomationError::Other("owned XCF reader changed catalog quota".into()));
        }
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| AutomationError::BadRequest("owned XCF path has no file name".into()))?;
        let mut reply = OwnedXcfOpenReply::new(&imported, path, name, self.session.documents().len(), control).map_err(ReplyError::into_automation)?;
        if let (Some(id), Some(bytes)) = (id, id_bytes) {
            reply.wrap_rpc(id, bytes, control).map_err(ReplyError::into_automation)?;
            reply.validate_rpc_wire_size(control).map_err(ReplyError::into_automation)?;
        }
        control.check(1).map_err(photocraft_engine::EngineError::from)?;
        match &imported {
            photocraft_io::OwnedXcfImport::Plain(value) => self.session.add_owned_plain_xcf_import(value, name, Some(path), control)?,
            photocraft_io::OwnedXcfImport::ThresholdAlpha(value) => self.session.add_owned_threshold_alpha_xcf_import(value, name, Some(path), control)?,
            photocraft_io::OwnedXcfImport::HistogramThreshold(value) => {
                self.session.add_owned_histogram_threshold_xcf_import(value, name, Some(path), control)?
            }
        };
        // No fallible operation or token poll after typed Session publication.
        Ok(reply)
    }
}

pub(super) fn id_copy_bound(value: &Value, control: &ReadControl<'_>) -> Result<u64, ReplyError> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize, text: &mut usize, control: &ReadControl<'_>) -> Result<u64, ReplyError> {
        control.check(1).map_err(ReplyError::Backing)?;
        *nodes = nodes.checked_add(1).ok_or("RPC ID node overflow")?;
        if depth > 8 || *nodes > 4096 {
            return Err("owned RPC ID exceeds depth/node limit".into());
        }
        let mut bytes = std::mem::size_of::<Value>() as u64;
        let mut string = |s: &str| -> Result<u64, ReplyError> {
            *text = text.checked_add(s.len()).ok_or("RPC ID text overflow")?;
            if *text > crate::security::MAX_REQUEST_BYTES {
                return Err("owned RPC ID text limit exceeded".into());
            }
            control.check(s.len() as u64).map_err(ReplyError::Backing)?;
            (s.len() as u64).checked_mul(2).ok_or_else(|| "RPC ID capacity overflow".into())
        };
        match value {
            Value::String(s) => bytes = bytes.checked_add(string(s)?).ok_or("RPC ID overflow")?,
            Value::Array(a) => {
                bytes = bytes
                    .checked_add((a.len() as u64).checked_mul(2 * std::mem::size_of::<Value>() as u64).ok_or("RPC ID overflow")?)
                    .ok_or("RPC ID overflow")?;
                for v in a {
                    bytes = bytes.checked_add(visit(v, depth + 1, nodes, text, control)?).ok_or("RPC ID overflow")?;
                }
            }
            Value::Object(o) => {
                bytes = bytes.checked_add((o.len() as u64).checked_mul(256).ok_or("RPC ID overflow")?).ok_or("RPC ID overflow")?;
                for (key, v) in o {
                    // End the short closure borrow before recursing.
                    control.check(key.len() as u64).map_err(ReplyError::Backing)?;
                    *text = text.checked_add(key.len()).ok_or("RPC ID text overflow")?;
                    if *text > crate::security::MAX_REQUEST_BYTES {
                        return Err("owned RPC ID text limit exceeded".into());
                    }
                    bytes = bytes
                        .checked_add((key.len() as u64).checked_mul(2).ok_or("RPC ID overflow")?)
                        .and_then(|b| b.checked_add(256))
                        .ok_or("RPC ID overflow")?;
                    bytes = bytes.checked_add(visit(v, depth + 1, nodes, text, control)?).ok_or("RPC ID overflow")?;
                }
            }
            _ => {}
        }
        Ok(bytes)
    }
    visit(value, 0, &mut 0, &mut 0, control)
}
impl OwnedXcfOpenReply {
    pub(super) fn validate_rpc_wire_size(&self, control: &ReadControl<'_>) -> Result<(), ReplyError> {
        // Allocation-free serialization census prevents an oversized successful
        // envelope being replaced by a transport-limit error after adoption.
        control.check(self._lease.bytes().checked_mul(8).ok_or("RPC wire work overflow")?).map_err(ReplyError::Backing)?;
        struct Count<'a, 'b> {
            used: usize,
            control: &'a ReadControl<'b>,
            failure: Option<BackingError>,
        }
        impl std::io::Write for Count<'_, '_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if let Err(error) = self.control.check(bytes.len() as u64 + 1) {
                    self.failure = Some(error);
                    return Err(std::io::Error::other("RPC wire census interrupted"));
                }
                let Some(next) = self.used.checked_add(bytes.len()).filter(|n| *n < crate::budgets::MAX_RESPONSE_BYTES) else {
                    return Err(std::io::Error::other("RPC reply exceeds transport limit"));
                };
                self.used = next;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut count = Count { used: 0, control, failure: None };
        let result = serde_json::to_writer(&mut count, &self.value);
        if let Some(error) = count.failure {
            return Err(ReplyError::Backing(error));
        }
        result.map_err(|_| ReplyError::from("owned RPC reply exceeds transport limit"))
    }
    pub(super) fn wrap_rpc(&mut self, id: &Value, id_bound: u64, control: &ReadControl<'_>) -> Result<(), ReplyError> {
        let before = self._lease.bytes();
        let extra = id_bound.checked_mul(4).and_then(|v| v.checked_add(4096)).ok_or("RPC envelope overflow")?;
        self._lease.try_resize(before.checked_add(extra).ok_or("RPC envelope overflow")?).map_err(|e| e.to_string())?;
        control.check(extra).map_err(ReplyError::Backing)?;
        let id = id.clone(); // Bounded library copy after genuine admission.
        let actual = tree_bytes(&id, 0, control)?.checked_add(4096).ok_or("RPC envelope overflow")?;
        if actual > extra {
            return Err("RPC ID capacity exceeded admitted envelope".into());
        }
        let mut envelope = serde_json::Map::new();
        envelope.insert("id".into(), id);
        envelope.insert("ok".into(), Value::Bool(true));
        envelope.insert("result".into(), std::mem::take(&mut self.value));
        self.value = Value::Object(envelope);
        self._lease.reconcile_capacity(before.checked_add(actual).ok_or("RPC envelope overflow")?).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub(super) enum ReplyError {
    Backing(BackingError),
    Message(String),
}
impl From<String> for ReplyError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}
impl From<&str> for ReplyError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_owned())
    }
}
impl ReplyError {
    pub(super) fn into_automation(self) -> AutomationError {
        match self {
            Self::Backing(e) => AutomationError::Engine(e.into()),
            Self::Message(e) => AutomationError::Other(e),
        }
    }
}

fn copy_text(text: &str, control: &ReadControl<'_>) -> Result<String, ReplyError> {
    let mut result = String::new();
    result.try_reserve_exact(text.len()).map_err(|_| BackingError::Allocation.to_string())?;
    if result.capacity() > text.len().saturating_mul(2) {
        return Err("XCF diagnostic allocation exceeds admitted capacity".into());
    }
    let mut begin = 0;
    while begin < text.len() {
        let mut end = begin.saturating_add(4096).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let part = text.get(begin..end).ok_or("XCF diagnostic UTF-8 span")?;
        control.check(part.len() as u64).map_err(ReplyError::Backing)?;
        result.push_str(part);
        begin = end;
    }
    Ok(result)
}
fn push_message(warnings: &mut Vec<String>, message: &str, control: &ReadControl<'_>) -> Result<(), ReplyError> {
    for previous in warnings.iter() {
        control.check(previous.len().min(message.len()) as u64 + 1).map_err(ReplyError::Backing)?;
        if previous == message {
            return Ok(());
        }
    }
    warnings.push(copy_text(message, control)?);
    Ok(())
}
fn tree_bytes(value: &Value, depth: usize, control: &ReadControl<'_>) -> Result<u64, ReplyError> {
    if depth > 8 {
        return Err("XCF diagnostic JSON depth exceeded".into());
    }
    control.check(1).map_err(ReplyError::Backing)?;
    let mut bytes = std::mem::size_of::<Value>() as u64;
    let mut add = |extra: u64| -> Result<(), ReplyError> {
        bytes = bytes.checked_add(extra).ok_or("XCF diagnostic capacity overflow")?;
        Ok(())
    };
    match value {
        Value::String(s) => {
            control.check(s.len() as u64).map_err(ReplyError::Backing)?;
            add(s.capacity() as u64)?;
        }
        Value::Array(values) => {
            add((values.capacity() as u64).checked_mul(std::mem::size_of::<Value>() as u64).ok_or("XCF diagnostic capacity overflow")?)?;
            for item in values {
                add(tree_bytes(item, depth + 1, control)?)?;
            }
        }
        Value::Object(values) => {
            // Conservative BTree/optional map node envelope per actual entry.
            add((values.len() as u64).checked_mul(256).ok_or("XCF diagnostic capacity overflow")?)?;
            for (key, item) in values {
                control.check(key.len() as u64).map_err(ReplyError::Backing)?;
                add(key.capacity() as u64)?;
                add(tree_bytes(item, depth + 1, control)?)?;
            }
        }
        _ => {}
    }
    Ok(bytes)
}
impl OwnedXcfOpenReply {
    fn new(imported: &photocraft_io::OwnedXcfImport, path: &str, name: &str, index: usize, control: &ReadControl<'_>) -> Result<Self, ReplyError> {
        let assessment = imported.assessment();
        let original = imported.warnings();
        let diagnostics = assessment.map_or(0, |a| a.diagnostics.len());
        let dependencies = assessment.map_or(0, |a| a.dependencies.len());
        if original.len() > 1000 || diagnostics > 1000 || dependencies > 1000 {
            return Err("XCF diagnostic row limit exceeded".into());
        }
        let mut text = path.len().checked_add(name.len()).ok_or("XCF diagnostic length overflow")? as u64;
        let mut visit = |s: &str| -> Result<(), ReplyError> {
            if s.len() > 1 << 20 {
                return Err("XCF diagnostic text limit exceeded".into());
            }
            control.check(s.len() as u64 + 1).map_err(ReplyError::Backing)?;
            text = text.checked_add(s.len() as u64).ok_or("XCF diagnostic length overflow")?;
            if text > 1 << 20 {
                return Err("XCF diagnostic aggregate text limit exceeded".into());
            }
            Ok(())
        };
        for message in original {
            visit(message)?;
        }
        if let Some(a) = assessment {
            visit(a.source_format)?;
            visit(&a.container)?;
            for d in &a.diagnostics {
                if d.recovery_actions.len() > 16 {
                    return Err("XCF diagnostic recovery-action limit exceeded".into());
                }
                visit(d.code)?;
                visit(d.severity)?;
                visit(&d.resource_id)?;
                visit(d.message)?;
                visit(d.affected_capability)?;
                for action in &d.recovery_actions {
                    visit(action)?;
                }
            }
            for d in &a.dependencies {
                visit(&d.resource_id)?;
                visit(d.kind)?;
            }
        }
        let rows = original
            .len()
            .checked_add(diagnostics)
            .and_then(|n| n.checked_add(dependencies))
            .and_then(|n| n.checked_add(2))
            .ok_or("XCF diagnostic row overflow")?;
        let envelope = text
            .checked_mul(32)
            .and_then(|n| n.checked_add((rows as u64).checked_mul(8192)?))
            .and_then(|n| n.checked_add(16384))
            .ok_or("XCF diagnostic envelope overflow")?;
        let mut lease = imported.document().quota().reserve(QuotaKind::Index, envelope).map_err(|e| e.to_string())?;
        // Prepay serde field/string construction before its bounded library call;
        // exact warning comparisons/copies and output census are charged below.
        control
            .check(text.checked_mul(16).and_then(|n| n.checked_add((rows as u64).checked_mul(256)?)).ok_or("XCF diagnostic work overflow")?)
            .map_err(ReplyError::Backing)?;
        let max_warnings = original.len().checked_add(diagnostics).and_then(|n| n.checked_add(1)).ok_or("XCF warning count overflow")?;
        let mut warnings = Vec::new();
        warnings.try_reserve_exact(max_warnings).map_err(|_| BackingError::Allocation.to_string())?;
        if warnings.capacity() > max_warnings.saturating_mul(2) {
            return Err("XCF warning capacity exceeded".into());
        }
        for warning in original {
            warnings.push(copy_text(warning, control)?);
        }
        if let Some(a) = assessment {
            use photocraft_io::xcf_assessment::Fidelity;
            match a.fidelity {
                Fidelity::Full => {}
                Fidelity::Reduced => push_message(&mut warnings, "This XCF import has reduced fidelity.", control)?,
                Fidelity::DependencyLimited => {
                    push_message(&mut warnings, "This XCF import has fidelity limited by unavailable or inactive dependencies.", control)?
                }
            }
            for diagnostic in &a.diagnostics {
                push_message(&mut warnings, diagnostic.message, control)?;
            }
        }
        let assessment = serde_json::to_value(assessment).map_err(|_| "XCF assessment serialization failed")?;
        let read_only = imported.source_read_only() || imported.origin().is_read_only();
        let doc = imported.document().document();
        let value = json!({"path":path,"name":name,"width":doc.size.width,"height":doc.size.height,
            "layers":doc.layer_count(),"mode":doc.mode,"depth":doc.depth,"index":index,
            "warnings":&warnings,"assessment":assessment,"sourceReadOnly":read_only,
            "inspectionScope":"owned-import-summary","xcfProfilePolicy":"keepCurrent"});
        let mut actual = tree_bytes(&value, 0, control)?
            .checked_add((warnings.capacity() as u64).checked_mul(std::mem::size_of::<String>() as u64).ok_or("XCF warning capacity overflow")?)
            .ok_or("XCF diagnostic capacity overflow")?;
        for warning in &warnings {
            actual = actual.checked_add(warning.capacity() as u64).ok_or("XCF diagnostic capacity overflow")?;
        }
        actual = actual.checked_add(std::mem::size_of::<Self>() as u64 + 256).ok_or("XCF diagnostic capacity overflow")?;
        if actual > envelope {
            return Err("XCF diagnostic actual capacity exceeds admitted envelope".into());
        }
        lease.reconcile_capacity(actual).map_err(|e| e.to_string())?;
        Ok(Self { value, warnings, _lease: lease })
    }
}

#[cfg(test)]
mod tests;

impl OwnedXcfOpenReply {
    pub(super) fn new_native(
        archive: &photocraft_format::owned_document::OwnedDocumentArchive,
        path: &str,
        index: usize,
        control: &ReadControl<'_>,
    ) -> Result<Self, ReplyError> {
        let doc = archive.document();
        if path.len() > 4096 || doc.name.len() > 1 << 20 {
            return Err("native summary text limit exceeded".into());
        }
        let text = (path.len() as u64).checked_add(doc.name.len() as u64).ok_or("native summary text overflow")?;
        let envelope = text.checked_mul(8).and_then(|v| v.checked_add(16384)).ok_or("native summary envelope overflow")?;
        let mut lease = archive.quota().reserve(QuotaKind::Index, envelope).map_err(|e| e.to_string())?;
        // The authenticated archive already holds its complete bounded model.
        // Count layers with an explicitly bounded, polled borrowed recursion.
        fn layers(values: &[photocraft_doc::Layer], depth: usize, count: &mut usize, control: &ReadControl<'_>) -> Result<(), ReplyError> {
            if depth > photocraft_doc::MAX_GROUP_DEPTH {
                return Err("native summary depth limit".into());
            }
            for layer in values {
                control.check(1).map_err(ReplyError::Backing)?;
                *count = count.checked_add(1).filter(|v| *v <= 4096).ok_or("native summary layer limit")?;
                if let photocraft_doc::LayerContent::Group(group) = &layer.content {
                    layers(&group.children, depth + 1, count, control)?;
                }
            }
            Ok(())
        }
        let mut count = 0;
        layers(&doc.layers, 0, &mut count, control)?;
        control.check(text.checked_mul(4).and_then(|v| v.checked_add(4096)).ok_or("native summary work overflow")?).map_err(ReplyError::Backing)?;
        let value = json!({"path":path,"name":doc.name,"width":doc.size.width,"height":doc.size.height,
            "layers":count,"mode":doc.mode,"depth":doc.depth,"index":index,"warnings":[],
            "sourceReadOnly":false,"inspectionScope":"owned-native-summary","nativeProfilePolicy":"keepCurrent"});
        let actual = tree_bytes(&value, 0, control)?.checked_add(std::mem::size_of::<Self>() as u64 + 256).ok_or("native summary actual overflow")?;
        if actual > envelope {
            return Err("native summary actual exceeds admission".into());
        }
        lease.reconcile_capacity(actual).map_err(|e| e.to_string())?;
        Ok(Self { value, warnings: Vec::new(), _lease: lease })
    }
}
