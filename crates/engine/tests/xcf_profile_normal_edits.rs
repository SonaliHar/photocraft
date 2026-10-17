#![cfg(feature = "corpus")]
//! Registered commands against independently fresh public GIMP controls.
use photocraft_doc::{Document, Rect, item_identity::ItemRef};
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn read(root: &Path, blob: &Value) -> Vec<u8> {
    let b = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(sha(&b), blob["sha256"]);
    assert_eq!(b.len(), blob["bytes"].as_u64().unwrap() as usize);
    b
}
fn rect(v: &Value) -> Rect {
    Rect::from_xywh(
        v["offset"][0].as_i64().unwrap() as i32,
        v["offset"][1].as_i64().unwrap() as i32,
        v["size"][0].as_u64().unwrap() as u32,
        v["size"][1].as_u64().unwrap() as u32,
    )
}
fn appearance(doc: &Document, root: &Path, row: &Value) {
    let scene = photocraft_compose::gimp_render::PreparedRenderer::requires_preparation_budget(doc).then(|| {
        photocraft_compose::gimp_render::scene_budget::prepare(
            doc,
            &photocraft_raster::Interrupt::NONE,
            photocraft_compose::gimp_render::scene_budget::SceneLimits { resident_bytes: 512 << 20, work_units: 36_000_000 * 512 },
        )
        .unwrap()
    });
    let charged = scene.as_ref().map(|scene| scene.render(doc.bounds(), &photocraft_raster::Interrupt::NONE, &mut |_| Ok(())).unwrap());
    let ordinary = scene.is_none().then(|| photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap());
    let actual = charged.as_ref().map(|value| value.buffer()).or(ordinary.as_ref()).unwrap();
    let gray = row["model"] == "gray";
    let n = if gray { 2 } else { 4 };
    let current = photocraft_cms::Profile::parse(doc.icc_profile.as_ref().unwrap()).unwrap();
    // Interpret the declared transfer from CURRENT ICC primaries, without a
    // second serialization-rounded transfer or immutable-source substitution.
    let encoding = match doc.sample_encoding {
        Some(photocraft_doc::SampleEncoding::Linear) => photocraft_cms::xcf_color::Encoding::Linear,
        Some(photocraft_doc::SampleEncoding::Perceptual) => photocraft_cms::xcf_color::Encoding::Perceptual,
        _ => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
    };
    let profile = photocraft_cms::xcf_color::effective_profile_from_definition(&current, encoding).unwrap();
    let target = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
    let transform = photocraft_cms::Transform::new(&profile, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
    let reference = read(root, &row["projection"]["nonlinear_double"]);
    // The bound public oracle declares Half projection precision. Preserve the
    // prior alpha gate for every existing non-Half packet.
    let alpha_tolerance = if row["precision"] == "GIMP_PRECISION_HALF_NON_LINEAR" { 2.0_f64.powi(-12) + 1.0 / 65535.0 } else { 1.0 / 65535.0 };
    for (pixel, raw) in actual.px.iter().zip(reference.chunks_exact(n * 8)) {
        let a = f64::from_le_bytes(raw[(n - 1) * 8..n * 8].try_into().unwrap());
        assert!((pixel[3] - a).abs() <= alpha_tolerance);
        let mut converted = [0.; 3];
        transform.try_eval_f64(&pixel[..n - 1], &mut converted[..n - 1]).unwrap();
        for (channel, v) in converted.iter().enumerate().take(n - 1) {
            let r = f64::from_le_bytes(raw[channel * 8..channel * 8 + 8].try_into().unwrap());
            if a > 0. {
                assert!((*v - r).abs() <= 2. / 255., "{} visible", row["id"]);
            }
            assert!((*v * pixel[3] - r * a).abs() <= 2. / 255.);
        }
    }
}
#[test]
#[ignore = "requires reviewed unpublished profile command controls"]
fn profile_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-profile-command-controls", include_str!("xcf_profile_normal_edit_hashes.json"), 64);
}
#[test]
#[ignore = "requires reviewed unpublished profile transfer command controls"]
fn profile_transfer_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-profile-transfer-command-controls", include_str!("xcf_profile_transfer_normal_edit_hashes.json"), 64);
}
#[test]
#[ignore = "requires reviewed unpublished analytic class command controls"]
fn analytic_class_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-analytic-class-command-controls", include_str!("xcf_analytic_class_normal_edit_hashes.json"), 96);
}
#[test]
#[ignore = "requires reviewed unpublished analytic boundary command controls"]
fn analytic_boundary_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-analytic-boundary-command-controls", include_str!("xcf_analytic_boundary_normal_edit_hashes.json"), 64);
}
#[test]
#[ignore = "requires reviewed unpublished analytic encoding command controls"]
fn analytic_encoding_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-analytic-encoding-command-controls", include_str!("xcf_analytic_encoding_normal_edit_hashes.json"), 192);
}
#[test]
#[ignore = "requires reviewed unpublished Normal geometry controls"]
fn geometry_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-geometry-command-controls", include_str!("xcf_geometry_normal_edit_hashes.json"), 64);
}
#[test]
#[ignore = "requires reviewed unpublished Normal geometry encoding controls"]
fn geometry_encoding_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-geometry-encoding-command-controls", include_str!("xcf_geometry_encoding_normal_edit_hashes.json"), 128);
}
#[test]
#[ignore = "requires reviewed unpublished integer Normal geometry controls"]
fn integer_geometry_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-geometry-integer-command-controls", include_str!("xcf_geometry_integer_normal_edit_hashes.json"), 64);
}
#[test]
#[ignore = "requires reviewed unpublished ordinary scene command controls"]
fn ordinary_scene_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-ordinary-scene-command-controls", include_str!("xcf_ordinary_scene_normal_edit_hashes.json"), 64);
}
#[test]
#[ignore = "requires reviewed unpublished ordinary Half scene controls"]
fn ordinary_half_scene_sources_registered_commands_native_history_and_undo() {
    verify_controls("normal-mask-ordinary-half-scene-command-controls", include_str!("xcf_ordinary_half_scene_normal_edit_hashes.json"), 16);
}
fn verify_controls(directory: &str, frozen_hashes: &str, expected_count: usize) {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let controls = base.join(directory);
    let bytes = fs::read(controls.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(frozen_hashes).unwrap();
    for (file, expected) in hashes.as_object().unwrap() {
        assert_eq!(sha(&fs::read(controls.join(file)).unwrap()), expected.as_str().unwrap());
    }
    assert_eq!(sha(&bytes), hashes["observations.json"]);
    let rows: Value = serde_json::from_slice(&bytes).unwrap();
    let mut count = 0;
    for row in rows["records"].as_array().unwrap() {
        let packet = base.join(row["origin"]["packet"].as_str().unwrap());
        let raw = fs::read(packet.join(row["origin"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(sha(&raw), row["origin"]["sha256"]);
        let original = photocraft_io::import("original.xcf", &raw).unwrap().document;
        let target = &row["layers"][0];
        let owner = original.layers.iter().find(|l| l.name == target["name"].as_str().unwrap()).unwrap().id;
        let item = original.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(owner)).unwrap();
        let source_bounds = item.raster_bounds.unwrap();
        let target_bounds = rect(target);
        let pixels = original.layer(owner).unwrap().surface().unwrap().try_read_interleaved(source_bounds).unwrap();
        let mask = original.layer(owner).unwrap().mask.as_ref().unwrap().surface.try_read_interleaved(source_bounds).unwrap();
        let opaque = original.xcf_state.as_ref().unwrap().opaque.clone();
        let mut session = Session::new();
        session.open_document(original.clone(), None).unwrap();
        session.execute("layer.translate", json!({"layer":owner.0,"dx":target_bounds.x0-source_bounds.x0,"dy":target_bounds.y0-source_bounds.y0})).unwrap();
        session.execute("layer.setProps", json!({"layer":owner.0,"opacity":target["opacity"].as_f64().unwrap()/100.})).unwrap();
        let operator = match target["spaces"]["composite_mode"].as_i64().unwrap() {
            1 => "union",
            2 => "clip-to-backdrop",
            3 => "clip-to-layer",
            4 => "intersection",
            _ => panic!("unqualified control"),
        };
        session.execute("xcf.composition.set",json!({"layerId":owner.0.to_string(),"sourceMode":28,"legacy":false,"blendSpace":"linear-rgb","compositeSpace":"linear-rgb","operator":operator})).unwrap();
        let edited = (*session.active().unwrap().doc).clone();
        assert_eq!(edited.source_color_context, original.source_color_context);
        assert_eq!(edited.icc_profile, original.icc_profile);
        assert_eq!(edited.working_profile_interpretation, original.working_profile_interpretation);
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, opaque);
        assert_eq!(edited.layer(owner).unwrap().surface().unwrap().try_read_interleaved(target_bounds).unwrap(), pixels);
        assert_eq!(edited.layer(owner).unwrap().mask.as_ref().unwrap().surface.try_read_interleaved(target_bounds).unwrap(), mask);
        appearance(&edited, &controls, row);
        let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened, edited);
        appearance(&reopened, &controls, row);
        assert!(session.execute("xcf.composition.set",json!({"layerId":owner.0.to_string(),"sourceMode":28,"legacy":true,"blendSpace":"linear-rgb","compositeSpace":"linear-rgb","operator":operator})).is_err());
        assert_eq!(*session.active().unwrap().doc, edited);
        for _ in 0..3 {
            assert!(session.undo());
        }
        assert_eq!(*session.active().unwrap().doc, original);
        for _ in 0..3 {
            assert!(session.redo());
        }
        assert_eq!(*session.active().unwrap().doc, edited);
        assert_eq!(fs::read(packet.join(row["origin"]["file"].as_str().unwrap())).unwrap(), raw);
        count += 1;
    }
    assert_eq!(count, expected_count);
}

#[test]
#[ignore = "requires reviewed unpublished analytic current-profile controls"]
fn analytic_class_current_assign_and_convert_match_independent_controls() {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let controls = base.join("normal-analytic-current-profile-controls");
    let hashes: Value = serde_json::from_str(include_str!("xcf_analytic_current_profile_hashes.json")).unwrap();
    for (file, expected) in hashes.as_object().unwrap() {
        assert_eq!(sha(&fs::read(controls.join(file)).unwrap()), expected.as_str().unwrap());
    }
    let rows: Value = serde_json::from_slice(&fs::read(controls.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    let mut worst_native_color = 0.0f64;
    for row in rows["records"].as_array().unwrap() {
        let origin = base.join(row["origin"]["packet"].as_str().unwrap());
        let source_file = origin.join(row["origin"]["file"].as_str().unwrap());
        let source = fs::read(&source_file).unwrap();
        assert_eq!(sha(&source), row["origin"]["sha256"]);
        let original = photocraft_io::import("original.xcf", &source).unwrap().document;
        let target = &row["operation"]["target_profile"];
        let profile_file = base.join(target["packet"].as_str().unwrap()).join(target["file"].as_str().unwrap());
        let profile = fs::read(&profile_file).unwrap();
        assert_eq!(sha(&profile), target["sha256"]);
        let assigning = row["operation"]["action"] == "assign";
        let mut session = Session::new();
        session.open_document(original.clone(), None).unwrap();
        let parameters = if assigning { json!({"profile":profile_file}) } else { json!({"profile":profile_file,"intent":"relative","bpc":false}) };
        session.execute(if assigning { "edit.assignProfile" } else { "edit.convertToProfile" }, parameters).unwrap();
        let edited = (*session.active().unwrap().doc).clone();
        assert_eq!(edited.icc_profile.as_deref().unwrap().as_slice(), profile.as_slice());
        assert_eq!(edited.source_color_context, original.source_color_context);
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, original.xcf_state.as_ref().unwrap().opaque);
        for expected in row["layers"].as_array().unwrap() {
            let layer = edited.layers.iter().find(|l| l.name == expected["name"].as_str().unwrap()).unwrap();
            let previous = original.layer(layer.id).unwrap();
            let area = rect(expected);
            let surface = layer.surface().unwrap();
            let raw = surface.try_read_interleaved(area).unwrap();
            let previous_raw = previous.surface().unwrap().try_read_interleaved(area).unwrap();
            let reference = read(&controls, &expected["native"]);
            if assigning {
                assert_eq!(raw, previous_raw);
                assert_eq!(raw, reference);
            } else {
                let format = surface.format();
                let sample_bytes = format.sample.bytes();
                let color_channels = format.mode.color_channels();
                let pixel_bytes = format.bytes_per_pixel();
                for ((actual, before), independent) in
                    raw.chunks_exact(pixel_bytes).zip(previous_raw.chunks_exact(pixel_bytes)).zip(reference.chunks_exact(pixel_bytes))
                {
                    assert_eq!(&actual[color_channels * sample_bytes..], &before[color_channels * sample_bytes..]);
                    assert_eq!(&actual[color_channels * sample_bytes..], &independent[color_channels * sample_bytes..]);
                    for channel in 0..color_channels {
                        let value = photocraft_color::try_read_sample_f64(actual, format.sample, channel).unwrap();
                        let expected_value = photocraft_color::try_read_sample_f64(independent, format.sample, channel).unwrap();
                        worst_native_color = worst_native_color.max((value - expected_value).abs());
                    }
                }
            }
            if expected["mask"].is_object() {
                let mask = layer.mask.as_ref().unwrap().surface.try_read_interleaved(area).unwrap();
                assert_eq!(mask, previous.mask.as_ref().unwrap().surface.try_read_interleaved(area).unwrap());
                assert_eq!(mask, read(&controls, &expected["mask"]["native"]));
            }
        }
        appearance(&edited, &controls, row);
        let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened, edited);
        appearance(&reopened, &controls, row);
        assert!(session.undo());
        assert_eq!(*session.active().unwrap().doc, original);
        assert!(session.redo());
        assert_eq!(*session.active().unwrap().doc, edited);
        assert_eq!(fs::read(source_file).unwrap(), source);
        count += 1;
    }
    assert_eq!(count, 32);
    eprintln!("analytic current-profile derived native color maximum: {worst_native_color}");
}
