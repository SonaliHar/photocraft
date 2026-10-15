//! Logical work for the unchanged precise forward RGB -> encoded Lab evaluator.
//! These are bounded source-level schedule counts, not CPU cycles. Scalar
//! arithmetic/casts/comparisons/min/max/clamp/descriptor queries each count one; explicit array
//! initialization/load/store/copy counts one moved word. Indexed coefficients
//! and curve samples count as table accesses. Loop/dispatch overhead is priced
//! as one stage/curve dispatch in each recipe, not compiler iterator machinery.
//! powf/cbrt count provider invocations only: their
//! implementation interiors remain a bounded platform-provider boundary.
use super::{Curve, MAX, Pipeline, Stage};
use crate::control::{Budget, Error, Result, invalid};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExactF32Work {
    pub scalar_ops: u64,
    pub data_moves: u64,
    pub table_accesses: u64,
    pub powf_calls: u64,
    pub cbrt_calls: u64,
}
impl ExactF32Work {
    pub(crate) const ZERO: Self = Self { scalar_ops: 0, data_moves: 0, table_accesses: 0, powf_calls: 0, cbrt_calls: 0 };
    pub fn checked_add(self, other: Self) -> Result<Self> {
        let add = |a: u64, b: u64| a.checked_add(b).ok_or(Error::Limit("exact F32 work overflow"));
        Ok(Self {
            scalar_ops: add(self.scalar_ops, other.scalar_ops)?,
            data_moves: add(self.data_moves, other.data_moves)?,
            table_accesses: add(self.table_accesses, other.table_accesses)?,
            powf_calls: add(self.powf_calls, other.powf_calls)?,
            cbrt_calls: add(self.cbrt_calls, other.cbrt_calls)?,
        })
    }
    pub fn checked_mul(self, count: u64) -> Result<Self> {
        let mul = |n: u64| n.checked_mul(count).ok_or(Error::Limit("exact F32 work overflow"));
        Ok(Self {
            scalar_ops: mul(self.scalar_ops)?,
            data_moves: mul(self.data_moves)?,
            table_accesses: mul(self.table_accesses)?,
            powf_calls: mul(self.powf_calls)?,
            cbrt_calls: mul(self.cbrt_calls)?,
        })
    }
    /// One unit per listed logical primitive/provider invocation. This is not a
    /// calibrated replacement for the generic controlled evaluator's 128 costs.
    pub fn total_units(self) -> Result<u64> {
        [self.scalar_ops, self.data_moves, self.table_accesses, self.powf_calls, self.cbrt_calls]
            .into_iter()
            .try_fold(0u64, |n, v| n.checked_add(v).ok_or(Error::Limit("exact F32 work overflow")))
    }
}
fn work(scalar_ops: u64, data_moves: u64, table_accesses: u64, powf_calls: u64, cbrt_calls: u64) -> ExactF32Work {
    ExactF32Work { scalar_ops, data_moves, table_accesses, powf_calls, cbrt_calls }
}
fn curve_work(curve: &Curve) -> Result<ExactF32Work> {
    // Two sample moves are the enclosing Curves stage's read/write. A branch
    // recipe bounds each component independently, so mutually exclusive low
    // arithmetic and high pow paths may both be priced; no data prepass occurs.
    Ok(match curve {
        Curve::Identity => work(1, 2, 0, 0, 0),
        // eval/variant dispatch, two casts, identity/sign predicates and two
        // possible sign negations. Gamma==1 still takes powf for nonfinite x.
        Curve::Gamma(_) => work(2 + 2 + 4 + 2, 2 + 2, 0, 1, 0),
        Curve::Parametric { kind, .. } => {
            let (branches, arithmetic, coefficient_words) = match kind {
                0 => (4, 2, 7),
                1 => (4, 4, 7),
                2 => (4, 5, 7),
                // Negative pure-gamma mirror re-enters para once at most;
                // retain both seven-word destructures and all branch tests.
                3 => (8, 5, 14),
                4 => (2, 5, 7),
                _ => return Err(invalid("exact F32 parametric curve kind")),
            };
            // Curve::eval, eval64 and para each dispatch once; type3's
            // bounded recursive dispatch is included in its branch allowance.
            work(3 + 2 + branches + arithmetic, 2 + 1 + coefficient_words, 0, 1, 0)
        }
        Curve::Table(values) => match values.len() {
            0 => work(2 + 1, 2, 0, 0, 0),
            1 => work(2 + 1, 2, 1, 0, 0),
            // Two distinct neighbors, three indexed occurrences in table().
            // len dispatch, clamp, n offsets/casts, index/min, interpolation.
            _ => work(2 + 1 + 3 + 3 + 3 + 3 + 1, 2, 3, 0, 0),
        },
        Curve::Table64(values) => match values.len() {
            0 => work(2 + 2 + 1, 2, 0, 0, 0),
            1 => work(2 + 2 + 2 + 1 + 1, 2, 1, 0, 0),
            // table64 first/read guards, finite check, clamp/index/casts and
            // interpolation; first + two neighboring values, never a scan.
            _ => work(4 + (1 + 1 + 2) + (1 + 1 + 1 + 1) + (1 + 1 + 1) + 1 + 2 + 3 + (1 + 1) + 3 + 3, 2, 3, 0, 0),
        },
    })
}

