//! Root-scoped paid pure-point math. Whole-document membership and profile
//! authority are supplied by the sealed filtered-flat compiler, never a cloned
//! singleton Document. Existing singleton constructors keep their own gates.
use super::*;
use photocraft_doc::xcf_filters::FilterStack;

pub(crate) struct RootPointPlan {
    ordered: OrderedStack,
    format: PixelFormat,
}
impl RootPointPlan {
    pub(crate) fn prepare<B: PlanBudget>(
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
            return Ok((Self { ordered, format }, lease));
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
            return Ok((Self { ordered, format }, lease));
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
        Ok((Self { ordered, format }, lease))
    }
    pub(crate) fn evaluate(&self, quota: &BackingQuota, raw: &[u8], position: (i32, i32), control: &ReadControl<'_>) -> Result<[f64; 4], BackedNormalError> {
        evaluate_bound_point(&self.ordered, self.format, true, false, false, quota, raw, self.format, position, control)
    }
}
