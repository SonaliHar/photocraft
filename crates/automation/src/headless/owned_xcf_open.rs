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
        use std::sync::atomic::{AtomicU64, Ordering};
        let used = AtomicU64::new(0);
        let charge = |units: u64| {
            used.try_update(Ordering::Relaxed, Ordering::Relaxed, |old| old.checked_add(units).filter(|next| *next <= 8_000_000_000))
                .map_err(|_| BackingError::Work)?;
            Ok(())
        };
        self.open_owned_xcf_keep_current(path, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &charge })
    }
    /// Explicitly keep the current interpretation of one eligible original XCF.
    /// The installed authority is selected once; a failed reader never retries.
    pub fn open_owned_xcf_keep_current(&mut self, path: &Path, control: &ReadControl<'_>) -> Result<OwnedXcfOpenReply, AutomationError> {
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
        let reply = OwnedXcfOpenReply::new(&imported, path, name, self.session.documents().len(), control).map_err(ReplyError::into_automation)?;
        control.check(1).map_err(photocraft_engine::EngineError::from)?;
        match &imported {
            photocraft_io::OwnedXcfImport::Plain(value) => self.session.add_owned_plain_xcf_import(value, name, Some(path), control)?,
            photocraft_io::OwnedXcfImport::ThresholdAlpha(value) => self.session.add_owned_threshold_alpha_xcf_import(value, name, Some(path), control)?,
        };
        // No fallible operation or token poll after typed Session publication.
        Ok(reply)
    }
}

enum ReplyError {
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
    fn into_automation(self) -> AutomationError {
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