pub(crate) struct RgbLabPlan {
    stages: [ExactF32Work; 4],
    count: usize,
    pixel: ExactF32Work,
}
impl RgbLabPlan {
    pub(crate) fn pixel_work(&self) -> ExactF32Work {
        self.pixel
    }
}
// Fixed metadata ceiling: eight Transform/header fields plus two bounded passes
// (shape/cost, actual-retention) over at most four eight-field stage descriptors
// and three eight-word curve descriptors. No table samples or matrix coefficient
// values are traversed during this planning.
pub(crate) const PREPARATION_WORK: u64 = 8 + 2 * (4 * 8 + 3 * 8);
pub(crate) const SCRATCH_BYTES: usize = std::mem::size_of::<[f32; MAX]>() * 3 + std::mem::size_of::<[f32; 3]>() * 4;

pub(crate) fn prepare_rgb_lab(pipe: &Pipeline, budget: &Budget<'_>) -> Result<RgbLabPlan> {
    budget.poll()?;
    if pipe.inputs != 3 || pipe.outputs != 3 || !(2..=4).contains(&pipe.stages.len()) {
        return Err(invalid("exact F32 RGB Lab pipeline shape"));
    }
    let mut stages = [ExactF32Work::ZERO; 4];
    let mut cursor = 0usize;
    if let Some(Stage::Curves(curves)) = pipe.stages.first() {
        if curves.len() != 3 {
            return Err(invalid("exact F32 RGB curve count"));
        }
        let mut cost = work(1, 0, 0, 0, 0); // stage dispatch
        for curve in curves {
            budget.poll()?;
            cost = cost.checked_add(curve_work(curve)?)?;
        }
        *stages.get_mut(cursor).ok_or(Error::Limit("exact F32 stage index"))? = cost;
        cursor += 1;
    }
    if let Some(Stage::Matrix { rows, cols, m, offset }) = pipe.stages.get(cursor) {
        if *rows != 3 || *cols != 3 || m.len() != 9 || offset.len() != 3 {
            return Err(invalid("exact F32 matrix shape"));
        }
        // Nine ordered multiply/add pairs, nine input casts, three output casts,
        // nine two-operation coefficient indices; sixteen initialized temporary
        // words, nine input reads, three row stores, six final copy moves.
        *stages.get_mut(cursor).ok_or(Error::Limit("exact F32 stage index"))? = work(1 + 9 * 2 + 9 + 3 + 9 * 2, MAX as u64 + 9 + 3 + 6, 9 + 3, 0, 0);
        cursor += 1;
    }
    if !matches!(pipe.stages.get(cursor), Some(Stage::XyzToLab))
        || !matches!(pipe.stages.get(cursor + 1), Some(Stage::EncodeLabV4))
        || cursor + 2 != pipe.stages.len()
    {
        return Err(invalid("exact F32 RGB Lab stage order"));
    }
    // Stage max(0), white divisions, branch tests; component-wise bound prices
    // the low arithmetic and high cbrt alternatives. Six output arithmetic ops.
    *stages.get_mut(cursor).ok_or(Error::Limit("exact F32 stage index"))? = work(1 + 3 + 3 + 3 + 3 * 3 + 6, 3 + 3 + 3 + 3 + 6, 3, 0, 3);
    *stages.get_mut(cursor + 1).ok_or(Error::Limit("exact F32 stage index"))? = work(1 + 3 + 2 + 3, 3 + 3, 0, 0, 0);
    // Original Pipeline::eval zero initialization, three input and three output
    // read/write pairs. Per-stage numeric ordering is not changed.
    let mut pixel = work(1, MAX as u64 + 6 + 6, 0, 0, 0);
    for stage in stages.get(..pipe.stages.len()).ok_or(Error::Limit("exact F32 stage count"))? {
        pixel = pixel.checked_add(*stage)?;
    }
    budget.poll()?;
    Ok(RgbLabPlan { stages, count: pipe.stages.len(), pixel })
}

pub(crate) fn eval_rgb_lab(pipe: &Pipeline, plan: &RgbLabPlan, input: &[f32; 3], output: &mut [f32; 3], budget: &mut Budget<'_>) -> Result<()> {
    // The plan is held privately beside an immutable borrow of this pipeline.
    // No intermediate finite check is added: Stage::run is exactly the legacy
    // evaluator, including max/clamp behavior after pathological intermediates.
    budget.work(work(1, MAX as u64 + 6 + 6, 0, 0, 0).total_units()?)?;
    let mut values = [0f32; MAX];
    values.get_mut(..3).ok_or(Error::Limit("exact F32 input"))?.copy_from_slice(input);
    let mut n = 3;
    for (stage, cost) in pipe.stages.iter().zip(plan.stages.get(..plan.count).ok_or(Error::Limit("exact F32 stage count"))?) {
        budget.work(cost.total_units()?)?;
        n = stage.run(&mut values, n);
    }
    budget.poll()?;
    output.copy_from_slice(values.get(..3).ok_or(Error::Limit("exact F32 output"))?);
    Ok(())
}
