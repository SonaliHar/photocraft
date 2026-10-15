//! Paid assigned-original -> exact effective-current ICC relation.
//! This owner grants no source, geometry, editing or renderer qualification.
//! Consumers retain it while borrowing its generated effective definition;
//! reparsing the encoded current bytes can lose analytical transfer constants.
use crate::{backed_normal::BackedNormalError as Error, gimp_composite::CompositionError};
use photocraft_cms::{
    ColorSpace, Profile,
    control::{Error as CmsError, Limits as CmsLimits},
    xcf_color::{self, Encoding},
};
use photocraft_doc::{ColorMode, SampleEncoding, WorkingProfileInterpretation, accounted_document::AccountedDocument};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{QuotaKind, QuotaLease},
};

#[derive(Clone, Copy, Debug)]
pub struct AssignedCurrentProfileLimits {
    pub max_encoded_bytes: usize,
    pub cms: CmsLimits,
}
impl Default for AssignedCurrentProfileLimits {
    fn default() -> Self {
        Self { max_encoded_bytes: 65536, cms: CmsLimits::default() }
    }
}
/// Both parsed/derived allocations drop before their actual same-ledger fees.
/// The source is the immutable model that supplied these original/current roles;
/// changing even only its title requires a fresh binding to the new model.
pub struct PreparedAssignedCurrentProfile {
    _original: Profile,
    effective: Profile,
    source: AccountedDocument,
    encoding: SampleEncoding,
    _original_lease: QuotaLease,
    _effective_lease: QuotaLease,
    _holder_lease: QuotaLease,
}
fn fail() -> Error {
    Error::Unsupported("bounded assigned original/current ICC relation")
}
fn cms(error: CmsError) -> Error {
    match error {
        CmsError::Cancelled => Error::Backing(BackingError::Cancelled),
        CmsError::Allocation => Error::Allocation,
        CmsError::Limit(_) => Error::Backing(BackingError::Quota),
        CmsError::Cms(error) => Error::Color(CompositionError::Color(error)),
    }
}
struct Work<'a, 'i> {
    control: &'a ReadControl<'i>,
    limits: CmsLimits,
    used: u64,
}
impl Work<'_, '_> {
    fn charge(&mut self, units: u64) -> Result<(), Error> {
        let total = self.used.checked_add(units).ok_or(Error::Backing(BackingError::Length))?;
        if total > self.limits.work_units {
            return Err(Error::Backing(BackingError::Work));
        }
        self.control.check(units).map_err(Error::Backing)?;
        self.used = total;
        Ok(())
    }
    fn remaining(&self, resident: usize) -> Result<CmsLimits, Error> {
        Ok(CmsLimits {
            resident_bytes: self.limits.resident_bytes.checked_sub(resident).ok_or(Error::Backing(BackingError::Quota))?,
            work_units: self.limits.work_units.checked_sub(self.used).ok_or(Error::Backing(BackingError::Work))?,
        })
    }
}
fn strict_gate(bytes: &[u8], work: &mut Work<'_, '_>) -> Result<(), Error> {
    work.charge(128)?;
    let cancelled = || work.control.interrupt.cancelled();
    let gate = xcf_color::encoded_profile_gate_requirements_with_control(bytes, work.remaining(0)?, &cancelled).map_err(cms)?;
    work.charge(gate.work_units)?;
    let cancelled = || work.control.interrupt.cancelled();
    xcf_color::validate_encoded_profile_with_control(bytes, CmsLimits { resident_bytes: gate.retained_bytes, work_units: gate.work_units }, &cancelled)
        .map_err(cms)
}
struct ParsedOriginal {
    profile: Profile,
    peak: usize,
    _fee: QuotaLease,
}
fn parse_original(bytes: &[u8], owner: &AccountedDocument, work: &mut Work<'_, '_>) -> Result<ParsedOriginal, Error> {
    let planning = Profile::parse_planning_work_ceiling(bytes.len()).map_err(cms)?;
    work.charge(planning)?;
    let cancelled = || work.control.interrupt.cancelled();
    let required = Profile::parse_requirements_with_control(bytes, CmsLimits { work_units: planning, ..work.remaining(0)? }, &cancelled).map_err(cms)?;
    let peak = u64::try_from(required.peak_resident_bytes).map_err(|_| Error::Backing(BackingError::Length))?;
    let fee = owner.quota().reserve(QuotaKind::Resident, peak).map_err(Error::Quota)?;
    work.charge(required.work_units.checked_mul(2).ok_or(Error::Backing(BackingError::Length))?)?;
    let cancelled = || work.control.interrupt.cancelled();
    let profile = Profile::parse_with_control(bytes, CmsLimits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units }, &cancelled)
        .map_err(cms)?;
    Ok(ParsedOriginal { profile, peak: required.peak_resident_bytes, _fee: fee })
}
#[derive(Clone, Copy)]
enum Admission {
    Geometry,
    IsolatedNormalGroup,
}
impl PreparedAssignedCurrentProfile {
    /// Exact current relation over genuine accounted storage. This method does
    /// not infer an original ICC from current bytes or substitute a builtin.
    pub fn prepare(owner: &AccountedDocument, limits: AssignedCurrentProfileLimits, control: &ReadControl<'_>) -> Result<Self, Error> {
        Self::prepare_admitted(owner, limits, control, Admission::Geometry)
    }
    /// Exact same original/effective relation, separately admitted by the closed
    /// editable isolated-Normal-group storage gate. No group projection is used.
    pub fn prepare_isolated_normal_group(owner: &AccountedDocument, limits: AssignedCurrentProfileLimits, control: &ReadControl<'_>) -> Result<Self, Error> {
        Self::prepare_admitted(owner, limits, control, Admission::IsolatedNormalGroup)
    }
    fn prepare_admitted(
        owner: &AccountedDocument,
        limits: AssignedCurrentProfileLimits,
        control: &ReadControl<'_>,
        admission: Admission,
    ) -> Result<Self, Error> {
        let maximum = AssignedCurrentProfileLimits::default();
        if limits.max_encoded_bytes > maximum.max_encoded_bytes
            || limits.max_encoded_bytes < 132
            || limits.cms.resident_bytes > maximum.cms.resident_bytes
            || limits.cms.work_units > maximum.cms.work_units
        {
            return Err(fail());
        }
        let mut work = Work { control, limits: limits.cms, used: 0 };
        work.charge(1)?;
        match admission {
            Admission::Geometry => owner.validate_assigned_analytic_geometry_storage(control),
            Admission::IsolatedNormalGroup => owner.validate_assigned_isolated_normal_group_storage(control),
        }
        .map_err(Error::Backing)?;
        let holder_fee = owner.quota().reserve(QuotaKind::Index, std::mem::size_of::<Self>() as u64).map_err(Error::Quota)?;
        let doc = owner.document();
        if doc.working_profile_interpretation != WorkingProfileInterpretation::Icc {
            return Err(fail());
        }
        let expected = match doc.mode {
            ColorMode::Rgb => ColorSpace::Rgb,
            ColorMode::Grayscale => ColorSpace::Gray,
            _ => return Err(fail()),
        };
        let encoding = doc.sample_encoding.ok_or_else(fail)?;
        let transfer = match encoding {
            SampleEncoding::Linear => Encoding::Linear,
            SampleEncoding::NonlinearProfile => Encoding::NonlinearProfile,
            SampleEncoding::Perceptual => Encoding::Perceptual,
        };
        let original = doc.source_color_context.as_ref().and_then(|c| c.original_icc.as_deref()).ok_or_else(fail)?;
        let current = doc.icc_profile.as_deref().ok_or_else(fail)?;
        if !(132..=limits.max_encoded_bytes).contains(&original.len()) || !(132..=limits.max_encoded_bytes).contains(&current.len()) {
            return Err(fail());
        }
        strict_gate(original, &mut work)?;
        strict_gate(current, &mut work)?;
        let original = parse_original(original, owner, &mut work)?;
        if original.profile.color_space != expected {
            return Err(fail());
        }
        // This closed analytic qualifier has constant-size RGB/Gray curves and
        // refuses LUTs/tables before the effective-transfer planning path.
        let original_validation = xcf_color::assigned_effective_planning_work_ceiling(&original.profile, Encoding::NonlinearProfile).map_err(cms)?;
        work.charge(original_validation)?;
        xcf_color::qualify_effective_analytic_profile(&original.profile, Encoding::NonlinearProfile).map_err(|e| Error::Color(CompositionError::Color(e)))?;
        let planning = xcf_color::assigned_effective_planning_work_ceiling(&original.profile, transfer).map_err(cms)?;
        work.charge(planning)?;
        let cancelled = || control.interrupt.cancelled();
        // Future admission is independent of the already consumed planning
        // charge; the cumulative local and caller work counters are not reset.
        let required =
            xcf_color::effective_profile_requirements_with_control(&original.profile, transfer, work.remaining(original.peak)?, &cancelled).map_err(cms)?;
        if required.peak_resident_bytes > work.remaining(original.peak)?.resident_bytes {
            return Err(Error::Backing(BackingError::Quota));
        }
        let peak = u64::try_from(required.peak_resident_bytes).map_err(|_| Error::Backing(BackingError::Length))?;
        let effective_fee = owner.quota().reserve(QuotaKind::Resident, peak).map_err(Error::Quota)?;
        // The controlled factory repeats requirements, clone and writer passes.
        // Each bounded schedule is prepaid before any derived profile allocation.
        work.charge(required.work_units.checked_mul(4).ok_or(Error::Backing(BackingError::Length))?)?;
        let effective = xcf_color::effective_profile_from_definition_with_control(
            &original.profile,
            transfer,
            CmsLimits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units },
            &cancelled,
        )
        .map_err(cms)?;
        let effective_validation = xcf_color::assigned_effective_planning_work_ceiling(&effective, transfer).map_err(cms)?;
        work.charge(effective_validation)?;
        xcf_color::qualify_effective_analytic_profile(&effective, transfer).map_err(|e| Error::Color(CompositionError::Color(e)))?;
        let encoded = effective.encoded_bytes().ok_or_else(fail)?;
        work.charge(1)?;
        if encoded.len() != current.len() {
            return Err(fail());
        }
        for (a, b) in encoded.chunks(4096).zip(current.chunks(4096)) {
            work.charge(u64::try_from(a.len()).ok().and_then(|n| n.checked_mul(2)).ok_or(Error::Backing(BackingError::Length))?)?;
            if a != b {
                return Err(fail());
            }
        }
        let prepared = Self {
            _original: original.profile,
            effective,
            source: owner.clone(),
            encoding,
            _original_lease: original._fee,
            _effective_lease: effective_fee,
            _holder_lease: holder_fee,
        };
        work.charge(1)?;
        Ok(prepared)
    }
    /// Borrow the generated definition while retaining this owner. This is not
    /// permission to detach a raw cloned Profile from its accounted lifecycle.
    pub fn profile(&self) -> &Profile {
        &self.effective
    }
    pub fn encoding(&self) -> SampleEncoding {
        self.encoding
    }
    /// Constant-time sealed witness for the exact immutable current model and
    /// actual ledger; no arbitrary caller assertion or copied statistics suffice.
    pub fn matches(&self, owner: &AccountedDocument, encoding: SampleEncoding) -> bool {
        self.source.shares_model(owner) && self.source.quota().shares_ledger(owner.quota()) && self.encoding == encoding
    }
}
#[cfg(test)]
#[path = "backed_assigned_current_profile_tests.rs"]
mod tests;
