//! Explicit owned original-XCF open. This does not replace ordinary
//! Open, the Color Settings policy, or the profile-question queue.
use crate::{
    PhotocraftApp,
    file_dialog::{FileDialogAnswer, FileDialogRequest},
};
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl, quota::QuotaKind},
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

const WORK_LIMIT: u64 = 8_000_000_000;
fn path_shape(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 4096 || !std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("xcf")) {
        return Err("explicit keepCurrent Open requires a bounded .xcf path".into());
    }
    Ok(())
}
fn parameters(params: &Value) -> Result<Option<&str>, String> {
    let object = params.as_object().ok_or("explicit XCF Open parameters must be an object")?;
    if object.len() > 2 || object.keys().any(|key| !matches!(key.as_str(), "path" | "xcfProfilePolicy")) {
        return Err("explicit XCF Open accepts path and xcfProfilePolicy only".into());
    }
    if object.get("xcfProfilePolicy").and_then(Value::as_str) != Some("keepCurrent") {
        return Err("xcfProfilePolicy must be keepCurrent; omit it to use normal Color Settings".into());
    }
    match object.get("path") {
        None => Ok(None),
        Some(Value::String(path)) => {
            path_shape(path)?;
            Ok(Some(path))
        }
        Some(_) => Err("explicit XCF Open path must be a string".into()),
    }
}
pub(crate) fn invoke(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let path = parameters(params)?;
    // This is the trusted interactive path only. Automation must receive an
    // independent held-root reader; it cannot reuse this ambient capability.
    if app.automation_input {
        return Err("explicit XCF Open is unavailable through automation; use capability-scoped app.open".into());
    }
    if app.services.owned_xcf_open.is_none() {
        return Err("owned XCF Open service is not configured".into());
    }
    if let Some(path) = path {
        return open_path(app, path, Authority::Interactive);
    }
    let initial_dir =
        app.ui.recent_files.first().filter(|p| p.len() <= 4096).and_then(|p| std::path::Path::new(p).parent()).map(|p| p.to_string_lossy().into_owned());
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir, extensions: Some(vec!["xcf".into()]) }, |app, answer| match answer {
        FileDialogAnswer::Paths(paths) if paths.len() == 1 => {
            let path = paths.first().ok_or("owned XCF Open lost its selected path")?;
            open_path(app, path, Authority::Interactive)
        }
        FileDialogAnswer::Failed(error) => Err(error),
        _ => Err("owned XCF Open requires exactly one platform-authorized path; byte-only/browser input is unsupported".into()),
    })
}
#[derive(Clone, Copy)]
enum Authority {
    Interactive,
    HeldRoot,
}
/// Only the caller's independently installed held-root capability may read here.
pub(crate) fn invoke_authorized(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let path = parameters(params)?.ok_or("scoped XCF Open requires an explicit path")?;
    open_path(app, path, Authority::HeldRoot)
}
fn open_path(app: &mut PhotocraftApp, path: &str, authority: Authority) -> Result<Value, String> {
    path_shape(path)?;
    let work = AtomicU64::new(0);
    let charge = |units: u64| {
        work.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(units).filter(|next| *next <= WORK_LIMIT))
            .map_err(|_| BackingError::Work)?;
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    control.check(path.len() as u64).map_err(|e| e.to_string())?;
    if path.contains('\0') {
        return Err("owned XCF Open path contains NUL".into());
    }
    let quota = app.session.accounted_open_quota().cloned();
    let open = match authority {
        Authority::Interactive => app.services.owned_xcf_open.as_mut(),
        Authority::HeldRoot => app.services.automation_owned_xcf_open.as_mut(),
    }
    .ok_or("owned XCF Open service is not configured for this authority")?;
    let imported = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open(path, quota.as_ref(), &control)))
        .map_err(|_| "owned XCF Open failed unexpectedly; existing documents were retained".to_owned())??;
    if quota.as_ref().is_some_and(|q| !q.shares_ledger(imported.document().quota())) {
        return Err("owned XCF Open returned a different catalog quota".into());
    }
    let name = path.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()).ok_or("owned XCF Open has no file name")?;
    let prepared = PreparedReply::new(&imported, path, name, authority, &control)?;
    control.check(1).map_err(|e| e.to_string())?;
    app.session.add_owned_plain_xcf_import(&imported, name, Some(path), &control).map_err(|e| e.to_string())?;
    // Identity/path/profile publication is complete. Existing optional user
    // notifications cannot turn that success into an error. Passing None avoids
    // opened_from() repeating uncharged source/path mutation on the paid owner.
    match authority {
        Authority::Interactive => {
            app.push_recent(path);
            crate::jobs_ui::after_open(app, name, None, &prepared.warnings, None);
        }
        Authority::HeldRoot => {
            // Match automation import: no recent entries, scripts or Color Settings.
            app.sync_views();
            app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]);
            app.ui.status_error = false;
            crate::notices::io_warnings(app, &crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]), &prepared.warnings);
        }
    }
    Ok(prepared.value)
}

