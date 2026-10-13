#![cfg(feature = "corpus")]
//! A generic source proof must never override active current profile semantics.
use photocraft_cms::{Profile, curve::Curve};
use photocraft_compose::gimp_render::PreparedRenderer;
use photocraft_engine::Session;
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};

#[test]
#[ignore = "requires reviewed unpublished analytic profile class originals"]
fn analytic_epoch_revalidates_current_profiles_and_profile_commands_atomically() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("normal-mask-analytic-class-evidence");
    let temp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp").join(format!("sol4-analytic-current-{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    for model in ["rgb", "gray"] {
        for depth in ["f32", "f64"] {
            let input = fs::read(root.join("gamma").join(format!("{model}-{depth}-op2.xcf"))).unwrap();
            let original = photocraft_io::import("current.xcf", &input).unwrap().document;
            assert!(
                original
                    .xcf_state
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .any(|item| item.composition.as_ref().is_some_and(|p| p.compatibility_epoch == "xcf12-analytic-normal-mask-v1"))
            );
            let mut off = Session::new();
            off.color.settings.policy_rgb = photocraft_engine::color_cmds::Policy::Off;
            off.color.settings.policy_gray = photocraft_engine::color_cmds::Policy::Off;
            let (_, report) = off.open_document(original.clone(), None).unwrap();
            assert_eq!(report["action"], "kept");
            assert!(report["warning"].as_str().is_some_and(|warning| !warning.is_empty()));
            assert_eq!(*off.active().unwrap().doc, original);
            assert!(!off.undo());
            let current = fs::read(root.join("type3").join(format!("own-{model}-parametric-trc.icc"))).unwrap();
            let parsed = Profile::parse(&current).unwrap();
            photocraft_cms::xcf_color::qualify_analytic_profile(&parsed).unwrap();
            let mut alternative = original.clone();
            alternative.icc_profile = Some(Arc::new(current));
            alternative.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![1, 2, 3]));
            PreparedRenderer::new(&alternative).unwrap(); // malformed provenance is inert
            let mut outside = parsed;
            match model {
                "rgb" => outside.trc = Some(std::array::from_fn(|_| Curve::Table64(vec![0.0, 0.4, 1.0]))),
                _ => outside.gray_trc = Some(Curve::Table64(vec![0.0, 0.4, 1.0])),
            }
            let bytes = outside.with_encoded_bytes().to_bytes();
            let file = temp.join(format!("outside-{model}-{depth}.icc"));
            fs::write(&file, bytes.as_slice()).unwrap();
            let mut corrupted = original.clone();
            corrupted.icc_profile = Some(bytes);
            let before = corrupted.clone();
            assert!(PreparedRenderer::new(&corrupted).is_err());
            assert_eq!(corrupted, before);
            for (command, params) in [
                ("edit.assignProfile", json!({"profile":file})),
                ("edit.convertToProfile", json!({"profile":file,"intent":"relative","bpc":false})),
                ("edit.assignProfile", json!({"profile":"none"})),
                ("color.profileMismatch", json!({"action":"discard"})),
            ] {
                let mut session = Session::new();
                session.open_document(original.clone(), None).unwrap();
                assert!(session.execute(command, params).is_err(), "{model}/{depth} {command} must fail before installation");
                assert_eq!(*session.active().unwrap().doc, original);
                assert!(!session.undo());
            }
            fs::remove_file(file).unwrap();
        }
    }
    fs::remove_dir(temp).unwrap();
}

