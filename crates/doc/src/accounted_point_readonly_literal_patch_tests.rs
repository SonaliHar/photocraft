//! UNRUN literal-tag custody edits on an intentionally unqualified synthetic
//! operation. No source, evaluator, profile-conversion or command grant follows.
use super::*;

const WORDS: [u32; 4] = [0x8000_0000, 0x3f40_0000, 0x3e80_0000, 0x3f80_0000];
fn literal_owner(encoding: SampleEncoding, revision: u64) -> AccountedDocument {
    let q = quota();
    let (mut doc, guard) = fixture(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let stack = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0];
    stack.revision = revision;
    // The fixture's genuine original prehold precedes this bounded metadata
    // allocation; it is not a fabricated payload receipt or new semantic proof.
    let mut components = Vec::with_capacity(13);
    components.extend(WORDS.into_iter().map(ExactNumber::Float32Bits));
    stack.nodes[0].arguments[9].value = FilterValue::Color { components, encoding, profile: None };
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}
fn request(encoding: SampleEncoding, components: [u32; 4]) -> [PointArgumentPatch<'static>; 1] {
    [PointArgumentPatch { name: "raw-9", value: PointArgumentValue::LiteralColor4F32 { components, encoding } }]
}
fn retained(original: &AccountedDocument, changed: &AccountedDocument) {
    assert!(changed.is_readonly_point_storage());
    assert!(changed.quota().shares_ledger(original.quota()));
    assert_eq!(original.document().source_color_context, changed.document().source_color_context);
    assert_eq!(original.document().working_profile_interpretation, changed.document().working_profile_interpretation);
    assert!(Arc::ptr_eq(original.document().icc_profile.as_ref().unwrap(), changed.document().icc_profile.as_ref().unwrap()));
    let a = &original.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let b = &changed.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    assert_eq!(a.default_bytes(), b.default_bytes());
    assert!(Arc::ptr_eq(a.tile(TileCoord::new(1, 0)).unwrap(), b.tile(TileCoord::new(1, 0)).unwrap()));
    let LayerContent::BackedRaster(a) = &original.document().layers[0].content else { panic!() };
    let LayerContent::BackedRaster(b) = &changed.document().layers[0].content else { panic!() };
    assert_eq!(a.descriptor(), b.descriptor());
    assert_eq!(a.default_bytes(), b.default_bytes());
}

#[test]
#[ignore = "UNRUN same literal words with changed encoding change exactly one revision, activation combined once"]
fn encoding_is_an_independent_exact_patch_word() {
    for (before, after) in [(SampleEncoding::Linear, SampleEncoding::NonlinearProfile), (SampleEncoding::NonlinearProfile, SampleEncoding::Linear)] {
        let old = literal_owner(before, 7);
        let args = request(after, WORDS);
        let next = old.try_with_readonly_point_filter(patch(&old, &args, Some(true)), &ctl()).unwrap();
        let mut expected = old.document().clone(); // External expected-model harness.
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.revision = 8;
        stack.nodes[0].enabled = true;
        let FilterValue::Color { encoding, .. } = &mut stack.nodes[0].arguments[9].value else { panic!() };
        *encoding = after;
        assert_eq!(next.document(), &expected);
        retained(&old, &next);
        let FilterValue::Color { components, encoding, profile } = &next.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[9].value
        else {
            panic!()
        };
        assert_eq!(*encoding, after);
        assert!(profile.is_none());
        assert_eq!(components, &WORDS.into_iter().map(ExactNumber::Float32Bits).collect::<Vec<_>>());
        assert!((4..=8).contains(&components.capacity()));
        assert_eq!(old.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
    }
}

#[test]
#[ignore = "UNRUN exact bits and tag no-op at full Index/MAX revision; legacy NL raw NaN remains independent"]
fn raw_noop_and_legacy_color_contracts_do_not_merge() {
    for encoding in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
        let old = literal_owner(encoding, u64::MAX);
        let q = old.quota();
        let before = q.usage().unwrap();
        let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
        let same = request(encoding, WORDS);
        assert!(old.try_with_readonly_point_filter(patch(&old, &same, Some(false)), &ctl()).unwrap().shares_model(&old));
        let other = request(if encoding == SampleEncoding::Linear { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear }, WORDS);
        assert!(matches!(old.try_with_readonly_point_filter(patch(&old, &other, None), &ctl()), Err(BackingError::Length)));
        let mut positive_zero = WORDS;
        positive_zero[0] = 0;
        let signed_change = request(encoding, positive_zero);
        assert!(old.try_with_readonly_point_filter(patch(&old, &signed_change, None), &ctl()).is_err());
        drop(blocker);
        assert_eq!(q.usage().unwrap(), before);
    }
    let raw = current(true); // Existing NL raw color contains an exact NaN payload.
    let words = [0x8000_0000, 0x7fc0_1234, 0x3f12_3456, 0x3f80_0000];
    let legacy = [PointArgumentPatch { name: "raw-9", value: PointArgumentValue::Color4F32(words) }];
    assert!(raw.try_with_readonly_point_filter(patch(&raw, &legacy, None), &ctl()).unwrap().shares_model(&raw));
    assert!(raw.try_with_readonly_point_filter(patch(&raw, &request(SampleEncoding::NonlinearProfile, WORDS), None), &ctl()).is_err());
    let linear = literal_owner(SampleEncoding::Linear, 7);
    let legacy = [PointArgumentPatch { name: "raw-9", value: PointArgumentValue::Color4F32(WORDS) }];
    assert!(linear.try_with_readonly_point_filter(patch(&linear, &legacy, None), &ctl()).is_err());
}

