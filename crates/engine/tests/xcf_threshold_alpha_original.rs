#![cfg(feature = "corpus")]
//! Current configuration edits checked against separate saved GIMP projects.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn session(doc: photocraft_doc::Document) -> Session {
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
#[test]
#[ignore = "requires local unpublished independent Alpha Threshold edit references"]
fn real_alpha_threshold_enable_edits_native_undo_and_atomic_failures() {
    verify_packet("alpha-threshold-precision-evidence", "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263", 48);
}
fn verify_packet(directory: &str, manifest_hash: &str, expected_count: usize) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(directory);
    let manifest_bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest_bytes), manifest_hash);
    let hashes: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let read = |name: &str| {
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(hash(&bytes), hashes[name].as_str().unwrap());
        bytes
    };
    let proof: Value = serde_json::from_slice(&read("observations.json")).unwrap();
    let mut count = 0;
    for row in proof["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".half")) {
        let family = row["id"].as_str().unwrap().strip_suffix(".half").unwrap();
        let bytes = read(row["file"].as_str().unwrap());
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        for stage in ["zero", "quarter", "one", "disabled"] {
            let reference = photocraft_io::import("reference.xcf", &read(&format!("{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut target_session = session(reference.clone());
            let inspected = target_session.execute("xcf.filter.inspect", json!({"filterId":target.id.0.to_string()})).unwrap();
            assert_eq!(inspected["scalarEditCandidate"], true);
            let arguments: Vec<_> =
                inspected["arguments"].as_array().unwrap().iter().map(|argument| json!({"name":argument["name"],"value":argument["state"]["value"]})).collect();
            let params = if stage == "disabled" {
                json!({"filterId":id.0.to_string(),"enabled":false})
            } else {
                json!({"filterId":id.0.to_string(),"arguments":arguments})
            };
            let command = if stage == "disabled" { "xcf.filter.enable" } else { "xcf.filter.set" };
            let mut current = session(source.clone());
            current.execute(command, params.clone()).unwrap();
            let edited = (*current.active().unwrap().doc).clone();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            let state = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(state.opaque, original.opaque);
            assert_eq!(state.provenance, original.provenance);
            assert_eq!(state.filter_masks, original.filter_masks);
            assert_eq!(state.filter_stacks[0].nodes[0].arguments, target.arguments);
            assert_eq!(state.filter_stacks[0].nodes[0].enabled, target.enabled);
            let actual = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
            let expected = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.px, expected.px);
            let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(reopened, edited);
            assert_eq!(current.active().unwrap().history.past_len(), 1);
            current.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*current.active().unwrap().doc, source);
            current.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*current.active().unwrap().doc, edited);
            if stage != "disabled" {
                for variant in 0..7 {
                    let mut bad = params.clone();
                    let arguments = bad["arguments"].as_array_mut().unwrap();
                    let value = arguments.iter().position(|v| v["name"] == "value").unwrap();
                    match variant {
                        0 => arguments[value]["value"] = json!({"Enum":{"domain":"WrongAlphaType","value":6}}),
                        1 => arguments[value]["name"] = json!("wrong"),
                        2 => arguments[value]["value"] = json!({"Number":{"Signed":{"width":32,"value":0}}}),
                        3 => arguments[value]["value"] = json!({"Number":{"Float64Bits":"0x3fd0000000000000"}}),
                        4 => arguments[value]["value"] = json!({"Number":{"Float32Bits":f32::NAN.to_bits()}}),
                        5 => arguments[value]["value"] = json!({"Number":{"Float32Bits":(-0.125_f32).to_bits()}}),
                        _ => bad["filterId"] = json!("9999999999999"),
                    }
                    let mut rejected = session(source.clone());
                    assert!(rejected.execute(command, bad).is_err(), "{family} variant{variant}");
                    assert_eq!(*rejected.active().unwrap().doc, source);
                    assert_eq!(rejected.active().unwrap().history.past_len(), 0);
                }
                for late in [false, true] {
                    let calls = std::sync::atomic::AtomicUsize::new(0);
                    let cancel = || {
                        let count = calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        !late || count >= 10
                    };
                    let mut rejected = session(source.clone());
                    assert!(
                        photocraft_engine::xcf_filter_cmds::set_with_interrupt(&mut rejected, &params, &photocraft_raster::Interrupt::cancel_only(&cancel))
                            .is_err()
                    );
                    assert_eq!(*rejected.active().unwrap().doc, source);
                    assert_eq!(rejected.active().unwrap().history.past_len(), 0);
                }
            }
            for visibility_lock in [false, true] {
                let mut locked = source.clone();
                let owner = locked.xcf_state.as_ref().unwrap().filter_stacks[0].owner;
                let locks = &mut locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == owner).unwrap().locks;
                if visibility_lock {
                    locks.visibility = true;
                } else {
                    locks.content = true;
                }
                let mut rejected = session(locked.clone());
                let (locked_command, locked_params) =
                    if visibility_lock { ("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})) } else { (command, params.clone()) };
                assert!(rejected.execute(locked_command, locked_params).is_err());
                assert_eq!(*rejected.active().unwrap().doc, locked);
                assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            }
            count += 1;
        }
        assert_eq!(read(row["file"].as_str().unwrap()), bytes);
    }
    assert_eq!(count, expected_count);
}

#[test]
#[ignore = "requires local unpublished independently saved Alpha Threshold contexts"]
fn unqualified_current_contexts_fail_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let hashes: Value = serde_json::from_slice(&manifest).unwrap();
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".half")) {
        let file = row["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(file)).unwrap();
        assert_eq!(hash(&bytes), hashes[file]);
        let original = photocraft_io::import("threshold.xcf", &bytes).unwrap().document;
        for variant in 0..9 {
            let mut changed = original.clone();
            match variant {
                0 => changed.icc_profile = None,
                1 => changed.sample_encoding = Some(photocraft_doc::SampleEncoding::Perceptual),
                2 => {
                    changed.mode =
                        if original.mode == photocraft_doc::ColorMode::Rgb { photocraft_doc::ColorMode::Grayscale } else { photocraft_doc::ColorMode::Rgb }
                }
                3 => changed.layers[0].opacity = 0.5,
                4 => changed.size = photocraft_doc::Size::new(9, 6),
                5 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "unqualified".into(),
                6 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0,
                7 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                _ => {
                    let bounds = changed.bounds();
                    let mask = &mut changed.xcf_state.as_mut().unwrap().filter_masks[0].surface;
                    let mut raw = mask.try_read_interleaved(bounds).unwrap();
                    raw[0] = 1;
                    mask.try_write_interleaved(bounds, &raw).unwrap();
                }
            }
            let id = changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
            let mut current = session(changed.clone());
            assert!(current.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{file} variant{variant}");
            assert_eq!(*current.active().unwrap().doc, changed);
            assert_eq!(current.active().unwrap().history.past_len(), 0);
            count += 1;
        }
        assert_eq!(std::fs::read(root.join(file)).unwrap(), bytes);
    }
    assert_eq!(count, 108);
}
