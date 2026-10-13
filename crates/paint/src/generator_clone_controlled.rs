//! Controlled raw state cloning of the upstream incremental generator.
//! Only allocation-bearing fields are copied through the shared budget. All
//! walker/smoothing/time/jitter indices are retained, without replay or reset.
use super::*;
use crate::render::incremental_clone_controlled::{CloneBudget, IncrementalCloneRequirements, brush_requirements, copy_brush, fixed, vector_requirement};
use crate::render::{StrokeError, StrokeLimits};
use photocraft_raster::Interrupt;
type Result<T> = std::result::Result<T, StrokeError>;
impl DabGenerator {
    pub(crate) fn incremental_clone_requirements(&self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
        control.check().map_err(|_| StrokeError::Cancelled)?;
        let mut req = fixed::<Self>();
        req.add(brush_requirements(&self.builder.brush, limits, control)?, limits)?;
        if let Some(dual) = &self.dual { req.add(brush_requirements(&dual.brush, limits, control)?, limits)?; }
        req.add(vector_requirement(&self.scratch)?, limits)?;
        Ok(req)
    }
    pub(crate) fn clone_with_budget(&self, budget: &mut CloneBudget<'_,'_>) -> Result<Self> {
        budget.charge(std::mem::size_of::<Self>() as u64)?;
        let builder = DabBuilder { brush: copy_brush(&self.builder.brush, budget)?, seed: self.builder.seed,
            stroke_color: self.builder.stroke_color, next_index: self.builder.next_index };
        let dual = self.dual.as_ref().map(|dual| -> Result<DualBuilder> { Ok(DualBuilder { brush: copy_brush(&dual.brush, budget)?, next_index: dual.next_index }) }).transpose()?;
        let scratch = budget.copy(&self.scratch)?;
        budget.poll()?;
        Ok(Self { smoother: self.smoother.clone(), walker: self.walker.clone(), dual_walker: self.dual_walker.clone(), builder, dual, scratch })
    }
}
