//! UNRUN current-group metadata custody only. The prepaid synthetic fixture has
//! no source/CMS/render authority. Expected model clones are external oracles.
use super::tests::fixture;
use super::*;
use crate::Layer;
use photocraft_raster::Interrupt;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn layer(d: &Document, at: usize) -> &Layer {
    if at < 2 {
        &d.layers[at]
    } else {
        let LayerContent::Group(g) = &d.layers[1].content else { panic!() };
        &g.children[at - 2]
    }
}
fn layer_mut(d: &mut Document, at: usize) -> &mut Layer {
    if at < 2 {
        &mut d.layers[at]
    } else {
        let LayerContent::Group(g) = &mut d.layers[1].content else { panic!() };
        &mut g.children[at - 2]
    }
}
fn metadata(id: crate::LayerId, name: Option<&str>, visible: Option<bool>, opacity: Option<f32>) -> AccountedEdit<'_> {
    AccountedEdit::LayerMetadata { layer: id, name, visible, opacity }
}
fn payloads(a: &AccountedDocument, b: &AccountedDocument) {
    assert!(a.quota().shares_ledger(b.quota()));
    assert!(Arc::ptr_eq(&a.owner.retention, &b.owner.retention));
    for at in [0, 2, 3] {
        assert_eq!(layer(a.document(), at).content, layer(b.document(), at).content);
    }
    assert!(matches!(layer(b.document(), 1).content, LayerContent::Group(_)));
    if let Some(mask) = &a.document().layers[1].mask {
        assert!(Arc::ptr_eq(&mask.surface, &b.document().layers[1].mask.as_ref().unwrap().surface));
    }
    assert!(Arc::ptr_eq(a.document().icc_profile.as_ref().unwrap(), b.document().icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        a.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        b.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
}
#[test]
fn all_four_ids_compound_metadata_preserve_group_children_native_masks_and_profile_owners() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for masked in [false, true] {
                for at in 0..4 {
                    let source = fixture(mode, sample, masked, true, false);
                    let id = layer(source.document(), at).id;
                    let edit = metadata(id, Some(" renamed μ "), Some(false), Some(-0.0));
                    source.validate_imported_metadata_edit(edit, &ctl()).unwrap();
                    let result = source.try_edit(edit, &ctl()).unwrap();
                    result.validate_assigned_isolated_normal_group_storage(&ctl()).unwrap();
                    payloads(&source, &result);
                    let mut expected = source.document().clone();
                    let l = layer_mut(&mut expected, at);
                    l.name = " renamed μ ".into();
                    l.visible = false;
                    l.opacity = -0.0;
                    let record = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap();
                    record.name = " renamed μ ".into();
                    record.custom_name = true;
                    record.visible = false;
                    assert_eq!(result.document(), &expected);
                    assert_eq!(layer(result.document(), at).opacity.to_bits(), (-0.0f32).to_bits());
                    let titled = result.try_with_document_name("continued.pcraft", &ctl()).unwrap();
                    payloads(&source, &titled);
                    assert_eq!(titled.document().layers, result.document().layers);
                }
            }
        }
    }
}
#[test]
fn optional_group_mask_only_syncs_its_actual_flags_and_keeps_all_raw_owners() {
    for masked in [false, true] {
        let source = fixture(ColorMode::Rgb, SampleType::F64, masked, false, true);
        let id = layer(source.document(), 1).id;
        let edit = AccountedEdit::MaskEnabled { layer: id, enabled: false };
        if !masked {
            assert!(source.validate_assigned_analytic_mask_edit(edit, &ctl()).is_err());
            assert!(source.try_edit(edit, &ctl()).is_err());
            continue;
        }
        source.validate_assigned_analytic_mask_edit(edit, &ctl()).unwrap();
        let result = source.try_edit(edit, &ctl()).unwrap();
        payloads(&source, &result);
        let mut expected = source.document().clone();
        expected.layers[1].mask.as_mut().unwrap().enabled = false;
        expected.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap().enabled = false;
        assert_eq!(result.document(), &expected);
        result.validate_assigned_isolated_normal_group_storage(&ctl()).unwrap();
        for at in [0, 2, 3] {
            let bad = AccountedEdit::MaskEnabled { layer: layer(source.document(), at).id, enabled: false };
            assert!(source.validate_assigned_analytic_mask_edit(bad, &ctl()).is_err());
            assert!(source.try_edit(bad, &ctl()).is_err());
        }
        let enabled = result.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: true }, &ctl()).unwrap();
        assert_eq!(enabled.document(), source.document());
        assert!(source.try_edit(AccountedEdit::MaskLinked { layer: id, linked: false }, &ctl()).is_err());
        assert!(source.try_replace_selection(None, &ctl()).is_err());
    }
}
#[test]
fn all_none_raw_bit_noops_and_same_name_keep_one_undo_redo_and_custom_flag() {
    let source = fixture(ColorMode::Grayscale, SampleType::F32, true, true, true);
    let id = layer(source.document(), 2).id;
    let mut history = AccountedHistory::new(source.clone(), 4, &ctl()).unwrap();
    history.try_commit(metadata(id, Some(&layer(source.document(), 2).name), Some(false), Some(-0.0)), &ctl()).unwrap();
    let changed = history.current().unwrap().clone();
    assert!(history.can_undo());
    assert!(!changed.document().xcf_state.as_ref().unwrap().items.iter().find(|v| v.item == ItemRef::Layer(id)).unwrap().custom_name);
    history.undo(&ctl()).unwrap();
    assert!(history.current().unwrap().shares_model(&source));
    history.try_commit(metadata(id, None, None, None), &ctl()).unwrap();
    assert!(history.can_redo());
    history.redo(&ctl()).unwrap();
    assert!(history.current().unwrap().shares_model(&changed));
    assert!(changed.try_edit(metadata(id, None, None, Some(-0.0)), &ctl()).unwrap().shares_model(&changed));
    let positive = changed.try_edit(metadata(id, None, None, Some(0.0)), &ctl()).unwrap();
    assert!(!positive.shares_model(&changed));
    assert_eq!(layer(positive.document(), 2).opacity.to_bits(), 0.0f32.to_bits());
    payloads(&source, &positive);
}
#[test]
fn malformed_canonical_and_invalid_requests_refuse_while_late_copy_denial_refunds() {
    let source = fixture(ColorMode::Rgb, SampleType::U16, true, true, true);
    let id = layer(source.document(), 3).id;
    let q = source.quota();
    let before = q.usage().unwrap();
    for e in [
        metadata(id, Some("bad\0name"), None, None),
        metadata(id, None, None, Some(f32::NAN)),
        metadata(crate::LayerId(u64::MAX), None, Some(false), None),
        AccountedEdit::LayerVisibility { layer: id, visible: false },
        AccountedEdit::LayerName { layer: id, name: "closed" },
    ] {
        assert!(source.try_edit(e, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(source.try_edit(metadata(id, None, None, None), &ctl()).unwrap().shares_model(&source));
    assert!(source.try_edit(metadata(id, Some("changed"), Some(false), Some(0.5)), &ctl()).is_err());
    drop(blocker);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(source.try_edit(metadata(id, None, Some(false), None), &deny).is_err());
    let late = || q.usage().unwrap().index > before.index;
    let token = Interrupt::cancel_only(&late);
    assert!(source.try_edit(metadata(id, Some("changed"), Some(false), None), &ReadControl { interrupt: &token, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut d = source.document().clone();
    d.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap().name = "canonical divergence".into();
    let invalid = AccountedDocument::from_owned(d, Arc::new((source.clone(), fee)), q.clone(), &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    assert!(invalid.try_edit(metadata(id, None, None, None), &ctl()).is_err());
    assert!(invalid.try_edit(metadata(id, Some("repair"), None, None), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
}
