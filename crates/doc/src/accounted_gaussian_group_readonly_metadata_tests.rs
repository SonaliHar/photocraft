//! UNRUN CURRENT metadata custody only. These are prepaid synthetic native
//! fixtures, not GIMP source/render witnesses. Expected clones and input Strings
//! are external harness allocations, never assertions of production quota use.
use super::super::tests::{ctl, fixture, owned, quota};
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
fn ids(owner: &AccountedDocument) -> [LayerId; 4] {
    let doc = owner.document();
    let LayerContent::Group(group) = &doc.layers[1].content else { panic!("fixture") };
    [doc.layers[0].id, doc.layers[1].id, group.children[0].id, group.children[1].id]
}
fn row(doc: &Document, id: LayerId) -> &crate::xcf_state::ItemState {
    doc.xcf_state.as_ref().unwrap().items.iter().find(|v| v.item == ItemRef::Layer(id)).unwrap()
}
#[test]
fn combined_metadata_all_four_ids_preserves_raw_owners_filter_revision_and_flat_retention() {
    for edited in [false, true] {
        let q = quota();
        let baseline = q.usage().unwrap();
        let original = owned(&q, edited, true);
        let targets = ids(&original);
        let mut current = original.clone();
        let mut expected = original.document().clone();
        let names = ["base renamed", "group renamed", "multiply renamed", "normal renamed"];
        for (id, name) in targets.into_iter().zip(names) {
            let edit = AccountedEdit::LayerMetadata { layer: id, name: Some(name), visible: Some(false), opacity: Some(-0.0) };
            current.validate_imported_metadata_edit(edit, &ctl()).unwrap();
            current = current.try_edit(edit, &ctl()).unwrap();
            let target = find_layer_mut(&mut expected.layers, id, 0, &ctl()).unwrap().unwrap();
            target.name = name.into();
            target.visible = false;
            target.opacity = -0.0;
            let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap();
            item.name = name.into();
            item.custom_name = true;
            item.visible = false;
            assert_eq!(current.document(), &expected);
            current.validate_gaussian_group_storage(&ctl()).unwrap();
            assert_eq!(accounted_layer_access(current.document(), id, &ctl()).unwrap().unwrap().layer.opacity.to_bits(), (-0f32).to_bits());
        }
        let titled = current.try_with_document_name("metadata.pcraft", &ctl()).unwrap();
        expected.name = "metadata.pcraft".into();
        assert_eq!(titled.document(), &expected);
        assert!(titled.owner.retention.downcast_ref::<OriginalRetention>().unwrap().original.shares_model(&original));
        assert!(Arc::ptr_eq(&original.document().layers[1].mask.as_ref().unwrap().surface, &titled.document().layers[1].mask.as_ref().unwrap().surface));
        assert!(Arc::ptr_eq(original.document().icc_profile.as_ref().unwrap(), titled.document().icc_profile.as_ref().unwrap()));
        let a = &original.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        let b = &titled.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        for ((ca, ta), (cb, tb)) in a.tiles().zip(b.tiles()) {
            assert_eq!(ca, cb);
            assert!(Arc::ptr_eq(ta, tb));
        }
        assert_eq!(original.document().xcf_state.as_ref().unwrap().filter_stacks, titled.document().xcf_state.as_ref().unwrap().filter_stacks);
        drop(expected);
        drop(current);
        drop(original);
        drop(titled);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
fn physical_mask_toggle_excludes_saved_filter_mask_with_same_owner_and_preserves_every_sample() {
    let q = quota();
    let original = owned(&q, false, true);
    let group = ids(&original)[1];
    let filter = original.document().xcf_state.as_ref().unwrap().filter_masks[0].id;
    let physical = physical_index(original.document(), group, &ctl()).unwrap();
    assert_ne!(original.document().xcf_state.as_ref().unwrap().items[physical].item, ItemRef::Mask(filter));
    let disabled_edit = AccountedEdit::MaskEnabled { layer: group, enabled: false };
    original.validate_assigned_analytic_mask_edit(disabled_edit, &ctl()).unwrap();
    let disabled = original.try_edit(disabled_edit, &ctl()).unwrap();
    let mut expected = original.document().clone();
    expected.layers[1].mask.as_mut().unwrap().enabled = false;
    expected.xcf_state.as_mut().unwrap().items[physical].mask.as_mut().unwrap().enabled = false;
    assert_eq!(disabled.document(), &expected);
    assert!(disabled.document().xcf_state.as_ref().unwrap().items.iter().find(|v| v.item == ItemRef::Mask(filter)).unwrap().mask.as_ref().unwrap().enabled);
    assert!(Arc::ptr_eq(&original.document().layers[1].mask.as_ref().unwrap().surface, &disabled.document().layers[1].mask.as_ref().unwrap().surface));
    assert!(disabled.try_edit(disabled_edit, &ctl()).unwrap().shares_model(&disabled));
    let enabled = disabled.try_edit(AccountedEdit::MaskEnabled { layer: group, enabled: true }, &ctl()).unwrap();
    assert_eq!(enabled.document(), original.document());
    for id in [ids(&original)[0], ids(&original)[2], ids(&original)[3], LayerId(0)] {
        assert!(original.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &ctl()).is_err());
    }
    assert_eq!(enabled.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 17);
}
#[test]
fn exact_name_custom_flag_raw_zero_noops_and_history_redo_survive_without_allocations() {
    let q = quota();
    let original = owned(&q, false, true);
    let id = ids(&original)[2];
    let group = ids(&original)[1];
    let same = AccountedEdit::LayerMetadata { layer: id, name: Some("multiply"), visible: Some(true), opacity: None };
    assert!(original.try_edit(same, &ctl()).unwrap().shares_model(&original));
    assert!(!row(original.document(), id).custom_name);
    let zero = original.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: Some(-0.0) }, &ctl()).unwrap();
    let plus = zero.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: Some(0.0) }, &ctl()).unwrap();
    assert!(!plus.shares_model(&zero));
    assert_eq!(accounted_layer_access(plus.document(), id, &ctl()).unwrap().unwrap().layer.opacity.to_bits(), 0f32.to_bits());
    let mut history = AccountedHistory::new(original.clone(), 4, &ctl()).unwrap();
    history.try_commit(AccountedEdit::LayerMetadata { layer: id, name: Some("new"), visible: None, opacity: None }, &ctl()).unwrap();
    assert!(history.undo(&ctl()).unwrap());
    assert!(history.can_redo());
    let used = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (128 << 20) - used.resident).unwrap();
    let full = q.usage().unwrap();
    history.try_commit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: None }, &ctl()).unwrap();
    history.try_commit(AccountedEdit::MaskEnabled { layer: group, enabled: true }, &ctl()).unwrap();
    assert!(history.can_redo());
    assert_eq!(history.len(), 2);
    assert_eq!(q.usage().unwrap(), full);
    assert!(history.try_commit(AccountedEdit::LayerMetadata { layer: id, name: Some("different"), visible: None, opacity: None }, &ctl()).is_err());
    assert!(history.can_redo());
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
}
#[test]
fn malformed_canonical_prestate_and_unsupported_variants_refuse_before_noop_or_copy() {
    let q = quota();
    let (mut doc, guard) = fixture(&q, false, true);
    doc.xcf_state.as_mut().unwrap().items[0].name = "divergent canonical".into();
    let malformed = AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q.clone(), &ctl()).unwrap();
    let target = ids(&malformed)[0];
    let group = ids(&malformed)[1];
    let before = q.usage().unwrap();
    for edit in
        [AccountedEdit::LayerMetadata { layer: target, name: None, visible: None, opacity: None }, AccountedEdit::MaskEnabled { layer: group, enabled: true }]
    {
        assert!(malformed.try_edit(edit, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let valid = owned(&q, false, true);
    let id = ids(&valid)[1];
    for edit in [
        AccountedEdit::LayerName { layer: id, name: "new" },
        AccountedEdit::LayerVisibility { layer: id, visible: false },
        AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 },
        AccountedEdit::MaskLinked { layer: id, linked: false },
        AccountedEdit::NativePixels { layer: id, rect: Rect::from_xywh(0, 0, 1, 1), bytes: &[0; 16] },
    ] {
        assert!(valid.try_edit(edit, &ctl()).is_err());
    }
    assert!(valid.try_replace_selection(None, &ctl()).is_err());
    for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(valid.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: Some(opacity) }, &ctl()).is_err());
    }
    assert!(valid.try_edit(AccountedEdit::LayerMetadata { layer: id, name: Some("bad\0name"), visible: None, opacity: None }, &ctl()).is_err());
}
#[test]
fn two_name_copy_and_cumulative_work_failures_keep_original_model_and_quota_atomic() {
    let q = quota();
    let original = owned(&q, true, true);
    let id = ids(&original)[3];
    let name = "longλ".repeat(16384);
    let edit = AccountedEdit::LayerMetadata { layer: id, name: Some(&name), visible: Some(false), opacity: Some(0.125) };
    let before = q.usage().unwrap();
    let count = AtomicU64::new(0);
    let work = |n| {
        count.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = original.try_edit(edit, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work }).unwrap();
    assert_eq!(row(changed.document(), id).name, name);
    assert!(row(changed.document(), id).custom_name);
    assert_eq!(accounted_layer_access(changed.document(), id, &ctl()).unwrap().unwrap().layer.name, name);
    let total = count.load(Ordering::Relaxed);
    assert!(total > name.len() as u64 * 2);
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    let remaining = AtomicU64::new(total - 1);
    let bounded = |n| remaining.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Quota);
    assert!(original.try_edit(edit, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &bounded }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let cancel = |_: u64| if q.usage().unwrap().index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(original.try_edit(edit, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &cancel }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(row(original.document(), id).name, "normal");
    assert_eq!(original.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 17);
}
