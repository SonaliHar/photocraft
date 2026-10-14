//! Authorized-font import regression drafts, UNRUN.
use super::*;
use photocraft_text::xcf::runtime_font_input::{self as runtime, FaceIdentity, RuntimeInput};
fn font(bytes: &[u8]) -> PreparedRuntimeFont<'_> {
    runtime::prepare_with_charge(
        RuntimeInput { bytes, identity: FaceIdentity::inter_regular() },
        runtime::Limits::default(),
        &Interrupt::NONE,
        &mut |_| Ok(()),
        &mut |_| Ok(()),
    )
    .unwrap()
}
#[test]
fn lower_shared_font_ownership_refuses_before_decoding_or_materialization() {
    let owned = photocraft_text::fonts::INTER_REGULAR.to_vec();
    let font = font(&owned);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", font.requirements().borrowed_input_bytes - 1)])
        .unwrap();
    let shared = photocraft_xcf::Budget::new(limits, None);
    let result = import_candidate_with_budget_and_authorized_font(
        "not a font locator.xcf",
        b"gimp xcf v019\0",
        &Interrupt::NONE,
        &DecodeOptions::default(),
        &shared,
        photocraft_xcf::SourceAccounting::Borrowed,
        &font,
    );
    assert!(matches!(result,Err(IoError::Xcf(error)) if error.kind==ErrorKind::Limit));
    assert_eq!(shared.peak(photocraft_xcf::BudgetKind::Resident), 0);
    assert_eq!(shared.used(photocraft_xcf::BudgetKind::Work), 0);
}
#[test]
fn early_cancel_preserves_typed_failure_before_font_or_decoder_charge() {
    let owned = photocraft_text::fonts::INTER_REGULAR.to_vec();
    let font = font(&owned);
    let shared = photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None);
    let stop = || true;
    assert!(matches!(
        import_candidate_with_budget_and_authorized_font(
            "cancel.xcf",
            b"gimp xcf v019\0",
            &Interrupt::cancel_only(&stop),
            &DecodeOptions::default(),
            &shared,
            photocraft_xcf::SourceAccounting::Borrowed,
            &font
        ),
        Err(IoError::Cancelled)
    ));
    assert_eq!(shared.used(photocraft_xcf::BudgetKind::Resident), 0);
}
#[test]
fn valid_non_xcf_candidate_does_not_retain_font_charge_or_change_authorized_bytes() {
    let owned = photocraft_text::fonts::INTER_REGULAR.to_vec();
    let before = owned.clone();
    let font = font(&owned);
    let shared = photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None);
    assert!(
        import_candidate_with_budget_and_authorized_font(
            "misnamed.xcf",
            b"\x89PNG\r\n\x1a\n",
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &shared,
            photocraft_xcf::SourceAccounting::Borrowed,
            &font
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(shared.used(photocraft_xcf::BudgetKind::Resident), 0);
    assert_eq!(owned, before);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires pinned original rectilinear text controls; UNRUN"]
fn original_text_import_matches_default_and_does_not_embed_runtime_authority() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-evidence");
    let owned = photocraft_text::fonts::INTER_REGULAR.to_vec();
    let font = font(&owned);
    for name in ["rectilinear-controls/control-0.xcf", "rectilinear-pairs/shape-source.xcf"] {
        let bytes = std::fs::read(root.join(name)).unwrap();
        let baseline = import(name, &bytes, &Interrupt::NONE).unwrap();
        let supplied = import_with_authorized_font(name, &bytes, &font, &Interrupt::NONE).unwrap();
        assert_eq!(supplied.document, baseline.document);
        assert_eq!(supplied.origin, baseline.origin);
        assert_eq!(supplied.source_read_only, baseline.source_read_only);
        assert_eq!(supplied.preview_only, baseline.preview_only);
        let native = photocraft_format::save_to_bytes(&supplied.document, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), supplied.document);
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
    }
}
