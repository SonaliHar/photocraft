#![cfg(feature = "corpus")]
//! Encoding-bound analytic epochs validate current definitions, never source provenance.
use photocraft_compose::gimp_render::PreparedRenderer;
use photocraft_doc::SampleEncoding;
use photocraft_engine::Session;
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};
#[test]
#[ignore = "requires reviewed unpublished semantic analytic encoding originals"]
fn geometry_encoding_epochs_preserve_current_interpretation_and_reject_profile_stage_resets() {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let temporary = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp").join(format!("sol4-geometry-encoding-current-{}", std::process::id()));
    fs::create_dir_all(&temporary).unwrap();
    let mut count = 0;
    for (encoding, stored, epoch) in [
        ("linear", SampleEncoding::Linear, "xcf12-analytic-linear-normal-mask-geometry-v1"),
        ("perceptual", SampleEncoding::Perceptual, "xcf12-analytic-perceptual-normal-mask-geometry-v1"),
    ] {
        for variant in ["left-top", "right-bottom"] {
            for model in ["rgb", "gray"] {
                for depth in ["f32", "f64"] {
                    let packet = base.join(format!("normal-mask-geometry-{encoding}-evidence/{variant}"));
                    let source_path = packet.join(format!("{model}-{depth}-op2.xcf"));
                    let source = fs::read(&source_path).unwrap();
                    let original = photocraft_io::import("encoding.xcf", &source).unwrap().document;
                    assert_eq!(original.sample_encoding, Some(stored));
                    assert!(
                        original
                            .xcf_state
                            .as_ref()
                            .unwrap()
                            .items
                            .iter()
                            .any(|item| item.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == epoch))
                    );
                    PreparedRenderer::new(&original).unwrap();
                    let mut oversized = original.clone();
                    oversized.size = photocraft_doc::Size::new(65, 65);
                    assert!(PreparedRenderer::new(&oversized).is_err(), "encoding epoch retains bounded geometry");
                    let mut inert = original.clone();
                    inert.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![1, 2, 3]));
                    PreparedRenderer::new(&inert).unwrap();
                    let owner = original.layers.iter().find(|layer| layer.name == "Own Normal masked foreground").unwrap().id;
                    let mut edit = Session::new();
                    edit.open_document(original.clone(), None).unwrap();
                    edit.execute("xcf.composition.set",json!({"layerId":owner.0.to_string(),"sourceMode":28,"legacy":false,"blendSpace":"linear-rgb","compositeSpace":"linear-rgb","operator":"union"})).unwrap();
                    let edited = (*edit.active().unwrap().doc).clone();
                    assert!(
                        edited
                            .xcf_state
                            .as_ref()
                            .unwrap()
                            .items
                            .iter()
                            .any(|item| item.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == epoch))
                    );
                    let mut bad_after_edit = edited.clone();
                    bad_after_edit.icc_profile = Some(Arc::new(fs::read(packet.join(format!("own-{model}-parametric-trc.icc"))).unwrap()));
                    assert!(PreparedRenderer::new(&bad_after_edit).is_err(), "Normal operator edits cannot erase current transfer constraints");
                    assert!(edit.undo());
                    assert_eq!(*edit.active().unwrap().doc, original);
                    let mut wrong = original.clone();
                    wrong.icc_profile = Some(Arc::new(fs::read(packet.join(format!("own-{model}-parametric-trc.icc"))).unwrap()));
                    let before = wrong.clone();
                    assert!(PreparedRenderer::new(&wrong).is_err(), "current source-style TRCs cannot substitute for effective encoding");
                    assert_eq!(wrong, before);
                    for altered in [
                        None,
                        Some(SampleEncoding::NonlinearProfile),
                        Some(if stored == SampleEncoding::Linear { SampleEncoding::Perceptual } else { SampleEncoding::Linear }),
                    ] {
                        let mut wrong = original.clone();
                        wrong.sample_encoding = altered;
                        assert!(PreparedRenderer::new(&wrong).is_err(), "epoch must bind the current encoding");
                    }
                    let profile_path = temporary.join(format!("{encoding}-{variant}-{model}-{depth}.icc"));
                    fs::write(&profile_path, original.icc_profile.as_ref().unwrap().as_slice()).unwrap();
                    for (command, params) in [
                        ("edit.assignProfile", json!({"profile":profile_path})),
                        ("edit.convertToProfile", json!({"profile":profile_path,"intent":"relative","bpc":false})),
                        ("edit.assignProfile", json!({"profile":"none"})),
                        ("color.profileMismatch", json!({"action":"discard"})),
                    ] {
                        let mut session = Session::new();
                        session.open_document(original.clone(), None).unwrap();
                        assert!(session.execute(command, params).is_err(), "{encoding}/{variant}/{model}/{depth}/{command}: no unproved NL reset");
                        assert_eq!(*session.active().unwrap().doc, original);
                        assert!(!session.undo());
                    }
                    let mut off = Session::new();
                    off.color.settings.policy_rgb = photocraft_engine::color_cmds::Policy::Off;
                    off.color.settings.policy_gray = photocraft_engine::color_cmds::Policy::Off;
                    let (_, report) = off.open_document(original.clone(), None).unwrap();
                    assert_eq!(report["action"], "kept");
                    // Existing matched-working/F32 linear-HDR openings already keep
                    // the profile. A warning is required only for a blocked discard.
                    let current = photocraft_cms::Profile::parse(original.icc_profile.as_ref().unwrap()).unwrap();
                    let kept_linear_hdr =
                        original.depth == photocraft_doc::SampleType::F32 && current.same_colors(photocraft_cms::Builtin::LinearSrgb.profile());
                    if report["mismatch"] == true && !kept_linear_hdr {
                        assert!(report["warning"].as_str().is_some_and(|s| !s.is_empty()));
                    }
                    assert_eq!(*off.active().unwrap().doc, original);
                    assert!(!off.undo());
                    for current in [None, Some(Arc::new(vec![1, 2, 3]))] {
                        let mut broken = original.clone();
                        broken.icc_profile = current;
                        assert!(PreparedRenderer::new(&broken).is_err());
                        let mut session = Session::new();
                        let (_, report) = session.open_document(broken.clone(), None).unwrap();
                        assert_eq!(report["action"], "kept");
                        assert!(report["warning"].as_str().is_some_and(|s| !s.is_empty()));
                        assert_eq!(*session.active().unwrap().doc, broken);
                        assert!(!session.undo());
                    }
                    let native = photocraft_format::save_to_bytes(&original, &Default::default()).unwrap();
                    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), original);
                    assert_eq!(fs::read(&source_path).unwrap(), source);
                    fs::remove_file(profile_path).unwrap();
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 16);
    fs::remove_dir(temporary).unwrap();
}
