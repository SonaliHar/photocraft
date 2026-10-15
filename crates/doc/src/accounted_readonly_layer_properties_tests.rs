//! UNRUN combined CURRENT properties over the shared real-quota native fixture.
use super::*;
use crate::accounted_document::ReadonlyLayerPropertiesPatch;

fn properties(owner: &AccountedDocument) -> ReadonlyLayerPropertiesPatch<'static> {
    ReadonlyLayerPropertiesPatch { layer: owner.document().layers[0].id, name: None, visible: None, opacity_f32_bits: None }
}
struct FixtureGuard {
    _source: AccountedDocument,
    _metadata: photocraft_raster::backing::quota::QuotaLease,
}
fn current_fixture(histogram: bool, configure: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = quota();
    let source = fixture(&q, histogram, "old name", "old name", false);
    // Known CURRENT metadata variants only: pay before the test-only model
    // clone, retain the actual native/profile/mask/source owner and its quota.
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut document = source.document().clone();
    configure(&mut document);
    let guard: Arc<dyn Any + Send + Sync> = Arc::new(FixtureGuard { _source: source, _metadata: fee });
    if histogram {
        AccountedDocument::new_readonly_threshold_storage(document, guard, q, &control()).unwrap()
    } else {
        AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q, &control()).unwrap()
    }
}

#[test]
fn compound_fields_are_one_owner_copy_and_preserve_filter_native_mask_profile_source() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "old name", "old name", false);
        let before = old.document().clone();
        let next = old
            .try_with_readonly_layer_properties(
                ReadonlyLayerPropertiesPatch {
                    name: Some("  current 名  "),
                    visible: Some(false),
                    opacity_f32_bits: Some(0.375f32.to_bits()),
                    ..properties(&old)
                },
                &control(),
            )
            .unwrap();
        let mut expected = before.clone();
        expected.layers[0].name = "  current 名  ".into();
        expected.layers[0].visible = false;
        expected.layers[0].opacity = 0.375;
        let item = &mut expected.xcf_state.as_mut().unwrap().items[1];
        item.name = "  current 名  ".into();
        item.custom_name = true;
        item.visible = false;
        assert_eq!(next.document(), &expected);
        assert_eq!(old.document(), &before);
        assert!(!next.shares_model(&old));
        assert!(next.quota().shares_ledger(&q));
        assert!(Arc::ptr_eq(old.document().icc_profile.as_ref().unwrap(), next.document().icc_profile.as_ref().unwrap()));
        let a = &old.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        let b = &next.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        assert_eq!(a.default_bytes(), b.default_bytes());
        assert!(Arc::ptr_eq(a.tiles().next().unwrap().1, b.tiles().next().unwrap().1));
        assert_eq!(next.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, u64::MAX);
        assert!(
            next.try_edit(
                AccountedEdit::LayerMetadata { layer: next.document().layers[0].id, name: Some("generic"), visible: None, opacity: None },
                &control()
            )
            .is_err()
        );
    }
}

#[test]
fn exact_noops_need_no_room_and_signed_zero_is_a_real_layer_only_change() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "same", "same", false);
        let blocker = q.reserve(QuotaKind::Index, (16 << 20) - q.usage().unwrap().index).unwrap();
        let full = q.usage().unwrap();
        for patch in [
            properties(&old),
            ReadonlyLayerPropertiesPatch { name: Some("same"), visible: Some(true), opacity_f32_bits: Some(1f32.to_bits()), ..properties(&old) },
        ] {
            let noop = old.try_with_readonly_layer_properties(patch, &control()).unwrap();
            assert!(noop.shares_model(&old));
            assert!(!noop.document().xcf_state.as_ref().unwrap().items[1].custom_name);
        }
        assert!(old.try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { visible: Some(false), ..properties(&old) }, &control()).is_err());
        assert_eq!(q.usage().unwrap(), full);
        drop(blocker);
        let negative = old
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { opacity_f32_bits: Some((-0f32).to_bits()), ..properties(&old) }, &control())
            .unwrap();
        assert_eq!(negative.document().layers[0].opacity.to_bits(), (-0f32).to_bits());
        let noop = negative
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { opacity_f32_bits: Some((-0f32).to_bits()), ..properties(&negative) }, &control())
            .unwrap();
        assert!(noop.shares_model(&negative));
        let positive = negative
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { opacity_f32_bits: Some(0f32.to_bits()), ..properties(&negative) }, &control())
            .unwrap();
        assert!(!positive.shares_model(&negative));
        assert_eq!(positive.document().layers[0].opacity.to_bits(), 0f32.to_bits());
        assert_eq!(positive.document().xcf_state, old.document().xcf_state);
    }
}

#[test]
fn omitted_fields_reconcile_canonical_name_and_visibility_with_exact_custom_semantics() {
    for histogram in [false, true] {
        let old = current_fixture(histogram, |doc| {
            let item = &mut doc.xcf_state.as_mut().unwrap().items[1];
            item.name = "stale".into();
            item.visible = false;
        });
        let repaired = old.try_with_readonly_layer_properties(properties(&old), &control()).unwrap();
        assert!(!repaired.shares_model(&old));
        let item = &repaired.document().xcf_state.as_ref().unwrap().items[1];
        assert_eq!(item.name, "old name");
        assert!(item.custom_name);
        assert!(item.visible);
        assert_eq!(repaired.document().layers, old.document().layers);
        let q = quota();
        let old = fixture(&q, histogram, "different", "target", false);
        let updated = old
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { name: Some("target"), visible: Some(false), ..properties(&old) }, &control())
            .unwrap();
        assert!(!updated.document().xcf_state.as_ref().unwrap().items[1].custom_name);
        // Old name-only API still does not synchronize visibility on a name noop.
        let old = current_fixture(histogram, |doc| doc.xcf_state.as_mut().unwrap().items[1].visible = false);
        let same = old.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: old.document().layers[0].id, name: "old name" }, &control()).unwrap();
        assert!(same.shares_model(&old));
        assert!(!same.document().xcf_state.as_ref().unwrap().items[1].visible);
    }
}

