//! Scoped aggregate accounting for loaded ordinary Normal documents, not process RSS.
use super::{LeasedPreparedRenderer, PreparationBudget, PreparedRenderer, WideBuffer};
use crate::gimp_composite::{CompositionError, Result};
use photocraft_cms::WideWork;
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use std::{cell::Cell, rc::Rc};
#[derive(Clone, Copy, Debug)]
pub struct SceneLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
struct State {
    limits: SceneLimits,
    resident: Cell<u64>,
    peak: Cell<u64>,
    work: Cell<u64>,
}
#[derive(Clone)]
pub struct SceneBudget {
    state: Rc<State>,
}
pub struct SceneLease {
    state: Rc<State>,
    bytes: u64,
}
impl Drop for SceneLease {
    fn drop(&mut self) {
        self.state.resident.set(self.state.resident.get().saturating_sub(self.bytes));
    }
}
fn invalid(message: &'static str) -> CompositionError {
    CompositionError::Invalid(message)
}
impl SceneBudget {
    pub fn new(limits: SceneLimits) -> Self {
        Self { state: Rc::new(State { limits, resident: Cell::new(0), peak: Cell::new(0), work: Cell::new(0) }) }
    }
    pub fn resident_bytes(&self) -> u64 {
        self.state.resident.get()
    }
    pub fn peak_bytes(&self) -> u64 {
        self.state.peak.get()
    }
    pub fn work_units(&self) -> u64 {
        self.state.work.get()
    }
    /// Reserves an external destination/codec allocation on this same ledger.
    /// Reserve before allocation; retain the lease until that allocation drops
    /// or transfers to a separately accounted persistent owner. Existing source
    /// and prepared leases are not charged again.
    pub fn reserve_external_bytes(&self, bytes: u64) -> Result<SceneLease> {
        self.reserve(bytes)
    }
    fn reserve(&self, bytes: u64) -> Result<SceneLease> {
        self.charge(bytes)?;
        Ok(SceneLease { state: self.state.clone(), bytes })
    }
    fn charge(&self, bytes: u64) -> Result<()> {
        let next = self.resident_bytes().checked_add(bytes).ok_or_else(|| invalid("scene resident overflow"))?;
        if next > self.state.limits.resident_bytes {
            return Err(invalid("scene aggregate resident limit"));
        }
        self.state.resident.set(next);
        self.state.peak.set(self.peak_bytes().max(next));
        Ok(())
    }
    fn work(&self, units: u64) -> Result<()> {
        let next = self.work_units().checked_add(units).ok_or_else(|| invalid("scene work overflow"))?;
        if next > self.state.limits.work_units {
            return Err(invalid("scene aggregate work limit"));
        }
        self.state.work.set(next);
        Ok(())
    }
    fn event(&self, event: WideWork) -> Result<()> {
        match event {
            WideWork::ReserveBytes(bytes) => self.charge(bytes),
            WideWork::ReleaseBytes(bytes) => {
                self.state.resident.set(self.resident_bytes().checked_sub(bytes).ok_or_else(|| invalid("scene release exceeds charge"))?);
                Ok(())
            }
            WideWork::SampleUnits(units) => self.work(units),
        }
    }
}
impl PreparationBudget for SceneBudget {
    type Lease = SceneLease;
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<Self::Lease> {
        self.reserve(bytes)
    }
    fn consume_preparation_work(&mut self, units: u64) -> Result<()> {
        self.work(units)
    }
}
fn add(total: &mut u64, bytes: usize) -> Result<()> {
    *total = total.checked_add(u64::try_from(bytes).map_err(|_| invalid("scene census size"))?).ok_or_else(|| invalid("scene census overflow"))?;
    Ok(())
}
fn capacity<T>(total: &mut u64, values: &Vec<T>) -> Result<()> {
    add(total, values.capacity().checked_mul(std::mem::size_of::<T>()).ok_or_else(|| invalid("scene capacity overflow"))?)
}
/// Payload/capacity census for the supported two-raster ordinary domain. Shared
/// Arcs are conservatively charged per owner. Tile-map bookkeeping is bounded
/// conservatively; this is not a universal Document allocator/RSS measurement.
pub fn source_bytes(doc: &Document, control: &Interrupt<'_>) -> Result<u64> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("missing ordinary state"))?;
    if !PreparedRenderer::requires_preparation_budget(doc)
        || doc.layers.len() != 2
        || state.items.len() != 3
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.selection.is_some()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || !doc.patterns.is_empty()
        || doc.color_table.is_some()
        || doc.duotone.is_some()
        || !doc.layer_comps.is_empty()
        || doc.last_document_state.is_some()
        || !doc.notes.is_empty()
        || doc.timeline.is_some()
        || doc.measurement != Default::default()
        || doc.text_styles != Default::default()
        || doc.slices != Default::default()
        || doc.variables != Default::default()
        || !state.resources.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.item_sets.is_empty()
        || !doc.metadata.psd_resources.is_empty()
        || !doc.metadata.psd_global_blocks.is_empty()
    {
        return Err(invalid("ordinary source census unsupported retained collection"));
    }
    control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
    let mut total = std::mem::size_of::<Document>() as u64;
    add(&mut total, doc.name.capacity())?;
    capacity(&mut total, &doc.layers)?;
    if let Some(bytes) = &doc.icc_profile {
        add(&mut total, bytes.capacity())?;
    }
    if let Some(bytes) = doc.source_color_context.as_ref().and_then(|v| v.original_icc.as_ref()) {
        add(&mut total, bytes.capacity())?;
    }
    if let Some(xmp) = &doc.metadata.xmp {
        add(&mut total, xmp.capacity())?;
    }
    if let Some(exif) = &doc.metadata.exif {
        add(&mut total, exif.capacity())?;
    }
    capacity(&mut total, &doc.guides.horizontal)?;
    capacity(&mut total, &doc.guides.vertical)?;
    for layer in &doc.layers {
        control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !layer.psd_blocks.is_empty() || layer.fill_cache.is_some() || layer.indexed_source.is_some() || layer.video.is_some() {
            return Err(invalid("ordinary source census additional layer payload"));
        }
        add(&mut total, layer.name.capacity())?;
        let LayerContent::Raster(surface) = &layer.content else { return Err(invalid("ordinary source census raster required")) };
        for surface in std::iter::once(surface).chain(layer.mask.as_ref().map(|m| m.surface.as_ref())) {
            surface.try_validate_tile_origins_with(control)?;
            add(&mut total, surface.default_bytes().len())?;
            for (index, (_, tile)) in surface.tiles().enumerate() {
                if index.is_multiple_of(64) {
                    control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
                }
                add(&mut total, tile.bytes().len())?;
                add(&mut total, 128)?;
            }
        }
    }
    add(&mut total, std::mem::size_of_val(state))?;
    capacity(&mut total, &state.items)?;
    capacity(&mut total, &state.opaque)?;
    capacity(&mut total, &state.selection.selected)?;
    capacity(&mut total, &state.selection.selected_paths)?;
    for item in &state.items {
        add(&mut total, item.name.capacity())?;
        if let Some(policy) = &item.composition {
            add(&mut total, policy.compatibility_epoch.capacity())?;
        }
    }
    let p = &state.provenance;
    for text in [&p.format, &p.container, &p.source_label] {
        add(&mut total, text.capacity())?;
    }
    for text in [&p.producer, &p.release_id, &p.source_hash, &p.original_locator].into_iter().flatten() {
        add(&mut total, text.capacity())?;
    }
    capacity(&mut total, &p.protected_source_fingerprints)?;
    for text in &p.protected_source_fingerprints {
        add(&mut total, text.capacity())?;
    }
    for record in &state.opaque {
        control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        add(&mut total, record.bytes.capacity())?;
        for text in [&record.namespace, &record.identity, &record.blake3] {
            add(&mut total, text.capacity())?;
        }
    }
    control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
    Ok(total)
}
/// Returned buffers retain their output charge, even after preparation is dropped.
pub struct SceneBuffer<T> {
    buffer: T,
    _lease: SceneLease,
}
impl<T> SceneBuffer<T> {
    pub fn buffer(&self) -> &T {
        &self.buffer
    }
    /// Transfers the allocation to an already accounted persistent output owner.
    /// The scene charge is released; the caller must retain its replacement
    /// reservation before this call. This does not clone the pixel payload.
    pub fn into_buffer(self) -> T {
        self.buffer
    }
    /// Transfers both payload and its live reservation without any accounting gap.
    pub fn into_parts(self) -> (T, SceneLease) {
        (self.buffer, self._lease)
    }
}
pub struct ScenePrepared<'a> {
    prepared: LeasedPreparedRenderer<'a, SceneLease, SceneLease>,
    budget: SceneBudget,
}
pub fn prepare<'a>(doc: &'a Document, control: &Interrupt<'_>, limits: SceneLimits) -> Result<ScenePrepared<'a>> {
    let mut budget = SceneBudget::new(limits);
    let bytes = source_bytes(doc, control)?;
    let source = budget.reserve(bytes)?;
    budget.work(bytes)?;
    let prepared = PreparedRenderer::new_budgeted(doc, control, &mut budget, source)?;
    Ok(ScenePrepared { prepared, budget })
}

