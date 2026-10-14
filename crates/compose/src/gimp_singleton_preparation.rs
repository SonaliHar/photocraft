//! Existing qualified Exposure preparation, shared by import validation and canvas rendering.
//! Caller-held source/model backing must remain admitted while the prepared renderer lives.
use crate::{
    Buffer,
    gimp_composite::{CompositionError, Result},
    gimp_render::{LeasedPreparedRenderer, PreparationBudget, PreparedRenderer},
};
use photocraft_cms::WideWork;
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
use std::sync::{Arc, Mutex};

pub fn supports(doc: &Document) -> bool {
    PreparedRenderer::supports_plain_point_budgeted(doc)
}
pub struct PreparedSingleton<'a, 'b, B: PreparationBudget, S> {
    prepared: LeasedPreparedRenderer<'a, S, B::Lease>,
    budget: &'b mut B,
    _header: B::Lease,
}
struct ControlledBudget<'b, 'c, 'i, B> {
    budget: &'b mut B,
    control: &'c Interrupt<'i>,
}
impl<B: PreparationBudget> PreparationBudget for ControlledBudget<'_, '_, '_, B> {
    type Lease = B::Lease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<Self::Lease> {
        self.control.check().map_err(|_| RasterInputError::Cancelled)?;
        self.budget.reserve_prepared_bytes(n)
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
        self.control.check().map_err(|_| RasterInputError::Cancelled)?;
        self.budget.consume_preparation_work(n)
    }
}
/// Select only the already qualified singleton gate; a selected failure never falls back.
pub fn prepare<'a, 'b, B: PreparationBudget, S>(
    doc: &'a Document,
    control: &Interrupt<'_>,
    budget: &'b mut B,
    source: S,
) -> Result<PreparedSingleton<'a, 'b, B, S>> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    if !supports(doc) {
        return Err(CompositionError::Unsupported("qualified singleton Exposure preparation"));
    }
    let header = budget.reserve_prepared_bytes(std::mem::size_of::<PreparedSingleton<'a, 'b, B, S>>() as u64)?;
    let prepared = PreparedRenderer::new_plain_point_budgeted(doc, control, &mut ControlledBudget { budget: &mut *budget, control }, source)?;
    Ok(PreparedSingleton { prepared, budget, _header: header })
}
/// Output allocation and its real reservation transfer together; raw renderer equations unchanged.
pub struct SingletonBuffer<L> {
    buffer: Buffer,
    _lease: Option<L>,
    _header: L,
}
impl<L> SingletonBuffer<L> {
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }
    pub fn into_parts(self) -> (Buffer, Option<L>, L) {
        (self.buffer, self._lease, self._header)
    }
}
impl<B: PreparationBudget, S> PreparedSingleton<'_, '_, B, S> {
    pub fn render_reduced(&mut self, w: u32, h: u32, damage: Option<Rect>, control: &Interrupt<'_>) -> Result<SingletonBuffer<B::Lease>> {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let header = self.budget.reserve_prepared_bytes(std::mem::size_of::<SingletonBuffer<B::Lease>>() as u64)?;
        let mut lease = None;
        let mut declared = 0u64;
        let mut upper = 0u64;
        let mut events = |event| -> Result<()> {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
            match event {
                WideWork::SampleUnits(n) => self.budget.consume_preparation_work(n),
                WideWork::ReserveBytes(0) | WideWork::ReleaseBytes(0) => Ok(()),
                WideWork::ReserveBytes(n) => {
                    if lease.is_some() {
                        return Err(CompositionError::Unsupported("singleton additional render workspace"));
                    }
                    declared = n;
                    upper = n.checked_mul(2).ok_or(CompositionError::Invalid("singleton output envelope"))?;
                    lease = Some(self.budget.reserve_prepared_bytes(upper)?);
                    Ok(())
                }
                WideWork::ReleaseBytes(n) => {
                    if n != declared || lease.is_none() {
                        return Err(CompositionError::Invalid("singleton output release"));
                    }
                    lease = None;
                    declared = 0;
                    upper = 0;
                    Ok(())
                }
            }
        };
        let buffer = self.prepared.renderer().render_reduced_with_interrupt(w, h, damage, &mut events, control)?;
        let actual = (buffer.px.capacity() as u64).checked_mul(16).ok_or(CompositionError::Invalid("singleton actual output capacity"))?;
        if !buffer.px.is_empty() && (lease.is_none() || actual > upper) {
            return Err(CompositionError::Invalid("singleton output exceeded admitted capacity"));
        }
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(SingletonBuffer { buffer, _lease: lease, _header: header })
    }
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: 512 << 20, work_units: 36_000_000 * 512 }
    }
}
struct State {
    live: u64,
    work: u64,
    limits: Limits,
}
pub struct Budget {
    state: Arc<Mutex<State>>,
}
pub struct Lease {
    state: Arc<Mutex<State>>,
    bytes: u64,
}
impl Budget {
    pub fn new(limits: Limits) -> Result<Self> {
        let max = Limits::default();
        if limits.resident_bytes > max.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || limits.work_units > max.work_units {
            return Err(RasterInputError::LimitExceeded.into());
        }
        Ok(Self { state: Arc::new(Mutex::new(State { live: 0, work: 0, limits })) })
    }
    pub fn reserve(&self, bytes: u64, ctl: &Interrupt<'_>) -> Result<Lease> {
        ctl.check().map_err(|_| RasterInputError::Cancelled)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let next = state.live.checked_add(bytes).ok_or(RasterInputError::LimitExceeded)?;
        if next > state.limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded.into());
        }
        state.live = next;
        Ok(Lease { state: Arc::clone(&self.state), bytes })
    }
    pub fn work(&self, n: u64, ctl: &Interrupt<'_>) -> Result<()> {
        ctl.check().map_err(|_| RasterInputError::Cancelled)?;
        self.consume(n)
    }
    fn consume(&self, n: u64) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let next = state.work.checked_add(n).ok_or(RasterInputError::LimitExceeded)?;
        if next > state.limits.work_units {
            return Err(RasterInputError::LimitExceeded.into());
        }
        state.work = next;
        Ok(())
    }
}
impl Lease {
    /// Release only completed census scratch/headroom; actual retained source stays admitted.
    pub fn retain_bytes(&mut self, bytes: u64) -> Result<()> {
        if bytes > self.bytes {
            return Err(RasterInputError::LimitExceeded.into());
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.live = state.live.checked_sub(self.bytes - bytes).ok_or(CompositionError::Invalid("singleton lease accounting"))?;
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.live = state.live.saturating_sub(self.bytes);
    }
}
impl PreparationBudget for Budget {
    type Lease = Lease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<Lease> {
        self.reserve(n, &Interrupt::NONE)
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
        self.consume(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_document_never_becomes_a_qualified_singleton_and_zero_quota_refuses() {
        let doc = Document::new("Own ordinary", photocraft_geom::Size::new(32, 32), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut budget = Budget::new(Limits::default()).unwrap();
        assert!(!supports(&doc));
        assert!(prepare(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
        let zero = Budget::new(Limits { resident_bytes: 0, work_units: 0 }).unwrap();
        assert!(zero.reserve(1, &Interrupt::NONE).is_err());
        assert!(zero.work(1, &Interrupt::NONE).is_err());
    }
    #[test]
    fn retained_source_and_output_share_one_lower_ledger_without_work_refund() {
        let budget = Budget::new(Limits { resident_bytes: 100, work_units: 10 }).unwrap();
        let mut source = budget.reserve(80, &Interrupt::NONE).unwrap();
        assert!(budget.reserve(21, &Interrupt::NONE).is_err());
        source.retain_bytes(60).unwrap();
        let output = budget.reserve(40, &Interrupt::NONE).unwrap();
        assert!(budget.reserve(1, &Interrupt::NONE).is_err());
        budget.work(10, &Interrupt::NONE).unwrap();
        drop(output);
        assert!(budget.work(1, &Interrupt::NONE).is_err());
        assert!(budget.reserve(1, &Interrupt::cancel_only(&|| true)).is_err());
    }
}
