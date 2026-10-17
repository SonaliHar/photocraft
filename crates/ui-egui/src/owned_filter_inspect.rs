//! A dedicated retained metadata reply; no model edit, job synchronization or
//! Value-only escape. The transport owns the encoded carrier through flush.
use crate::{
    PhotocraftApp,
    control::{ControlRequest, Outcome},
};
use photocraft_engine::accounted_filter_inspect::{OwnedFilterInspectWire, encode_rpc, prepare, validate_rpc_id};
use photocraft_raster::{Interrupt, backing::ReadControl};
use serde_json::Value;
use std::sync::mpsc::Sender;

pub(crate) struct RetainedQuery {
    pub(crate) id: Value,
    pub(crate) limit: usize,
    pub(crate) reply: Sender<OwnedFilterInspectWire>,
}

pub(crate) fn handle(app: &mut PhotocraftApp, request: &ControlRequest, command: &str) -> Option<Outcome> {
    if !matches!(command, "xcf.inspect" | "xcf.filter.inspect") {
        return None;
    }
    let owner = app.session.active()?.doc.accounted()?;
    let filtered = owner.is_readonly_normal_hierarchy_storage()
        || owner.is_readonly_filtered_flat_point_storage()
        || owner.is_readonly_threshold_alpha_storage()
        || owner.is_readonly_threshold_storage()
        || owner.is_readonly_point_storage()
        || owner.is_readonly_assigned_point_storage()
        || owner.is_readonly_gaussian_group_storage();
    if (command == "xcf.filter.inspect" && !filtered) || (command == "xcf.inspect" && owner.document().xcf_state.is_none()) {
        return None;
    }
    let mut rejected_id = false;
    let result = (|| -> Result<(), String> {
        let retained = request.retained_query.as_ref().ok_or("accounted filter inspection requires a retained control transport")?;
        let fields = request.params.as_object().ok_or("engine.execute parameters must be an object")?;
        if fields.len() > 4 || fields.keys().any(|key| !matches!(key.as_str(), "command" | "id" | "params" | "wait")) {
            return Err("accounted filter inspection accepts command, params and wait only".into());
        }
        if fields.get("wait").is_some_and(|value| !value.is_boolean())
            || fields.get("command").is_some_and(|value| value.as_str() != Some(command))
            || fields.get("id").is_some_and(|value| value.as_str() != Some(command))
        {
            return Err("invalid accounted filter inspection command or wait".into());
        }
        let empty = Value::Null;
        let params = fields.get("params").unwrap_or(&empty);
        let work = photocraft_io::accounted_operation_work::OperationWork::legacy(4_294_967_296);
        let debit = |units| work.check(units);
        let cancelled = || request.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) || work.deadline_expired();
        let interrupt = Interrupt::cancel_only(&cancelled);
        let control = ReadControl { interrupt: &interrupt, work: &debit };
        if validate_rpc_id(&retained.id, &control).is_err() {
            rejected_id = true;
            return Err("retained inspection request ID or control limit rejected".into());
        }
        if photocraft_engine::accounted_native_open::is_current_pure_point_work_candidate(owner, &control).map_err(|error| error.to_string())? {
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).map_err(|error| error.to_string())?;
            work.select_standard(&limits).map_err(|error| error.to_string())?;
        }
        if let Some(authorize) = app.services.automation_command.as_ref() {
            authorize(command, params)?;
        }
        if let Some(authorize) = app.session.authorize {
            authorize(command, params).map_err(|error| error.to_string())?;
        }
        let reply = if command == "xcf.inspect" {
            if filtered || owner.is_readonly_flat_composite_storage() {
                photocraft_engine::accounted_xcf_inspect::prepare(&app.session, params, &control)
            } else {
                photocraft_engine::accounted_plain_xcf_inspect::prepare(&app.session, params, &control)
            }
        } else {
            prepare(&app.session, params, &control)
        }
        .map_err(|error| error.to_string())?;
        let wire = encode_rpc(reply, &retained.id, retained.limit, &control).map_err(|error| error.to_string())?;
        // Queue the actual paid carrier BEFORE waking the ordinary receiver.
        // A timeout/disconnect drops it without changing installed state.
        retained.reply.send(wire).map_err(|_| "retained control receiver closed")?;
        let _ = request.reply.send(Value::Null);
        Ok(())
    })();
    Some(match result {
        Ok(()) => Outcome::Deferred,
        Err(error) if rejected_id => Outcome::Done(serde_json::json!({"id":null,"ok":false,"error":error})),
        Err(error) => Outcome::Done(serde_json::json!({"ok":false,"error":error})),
    })
}

#[cfg(test)]
#[path = "owned_filter_inspect_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "owned_plain_xcf_inspect_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod plain_tests;
