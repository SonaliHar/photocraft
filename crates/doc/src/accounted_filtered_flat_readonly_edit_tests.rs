//! Ignored UNRUN target-aware structural edits, with synthetic unqualified
//! operations/opaque ICC. They provide no source, CMS or numerical evidence.
use super::*;
fn editable(sample: SampleType, first_revision: u64, second_revision: u64) -> AccountedDocument {
    let q = quota();
    let (mut d, g) = fixture(&q, sample);
    let state = d.xcf_state.as_mut().unwrap();
    for (index, stack) in state.filter_stacks.iter_mut().enumerate() {
        stack.revision = if index == 0 { first_revision } else { second_revision };
        let mut components = Vec::with_capacity(13);
        components.extend([0.25f32, 0.5, 0.75, 1.0].map(|v| ExactNumber::Float32Bits(v.to_bits())));
        stack.nodes[0].arguments = vec![
            FilterArgument {
                name: "scalar".into(),
                value: FilterValue::Number(ExactNumber::Float32Bits(if index == 0 { (-0f32).to_bits() } else { 0.25f32.to_bits() })),
            },
            FilterArgument { name: "color".into(), value: FilterValue::Color { components, encoding: SampleEncoding::Linear, profile: None } },
            FilterArgument { name: "flag".into(), value: FilterValue::Bool(false) },
            FilterArgument { name: "mode".into(), value: FilterValue::Enum { domain: "stored-domain".into(), value: 7 } },
        ];
    }
    // Fixture preholds precede these bounded replacements and keep spare vectors.
    AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap()
}
fn patch<'a>(owner: &AccountedDocument, stack_index: usize, arguments: &'a [PointArgumentPatch<'a>], enabled: Option<bool>) -> PointFilterPatch<'a> {
    let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[stack_index];
    let ItemRef::Layer(layer) = stack.owner else { panic!("fixture") };
    PointFilterPatch { layer, filter: stack.nodes[0].id, expected_stack_revision: stack.revision, arguments, enabled }
}
fn unchanged_owners(before: &AccountedDocument, after: &AccountedDocument) {
    assert!(after.is_readonly_filtered_flat_point_storage());
    assert!(Arc::ptr_eq(&before.owner.retention, &after.owner.retention));
    assert!(Arc::ptr_eq(before.document().icc_profile.as_ref().unwrap(), after.document().icc_profile.as_ref().unwrap()));
    assert_eq!(before.document().source_color_context, after.document().source_color_context);
    assert_eq!(before.document().working_profile_interpretation, after.document().working_profile_interpretation);
    for (a, b) in before.document().layers.iter().zip(&after.document().layers) {
        let LayerContent::BackedRaster(a) = &a.content else { panic!() };
        let LayerContent::BackedRaster(b) = &b.content else { panic!() };
        assert_eq!(a, b); // BackedLayer Eq is conservative exact shared native state.
    }
    let a = before.document().xcf_state.as_ref().unwrap();
    let b = after.document().xcf_state.as_ref().unwrap();
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.selection, b.selection);
    for (a, b) in a.filter_masks.iter().zip(&b.filter_masks) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.owner, b.owner);
        assert_eq!(a.bounds, b.bounds);
        assert_eq!(a.surface.default_bytes(), b.surface.default_bytes());
        for ((ca, ta), (cb, tb)) in a.surface.tiles().zip(b.surface.tiles()) {
            assert_eq!(ca, cb);
            assert!(Arc::ptr_eq(ta, tb));
        }
    }
}
#[test]
#[ignore = "UNRUN: nonfirst actual owner/stack/filter target changes once; all other roots/arguments/raw/masks/profile/source stay exact"]
fn nonfirst_filter_target_is_atomic_and_title_retention_stays_flat() {
    for sample in [SampleType::F32, SampleType::F64] {
        let original = editable(sample, 17, 23);
        let arguments = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.75f32.to_bits()) }];
        let p = patch(&original, 1, &arguments, Some(true));
        original.validate_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
        let changed = original.try_with_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
        let mut expected = original.document().clone(); // external harness comparison only
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[1];
        stack.revision = 24;
        stack.nodes[0].enabled = true;
        stack.nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits()));
        assert_eq!(changed.document(), &expected);
        unchanged_owners(&original, &changed);
        assert!(changed.try_with_readonly_filtered_flat_point_filter(patch(&changed, 1, &arguments, Some(true)), &ctl()).unwrap().shares_model(&changed));
        let titled = changed.try_with_document_name("after-edit.pcraft", &ctl()).unwrap();
        unchanged_owners(&original, &titled);
        let changed = titled.try_with_readonly_filtered_flat_point_filter(patch(&titled, 0, &[], Some(true)), &ctl()).unwrap();
        unchanged_owners(&original, &changed);
        assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 18);
        assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[1].revision, 24);
    }
}
#[test]
#[ignore = "UNRUN: raw signedzero+literal encoding+activation combine once, exact MAX/fullquota noops share, changed MAX refuses"]
fn combined_literal_and_scalar_bits_keep_exact_noop_boundaries() {
    let owner = editable(SampleType::F64, 17, u64::MAX);
    let words = [0.25f32, 0.5, 0.75, 1.].map(f32::to_bits);
    let args = [
        PointArgumentPatch { name: "color", value: PointArgumentValue::LiteralColor4F32 { components: words, encoding: SampleEncoding::NonlinearProfile } },
        PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0f32.to_bits()) },
    ];
    let changed = owner.try_with_readonly_filtered_flat_point_filter(patch(&owner, 0, &args, Some(true)), &ctl()).unwrap();
    unchanged_owners(&owner, &changed);
    let stack = &changed.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    assert_eq!(stack.revision, 18);
    assert!(stack.nodes[0].enabled);
    assert_eq!(stack.nodes[0].arguments[0].value, FilterValue::Number(ExactNumber::Float32Bits(0f32.to_bits())));
    let FilterValue::Color { components, encoding, profile } = &stack.nodes[0].arguments[1].value else { panic!() };
    assert_eq!(*encoding, SampleEncoding::NonlinearProfile);
    assert!(profile.is_none());
    assert!((4..=8).contains(&components.capacity()));
    assert_eq!(components.as_slice(), &words.map(ExactNumber::Float32Bits));
    let before = owner.quota().usage().unwrap();
    let blocker = owner.quota().reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    let equal = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.25f32.to_bits()) }];
    assert!(owner.try_with_readonly_filtered_flat_point_filter(patch(&owner, 1, &equal, Some(false)), &ctl()).unwrap().shares_model(&owner));
    assert!(owner.try_with_readonly_filtered_flat_point_filter(patch(&owner, 0, &args, Some(true)), &ctl()).is_err());
    drop(blocker);
    assert!(owner.try_with_readonly_filtered_flat_point_filter(patch(&owner, 1, &[], Some(true)), &ctl()).is_err());
    assert_eq!(owner.quota().usage().unwrap(), before);
}
#[test]
#[ignore = "UNRUN: compound metadata exact target/canonical sync on plain and filtered roots, raw opacity signedzero, stack revisions and individual denials"]
fn metadata_targets_all_roots_without_touching_filter_or_mask_records() {
    let initial = editable(SampleType::F32, 17, 23);
    for index in 0..3 {
        let id = initial.document().layers[index].id;
        let edit = AccountedEdit::LayerMetadata { layer: id, name: Some("renamed root"), visible: Some(false), opacity: Some(-0.) };
        initial.validate_imported_metadata_edit(edit, &ctl()).unwrap();
        let changed = initial.try_edit(edit, &ctl()).unwrap();
        unchanged_owners(&initial, &changed);
        let mut expected = initial.document().clone(); // external harness only
        expected.layers[index].name = "renamed root".into();
        expected.layers[index].visible = false;
        expected.layers[index].opacity = -0.;
        let state = expected.xcf_state.as_mut().unwrap();
        let item = state.items.iter_mut().find(|row| row.item == ItemRef::Layer(id)).unwrap();
        item.name = "renamed root".into();
        item.custom_name = true;
        item.visible = false;
        assert_eq!(changed.document(), &expected);
        let noop = AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: None };
        assert!(changed.try_edit(noop, &ctl()).unwrap().shares_model(&changed));
        assert!(changed.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: id, name: "renamed root" }, &ctl()).is_err());
        assert!(changed.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &ctl()).is_err());
        let mut history = AccountedHistory::new(initial.clone(), 4, &ctl()).unwrap();
        history.try_commit(edit, &ctl()).unwrap();
        assert_eq!(history.len(), 2);
        history.try_commit(noop, &ctl()).unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.undo(&ctl()).unwrap());
        assert!(history.current().unwrap().shares_model(&initial));
        assert!(history.redo(&ctl()).unwrap());
        assert_eq!(history.current().unwrap().document(), &expected);
    }
}
#[test]
#[ignore = "UNRUN: crossroot/stale/duplicate/unknown/type/tag/unit failures publish no copy or revision; current malformed input remains refused by constructor"]
fn invalid_requests_and_current_prestate_refuse_atomically() {
    let owner = editable(SampleType::F32, 17, 23);
    let before = owner.quota().usage().unwrap();
    let duplicate = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(1f32.to_bits()) }; 2];
    let unknown = [PointArgumentPatch { name: "absent", value: PointArgumentValue::Bool(true) }];
    let wrong = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Bool(true) }];
    for args in [&duplicate[..], &unknown[..], &wrong[..]] {
        assert!(owner.try_with_readonly_filtered_flat_point_filter(patch(&owner, 0, args, Some(true)), &ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
    for variant in 0..4 {
        let mut p = patch(&owner, 0, &[], None);
        match variant {
            0 => p.layer = owner.document().layers[0].id,
            1 => p.filter = owner.document().xcf_state.as_ref().unwrap().filter_stacks[1].nodes[0].id,
            2 => p.expected_stack_revision += 1,
            _ => p.filter = crate::item_identity::FilterId(0),
        }
        assert!(owner.validate_readonly_filtered_flat_point_filter(p, &ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
    for value in [
        PointArgumentValue::LiteralColor4F32 { components: [0; 4], encoding: SampleEncoding::Perceptual },
        PointArgumentValue::LiteralColor4F32 { components: [f32::NAN.to_bits(); 4], encoding: SampleEncoding::Linear },
        PointArgumentValue::LiteralColor4F32 { components: [2f32.to_bits(); 4], encoding: SampleEncoding::NonlinearProfile },
        PointArgumentValue::Color4F32([0; 4]),
    ] {
        let args = [PointArgumentPatch { name: "color", value }];
        assert!(owner.try_with_readonly_filtered_flat_point_filter(patch(&owner, 0, &args, None), &ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
    let layer = owner.document().layers[2].id;
    for opacity in [f32::NAN, 2.0] {
        assert!(owner.try_edit(AccountedEdit::LayerMetadata { layer, name: None, visible: None, opacity: Some(opacity) }, &ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
    assert!(owner.try_edit(AccountedEdit::LayerMetadata { layer, name: Some("bad\0name"), visible: None, opacity: None }, &ctl()).is_err());
    for variant in 0..3 {
        let q = quota();
        let (mut d, g) = fixture(&q, SampleType::F32);
        if variant == 0 {
            d.layers[1].opacity = f32::NAN;
        } else if variant == 1 {
            d.xcf_state.as_mut().unwrap().items[0].name.push('x');
        } else {
            d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments = vec![FilterArgument {
                name: "color".into(),
                value: FilterValue::Color {
                    components: vec![ExactNumber::Float32Bits(0); 4],
                    encoding: SampleEncoding::Linear,
                    profile: Some(crate::item_identity::ResourceId(1)),
                },
            }];
        }
        assert!(AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: quota and early/late sameledger Work/cancel failures refund whole copied model and preserve redo/name/source"]
fn copy_failure_lifetimes_and_metadata_history_redo_are_preserved() {
    let owner = editable(SampleType::F64, 17, 23);
    let before = owner.quota().usage().unwrap();
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = owner
        .try_with_readonly_filtered_flat_point_filter(patch(&owner, 1, &[], Some(true)), &ReadControl { interrupt: &Interrupt::NONE, work: &count })
        .unwrap();
    drop(changed);
    assert_eq!(owner.quota().usage().unwrap(), before);
    for limit in [0, total.load(Ordering::Relaxed) - 1] {
        let left = AtomicU64::new(limit);
        let work = |n| left.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
        assert!(
            owner
                .try_with_readonly_filtered_flat_point_filter(patch(&owner, 1, &[], Some(true)), &ReadControl { interrupt: &Interrupt::NONE, work: &work })
                .is_err()
        );
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
    let work = |_| if owner.quota().usage().map_err(|_| BackingError::Quota)?.index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(
        owner
            .try_with_readonly_filtered_flat_point_filter(patch(&owner, 1, &[], Some(true)), &ReadControl { interrupt: &Interrupt::NONE, work: &work })
            .is_err()
    );
    assert_eq!(owner.quota().usage().unwrap(), before);
    let blocker = owner.quota().reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    let id = owner.document().layers[2].id;
    let edit = AccountedEdit::LayerMetadata { layer: id, name: Some("new name"), visible: None, opacity: None };
    assert!(owner.try_edit(edit, &ctl()).is_err());
    assert!(owner.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: None }, &ctl()).unwrap().shares_model(&owner));
    drop(blocker);
    assert_eq!(owner.quota().usage().unwrap(), before);
    let mut history = AccountedHistory::new(owner.clone(), 4, &ctl()).unwrap();
    history.try_commit(edit, &ctl()).unwrap();
    history.undo(&ctl()).unwrap();
    assert!(history.can_redo());
    let snapshot = history.current().unwrap().clone();
    let work = |_| Err(BackingError::Work);
    assert!(history.try_commit(edit, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert!(history.can_redo());
    assert!(history.current().unwrap().shares_model(&snapshot));
}
