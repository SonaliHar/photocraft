//! UNRUN structural native-view drafts, not Box/source/profile/rendering proof.
//! Existing fixture preholds pay synthetic model metadata. Expected model clones
//! and comparison byte buffers are external harness data, not production peaks.
use super::*;
use photocraft_raster::Interrupt;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owner(assigned: bool) -> AccountedDocument {
    if assigned {
        super::super::assigned::tests::owner()
    } else {
        super::super::tests::owned(crate::SampleType::F64, crate::SampleEncoding::NonlinearProfile, 750)
    }
}
fn leaf(owner: &AccountedDocument) -> &crate::BackedLayer {
    let LayerContent::BackedRaster(v) = &owner.document().layers[0].content else { panic!("fixture") };
    v
}
fn raw(owner: &AccountedDocument, x: i32, y: i32) -> Vec<u8> {
    let mut bytes = vec![0; leaf(owner).format().bytes_per_pixel()];
    leaf(owner).checked_view().read_pixel_native_into(x, y, &mut bytes, &ctl()).unwrap();
    bytes
}
fn same_custody(a: &AccountedDocument, b: &AccountedDocument) {
    assert_eq!(a.is_readonly_point_storage(), b.is_readonly_point_storage());
    assert_eq!(a.is_readonly_assigned_point_storage(), b.is_readonly_assigned_point_storage());
    assert!(a.quota().shares_ledger(b.quota()));
    assert!(Arc::ptr_eq(a.document().icc_profile.as_ref().unwrap(), b.document().icc_profile.as_ref().unwrap()));
    assert_eq!(a.document().source_color_context, b.document().source_color_context);
    if let Some(icc) = a.document().source_color_context.as_ref().unwrap().original_icc.as_ref() {
        assert!(Arc::ptr_eq(icc, b.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()));
    }
    assert_eq!(leaf(a).format(), leaf(b).format());
    assert_eq!(leaf(a).default_bytes(), leaf(b).default_bytes());
    let sa = a.document().xcf_state.as_ref().unwrap();
    let sb = b.document().xcf_state.as_ref().unwrap();
    assert_eq!(sa.filter_stacks, sb.filter_stacks);
    assert_eq!(sa.filter_masks, sb.filter_masks);
    assert_eq!(sa.provenance, sb.provenance);
    assert_eq!(sa.opaque, sb.opaque);
    for (ma, mb) in sa.filter_masks.iter().zip(&sb.filter_masks) {
        assert_eq!(ma.bounds, a.document().bounds());
        assert_eq!(mb.bounds, b.document().bounds());
        for ((ca, ta), (cb, tb)) in ma.surface.tiles().zip(mb.surface.tiles()) {
            assert_eq!(ca, cb);
            assert!(Arc::ptr_eq(ta, tb));
        }
    }
    for item in sa.items.iter().filter(|item| matches!(item.item, ItemRef::Mask(_))) {
        assert_eq!(Some(item), sb.items.iter().find(|other| other.item == item.item));
    }
}
fn patterned(assigned: bool) -> AccountedDocument {
    let mut old = owner(assigned);
    let sample = old.document().depth;
    for (n, (x, y)) in [(-13, 13), (0, 14), (243, 141)].into_iter().enumerate() {
        let bytes: Vec<u8> = [0.125 + n as f64 * 0.25, -0.0, 1.125, 0.5]
            .into_iter()
            .flat_map(|v| if sample == crate::SampleType::F32 { (v as f32).to_ne_bytes().to_vec() } else { v.to_ne_bytes().to_vec() })
            .collect();
        let next = leaf(&old).try_edited_native(Rect::new(x, y, x + 1, y + 1), &bytes, &ctl()).unwrap();
        Arc::get_mut(&mut old.owner).unwrap().document.layers[0].content = LayerContent::BackedRaster(next);
    }
    old
}
#[test]
#[ignore = "UNRUN negative, unaligned and aligned coordinate views with fixed canvas effect mask"]
fn moves_both_classes_without_changing_raw_recipe_or_canvas_masks() {
    for assigned in [false, true] {
        let old = patterned(assigned);
        let id = old.document().layers[0].id;
        for (dx, dy) in [(-257, 259), (256, -256), (1, -1)] {
            old.validate_readonly_point_layer_translation(id, dx, dy, &ctl()).unwrap();
            let next = old.try_with_readonly_point_layer_translation(id, dx, dy, &ctl()).unwrap();
            same_custody(&old, &next);
            for (x, y) in [(-13, 13), (0, 14), (243, 141)] {
                assert_eq!(raw(&old, x, y), raw(&next, x + dx, y + dy));
            }
            let bounds = leaf(&next).operative_bounds();
            let exterior = (bounds.x0 - 1, bounds.y0);
            let pin = leaf(&next).checked_view().pin_tile(photocraft_geom::TileCoord::containing(exterior.0, exterior.1), &ctl()).unwrap();
            let mut padding = vec![0; leaf(&next).format().bytes_per_pixel()];
            pin.read_pixel_native_into(exterior.0, exterior.1, &mut padding, &ctl()).unwrap();
            assert_eq!(padding, leaf(&next).default_bytes());
            let mut expected = old.document().clone();
            expected.layers[0].content = next.document().layers[0].content.clone();
            expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().raster_bounds =
                Some(leaf(&next).operative_bounds());
            assert_eq!(next.document(), &expected);
        }
    }
}
#[test]
#[ignore = "UNRUN zero full gates, actual full Index quota and translated Point coordinate limits"]
fn zero_shares_after_full_gates_and_checked_targets_refuse_before_copy() {
    for assigned in [false, true] {
        let old = owner(assigned);
        let id = old.document().layers[0].id;
        let q = old.quota().clone();
        let before = q.usage().unwrap();
        let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
        let full = q.usage().unwrap();
        assert!(old.try_with_readonly_point_layer_translation(id, 0, 0, &ctl()).unwrap().shares_model(&old));
        assert!(old.try_with_readonly_point_layer_translation(id, 1, 0, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), full);
        drop(blocker);
        for (target, dx, dy) in [(LayerId(u64::MAX), 0, 0), (id, 1 << 29, 0), (id, -(1 << 29), 0), (id, i32::MAX, i32::MIN)] {
            assert!(old.validate_readonly_point_layer_translation(target, dx, dy, &ctl()).is_err());
            assert!(old.try_with_readonly_point_layer_translation(target, dx, dy, &ctl()).is_err());
        }
        for case in 0..5 {
            let mut bad = owner(assigned);
            let doc = &mut Arc::get_mut(&mut bad.owner).unwrap().document;
            let bad_id = doc.layers[0].id;
            match case {
                0 => doc.layers[0].link_group = Some(1),
                1 => doc.layers[0].opacity = f32::NAN,
                2 => doc.layers[0].visible = false,
                3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(bad_id)).unwrap().name = "divergent".into(),
                _ => {
                    let s = doc.xcf_state.as_mut().unwrap();
                    s.filter_stacks[0].nodes.clear();
                    s.filter_masks.clear();
                    s.items.retain(|item| matches!(item.item, ItemRef::Layer(_)));
                }
            }
            let bad_id = bad.document().layers[0].id;
            for dx in [0, 1] {
                assert!(bad.try_with_readonly_point_layer_translation(bad_id, dx, 0, &ctl()).is_err());
            }
        }
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
#[ignore = "UNRUN exact upstream inert reference arithmetic and zero-vector raw preservation"]
fn nonzero_move_adds_both_reference_axes_without_normalizing_words() {
    for assigned in [false, true] {
        for reference in [(-0.0, -0.0), (f64::from_bits(0x7ff8_0000_0000_0042), f64::INFINITY), (1.25, -2.5)] {
            let mut old = owner(assigned);
            Arc::get_mut(&mut old.owner).unwrap().document.layers[0].effects.reference = Some(reference);
            let id = old.document().layers[0].id;
            let zero = old.try_with_readonly_point_layer_translation(id, 0, 0, &ctl()).unwrap();
            assert!(zero.shares_model(&old));
            for (dx, dy) in [(1, 0), (0, -1)] {
                let next = old.try_with_readonly_point_layer_translation(id, dx, dy, &ctl()).unwrap();
                let actual = next.document().layers[0].effects.reference.unwrap();
                assert_eq!(actual.0.to_bits(), (reference.0 + f64::from(dx)).to_bits());
                assert_eq!(actual.1.to_bits(), (reference.1 + f64::from(dy)).to_bits());
                let unchanged = old.document().layers[0].effects.reference.unwrap();
                assert_eq!(unchanged.0.to_bits(), reference.0.to_bits());
                assert_eq!(unchanged.1.to_bits(), reference.1.to_bits());
            }
        }
    }
}
#[test]
#[ignore = "UNRUN repeated moves plus title/metadata preserve flat original codec custody"]
fn repeated_moves_do_not_retain_intermediate_models_or_mutate_filter_revision() {
    for assigned in [false, true] {
        let old = patterned(assigned);
        let id = old.document().layers[0].id;
        let first = old.try_with_readonly_point_layer_translation(id, -1, 1, &ctl()).unwrap();
        let weak = Arc::downgrade(&first.owner);
        let second = first.try_with_readonly_point_layer_translation(id, 2, -2, &ctl()).unwrap();
        drop(first);
        assert!(weak.upgrade().is_none());
        let named = second.try_with_document_name("moved.pcraft", &ctl()).unwrap();
        let next =
            named.try_edit(AccountedEdit::LayerMetadata { layer: id, name: Some("moved layer"), visible: Some(false), opacity: Some(-0.0) }, &ctl()).unwrap();
        let original = next.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
        assert!(original.original.shares_model(&old));
        assert_eq!(raw(&old, -13, 13), raw(&next, -12, 12));
        same_custody(&old, &next);
        assert!(next.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: true }, &ctl()).is_err());
        assert!(next.try_replace_selection(None, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN work and allocation-stage cancellation rollback without source mutation"]
fn work_or_cancel_errors_refund_private_copy_and_preserve_original() {
    for assigned in [false, true] {
        let old = owner(assigned);
        let id = old.document().layers[0].id;
        let q = old.quota().clone();
        let before = q.usage().unwrap();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(old.try_with_readonly_point_layer_translation(id, 0, 0, &denied).is_err());
        assert!(old.try_with_readonly_point_layer_translation(id, -1, 1, &denied).is_err());
        let allocated = || q.usage().unwrap().index > before.index;
        let stop = Interrupt::cancel_only(&allocated);
        assert!(old.try_with_readonly_point_layer_translation(id, -1, 1, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(leaf(&old).operative_bounds(), Rect::new(-13, 13, 244, 142));
    }
}
