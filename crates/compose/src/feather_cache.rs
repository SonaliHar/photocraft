//! Private checked feather cache. Only registered immutable document owners may
//! be cached by address; temporary effect layers are evaluated without insertion.
use super::{
    feather_interrupt::{self, FeatherPreparationLimits},
    masks,
};
use photocraft_doc::{Document, Layer};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
pub(super) const FEATHER_BYTES: usize = 64 << 20;
type Key = (usize, usize);
#[derive(Default)]
struct State {
    sources: HashSet<Key>,
    maps: HashMap<Key, Arc<Surface>>,
    retained: usize,
}
#[derive(Default)]
pub(super) struct CheckedFeatherMasks {
    state: Mutex<State>,
}
fn key(layer: &Layer, owner: &Layer) -> Key {
    let pixel = layer.mask.as_ref().filter(|m| m.enabled).and(owner.mask.as_ref()).map_or(0, |m| std::ptr::from_ref(m) as usize);
    let vector = layer.vector_mask.as_ref().filter(|m| m.enabled).and(owner.vector_mask.as_ref()).map_or(0, |m| std::ptr::from_ref(m) as usize);
    (pixel, vector)
}
impl CheckedFeatherMasks {
    pub(super) fn register_sources(&self, doc: &Document, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
        self.register_sources_budgeted(doc, control, None)
    }
    pub(super) fn register_sources_budgeted(
        &self,
        doc: &Document,
        control: &Interrupt<'_>,
        bridge: Option<&super::preparation_bridge::PreparationBridge<'_>>,
    ) -> Result<(), RasterInputError> {
        let stage = if let Some(bridge) = bridge {
            let cap = doc
                .layers
                .len()
                .checked_mul(3)
                .and_then(|n| n.max(8).checked_next_power_of_two())
                .and_then(|n| n.checked_mul(128))
                .ok_or(RasterInputError::LimitExceeded)?;
            let reservation = bridge.stage(cap, control)?;
            if reservation.bytes() < cap {
                return Err(RasterInputError::LimitExceeded);
            }
            Some(reservation)
        } else {
            None
        };
        fn visit(layers: &[Layer], state: &mut State, depth: usize, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
            if depth >= 100 {
                return Err(RasterInputError::LimitExceeded);
            }
            for layer in layers {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
                if masks::has_feather(layer) {
                    let pair = key(layer, layer);
                    for pair in [pair, (pair.0, 0), (0, pair.1)] {
                        if pair == (0, 0) || state.sources.contains(&pair) {
                            continue;
                        }
                        if state.sources.len() >= 300000 {
                            return Err(RasterInputError::LimitExceeded);
                        }
                        // Conservative known slot/container payload headroom. No
                        // allocator/process-RSS equivalence is asserted.
                        let next = state.sources.capacity().saturating_mul(2).max(8);
                        if next.checked_mul(64).is_none_or(|n| n > FEATHER_BYTES) {
                            return Err(RasterInputError::LimitExceeded);
                        }
                        state.sources.try_reserve(1).map_err(|_| RasterInputError::AllocationFailed)?;
                        state.sources.insert(pair);
                    }
                }
                if let Some(children) = layer.children() {
                    visit(children, state, depth + 1, control)?;
                }
            }
            Ok(())
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = visit(&doc.layers, &mut state, 0, control) {
            // Destroy partial slot storage before the staging reservation ends.
            state.sources = HashSet::new();
            return Err(error);
        }
        state.retained = state.sources.capacity().checked_mul(64).ok_or(RasterInputError::LimitExceeded)?;
        if state.retained > FEATHER_BYTES {
            return Err(RasterInputError::LimitExceeded);
        }
        if let (Some(bridge), Some(stage)) = (bridge, stage) {
            bridge.retained(stage, state.retained)?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn values(&self, layer: &Layer, owner: &Layer, canvas: Rect, rect: Rect, control: &Interrupt<'_>) -> Result<Option<Vec<f32>>, RasterInputError> {
        self.values_budgeted(layer, owner, canvas, rect, control, None)
    }
    pub(super) fn values_budgeted(
        &self,
        layer: &Layer,
        owner: &Layer,
        canvas: Rect,
        rect: Rect,
        control: &Interrupt<'_>,
        bridge: Option<&super::preparation_bridge::PreparationBridge<'_>>,
    ) -> Result<Option<Vec<f32>>, RasterInputError> {
        let mut state = loop {
            match self.state.try_lock() {
                Ok(state) => break state,
                Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    control.check().map_err(|_| RasterInputError::Cancelled)?;
                    std::thread::yield_now();
                }
            }
        };
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let key = key(layer, owner);
        let known = state.sources.contains(&key);
        let surface = if let Some(surface) = state.maps.get(&key).filter(|_| known) {
            surface.clone()
        } else {
            let remaining = FEATHER_BYTES.checked_sub(state.retained).and_then(|n| n.checked_sub(1024)).ok_or(RasterInputError::LimitExceeded)?;
            if state.maps.len() >= 1024 {
                return Err(RasterInputError::LimitExceeded);
            }
            let stage = bridge.map(|b| b.stage(remaining, control)).transpose()?;
            let remaining = stage.as_ref().map_or(remaining, |s| s.bytes());
            let Some(surface) =
                feather_interrupt::try_combined_mask(layer, canvas, FeatherPreparationLimits { max_bytes: remaining, ..Default::default() }, control)?
            else {
                return Ok(None);
            };
            let bytes = usize::try_from(surface.resident_payload_bytes()?).map_err(|_| RasterInputError::LimitExceeded)?;
            if let (Some(bridge), Some(stage)) = (bridge, stage) {
                if known {
                    bridge.retained(stage, bytes.checked_add(1024).ok_or(RasterInputError::LimitExceeded)?)?;
                } else {
                    bridge.transient(stage, bytes)?;
                }
            }
            let surface = Arc::new(surface);
            if known {
                state.maps.try_reserve(1).map_err(|_| RasterInputError::AllocationFailed)?;
                state.retained = state.retained.checked_add(bytes).and_then(|n| n.checked_add(1024)).ok_or(RasterInputError::LimitExceeded)?;
                if state.retained > FEATHER_BYTES {
                    return Err(RasterInputError::LimitExceeded);
                }
                state.maps.insert(key, surface.clone());
            }
            surface
        };
        let format = surface.format();
        let bytes = photocraft_raster::checked_region_byte_len(rect, format)?;
        let transient = if known { 0 } else { usize::try_from(surface.resident_payload_bytes()?).map_err(|_| RasterInputError::LimitExceeded)? };
        if bytes.checked_add(state.retained).and_then(|n| n.checked_add(transient)).is_none_or(|n| n > FEATHER_BYTES) {
            return Err(RasterInputError::LimitExceeded);
        }
        let read_lease = bridge.map(|b| b.budget.reserve(bytes, control)).transpose()?;
        let mut values = Vec::new();
        values.try_reserve_exact(bytes / 4).map_err(|_| RasterInputError::AllocationFailed)?;
        if values.capacity() != bytes / 4 {
            return Err(RasterInputError::LimitExceeded);
        }
        if let (Some(bridge), Some(lease)) = (bridge, read_lease) {
            bridge.transient(lease, bytes)?;
        }
        surface.read_region_into(rect, &mut values);
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        // Ephemeral-owner output remains under the serial lock until it is
        // dropped, so two workers cannot retain uncharged whole-mask surfaces.
        drop(surface);
        drop(state);
        Ok(Some(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, LayerMask, SampleType, Size};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn cancelled_preparation_does_not_publish_private_or_global_cached_surface() {
        let mut doc = Document::with_background("own feather cache", Size::new(256, 256), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut mask = LayerMask::hide_all();
        std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(64, 64, 192, 192), &[0.73]);
        mask.feather = 16.25;
        doc.layers[0].mask = Some(mask);
        let before = doc.clone();
        let cache = CheckedFeatherMasks::default();
        cache.register_sources(&doc, &Interrupt::NONE).unwrap();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        assert!(matches!(
            cache.values(&doc.layers[0], &doc.layers[0], doc.bounds(), doc.bounds(), &Interrupt::cancel_only(&cancel)),
            Err(RasterInputError::Cancelled)
        ));
        assert!(cache.state.lock().unwrap_or_else(|e| e.into_inner()).maps.is_empty());
        let expected = masks::combined_mask(&doc.layers[0], doc.bounds()).unwrap();
        let mut expected_values = Vec::new();
        expected.read_region_into(doc.bounds(), &mut expected_values);
        assert_eq!(cache.values(&doc.layers[0], &doc.layers[0], doc.bounds(), doc.bounds(), &Interrupt::NONE).unwrap().unwrap(), expected_values);
        let state = cache.state.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(state.maps.len(), 1);
        let surface = state.maps.values().next().unwrap().clone();
        drop(state);
        assert_eq!(cache.values(&doc.layers[0], &doc.layers[0], doc.bounds(), doc.bounds(), &Interrupt::NONE).unwrap().unwrap(), expected_values);
        assert!(cache.state.lock().unwrap_or_else(|e| e.into_inner()).maps.values().any(|s| Arc::ptr_eq(s, &surface)));
        assert_eq!(doc, before);
    }
}
