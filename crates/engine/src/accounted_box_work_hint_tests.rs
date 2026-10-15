//! UNRUN work-policy routing regressions; no source/render acceptance claim.
use super::*;

fn box_document() -> Document {
    let mut doc = Document::new("untrusted recipe", Size::new(6000, 6000), ColorMode::Rgb, SampleType::F32);
    doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
    let mut state = XcfState::default();
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(LayerId(1)), revision: 0, nodes: vec![node("gegl:box-blur", 1)] });
    doc.xcf_state = Some(state);
    doc
}

#[test]
#[ignore = "UNRUN: policy selection cannot grant storage, profile or rendering authority"]
fn work_hint_can_select_finite_policy_but_incomplete_native_recipe_still_refuses() {
    let doc = box_document();
    let meter = photocraft_io::accounted_operation_work::OperationWork::legacy(256);
    let charge = |units| meter.check(units);
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    assert!(is_current_box_work_candidate(&doc, &control).unwrap());
    let before = meter.diagnostics().used;
    meter.select_standard_box(&photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()).unwrap();
    assert_eq!(meter.diagnostics().used, before);
    assert_eq!(meter.diagnostics().policy, Some(Default::default()));
    let q = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 });
    let guard = Arc::new(q.reserve(QuotaKind::Resident, 16384).unwrap());
    assert!(photocraft_doc::accounted_document::AccountedDocument::new_readonly_point_storage(doc, guard, q, &control).is_err());
}

#[test]
#[ignore = "UNRUN: unrelated recipes, malformed shape, work and cancellation retain bounded routing"]
fn work_hint_does_not_promote_other_families_or_bypass_control() {
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let base = box_document();
    for name in ["gegl:median-blur", "gegl:gaussian-blur", "Gegl:box-blur"] {
        let mut doc = base.clone();
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.name = name.into();
        assert!(!is_current_box_work_candidate(&doc, &control).unwrap());
    }
    let mut gray = base.clone();
    gray.mode = ColorMode::Grayscale;
    assert!(!is_current_box_work_candidate(&gray, &control).unwrap());
    let mut two = base.clone();
    two.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.push(node("gegl:box-blur", 2));
    assert!(!is_current_box_work_candidate(&two, &control).unwrap());
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert_eq!(is_current_box_work_candidate(&base, &denied), Err(BackingError::Work));
    let interrupt = Interrupt::cancel_only(&|| true);
    assert_eq!(is_current_box_work_candidate(&base, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(BackingError::Cancelled));
}
