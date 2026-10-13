//! Preparation parity over existing frozen, tiny reference documents. The
//! conservative source envelope covers retained loaded test state; this is not
//! a generic source census or a native-save/process-memory benchmark.
use photocraft_compose::{
    gimp_composite::{CompositionError, Result},
    gimp_render::{PreparationBudget, PreparedRenderer, WideBuffer},
};
use photocraft_doc::Document;
use photocraft_raster::Interrupt;
use std::{cell::Cell, rc::Rc};
struct Lease(Rc<Cell<u64>>, u64);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get() - self.1);
    }
}
struct Budget(Rc<Cell<u64>>);
impl PreparationBudget for Budget {
    type Lease = Lease;
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<Lease> {
        let next = self.0.get().checked_add(bytes).ok_or(CompositionError::Invalid("test reserve overflow"))?;
        if next > 64 << 20 {
            return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
        }
        self.0.set(next);
        Ok(Lease(self.0.clone(), bytes))
    }
    fn consume_preparation_work(&mut self, _: u64) -> Result<()> {
        Ok(())
    }
}
pub fn verify(doc: &Document, expected: &WideBuffer) {
    let before = doc.clone();
    let mut budget = Budget(Rc::new(Cell::new(0)));
    let source = budget.reserve_prepared_bytes(16 << 20).unwrap();
    let prepared = PreparedRenderer::new_accounted(doc, &Interrupt::NONE, &mut budget, source).unwrap();
    assert_eq!(&prepared.renderer().render(doc.bounds(), &mut |_| Ok(())).unwrap(), expected);
    assert_eq!(doc, &before);
    drop(prepared);
    assert_eq!(budget.0.get(), 0);
}
