//! Real same-ledger target storage derived from the actual immutable view.
use super::*;
#[derive(Clone, Copy)]
pub(super) enum TargetMode {
    Zero,
    Explicit(LayerId),
    Selected,
}
pub(super) struct Targets {
    ids: Vec<LayerId>,
    _lease: Option<QuotaLease>,
}
impl Targets {
    pub(super) fn ids(&self) -> &[LayerId] {
        &self.ids
    }
    pub(super) fn prepare(state: &crate::DocState, mode: TargetMode, c: &ReadControl<'_>) -> Result<Self> {
        if matches!(mode, TargetMode::Zero) {
            c.check(1)?;
            return Ok(Self { ids: Vec::new(), _lease: None });
        }
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        view_check(state, c)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        let selected = view.selected();
        let active = view.target().active;
        let mut active_selected = false;
        for id in selected {
            c.check(1)?;
            active_selected |= Some(*id) == active;
        }
        let wanted = |id: LayerId| -> Result<bool> {
            match mode {
                TargetMode::Zero => Ok(false),
                TargetMode::Explicit(want) => Ok(id == want),
                TargetMode::Selected if active_selected => {
                    for want in selected {
                        c.check(1)?;
                        if *want == id {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                TargetMode::Selected => Ok(Some(id) == active),
            }
        };
        let expected = match mode {
            TargetMode::Selected if active_selected => selected.len(),
            _ => 1,
        };
        if expected == 0 || expected > 4096 || state.doc.layers.len() > 4096 {
            return Err(BackingError::Bounds.into());
        }
        let mut count = 0usize;
        for layer in &state.doc.layers {
            c.check(1)?;
            if wanted(layer.id)? {
                target_check(owner, layer.id, c)?;
                count = count.checked_add(1).ok_or(BackingError::Length)?;
            }
        }
        if count != expected {
            return Err(BackingError::Integrity.into());
        }
        // This destination retains no detached view authority. All IDs are
        // copied only after bounded membership/count/lock admission above.
        let bytes = count
            .checked_mul(std::mem::size_of::<LayerId>())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 128))
            .ok_or(BackingError::Length)?;
        let mut fee = owner.quota().reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
        c.check(count.checked_mul(std::mem::size_of::<LayerId>()).ok_or(BackingError::Length)? as u64)?;
        let mut ids = Vec::new();
        ids.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
        if ids.capacity() > count.checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation.into());
        }
        for layer in &state.doc.layers {
            c.check(1)?;
            if wanted(layer.id)? {
                if ids.len() >= count {
                    return Err(BackingError::Integrity.into());
                }
                ids.push(layer.id);
            }
        }
        if ids.len() != count {
            return Err(BackingError::Integrity.into());
        }
        let actual = ids
            .capacity()
            .checked_mul(std::mem::size_of::<LayerId>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 128))
            .ok_or(BackingError::Length)?;
        fee.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        let result = Self { ids, _lease: Some(fee) };
        c.check(1)?;
        Ok(result)
    }
}
