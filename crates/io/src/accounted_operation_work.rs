//! Finite operation work accounting for explicitly selected CURRENT Box services.
//!
//! This is a policy ceiling, not a completion estimate or source/render grant.
//! Callers must select the actual eligible route themselves. Existing caller-owned
//! `ReadControl`s are never changed by this helper. No callback escapes into a
//! source, document or persistent receipt.
use photocraft_raster::backing::BackingError;
use photocraft_xcf::{BudgetKind, ResourceLimits, ResourcePolicy};
use std::sync::{Mutex, MutexGuard};
use web_time::Instant;

#[derive(Debug)]
struct State {
    used: u64,
    limit: u64,
    started: Option<Instant>,
    selected: Option<(ResourcePolicy, u64)>,
}

/// One cumulative work counter and active-operation clock. Wrap in an `Arc`
/// when capture and adoption belong to the same operation; do not reconstruct
/// the meter at either boundary. Legacy paths retain their supplied work cap
/// and have no new deadline.
#[derive(Debug)]
pub struct OperationWork {
    state: Mutex<State>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkDiagnostics {
    pub used: u64,
    pub limit: u64,
    pub policy: Option<ResourcePolicy>,
    pub deadline_ms: Option<u64>,
    pub deadline_expired: bool,
}

impl OperationWork {
    /// A finite legacy cap, including zero as a real zero allowance.
    pub fn legacy(baseline: u64) -> Self {
        Self { state: Mutex::new(State { used: 0, limit: baseline, started: None, selected: None }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Start only at actual active work, after interactive picker/user wait.
    /// Repeated calls preserve the original start, including before selection.
    pub fn begin_active(&self) {
        let mut state = self.state();
        if state.started.is_none() {
            state.started = Some(Instant::now());
        }
    }

    /// Select a validated standard profile once. Lowered limits remain lowered;
    /// the old bootstrap usage and first active clock are retained. A valid
    /// selection that already exceeds work/deadline fails closed and stays
    /// selected, so retrying cannot grant another allowance.
    ///
    /// This changes no memory limits and promises no provider will complete.
    pub fn select_standard_box(&self, limits: &ResourceLimits) -> Result<(), BackingError> {
        let policy = limits.policy();
        if policy == ResourcePolicy::NativeLarge || cfg!(target_arch = "wasm32") && policy != ResourcePolicy::WasmStandard {
            return Err(BackingError::Unsupported);
        }
        let ceiling = ResourceLimits::for_policy(policy).map_err(|_| BackingError::Unsupported)?;
        let work = limits.get(BudgetKind::Work);
        let deadline = limits.get(BudgetKind::Deadline);
        if work > ceiling.get(BudgetKind::Work) || deadline > ceiling.get(BudgetKind::Deadline) {
            return Err(BackingError::Bounds);
        }
        let mut state = self.state();
        if state.selected.is_some() {
            return Err(BackingError::Integrity);
        }
        state.limit = work;
        state.selected = Some((policy, deadline));
        if expired(&state) {
            return Err(BackingError::Cancelled);
        }
        if state.used > state.limit {
            return Err(BackingError::Work);
        }
        Ok(())
    }

    /// Charge every actual debit, including provider work, without refunds or
    /// discounts. A zero check still starts active work and polls the deadline.
    /// Use the original caller cancellation token as well as this meter.
    pub fn check(&self, units: u64) -> Result<(), BackingError> {
        let mut state = self.state();
        if state.started.is_none() {
            state.started = Some(Instant::now());
        }
        if expired(&state) {
            return Err(BackingError::Cancelled);
        }
        let next = state.used.checked_add(units).ok_or(BackingError::Work)?;
        if next > state.limit {
            return Err(BackingError::Work);
        }
        state.used = next;
        if expired(&state) {
            return Err(BackingError::Cancelled);
        }
        Ok(())
    }

    /// OR this with the original token for interrupt-only lower kernels. This
    /// does not start a clock while a picker is awaiting interactive input.
    /// Expiry maps to `Cancelled`; diagnostics distinguish deadline from token.
    pub fn deadline_expired(&self) -> bool {
        expired(&self.state())
    }

    pub fn diagnostics(&self) -> WorkDiagnostics {
        let state = self.state();
        WorkDiagnostics {
            used: state.used,
            limit: state.limit,
            policy: state.selected.map(|(policy, _)| policy),
            deadline_ms: state.selected.map(|(_, deadline)| deadline),
            deadline_expired: expired(&state),
        }
    }
}

fn expired(state: &State) -> bool {
    match (state.started, state.selected) {
        (Some(started), Some((_, deadline))) => started.elapsed().as_millis() >= u128::from(deadline),
        _ => false,
    }
}

#[cfg(test)]
#[path = "accounted_operation_work_tests.rs"]
mod tests;
