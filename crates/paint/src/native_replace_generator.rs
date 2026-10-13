//! Replacement-only incremental generation: no coverage raster/history replay.
use super::*;
use incremental_clone_controlled::CloneBudget;
use photocraft_raster::Interrupt;
type Result<T>=std::result::Result<T,StrokeError>;
fn poll(c:&Interrupt<'_>)->Result<()> {c.check().map_err(|_|StrokeError::Cancelled)}
pub struct NativeReplacementGenerator {pub ctx:BrushContext,generator:DabGenerator,held:u64,base_held:u64,work:u64,points:usize,dabs:usize}
impl NativeReplacementGenerator {
    pub fn new(brush:&BrushSettings,limits:StrokeLimits,c:&Interrupt<'_>)->Result<Self>{
        let renderer=StrokeRenderer::try_new_with_interrupt(brush,None,1.0,limits,c)?;
        let req=renderer.incremental_clone_requirements(limits,c)?;poll(c)?;
        Ok(Self{ctx:renderer.ctx,generator:renderer.generator,held:req.retained_bytes,base_held:req.retained_bytes,work:req.work_units,points:0,dabs:0})
    }
    pub fn retained_bytes(&self)->u64{self.held}
    pub fn work_units(&self)->u64{self.work}
    pub fn try_clone_with_interrupt(&self,limits:StrokeLimits,c:&Interrupt<'_>)->Result<Self>{
        poll(c)?;let mut req=self.generator.incremental_clone_requirements(limits,c)?;
        req.add(incremental_clone_controlled::brush_requirements(&self.ctx.brush,limits,c)?,limits)?;
        req.retained_bytes=req.retained_bytes.checked_add(self.held).ok_or(StrokeError::Limit)?;
        let cloned_work=self.work.checked_add(req.work_units).ok_or(StrokeError::Limit)?;if cloned_work>limits.work_units{return Err(StrokeError::Limit);}
        let mut budget=CloneBudget::new(req,limits,c)?;
        let ctx=BrushContext{brush:incremental_clone_controlled::copy_brush(&self.ctx.brush,&mut budget)?,tip:self.ctx.tip.as_ref().map(Arc::clone),dual_tip:self.ctx.dual_tip.as_ref().map(Arc::clone),texture:self.ctx.texture.as_ref().map(Arc::clone)};
        let generator=self.generator.clone_with_budget(&mut budget)?;poll(c)?;
        Ok(Self{ctx,generator,held:self.held,base_held:self.base_held,work:cloned_work,points:self.points,dabs:self.dabs})
    }
    /// Private generator state and delta are published together after final poll.
    pub fn delta_with_interrupt(&mut self,points:&[StrokePoint],finish:bool,limits:StrokeLimits,c:&Interrupt<'_>)->Result<Vec<Dab>>{
        poll(c)?;let standard=StrokeLimits::default();if limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units||self.points.checked_add(points.len()).is_none_or(|n|n>100_000){return Err(StrokeError::Limit);}
        let mut primary=Vec::new();let mut dual=Vec::new();
        let req=if finish{self.generator.finish_requirements(&primary,&dual,limits,c)?}else{self.generator.push_requirements(points,&primary,&dual,limits,c)?};
        let clone_req=self.generator.incremental_clone_requirements(limits,c)?;
        let context_req=incremental_clone_controlled::brush_requirements(&self.ctx.brush,limits,c)?;
        let work=self.work.checked_add(req.work_units).and_then(|n|n.checked_add(clone_req.work_units)).and_then(|n|n.checked_add(context_req.work_units)).ok_or(StrokeError::Limit)?;
        if work>limits.work_units||self.held.checked_mul(2).and_then(|n|n.checked_add(req.retained_bytes)).and_then(|n|n.checked_add(req.staged_bytes)).is_none_or(|n|n>limits.resident_bytes){return Err(StrokeError::Limit);}
        let mut next=self.try_clone_with_interrupt(limits,c)?;
        if finish{next.generator.finish_with_interrupt(&mut primary,&mut dual,limits,c)?;}else{next.generator.push_with_interrupt(points,&mut primary,&mut dual,limits,c)?;}
        next.points=next.points.checked_add(points.len()).ok_or(StrokeError::Limit)?;next.dabs=next.dabs.checked_add(primary.len()).ok_or(StrokeError::Limit)?;if next.dabs>100_000{return Err(StrokeError::Limit);}
        let held=next.generator.incremental_clone_requirements(limits,c)?;
        next.held=self.base_held.checked_add(held.retained_bytes).ok_or(StrokeError::Limit)?; // conservative retained historical constructor plus actual generator
        if next.held>limits.resident_bytes{return Err(StrokeError::Limit);}next.work=work;poll(c)?;*self=next;Ok(primary)
    }
}
