//! UNRUN structural metadata drafts. Synthetic unqualified operations and opaque
//! assigned ICC fixtures prove custody only, not Box/source/CMS eligibility.
use super::*;
use photocraft_raster::Interrupt;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owners() -> [AccountedDocument; 2] {
    [super::super::tests::owned(crate::SampleType::F64, crate::SampleEncoding::NonlinearProfile, 750), super::super::assigned::tests::owner()]
}
fn edit<'a>(owner: &AccountedDocument, name: Option<&'a str>, visible: Option<bool>, opacity: Option<f32>) -> AccountedEdit<'a> {
    AccountedEdit::LayerMetadata { layer: owner.document().layers[0].id, name, visible, opacity }
}
fn same_payloads(a: &AccountedDocument, b: &AccountedDocument) {
    assert_eq!(a.is_readonly_assigned_point_storage(), b.is_readonly_assigned_point_storage());
    assert_eq!(a.is_readonly_point_storage(), b.is_readonly_point_storage());
    assert_eq!(a.document().layers[0].content, b.document().layers[0].content);
    assert!(Arc::ptr_eq(a.document().icc_profile.as_ref().unwrap(), b.document().icc_profile.as_ref().unwrap()));
    assert_eq!(a.document().source_color_context, b.document().source_color_context);
    if let Some(original) = a.document().source_color_context.as_ref().unwrap().original_icc.as_ref() {
        assert!(Arc::ptr_eq(original, b.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()));
    }
    let sa = a.document().xcf_state.as_ref().unwrap();
    let sb = b.document().xcf_state.as_ref().unwrap();
    assert_eq!(sa.filter_stacks, sb.filter_stacks);
    assert_eq!(sa.provenance, sb.provenance);
    assert_eq!(sa.opaque, sb.opaque);
    for (ma, mb) in sa.filter_masks.iter().zip(&sb.filter_masks) {
        assert_eq!((ma.id, ma.owner, ma.bounds), (mb.id, mb.owner, mb.bounds));
        assert_eq!(ma.surface.default_bytes(), mb.surface.default_bytes());
        for ((ca, ta), (cb, tb)) in ma.surface.tiles().zip(mb.surface.tiles()) {
            assert_eq!(ca, cb);
            assert!(Arc::ptr_eq(ta, tb));
        }
    }
    assert!(a.quota().shares_ledger(b.quota()));
}
#[test]
#[ignore = "UNRUN both runtime classes compound copy and raw custody"]
fn combined_fields_change_only_target_and_canonical_fields_without_filter_revision() {
    for old in owners() {
        let next = old.try_edit(edit(&old, Some("  名 metadata  "), Some(false), Some(-0.0)), &ctl()).unwrap();
        same_payloads(&old, &next);
        assert!(!next.shares_model(&old));
        assert_eq!(next.document().layers[0].name, "  名 metadata  ");
        assert!(!next.document().layers[0].visible);
        assert_eq!(next.document().layers[0].opacity.to_bits(), (-0.0f32).to_bits());
        let item = next.document().xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(old.document().layers[0].id)).unwrap();
        assert_eq!(item.name, "  名 metadata  ");
        assert!(item.custom_name);
        assert!(!item.visible);
        assert_eq!(old.document().layers[0].name, "original layer");
        if old.is_readonly_assigned_point_storage() {
            assert!(
                old.try_with_readonly_layer_properties(
                    ReadonlyLayerPropertiesPatch { layer: old.document().layers[0].id, name: Some("old public route"), visible: None, opacity_f32_bits: None },
                    &ctl()
                )
                .is_err()
            );
        }
    }
}
#[test]
#[ignore = "UNRUN full Index no-op, signed-zero change and generic closure"]
fn noop_shares_at_full_quota_but_changes_and_noncompound_routes_refuse() {
    for old in owners() {
        let q = old.quota().clone();
        let before = q.usage().unwrap();
        let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
        let full = q.usage().unwrap();
        assert!(old.try_edit(edit(&old, None, None, None), &ctl()).unwrap().shares_model(&old));
        assert!(old.try_edit(edit(&old, Some("replacement"), None, None), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), full);
        drop(blocker);
        let zero = old.try_edit(edit(&old, None, None, Some(-0.0)), &ctl()).unwrap();
        let plus = zero.try_edit(edit(&zero, None, None, Some(0.0)), &ctl()).unwrap();
        assert!(!plus.shares_model(&zero));
        assert_eq!(plus.document().layers[0].opacity.to_bits(), 0);
        let id = old.document().layers[0].id;
        for unsupported in [
            AccountedEdit::LayerVisibility { layer: id, visible: true },
            AccountedEdit::LayerOpacity { layer: id, opacity: 1.0 },
            AccountedEdit::LayerName { layer: id, name: "original layer" },
            AccountedEdit::MaskLinked { layer: id, linked: true },
        ] {
            assert!(old.try_edit(unsupported, &ctl()).is_err());
            assert!(old.validate_imported_metadata_edit(unsupported, &ctl()).is_err());
        }
        assert!(old.try_replace_selection(None, &ctl()).is_err());
    }
}
// Explicit native-current malformed fixture. Supplemental actual same-q fees
// precede the external harness clone; retained original pays all shared payload.
fn malformed(old: AccountedDocument, case: usize) -> AccountedDocument {
    let q = old.quota().clone();
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    let mut doc = old.document().clone();
    let layer_id = doc.layers[0].id;
    match case {
        0 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer_id)).unwrap().name = "divergent".into(),
        1 => doc.layers[0].visible = !doc.layers[0].visible,
        2 => doc.layers[0].opacity = f32::NAN,
        3 => doc.layers[0].opacity = 1.1,
        _ => {
            let state = doc.xcf_state.as_mut().unwrap();
            state.filter_stacks[0].nodes.clear();
            state.filter_masks.clear();
            state.items.retain(|item| matches!(item.item, ItemRef::Layer(_)));
        }
    }
    let assigned = old.is_readonly_assigned_point_storage();
    let retained: Arc<dyn Any + Send + Sync> = Arc::new((old, index, resident));
    if assigned {
        AccountedDocument::new_readonly_assigned_point_storage(doc, retained, q, &ctl()).unwrap()
    } else {
        AccountedDocument::new_readonly_point_storage(doc, retained, q, &ctl()).unwrap()
    }
}
#[test]
#[ignore = "UNRUN malformed canonical/current opacity and missing target prestate"]
fn malformed_current_refuses_all_none_equal_and_repair_before_copy() {
    for assigned in [false, true] {
        for case in 0..5 {
            let old = if assigned {
                super::super::assigned::tests::owner()
            } else {
                super::super::tests::owned(crate::SampleType::F64, crate::SampleEncoding::NonlinearProfile, 750)
            };
            let bad = malformed(old, case);
            let before = bad.quota().usage().unwrap();
            for request in [
                edit(&bad, None, None, None),
                edit(&bad, Some("original layer"), Some(true), Some(1.0)),
                edit(&bad, None, None, Some(bad.document().layers[0].opacity)),
            ] {
                assert!(bad.validate_imported_metadata_edit(request, &ctl()).is_err());
                assert!(bad.try_edit(request, &ctl()).is_err());
            }
            assert_eq!(bad.quota().usage().unwrap(), before);
        }
    }
}
#[test]
#[ignore = "UNRUN history redo preservation and flat original retention"]
fn compound_history_noop_preserves_redo_and_repeated_copies_drop_intermediate_models() {
    for old in owners() {
        let mut history = AccountedHistory::new(old.clone(), 8, &ctl()).unwrap();
        history.try_commit(edit(&old, Some("first"), None, None), &ctl()).unwrap();
        history.undo(&ctl()).unwrap();
        history.try_commit(edit(&old, None, None, None), &ctl()).unwrap();
        assert!(history.can_redo());
        assert!(history.current().unwrap().shares_model(&old));
        drop(history);
        let first = old.try_edit(edit(&old, Some("first"), None, None), &ctl()).unwrap();
        let weak_first = Arc::downgrade(&first.owner);
        let second = first.try_edit(edit(&first, Some("second"), Some(false), Some(0.5)), &ctl()).unwrap();
        let root = second.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
        assert!(root.original.shares_model(&old));
        drop(first);
        assert!(weak_first.upgrade().is_none());
        same_payloads(&old, &second);
    }
}
#[test]
#[ignore = "UNRUN work, cancellation and invalid scalar rollback"]
fn failures_leave_quota_and_original_model_unchanged() {
    for old in owners() {
        let q = old.quota().clone();
        let before = q.usage().unwrap();
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(old.try_edit(edit(&old, Some("changed"), None, None), &deny).is_err());
        let allocated = || q.usage().unwrap().index > before.index;
        let stop = Interrupt::cancel_only(&allocated);
        assert!(old.try_edit(edit(&old, Some("changed"), Some(false), Some(0.25)), &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
        for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(old.try_edit(edit(&old, None, None, Some(opacity)), &ctl()).is_err());
        }
        let wrong = AccountedEdit::LayerMetadata { layer: LayerId(u64::MAX), name: None, visible: None, opacity: None };
        assert!(old.try_edit(wrong, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(old.document().layers[0].name, "original layer");
    }
}
