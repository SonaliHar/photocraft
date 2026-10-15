//! UNRUN structural metadata custody only. Reuses the genuinely prepaid synthetic
//! native/mask fixture; its ICC bytes are not a CMS/source/render qualification.
use super::tests::fixture_encoding;
use super::*;
use photocraft_raster::Interrupt;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn metadata(layer: crate::LayerId, name: Option<&str>, visible: Option<bool>, opacity: Option<f32>) -> AccountedEdit<'_> {
    AccountedEdit::LayerMetadata { layer, name, visible, opacity }
}
fn same_payload(source: &AccountedDocument, copy: &AccountedDocument) {
    assert!(source.quota().shares_ledger(copy.quota()));
    assert!(Arc::ptr_eq(&source.owner.retention, &copy.owner.retention));
    assert!(Arc::ptr_eq(source.document().icc_profile.as_ref().unwrap(), copy.document().icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        source.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        copy.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&source.document().layers[1].mask.as_ref().unwrap().surface, &copy.document().layers[1].mask.as_ref().unwrap().surface));
    for (a, b) in source.document().layers.iter().zip(&copy.document().layers) {
        assert_eq!(a.content, b.content);
    }
}
#[test]
fn both_layer_ids_compound_metadata_preserves_native_mask_and_profile_owners() {
    for (sample, encoding) in [
        (SampleType::U16, SampleEncoding::NonlinearProfile),
        (SampleType::U32, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::NonlinearProfile),
        (SampleType::F64, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::Linear),
        (SampleType::F64, SampleEncoding::Linear),
        (SampleType::F32, SampleEncoding::Perceptual),
        (SampleType::F64, SampleEncoding::Perceptual),
    ] {
        for at in 0..2 {
            let source = fixture_encoding(sample, encoding);
            let id = source.document().layers[at].id;
            let edit = metadata(id, Some(" renamed μ "), Some(false), Some(0.375));
            source.validate_imported_metadata_edit(edit, &ctl()).unwrap();
            let copy = source.try_edit(edit, &ctl()).unwrap();
            copy.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
            assert!(copy.validate_masked_analytic_normal_storage(&ctl()).is_err());
            same_payload(&source, &copy);
            let mut expected = source.document().clone(); // external model oracle
            expected.layers[at].name = " renamed μ ".into();
            expected.layers[at].visible = false;
            expected.layers[at].opacity = 0.375;
            let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap();
            item.name = " renamed μ ".into();
            item.custom_name = true;
            item.visible = false;
            assert_eq!(copy.document(), &expected);
            let title = copy.try_with_document_name("continued.pcraft", &ctl()).unwrap();
            same_payload(&source, &title);
            assert_eq!(title.document().layers, copy.document().layers);
            assert!(copy.try_replace_selection(None, &ctl()).is_err());
        }
    }
}
#[test]
fn raw_negative_zero_is_changed_but_all_none_and_exact_noop_preserve_redo_and_custom_name() {
    let source = fixture_encoding(SampleType::F64, SampleEncoding::Perceptual);
    let id = source.document().layers[0].id;
    let mut history = AccountedHistory::new(source.clone(), 4, &ctl()).unwrap();
    history.try_commit(metadata(id, Some(&source.document().layers[0].name), None, Some(-0.0)), &ctl()).unwrap();
    assert!(history.can_undo());
    assert_eq!(history.current().unwrap().document().layers[0].opacity.to_bits(), (-0.0f32).to_bits());
    assert!(!history.current().unwrap().document().xcf_state.as_ref().unwrap().items.iter().find(|v| v.item == ItemRef::Layer(id)).unwrap().custom_name);
    history.undo(&ctl()).unwrap();
    assert!(history.current().unwrap().shares_model(&source));
    history.try_commit(metadata(id, None, None, None), &ctl()).unwrap();
    assert!(history.can_redo());
    history.redo(&ctl()).unwrap();
    let negative = history.current().unwrap().clone();
    assert!(negative.try_edit(metadata(id, None, None, Some(-0.0)), &ctl()).unwrap().shares_model(&negative));
    let positive = negative.try_edit(metadata(id, None, None, Some(0.0)), &ctl()).unwrap();
    assert!(!positive.shares_model(&negative));
    assert_eq!(positive.document().layers[0].opacity.to_bits(), 0.0f32.to_bits());
    same_payload(&source, &positive);
}
#[test]
fn invalid_parameters_individual_edits_and_malformed_canonical_input_refuse_without_mutation() {
    let source = fixture_encoding(SampleType::F32, SampleEncoding::Linear);
    let id = source.document().layers[1].id;
    let baseline = source.quota().usage().unwrap();
    for edit in [
        metadata(id, Some("bad\0name"), None, None),
        metadata(id, None, None, Some(f32::NAN)),
        metadata(id, None, None, Some(f32::INFINITY)),
        metadata(id, None, None, Some(1.01)),
        metadata(crate::LayerId(u64::MAX), None, Some(false), None),
        AccountedEdit::LayerName { layer: id, name: "closed" },
        AccountedEdit::LayerVisibility { layer: id, visible: false },
        AccountedEdit::LayerOpacity { layer: id, opacity: 0.2 },
        AccountedEdit::NativePixels { layer: id, rect: Rect::new(-3, -4, -2, -3), bytes: &[0; 16] },
    ] {
        assert!(source.try_edit(edit, &ctl()).is_err());
        assert_eq!(source.quota().usage().unwrap(), baseline);
    }
    let q = source.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut malformed = source.document().clone();
    // Strict imported edits require coherent canonical prestate. Even all-none
    // must refuse this synthetic malformed native model instead of repairing it.
    malformed.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap().name = "divergent".into();
    let invalid = AccountedDocument::from_owned(malformed, Arc::new((source.clone(), fee)), q.clone(), &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    assert!(invalid.try_edit(metadata(id, None, None, None), &ctl()).is_err());
    assert!(invalid.try_edit(metadata(id, Some("repair"), None, None), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut malformed = source.document().clone();
    malformed.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap().visible = false;
    let invalid = AccountedDocument::from_owned(malformed, Arc::new((source, fee)), q.clone(), &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    assert!(invalid.validate_assigned_analytic_geometry_storage(&ctl()).is_err());
    assert!(invalid.try_edit(metadata(id, None, Some(false), None), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn quota_and_late_cancellation_refund_and_noop_needs_no_copy_allocation() {
    let source = fixture_encoding(SampleType::U16, SampleEncoding::NonlinearProfile);
    let id = source.document().layers[0].id;
    let q = source.quota();
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(source.try_edit(metadata(id, None, None, None), &ctl()).unwrap().shares_model(&source));
    assert!(source.try_edit(metadata(id, Some("changed"), Some(false), Some(0.5)), &ctl()).is_err());
    drop(blocker);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(source.try_edit(metadata(id, None, Some(false), None), &deny).is_err());
    let stop = || q.usage().unwrap().index > baseline.index;
    let token = Interrupt::cancel_only(&stop);
    assert!(source.try_edit(metadata(id, Some("changed"), Some(false), Some(0.5)), &ReadControl { interrupt: &token, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let first = source.try_edit(metadata(id, Some("a"), Some(false), Some(0.5)), &ctl()).unwrap();
    let second = first.try_edit(metadata(id, Some("b"), Some(true), Some(0.75)), &ctl()).unwrap();
    same_payload(&source, &second);
    drop(first);
    drop(second);
    assert_eq!(q.usage().unwrap(), baseline);
}
