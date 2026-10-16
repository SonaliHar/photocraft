//! Explicit owned native reopen; ordinary Open/profile policy remains unchanged.
use super::owned_xcf_open::{OwnedXcfOpenReply, ReplyError, id_copy_bound};
use super::{Filesystem, Headless};
use crate::AutomationError;
use photocraft_raster::backing::ReadControl;
use serde_json::Value;
use std::path::Path;
/// Same charged summary/envelope carrier as owned XCF Open; the public existing
/// OwnedXcfOpenReply API is unchanged. The value is borrowed while its lease lives.
pub type OwnedNativeOpenReply = OwnedXcfOpenReply;
impl Headless {
    pub fn open_owned_native_keep_current_bounded(&mut self, path: &Path) -> Result<OwnedNativeOpenReply, AutomationError> {
        self.open_owned_native_bounded(path, None)
    }
    pub(crate) fn open_owned_native_rpc_bounded(&mut self, path: &Path, id: &Value) -> Result<OwnedNativeOpenReply, AutomationError> {
        self.open_owned_native_bounded(path, Some(id))
    }
    fn open_owned_native_bounded(&mut self, path: &Path, id: Option<&Value>) -> Result<OwnedNativeOpenReply, AutomationError> {
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).map_err(|e| AutomationError::Other(e.to_string()))?;
        let work = photocraft_io::accounted_operation_work::OperationWork::standard(&limits).map_err(photocraft_engine::EngineError::from)?;
        let charge = |units| work.check(units);
        let expired = || work.deadline_expired();
        let interrupt = photocraft_raster::Interrupt::cancel_only(&expired);
        self.open_owned_native_impl(path, id, &ReadControl { interrupt: &interrupt, work: &charge })
    }
    pub fn open_owned_native_keep_current(&mut self, path: &Path, control: &ReadControl<'_>) -> Result<OwnedNativeOpenReply, AutomationError> {
        self.open_owned_native_impl(path, None, control)
    }
    pub fn open_owned_native_keep_current_with_read_control(&mut self, path: &str, control: &ReadControl<'_>) -> Result<OwnedNativeOpenReply, AutomationError> {
        self.open_owned_native_impl(Path::new(path), None, control)
    }
    fn open_owned_native_impl(&mut self, path: &Path, id: Option<&Value>, control: &ReadControl<'_>) -> Result<OwnedNativeOpenReply, AutomationError> {
        let id_bytes = id.map(|v| id_copy_bound(v, control)).transpose().map_err(ReplyError::into_automation)?;
        let path = path.to_str().ok_or_else(|| AutomationError::BadRequest("owned native path must be UTF-8".into()))?;
        if path.is_empty() || path.len() > 4096 || !Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pcraft")) {
            return Err(AutomationError::BadRequest("explicit keepCurrent requires a bounded .pcraft path".into()));
        }
        control.check(path.len() as u64).map_err(photocraft_engine::EngineError::from)?;
        if path.contains('\0') {
            return Err(AutomationError::BadRequest("owned native path contains NUL".into()));
        }
        if matches!(self.filesystem, Filesystem::Denied) {
            return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into()));
        }
        if self.session.active_job().is_some() {
            return Err(AutomationError::BadRequest("owned native Open requires an idle session".into()));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = id_bytes;
            Err(AutomationError::BadRequest("native path capture is unavailable on this platform".into()))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let quota = match self.session.accounted_open_quota() {
                Some(q) => q.clone(),
                None => photocraft_io::accounted_open_quota::new_accounted_open_quota(control).map_err(|e| match e {
                    photocraft_io::accounted_open_quota::AccountedOpenQuotaError::Backing(e) => AutomationError::Engine(e.into()),
                    other => AutomationError::Other(other.to_string()),
                })?,
            };
            let maximum = u64::from(photocraft_io::native_plane_store::NATIVE_STORE_MAX_BLOCKS)
                .checked_mul(photocraft_io::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64)
                .ok_or_else(|| AutomationError::BadRequest("native source limit overflow".into()))?;
            let archive = match &self.filesystem {
                Filesystem::TrustedLocal => crate::files::open_accounted_native_selected_path(path, Default::default(), quota.clone(), control)?,
                Filesystem::Workspace(root) => root.open_accounted_native_with_read_control(path, maximum, Default::default(), quota.clone(), control)?,
                Filesystem::Denied => return Err(AutomationError::BadRequest("read authority is absent".into())),
            };
            if !quota.shares_ledger(archive.quota()) {
                return Err(AutomationError::Other("owned native reader changed catalog quota".into()));
            }
            let label = path
                .rsplit(['/', '\\'])
                .next()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AutomationError::BadRequest("owned native path has no file name".into()))?;
            let mut reply = OwnedXcfOpenReply::new_native(&archive, path, self.session.documents().len(), control).map_err(ReplyError::into_automation)?;
            if let (Some(id), Some(bound)) = (id, id_bytes) {
                reply.wrap_rpc(id, bound, control).map_err(ReplyError::into_automation)?;
                reply.validate_rpc_wire_size(control).map_err(ReplyError::into_automation)?;
            }
            let threshold = if archive.current_storage_hint().is_some() { false } else {
                photocraft_engine::accounted_native_open::is_current_threshold_alpha_candidate(archive.document(), control)
                    .map_err(photocraft_engine::EngineError::from)?
            };
            control.check(1).map_err(photocraft_engine::EngineError::from)?;
            if archive.current_storage_hint().is_some() {
                self.session.add_owned_native_filtered_flat_archive(archive, label, Some(path), control)?;
            } else if threshold {
                self.session.add_owned_native_threshold_alpha_archive(archive, label, Some(path), control)?;
            } else if photocraft_engine::accounted_native_open::is_current_threshold_candidate(archive.document(), control)
                .map_err(photocraft_engine::EngineError::from)?
            {
                self.session.add_owned_native_threshold_archive(archive, label, Some(path), control)?;
            } else if photocraft_engine::accounted_native_open::is_current_separable_blend_candidate(archive.document(), control)
                .map_err(photocraft_engine::EngineError::from)?
            {
                self.session.add_owned_native_separable_blend_archive(archive, label, Some(path), control)?;
            } else if photocraft_engine::accounted_native_open::is_current_isolated_gaussian_group_candidate(archive.document(), control)
                .map_err(photocraft_engine::EngineError::from)?
            {
                self.session.add_owned_native_isolated_gaussian_group_archive(archive, label, Some(path), control)?;
            } else if photocraft_engine::accounted_native_open::is_current_isolated_normal_group_candidate(archive.document(), control)
                .map_err(photocraft_engine::EngineError::from)?
            {
                self.session.add_owned_native_isolated_normal_group_archive(archive, label, Some(path), control)?;
            } else if photocraft_engine::accounted_native_open::is_current_analytic_geometry_candidate(archive.document(), control)
                .map_err(photocraft_engine::EngineError::from)?
            {
                self.session.add_owned_native_analytic_geometry_archive(archive, label, Some(path), control)?;
            } else if photocraft_engine::accounted_native_open::is_owned_current_filtered_flat_candidate(&archive, control)
                .map_err(photocraft_engine::EngineError::from)?
            {
                self.session.add_owned_native_filtered_flat_archive(archive, label, Some(path), control)?;
            } else if photocraft_engine::accounted_native_open::is_current_point_candidate(archive.document(), control)
                .map_err(photocraft_engine::EngineError::from)?
            {
                self.session.add_owned_native_point_archive(archive, label, Some(path), control)?;
            } else {
                self.session.add_owned_native_archive(
                    archive,
                    photocraft_engine::accounted_native_open::AccountedNativeOpenPolicy::KeepCurrentInterpretation,
                    label,
                    Some(path),
                    control,
                )?;
            }
            // Complete reply and optional wire envelope already exist. No poll,
            // source mutation, allocation or alternate handoff follows publication.
            Ok(reply)
        }
    }
}
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "owned_native_open_tests.rs"]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "owned_native_box_work_tests.rs"]
mod box_work_tests;
