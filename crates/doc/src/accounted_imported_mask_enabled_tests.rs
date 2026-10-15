//! UNRUN structural mask-enabled custody. The reused fixture owns real native,
//! mask and metadata fees; its synthetic ICC is not CMS/source/render evidence.
use super::tests::fixture_encoding;
use super::*;
use photocraft_raster::Interrupt;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn edit(layer: crate::LayerId, enabled: bool) -> AccountedEdit<'static> {
    AccountedEdit::MaskEnabled { layer, enabled }
}
fn shared(source: &AccountedDocument, copy: &AccountedDocument) {
    assert!(source.quota().shares_ledger(copy.quota()));
    assert!(Arc::ptr_eq(&source.owner.retention, &copy.owner.retention));
    assert!(Arc::ptr_eq(&source.document().layers[1].mask.as_ref().unwrap().surface, &copy.document().layers[1].mask.as_ref().unwrap().surface));
    assert!(Arc::ptr_eq(source.document().icc_profile.as_ref().unwrap(), copy.document().icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        source.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        copy.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
    for (a, b) in source.document().layers.iter().zip(&copy.document().layers) {
        assert_eq!(a.content, b.content);
    }
}
#[test]
fn disable_enable_preserves_raw_mask_native_profile_owners_across_all_storage_pairs() {
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
        let source = fixture_encoding(sample, encoding);
        let id = source.document().layers[1].id;
        source.validate_assigned_analytic_mask_edit(edit(id, false), &ctl()).unwrap();
        let disabled = source.try_edit(edit(id, false), &ctl()).unwrap();
        disabled.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert!(disabled.validate_masked_analytic_normal_storage(&ctl()).is_err());
        shared(&source, &disabled);
        let mut expected = source.document().clone(); // external structural oracle
        expected.layers[1].mask.as_mut().unwrap().enabled = false;
        expected.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap().enabled = false;
        assert_eq!(disabled.document(), &expected);
        let metadata = disabled
            .try_edit(AccountedEdit::LayerMetadata { layer: id, name: Some("disabled current"), visible: Some(false), opacity: Some(0.25) }, &ctl())
            .unwrap();
        assert!(!metadata.document().layers[1].mask.as_ref().unwrap().enabled);
        let title = metadata.try_with_document_name("continued.pcraft", &ctl()).unwrap();
        shared(&source, &title);
        let enabled = disabled.try_edit(edit(id, true), &ctl()).unwrap();
        assert_eq!(enabled.document(), source.document());
        shared(&source, &enabled);
        assert!(disabled.try_replace_selection(None, &ctl()).is_err());
    }
}
#[test]
fn one_undo_and_full_quota_exact_noop_preserve_redo() {
    let source = fixture_encoding(SampleType::F64, SampleEncoding::NonlinearProfile);
    let id = source.document().layers[1].id;
    let mut history = AccountedHistory::new(source.clone(), 4, &ctl()).unwrap();
    history.try_commit(edit(id, false), &ctl()).unwrap();
    assert!(history.can_undo());
    let disabled = history.current().unwrap().clone();
    history.undo(&ctl()).unwrap();
    assert!(history.current().unwrap().shares_model(&source));
    history.try_commit(edit(id, true), &ctl()).unwrap();
    assert!(history.can_redo());
    history.redo(&ctl()).unwrap();
    assert!(history.current().unwrap().shares_model(&disabled));
    let q = source.quota();
    let baseline = q.usage().unwrap();
    let block = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(disabled.try_edit(edit(id, false), &ctl()).unwrap().shares_model(&disabled));
    assert!(disabled.try_edit(edit(id, true), &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn unsupported_targets_other_edits_and_incoherent_flags_refuse_without_mutation() {
    let source = fixture_encoding(SampleType::F32, SampleEncoding::Linear);
    let id = source.document().layers[1].id;
    let before = source.quota().usage().unwrap();
    for e in [
        edit(source.document().layers[0].id, false),
        edit(crate::LayerId(u64::MAX), false),
        AccountedEdit::MaskLinked { layer: id, linked: true },
        AccountedEdit::NativePixels { layer: id, rect: Rect::new(-3, -4, -2, -3), bytes: &[0; 16] },
    ] {
        assert!(source.try_edit(e, &ctl()).is_err());
        assert_eq!(source.quota().usage().unwrap(), before);
    }
    for fault in 0..3 {
        let q = source.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = source.document().clone();
        doc.layers[1].mask.as_mut().unwrap().enabled = false;
        let m = doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap();
        if fault != 0 {
            m.enabled = false;
        }
        if fault == 1 {
            m.offset[0] += 1;
        }
        if fault == 2 {
            doc.layers[1].mask.as_mut().unwrap().density = 0.5;
        }
        let invalid = AccountedDocument::from_owned(doc, Arc::new((source.clone(), fee)), q.clone(), &ctl()).unwrap();
        let baseline = q.usage().unwrap();
        // Disabled state cannot bypass complete structural map/format/association
        // validation. Operative unit sample scans remain the compositor's duty.
        assert!(invalid.validate_assigned_analytic_geometry_storage(&ctl()).is_err());
        assert!(invalid.validate_assigned_analytic_mask_edit(edit(id, false), &ctl()).is_err());
        assert!(invalid.try_edit(edit(id, true), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
fn work_and_cancellation_refund_before_shared_payload_release() {
    let source = fixture_encoding(SampleType::U32, SampleEncoding::NonlinearProfile);
    let id = source.document().layers[1].id;
    let q = source.quota();
    let baseline = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(source.validate_assigned_analytic_mask_edit(edit(id, false), &denied).is_err());
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(source.try_edit(edit(id, false), &ReadControl { interrupt: &cancelled, work: &|_| Ok(()) }).is_err());
    let late = || q.usage().unwrap().index > baseline.index;
    let token = Interrupt::cancel_only(&late);
    assert!(source.try_edit(edit(id, false), &ReadControl { interrupt: &token, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let a = source.try_edit(edit(id, false), &ctl()).unwrap();
    let b = a.try_edit(edit(id, true), &ctl()).unwrap();
    shared(&source, &b);
    drop(a);
    drop(b);
    assert_eq!(q.usage().unwrap(), baseline);
}
