//! Paid original -> nonlinear CURRENT relation for the assigned neighborhood lane.
//! This private owner supplies no independent source or storage eligibility.
use crate::{backed_normal::BackedNormalError as E, backed_stock_point_context::PlanBudget, gimp_composite::CompositionError};
use photocraft_cms::{
    ColorSpace, Profile,
    control::{Error as CmsError, Limits},
    xcf_color::{self, Encoding},
};
use photocraft_doc::{ColorMode, Document, SampleEncoding, WorkingProfileInterpretation};
use photocraft_raster::{Interrupt, backing::BackingError};
fn scope() -> E {
    E::Unsupported("assigned neighborhood original/current ICC relation")
}
fn size() -> E {
    E::Backing(BackingError::Length)
}
fn cms(e: CmsError) -> E {
    match e {
        CmsError::Cancelled => E::Backing(BackingError::Cancelled),
        CmsError::Allocation => E::Allocation,
        CmsError::Limit(_) => E::Backing(BackingError::Quota),
        CmsError::Cms(e) => E::Color(CompositionError::Color(e)),
    }
}
struct Work<'a, 'i, B> {
    budget: &'a mut B,
    interrupt: &'a Interrupt<'i>,
    used: u64,
}
impl<B: PlanBudget> Work<'_, '_, B> {
    fn charge(&mut self, n: u64) -> Result<(), E> {
        let next = self.used.checked_add(n).ok_or_else(size)?;
        if next > Limits::default().work_units {
            return Err(E::Backing(BackingError::Work));
        }
        self.budget.check(n)?;
        self.interrupt.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
        self.used = next;
        Ok(())
    }
    fn remaining(&self, held: usize) -> Result<Limits, E> {
        let max = Limits::default();
        Ok(Limits {
            resident_bytes: max.resident_bytes.checked_sub(held).ok_or(E::Backing(BackingError::Quota))?,
            work_units: max.work_units.checked_sub(self.used).ok_or(E::Backing(BackingError::Work))?,
        })
    }
    fn gate(&mut self, bytes: &[u8]) -> Result<(), E> {
        self.charge(128)?;
        let cancel = || self.interrupt.cancelled();
        let req = xcf_color::encoded_profile_gate_requirements_with_control(bytes, self.remaining(0)?, &cancel).map_err(cms)?;
        self.charge(req.work_units)?;
        let cancel = || self.interrupt.cancelled();
        xcf_color::validate_encoded_profile_with_control(bytes, Limits { resident_bytes: req.retained_bytes, work_units: req.work_units }, &cancel).map_err(cms)
    }
}
/// Each profile payload drops before the prospective fee that pays its peak.
struct PaidProfile<L> {
    profile: Profile,
    peak: usize,
    _lease: L,
}
pub(super) struct BoundAssignedNeighborhoodProfile<L> {
    original: PaidProfile<L>,
    current: PaidProfile<L>,
    _holder: L,
}
impl<L> BoundAssignedNeighborhoodProfile<L> {
    pub(super) fn profile(&self) -> &Profile {
        &self.current.profile
    }
    pub(super) fn prepare<B: PlanBudget<Lease = L>>(doc: &Document, interrupt: &Interrupt<'_>, budget: &mut B) -> Result<Self, E> {
        if doc.mode != ColorMode::Rgb
            || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
            || doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
        {
            return Err(scope());
        }
        let source = doc.source_color_context.as_ref().and_then(|s| s.original_icc.as_deref()).ok_or_else(scope)?;
        let current = doc.icc_profile.as_deref().ok_or_else(scope)?;
        if !(132..=65536).contains(&source.len()) || !(132..=65536).contains(&current.len()) {
            return Err(scope());
        }
        let mut w = Work { budget, interrupt, used: 0 };
        w.charge(1)?;
        let holder = w.budget.reserve(std::mem::size_of::<Self>() as u64)?;
        w.gate(source)?;
        w.gate(current)?;
        let planning = Profile::parse_planning_work_ceiling(source.len()).map_err(cms)?;
        w.charge(planning)?;
        let cancel = || interrupt.cancelled();
        let req = Profile::parse_requirements_with_control(source, Limits { work_units: planning, ..w.remaining(0)? }, &cancel).map_err(cms)?;
        let fee = w.budget.reserve(u64::try_from(req.peak_resident_bytes).map_err(|_| size())?)?;
        w.charge(req.work_units.checked_mul(2).ok_or_else(size)?)?;
        let original = PaidProfile {
            profile: Profile::parse_with_control(source, Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units }, &cancel)
                .map_err(cms)?,
            peak: req.peak_resident_bytes,
            _lease: fee,
        };
        if original.profile.color_space != ColorSpace::Rgb {
            return Err(scope());
        }
        let semantic = xcf_color::assigned_effective_planning_work_ceiling(&original.profile, Encoding::NonlinearProfile).map_err(cms)?;
        w.charge(semantic)?;
        xcf_color::qualify_effective_analytic_profile(&original.profile, Encoding::NonlinearProfile).map_err(|e| E::Color(CompositionError::Color(e)))?;
        let planning = xcf_color::assigned_effective_planning_work_ceiling(&original.profile, Encoding::NonlinearProfile).map_err(cms)?;
        w.charge(planning)?;
        let req = xcf_color::effective_profile_requirements_with_control(&original.profile, Encoding::NonlinearProfile, w.remaining(original.peak)?, &cancel)
            .map_err(cms)?;
        if req.peak_resident_bytes > w.remaining(original.peak)?.resident_bytes {
            return Err(E::Backing(BackingError::Quota));
        }
        let fee = w.budget.reserve(u64::try_from(req.peak_resident_bytes).map_err(|_| size())?)?;
        w.charge(req.work_units.checked_mul(4).ok_or_else(size)?)?;
        let current_model = PaidProfile {
            profile: xcf_color::effective_profile_from_definition_with_control(
                &original.profile,
                Encoding::NonlinearProfile,
                Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
                &cancel,
            )
            .map_err(cms)?,
            peak: req.peak_resident_bytes,
            _lease: fee,
        };
        let semantic = xcf_color::assigned_effective_planning_work_ceiling(&current_model.profile, Encoding::NonlinearProfile).map_err(cms)?;
        w.charge(semantic)?;
        xcf_color::qualify_effective_analytic_profile(&current_model.profile, Encoding::NonlinearProfile).map_err(|e| E::Color(CompositionError::Color(e)))?;
        let encoded = current_model.profile.encoded_bytes().ok_or_else(scope)?;
        w.charge(1)?;
        if encoded.len() != current.len() {
            return Err(scope());
        }
        for (a, b) in encoded.chunks(4096).zip(current.chunks(4096)) {
            w.charge((a.len() as u64).checked_mul(2).ok_or_else(size)?)?;
            if a != b {
                return Err(scope());
            }
        }
        w.charge(0)?;
        Ok(Self { original, current: current_model, _holder: holder })
    }
    pub(super) fn held_peak(&self) -> Result<usize, E> {
        self.original.peak.checked_add(self.current.peak).ok_or_else(size)
    }
}