#[test]
#[ignore = "requires reviewed unpublished analytic profile class originals"]
fn analytic_epoch_malformed_or_missing_current_opening_never_assigns_working() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("normal-mask-analytic-class-evidence");
    for model in ["rgb", "gray"] {
        let input = fs::read(root.join("gamma").join(format!("{model}-f64-op1.xcf"))).unwrap();
        let original = photocraft_io::import("current.xcf", &input).unwrap().document;
        let target = root.join("type3").join(format!("own-{model}-parametric-trc.icc"));
        for bytes in [None, Some(Arc::new(vec![1, 2, 3]))] {
            let mut broken = original.clone();
            broken.icc_profile = bytes;
            assert!(PreparedRenderer::new(&broken).is_err());
            let mut session = Session::new();
            if model == "rgb" {
                session.color.settings.working_rgb = target.to_string_lossy().into_owned();
            } else {
                session.color.settings.working_gray = target.to_string_lossy().into_owned();
            }
            let (_, report) = session.open_document(broken.clone(), None).unwrap();
            assert_eq!(report["action"], "kept", "{model}: implicit working assignment must not replace missing/current-invalid ICC");
            assert!(report["warning"].as_str().is_some_and(|warning| !warning.is_empty()));
            assert_eq!(*session.active().unwrap().doc, broken);
            assert!(!session.undo());
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished analytic boundary profiles"]
fn analytic_boundary_current_profile_identity_and_outside_class_are_atomic() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("normal-mask-analytic-boundary-evidence");
    let temporary = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp").join(format!("sol4-boundary-current-{}", std::process::id()));
    fs::create_dir_all(&temporary).unwrap();
    for variant in ["edge-gamma", "edge-piecewise"] {
        for model in ["rgb", "gray"] {
            for depth in ["f32", "f64"] {
                let path = root.join(variant).join(format!("{model}-{depth}-op2.xcf"));
                let source = fs::read(&path).unwrap();
                let original = photocraft_io::import("boundary.xcf", &source).unwrap().document;
                let current = original.icc_profile.as_ref().unwrap();
                let profile_path = temporary.join(format!("{variant}-{model}-{depth}.icc"));
                fs::write(&profile_path, current.as_slice()).unwrap();
                let mut session = Session::new();
                session.open_document(original.clone(), None).unwrap();
                session.execute("edit.convertToProfile", json!({"profile":profile_path,"intent":"relative","bpc":false})).unwrap();
                let converted = (*session.active().unwrap().doc).clone();
                for layer in &original.layers {
                    let after = converted.layer(layer.id).unwrap();
                    assert_eq!(after.surface(), layer.surface(), "identity current profile preserves native samples/defaults");
                    assert_eq!(after.mask, layer.mask, "identity current profile preserves masks");
                }
                assert_eq!(converted.icc_profile, original.icc_profile);
                assert_eq!(converted.source_color_context, original.source_color_context);
                PreparedRenderer::new(&converted).unwrap();
                let native = photocraft_format::save_to_bytes(&converted, &Default::default()).unwrap();
                assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), converted);
                assert!(session.undo());
                assert_eq!(*session.active().unwrap().doc, original);
                assert!(session.redo());
                assert_eq!(*session.active().unwrap().doc, converted);
                let mut outside = Profile::parse(current).unwrap();
                if model == "gray" {
                    outside.gray_trc = Some(Curve::Gamma(4.0));
                } else {
                    outside.trc = Some(std::array::from_fn(|_| Curve::Gamma(4.0)));
                }
                let bad = outside.with_encoded_bytes().to_bytes();
                let bad_path = temporary.join(format!("outside-{variant}-{model}-{depth}.icc"));
                fs::write(&bad_path, bad.as_slice()).unwrap();
                let mut invalid_current = original.clone();
                invalid_current.icc_profile = Some(bad);
                assert!(PreparedRenderer::new(&invalid_current).is_err());
                for command in ["edit.assignProfile", "edit.convertToProfile"] {
                    let mut atomic = Session::new();
                    atomic.open_document(original.clone(), None).unwrap();
                    assert!(atomic.execute(command, json!({"profile":bad_path,"intent":"relative","bpc":false})).is_err());
                    assert_eq!(*atomic.active().unwrap().doc, original);
                    assert!(!atomic.undo());
                }
                assert_eq!(fs::read(&path).unwrap(), source);
                fs::remove_file(profile_path).unwrap();
                fs::remove_file(bad_path).unwrap();
            }
        }
    }
    fs::remove_dir(temporary).unwrap();
}
