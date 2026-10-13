//! Staged plain-XCF source integration. Filesystem authority remains with caller.
use super::*;

/// Import a caller-owned immutable plain source without materializing its encoded
/// bytes. Accounting mode is a trusted contract; it does not grant file access.
/// Decoded planes and the published document remain eager resident storage.
pub fn import_plain_source_with_budget(
    name: &str,
    source: &mut dyn photocraft_xcf::Source,
    control: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    budget: &photocraft_xcf::Budget<'_>,
    accounting: photocraft_xcf::PlainSourceAccounting,
) -> Result<ImportResult, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    let mapping_options =
        DecodeOptions { limits: options.limits.clone(), allow_finite_drawable_alpha: true, cancelled: options.cancelled, progress: options.progress };
    let photocraft_xcf::BudgetDecodedImport { decoded, reservations } =
        photocraft_xcf::decode_plain_source_with_budget(source, &mapping_options, budget, accounting).map_err(accounting_error)?;
    let mut file = decoded.file;
    // The compatibility parser counter includes encoded length. Source leases
    // already account its actual storage; mapping counts resident objects only.
    let encoded = usize::try_from(decoded.encoded_bytes).map_err(|_| IoError::Unsupported("plain source accounting address space".into()))?;
    file.charged_bytes = file.charged_bytes.checked_sub(encoded).ok_or_else(|| IoError::Unsupported("plain source accounting mismatch".into()))?;
    let previous = std::cell::Cell::new(file.charged_bytes);
    let leases = std::cell::RefCell::new(reservations);
    let account = |used: usize| {
        if used > previous.get() {
            budget.consume_work((used - previous.get()) as u64, "plain source mapping materialization").map_err(accounting_error)?;
        }
        leases
            .try_borrow_mut()
            .map_err(|_| IoError::Unsupported("plain source accounting already borrowed".into()))?
            .resize_retained(used as u64, "plain source mapping")
            .map_err(accounting_error)?;
        previous.set(used);
        Ok(())
    };
    let maximum = options.limits.resident_bytes.min(
        usize::try_from(budget.limits().get(photocraft_xcf::BudgetKind::Resident))
            .map_err(|_| IoError::Unsupported("plain source resident address space".into()))?,
    );
    let (mut result, _) = map_accounted(name, file, control, maximum, Some(&account), Some((budget, &leases)))?;
    provenance::stamp(&mut result, source, decoded.encoded_bytes, control, budget, previous.get(), &account)?;
    Ok(result)
}
