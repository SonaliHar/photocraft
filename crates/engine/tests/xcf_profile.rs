//! Active profile interpretation is editable state, separate from source evidence.
use photocraft_cms::{
    Builtin, ColorSpace,
    xcf_color::{Encoding, effective_profile},
};
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleEncoding, SampleType, Size, SourceColorContext, WorkingProfileInterpretation};
use photocraft_engine::{Session, color_cmds};
use photocraft_raster::Surface;
use serde_json::json;

fn implicit_document() -> Document {
    let mut doc = Document::new("implicit", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let icc = effective_profile(None, ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap().to_bytes();
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&icc);
    doc.icc_profile = Some(icc.clone());
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 700 });
    let raw: Vec<_> = [0.02_f64, 0.19, 1.0 + f64::EPSILON, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect();
    doc.layers.push(Layer::new("source", LayerContent::Raster(Surface::try_from_interleaved(doc.pixel_format(), doc.bounds(), &raw).unwrap())));
    doc
}

#[test]
fn absent_encoding_matches_existing_profile_semantics_at_every_depth() {
    for depth in [SampleType::U8, SampleType::U32, SampleType::F32, SampleType::F64] {
        for assigned in [false, true] {
            let mut doc = Document::new("legacy", Size::new(1, 1), ColorMode::Rgb, depth);
            if assigned {
                doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
            }
            let checked = color_cmds::try_document_profile(&doc).unwrap();
            assert!(checked.same_colors(&color_cmds::document_profile(&doc)), "{depth:?}/{assigned}");
        }
    }
}

#[test]
fn explicit_same_byte_assignment_resets_interpretation_and_undo_restores_it() {
    let original = implicit_document();
    let mut session = Session::new();
    session.open_document(original.clone(), None).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
    session.execute("edit.assignProfile", json!({"profile":"srgb"})).unwrap();
    let assigned = &session.active().unwrap().doc;
    assert_eq!(assigned.icc_profile, original.icc_profile);
    assert_eq!(assigned.working_profile_interpretation, WorkingProfileInterpretation::Icc);
    assert_eq!(assigned.source_color_context, original.source_color_context);
    assert_eq!(assigned.layers, original.layers);
    let native = photocraft_format::save_to_bytes(assigned, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **assigned);
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, original);
    let native = photocraft_format::save_to_bytes(&original, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened, original);
    color_cmds::try_document_profile(&reopened).unwrap();
}

#[test]
fn conversion_and_generic_profile_changes_keep_source_evidence_and_clear_marker() {
    let original = implicit_document();
    let mut session = Session::new();
    session.open_document(original.clone(), None).unwrap();
    session.execute("edit.convertToProfile", json!({"profile":"display-p3"})).unwrap();
    assert_eq!(session.active().unwrap().doc.working_profile_interpretation, WorkingProfileInterpretation::Icc);
    assert_eq!(session.active().unwrap().doc.source_color_context, original.source_color_context);
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, original);
    session
        .edit("profile replacement", |doc, _| {
            doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
            Ok(())
        })
        .unwrap();
    assert_eq!(session.active().unwrap().doc.working_profile_interpretation, WorkingProfileInterpretation::Icc);
    assert_eq!(session.active().unwrap().doc.source_color_context, original.source_color_context);
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, original);
}

#[test]
fn self_consistent_hash_cannot_admit_an_arbitrary_implicit_profile() {
    let mut forged = implicit_document();
    let arbitrary = Builtin::DisplayP3.profile().to_bytes();
    forged.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&arbitrary);
    forged.icc_profile = Some(arbitrary);
    forged.validate_xcf_state().unwrap(); // DTO validates identity; CMS validates meaning.
    assert!(color_cmds::try_document_profile(&forged).is_err());
}
