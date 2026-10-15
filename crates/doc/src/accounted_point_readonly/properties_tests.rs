//! UNRUN CURRENT Point property-copy drafts; structural custody is not an evaluator proof.
use super::*;
use crate::{SampleEncoding, SampleType, item_identity::ItemRef};
use photocraft_raster::{Interrupt, backing::quota::QuotaKind};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owner() -> AccountedDocument {
    super::super::tests::owned(SampleType::F64, SampleEncoding::NonlinearProfile, 750)
}
#[test]
fn combined_properties_reconcile_names_and_visibility_without_touching_raw_native_recipe() {
    let old = owner();
    let id = old.document().layers[0].id;
    let next = old
        .try_with_readonly_layer_properties(
            ReadonlyLayerPropertiesPatch { layer: id, name: Some("  current 名  "), visible: Some(false), opacity_f32_bits: Some((-0.0f32).to_bits()) },
            &ctl(),
        )
        .unwrap();
    let mut expected = old.document().clone();
    expected.layers[0].name = "  current 名  ".into();
    expected.layers[0].visible = false;
    expected.layers[0].opacity = -0.0;
    let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap();
    item.name = "  current 名  ".into();
    item.custom_name = true;
    item.visible = false;
    assert_eq!(next.document(), &expected);
    assert_eq!(old.document().layers[0].name, "original layer");
    assert_eq!(next.document().layers[0].opacity.to_bits(), (-0.0f32).to_bits());
    assert_eq!(next.document().layers[0].content, old.document().layers[0].content);
    let a = &old.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let b = &next.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    assert_eq!(a.default_bytes(), b.default_bytes());
    for ((ca, a), (cb, b)) in a.tiles().zip(b.tiles()) {
        assert_eq!(ca, cb);
        assert!(Arc::ptr_eq(a, b));
    }
    assert!(Arc::ptr_eq(old.document().icc_profile.as_ref().unwrap(), next.document().icc_profile.as_ref().unwrap()));
    assert!(next.quota().shares_ledger(old.quota()));
    assert!(next.try_edit(AccountedEdit::LayerName { layer: id, name: "generic" }, &ctl()).is_err());
}
#[test]
fn bit_exact_noop_precedes_allocation_and_failures_leave_owner_and_actual_quota_unchanged() {
    let old = owner();
    let q = old.quota().clone();
    let id = old.document().layers[0].id;
    let item = old.document().xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap();
    assert_eq!(item.name, old.document().layers[0].name);
    let before = q.usage().unwrap();
    let blockade = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    let same = old
        .try_with_readonly_layer_properties(
            ReadonlyLayerPropertiesPatch { layer: id, name: None, visible: None, opacity_f32_bits: Some(old.document().layers[0].opacity.to_bits()) },
            &ctl(),
        )
        .unwrap();
    assert!(same.shares_model(&old));
    assert_eq!(q.usage().unwrap(), full);
    assert!(old.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: id, name: "changed" }, &ctl()).is_err());
    drop(blockade);
    for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.1f32).to_bits(), 1.1f32.to_bits()] {
        assert!(
            old.try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { layer: id, name: None, visible: None, opacity_f32_bits: Some(bits) }, &ctl())
                .is_err()
        );
    }
    let after_fees = || q.usage().unwrap().index > before.index;
    let stop = Interrupt::cancel_only(&after_fees);
    assert!(
        old.try_with_readonly_layer_properties(
            ReadonlyLayerPropertiesPatch { layer: id, name: Some("changed"), visible: Some(false), opacity_f32_bits: None },
            &ReadControl { interrupt: &stop, work: &|_| Ok(()) }
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(old.document().layers[0].name, "original layer");
}
#[test]
fn repeated_title_and_combined_properties_share_original_flat_custody() {
    let old = owner();
    let q = old.quota().clone();
    let id = old.document().layers[0].id;
    let mut current = old.clone();
    let mut usage = None;
    for i in 0..32 {
        current = current.try_with_document_name(if i % 2 == 0 { "title A" } else { "title B" }, &ctl()).unwrap();
        current = current
            .try_with_readonly_layer_properties(
                ReadonlyLayerPropertiesPatch {
                    layer: id,
                    name: Some(if i % 2 == 0 { "layer A" } else { "layer B" }),
                    visible: Some(i % 2 == 0),
                    opacity_f32_bits: Some(0.5f32.to_bits()),
                },
                &ctl(),
            )
            .unwrap();
        if let Some(previous) = usage {
            assert_eq!(q.usage().unwrap(), previous);
        }
        usage = Some(q.usage().unwrap());
    }
    assert_eq!(current.document().layers[0].content, old.document().layers[0].content);
    assert_eq!(current.document().xcf_state.as_ref().unwrap().filter_stacks, old.document().xcf_state.as_ref().unwrap().filter_stacks);
    assert_eq!(current.document().source_color_context, old.document().source_color_context);
}