#[test]
#[ignore = "UNRUN literal finite-unit/tag/type/profile preflight refuses privately without normalization"]
fn invalid_literal_or_existing_shape_cannot_be_repaired_by_patch() {
    let old = literal_owner(SampleEncoding::Linear, 7);
    let before = old.quota().usage().unwrap();
    for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.01f32).to_bits(), 1.01f32.to_bits()] {
        for channel in 0..4 {
            let mut words = WORDS;
            words[channel] = bits;
            assert!(old.try_with_readonly_point_filter(patch(&old, &request(SampleEncoding::Linear, words), None), &ctl()).is_err());
            assert_eq!(old.quota().usage().unwrap(), before);
        }
    }
    assert!(old.try_with_readonly_point_filter(patch(&old, &request(SampleEncoding::Perceptual, WORDS), None), &ctl()).is_err());
    let wrong_type =
        [PointArgumentPatch { name: "raw-0", value: PointArgumentValue::LiteralColor4F32 { components: WORDS, encoding: SampleEncoding::Linear } }];
    assert!(old.try_with_readonly_point_filter(patch(&old, &wrong_type, None), &ctl()).is_err());
    for defect in 0..4 {
        let q = quota();
        let (mut doc, guard) = fixture(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
        let mut components: Vec<_> = WORDS.into_iter().map(ExactNumber::Float32Bits).collect();
        let mut profile = None;
        match defect {
            0 => {
                components.pop();
            }
            1 => components[0] = ExactNumber::Float64Bits(0),
            2 => profile = Some(crate::item_identity::ResourceId(1)),
            _ => components.push(ExactNumber::Float32Bits(0)),
        }
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[9].value =
            FilterValue::Color { components, encoding: SampleEncoding::Linear, profile };
        assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}

#[test]
#[ignore = "UNRUN literal tag copy actual quota/work/cancellation atomicity and flat retention"]
fn tag_change_copy_failures_refund_and_preserve_all_original_words() {
    let old = literal_owner(SampleEncoding::NonlinearProfile, 7);
    let q = old.quota().clone();
    let before = q.usage().unwrap();
    let args = request(SampleEncoding::Linear, WORDS);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(old.try_with_readonly_point_filter(patch(&old, &args, Some(true)), &deny), Err(BackingError::Work)));
    let allocated = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&allocated);
    assert!(matches!(
        old.try_with_readonly_point_filter(patch(&old, &args, Some(true)), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), before);
    let block = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(old.try_with_readonly_point_filter(patch(&old, &args, None), &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), before);
    let changed = old.try_with_readonly_point_filter(patch(&old, &args, Some(true)), &ctl()).unwrap();
    let weak = Arc::downgrade(&changed.owner);
    let title = changed.try_with_document_name("literal retained", &ctl()).unwrap();
    retained(&old, &title);
    assert!(title.owner.retention.downcast_ref::<OriginalRetention>().unwrap().original.shares_model(&old));
    assert_eq!(title.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 8);
    drop(changed);
    assert!(weak.upgrade().is_none());
    let FilterValue::Color { encoding, .. } = &old.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[9].value else { panic!() };
    assert_eq!(*encoding, SampleEncoding::NonlinearProfile);
    assert_eq!(old.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
    drop(old);
    drop(title);
    assert_eq!(q.usage().unwrap(), Default::default());
}