// The immutable imported owner retains all original diagnostics while this
// independently charged local response/notification handoff is built. Serde's
// bounded tree construction is a library boundary; its actual capacities are
// reconciled before adoption, never after publication.
struct PreparedReply {
    value: Value,
    warnings: Vec<String>,
    _lease: photocraft_raster::backing::quota::QuotaLease,
}
fn copy_text(text: &str, control: &ReadControl<'_>) -> Result<String, String> {
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
        control.check(part.len() as u64).map_err(|e| e.to_string())?;
        result.push_str(part);
        begin = end;
    }
    Ok(result)
}
fn push_message(warnings: &mut Vec<String>, message: &str, control: &ReadControl<'_>) -> Result<(), String> {
    for previous in warnings.iter() {
        control.check(previous.len().min(message.len()) as u64 + 1).map_err(|e| e.to_string())?;
        if previous == message {
            return Ok(());
        }
    }
    warnings.push(copy_text(message, control)?);
    Ok(())
}
fn tree_bytes(value: &Value, depth: usize, control: &ReadControl<'_>) -> Result<u64, String> {
    if depth > 8 {
        return Err("XCF diagnostic JSON depth exceeded".into());
    }
    control.check(1).map_err(|e| e.to_string())?;
    let mut bytes = std::mem::size_of::<Value>() as u64;
    let mut add = |extra: u64| -> Result<(), String> {
        bytes = bytes.checked_add(extra).ok_or("XCF diagnostic capacity overflow")?;
        Ok(())
    };
    match value {
        Value::String(s) => {
            control.check(s.len() as u64).map_err(|e| e.to_string())?;
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
                control.check(key.len() as u64).map_err(|e| e.to_string())?;
                add(key.capacity() as u64)?;
                add(tree_bytes(item, depth + 1, control)?)?;
            }
        }
        _ => {}
    }
    Ok(bytes)
}
impl PreparedReply {
    fn new(imported: &photocraft_io::OwnedPlainXcfImport, path: &str, name: &str, authority: Authority, control: &ReadControl<'_>) -> Result<Self, String> {
        let assessment = imported.assessment();
        let original = imported.warnings();
        let diagnostics = assessment.map_or(0, |a| a.diagnostics.len());
        let dependencies = assessment.map_or(0, |a| a.dependencies.len());
        if original.len() > 1000 || diagnostics > 1000 || dependencies > 1000 {
            return Err("XCF diagnostic row limit exceeded".into());
        }
        let mut text = path.len().checked_add(name.len()).ok_or("XCF diagnostic length overflow")? as u64;
        let mut visit = |s: &str| -> Result<(), String> {
            if s.len() > 1 << 20 {
                return Err("XCF diagnostic text limit exceeded".into());
            }
            control.check(s.len() as u64 + 1).map_err(|e| e.to_string())?;
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
            .map_err(|e| e.to_string())?;
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
        let value = match authority {
            Authority::Interactive => json!({"warnings":&warnings,"assessment":assessment,"sourceReadOnly":read_only}),
            Authority::HeldRoot => json!({"path":path,"name":name,"warnings":&warnings,"assessment":assessment,"sourceReadOnly":read_only}),
        };
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
#[path = "owned_xcf_open_tests.rs"]
mod tests;
