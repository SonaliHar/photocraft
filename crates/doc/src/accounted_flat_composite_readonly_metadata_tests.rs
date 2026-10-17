//! UNRUN synthetic paid CURRENT storage tests. These do not qualify source,
//! profiles, blend math, command locks or Session gesture coalescing. Expected
//! model clones/native buffers are external test harness data.
use super::super::tests::{ctl, fixture, leaf, owned, quota};
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn edit(layer: LayerId, name: Option<&str>, visible: Option<bool>, opacity: Option<f32>) -> AccountedEdit<'_> {
    AccountedEdit::LayerMetadata { layer, name, visible, opacity }
}
#[test]
fn either_layer_combined_properties_preserve_native_profile_policies_and_flat_title_custody() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let q = quota();
            let baseline = q.usage().unwrap();
            let (owner, reads) = owned(&q, mode, sample);
            for index in 0..2 {
                let id = owner.document().layers[index].id;
                let request = edit(id, Some(" λ unchanged spaces "), Some(false), Some(0.375));
                owner.validate_imported_metadata_edit(request, &ctl()).unwrap();
                let changed = owner.try_edit(request, &ctl()).unwrap();
                let mut expected = owner.document().clone();
                let layer = &mut expected.layers[index];
                layer.name = " λ unchanged spaces ".into();
                layer.visible = false;
                layer.opacity = 0.375;
                let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap();
                item.name = " λ unchanged spaces ".into();
                item.visible = false;
                item.custom_name = true;
                assert_eq!(changed.document(), &expected);
                assert!(changed.is_readonly_flat_composite_storage());
                changed.validate_readonly_flat_composite_storage(&ctl()).unwrap();
                assert!(Arc::ptr_eq(&changed.owner.retention, &owner.owner.retention));
                assert!(Arc::ptr_eq(changed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
                assert!(Arc::ptr_eq(
                    changed.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
                    owner.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
                ));
                assert_eq!(leaf(&changed, index), leaf(&owner, index));
                let titled = changed.try_with_document_name("continued.pcraft", &ctl()).unwrap();
                expected.name = "continued.pcraft".into();
                assert_eq!(titled.document(), &expected);
                assert!(Arc::ptr_eq(&titled.owner.retention, &owner.owner.retention));
            }
            assert_eq!(reads.load(Ordering::Relaxed), 0);
            drop(owner);
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
}
#[test]
fn raw_zero_names_and_all_none_noops_keep_owner_at_full_quota_and_preserve_redo() {
    let q = quota();
    let (owner, _) = owned(&q, ColorMode::Rgb, SampleType::F32);
    let id = owner.document().layers[0].id;
    let zero = owner.try_edit(edit(id, None, None, Some(-0.0)), &ctl()).unwrap();
    assert_eq!(zero.document().layers[0].opacity.to_bits(), (-0.0f32).to_bits());
    let positive = zero.try_edit(edit(id, None, None, Some(0.0)), &ctl()).unwrap();
    assert!(!positive.shares_model(&zero));
    assert_eq!(positive.document().layers[0].opacity.to_bits(), 0f32.to_bits());
    let mut history = AccountedHistory::new(owner.clone(), 4, &ctl()).unwrap();
    history.try_commit(edit(id, Some("changed"), Some(false), Some(-0.0)), &ctl()).unwrap();
    assert!(history.undo(&ctl()).unwrap());
    assert!(history.can_redo());
    history.try_commit(edit(id, None, None, None), &ctl()).unwrap();
    assert!(history.can_redo());
    assert_eq!(history.len(), 2);
    assert!(history.redo(&ctl()).unwrap());
    let used = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - used.resident).unwrap();
    let before = q.usage().unwrap();
    for request in [edit(id, None, None, None), edit(id, Some("leaf0"), Some(true), Some(1.0))] {
        assert!(owner.try_edit(request, &ctl()).unwrap().shares_model(&owner));
    }
    assert!(zero.try_edit(edit(id, None, None, Some(-0.0)), &ctl()).unwrap().shares_model(&zero));
    assert!(owner.try_edit(edit(id, Some("allocation required"), None, None), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(blocker);
}
#[test]
fn malformed_prestate_bad_targets_and_unsupported_edits_refuse_even_when_requested_noop() {
    for fault in 0..4 {
        let q = quota();
        let (mut owner, _) = owned(&q, ColorMode::Rgb, SampleType::F32);
        let id = owner.document().layers[0].id;
        // Explicit test-only corruption of an already-paid CURRENT owner probes
        // validation-before-noop. It does not model an admitted source/adoption.
        let doc = &mut Arc::get_mut(&mut owner.owner).unwrap().document;
        match fault {
            0 => doc.xcf_state.as_mut().unwrap().items[0].name = "divergent".into(),
            1 => doc.xcf_state.as_mut().unwrap().items[0].visible = false,
            2 => doc.xcf_state.as_mut().unwrap().selection.active = Some(ItemRef::Layer(LayerId(999999))),
            _ => doc.xcf_state.as_mut().unwrap().current_filter_output_precision = FilterOutputPrecision::Half,
        }
        let before = q.usage().unwrap();
        assert!(owner.try_edit(edit(id, None, None, None), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let q = quota();
    let (owner, _) = owned(&q, ColorMode::Rgb, SampleType::F32);
    let id = owner.document().layers[0].id;
    let before = q.usage().unwrap();
    for request in [
        edit(LayerId(0), None, None, None),
        edit(LayerId(999999), None, None, None),
        edit(id, Some("bad\0name"), None, None),
        edit(id, None, None, Some(f32::NAN)),
        edit(id, None, None, Some(f32::INFINITY)),
        edit(id, None, None, Some(-0.001)),
        edit(id, None, None, Some(1.001)),
    ] {
        assert!(owner.validate_imported_metadata_edit(request, &ctl()).is_err());
        assert!(owner.try_edit(request, &ctl()).is_err());
    }
    for request in [
        AccountedEdit::LayerName { layer: id, name: "leaf0" },
        AccountedEdit::LayerVisibility { layer: id, visible: true },
        AccountedEdit::LayerOpacity { layer: id, opacity: 1.0 },
        AccountedEdit::MaskEnabled { layer: id, enabled: false },
        AccountedEdit::MaskLinked { layer: id, linked: false },
        AccountedEdit::NativePixels { layer: id, rect: Rect::new(-3, -2, -2, -1), bytes: &[0; 16] },
    ] {
        assert!(owner.try_edit(request, &ctl()).is_err());
        assert!(owner.validate_imported_metadata_edit(request, &ctl()).is_err());
    }
    assert!(owner.try_with_imported_layer_translation(id, 0, 0, &ctl()).is_err());
    assert!(owner.try_replace_selection(None, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let (doc, guard, _) = fixture(&q, ColorMode::Rgb, SampleType::F32);
    let legacy = AccountedDocument::from_owned(doc, guard, q.clone(), &ctl()).unwrap();
    assert!(prepare(&legacy, edit(legacy.document().layers[0].id, None, None, None), &ctl()).is_err());
}
#[test]
fn exact_empty_whitespace_unicode_names_and_late_two_string_cancellation_are_transactional() {
    let q = quota();
    let (owner, _) = owned(&q, ColorMode::Rgb, SampleType::F64);
    let id = owner.document().layers[1].id;
    for name in ["", "  ", " λ😀 "] {
        let changed = owner.try_edit(edit(id, Some(name), None, None), &ctl()).unwrap();
        assert_eq!(changed.document().layers[1].name, name);
        let item = changed.document().xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap();
        assert_eq!(item.name, name);
        assert!(item.custom_name);
    }
    let name = "λ".repeat(8192); // external harness input; 16KiB exact UTF8
    let before = q.usage().unwrap();
    let hits = AtomicU64::new(0);
    let work = |n| {
        // copy_name checks each two-byte lambda; the small old-model copy has
        // fewer than 1024 such checks. Cross the complete first replacement
        // plus that margin before cancelling while the second is private.
        if n == 2 && q.usage().map_err(|_| BackingError::Quota)?.index > before.index && hits.fetch_add(1, Ordering::Relaxed) >= 8192 + 1024 {
            Err(BackingError::Cancelled)
        } else {
            Ok(())
        }
    };
    assert!(owner.try_edit(edit(id, Some(&name), None, None), &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work }).is_err());
    assert!(hits.load(Ordering::Relaxed) >= 8192 + 1025);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().layers[1].name, "leaf1");
    let renamed = owner.try_edit(edit(id, Some(&name), None, None), &ctl()).unwrap();
    assert_eq!(renamed.document().layers[1].name, name);
    assert!(Arc::ptr_eq(&renamed.owner.retention, &owner.owner.retention));
}
#[test]
fn cumulative_work_budget_cannot_reset_between_full_validation_and_copy() {
    let q = quota();
    let (owner, _) = owned(&q, ColorMode::Rgb, SampleType::U16);
    let id = owner.document().layers[0].id;
    let before = q.usage().unwrap();
    let measured_work = AtomicU64::new(0);
    let count = |n| {
        measured_work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    owner
        .validate_imported_metadata_edit(
            edit(id, Some("new"), Some(false), Some(0.5)),
            &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &count },
        )
        .unwrap();
    let left = AtomicU64::new(measured_work.load(Ordering::Relaxed));
    let work = |n| left.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(
        owner.try_edit(edit(id, Some("new"), Some(false), Some(0.5)), &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work }).is_err()
    );
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().layers[0].name, "leaf0");
}
