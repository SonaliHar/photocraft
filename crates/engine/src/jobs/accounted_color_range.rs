//! Accounted Color Range uses the existing worker service and its actual token,
//! with an owned journal published by the typed transaction rather than hooks.
use super::*;
use crate::{accounted_color_range_banded::CapturedAccountedColorRange, accounted_commands::PreparedAccountedCommand};
use photocraft_raster::backing::ReadControl;
use photocraft_raster::backing::{BackingError, quota::QuotaKind};

impl Session {
    pub(super) fn dispatch_accounted_color_range(&mut self, params: Value, background: bool) -> Result<Started> {
        // Capture performs only bounded metadata/parameter ownership. CMS and
        // all source reads occur in the worker under its original JobCtx.
        let capture_control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let captured = CapturedAccountedColorRange::capture(self, &params, &capture_control)?;
        let journal: crate::accounted_commands::PreparedAccountedJobJournal = captured
            .with_control(&Interrupt::NONE, |control| Ok(PreparedAccountedCommand::prepare(self, "select.colorRange", &params, control)?.into_job_journal()))?;
        // Actual known headers plus a conservative fixed envelope for callback
        // control storage in the existing executor. Global table capacity,
        // thread stacks and channel allocator internals remain its boundary;
        // this is not a process-RSS claim.
        let quota = self.active().and_then(|state| state.doc.quota()).ok_or(BackingError::Unsupported)?;
        let metadata_bytes =
            std::mem::size_of::<Running>().checked_add(std::mem::size_of::<Shared>()).and_then(|bytes| bytes.checked_add(8192)).ok_or(BackingError::Length)?;
        let dispatch_lease = Arc::new(quota.reserve(QuotaKind::Index, metadata_bytes as u64).map_err(|_| BackingError::Quota)?);
        let worker_dispatch = Arc::clone(&dispatch_lease);
        let mut command = String::new();
        command.try_reserve_exact("select.colorRange".len()).map_err(|_| BackingError::Allocation)?;
        if command.capacity() > 128 {
            return Err(BackingError::Allocation.into());
        }
        command.push_str("select.colorRange");
        let previous_spawn = self.jobs.spawn;
        self.jobs.spawn = background;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(
                self,
                "Color Range",
                true,
                move |ctx| {
                    ctx.retain_accounted_dispatch(worker_dispatch);
                    Ok((captured.prepare(ctx)?, ctx.clone()))
                },
                move |session, (prepared, ctx)| {
                    let cancelled = || ctx.cancelled();
                    let interrupt = Interrupt::cancel_only(&cancelled);
                    prepared.with_control(&interrupt, |control| journal.validate(session, control))?;
                    let response = prepared.apply(session, &ctx)?;
                    // Both document and journal payloads were paid and fenced before
                    // the edit. No generic fallible bookkeeping follows publication.
                    Ok(journal.publish(session, response))
                },
            )
        }))
        .unwrap_or_else(|_| Err(EngineError::Other("Color Range failed with an internal error; the document is unchanged".into())));
        self.jobs.spawn = previous_spawn;
        let pending = self.jobs.pending.take();
        match (result, pending) {
            (Err(error), Some(job)) => {
                job.ctx.cancel();
                Err(error)
            }
            (Err(error), None) => Err(error),
            (Ok(_), Some(mut job)) => {
                if job.label.capacity() > 128 {
                    job.ctx.cancel();
                    return Err(BackingError::Allocation.into());
                }
                job.command = command;
                // The prepaid journal owns the exact command parameters.
                // Running.params is used only by the bypassed generic hook.
                job.params = Value::Null;
                job.journal = false;
                job.prepaid_bookkeeping = true;
                job._accounted_dispatch = Some(dispatch_lease);
                let id = job.id;
                self.jobs.running.push(job);
                Ok(Started::Job(id))
            }
            (Ok(value), None) => Ok(Started::Done(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};

    /// UNRUN: cancellation may drop Running before the draining worker's token.
    #[test]
    fn cancelled_running_owner_cannot_refund_live_worker_control_storage() {
        let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 1 << 20 });
        let before = quota.usage().unwrap();
        let running_owner = Arc::new(quota.reserve(QuotaKind::Index, 8192).unwrap());
        let ctx = JobCtx::new();
        let worker_ctx = ctx.clone();
        worker_ctx.retain_accounted_dispatch(Arc::clone(&running_owner));
        ctx.cancel();
        drop(running_owner);
        drop(ctx);
        assert!(worker_ctx.cancelled());
        assert_eq!(quota.usage().unwrap().index, before.index + 8192);
        drop(worker_ctx);
        assert_eq!(quota.usage().unwrap(), before);
    }
}
