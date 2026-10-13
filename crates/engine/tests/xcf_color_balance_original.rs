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
#[ignore = "requires local unpublished independent Color Balance edit references"]
fn real_config_luminosity_enable_edits_native_undo_and_atomic_failures() {
    verify_packet("color-balance-evidence", "2f2ec198f19fb27e34f5d5e6f6b37b270f3249db06f768247481abf3dcd93833", 18);
}
#[test]
#[ignore = "requires local unpublished integer half perceptual Color Balance edit references"]
fn integer_half_perceptual_config_edits_native_undo_and_atomic_failures() {
    verify_packet("color-balance-depth-evidence", "247c44692802e299f20b144497b1fa9596f2e6c28b8883c50002bd4d0e372923", 36);
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
    for row in proof["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let family = row["id"].as_str().unwrap().strip_suffix(".source").unwrap();
        let bytes = read(row["file"].as_str().unwrap());
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        for stage in ["coupled", "preserve", "disabled"] {
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
                    match variant {
                        0 => bad["arguments"][1]["value"]["Config"][0]["value"]["Config"][0]["value"] = json!({"Number":{"Float64Bits":"0x7ff8000000000000"}}),
                        1 => bad["arguments"][1]["value"]["Config"][0]["value"]["Config"][0]["value"] = json!({"Number":{"Float32Bits":0.5_f32.to_bits()}}),
                        2 => bad["arguments"][1]["value"]["Config"][0]["value"]["Config"][0]["value"] = json!({"Number":{"Float64Bits":"0x4000000000000000"}}),
                        3 => bad["arguments"][1]["value"]["Config"][1]["name"] = json!("shadows"),
                        4 => bad["arguments"][0]["value"] = json!({"Bool":1}),
                        5 => bad["arguments"][1]["value"]["Config"][0]["value"]["Config"][1]["name"] = json!("cyan-red"),
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
#[ignore = "requires local unpublished integer half perceptual Color Balance references"]
fn depth_epoch_rejects_unobserved_current_storage_and_mask_atomically() {
    use photocraft_doc::SampleEncoding;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-balance-depth-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "247c44692802e299f20b144497b1fa9596f2e6c28b8883c50002bd4d0e372923");
    let hashes: Value = serde_json::from_slice(&manifest).unwrap();
    let proof: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut rejected_count = 0;
    for row in proof["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let file = row["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(file)).unwrap();
        assert_eq!(hash(&bytes), hashes[file]);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        for variant in 0..11 {
            let mut changed = source.clone();
            match variant {
                0 => changed.source_color_context.as_mut().unwrap().source_precision = 650,
                1 => {
                    changed.sample_encoding =
                        Some(if changed.sample_encoding == Some(SampleEncoding::Linear) { SampleEncoding::Perceptual } else { SampleEncoding::Linear })
                }
                2 => {
                    changed.depth =
                        if changed.depth == photocraft_color::SampleType::U8 { photocraft_color::SampleType::U16 } else { photocraft_color::SampleType::U8 }
                }
                3 => changed.icc_profile = None,
                4 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "xcf-22-stock-color-balance-precision-v1".into(),
                5 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = -113999279,
                6 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                7 => {
                    let bounds = changed.bounds();
                    let mask = &mut changed.xcf_state.as_mut().unwrap().filter_masks[0].surface;
                    let mut raw = mask.try_read_interleaved(bounds).unwrap();
                    raw[0] = 1;
                    mask.try_write_interleaved(bounds, &raw).unwrap();
                }
                8 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits = 0.5_f64.to_bits(),
                9 => changed.layers[0].opacity = 0.5,
                _ => {
                    // This is a legal current F32 value, but is not an exact Half
                    // sample. Original precision does not authorize narrowing it.
                    if !matches!(changed.source_color_context.as_ref().unwrap().source_precision, 500 | 550 | 575) {
                        continue;
                    }
                    let bounds = changed.bounds();
                    let surface = changed.layers[0].surface_mut().unwrap();
                    let mut raw = surface.try_read_interleaved(bounds).unwrap();
                    raw[..4].copy_from_slice(&0.1_f32.to_ne_bytes());
                    surface.try_write_interleaved(bounds, &raw).unwrap();
                }
            }
            let id = changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
            let mut current = session(changed.clone());
            assert!(current.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{file} variant{variant}");
            assert_eq!(*current.active().unwrap().doc, changed);
            assert_eq!(current.active().unwrap().history.past_len(), 0);
            rejected_count += 1;
        }
        assert_eq!(std::fs::read(root.join(file)).unwrap(), bytes);
    }
    assert_eq!(rejected_count, 123);
}
