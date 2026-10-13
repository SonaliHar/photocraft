//! One cancellation/deadline authority fence for decoded-source handoff.
//! A caller-owned Budget may have a different callback from DecodeOptions.
use crate::{Budget, XcfError};
pub(super) fn poll(budget: &Budget<'_>, cancelled: Option<&dyn Fn() -> bool>, phase: &str) -> Result<(), XcfError> {
    // Preserve existing deadline/budget callback precedence at admission.
    budget.poll(phase)?;
    if cancelled.is_some_and(|callback| callback()) {
        budget.mark_cancelled();
        if phase == "import entry" {
            return Err(XcfError::new(crate::ErrorKind::Cancelled, 0, "cancelled at import entry"));
        }
        return budget.poll(phase);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BudgetKind, ErrorKind, ResourceLimits, ResourcePolicy};
    #[test]
    fn separate_options_token_cancels_completion_and_latches_budget() {
        let budget = Budget::new(
            ResourceLimits::for_policy(if cfg!(target_arch = "wasm32") { ResourcePolicy::WasmStandard } else { ResourcePolicy::NativeStandard }).unwrap(),
            None,
        );
        let cancelled = std::cell::Cell::new(false);
        let token = || cancelled.get();
        poll(&budget, Some(&token), "entry").unwrap();
        let lease = budget.reserve(BudgetKind::Resident, 32, "decoded handoff").unwrap();
        // This models a100% progress callback changing the separate source token.
        cancelled.set(true);
        assert_eq!(poll(&budget, Some(&token), "completion").unwrap_err().kind, ErrorKind::Cancelled);
        cancelled.set(false);
        assert_eq!(budget.poll("subsequent authority").unwrap_err().kind, ErrorKind::Cancelled);
        drop(lease);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
    }
}
