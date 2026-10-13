#![cfg(feature = "corpus")]
//! Genuine signed/over-one float coverage must remain authoritative, distinct from unit masks.
use photocraft_doc::Rect;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
fn read(root: &Path, entry: &Value) -> Vec<u8> {
    let bytes = std::fs::read(root.join(entry["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), entry["sha256"].as_str().unwrap());
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished local wide-transform originals"]
fn genuine_float_resample_coverage_import_native_and_projection() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS");
    let root = Path::new(&base).join("float-alpha-composition-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "adecf73e948e7c62db8ecee561cd440d1eefcbd0f76ef57e5d4c5614619bac37");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in packet["records"].as_array().unwrap().iter().filter(|r| r["backdrop"] == "transparent" && r.get("literal_alphas").is_none()) {
        let id = row["id"].as_str().unwrap();
        let original = &row["original"];
        let source_root = Path::new(&base).join(original["folder"].as_str().unwrap());
        let raw = read(&source_root, original);
        let doc = photocraft_io::import(&format!("{id}.xcf"), &raw).unwrap_or_else(|e| panic!("legitimate float coverage {id}: {e}")).document;
        let state = &row["source"];
        let x = state["offset"][0].as_i64().unwrap() as i32;
        let y = state["offset"][1].as_i64().unwrap() as i32;
        let bounds = Rect::new(x, y, x + state["size"][0].as_i64().unwrap() as i32, y + state["size"][1].as_i64().unwrap() as i32);
        let expected_native = read(&root, &state["native"]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), expected_native, "retained native {id}");
        let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened, doc);
        assert_eq!(reopened.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), expected_native);
        let output = photocraft_compose::gimp_render::try_render(&reopened, reopened.bounds(), &mut |_| Ok(())).unwrap();
        let expected = read(&root, &row["projection"]["encoded_double"]);
        let channels = reopened.mode.color_channels() + 1;
        assert_eq!(expected.len(), output.px.len() * channels * 8);
        let mut maximum = 0_f64;
        for (actual, bytes) in output.px.iter().zip(expected.chunks_exact(channels * 8)) {
            let reference = bytes.as_chunks::<8>().0.iter().map(|b| f64::from_le_bytes(*b)).collect::<Vec<_>>();
            for c in 0..4 {
                let e = reference[if channels == 2 { usize::from(c == 3) } else { c }];
                if c < 3 && actual[3] == 0. && reference[channels - 1] == 0. {
                    continue;
                }
                maximum = maximum.max((actual[c] - e).abs() * 255.);
            }
        }
        assert!(maximum <= 2., "{id} transparent projection {maximum}/255");
        assert_eq!(raw, read(&source_root, original), "immutable source {id}");
        count += 1;
    }
    assert_eq!(count, 6);
}

#[test]
#[ignore = "requires reviewed unpublished local wide-transform originals"]
fn changed_finite_source_context_rejects_without_native_history_changes() {
    use photocraft_doc::{SampleEncoding, WorkingProfileInterpretation, import_compositing::GimpOperator};
    use photocraft_engine::Session;
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let raw = std::fs::read(Path::new(&base).join("wide-transform-evidence/double_non_linear-rgb-impulse.edit.xcf")).unwrap();
    let imported = photocraft_io::import("own-float-context.xcf", &raw).unwrap().document;
    // The valid container/header and unchanged drawable decode, but this canvas
    // no longer has the independently observed source context. Public IO must
    // reject it after the explicit decoder opt-in, without an admission fallback.
    let mut changed_header = raw.clone();
    changed_header[14..18].copy_from_slice(&17_u32.to_be_bytes());
    assert!(photocraft_io::import("own-unobserved-float-canvas.xcf", &changed_header).is_err());
    assert_eq!(raw, std::fs::read(Path::new(&base).join("wide-transform-evidence/double_non_linear-rgb-impulse.edit.xcf")).unwrap());
    for scenario in 0..5 {
        let mut doc = imported.clone();
        match scenario {
            0 => doc.layers[0].opacity = 0.5,
            1 => doc.working_profile_interpretation = WorkingProfileInterpretation::Icc,
            2 => doc.sample_encoding = Some(SampleEncoding::Linear),
            3 => doc.size.width = 17,
            _ => {
                doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|item| item.composition.as_mut()).unwrap().resolved.as_mut().unwrap().operator =
                    GimpOperator::ClipToLayer
            }
        }
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        assert!(
            photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&doc, &photocraft_raster::Interrupt::NONE).is_err(),
            "context {scenario}"
        );
        let mut session = Session::new();
        session.add_document(doc.clone(), None).unwrap();
        assert!(session.execute("edit.transform", serde_json::json!({"matrix":[1,0,0,1,0.5,0.375],"interpolation":"gimpCubic"})).is_err());
        let active = session.active().unwrap();
        assert_eq!(*active.doc, doc);
        assert_eq!(active.history.past_len(), 0);
        assert!(!active.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), before);
    }
}

#[test]
#[ignore = "requires reviewed unpublished local wide-transform originals"]
fn generated_nonunit_epoch_late_cancellation_preserves_native_and_history() {
    use photocraft_engine::Session;
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let raw = std::fs::read(Path::new(&base).join("wide-transform-evidence/double_non_linear-rgb-impulse.xcf")).unwrap();
    let source = photocraft_io::import("own-generated-alpha.xcf", &raw).unwrap().document;
    let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let params = serde_json::json!({"matrix":[1,0,0,1,0.5,0.375],"interpolation":"gimpCubic"});
    let polls = AtomicUsize::new(0);
    let observe = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let mut probe = Session::new();
    probe.add_document(source.clone(), None).unwrap();
    photocraft_engine::transform_cmds::transform_with_interrupt(&mut probe, &params, &Interrupt::cancel_only(&observe)).unwrap();
    let total = polls.load(Ordering::Relaxed);
    assert!(total > 10);
    let edited = &probe.active().unwrap().doc;
    assert_eq!(
        edited.xcf_state.as_ref().unwrap().items.iter().find_map(|item| item.composition.as_ref()).unwrap().compatibility_epoch,
        photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH
    );
    for cutoff in [total / 2, total.saturating_sub(5), total - 1] {
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let calls = AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= cutoff;
        let result = photocraft_engine::transform_cmds::transform_with_interrupt(&mut session, &params, &Interrupt::cancel_only(&cancel));
        assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)), "cancellation result {result:?}, cutoff {cutoff} of {total} polls");
        let active = session.active().unwrap();
        assert_eq!(*active.doc, source);
        assert_eq!(active.history.past_len(), 0);
        assert!(!active.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), before);
    }
}
