//! Exact immutable source provenance, shared by eager and staged transactions.
use super::*;

pub(super) fn stamp(
    result: &mut ImportResult,
    source: &mut dyn photocraft_xcf::Source,
    encoded_bytes: u64,
    control: &Interrupt<'_>,
    budget: &photocraft_xcf::Budget<'_>,
    resident_bytes: usize,
    account: &dyn Fn(usize) -> Result<(), IoError>,
) -> Result<(), IoError> {
    // Hash the exact immutable encoded source through the same capability. A
    // late read/cancellation failure discards the staged document before return.
    let _buffer_lease = budget.reserve(photocraft_xcf::BudgetKind::Resident, 4096, "plain source hash buffer").map_err(accounting_error)?;
    let mut bytes = [0_u8; 4096];
    let mut hash = blake3::Hasher::new();
    let mut offset = 0_u64;
    while offset < encoded_bytes {
        control.check().map_err(|_| IoError::Cancelled)?;
        let count = usize::try_from((encoded_bytes - offset).min(bytes.len() as u64)).map_err(|_| IoError::Unsupported("plain source hash slice".into()))?;
        let chunk = bytes.get_mut(..count).ok_or_else(|| IoError::Unsupported("plain source hash slice".into()))?;
        source.read_exact_at(offset, chunk, budget).map_err(accounting_error)?;
        budget.consume_work(count as u64, "plain source provenance hash").map_err(accounting_error)?;
        hash.update(chunk);
        offset = offset.checked_add(count as u64).ok_or_else(|| IoError::Unsupported("plain source hash overflow".into()))?;
    }
    if source.len() != encoded_bytes {
        return Err(IoError::Unsupported("plain source length changed during import".into()));
    }
    if let Some(state) = &mut result.document.xcf_state {
        let base = resident_bytes;
        account(base.checked_add(138).ok_or_else(|| IoError::Unsupported("plain source stamp accounting overflow".into()))?)?;
        let mut source_hash = String::new();
        let mut container = String::new();
        source_hash.try_reserve_exact(64).map_err(|_| IoError::Unsupported("plain source hash allocation denied".into()))?;
        container.try_reserve_exact(5).map_err(|_| IoError::Unsupported("plain source container allocation denied".into()))?;
        let capacity = source_hash.capacity().checked_add(container.capacity()).ok_or_else(|| IoError::Unsupported("plain source stamp capacity".into()))?;
        if capacity > 138 {
            return Err(IoError::Unsupported("plain source stamp allocation capacity".into()));
        }
        source_hash.push_str(hash.finalize().to_hex().as_str());
        container.push_str("plain");
        account(base.checked_add(capacity).ok_or_else(|| IoError::Unsupported("plain source stamp accounting overflow".into()))?)?;
        state.provenance.source_hash = Some(source_hash);
        state.provenance.container = container;
    }
    control.check().map_err(|_| IoError::Cancelled)?;
    budget.poll("plain source mapped document completed").map_err(accounting_error)?;
    Ok(())
}
