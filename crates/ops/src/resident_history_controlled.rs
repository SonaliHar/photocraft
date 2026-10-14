//! Controlled resident history metadata/cache staging. Historical document payloads remain caller-owned.
use super::{History, HistoryState, LayerTarget, tiles};
use photocraft_doc::Document;
use photocraft_raster::{Interrupt, RasterInputError};
use std::{
    collections::{HashSet, VecDeque},
    mem::size_of,
    sync::Arc,
};
type Result<T> = std::result::Result<T, RasterInputError>;
#[derive(Clone, Copy, Debug)]
pub struct ResidentHistoryLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for ResidentHistoryLimits {
    fn default() -> Self {
        Self { resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES, work_units: 1 << 32 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResidentHistoryRequirements {
    pub retained_bytes: u64,
    pub clone_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_bytes: u64,
    /// Conservative census plus clone, record, cache and repeated trim traversal work.
    pub work_units: u64,
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn amount(n: usize) -> Result<u64> {
    u64::try_from(n).map_err(|_| RasterInputError::LimitExceeded)
}
fn bytes<T>(n: usize) -> Result<u64> {
    mul(amount(n)?, size_of::<T>() as u64)
}
fn validate(l: ResidentHistoryLimits) -> Result<()> {
    let max = ResidentHistoryLimits::default();
    if l.resident_bytes > max.resident_bytes || l.work_units > max.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}
pub(crate) struct Meter<'c, 'i> {
    pub(crate) ctl: &'c Interrupt<'i>,
    limits: ResidentHistoryLimits,
    used: u64,
    resident: u64,
}
impl Meter<'_, '_> {
    pub(crate) fn step(&mut self, n: u64) -> Result<()> {
        self.ctl.check().map_err(|_| RasterInputError::Cancelled)?;
        self.used = add(self.used, n)?;
        if self.used > self.limits.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        Ok(())
    }
    pub(crate) fn allocate(&mut self, n: u64) -> Result<()> {
        self.step(1)?;
        self.resident = add(self.resident, n)?;
        if self.resident > self.limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        Ok(())
    }
}
fn target_bytes(v: &LayerTarget) -> Result<u64> {
    bytes::<photocraft_doc::LayerId>(v.selected.capacity())
}
fn string_copy(s: &str, capacity: usize, m: &mut Meter<'_, '_>) -> Result<String> {
    m.allocate(amount(capacity.max(s.len()))?)?;
    let mut out = String::new();
    out.try_reserve_exact(capacity.max(s.len())).map_err(|_| RasterInputError::AllocationFailed)?;
    m.allocate(amount(out.capacity().saturating_sub(capacity.max(s.len())))?)?;
    let mut start = 0usize;
    while start < s.len() {
        let mut end = start.checked_add(32768).ok_or(RasterInputError::LimitExceeded)?.min(s.len());
        while !s.is_char_boundary(end) {
            end = end.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
        }
        let chunk = s.get(start..end).ok_or(RasterInputError::InvalidBounds)?;
        m.step(chunk.len() as u64)?;
        out.push_str(chunk);
        m.step(1)?;
        start = end;
    }
    Ok(out)
}
fn target_copy(v: &LayerTarget, m: &mut Meter<'_, '_>) -> Result<LayerTarget> {
    m.allocate(target_bytes(v)?)?;
    let mut selected = Vec::new();
    selected.try_reserve_exact(v.selected.capacity()).map_err(|_| RasterInputError::AllocationFailed)?;
    m.allocate(bytes::<photocraft_doc::LayerId>(selected.capacity().saturating_sub(v.selected.capacity()))?)?;
    for chunk in v.selected.chunks(4096) {
        m.step(chunk.len() as u64)?;
        selected.extend_from_slice(chunk);
    }
    Ok(LayerTarget { active: v.active, selected })
}
fn unique_bytes(doc: &Document, seen: &mut HashSet<usize>, m: &mut Meter<'_, '_>) -> Result<usize> {
    let mut total = 0usize;
    tiles::surfaces_with(doc, m, |surface, m| {
        for (_, tile) in surface.tiles() {
            m.step(1)?;
            let key = Arc::as_ptr(tile) as usize;
            if seen.insert(key) {
                total = total.saturating_add(tile.bytes().len());
            }
        }
        Ok(())
    })?;
    Ok(total)
}
impl History {
    #[allow(clippy::too_many_arguments)]
    pub fn resident_record_requirements(
        &self,
        before: &Document,
        candidate: &Document,
        label: &str,
        prior: &LayerTarget,
        layers: &LayerTarget,
        max_states: usize,
        max_bytes: usize,
        limits: ResidentHistoryLimits,
        ctl: &Interrupt<'_>,
    ) -> Result<ResidentHistoryRequirements> {
        self.resident_record_requirements_with_extra_tiles(before, candidate, label, prior, layers, max_states, max_bytes, 0, limits, ctl)
    }
    /// `extra_candidate_tile_occurrences` is caller-proven prospective geometry, not source authority.
    /// It charges scratch/growth/work only; actual native payload backing must be retained separately.
    #[allow(clippy::too_many_arguments)]
    pub fn resident_record_requirements_with_extra_tiles(
        &self,
        before: &Document,
        candidate: &Document,
        label: &str,
        prior: &LayerTarget,
        layers: &LayerTarget,
        _max_states: usize,
        max_bytes: usize,
        extra_candidate_tile_occurrences: u64,
        limits: ResidentHistoryLimits,
        ctl: &Interrupt<'_>,
    ) -> Result<ResidentHistoryRequirements> {
        validate(limits)?;
        let mut m = Meter { ctl, limits, used: 0, resident: 0 };
        m.step(1)?;
        if label.len() > 1 << 20 || label.contains('\0') {
            return Err(RasterInputError::InvalidBounds);
        }
        m.step(label.len() as u64)?;
        let mut retained = add(size_of::<Self>() as u64, add(bytes::<HistoryState>(self.undo.capacity())?, bytes::<HistoryState>(self.redo.capacity())?)?)?;
        retained = add(retained, add(self.current_label.capacity() as u64, target_bytes(&self.current_layers)?)?)?;
        if retained > limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut visits = 0u64;
        let mut occurrences = extra_candidate_tile_occurrences;
        let mut scan = |doc: &Document, m: &mut Meter<'_, '_>| -> Result<()> {
            tiles::surfaces_with(doc, m, |s, m| {
                visits = add(visits, 1)?;
                occurrences = add(occurrences, amount(s.tile_count())?)?;
                // Origin checking and actual enumeration are polled; count alone is not validation.
                s.try_validate_tile_origins_with(m.ctl)?;
                for _ in s.tiles() {
                    m.step(1)?;
                }
                Ok(())
            })
        };
        for state in self.undo.iter().chain(&self.redo) {
            m.step(1)?;
            retained = add(retained, add(state.label.capacity() as u64, target_bytes(&state.layers)?)?)?;
            if retained > limits.resident_bytes {
                return Err(RasterInputError::LimitExceeded);
            }
            scan(&state.doc, &mut m)?;
        }
        scan(before, &mut m)?;
        scan(candidate, &mut m)?;
        let cache = self.tiles.as_ref().map_or(0, tiles::Tiles::capacity);
        retained = add(retained, mul(cache as u64, 128)?)?;
        let targets = add(target_bytes(prior)?, target_bytes(layers)?)?;
        let metadata = add(mul(retained, 2)?, add(add(label.len() as u64, targets)?, mul(size_of::<HistoryState>() as u64, 2)?)?)?;
        let slots = add(cache as u64, occurrences)?;
        let cache_clone = if self.tiles.is_some() || max_bytes != 0 { mul(slots, 256)? } else { 0 };
        let scratch = if max_bytes != 0 { mul(occurrences, 256)? } else { 0 };
        let clone_bytes = add(metadata, cache_clone)?;
        let peak_bytes = add(retained, add(clone_bytes, scratch)?)?;
        // Twenty complete structural/tile walks cover clone, add/remove, initialization,
        // fits and newest-first trimming, including duplicates and rejected attempts.
        let work_units = add(
            mul(m.used, 20)?,
            add(mul(add(add(visits, occurrences)?, add(self.undo.len() as u64, self.redo.len() as u64)?)?, 20)?, mul(add(clone_bytes, scratch)?, 4)?)?,
        )?;
        let work_units = add(work_units, 1)?;
        if peak_bytes > limits.resident_bytes || work_units > limits.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        m.step(1)?;
        Ok(ResidentHistoryRequirements { retained_bytes: retained, clone_bytes, scratch_bytes: scratch, peak_bytes, work_units })
    }
    /// Private fallible clone/record/trim; errors and cancellation leave `self` and documents intact.
    #[allow(clippy::too_many_arguments)]
    pub fn try_clone_record_trim_with_interrupt(
        &self,
        before: Arc<Document>,
        candidate: &Document,
        label: &str,
        prior: &LayerTarget,
        layers: &LayerTarget,
        max_states: usize,
        max_bytes: usize,
        limits: ResidentHistoryLimits,
        ctl: &Interrupt<'_>,
    ) -> Result<History> {
        let req = self.resident_record_requirements(&before, candidate, label, prior, layers, max_states, max_bytes, limits, ctl)?;
        let mut m = Meter { ctl, limits, used: 0, resident: req.retained_bytes };
        m.allocate(size_of::<History>() as u64)?;
        let undo_slots = self.undo.capacity().max(self.undo.len().checked_add(1).ok_or(RasterInputError::LimitExceeded)?);
        m.allocate(bytes::<HistoryState>(undo_slots)?)?;
        let mut undo = VecDeque::new();
        undo.try_reserve_exact(undo_slots).map_err(|_| RasterInputError::AllocationFailed)?;
        m.allocate(bytes::<HistoryState>(undo.capacity().saturating_sub(undo_slots))?)?;
        for state in &self.undo {
            m.step(1)?;
            undo.push_back(HistoryState {
                label: string_copy(&state.label, state.label.capacity(), &mut m)?,
                doc: Arc::clone(&state.doc),
                layers: target_copy(&state.layers, &mut m)?,
            });
        }
        // Redo states will be discarded by record. They are borrowed for exact cache removals,
        // so copying their metadata would add no semantic value or ownership.
        let current_label = string_copy(&self.current_label, self.current_label.capacity(), &mut m)?;
        let current_layers = target_copy(prior, &mut m)?;
        let mut cache = if max_bytes == 0 { None } else { self.tiles.as_ref().map(|v| v.try_clone_with(&mut m)).transpose()? };
        if let Some(v) = &mut cache {
            v.add_with(&before, &mut m)?;
            for state in &self.redo {
                v.remove_with(&state.doc, &mut m)?;
            }
        }
        let fresh_label = string_copy(label, label.len(), &mut m)?;
        let fresh_layers = target_copy(layers, &mut m)?;
        undo.push_back(HistoryState { label: current_label, doc: before, layers: current_layers });
        while undo.len() > max_states {
            m.step(1)?;
            if let Some(state) = undo.pop_front() {
                if let Some(v) = &mut cache {
                    v.remove_with(&state.doc, &mut m)?;
                }
            }
        }
        if max_bytes != 0 && undo.len() > 1 {
            if cache.is_none() {
                let mut v = tiles::Tiles::default();
                for state in &undo {
                    v.add_with(&state.doc, &mut m)?;
                }
                cache = Some(v);
            }
            let v = cache.as_mut().ok_or(RasterInputError::InvalidBounds)?;
            // A reusable fallibly reserved set preserves both legacy alias-counting passes.
            let mut seen = HashSet::new();
            let slots = usize::try_from(req.scratch_bytes / 256).map_err(|_| RasterInputError::LimitExceeded)?;
            m.allocate(mul(slots as u64, 128)?)?;
            seen.try_reserve(slots).map_err(|_| RasterInputError::AllocationFailed)?;
            m.allocate(mul(seen.capacity().saturating_sub(slots) as u64, 128)?)?;
            if !v.fits_with(candidate, max_bytes, &mut seen, &mut m)? {
                seen.clear();
                let mut total = unique_bytes(candidate, &mut seen, &mut m)?;
                let mut keep = 0usize;
                for state in undo.iter().rev() {
                    m.step(1)?;
                    total = total.saturating_add(unique_bytes(&state.doc, &mut seen, &mut m)?);
                    if total > max_bytes && keep >= 1 {
                        break;
                    }
                    keep += 1;
                }
                let count = undo.len().checked_sub(keep).ok_or(RasterInputError::InvalidBounds)?;
                for _ in 0..count {
                    m.step(1)?;
                    if let Some(state) = undo.pop_front() {
                        v.remove_with(&state.doc, &mut m)?;
                    }
                }
            }
        }
        m.step(1)?;
        Ok(History { undo, redo: Vec::new(), tiles: cache, max_states, max_bytes, current_label: fresh_label, current_layers: fresh_layers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, LayerContent, PixelFormat, SampleType, Size};
    use photocraft_raster::Surface;
    fn document(value: u8) -> Arc<Document> {
        let mut doc = Document::new("Own history", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let surface = Surface::try_from_interleaved(PixelFormat::RGBA8, photocraft_doc::Rect::new(0, 0, 1, 1), &[value, 0, 0, 255]).unwrap();
        doc.layers.push(Layer::new("Own tile", LayerContent::Raster(surface)));
        Arc::new(doc)
    }
    fn target() -> LayerTarget {
        LayerTarget::default()
    }
    #[test]
    fn cached_and_uncached_record_trim_match_actual_legacy_order_and_bytes() {
        for initialize in [false, true] {
            let a = document(1);
            let b = document(2);
            let c = document(3);
            let next = document(4);
            let mut history = History::new(10);
            history.record("A", a.clone(), target());
            history.record("B", b.clone(), target());
            history.max_bytes = 1;
            if initialize {
                history.trim(&c);
            }
            let previous = history.clone();
            let mut expected = history.clone();
            expected.max_states = 10;
            expected.max_bytes = 1;
            expected.set_current_layers(target());
            expected.record("Own update", c.clone(), target());
            expected.trim(&next);
            let result = history
                .try_clone_record_trim_with_interrupt(
                    c.clone(),
                    &next,
                    "Own update",
                    &target(),
                    &target(),
                    10,
                    1,
                    ResidentHistoryLimits::default(),
                    &Interrupt::NONE,
                )
                .unwrap();
            assert_eq!(result.entry_labels().collect::<Vec<_>>(), expected.entry_labels().collect::<Vec<_>>());
            assert_eq!(result.pixel_bytes(&next), expected.pixel_bytes(&next));
            assert_eq!(result.undo.len(), expected.undo.len());
            for (actual, oracle) in result.undo.iter().zip(&expected.undo) {
                assert!(Arc::ptr_eq(&actual.doc, &oracle.doc));
                assert_eq!(actual.layers, oracle.layers);
            }
            assert_eq!(history.entry_labels().collect::<Vec<_>>(), previous.entry_labels().collect::<Vec<_>>());
            assert_eq!(history.pixel_bytes(&c), previous.pixel_bytes(&c));
        }
    }
    #[test]
    fn redo_aliases_are_removed_before_new_record_and_source_cache_is_preserved() {
        let a = document(1);
        let b = document(2);
        let next = document(3);
        let mut h = History::new(10);
        h.max_bytes = usize::MAX;
        h.record("A", a.clone(), target());
        h.record("Alias", a.clone(), target());
        h.trim(&b);
        h.undo(b.clone()).unwrap();
        let mut oracle = h.clone();
        oracle.set_current_layers(target());
        oracle.record("Own update", a.clone(), target());
        oracle.trim(&next);
        let result = h
            .try_clone_record_trim_with_interrupt(
                a.clone(),
                &next,
                "Own update",
                &target(),
                &target(),
                10,
                usize::MAX,
                ResidentHistoryLimits::default(),
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(result.pixel_bytes(&next), oracle.pixel_bytes(&next));
        assert!(!result.can_redo());
        assert!(h.can_redo());
        assert_eq!(result.entry_labels().collect::<Vec<_>>(), oracle.entry_labels().collect::<Vec<_>>());
    }
    #[test]
    fn prospective_extra_tile_and_lower_peak_refuse_before_private_clone() {
        let a = document(1);
        let h = History::new(10);
        let limits = ResidentHistoryLimits::default();
        let exact = h.resident_record_requirements(&a, &a, "Own update", &target(), &target(), 10, 1, limits, &Interrupt::NONE).unwrap();
        let extra = h.resident_record_requirements_with_extra_tiles(&a, &a, "Own update", &target(), &target(), 10, 1, 1, limits, &Interrupt::NONE).unwrap();
        assert!(extra.peak_bytes > exact.peak_bytes);
        assert!(extra.work_units > exact.work_units);
        assert!(
            h.try_clone_record_trim_with_interrupt(
                a.clone(),
                &a,
                "Own update",
                &target(),
                &target(),
                10,
                1,
                ResidentHistoryLimits { resident_bytes: exact.peak_bytes - 1, ..limits },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(h.past_len(), 0);
        assert!(
            h.resident_record_requirements_with_extra_tiles(&a, &a, "Own update", &target(), &target(), 10, 1, u64::MAX, limits, &Interrupt::NONE).is_err()
        );
    }
    #[test]
    fn late_cancel_does_not_publish_private_history_or_change_shared_documents() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let a = document(1);
        let b = document(2);
        let mut h = History::new(10);
        h.record("A", a.clone(), target());
        h.record("B", a.clone(), target());
        h.max_bytes = usize::MAX;
        h.trim(&b);
        h.current_label = "α".repeat(32768);
        let checks = AtomicU64::new(0);
        let stop = || checks.fetch_add(1, Ordering::Relaxed) > 70;
        let ctl = Interrupt::cancel_only(&stop);
        let labels = h.entry_labels().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            h.try_clone_record_trim_with_interrupt(b.clone(), &a, "Own update", &target(), &target(), 10, usize::MAX, ResidentHistoryLimits::default(), &ctl)
                .unwrap_err(),
            RasterInputError::Cancelled
        );
        assert_eq!(h.entry_labels().map(str::to_owned).collect::<Vec<_>>(), labels);
        assert_eq!(a.name, "Own history");
    }
}
