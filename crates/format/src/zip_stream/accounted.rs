//! Logical-capacity leases for optional ZIP construction, not allocator/RSS bytes.
use crate::{FormatError, Result};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
use std::ops::{Deref, DerefMut};
pub(super) type Accounting<'c, 'a> = Option<(&'c ReadControl<'a>, &'c BackingQuota)>;
pub(super) fn charge(account: Accounting<'_, '_>, control: &Interrupt<'_>, units: u64) -> Result<()> {
    if let Some((ctl, _)) = account {
        ctl.check(units).map_err(|e| match e {
            BackingError::Cancelled => FormatError::Cancelled,
            other => FormatError::LimitExceeded(format!("ZIP construction control: {other}")),
        })
    } else {
        super::poll(control)
    }
}
pub(super) fn reserve(account: Accounting<'_, '_>, bytes: u64) -> Result<Option<QuotaLease>> {
    account.map(|(_, quota)| quota.reserve(QuotaKind::Resident, bytes).map_err(|_| FormatError::LimitExceeded("ZIP resident quota".into()))).transpose()
}
/// Payload precedes lease. Unexpected allocator capacity beyond prepaid envelope
/// fails before publication; this accounts logical Vec capacity, not total RSS.
pub(super) struct OwnedVec<T> {
    data: Vec<T>,
    _lease: Option<QuotaLease>,
}
impl<T> OwnedVec<T> {
    pub(super) fn with_capacity(count: usize, account: Accounting<'_, '_>, control: &Interrupt<'_>) -> Result<Self> {
        charge(account, control, count as u64)?;
        if count == 0 {
            return Ok(Self { data: Vec::new(), _lease: None });
        }
        let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or_else(|| FormatError::LimitExceeded("ZIP capacity overflow".into()))?;
        let envelope = bytes.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or_else(|| FormatError::LimitExceeded("ZIP capacity envelope".into()))?;
        let mut lease = reserve(account, envelope as u64)?;
        let mut data = Vec::new();
        data.try_reserve_exact(count).map_err(|_| FormatError::LimitExceeded("ZIP allocation".into()))?;
        let actual = data.capacity().checked_mul(std::mem::size_of::<T>()).ok_or_else(|| FormatError::LimitExceeded("ZIP actual capacity overflow".into()))?;
        if actual > envelope { return Err(FormatError::LimitExceeded("ZIP capacity exceeded prepaid envelope".into())); }
        if let Some(token) = &mut lease {
            token.reconcile_capacity(actual as u64).map_err(|_| FormatError::LimitExceeded("ZIP capacity exceeded prepaid envelope".into()))?;
        }
        charge(account, control, 0)?;
        Ok(Self { data, _lease: lease })
    }
}
impl OwnedVec<u8> {
    pub(super) fn bytes(length: u64, account: Accounting<'_, '_>, control: &Interrupt<'_>) -> Result<Self> {
        let count = usize::try_from(length).map_err(|_| FormatError::LimitExceeded("ZIP address space".into()))?;
        let mut result = Self::with_capacity(count, account, control)?;
        // Avoid a whole declared-entry zero fill without cancellation. Allocation
        // itself is not interruptible; initialization is bounded to 64 KiB steps.
        while result.data.len() < count {
            let next = result.data.len().checked_add((count - result.data.len()).min(64 << 10))
                .ok_or_else(|| FormatError::LimitExceeded("ZIP initialization overflow".into()))?;
            charge(account, control, (next - result.data.len()) as u64)?;
            result.data.resize(next, 0);
        }
        charge(account, control, 0)?;
        Ok(result)
    }
}
impl<T> Deref for OwnedVec<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl<T> DerefMut for OwnedVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}
pub(super) struct OwnedName {
    pub(super) value: String,
    pub(super) lease: Option<QuotaLease>,
}
pub(super) fn name(value: &str, account: Accounting<'_, '_>, control: &Interrupt<'_>) -> Result<OwnedName> {
    let mut bytes = OwnedVec::<u8>::with_capacity(value.len(), account, control)?;
    for block in value.as_bytes().chunks(4096) {
        charge(account, control, block.len() as u64)?;
        bytes.data.extend_from_slice(block);
    }
    let OwnedVec { data, _lease } = bytes;
    // Existing validation established UTF8; conversion consumes the exact allocation.
    let value = String::from_utf8(data).map_err(|_| FormatError::corrupt("ZIP name encoding"))?;
    Ok(OwnedName { value, lease: _lease })
}
/// Internally polled in-place heapsort. No user comparator and no quota lock is
/// held during borrowed control calls. Comparisons and swaps charge bounded work.
pub(super) fn sort<T>(
    values: &mut [T],
    account: Accounting<'_, '_>,
    control: &Interrupt<'_>,
    compare: impl Fn(&T, &T) -> Result<std::cmp::Ordering>,
) -> Result<()> {
    fn sift<T>(
        v: &mut [T],
        mut root: usize,
        end: usize,
        a: Accounting<'_, '_>,
        c: &Interrupt<'_>,
        cmp: &impl Fn(&T, &T) -> Result<std::cmp::Ordering>,
    ) -> Result<()> {
        loop {
            charge(a, c, 1)?;
            let Some(mut child) = root.checked_mul(2).and_then(|n| n.checked_add(1)) else { return Err(FormatError::corrupt("ZIP sort index")) };
            if child >= end {
                return Ok(());
            }
            let right = child.checked_add(1).ok_or_else(|| FormatError::corrupt("ZIP sort index"))?;
            if right < end
                && cmp(
                    v.get(child).ok_or_else(|| FormatError::corrupt("ZIP sort range"))?,
                    v.get(right).ok_or_else(|| FormatError::corrupt("ZIP sort range"))?,
                )?
                .is_lt()
            {
                child = right;
            }
            if !cmp(v.get(root).ok_or_else(|| FormatError::corrupt("ZIP sort range"))?, v.get(child).ok_or_else(|| FormatError::corrupt("ZIP sort range"))?)?
                .is_lt()
            {
                return Ok(());
            }
            v.swap(root, child);
            root = child;
        }
    }
    for root in (0..values.len() / 2).rev() {
        sift(values, root, values.len(), account, control, &compare)?;
    }
    for end in (1..values.len()).rev() {
        charge(account, control, 1)?;
        values.swap(0, end);
        sift(values, 0, end, account, control, &compare)?;
    }
    Ok(())
}

#[cfg(test)]
mod initialization_tests {
    use super::*;
    #[test]
    fn declared_output_initialization_polls_between_chunks() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 3;
        assert!(matches!(OwnedVec::bytes(200_000, None, &Interrupt::cancel_only(&cancel)), Err(FormatError::Cancelled)));
        assert!(calls.load(std::sync::atomic::Ordering::Relaxed) >= 4);
    }
}
