//! Explicit current-interpretation native open. This does not replace ordinary
//! Open, the Color Settings policy, or the profile-question queue.
use crate::{
    PhotocraftApp,
    file_dialog::{FileDialogAnswer, FileDialogRequest},
};
use photocraft_engine::accounted_native_open::AccountedNativeOpenPolicy;
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl, quota::QuotaKind},
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

const WORK_LIMIT: u64 = 8_000_000_000;
fn path_shape(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 4096
        || !std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pcraft"))
    {
        return Err("explicit keepCurrent Open requires a bounded .pcraft path".into());
    }
    Ok(())
}
fn parameters(params: &Value) -> Result<Option<&str>, String> {
    let object = params.as_object().ok_or("explicit native Open parameters must be an object")?;
    if object.len() > 2 || object.keys().any(|key| !matches!(key.as_str(), "path" | "nativeProfilePolicy")) {
        return Err("explicit native Open accepts path and nativeProfilePolicy only".into());
    }
    if object.get("nativeProfilePolicy").and_then(Value::as_str) != Some("keepCurrent") {
        return Err("nativeProfilePolicy must be keepCurrent; omit it to use normal Color Settings".into());
    }
    match object.get("path") {
        None => Ok(None),
        Some(Value::String(path)) => {
            path_shape(path)?;
            Ok(Some(path))
        }
        Some(_) => Err("explicit native Open path must be a string".into()),
    }
}
pub(crate) fn invoke(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let path = parameters(params)?;
    // This is the trusted interactive path only. Automation must receive an
    // independent held-root reader; it cannot reuse this ambient capability.
    if app.automation_input {
        return Err("explicit native Open is unavailable through automation; use capability-scoped app.open".into());
    }
    if path.is_none() && app.services.owned_native_bytes_picker.is_some() {
        return crate::owned_xcf_bytes::start_native(app);
    }
    if app.services.owned_native_open.is_none() {
        return Err("owned native Open service is not configured".into());
    }
    if let Some(path) = path {
        return open_path(app, path, Authority::Interactive);
    }
    let initial_dir =
        app.ui.recent_files.first().filter(|p| p.len() <= 4096).and_then(|p| std::path::Path::new(p).parent()).map(|p| p.to_string_lossy().into_owned());
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir, extensions: Some(vec!["pcraft".into()]) }, |app, answer| match answer {
        FileDialogAnswer::Paths(paths) if paths.len() == 1 => {
            let path = paths.first().ok_or("owned native Open lost its selected path")?;
            open_path(app, path, Authority::Interactive)
        }
        FileDialogAnswer::Failed(error) => Err(error),
        _ => Err("owned native Open requires exactly one platform-authorized path; byte-only/browser input is unsupported".into()),
    })
}
#[derive(Clone, Copy)]
enum Authority {
    Interactive,
    HeldRoot,
}
// The shared hint preserves the original string-error boundary. The selected
// strict Session handoff still performs all current-storage/recipe validation.
fn threshold_candidate(document: &photocraft_doc::Document, control: &ReadControl<'_>) -> Result<bool, String> {
    photocraft_engine::accounted_native_open::is_current_threshold_alpha_candidate(document, control).map_err(|e| e.to_string())
}
/// Only the caller's independently installed held-root capability may read here.
pub(crate) fn invoke_authorized(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let path = parameters(params)?.ok_or("scoped native Open requires an explicit path")?;
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
        return Err("owned native Open path contains NUL".into());
    }
    let quota = app.session.accounted_open_quota().cloned();
    let open = match authority {
        Authority::Interactive => app.services.owned_native_open.as_mut(),
        Authority::HeldRoot => app.services.automation_owned_native_open.as_mut(),
    }
    .ok_or("owned native Open service is not configured for this authority")?;
    let archive = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open(path, quota.as_ref(), &control)))
        .map_err(|_| "owned native Open failed unexpectedly; existing documents were retained".to_owned())??;
    if quota.as_ref().is_some_and(|q| !q.shares_ledger(archive.quota())) {
        return Err("owned native Open returned a different catalog quota".into());
    }
    let name = path.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()).ok_or("owned native Open has no file name")?;
    // Reply strings and serde containers are admitted before construction and
    // retained through adoption. This is a logical envelope, not an RSS claim.
    let reply_bytes = match authority {
        Authority::Interactive => 4096,
        Authority::HeldRoot => path
            .len()
            .checked_add(name.len())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(4096))
            .ok_or("owned native Open reply size overflow")?,
    };
    let _reply_lease = archive.quota().reserve(QuotaKind::Index, reply_bytes as u64).map_err(|e| e.to_string())?;
    control.check(reply_bytes as u64).map_err(|e| e.to_string())?;
    let reply = match authority {
        Authority::Interactive => json!({"warnings":[]}),
        Authority::HeldRoot => json!({"path":path,"name":name,"warnings":[]}),
    };
    control.check(1).map_err(|e| e.to_string())?;
    if threshold_candidate(archive.document(), &control)? {
        app.session.add_owned_native_threshold_alpha_archive(archive, name, Some(path), &control).map_err(|e| e.to_string())?;
    } else if photocraft_engine::accounted_native_open::is_current_threshold_candidate(archive.document(), &control).map_err(|e| e.to_string())? {
        app.session.add_owned_native_threshold_archive(archive, name, Some(path), &control).map_err(|e| e.to_string())?;
    } else if photocraft_engine::accounted_native_open::is_current_point_candidate(archive.document(), &control).map_err(|e| e.to_string())? {
        app.session.add_owned_native_point_archive(archive, name, Some(path), &control).map_err(|e| e.to_string())?;
    } else {
        app.session
            .add_owned_native_archive(archive, AccountedNativeOpenPolicy::KeepCurrentInterpretation, name, Some(path), &control)
            .map_err(|e| e.to_string())?;
    }
    // Identity/path/profile publication is complete. Existing optional user
    // notifications cannot turn that success into an error. Passing None avoids
    // opened_from() repeating uncharged source/path mutation on the paid owner.
    match authority {
        Authority::Interactive => {
            app.push_recent(path);
            crate::jobs_ui::after_open(app, name, None, &[], None);
        }
        Authority::HeldRoot => {
            // Match automation import: no recent entries, scripts or Color Settings.
            app.sync_views();
            app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]);
            app.ui.status_error = false;
            crate::notices::io_warnings(app, &crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]), &[]);
        }
    }
    Ok(reply)
}

