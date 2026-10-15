//! Typed installed-snapshot query. No job synchronization, Value extraction,
//! Session mutation, locator authority or per-stage work allowance is introduced.
use super::{Filesystem, Headless};
use crate::{AutomationError, workspace::authorize_engine_command};
use photocraft_engine::accounted_filter_inspect::{OwnedFilterInspectReply, OwnedFilterInspectWire};
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl},
};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

const COMMAND: &str = "xcf.filter.inspect";
fn bounded<R>(run: impl FnOnce(&ReadControl<'_>) -> Result<R, AutomationError>) -> Result<R, AutomationError> {
    let used = AtomicU64::new(0);
    let charge = |units| {
        used.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(units).filter(|n| *n <= 8_000_000_000))
            .map_err(|_| BackingError::Work)?;
        Ok(())
    };
    run(&ReadControl { interrupt: &Interrupt::NONE, work: &charge })
}
impl Headless {
    /// Cheap runtime routing only; the paid query validates the real class.
    pub fn has_owned_filter_inspection(&self) -> bool {
        self.session
            .active()
            .and_then(|state| state.doc.accounted())
            .is_some_and(|owner| owner.is_readonly_threshold_alpha_storage() || owner.is_readonly_threshold_storage())
    }
    /// Routing hint only: CURRENT XCF metadata is not original-source proof.
    /// The selected paid factory must still validate its exact runtime class.
    pub fn has_owned_inspection_command(&self, id: &str) -> bool {
        match id {
            "xcf.filter.inspect" => self.has_owned_filter_inspection(),
            "xcf.inspect" => self.session.active().is_some_and(|state| state.doc.accounted().is_some_and(|owner| owner.document().xcf_state.is_some())),
            _ => false,
        }
    }
    pub fn inspect_owned_filter_bounded(&mut self, params: &Value, wait: bool) -> Result<OwnedFilterInspectReply, AutomationError> {
        self.inspect_owned_command_bounded(COMMAND, params, wait)
    }
    pub fn inspect_owned_filter_with_read_control(
        &mut self,
        params: &Value,
        wait: bool,
        control: &ReadControl<'_>,
    ) -> Result<OwnedFilterInspectReply, AutomationError> {
        self.inspect_owned_command_with_read_control(COMMAND, params, wait, control)
    }
    pub fn inspect_owned_filter_cli_bounded(&mut self, params: &Value, compact: bool, limit: usize) -> Result<OwnedFilterInspectWire, AutomationError> {
        self.inspect_owned_command_cli_bounded(COMMAND, params, compact, limit)
    }
    pub(crate) fn inspect_owned_filter_rpc_bounded(&mut self, params: &Value, wait: bool, id: &Value) -> Result<OwnedFilterInspectWire, AutomationError> {
        self.inspect_owned_command_rpc_bounded(COMMAND, params, wait, id)
    }
    /// Whitelisted typed metadata query. Both wait forms inspect the currently
    /// installed snapshot inline; completed queued jobs are not applied.
    pub fn inspect_owned_command_bounded(&mut self, id: &str, params: &Value, wait: bool) -> Result<OwnedFilterInspectReply, AutomationError> {
        bounded(|control| self.inspect_owned_command_with_read_control(id, params, wait, control))
    }
    pub fn inspect_owned_command_with_read_control(
        &mut self,
        id: &str,
        params: &Value,
        _wait: bool,
        control: &ReadControl<'_>,
    ) -> Result<OwnedFilterInspectReply, AutomationError> {
        control.check(1).map_err(photocraft_engine::EngineError::from)?;
        if !matches!(id, "xcf.filter.inspect" | "xcf.inspect") {
            return Err(AutomationError::BadRequest("unsupported owned inspection command".into()));
        }
        if !matches!(&self.filesystem, Filesystem::TrustedLocal) {
            authorize_engine_command(id, params)?;
        }
        if let Some(authorize) = self.session.authorize {
            authorize(id, params)?;
        }
        Ok(match id {
            "xcf.filter.inspect" => photocraft_engine::accounted_filter_inspect::prepare(&self.session, params, control)?,
            "xcf.inspect" => {
                if self.has_owned_filter_inspection() {
                    photocraft_engine::accounted_xcf_inspect::prepare(&self.session, params, control)?
                } else {
                    photocraft_engine::accounted_plain_xcf_inspect::prepare(&self.session, params, control)?
                }
            }
            _ => return Err(AutomationError::BadRequest("unsupported owned inspection command".into())),
        })
    }
    /// Query and complete CLI encoding share one cumulative bounded control.
    pub fn inspect_owned_command_cli_bounded(
        &mut self,
        id: &str,
        params: &Value,
        compact: bool,
        limit: usize,
    ) -> Result<OwnedFilterInspectWire, AutomationError> {
        bounded(|control| {
            let reply = self.inspect_owned_command_with_read_control(id, params, true, control)?;
            Ok(photocraft_engine::accounted_filter_inspect::encode_cli(reply, compact, limit, control)?)
        })
    }
    pub(crate) fn inspect_owned_command_rpc_bounded(
        &mut self,
        command: &str,
        params: &Value,
        wait: bool,
        id: &Value,
    ) -> Result<OwnedFilterInspectWire, AutomationError> {
        bounded(|control| {
            photocraft_engine::accounted_filter_inspect::validate_rpc_id(id, control)?;
            let reply = self.inspect_owned_command_with_read_control(command, params, wait, control)?;
            Ok(photocraft_engine::accounted_filter_inspect::encode_rpc(reply, id, crate::budgets::MAX_RESPONSE_BYTES, control)?)
        })
    }
}

#[cfg(test)]
#[path = "owned_filter_inspect_tests.rs"]
mod tests;
