#![cfg(feature = "corpus")]
//! Independently authored expanded Channel Mixer current edits, native state and history.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn packet() -> (PathBuf, Value) {
    let base = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let root = base.join("channel-mixer-precision-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "91be8128c1d351336cd7ef304dd81d42b010aedba1610384aed9526a7ca04f20");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let obs = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&obs), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap()["sha256"]);
    (root, serde_json::from_slice(&obs).unwrap())
}
fn import(root: &std::path::Path, row: &Value) -> photocraft_doc::Document {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("source.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "local exact Channel Double malformed and out-of-domain atomic controls"]
fn channel_exact_double_hostile_wire_is_atomic() {
    let (root, obs) = packet();
    let row = obs["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb.float_non_linear.source").unwrap();
    let source = import(&root, row);
    let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let values = [
        json!(f64::NAN.to_bits()),
        json!("0x7ff8000000000000"),
        json!("0x7ff0000000000000"),
        json!("0xfff0000000000000"),
        json!(format!("0x{:016x}", 2.01_f64.to_bits())),
        json!(format!("0x{:016x}", (-2.01_f64).to_bits())),
        json!("0x3fe000000000000"),
        json!("0x3FE0000000000000"),
        json!("0xnot-a-number000"),
        json!(0.5),
    ];
    for value in values {
        let mut arguments = serde_json::to_value(&node.arguments).unwrap();
        let argument = arguments.as_array_mut().unwrap().iter_mut().find(|a| a["name"] == "rr-gain").unwrap();
        argument["value"] = json!({"Number":{"Float64Bits":value}});
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        assert!(session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":arguments})).is_err());
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), before);
    }
}
#[test]
#[ignore = "local independent expanded Channel Mixer144 frozen source references"]
fn channel_mixer_signed_registered_current_references_native_history_and_atomic_guards() {
    let (root, obs) = packet();
    let rows = obs["records"].as_array().unwrap();
    let mut edits = 0;
    let mut negatives = 0;
    for row in rows.iter().filter(|r| r["action"] == "source") {
        let source = import(&root, row);
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node));
        let fid = node.id.0.to_string();
        let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        for stage in ["signed", "preserve", "disabled"] {
            let targetid = format!("{family}.{stage}");
            let target = import(&root, rows.iter().find(|r| r["id"] == targetid).unwrap());
            let targetnode = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            if stage == "disabled" {
                session.execute("xcf.filter.enable", json!({"filterId":fid,"enabled":false})).unwrap();
            } else {
                session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":targetnode.arguments})).unwrap();
            }
            let edited = (*session.active().unwrap().doc).clone();
            let current = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            assert_eq!(current.opaque, original.opaque);
            assert_eq!(current.provenance, original.provenance);
            assert_eq!(current.filter_masks, original.filter_masks);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, targetnode.arguments);
            assert_eq!(current.filter_stacks[0].nodes[0].enabled, targetnode.enabled);
            let rendered = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
            let expected = photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px, expected.px, "{id}/{stage}");
            let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
            assert_eq!(session.active().unwrap().history.past_len(), 1);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, source);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, edited);
            edits += 1;
        }
        let mut arguments = node.arguments.clone();
        arguments.iter_mut().find(|argument| argument.name == "rr-gain").unwrap().value = photocraft_doc::xcf_filters::FilterValue::Bool(true);
        for params in [
            json!({"filterId":fid,"arguments":arguments}),
            json!({"filterId":fid,"arguments":[{"name":"rr-gain","value":{"Number":{"Float32Bits":f32::NAN.to_bits()}}}]}),
            json!({"filterId":fid,"arguments":[{"name":"preserve-luminosity","value":{"Number":{"Float32Bits":0}}}]}),
            json!({"filterId":fid,"arguments":[{"name":"unknown","value":{"Bool":true}}]}),
        ] {
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            assert!(session.execute("xcf.filter.set", params).is_err());
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            negatives += 1;
        }
        let mut cancelled = Session::new();
        cancelled.add_document(source.clone(), None).unwrap();
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut cancelled,
                &json!({"filterId":fid,"arguments":node.arguments}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
        assert_eq!(photocraft_format::save_to_bytes(&cancelled.active().unwrap().doc, &Default::default()).unwrap(), before);
        for mutation in 0..4 {
            let mut bad = source.clone();
            let state = bad.xcf_state.as_mut().unwrap();
            match mutation {
                0 => state.filter_stacks[0].nodes[0].operation.schema_epoch = "unknown".into(),
                1 => state.filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0,
                2 => state.items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(bad.layers[0].id)).unwrap().locks.content = true,
                _ => state.filter_stacks[0].nodes[0].operation.stored_version = "future".into(),
            }
            let mut session = Session::new();
            session.add_document(bad.clone(), None).unwrap();
            assert!(session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":node.arguments})).is_err());
            assert_eq!(*session.active().unwrap().doc, bad);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            negatives += 1;
        }
    }
    assert_eq!(edits, 108);
    assert_eq!(negatives, 288);
}