impl ScenePrepared<'_> {
    pub fn budget(&self) -> &SceneBudget {
        &self.budget
    }
    fn evaluate<T>(
        &self,
        observer: &mut impl FnMut(WideWork) -> Result<()>,
        run: impl FnOnce(&mut dyn FnMut(WideWork) -> Result<()>) -> Result<T>,
    ) -> Result<SceneBuffer<T>> {
        // Track this evaluation's events, rather than a global resident delta:
        // an observer may release an older output or retain a nested output.
        let mut retained = 0_u64;
        let mut callback = |event| {
            let next = match event {
                WideWork::ReserveBytes(bytes) => retained.checked_add(bytes).ok_or_else(|| invalid("scene output charge overflow"))?,
                WideWork::ReleaseBytes(bytes) => retained.checked_sub(bytes).ok_or_else(|| invalid("scene output release exceeds its reservation"))?,
                WideWork::SampleUnits(_) => retained,
            };
            self.budget.event(event)?;
            retained = next;
            observer(event)
        };
        let result = run(&mut callback);
        let lease = SceneLease { state: self.budget.state.clone(), bytes: retained };
        match result {
            Ok(buffer) => Ok(SceneBuffer { buffer, _lease: lease }),
            Err(error) => {
                drop(lease);
                Err(error)
            }
        }
    }
    pub fn render(&self, rect: Rect, control: &Interrupt<'_>, observer: &mut impl FnMut(WideWork) -> Result<()>) -> Result<SceneBuffer<WideBuffer>> {
        self.evaluate(observer, |callback| self.prepared.renderer().render_with_interrupt(rect, &mut |event| callback(event), control))
    }
    pub fn render_reduced(
        &self,
        w: u32,
        h: u32,
        damage: Option<Rect>,
        control: &Interrupt<'_>,
        observer: &mut impl FnMut(WideWork) -> Result<()>,
    ) -> Result<SceneBuffer<crate::Buffer>> {
        self.evaluate(observer, |callback| self.prepared.renderer().render_reduced_with_interrupt(w, h, damage, &mut |event| callback(event), control))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_source_preparation_and_output_leases_refund_independently() {
        let budget = SceneBudget::new(SceneLimits { resident_bytes: 100, work_units: 10 });
        let source = budget.reserve(40).unwrap();
        let prepared = budget.reserve(30).unwrap();
        assert!(budget.reserve(31).is_err());
        assert_eq!(budget.resident_bytes(), 70);
        assert!(budget.reserve_external_bytes(31).is_err());
        assert_eq!(budget.resident_bytes(), 70);
        let output = budget.reserve_external_bytes(30).unwrap();
        assert_eq!(budget.peak_bytes(), 100);
        drop(prepared);
        drop(source);
        assert_eq!(budget.resident_bytes(), 30, "returned output remains charged after preparation");
        drop(output);
        assert_eq!(budget.resident_bytes(), 0);
        budget.work(7).unwrap();
        assert!(budget.work(4).is_err());
        assert_eq!(budget.work_units(), 7, "denial does not charge failed work");
        assert!(budget.event(WideWork::ReleaseBytes(1)).is_err());
    }
    #[test]
    fn observer_dropping_previous_output_cannot_change_new_output_charge() {
        let mut doc = Document::new("ledger", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32);
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let budget = SceneBudget::new(SceneLimits { resident_bytes: 100, work_units: 100 });
        let renderer = PreparedRenderer::new(&doc).unwrap();
        let prepared = LeasedPreparedRenderer { renderer, _source_lease: budget.reserve(0).unwrap(), _prepared_lease: budget.reserve(0).unwrap() };
        let scene = ScenePrepared { prepared, budget: budget.clone() };
        let mut previous = Some(budget.reserve(20).unwrap());
        let output = scene
            .evaluate(
                &mut |_| {
                    drop(previous.take());
                    Ok(())
                },
                |callback| {
                    callback(WideWork::ReserveBytes(30))?;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(budget.resident_bytes(), 30);
        drop(output);
        assert_eq!(budget.resident_bytes(), 0, "the new output lease must own all30 bytes, regardless of unrelated release");
        let mut previous = Some(budget.reserve(20).unwrap());
        let denied = scene.evaluate(
            &mut |_| {
                drop(previous.take());
                Err(invalid("observer denial"))
            },
            |callback| {
                callback(WideWork::ReserveBytes(30))?;
                Ok(())
            },
        );
        assert!(denied.is_err());
        assert_eq!(budget.resident_bytes(), 0, "failure must not restore an unrelated released reservation");
        let mut nested = None;
        let output = scene
            .evaluate(
                &mut |_| {
                    nested = Some(scene.evaluate(&mut |_| Ok(()), |callback| {
                        callback(WideWork::ReserveBytes(10))?;
                        Ok(())
                    })?);
                    Ok(())
                },
                |callback| {
                    callback(WideWork::ReserveBytes(30))?;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(budget.resident_bytes(), 40);
        drop(output);
        assert_eq!(budget.resident_bytes(), 10, "outer output owns only its own events");
        drop(nested);
        assert_eq!(budget.resident_bytes(), 0);
    }
}
