//! Strict current analytic-geometry publication for a sealed paid XCF owner.
//! Original-source qualification belongs to the typed IO carrier; this stage
//! independently validates the current assigned profile and complete canvas.
use super::*;
use photocraft_cms::{
    Profile,
    control::{Error as CmsError, Limits as CmsLimits},
};
use photocraft_compose::{
    backed_document::{BackedDocumentSinkError, PreparedBackedDocument},
    backed_normal::BackedNormalError,
};
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_raster::backing::quota::{QuotaKind, QuotaLease};

// Every parsed profile allocation dies before the real same-ledger peak lease.
struct PaidCurrentProfile {
    profile: Profile,
    _lease: QuotaLease,
}
fn cms_error(error: CmsError) -> EngineError {
    match error {
        CmsError::Cancelled => EngineError::Cancelled,
        CmsError::Allocation => BackingError::Allocation.into(),
        other => EngineError::Other(format!("owned analytic geometry current profile: {other}")),
    }
}
fn render_error(error: BackedNormalError, control: &ReadControl<'_>) -> EngineError {
    use photocraft_raster::checked_view::RasterViewError;
    match error {
        BackedNormalError::Backing(e) | BackedNormalError::View(RasterViewError::Backed(e) | RasterViewError::Control(e) | RasterViewError::Descriptor(e)) => {
            e.into()
        }
        BackedNormalError::Quota(_) => BackingError::Quota.into(),
        BackedNormalError::Allocation => BackingError::Allocation.into(),
        _ if control.interrupt.cancelled() => EngineError::Cancelled,
        other => EngineError::Other(format!("owned analytic geometry current recipe: {other}")),
    }
}
fn current_profile(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<PaidCurrentProfile> {
    let bytes = owner.document().icc_profile.as_ref().ok_or(BackingError::Unsupported)?;
    // This is the assigned-ICC lane only. The genuine structural gate above
    // binds original/current bytes and its bounded profile/source policy.
    let cancelled = || control.interrupt.cancelled();
    // General ICC parsing deliberately tolerates some legacy encodings. This
    // current XCF lane first repeats its strict borrowed encoded gate, including
    // duplicate/overlap/intent/optional-tag checks, without allocating a profile.
    control.check(128)?;
    let gate = photocraft_cms::xcf_color::encoded_profile_gate_requirements_with_control(bytes, CmsLimits::default(), &cancelled).map_err(cms_error)?;
    control.check(gate.work_units)?;
    photocraft_cms::xcf_color::validate_encoded_profile_with_control(
        bytes,
        CmsLimits { resident_bytes: gate.retained_bytes, work_units: gate.work_units },
        &cancelled,
    )
    .map_err(cms_error)?;
    let planning = Profile::parse_planning_work_ceiling(bytes.len()).map_err(cms_error)?;
    control.check(planning)?;
    let required =
        Profile::parse_requirements_with_control(bytes, CmsLimits { work_units: planning, ..CmsLimits::default() }, &cancelled).map_err(cms_error)?;
    let peak = u64::try_from(required.peak_resident_bytes).map_err(|_| BackingError::Length)?;
    let lease = owner.quota().reserve(QuotaKind::Resident, peak).map_err(|_| BackingError::Quota)?;
    // parse_with_control repeats the bounded requirements walk and constructs
    // under its own actual-capacity limits. Charge both schedules to the same
    // ephemeral parent before either pass, retaining no caller callback.
    control.check(required.work_units.checked_mul(2).ok_or(BackingError::Length)?)?;
    let profile = Profile::parse_with_control(bytes, CmsLimits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units }, &cancelled)
        .map_err(cms_error)?;
    let paid = PaidCurrentProfile { profile, _lease: lease };
    control.check(1)?;
    Ok(paid)
}
/// Current storage/profile/render validation, never an original source grant.
/// The L1 gate retains all original float/integer geometry epochs and raw
/// tuples; the compositor independently binds the exact retained ICC definition.
pub(crate) fn validate_current_analytic_geometry_storage(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<()> {
    owner.validate_masked_analytic_normal_storage(control)?;
    let profile = current_profile(owner, control)?;
    let prepared = PreparedBackedDocument::new_accounted(owner, &profile.profile, Default::default(), control).map_err(|e| render_error(e, control))?;
    prepared
        .render_bands(owner.document().bounds(), control, |band| -> std::result::Result<(), BackedNormalError> {
            drop(band);
            Ok(())
        })
        .map_err(|e| match e {
            BackedDocumentSinkError::Render(e) | BackedDocumentSinkError::Sink(e) => render_error(e, control),
        })?;
    drop(prepared);
    drop(profile);
    control.check(1)?;
    Ok(())
}
impl Session {
    /// Explicit keep-current source handoff. Actual assigned ICC/native/mask
    /// validation finishes before the common paid history/view/tab publication.
    /// Source paths remain service-supplied exclusion identities, not read grants.
    pub fn add_owned_analytic_geometry_xcf_import(
        &mut self,
        imported: &photocraft_io::OwnedAnalyticGeometryXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        let owner = imported.document();
        validate_current_analytic_geometry_storage(owner, control)?;
        self.add_owned_xcf_model(
            ImportedModel { document: owner, origin: imported.origin(), source_read_only: imported.source_read_only(), preview_only: imported.preview_only() },
            label,
            source_path,
            control,
        )
    }
}
