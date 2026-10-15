//! Private single-decode source-selected staging, before owned-model adoption.
//! This does not enable a default loader, AccountedDocument class, Session path,
//! editable-plain exception, resource grant or broader source qualification.
pub(super) use super::staged_source::SelectedStagedFamily;
use super::{Arc, DecodeOptions, Document, ErrorKind, Interrupt, IoError, XcfError, accounting_error, staged_source};
use photocraft_xcf::{Budget, BudgetKind, Reservation, Source};

/// Read-only staged result with its actual original encoded owner and all live
/// decoder, Surface and mapper/provenance reservations. Renderer validation
/// was paid transiently; any future rendering preparation must pay again.
/// These are the original logical accounting receipts, not an exact full-model
/// capacity census or an AccountedDocument ownership proof.
///
/// The private input guard drops encoded storage before its actual same-ledger
/// source fees on every decoder failure and success path. CompletedStagedImport
/// drops its Document before its Surface tokens/registry/decoder reservations. No deep
/// clone/extraction or prepared renderer may escape this borrowed-holder contract.
pub(super) struct StagedSelectedImport<'b, 'a> {
    encoded: Arc<Vec<u8>>,
    completed: staged_source::CompletedStagedImport<'b, 'a>,
    family: SelectedStagedFamily,
    _encoded_fee: Option<Reservation<'b, 'a>>,
    _resident_fee: Option<Reservation<'b, 'a>>,
    _header_names: Reservation<'b, 'a>,
}
struct Input<'b, 'a> {
    encoded: Arc<Vec<u8>>,
    encoded_fee: Option<Reservation<'b, 'a>>,
    resident_fee: Option<Reservation<'b, 'a>>,
}
impl std::fmt::Debug for StagedSelectedImport<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagedSelectedImport")
            .field("document_id", &self.completed.result.document.id)
            .field("encoded_bytes", &self.encoded.len())
            .finish_non_exhaustive()
    }
}
impl StagedSelectedImport<'_, '_> {
    pub(super) fn family(&self) -> SelectedStagedFamily {
        self.family
    }
    pub fn document(&self) -> &Document {
        &self.completed.result.document
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.encoded.as_slice()
    }
}

impl<'b, 'a> StagedSelectedImport<'b, 'a> {
    /// Internal owned-promotion preparation only. This census observes the
    /// actual result capacities, never a caller-asserted statistics receipt.
    pub(in crate::xcf_import) fn report_capacity_with_budget(&self, budget: &Budget<'_>) -> Result<u64, IoError> {
        super::owned_plain_source::census::reports(&self.completed.result, budget)
    }

