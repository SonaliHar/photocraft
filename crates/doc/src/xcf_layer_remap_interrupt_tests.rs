//! Unrun controlled layer-remapping regressions for the implementation wave.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use photocraft_raster::Interrupt;

fn item(id: u64) -> ItemState {
    ItemState {
        item: ItemRef::Layer(LayerId(id)), name: "Own retained name".into(), custom_name: false,
        visible: true, locks: ItemLocks::default(), parent: None, source_tattoo: Some(17),
        source_flags: 29, raster_bounds: None, mask: None, composition: None,
    }
}
fn source() -> XcfState {
    let mut state = XcfState { items: vec![item(1), item(2)], ..Default::default() };
    state.selection.selected = vec![ItemRef::Layer(LayerId(1)); 4096];
    state.selection.active = Some(ItemRef::Layer(LayerId(2)));
    state
}

#[test]
fn repeats_remap_exactly_once_per_identity_and_keep_archival_fields() {
    let mut state = source();
    let mut calls = 0;
    state.remap_layers_with_interrupt(|id| { calls += 1; Some(LayerId(id.0 + 10)) }, &Interrupt::NONE).unwrap();
    assert_eq!(calls, 2);
    assert!(state.selection.selected.iter().all(|reference| *reference == ItemRef::Layer(LayerId(11))));
    assert_eq!(state.selection.active, Some(ItemRef::Layer(LayerId(12))));
    assert_eq!(state.items[0].source_tattoo, Some(17));
    assert_eq!(state.items[0].source_flags, 29);
}

#[test]
fn missing_mapping_and_cancel_after_mapping_are_atomic() {
    for cancel_after_mapping in [false, true] {
        let mut state = source();
        let before = state.clone();
        let cancelled = AtomicBool::new(false);
        let poll = || cancelled.load(Ordering::Relaxed);
        let control = Interrupt::cancel_only(&poll);
        let failure = state.remap_layers_with_interrupt(|id| {
            if id == LayerId(2) {
                if cancel_after_mapping { cancelled.store(true, Ordering::Relaxed); }
                else { return None; }
            }
            Some(LayerId(id.0 + 10))
        }, &control).unwrap_err();
        assert_eq!(failure.code, if cancel_after_mapping { "cancelled" } else { "layer_reference" });
        assert_eq!(state, before);
    }
}

#[test]
fn repeated_reference_census_polls_and_refuses_excess_before_map() {
    let mut state = source();
    let before = state.clone();
    let checks = AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 128;
    let control = Interrupt::cancel_only(&cancel);
    let mut calls = 0;
    assert_eq!(state.remap_layers_with_interrupt(|id| { calls += 1; Some(id) }, &control).unwrap_err().code, "cancelled");
    assert_eq!(calls, 0);
    assert_eq!(state, before);
    state.selection.selected = vec![ItemRef::Layer(LayerId(1)); 100_001];
    let excessive = state.clone();
    assert_eq!(state.remap_layers_with_interrupt(|id| { calls += 1; Some(id) }, &Interrupt::NONE).unwrap_err().code, "item_limit");
    assert_eq!(calls, 0);
    assert_eq!(state, excessive);
}
