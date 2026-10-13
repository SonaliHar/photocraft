//! Current qualification uses a real renderer and one bounded preparation ledger.
//! Known retained payload/model capacity is charged during this immutable borrow;
//! this is not a census of allocator overhead or every legacy private collection.
use crate::{
    IoError,
    xcf_assessment::{ImportAssessment, assess_prepared_xcf_document},
};
use photocraft_compose::{
    gimp_composite::CompositionError,
    gimp_render::{PreparationBudget, PreparedRenderer},
    preview_budget::{PreparationLease, PreviewPreparationBudget},
};
use photocraft_doc::Document;
use photocraft_format::{FormatError, PreparationLimits};
use photocraft_raster::{Interrupt, RasterInputError};
use std::cell::Cell;
fn limit(name: &str) -> IoError {
    FormatError::LimitExceeded(name.into()).into()
}
fn composition(error: CompositionError) -> IoError {
    match error {
        CompositionError::Raster(RasterInputError::Cancelled) => IoError::Cancelled,
        CompositionError::Raster(RasterInputError::LimitExceeded) => limit("current XCF assessment resident bytes"),
        other => IoError::Unsupported(format!("current XCF assessment: {other}")),
    }
}
fn raster(error: RasterInputError) -> IoError {
    composition(CompositionError::Raster(error))
}
struct Adapter<'a, 'b, 'i> {
    budget: &'a PreviewPreparationBudget<'b>,
    control: &'a Interrupt<'i>,
    work: Cell<u64>,
}
impl<'a, 'b> PreparationBudget for Adapter<'a, 'b, '_> {
    type Lease = PreparationLease<'a, 'b>;
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<Self::Lease, CompositionError> {
        let bytes = usize::try_from(bytes).map_err(|_| CompositionError::Raster(RasterInputError::LimitExceeded))?;
        self.budget.reserve_preparation_bytes(bytes, self.control).map_err(CompositionError::Raster)
    }
    fn consume_preparation_work(&mut self, units: u64) -> Result<(), CompositionError> {
        self.control.check().map_err(|_| CompositionError::Raster(RasterInputError::Cancelled))?;
        let next = self.work.get().checked_add(units).ok_or(CompositionError::Raster(RasterInputError::LimitExceeded))?;
        if next > 1 << 36 {
            return Err(CompositionError::Raster(RasterInputError::LimitExceeded));
        }
        self.work.set(next);
        Ok(())
    }
}
pub(crate) fn assess(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<Option<ImportAssessment>, IoError> {
    let observer = |_| {};
    let budget = PreviewPreparationBudget::new(limits.resident_bytes, &observer);
    assess_with_budget(doc, limits, control, &budget)
}
fn assess_with_budget(
    doc: &Document,
    limits: PreparationLimits,
    control: &Interrupt<'_>,
    budget: &PreviewPreparationBudget<'_>,
) -> Result<Option<ImportAssessment>, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    let ceiling = PreparationLimits::default();
    if limits.resident_bytes > ceiling.resident_bytes
        || limits.index_bytes > ceiling.index_bytes
        || limits.manifest_bytes > ceiling.manifest_bytes
        || limits.object_bytes > ceiling.object_bytes
    {
        return Err(limit("current XCF assessment platform ceiling"));
    }
    if doc.xcf_state.as_ref().is_none_or(|state| state.provenance.format != "xcf") {
        return Ok(None);
    }
    // Reject backed storage before source census or any pixel read. Its source
    // and Session lifecycle require the separate accounted native owner.
    if doc.has_backed_rasters().map_err(raster)? {
        return Err(IoError::Unsupported("current backed XCF assessment requires accounted Session ownership".into()));
    }
    let retained = {
        // Existing census has sequential, input-bounded identity HashSets;
        // prepay its complete caller-selected index ceiling before allocation.
        let _index = budget.reserve_preparation_bytes(limits.index_bytes, control).map_err(raster)?;
        crate::native_save::retained_native_input_bytes(doc, limits, control)?
    };
    let source = budget.reserve_preparation_bytes(retained, control).map_err(raster)?;
    let output_bound = crate::xcf_assessment::allocation_bound(doc)?;
    let _output = budget.reserve_preparation_bytes(output_bound, control).map_err(raster)?;
    let mut adapter = Adapter { budget, control, work: Cell::new(0) };
    // These predicates select accounting routes only. Their constructors still
    // qualify all current fields/samples and produce the immutable proof.
    let prepared = if PreparedRenderer::supports_plain_normal_budgeted(doc) {
        PreparedRenderer::new_plain_normal_budgeted(doc, control, &mut adapter, (doc, source))
    } else if PreparedRenderer::supports_plain_point_budgeted(doc) {
        PreparedRenderer::new_plain_point_budgeted(doc, control, &mut adapter, (doc, source))
    } else if PreparedRenderer::requires_preparation_budget(doc) {
        PreparedRenderer::new_budgeted(doc, control, &mut adapter, (doc, source))
    } else if PreparedRenderer::supports_accounted_scalar_preparation(doc) {
        PreparedRenderer::new_accounted(doc, control, &mut adapter, (doc, source))
    } else {
        return Err(IoError::Unsupported("current XCF scene has no accounted qualification route".into()));
    }
    .map_err(composition)?;
    let report = assess_prepared_xcf_document(prepared.renderer(), control)?;
    if let Some(report) = &report
        && output_bytes(report).is_none_or(|bytes| bytes > output_bound)
    {
        return Err(limit("current XCF assessment output capacity"));
    }
    control.check().map_err(|_| IoError::Cancelled)?;
    // The public result transfers bounded report accounting to its caller,
    // as import results do. Source/prepared leases cover report generation.
    Ok(report)
}
fn output_bytes(report: &ImportAssessment) -> Option<usize> {
    let mut bytes = std::mem::size_of::<ImportAssessment>().checked_add(report.container.capacity())?;
    bytes = bytes.checked_add(report.dependencies.capacity().checked_mul(std::mem::size_of::<crate::xcf_assessment::DependencyAssessment>())?)?;
    bytes = bytes.checked_add(report.diagnostics.capacity().checked_mul(std::mem::size_of::<crate::xcf_assessment::SuccessDiagnostic>())?)?;
    for row in &report.dependencies {
        bytes = bytes.checked_add(row.resource_id.capacity())?;
    }
    for row in &report.diagnostics {
        bytes = bytes.checked_add(row.resource_id.capacity())?.checked_add(row.recovery_actions.capacity().checked_mul(std::mem::size_of::<&str>())?)?;
    }
    Some(bytes)
}
#[cfg(test)]
use crate::xcf_fixture as fixture;
#[cfg(test)]
mod tests {
    use super::*;
    fn current() -> crate::ImportResult {
        let options = super::fixture::FixtureOptions {
            layer_properties: vec![
                (7, 28_i32.to_be_bytes().to_vec()),
                (35, 0_i32.to_be_bytes().to_vec()),
                (36, 0_i32.to_be_bytes().to_vec()),
                (37, 0_i32.to_be_bytes().to_vec()),
            ],
            ..Default::default()
        };
        let imported = crate::import("current.xcf", &super::fixture::image_custom(12, 150, 0, 1, 1, &[40, 60, 80, 255], &options)).unwrap();
        assert!(PreparedRenderer::supports_plain_normal_budgeted(&imported.document));
        imported
    }
    #[test]
    fn limits_and_cancellation_refund_without_mutating_real_source() {
        use photocraft_compose::preview_budget::PreparationCharge;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let imported = current();
        let before = imported.document.clone();
        let limits = PreparationLimits { resident_bytes: 8 << 20, index_bytes: 1 << 20, ..PreparationLimits::default() };
        let budget = PreviewPreparationBudget::new(limits.resident_bytes, &|_| {});
        let result = assess_with_budget(&imported.document, limits, &Interrupt::NONE, &budget).unwrap();
        assert_eq!(result, imported.assessment);
        assert_eq!(budget.usage().live_bytes, 0);
        assert!(budget.usage().peak_bytes <= limits.resident_bytes);
        for phase in 1..=4 {
            let stop = AtomicBool::new(false);
            let count = AtomicUsize::new(0);
            let observer = |charge| {
                if matches!(charge, PreparationCharge::Reserve(_)) && count.fetch_add(1, Ordering::Relaxed) + 1 == phase {
                    stop.store(true, Ordering::Relaxed);
                }
            };
            let budget = PreviewPreparationBudget::new(limits.resident_bytes, &observer);
            let cancelled = || stop.load(Ordering::Relaxed);
            assert!(
                matches!(assess_with_budget(&imported.document, limits, &Interrupt::cancel_only(&cancelled), &budget), Err(IoError::Cancelled)),
                "phase {phase}"
            );
            assert_eq!(budget.usage().live_bytes, 0);
        }
        let budget = PreviewPreparationBudget::new(128, &|_| {});
        assert!(matches!(assess_with_budget(&imported.document, limits, &Interrupt::NONE, &budget), Err(IoError::Pcraft(FormatError::LimitExceeded(_)))));
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(imported.document, before);
    }
    #[test]
    fn source_and_report_must_coexist_on_one_aggregate_ledger() {
        let imported = current();
        let limits = PreparationLimits { resident_bytes: 8 << 20, index_bytes: 16 << 10, ..PreparationLimits::default() };
        let retained = crate::native_save::retained_native_input_bytes(&imported.document, limits, &Interrupt::NONE).unwrap();
        let bound = crate::xcf_assessment::allocation_bound(&imported.document).unwrap();
        let cap = retained.checked_add(bound).unwrap() - 1;
        assert!(cap >= limits.index_bytes && cap >= retained);
        let budget = PreviewPreparationBudget::new(cap, &|_| {});
        assert!(matches!(assess_with_budget(&imported.document, limits, &Interrupt::NONE, &budget), Err(IoError::Pcraft(FormatError::LimitExceeded(_)))));
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(budget.usage().peak_bytes, limits.index_bytes.max(retained));
    }
    #[test]
    fn platform_ceiling_rejects_each_raised_limit_before_preparation() {
        let imported = current();
        let ceiling = PreparationLimits::default();
        let mut cases = [ceiling; 4];
        cases[0].resident_bytes += 1;
        cases[1].index_bytes += 1;
        cases[2].manifest_bytes += 1;
        cases[3].object_bytes += 1;
        for limits in cases {
            let budget = PreviewPreparationBudget::new(ceiling.resident_bytes, &|_| {});
            assert!(matches!(assess_with_budget(&imported.document, limits, &Interrupt::NONE, &budget), Err(IoError::Pcraft(FormatError::LimitExceeded(_)))));
            assert_eq!(budget.usage().peak_bytes, 0);
        }
    }
    #[test]
    fn actual_ordinary_source_current_assessment_equals_import_report() {
        let imported = current();
        let before = imported.document.clone();
        let actual = assess(&imported.document, PreparationLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(actual, imported.assessment);
        assert_eq!(imported.document, before);
    }
    #[test]
    fn caller_one_byte_budget_cannot_report_success() {
        let bytes = super::fixture::image(12, 150, 0, 1, 1, &[40, 60, 80, 255]);
        let imported = crate::import("current.xcf", &bytes).unwrap();
        let limits = PreparationLimits { resident_bytes: 1, index_bytes: 0, ..PreparationLimits::default() };
        assert!(assess(&imported.document, limits, &Interrupt::NONE).is_err());
    }
    #[test]
    fn legacy_minimal_packet_has_no_accounted_fallback() {
        let bytes = super::fixture::image(12, 150, 0, 1, 1, &[40, 60, 80, 255]);
        let imported = crate::import("legacy.xcf", &bytes).unwrap();
        assert!(matches!(assess(&imported.document, PreparationLimits::default(), &Interrupt::NONE), Err(IoError::Unsupported(_))));
        assert_eq!(crate::xcf_assessment::assess_xcf_document(&imported.document, &Interrupt::NONE).unwrap(), imported.assessment);
    }
    #[test]
    #[ignore = "requires verified sibling XCF corpus; run explicitly"]
    fn frozen_medium_ordinary_scene_current_query_matches_actual_import() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../photocraft-corpus/xcf/full/normal-mask-ordinary-scene-evidence/medium/rgb-f64-op2.xcf");
        let bytes = std::fs::read(path).unwrap();
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(hash, "7a73598e168ddc7c899f6cd988688aaa0e22a43afb08ec008a2669b07e6e546b");
        let imported = crate::import("current.xcf", &bytes).unwrap();
        assert!(photocraft_compose::gimp_render::PreparedRenderer::requires_preparation_budget(&imported.document));
        let before = imported.document.clone();
        assert_eq!(assess(&imported.document, PreparationLimits::default(), &Interrupt::NONE).unwrap(), imported.assessment);
        assert_eq!(imported.document, before);
    }
}
