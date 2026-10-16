//! Root-scoped paid pure-point math. Whole-document membership and profile
//! authority are supplied by the sealed filtered-flat compiler, never a cloned
//! singleton Document. Existing singleton constructors keep their own gates.
use super::*;
use photocraft_doc::xcf_filters::FilterStack;

enum StageKind {
    Exposure(crate::gimp_exposure::PreparedExposure),
    Semi(crate::gimp_semi_flatten::PreparedSemiFlatten),
    Hue(crate::gimp_hue_chroma::OwnedHueChroma),
    Vibrance(crate::gimp_vibrance::OwnedVibrance),
}
struct PreparedRootStage<L> {
    kind: StageKind,
    enabled: bool,
    _lease: L,
}
// Explicit ordered compatibility policy: ordinary roots are bottom-first,
// but retained raw/public filter nodes execute in reverse canonical order.
const ORDERED_NL_FLOAT_BRIDGE_WORK: u64 = 32;
const SEMI_PIXEL_SCRATCH_BYTES: u64 = (16 * std::mem::size_of::<[f64; 4]>()) as u64;
fn nl_float_bridge(pixel: [f64; 4], control: &ReadControl<'_>) -> Result<[f64; 4], BackedNormalError> {
    control.check(ORDERED_NL_FLOAT_BRIDGE_WORK).map_err(BackedNormalError::Backing)?;
    if pixel.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&pixel[3]) {
        return Err(scope());
    }
    let staged = pixel.map(|v| v as f32);
    if staged.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&staged[3]) {
        return Err(scope());
    }
    let result = staged.map(f64::from);
    control.check(0).map_err(BackedNormalError::Backing)?;
    Ok(result)
}
impl<L> PreparedRootStage<L> {
    fn evaluate(&self, pixel: [f64; 4], derived: bool, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<[f64; 4], BackedNormalError> {
        // Exposure owns its complete typed validation/application pricing and
        // same-q scratch. Dispatch before the legacy wrapper to avoid doubling it.
        if let StageKind::Exposure(candidate)=&self.kind {
            return candidate.evaluate(pixel,self.enabled,derived,quota,control);
        }
        control.check(128).map_err(BackedNormalError::Backing)?;
        if pixel.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&pixel[3]) {
            return Err(scope());
        }
        let (scratch, validation, evaluation) = match &self.kind {
            StageKind::Exposure(_) => return Err(scope()), // handled above
            StageKind::Semi(_) => (SEMI_PIXEL_SCRATCH_BYTES, 3 * 128 + 4, if self.enabled { 9 * 128 + 128 } else { 3 * 128 + 4 }),
            StageKind::Hue(candidate) => (crate::gimp_hue_chroma::PIXEL_SCRATCH_BYTES, candidate.validation_work(), candidate.evaluation_work(self.enabled)),
            StageKind::Vibrance(candidate) => (crate::gimp_vibrance::PIXEL_SCRATCH_BYTES, candidate.validation_work(), candidate.evaluation_work(self.enabled)),
        };
        control.check(validation).map_err(BackedNormalError::Backing)?;
        // Actual runtime scratch is independent of all conservatively retained
        // construction-peak leases. It spans both validation and application.
        let _scratch = quota.reserve(QuotaKind::Resident, scratch).map_err(BackedNormalError::Quota)?;
        let failed = std::cell::Cell::new(None);
        let work = |units| match control.check(units) {
            Ok(()) => Ok(()),
            Err(error) => {
                if failed.get().is_none() {
                    failed.set(Some(error));
                }
                Err(if error == photocraft_raster::backing::BackingError::Cancelled {
                    photocraft_raster::RasterInputError::Cancelled
                } else {
                    photocraft_raster::RasterInputError::LimitExceeded
                })
            }
        };
        let evaluated = (|| {
            match &self.kind {
                StageKind::Exposure(_) => return Err(scope()), // handled above
                StageKind::Semi(candidate) => {
                    if derived {
                        candidate.validate_derived_input(pixel)
                    } else {
                        candidate.validate_input(pixel)
                    }
                }
                StageKind::Hue(candidate) => {
                    if derived {
                        candidate.validate_derived_input(pixel, control.interrupt, &work)
                    } else {
                        candidate.validate_input(pixel, control.interrupt, &work)
                    }
                }
                StageKind::Vibrance(candidate) => {
                    if derived {
                        candidate.validate_derived_input(pixel, control.interrupt, &work)
                    } else {
                        candidate.validate_input(pixel, control.interrupt, &work)
                    }
                }
            }
            .map_err(BackedNormalError::Color)?;
            control.check(evaluation).map_err(BackedNormalError::Backing)?;
            match &self.kind {
                StageKind::Exposure(_) => return Err(scope()), // handled above
                StageKind::Semi(candidate) => {
                    if derived {
                        candidate.apply_derived(pixel, self.enabled)
                    } else {
                        candidate.apply(pixel, self.enabled)
                    }
                }
                StageKind::Hue(candidate) => {
                    if derived {
                        candidate.apply_derived(pixel, self.enabled, control.interrupt, &work)
                    } else {
                        candidate.apply(pixel, self.enabled, control.interrupt, &work)
                    }
                }
                StageKind::Vibrance(candidate) => {
                    if derived {
                        candidate.apply_derived(pixel, self.enabled, control.interrupt, &work)
                    } else {
                        candidate.apply(pixel, self.enabled, control.interrupt, &work)
                    }
                }
            }
            .map_err(BackedNormalError::Color)
        })();
        if let Some(error) = failed.get() {
            return Err(BackedNormalError::Backing(error));
        }
        let output = evaluated?;
        if output.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&output[3]) {
            return Err(scope());
        }
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(output)
    }
}
enum RootProgram<L> {
    Singleton(OrderedStack),
    Ordered(Vec<PreparedRootStage<L>>),
}
pub(crate) struct RootPointPlan<L> {
    program: RootProgram<L>,
    format: PixelFormat,
}
impl<L> RootPointPlan<L> {
    pub(crate) fn prepare<B: PlanBudget<Lease = L>>(
        stack: &FilterStack,
        format: PixelFormat,
        extent: Rect,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
    ) -> Result<(Self, L), BackedNormalError> {
        let sole_exposure=stack.nodes.len()==1 && stack.nodes.first().is_some_and(crate::gimp_exposure::claims);
        if stack.nodes.len() == 1 && !sole_exposure {
            return Self::prepare_singleton(stack, format, extent, interrupt, budget);
        }
        if stack.nodes.is_empty() || stack.nodes.len() > 1024 {
            return Err(scope());
        }
        let count = stack.nodes.len();
        let bytes = count
            .checked_mul(2)
            .and_then(|n| n.checked_mul(std::mem::size_of::<PreparedRootStage<L>>()))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .and_then(|n| n.checked_add(4 * std::mem::size_of::<[f64; 4]>() + 128))
            .ok_or_else(length)?;
        let lease = budget.reserve(u64::try_from(bytes).map_err(|_| length())?)?;
        let mut stages = Vec::new();
        stages.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
        if stages.capacity() > count.checked_mul(2).ok_or_else(length)? {
            return Err(BackedNormalError::Allocation);
        }
        for node in &stack.nodes {
            let basis = Builtin::Srgb.profile();
            let header = (std::mem::size_of::<PreparedRootStage<L>>() as u64).checked_add(65536).ok_or_else(length)?;
            // Each independently owned Parts peak stays leased while all earlier
            // stage payloads remain live. Field order protects failed unwinds.
            let prepared = if crate::gimp_exposure::claims(node) {
                work(budget,interrupt,512)?;
                let parameters=crate::gimp_exposure::canonical(node,SampleEncoding::NonlinearProfile,22,extent).map_err(BackedNormalError::Color)?;
                let (candidate,fee)=crate::gimp_exposure::PreparedExposure::prepare(parameters,std::mem::size_of::<PreparedRootStage<L>>()as u64,interrupt,budget)?;
                PreparedRootStage{kind:StageKind::Exposure(candidate),enabled:node.enabled,_lease:fee}
            } else if crate::gimp_hue_chroma::claims(node) {
                let parameters = crate::gimp_hue_chroma::canonical(node, SampleEncoding::NonlinearProfile, 22, extent).map_err(BackedNormalError::Color)?;
                let preparation = crate::gimp_hue_chroma::Preparation::plan(basis, interrupt, budget)?;
                let bytes = header
                    .checked_add(preparation.peak_bytes()?)
                    .and_then(|n| n.checked_add(crate::gimp_hue_chroma::PIXEL_SCRATCH_BYTES))
                    .ok_or_else(length)?;
                let fee = budget.reserve(bytes)?;
                let candidate = preparation.construct(basis, parameters, interrupt, budget)?;
                PreparedRootStage { kind: StageKind::Hue(candidate), enabled: node.enabled, _lease: fee }
            } else if crate::gimp_vibrance::claims(node) {
                let parameters = crate::gimp_vibrance::canonical(node, SampleEncoding::NonlinearProfile, 22, extent).map_err(BackedNormalError::Color)?;
                let preparation = crate::gimp_vibrance::Preparation::plan(basis, interrupt, budget)?;
                let bytes =
                    header.checked_add(preparation.peak_bytes()?).and_then(|n| n.checked_add(crate::gimp_vibrance::PIXEL_SCRATCH_BYTES)).ok_or_else(length)?;
                let fee = budget.reserve(bytes)?;
                let candidate = preparation.construct(basis, parameters, interrupt, budget)?;
                PreparedRootStage { kind: StageKind::Vibrance(candidate), enabled: node.enabled, _lease: fee }
            } else if crate::gimp_semi_flatten::claims(node) {
                let fee = budget.reserve(header)?;
                let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(cms_error)?;
                work(
                    budget,
                    interrupt,
                    semantic
                        .checked_mul(3)
                        .and_then(|n| n.checked_add(crate::gimp_semi_flatten::LITERAL_PREPARATION_WORK))
                        .and_then(|n| n.checked_add(header.checked_mul(64)?))
                        .ok_or_else(length)?,
                )?;
                let candidate = crate::gimp_semi_flatten::PreparedSemiFlatten::new(node, basis, SampleEncoding::NonlinearProfile, 22, extent, interrupt)
                    .map_err(BackedNormalError::Color)?;
                PreparedRootStage { kind: StageKind::Semi(candidate), enabled: node.enabled, _lease: fee }
            } else {
                return Err(scope());
            };
            stages.push(prepared);
        }
        Ok((Self { program: RootProgram::Ordered(stages), format }, lease))
    }
    fn prepare_singleton<B: PlanBudget<Lease = L>>(
        stack: &FilterStack,
        format: PixelFormat,
        extent: Rect,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
    ) -> Result<(Self, B::Lease), BackedNormalError> {
        let [node] = stack.nodes.as_slice() else { return Err(scope()) };
        let basis = Builtin::Srgb.profile();
        let stage = crate::gimp_neighborhood::scalar_preparation_bytes(1).map_err(BackedNormalError::Color)?.checked_mul(2).ok_or_else(length)?;
        let header = (std::mem::size_of::<Self>() as u64).checked_add(stage).and_then(|v| v.checked_add(65536)).ok_or_else(length)?;
        // All constructors own their actual Parts. The global current profile
        // is retained by the enclosing plan, not cloned into each root.
        if crate::gimp_hue_chroma::claims(node) {
            let parameters = crate::gimp_hue_chroma::canonical(node, SampleEncoding::NonlinearProfile, 22, extent).map_err(BackedNormalError::Color)?;
            let preparation = crate::gimp_hue_chroma::Preparation::plan(basis, interrupt, budget)?;
            let bytes = header
                .checked_add(preparation.peak_bytes()?)
                .and_then(|n| n.checked_add(crate::gimp_hue_chroma::PIXEL_SCRATCH_BYTES))
                .and_then(|n| n.checked_add(crate::gimp_hue_chroma::OwnedHueChroma::holder_peak_bytes().ok()?))
                .ok_or_else(length)?;
            let lease = budget.reserve(bytes)?;
            let candidate = preparation.construct(basis, parameters, interrupt, budget)?;
            let point =
                gimp_filters::PointStack::from_prepared_hue_chroma(candidate, node.enabled, format.sample, interrupt).map_err(BackedNormalError::Color)?;
            let ordered = OrderedStack::from_prepared_stock_point(point, extent, interrupt)
                .map_err(BackedNormalError::Color)?
                .with_current_point_storage(Some(format.sample));
            if ordered.stock_retained_bytes().map_err(BackedNormalError::Color)? > bytes {
                return Err(BackedNormalError::Allocation);
            }
            return Ok((Self { program: RootProgram::Singleton(ordered), format }, lease));
        }
        if crate::gimp_vibrance::claims(node) {
            let parameters = crate::gimp_vibrance::canonical(node, SampleEncoding::NonlinearProfile, 22, extent).map_err(BackedNormalError::Color)?;
            let preparation = crate::gimp_vibrance::Preparation::plan(basis, interrupt, budget)?;
            let bytes = header
                .checked_add(preparation.peak_bytes()?)
                .and_then(|n| n.checked_add(crate::gimp_vibrance::PIXEL_SCRATCH_BYTES))
                .and_then(|n| n.checked_add(crate::gimp_vibrance::OwnedVibrance::holder_peak_bytes().ok()?))
                .ok_or_else(length)?;
            let lease = budget.reserve(bytes)?;
            let candidate = preparation.construct(basis, parameters, interrupt, budget)?;
            let point =
                gimp_filters::PointStack::from_prepared_vibrance(candidate, node.enabled, format.sample, interrupt).map_err(BackedNormalError::Color)?;
            let ordered = OrderedStack::from_prepared_stock_point(point, extent, interrupt)
                .map_err(BackedNormalError::Color)?
                .with_current_point_storage(Some(format.sample));
            if ordered.stock_retained_bytes().map_err(BackedNormalError::Color)? > bytes {
                return Err(BackedNormalError::Allocation);
            }
            return Ok((Self { program: RootProgram::Singleton(ordered), format }, lease));
        }
        if !crate::gimp_semi_flatten::claims(node) {
            return Err(scope());
        }
        crate::gimp_semi_flatten::canonical(node, SampleEncoding::NonlinearProfile, 22, extent).map_err(BackedNormalError::Color)?;
        let lease = budget.reserve(header)?;
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(cms_error)?;
        work(
            budget,
            interrupt,
            semantic
                .checked_mul(3)
                .and_then(|v| v.checked_add(crate::gimp_semi_flatten::LITERAL_PREPARATION_WORK))
                .and_then(|v| v.checked_add(header.checked_mul(64)?))
                .ok_or_else(length)?,
        )?;
        let ordered = OrderedStack::new_for_stock(stack, basis, SampleEncoding::NonlinearProfile, 22, extent, interrupt)
            .map_err(BackedNormalError::Color)?
            .with_current_point_storage(Some(format.sample));
        if ordered.stock_retained_bytes().map_err(BackedNormalError::Color)? > header {
            return Err(BackedNormalError::Allocation);
        }
        Ok((Self { program: RootProgram::Singleton(ordered), format }, lease))
    }
    pub(crate) fn evaluate(&self, quota: &BackingQuota, raw: &[u8], position: (i32, i32), control: &ReadControl<'_>) -> Result<[f64; 4], BackedNormalError> {
        match &self.program {
            RootProgram::Singleton(ordered) => evaluate_bound_point(ordered, self.format, true, false, false, quota, raw, self.format, position, control),
            RootProgram::Ordered(stages) => {
                control.check(self.format.bytes_per_pixel() as u64 + 1).map_err(BackedNormalError::Backing)?;
                if raw.len() != self.format.bytes_per_pixel() {
                    return Err(length());
                }
                let read = |channel| photocraft_color::try_read_sample_f64(raw, self.format.sample, channel).map_err(|_| length());
                let mut pixel = [read(0)?, read(1)?, read(2)?, read(3)?];
                if pixel.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                    return Err(scope());
                }
                let mut enabled = 0usize;
                for stage in stages.iter().rev() {
                    // Disabled nodes neither narrow nor cause a bridge. Their
                    // actual incoming value is still fully validated/applied.
                    if stage.enabled && enabled != 0 {
                        pixel = nl_float_bridge(pixel, control)?;
                    }
                    pixel = stage.evaluate(pixel, enabled != 0, quota, control)?;
                    if stage.enabled {
                        enabled = enabled.checked_add(1).ok_or_else(length)?;
                    }
                }
                control.check(0).map_err(BackedNormalError::Backing)?;
                Ok(pixel)
            }
        }
    }
}
