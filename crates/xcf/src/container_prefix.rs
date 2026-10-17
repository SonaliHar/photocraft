//! Bounded signature-only routing under the original container budget.
//! This is not a complete wrapper/checksum/EOF validation result.
use super::*;
// Conservative primitive-schedule bound, not elapsed-time calibration. Gzip
// consumes at most a 64KiB encoded window per core call; Bzip polls each 4096
// symbols (each bounded Huffman/256-byte MTF operation); XZ polls dictionary
// initialization each 4096 bytes and index groups of at most 256 records.
const PREFIX_POLL_WORK: u64 = 1 << 24;

pub(super) fn probe<'a, 'b, 'c>(
    bytes: &'a [u8],
    kind: SourceContainer,
    budget: &'b Budget<'c>,
    limits: &ResourceLimits,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<(Cow<'a, [u8]>, Option<Reservation<'b, 'c>>), XcfError> {
    budget.check(BudgetKind::Encoded, bytes.len() as u64, "encoded source")?;
    if bytes.len() as u64 > limits.get(BudgetKind::Encoded) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "encoded source budget exceeded"));
    }
    if kind == SourceContainer::Plain {
        budget.check(BudgetKind::Logical, bytes.len() as u64, "logical source")?;
        if bytes.len() as u64 > limits.get(BudgetKind::Logical) {
            return Err(XcfError::new(ErrorKind::Limit, 0, "logical source budget exceeded"));
        }
        return Ok((Cow::Borrowed(bytes), None));
    }
    let input = usize_limit(limits, BudgetKind::Encoded)?;
    let output = usize_limit(limits, BudgetKind::Logical)?;
    let state = usize_limit(limits, BudgetKind::Decoder)?;
    let index = usize_limit(limits, BudgetKind::Index)?;
    let control = Control {
        budget,
        limits,
        state: RefCell::new(budget.reserve(BudgetKind::Decoder, 0, "wrapper decoder entry")?),
        index: RefCell::new(budget.reserve(BudgetKind::Index, 0, "wrapper index entry")?),
        output: RefCell::new(budget.reserve(BudgetKind::Resident, 0, "wrapper output entry")?),
        failure: RefCell::new(None),
    };
    let mut poll = || {
        if cancelled.is_some_and(|poll| poll()) {
            budget.mark_cancelled();
            return control.io(Err(XcfError::new(ErrorKind::Cancelled, 0, "cancelled during wrapper decode")));
        }
        // A prefix is a separate actual decoding pass. Its bounded vendor
        // poll intervals include encoded/header/state work, never an old-pass
        // allowance or refunded credit.
        control.io(budget.consume_work(PREFIX_POLL_WORK, "logical prefix decoder work"))
    };
    let result = match kind {
        SourceContainer::Gzip => xcf_gzip::decode_prefix_with_reservation(bytes, input, output, state, &mut poll, &mut |kind, n| {
            control.peak(
                match kind {
                    xcf_gzip::Reservation::DecoderState => BudgetKind::Decoder,
                    xcf_gzip::Reservation::OutputOverlap => BudgetKind::Resident,
                },
                n,
            )
        }),
        SourceContainer::Bzip2 => xcf_bzip2::decode_prefix_with_reservation(bytes, input, output, state, &mut poll, &mut |kind, n| {
            control.peak(
                match kind {
                    xcf_bzip2::Reservation::DecoderState => BudgetKind::Decoder,
                    xcf_bzip2::Reservation::OutputOverlap => BudgetKind::Resident,
                },
                n,
            )
        }),
        SourceContainer::Xz => xcf_xz::decode_prefix_with_reservation(bytes, input, output, state, index, &mut poll, &mut |kind, n| {
            control.peak(
                match kind {
                    xcf_xz::Reservation::DecoderState => BudgetKind::Decoder,
                    xcf_xz::Reservation::IndexPeak => BudgetKind::Index,
                    xcf_xz::Reservation::OutputPeak => BudgetKind::Resident,
                },
                n,
            )
        }),
        SourceContainer::Plain => return Err(XcfError::new(ErrorKind::Structure, 0, "plain wrapper dispatch")),
    }
    .map_err(|error| control.error(error))?;
    // A backend's final allocation/copy is still inside cancellation authority;
    // do not hand a completed logical buffer to the parser without a final poll.
    poll().map_err(|error| control.error(error))?;
    budget.check(BudgetKind::Logical, result.len() as u64, "logical source")?;
    let retained_capacity = result.capacity() as u64;
    if retained_capacity > limits.get(BudgetKind::Resident) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "logical buffer retained capacity exceeds resident ceiling"));
    }
    let mut logical_lease = control.output.into_inner();
    let retained_peak = budget
        .used(BudgetKind::Resident)
        .checked_sub(logical_lease.bytes())
        .and_then(|n| n.checked_add(retained_capacity))
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "logical handoff resident accounting overflow"))?;
    if retained_peak > limits.get(BudgetKind::Resident) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "lowered logical handoff resident ceiling exceeded"));
    }
    // Length describes the logical stream. Capacity describes the live Vec
    // allocation retained alongside parsed records; its spare bytes stay charged.
    logical_lease.resize(retained_capacity, "logical buffer retained capacity")?;
    if cancelled.is_some_and(|callback| callback()) {
        budget.mark_cancelled();
        return Err(XcfError::new(ErrorKind::Cancelled, 0, "cancelled at logical buffer handoff"));
    }
    budget.poll("logical buffer handoff")?;
    Ok((Cow::Owned(result), Some(logical_lease)))
}
