//! UNRUN original witnessed stack source/control and deterministic-route drafts.
use super::super::super::plane::fixture;
use super::*;
use photocraft_xcf::Effect;
fn file(count: usize) -> XcfFile {
    let bytes = fixture::image(22, 150, 0, 8, 6, &[63, 127, 191, 37].repeat(48));
    let mut f = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
    let d = &mut f.layers[0];
    d.visible = true;
    d.opacity = 1.;
    d.mode = 28;
    d.blend_space = 0;
    d.composite_space = 2;
    d.composite_mode = 1;
    for operation in ["gegl:exposure", "gegl:invert-linear"].into_iter().take(count) {
        d.effects.push(Effect {
            name: String::new(),
            icon: String::new(),
            operation: operation.into(),
            version: Some(if operation == "gegl:exposure" { "1:0" } else { "0:0" }.into()),
            properties: Vec::new(),
            arguments: Vec::new(),
            mask: None,
            state: Default::default(),
        });
    }
    f
}
#[test]
fn source_policy_matches_original_and_single_removed_node_keeps_stack_route() {
    for count in 1..=2 {
        let mut f = file(count);
        assert!(super::super::super::qualified_point_stack_edits(&f));
        assert!(qualify(&f, &Interrupt::NONE, None).unwrap());
        assert_eq!(SourceSelectedPolicy::family(&f), SelectedStagedFamily::PointStackEdits);
        // Deliberately incomplete typed records: a source hint/outer envelope
        // never bypasses the old precise argument/raw-state qualified mapper.
        assert!(
            crate::xcf_filters::qualified_stack(
                &f.layers[0].effects,
                &vec![None; count],
                photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
                1,
                22,
                crate::xcf_filters::NeighborhoodDomain::PointStackEdits
            )
            .is_err()
        );
        f.layers[0].composite_space = -1;
        f.layers[0].composite_mode = -1;
        assert!(!super::super::super::qualified_point_stack_edits(&f));
        assert!(!qualify(&f, &Interrupt::NONE, None).unwrap());
        assert_ne!(SourceSelectedPolicy::family(&f), SelectedStagedFamily::PointStackEdits);
    }
}
#[test]
fn original_singleton_and_oversize_wrong_geometry_or_operation_are_not_stack_grants() {
    let mut f = file(2);
    f.layers[0].effects[0].operation = "gegl:brightness-contrast".into();
    assert!(!qualify(&f, &Interrupt::NONE, None).unwrap());
    assert!(!super::super::super::qualified_point_stack_edits(&f));
    f.layers[0].effects[0].operation = "gegl:exposure".into();
    let extra = f.layers[0].effects[0].clone();
    f.layers[0].effects.resize(100000, extra);
    assert!(!qualify(&f, &Interrupt::NONE, None).unwrap());
    assert_eq!(SourceSelectedPolicy::family(&f), SelectedStagedFamily::PointStackEdits);
    f.layers[0].effects.truncate(2);
    f.layers[0].offset = (-1, 0);
    assert!(!qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].offset = (0, 0);
    f.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(!qualify(&f, &Interrupt::NONE, None).unwrap());
}
#[test]
fn actual_native_work_and_cancellation_are_cumulative_and_refuse_before_mapping() {
    let f = file(2);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&budget)).is_err());
    assert_eq!(budget.used(photocraft_xcf::BudgetKind::Work), 32);
    assert!(qualify(&f, &Interrupt::NONE, Some(&budget)).is_err());
    assert_eq!(budget.used(photocraft_xcf::BudgetKind::Work), 32);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(qualify(&f, &interrupt, None), Err(IoError::Cancelled)));
}
