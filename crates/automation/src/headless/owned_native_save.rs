//! Active owned-native Save/Save As through existing paid engine completion.
//! Filesystem authority remains the installed Headless capability. Ordinary
//! resident saves keep their existing implementation and notifications.
use super::{Filesystem, Headless};
use crate::AutomationError;
use photocraft_engine::{accounted_commands::PreparedAccountedSaveReply, accounted_native_save::PreparedAccountedNativeSave};
#[cfg(test)]
use photocraft_raster::backing::BackingError;
use photocraft_raster::{Interrupt, backing::ReadControl};
use serde_json::Value;
use std::path::Path;
impl Headless {
    /// Save the active genuine accounted owner, retaining its native precision,
    /// source exclusions and current profile. No source locator grants writes.
    /// Only a complete .pcraft save is supported; this is not flat export/copy.
    pub fn save_owned_native_active(&mut self, path: Option<&Path>, format: Option<&str>, control: &ReadControl<'_>) -> Result<Value, AutomationError> {
        // Relay the actual parent callback exactly once per checked debit. The
        // shared UI-compatible helpers return String; retain actual failure
        // identity through that boundary without parsing its text or resetting
        // the ledger. No callback or borrowed control enters a persistent owner.
        let parent = control;
        let observed = std::sync::Mutex::new(None);
        let relay = |units| match (parent.work)(units) {
            Ok(()) => Ok(()),
            Err(error) => {
                let mut first = observed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if first.is_none() {
                    *first = Some(error);
                }
                Err(error)
            }
        };
        let forwarded = ReadControl { interrupt: parent.interrupt, work: &relay };
        let control = &forwarded;
        let boundary = |message: String| {
            let actual = *observed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match actual {
                Some(error) => photocraft_engine::EngineError::from(error),
                None if parent.interrupt.cancelled() => photocraft_engine::EngineError::Cancelled,
                None => photocraft_engine::EngineError::Other(message),
            }
        };
        control.check(1).map_err(photocraft_engine::EngineError::from)?;
        if matches!(self.filesystem, Filesystem::Denied) {
            return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into()));
        }
        if let Some(value) = format {
            if value.len() > 16 {
                return Err(AutomationError::BadRequest("owned native format exceeds limit".into()));
            }
            control.check(value.len() as u64).map_err(photocraft_engine::EngineError::from)?;
        }
        if format.is_some_and(|value| !value.trim_start_matches('.').eq_ignore_ascii_case("pcraft")) {
            return Err(AutomationError::BadRequest("owned native save supports pcraft format only".into()));
        }
        let state = self.session.active().ok_or(photocraft_engine::EngineError::NoDocument)?;
        if state.doc.accounted().is_none() {
            return Err(AutomationError::BadRequest("owned native save requires the active accounted document".into()));
        }
        let target = match path {
            Some(path) => path.to_str().ok_or_else(|| AutomationError::BadRequest("owned native path must be UTF-8".into()))?,
            None => {
                state.path.as_deref().ok_or_else(|| AutomationError::BadRequest("owned document has no writable native path; pass path for Save As".into()))?
            }
        };
        if target.is_empty() || target.len() > 4096 || !Path::new(target).extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("pcraft"))
        {
            return Err(AutomationError::BadRequest("owned native save requires a bounded .pcraft destination".into()));
        }
        control.check(target.len() as u64).map_err(photocraft_engine::EngineError::from)?;
        if target.contains('\0') {
            return Err(AutomationError::BadRequest("owned native destination contains NUL".into()));
        }
        let protection =
            photocraft_engine::AccountedSaveProtection::prepare(&self.session, control).map_err(|message| AutomationError::Engine(boundary(message)))?;
        // Direct automation IO synthesizes no user command journal entry.
        let reply = PreparedAccountedSaveReply::prepare(&self.session, target, None, control)?;
        let completion = PreparedAccountedNativeSave::prepare(&self.session, target, control)?;
        reply.validate(&self.session, control)?;
        let filesystem = &self.filesystem;
        completion.write_and_publish(&mut self.session, control, |snapshot, target| match filesystem {
            Filesystem::TrustedLocal => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    crate::files::save_accounted_native_selected_path(snapshot, target, &protection.protection, control).map_err(boundary)
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (snapshot, target);
                    Err(photocraft_engine::EngineError::Other("trusted filesystem owned native save is unavailable on this platform".into()))
                }
            }
            Filesystem::Workspace(workspace) => workspace
                .save_accounted_native_with_read_control(snapshot, target, &protection.protection, Default::default(), control)
                .map_err(|error| match error {
                    AutomationError::Engine(error) => error,
                    other => boundary(other.to_string()),
                }),
            Filesystem::Denied => Err(photocraft_engine::EngineError::Other("write authority is absent".into())),
        })?;
        // Committed writer -> engine identity/history -> paid journal/reply.
        // No saved_to(), hook, allocation or cancellation poll follows commit.
        Ok(reply.publish(&mut self.session))
    }
    pub(super) fn save_owned_native_active_bounded(
        &mut self,
        path: Option<&Path>,
        format: Option<&str>,
        interrupt: &Interrupt<'_>,
    ) -> Result<Value, AutomationError> {
        let work = photocraft_io::accounted_operation_work::OperationWork::legacy(8_000_000_000);
        let charge = |units| work.check(units);
        let cancelled = || interrupt.cancelled() || work.deadline_expired();
        let progress = |value| interrupt.progress(value);
        let timed = Interrupt::new(&cancelled, &progress);
        let control = ReadControl { interrupt: &timed, work: &charge };
        if let Some(owner) = self.session.active().and_then(|state| state.doc.accounted())
            && photocraft_engine::accounted_native_open::is_current_standard_work_candidate(owner, &control).map_err(photocraft_engine::EngineError::from)?
        {
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).map_err(|e| AutomationError::Other(e.to_string()))?;
            work.select_standard(&limits).map_err(photocraft_engine::EngineError::from)?;
        }
        self.save_owned_native_active(path, format, &control)
    }
}
#[cfg(test)]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "owned_native_save/box_work_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod box_work_tests;