#[test]
fn effective_name_repair_is_prepaid_when_name_is_absent_and_request_keeps_opacity() {
    for histogram in [false, true] {
        let old = current_fixture(histogram, |doc| {
            doc.layers[0].name = "x".repeat(1 << 20);
            doc.xcf_state.as_mut().unwrap().items[1].name.clear();
        });
        let q = old.quota().clone();
        let before = q.usage().unwrap();
        let stop_after_fees = || q.usage().unwrap().index > before.index;
        let interrupt = Interrupt::cancel_only(&stop_after_fees);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(old.try_with_readonly_layer_properties(properties(&old), &ctl), Err(BackingError::Cancelled)));
        assert_eq!(q.usage().unwrap(), before);
        let fixed = old.try_with_readonly_layer_properties(properties(&old), &control()).unwrap();
        assert_eq!(fixed.document().xcf_state.as_ref().unwrap().items[1].name, old.document().layers[0].name);
        assert_eq!(fixed.document().layers[0].opacity.to_bits(), old.document().layers[0].opacity.to_bits());
    }
}

#[test]
fn malformed_fields_targets_and_actual_work_cancel_fail_privately_and_legacy_refuses() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "old", "old", false);
        let before = q.usage().unwrap();
        for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.1f32).to_bits(), 1.1f32.to_bits()] {
            assert!(
                old.try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { opacity_f32_bits: Some(bits), ..properties(&old) }, &control()).is_err()
            );
        }
        for name in ["bad\0name".into(), "x".repeat((1 << 20) + 1)] {
            assert!(old.try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { name: Some(&name), ..properties(&old) }, &control()).is_err());
        }
        assert!(old.try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { layer: LayerId(0), ..properties(&old) }, &control()).is_err());
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(old.try_with_readonly_layer_properties(properties(&old), &deny), Err(BackingError::Work)));
        let calls = AtomicU64::new(0);
        let cancelled = || calls.fetch_add(1, Ordering::Relaxed) > 12;
        let interrupt = Interrupt::cancel_only(&cancelled);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(
            old.try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { name: Some("new"), visible: Some(false), ..properties(&old) }, &ctl),
            Err(BackingError::Cancelled)
        ));
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(old.document().layers[0].name, "old");
    }
    let q = quota();
    let owner = fixture(&q, false, "old", "old", false);
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut document = owner.document().clone();
    let state = document.xcf_state.as_mut().unwrap();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.items.retain(|item| matches!(item.item, ItemRef::Layer(_)));
    let guard: Arc<dyn Any + Send + Sync> = Arc::new(FixtureGuard { _source: owner, _metadata: fee });
    let legacy = AccountedDocument::from_owned(document, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(legacy.try_with_readonly_layer_properties(properties(&legacy), &control()), Err(BackingError::Unsupported)));
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
fn combined_title_name_and_filter_copies_keep_flat_retention_and_old_name_api() {
    for histogram in [false, true] {
        let original = current_fixture(histogram, |doc| doc.xcf_state.as_mut().unwrap().filter_stacks[0].revision = 0);
        let q = original.quota().clone();
        let mut current = original.clone();
        let mut last = None;
        for i in 0..24 {
            current = current
                .try_with_readonly_layer_properties(
                    ReadonlyLayerPropertiesPatch {
                        name: Some(if i % 2 == 0 { "layer A" } else { "layer B" }),
                        visible: Some(i % 2 == 0),
                        opacity_f32_bits: Some(0.5f32.to_bits()),
                        ..properties(&current)
                    },
                    &control(),
                )
                .unwrap();
            current = current.try_with_document_name(if i % 2 == 0 { "title A" } else { "title B" }, &control()).unwrap();
            let stack = &current.document().xcf_state.as_ref().unwrap().filter_stacks[0];
            current = if histogram {
                current
                    .try_with_threshold(
                        ThresholdPatch {
                            layer: current.document().layers[0].id,
                            filter: stack.nodes[0].id,
                            expected_stack_revision: stack.revision,
                            low_f32_bits: None,
                            high_f32_bits: None,
                            channel: None,
                            enabled: Some(i % 2 == 0),
                        },
                        &control(),
                    )
                    .unwrap()
            } else {
                current
                    .try_with_threshold_alpha(
                        ThresholdAlphaPatch {
                            layer: current.document().layers[0].id,
                            filter: stack.nodes[0].id,
                            expected_stack_revision: stack.revision,
                            value_f32_bits: None,
                            enabled: Some(i % 2 == 0),
                        },
                        &control(),
                    )
                    .unwrap()
            };
            // The first activation is already true. Later frames all have the
            // same-sized metadata and a real changed activation/revision.
            if i > 1 {
                if let Some(before) = last {
                    assert_eq!(q.usage().unwrap(), before);
                }
            }
            last = Some(q.usage().unwrap());
        }
        let old_mask = &original.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        let new_mask = &current.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        assert!(Arc::ptr_eq(old_mask.tiles().next().unwrap().1, new_mask.tiles().next().unwrap().1));
        let renamed =
            current.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: current.document().layers[0].id, name: "old API" }, &control()).unwrap();
        assert_eq!(renamed.document().layers[0].visible, current.document().layers[0].visible);
        assert_eq!(renamed.document().layers[0].opacity.to_bits(), current.document().layers[0].opacity.to_bits());
    }
}