/// Pathless browser adoption: the selected label never becomes a runtime path.
pub(crate) fn adopt_bytes(
    app: &mut PhotocraftApp,
    name: &str,
    archive: photocraft_format::owned_document::OwnedDocumentArchive,
    control: &ReadControl<'_>,
) -> Result<Value, String> {
    let _reply_lease = archive.quota().reserve(QuotaKind::Index, 4096).map_err(|e| e.to_string())?;
    control.check(4096).map_err(|e| e.to_string())?;
    let reply = json!({"warnings":[]});
    if threshold_candidate(archive.document(), control)? {
        app.session.add_owned_native_threshold_alpha_archive(archive, name, None, control).map_err(|e| e.to_string())?;
    } else if photocraft_engine::accounted_native_open::is_current_threshold_candidate(archive.document(), control).map_err(|e| e.to_string())? {
        app.session.add_owned_native_threshold_archive(archive, name, None, control).map_err(|e| e.to_string())?;
    } else if photocraft_engine::accounted_native_open::is_current_point_candidate(archive.document(), control).map_err(|e| e.to_string())? {
        app.session.add_owned_native_point_archive(archive, name, None, control).map_err(|e| e.to_string())?;
    } else {
        app.session.add_owned_native_archive(archive, AccountedNativeOpenPolicy::KeepCurrentInterpretation, name, None, control).map_err(|e| e.to_string())?;
    }
    crate::jobs_ui::after_open(app, name, None, &[], None);
    Ok(reply)
}

#[cfg(test)]
#[path = "owned_native_open_tests.rs"]
mod tests;