    /// Consumes the private transaction into existing genuine persistent
    /// reservations. The caller must still perform its independently admitted
    /// current-model/native-store adoption; this grants no renderer/source class.
    pub(in crate::xcf_import) fn detach_owned(
        self,
        extra: &mut [&mut Reservation<'b, 'a>],
        budget: &'b Budget<'a>,
    ) -> Result<staged_source::OwnedStagedImport, IoError> {
        struct Transfer<'b, 'a> {
            // Every failure drops bytes/result BEFORE their actual entry fees.
            encoded: Option<Arc<Vec<u8>>>,
            completed: Option<staged_source::CompletedStagedImport<'b, 'a>>,
            encoded_fee: Option<Reservation<'b, 'a>>,
            resident_fee: Option<Reservation<'b, 'a>>,
            header_names: Reservation<'b, 'a>,
        }
        let Self { encoded, completed, _encoded_fee, _resident_fee, _header_names, .. } = self;
        let mut transfer = Transfer {
            encoded: Some(encoded),
            completed: Some(completed),
            encoded_fee: _encoded_fee,
            resident_fee: _resident_fee,
            header_names: _header_names,
        };
        if extra.len() > 4096 {
            return Err(IoError::Unsupported("staged selected source owned receipt count".into()));
        }
        let count = extra.len().checked_add(3).ok_or_else(|| IoError::Unsupported("staged selected source owned receipt count".into()))?;
        let requested = count
            .checked_mul(std::mem::size_of::<&mut Reservation<'b, 'a>>())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(128))
            .ok_or_else(|| IoError::Unsupported("staged selected source owned receipt index".into()))?;
        let mut index =
            budget.reserve(BudgetKind::Index, checked_u64(requested)?, "staged selected source owned receipt references").map_err(accounting_error)?;
        let mut references = Vec::new();
        references.try_reserve_exact(count).map_err(|_| {
            accounting_error(XcfError {
                kind: ErrorKind::Allocation,
                offset: 0,
                item: String::new(),
                message: "staged selected source owned references allocation".into(),
            })
        })?;
        let actual = references
            .capacity()
            .checked_mul(std::mem::size_of::<&mut Reservation<'b, 'a>>())
            .and_then(|n| n.checked_add(128))
            .ok_or_else(|| IoError::Unsupported("staged selected source owned receipt capacity".into()))?;
        if actual > requested {
            return Err(IoError::Unsupported("staged selected source owned receipt allocator capacity".into()));
        }
        index.resize(checked_u64(actual)?, "staged selected source actual owned references").map_err(accounting_error)?;
        budget.consume_work(count as u64, "staged selected source owned receipt references").map_err(accounting_error)?;
        references.push(transfer.encoded_fee.as_mut().ok_or_else(|| IoError::Unsupported("staged selected source encoded receipt missing".into()))?);
        references.push(transfer.resident_fee.as_mut().ok_or_else(|| IoError::Unsupported("staged selected source resident receipt missing".into()))?);
        references.push(&mut transfer.header_names);
        for fee in extra {
            references.push(&mut **fee);
        }
        // Only checked payload moves follow reference preparation. The called
        // existing transfer owns both payloads across every fallible batch step;
        // entry fees stay here until that transfer succeeds or drops its payload.
        let completed = transfer.completed.take().ok_or_else(|| IoError::Unsupported("staged selected source completion missing".into()))?;
        let encoded = transfer.encoded.take().ok_or_else(|| IoError::Unsupported("staged selected source source missing".into()))?;
        let owned = completed.detach_owned(encoded, &mut references, budget)?;
        drop(references);
        drop(index);
        Ok(owned)
    }
}

fn plus(a: usize, b: usize) -> Result<usize, IoError> {
    a.checked_add(b).ok_or_else(|| IoError::Unsupported("staged selected source retained capacity overflow".into()))
}
fn checked_u64(n: usize) -> Result<u64, IoError> {
    u64::try_from(n).map_err(|_| IoError::Unsupported("staged selected source retained address space".into()))
}
fn source_error(message: &str) -> XcfError {
    XcfError { kind: ErrorKind::Structure, offset: 0, item: String::new(), message: message.into() }
}
struct Bytes<'s, 'c, 'i> {
    bytes: &'s [u8],
    control: &'c Interrupt<'i>,
}
impl Source for Bytes<'_, '_, '_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_exact_at(&mut self, offset: u64, output: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        self.control.check().map_err(|_| source_error("staged selected source source cancelled")).map_err(|mut e| {
            e.kind = ErrorKind::Cancelled;
            e
        })?;
        budget.check(BudgetKind::Encoded, self.len(), "staged selected source source length")?;
        let begin = usize::try_from(offset).map_err(|_| source_error("staged selected source source offset"))?;
        let end = begin.checked_add(output.len()).ok_or_else(|| source_error("staged selected source source span"))?;
        let input = self.bytes.get(begin..end).ok_or_else(|| source_error("staged selected source source short read"))?;
        for (input, target) in input.chunks(4096).zip(output.chunks_mut(4096)) {
            if self.control.cancelled() {
                return Err(XcfError { kind: ErrorKind::Cancelled, offset: 0, item: String::new(), message: "staged selected source source cancelled".into() });
            }
            budget.consume_work(input.len() as u64, "staged selected source source copy")?;
            target.copy_from_slice(input);
        }
        budget.poll("staged selected source source read complete")?;
        if self.control.cancelled() {
            return Err(XcfError { kind: ErrorKind::Cancelled, offset: 0, item: String::new(), message: "staged selected source source cancelled".into() });
        }
        Ok(())
    }
}