#[test]
#[ignore = "independent public Half-to-Float24 controls and Channel activation"]
fn current_half_promotion_and_activation_use_mutable_precision_not_provenance() {
    use photocraft_doc::FilterOutputPrecision;
    let (root, obs) = packet();
    let promotion = root.with_file_name("channel-mixer-half-promotion-evidence");
    let controls: Value = serde_json::from_slice(&fs::read(promotion.join("observations.json")).unwrap()).unwrap();
    let fresh: Value = serde_json::from_slice(&fs::read(promotion.join("fresh-observations.json")).unwrap()).unwrap();
    let mut promoted = 0;
    for row in controls["records"].as_array().unwrap() {
        let original = obs["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
        let source = import(&root, original);
        assert_eq!(source.xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Half);
        let bytes = fs::read(promotion.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let target = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(target.sample, photocraft_xcf::Sample::F32);
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("image.mode.bits32", json!({})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        assert_eq!(current.layers, source.layers);
        assert_eq!(current.source_color_context, source.source_color_context);
        assert_eq!(current.xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Canonical);
        let rendered = photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(current.layers[0].surface().unwrap().try_read_interleaved(current.bounds()).unwrap(), target.layers[0].plane.as_ref().unwrap().bytes);
        let repeat = fresh["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
        let projection = &repeat["current"]["projection"];
        let bytes = fs::read(promotion.join(projection["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), projection["sha256"]);
        let profile = if current.mode == photocraft_doc::ColorMode::Grayscale {
            photocraft_cms::Builtin::SGray.profile()
        } else {
            photocraft_cms::Builtin::Srgb.profile()
        };
        let channels = profile.channels();
        for (pixel, reference) in rendered.px.iter().zip(bytes.as_chunks::<32>().0.iter()) {
            let mut color = [0.; 3];
            photocraft_cms::xcf_color::convert_encoding_f64(
                profile,
                photocraft_compose::gimp_composite::encoding(current.sample_encoding.unwrap()),
                photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                &pixel[..channels],
                &mut color[..channels],
            )
            .unwrap();
            if channels == 1 {
                color = [color[0]; 3];
            }
            let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
            assert!((pixel[3] - alpha).abs() <= 1. / 65535.);
            for (i, a) in color.iter().enumerate() {
                let b = f64::from_le_bytes(reference[i * 8..i * 8 + 8].try_into().unwrap());
                assert!((a * pixel[3] - b * alpha).abs() <= 2. / 255., "{} public projection", row["id"]);
            }
        }
        let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), current);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        promoted += 1;
    }
    assert_eq!(promoted, 6);
    let mut enabled = 0;
    for row in obs["records"].as_array().unwrap().iter().filter(|r| r["action"] == "disabled") {
        let source = import(&root, row);
        let id = row["id"].as_str().unwrap().replace(".disabled", ".source");
        let target = import(&root, obs["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap());
        let fid = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("xcf.filter.enable", json!({"filterId":fid,"enabled":true})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        assert_eq!(
            photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
        );
        assert_eq!(current.layers, source.layers);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        enabled += 1;
    }
    assert_eq!(enabled, 36);
}

#[test]
#[ignore = "local Channel public Double parameter authority, all36 current contexts"]
fn exact_double_parameters_survive_current_half_and_native_without_normalization() {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let (root, obs) = packet();
    let rows = obs["records"].as_array().unwrap();
    let mut count = 0;
    let mut different = 0;
    for row in rows.iter().filter(|r| r["action"] == "source") {
        let source = import(&root, row);
        let targetid = row["id"].as_str().unwrap().replace(".source", ".signed");
        let target = import(&root, rows.iter().find(|r| r["id"] == targetid).unwrap());
        let mut arguments = target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.clone();
        let value = (-0.50000000001_f64).to_bits();
        arguments.iter_mut().find(|a| a.name == "rr-gain").unwrap().value = FilterValue::Number(ExactNumber::Float64Bits(value));
        let fid = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let mut wire = serde_json::to_value(&arguments).unwrap();
        wire.as_array_mut().unwrap().iter_mut().find(|a| a["name"] == "rr-gain").unwrap()["value"]["Number"]["Float64Bits"] = json!(format!("0x{value:016x}"));
        session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":wire})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        assert_eq!(current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, arguments);
        let current_render = photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap();
        let original_f32_render = photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap();
        if current_render.px != original_f32_render.px {
            different += 1;
        }
        session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":wire})).unwrap();
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        assert_eq!(*session.active().unwrap().doc, current);
        let bytes = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened, current);
        assert_eq!(photocraft_compose::gimp_render::try_render(&reopened, reopened.bounds(), &mut |_| Ok(())).unwrap().px, current_render.px);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        count += 1;
    }
    assert_eq!(count, 36);
    assert!(different > 0, "Double gain authority must not flatten to original Float32");
}

#[test]
#[ignore = "local Channel current profile/native/mask refusal and Half removal lifecycle"]
fn channel_current_native_profile_and_mask_guards_are_atomic_and_removal_resets_half() {
    let (root, obs) = packet();
    let mut rejected = 0;
    let mut removed = 0;
    for row in obs["records"].as_array().unwrap().iter().filter(|r| r["action"] == "source") {
        let source = import(&root, row);
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let fid = node.id.0.to_string();
        if row["precision"].as_str().unwrap().starts_with("FLOAT_") || row["precision"].as_str().unwrap().starts_with("DOUBLE_") {
            for mutation in 0..9 {
                let mut bad = source.clone();
                match mutation {
                    0 => bad.size = photocraft_geom::Size::new(9, 6),
                    1 => bad.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                    2 => bad.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.5]).unwrap(),
                    3 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                    4 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_clipping = Some(1),
                    5 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
                    6 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_blend_space = 0,
                    _ => {
                        let channels = bad.pixel_format().mode.color_channels();
                        let mut values = vec![0.5; channels + 1];
                        values[if mutation == 7 { 0 } else { channels }] = 1.125;
                        bad.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &values).unwrap();
                    }
                }
                let mut session = Session::new();
                session.add_document(bad.clone(), None).unwrap();
                assert!(session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":node.arguments})).is_err(), "{}/mutation{mutation}", row["id"]);
                assert_eq!(*session.active().unwrap().doc, bad);
                assert_eq!(session.active().unwrap().history.past_len(), 0);
                rejected += 1;
            }
        }
        if row["precision"].as_str().unwrap().starts_with("HALF_") {
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            session.execute("xcf.filter.remove", json!({"filterId":fid})).unwrap();
            let current = (*session.active().unwrap().doc).clone();
            let state = current.xcf_state.as_ref().unwrap();
            assert_eq!(state.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
            assert!(state.filter_masks.is_empty());
            assert!(state.filter_stacks.iter().all(|s| s.nodes.is_empty()));
            assert_eq!(current.layers, source.layers);
            assert_eq!(current.source_color_context, source.source_color_context);
            let bytes = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), current);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, source);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, current);
            removed += 1;
        }
    }
    assert_eq!(rejected, 108);
    assert_eq!(removed, 6);
}
