//! Ignored UNRUN/uncompiled shared data-custody contracts. The inherited tree
//! is prospectively paid synthetic CURRENT data, not a Source/CMS/math oracle.
//! Other actual copier classes use the engine's genuine fixture matrix; these
//! groups focus on programme preservation and atomicity of the common Doc path.
use super::*;
use crate::accounted_document::ProgrammeNativeStorageKind;

fn programme(q: &BackingQuota, sample: SampleType, nonempty: bool, max_revision: bool) -> AccountedDocument {
    let (mut doc, hold) = fixture(q, sample);
    let state = doc.xcf_state.as_mut().unwrap();
    // Disabled programs remain stored and must not become a plain-tree grant.
    for node in &mut state.filter_stacks[0].nodes {
        node.enabled = false;
    }
    if max_revision {
        state.filter_stacks[0].revision = u64::MAX;
    }
    if nonempty {
        let value = if sample == SampleType::F64 { 0x3ff0000009b25010u64.to_ne_bytes().to_vec() } else { 0.375f32.to_ne_bytes().to_vec() };
        state.filter_masks[1].surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap().bytes_mut()[..value.len()].copy_from_slice(&value);
    }
    AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(doc, hold, q.clone(), &ctl()).unwrap()
}
fn native(owner: &AccountedDocument, id: LayerId, x: i32, y: i32) -> Vec<u8> {
    let target = owner.checked_programme_native_target(id, &ctl()).unwrap();
    let mut out = vec![0; target.backed().format().bytes_per_pixel()];
    target.backed().checked_view().read_logical_pixel_native_into(x, y, &mut out, &ctl()).unwrap();
    out
}
fn changed_pixel(sample: SampleType) -> Vec<u8> {
    match sample {
        SampleType::F32 => [0.375f32, -0., f32::from_bits(1), -0.].into_iter().flat_map(f32::to_ne_bytes).collect(),
        SampleType::F64 => [0.375 + 2f64.powi(-44), -0., f64::from_bits(1), -0.].into_iter().flat_map(f64::to_ne_bytes).collect(),
        _ => panic!("fixture supports the existing float programme domain"),
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: actual nonfirst nested target, inactive programme and mask/default/source custody through common same-class COW"]
fn programmes_survive_native_region_changes_without_role_conversion() {
    for sample in [SampleType::F32, SampleType::F64] {
        for nonempty in [false, true] {
            let q = quota();
            let owner = programme(&q, sample, nonempty, false);
            let before = owner.checked_programme_native_target(LayerId(2), &ctl()).unwrap();
            assert_eq!(before.storage_kind(), ProgrammeNativeStorageKind::NormalHierarchy);
            assert_eq!((before.layer_id(), before.ordinal(), before.parent(), before.canonical_item_index()), (LayerId(2), 3, Some((2, LayerId(11))), 3));
            let descriptor = before.descriptor();
            assert_eq!(before.backed().descriptor(), descriptor);
            drop(before);
            let bytes = changed_pixel(sample);
            let rect = Rect::new(1, 2, 2, 3);
            let original = native(&owner, LayerId(2), 1, 2);
            let edit = AccountedEdit::NativePixels { layer: LayerId(2), rect, bytes: &bytes };
            // Named new structural route, not a relaxation of either old route.
            assert!(owner.try_edit_readonly_normal_hierarchy_native(edit, &ctl()).is_err());
            assert!(owner.validate_imported_native_edit(edit, &ctl()).is_err());
            assert!(owner.try_edit(edit, &ctl()).is_err());
            let changed = owner.try_edit_programme_preserving_native(edit, &ctl()).unwrap();
            assert!(changed.is_readonly_normal_hierarchy_storage());
            assert!(!changed.shares_model(&owner));
            assert_eq!(native(&changed, LayerId(2), 1, 2), bytes);
            assert_eq!(native(&owner, LayerId(2), 1, 2), original);
            assert_eq!(native(&changed, LayerId(2), 2, 2), native(&owner, LayerId(2), 2, 2));
            assert_eq!(changed.document().xcf_state, owner.document().xcf_state);
            assert_eq!(changed.document().source_color_context, owner.document().source_color_context);
            assert_eq!(changed.document().working_profile_interpretation, owner.document().working_profile_interpretation);
            assert!(Arc::ptr_eq(changed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
            let target = changed.checked_programme_native_target(LayerId(2), &ctl()).unwrap();
            assert_eq!(target.descriptor(), descriptor);
            assert_eq!(target.backed().default_bytes(), descriptor.default_bytes());
            drop(target);
            let a = owner.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
            let b = changed.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
            for (a, b) in a.rows().iter().zip(b.rows()) {
                assert_eq!((a.id, a.parent, a.canonical_item_index, a.operative_bounds), (b.id, b.parent, b.canonical_item_index, b.operative_bounds));
                assert_eq!(
                    (&a.layer.name, a.layer.visible, a.layer.opacity.to_bits(), a.layer.locks),
                    (&b.layer.name, b.layer.visible, b.layer.opacity.to_bits(), b.layer.locks)
                );
                if a.id != LayerId(2) && !a.layer.is_group() {
                    assert_eq!(a.layer.content, b.layer.content);
                }
            }
            drop(a);
            drop(b);
            // Existing title/copy continuation must retain the actual COW and all programmes.
            let titled = changed.try_with_document_name("programme painted", &ctl()).unwrap();
            drop(changed);
            drop(owner);
            titled.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
            assert_eq!(native(&titled, LayerId(2), 1, 2), bytes);
            assert_eq!(titled.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: actual group/foreign/invalid-region and wrong edit domains are not class-marker authority"]
fn common_target_and_native_payload_refusals_preserve_owner() {
    let q = quota();
    let owner = programme(&q, SampleType::F64, true, false);
    let bytes = changed_pixel(SampleType::F64);
    let original = native(&owner, LayerId(2), 0, 0); // warm actual original cache before accounting baseline
    let baseline = q.usage().unwrap();
    for edit in [
        AccountedEdit::NativePixels { layer: LayerId(10), rect: Rect::new(0, 0, 1, 1), bytes: &bytes },
        AccountedEdit::NativePixels { layer: LayerId(12), rect: Rect::new(0, 0, 1, 1), bytes: &bytes },
        AccountedEdit::NativePixels { layer: LayerId(u64::MAX), rect: Rect::new(0, 0, 1, 1), bytes: &bytes },
        AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::EMPTY, bytes: &[] },
        AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::new(-1, 0, 0, 1), bytes: &bytes },
        AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::new(0, 0, 1, 1), bytes: &bytes[..31] },
        AccountedEdit::LayerName { layer: LayerId(2), name: "not a raw patch" },
    ] {
        assert!(owner.validate_programme_preserving_native_edit(edit, &ctl()).is_err());
        assert!(owner.try_edit_programme_preserving_native(edit, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    assert!(owner.checked_programme_native_target(LayerId(10), &ctl()).is_err());
    assert_eq!(native(&owner, LayerId(2), 0, 0), original);
    // A Legacy model, even backed, does not enroll via the new public enum.
    let held = Arc::new(Hold { _index: q.reserve(QuotaKind::Index, 4096).unwrap(), _resident: q.reserve(QuotaKind::Resident, 4096).unwrap() });
    let mut legacy_doc = Document::new("unrelated", Size::new(8, 6), ColorMode::Rgb, SampleType::F64);
    let target = owner.checked_programme_native_target(LayerId(2), &ctl()).unwrap();
    let legacy_layer = Layer::new("real paid native", LayerContent::BackedRaster(target.backed().clone()));
    let legacy_id = legacy_layer.id;
    legacy_doc.layers.push(legacy_layer);
    drop(target);
    let legacy = AccountedDocument::from_owned(legacy_doc, held, q.clone(), &ctl()).unwrap();
    assert!(legacy.checked_programme_native_target(legacy_id, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: exact raw noop/signedzero distinction with MAX programme revisions and independently paid fixed target scratch"]
fn raw_noop_never_changes_programme_revisions_or_borrows_copy_slack() {
    let q = quota();
    let owner = programme(&q, SampleType::F64, true, true);
    let zero = native(&owner, LayerId(2), 0, 0);
    let rect = Rect::new(0, 0, 1, 1);
    let noop = AccountedEdit::NativePixels { layer: LayerId(2), rect, bytes: &zero };
    let baseline = q.usage().unwrap();
    let same = owner.try_edit_programme_preserving_native(noop, &ctl()).unwrap();
    assert!(same.shares_model(&owner));
    drop(same);
    assert_eq!(q.usage().unwrap(), baseline);
    let block = q.reserve(QuotaKind::Resident, (128u64 << 20) - baseline.resident).unwrap();
    assert_eq!(owner.try_edit_programme_preserving_native(noop, &ctl()).err(), Some(BackingError::Quota));
    drop(block);
    assert_eq!(q.usage().unwrap(), baseline);
    let mut signed = zero.clone(); // external expected/input harness bytes
    signed[..8].copy_from_slice(&(-0f64).to_ne_bytes());
    let changed = owner.try_edit_programme_preserving_native(AccountedEdit::NativePixels { layer: LayerId(2), rect, bytes: &signed }, &ctl()).unwrap();
    assert!(!changed.shares_model(&owner));
    assert_eq!(native(&changed, LayerId(2), 0, 0), signed);
    assert_eq!(changed.document().xcf_state, owner.document().xcf_state);
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, u64::MAX);
}
#[test]
#[ignore = "UNRUN/uncompiled: cumulative original Work, full Index and physical COW cancellation leave unpublished copies refunded"]
fn changed_programme_copy_obeys_original_control_and_error_lifetimes() {
    let q = quota();
    let owner = programme(&q, SampleType::F64, true, false);
    let original = native(&owner, LayerId(2), 0, 0);
    let bytes = changed_pixel(SampleType::F64);
    let edit = AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::new(0, 0, 1, 1), bytes: &bytes };
    let baseline = q.usage().unwrap();
    let block = q.reserve(QuotaKind::Index, (64u64 << 20) - baseline.index).unwrap();
    assert!(owner.try_edit_programme_preserving_native(edit, &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), baseline);
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = owner.try_edit_programme_preserving_native(edit, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(changed);
    assert_eq!(q.usage().unwrap(), baseline);
    let left = AtomicU64::new(used.load(Ordering::Relaxed) - 1);
    let work = |n| left.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_edit_programme_preserving_native(edit, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    // This class also prospectively holds twice the actual retained mask/profile
    // payload while copying. Distinguish that envelope from real native COW.
    let copy_payload = measured_owner(&owner, &ctl()).unwrap().payload.checked_mul(2).unwrap();
    let stop = || q.usage().is_ok_and(|u| u.resident > baseline.resident + copy_payload + 65536);
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(owner.try_edit_programme_preserving_native(edit, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(native(&owner, LayerId(2), 0, 0), original);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
}