/// Decode once, select one original-record family, and run that family's old
/// complete source/mapper/renderer gates. Singleton nonempty effects select the
/// strict Threshold Alpha path; everything else selects strict plain staging.
/// A failure returns without a second decode or another-family retry. The
/// internal family value is returned only after successful full completion.
///
/// Caller supplies the actual operation Budget (including its callbacks/deadline)
/// and retains responsibility for the original input allocation. Before decode,
/// this entry acquires actual Encoded length and Resident full-capacity/header
/// fees on that Budget. Only these live fees justify CallerReservedMemory;
/// the decoder never charges the input length a second time.
/// No new/default ledger or numeric ceiling replaces the supplied one.
pub(super) fn import_staged_selected_bytes_with_budget<'b, 'a>(
    name: &str,
    encoded: Arc<Vec<u8>>,
    control: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    budget: &'b Budget<'a>,
) -> Result<StagedSelectedImport<'b, 'a>, IoError> {
    // Even before a reservation succeeds, place the moved input before its fee.
    let mut input = Input { encoded, encoded_fee: None, resident_fee: None };
    control.check().map_err(|_| IoError::Cancelled)?;
    budget.poll("staged selected source entry").map_err(accounting_error)?;
    if checked_u64(name.len())? > budget.limits().get(BudgetKind::String) {
        return Err(IoError::Unsupported("staged selected source name exceeds source string ceiling".into()));
    }
    input.encoded_fee =
        Some(budget.reserve(BudgetKind::Encoded, checked_u64(input.encoded.len())?, "staged selected source encoded source").map_err(accounting_error)?);
    let resident = plus(input.encoded.capacity(), plus(std::mem::size_of::<Vec<u8>>(), 128)?)?;
    input.resident_fee =
        Some(budget.reserve(BudgetKind::Resident, checked_u64(resident)?, "staged selected source source capacity/header").map_err(accounting_error)?);
    // Match the existing owned staged mapper's conservative name-copy envelope;
    // the original mapper's actual reservations stay retained independently.
    let names =
        name.len().checked_mul(4).and_then(|n| n.checked_add(1024)).ok_or_else(|| IoError::Unsupported("staged selected source name envelope".into()))?;
    let header_names =
        plus(plus(names, plus(std::mem::size_of::<StagedSelectedImport<'_, '_>>(), 128)?)?, std::mem::size_of::<super::owned_xcf_source::OwnedXcfImport>())?;
    let header_names =
        budget.reserve(BudgetKind::Index, checked_u64(header_names)?, "staged selected source holder/name envelope").map_err(accounting_error)?;
    let name_work = checked_u64(name.len().checked_mul(4).ok_or_else(|| IoError::Unsupported("staged selected source name work".into()))?)?;
    budget
        .check(
            BudgetKind::Work,
            budget.used(BudgetKind::Work).checked_add(name_work).ok_or_else(|| IoError::Unsupported("staged selected source cumulative name work".into()))?,
            "staged selected source prospective name copies",
        )
        .map_err(accounting_error)?;
    let mut left = name_work;
    while left != 0 {
        let part = left.min(4096);
        budget.consume_work(part, "staged selected source name copies").map_err(accounting_error)?;
        left -= part;
    }
    let (completed, family) = {
        let mut source = Bytes { bytes: input.encoded.as_slice(), control };
        staged_source::complete_staged_selected_source(
            name,
            &mut source,
            control,
            options,
            budget,
            photocraft_xcf::PlainSourceAccounting::CallerReservedMemory,
        )?
    };
    let Input { encoded, encoded_fee, resident_fee } = input;
    let candidate = StagedSelectedImport { encoded, completed, family, _encoded_fee: encoded_fee, _resident_fee: resident_fee, _header_names: header_names };
    control.check().map_err(|_| IoError::Cancelled)?;
    budget.poll("staged selected source retained handoff").map_err(accounting_error)?;
    Ok(candidate)
}
