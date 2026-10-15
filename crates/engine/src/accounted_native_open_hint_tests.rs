//! UNRUN shared routing-hint regressions. Deliberately incomplete nodes prove
//! this name scan is not runtime storage/source/profile eligibility.
use super::*;
use photocraft_doc::{
    ColorMode, Document, LayerId, SampleType, Size, XcfState,
    item_identity::{FilterId, ItemRef},
    xcf_filters::{FilterNode, FilterStack, OperationIdentity},
};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn node(name: &str, id: u64) -> FilterNode {
    FilterNode {
        id: FilterId(id),
        operation: OperationIdentity { name: name.into(), stored_version: "unqualified".into(), schema_epoch: "not-authority".into() },
        arguments: Vec::new(),
        source_display_name: None,
        source_icon: None,
        enabled: false,
        opacity_f64_bits: 0f64.to_bits(),
        source_clipping: None,
        source_region: None,
        mask: None,
        auxiliary: Vec::new(),
        composition: None,
    }
}
fn document() -> Document {
    let mut doc = Document::new("hint", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    let mut state = XcfState::default();
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(LayerId(1)), revision: 0, nodes: Vec::new() });
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(LayerId(2)),
        revision: 0,
        nodes: vec![node("gegl:exposure", 1), node("gimp:threshold-alpha", 2), node("ignored-after-match", 3)],
    });
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn same_order_poll_schedule_and_early_match_without_schema_or_activation_inference() {
    let doc = document();
    let units = AtomicU64::new(0);
    let work = |n| {
        units.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert_eq!(is_current_threshold_alpha_candidate(&doc, &c), Ok(true));
    assert_eq!(units.load(Ordering::Relaxed), 4, "one per stack and visited node; no visit after match");
    let mut no_match = doc.clone();
    no_match.xcf_state.as_mut().unwrap().filter_stacks[1].nodes[1].operation.name = "Gimp:threshold-alpha".into();
    units.store(0, Ordering::Relaxed);
    assert_eq!(is_current_threshold_alpha_candidate(&no_match, &c), Ok(false));
    assert_eq!(units.load(Ordering::Relaxed), 5);
}
#[test]
fn actual_control_error_propagation_and_empty_input_schedule_remain_unchanged() {
    let doc = document();
    let deny = |_| Err(BackingError::Work);
    assert_eq!(is_current_threshold_alpha_candidate(&doc, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(BackingError::Work));
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert_eq!(is_current_threshold_alpha_candidate(&doc, &c), Err(BackingError::Cancelled));
    let empty = Document::new("no state", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    assert_eq!(is_current_threshold_alpha_candidate(&empty, &c), Ok(false), "no new unconditional poll added to old zero-visit hint");
}
#[test]
fn a_positive_name_hint_does_not_supply_runtime_storage_adoption() {
    let doc = document();
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    assert_eq!(is_current_threshold_alpha_candidate(&doc, &c), Ok(true));
    let q = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 });
    let guard = Arc::new(q.reserve(QuotaKind::Resident, 16384).unwrap());
    assert!(
        photocraft_doc::accounted_document::AccountedDocument::from_owned_readonly_threshold_alpha_storage(doc, guard, q, &c).is_err(),
        "typed handoff rejects missing native owner/profile/schema despite the routing hint"
    );
}

#[path = "accounted_box_work_hint_tests.rs"]
mod box_work;
