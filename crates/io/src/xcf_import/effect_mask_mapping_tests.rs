//! UNRUN exact eager adapter/native-empty-mask mapping and failure drafts.
use super::*;
use photocraft_doc::item_identity::{FilterId, ItemRef, LayerId, MaskId};
use photocraft_xcf::{Effect, EffectState};

fn owner(sample: Sample, words: Vec<u8>) -> Drawable {
    let mask = Drawable {
        name: "exact zero selection".into(),
        width: 2,
        height: 1,
        model: Model::Gray,
        alpha: false,
        visible: true,
        mask_enabled: true,
        mask_shown: false,
        mask_editing: false,
        legacy_linked: false,
        mode: 0,
        blend_space: 2,
        composite_space: 2,
        composite_mode: 1,
        tattoo: Some(77),
        group_flags: 13,
        lock_content: true,
        lock_position: true,
        lock_alpha: true,
        lock_visibility: true,
        plane: Some(Plane { width: 2, height: 1, channels: 1, sample, bytes: words }),
        ..Default::default()
    };
    Drawable {
        name: "current owner".into(),
        width: 2,
        height: 1,
        plane: Some(Plane { width: 2, height: 1, channels: 4, sample, bytes: vec![0; sample.source_bytes() * 8] }),
        effects: vec![Effect {
            name: "unchanged".into(),
            icon: String::new(),
            operation: "gegl:exposure".into(),
            version: Some("0:0".into()),
            properties: Vec::new(),
            arguments: Vec::new(),
            mask: Some(Box::new(mask)),
            state: EffectState::default(),
        }],
        ..Default::default()
    }
}
fn budget(max: usize) -> Budget<'static> {
    Budget { used: 0, max, peak: 0, shared: None }
}
fn run(d: &mut Drawable, state: &mut photocraft_doc::XcfState, b: &mut Budget<'_>, ctl: &Interrupt<'_>) -> Result<Vec<Option<ItemRef>>, IoError> {
    map(d, ItemRef::Layer(LayerId(91)), 17, &mut 8, state, b, ctl, false, true, false, false, false, None, None)
}

#[test]
fn exact_negative_zero_native_words_and_full_mask_metadata_survive_eager_hook() {
    for (sample, words) in [(Sample::F32, (-0.0f32).to_ne_bytes().repeat(2)), (Sample::F64, (-0.0f64).to_ne_bytes().repeat(2))] {
        let mut source = owner(sample, words.clone());
        let pointer = source.effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes.as_ptr();
        let mut state = photocraft_doc::XcfState::default();
        let mapped = run(&mut source, &mut state, &mut budget(8 << 20), &Interrupt::NONE).unwrap();
        assert_eq!(mapped, [Some(ItemRef::Mask(MaskId(8)))]);
        assert_eq!(state.filter_masks[0].owner, FilterId(17));
        assert_eq!(state.filter_masks[0].surface.format().sample, depth(sample));
        assert_eq!(state.filter_masks[0].surface.try_read_interleaved(Rect::new(0, 0, 2, 1)).unwrap(), words);
        let item = &state.items[0];
        assert_eq!(item.source_tattoo, Some(77));
        assert_eq!(item.source_flags, 13);
        assert!(item.locks.content && item.locks.position && item.locks.alpha && item.locks.visibility);
        let mask = item.mask.as_ref().unwrap();
        assert_eq!(mask.owner, ItemRef::Layer(LayerId(91)));
        assert_eq!(mask.outside_f64_bits, 0.0f64.to_bits());
        assert!(mask.enabled && !mask.shown && !mask.editing && !mask.linked);
        // Eager adapter still preserves the authoritative original allocation.
        let original = source.effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
        assert_eq!(original.bytes.as_ptr(), pointer);
        assert_eq!(original.bytes, words);
    }
}

#[test]
fn nonempty_or_unsupported_mask_is_not_installed_and_cancellation_propagates() {
    for words in [0.5f32.to_ne_bytes().repeat(2), f32::NAN.to_ne_bytes().repeat(2)] {
        let mut source = owner(Sample::F32, words);
        let mut state = photocraft_doc::XcfState::default();
        assert!(run(&mut source, &mut state, &mut budget(8 << 20), &Interrupt::NONE).is_err());
        assert!(state.filter_masks.is_empty() && state.items.is_empty());
    }
    let mut source = owner(Sample::F64, 0.0f64.to_ne_bytes().repeat(2));
    source.effects[0].mask.as_mut().unwrap().mask_shown = true;
    let mut state = photocraft_doc::XcfState::default();
    assert!(run(&mut source, &mut state, &mut budget(8 << 20), &Interrupt::NONE).is_err());
    assert!(state.filter_masks.is_empty() && state.items.is_empty());
    let stop = || true;
    assert!(matches!(run(&mut source, &mut state, &mut budget(8 << 20), &Interrupt::cancel_only(&stop)), Err(IoError::Cancelled)));
    assert!(state.filter_masks.is_empty() && state.items.is_empty());
}

#[test]
fn eager_surface_budget_refuses_before_ingestion_and_no_mask_keeps_original_noop() {
    let mut source = owner(Sample::F64, (-0.0f64).to_ne_bytes().repeat(2));
    let mut state = photocraft_doc::XcfState::default();
    assert!(run(&mut source, &mut state, &mut budget(0), &Interrupt::NONE).is_err());
    assert!(state.filter_masks.is_empty() && state.items.is_empty());
    source.effects[0].mask = None;
    // Without any mask, invalid unused owner geometry was never consulted.
    source.width = u32::MAX;
    assert_eq!(run(&mut source, &mut state, &mut budget(0), &Interrupt::NONE).unwrap(), [None]);
    assert!(state.filter_masks.is_empty() && state.items.is_empty());
}
