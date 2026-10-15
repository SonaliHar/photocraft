//! UNRUN residual source/current canvas and native continuation. Resident
//! current rendering is an adapter reference here, not new GIMP math evidence.
//! Source/archive Vecs below are external harness buffers, not paid UI caches.
use super::*;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn cases() -> [(&'static str, &'static str, &'static str); 6] {
    [
        ("desaturate-precision-evidence", "ff2d1ce5981edb0b3d4b5c22e01e8af015945160062d9b057d997e720f231603", "rgb-u16_linear.average.xcf"),
        ("saturation-precision-evidence", "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c", "rgb-float_non_linear.native-half.xcf"),
        ("saturation-precision-evidence", "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c", "gray-double_linear.cieyuv-half.xcf"),
        ("saturation-precision-evidence", "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c", "rgb-float_linear.cieyuv-half.xcf"),
        ("value-invert-precision-evidence", "8a75ccbb88e61611d3c3523db8145f922b63676601986c124d2d8493a27e73a1", "double_non_linear.source.xcf"),
        ("operations/gimp-posterize", "4eac864e360186bb24174f091b7b84d424e804177fe75c0330d6e888b801968a", "operation-gimp-posterize.xcf"),
    ]
}
fn source(folder: &str, pin: &str, name: &str) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(folder);
    let proof_name = if folder.starts_with("operations/") { "probe.json" } else { "batch-manifest.json" };
    let proof = std::fs::read(root.join(proof_name)).unwrap();
    assert_eq!(hash(&proof), pin);
    let proof: Value = serde_json::from_slice(&proof).unwrap();
    let expected = if proof_name == "probe.json" {
        assert_eq!(proof["cases"][0]["input"], name);
        proof["cases"][0]["input_sha256"].as_str().unwrap()
    } else {
        proof["artifacts"].as_array().unwrap().iter().find(|v| v["path"] == name).unwrap()["sha256"].as_str().unwrap()
    };
    let bytes = std::fs::read(root.join(name)).unwrap();
    assert_eq!(hash(&bytes), expected);
    bytes
}
fn expected(doc: &photocraft_doc::Document) -> egui::ColorImage {
    let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(doc).unwrap();
    let reference = renderer.render(doc.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
    display_image(None, &reference)
}

#[test]
#[ignore = "UNRUN: genuine residual originals -> canvas reference -> enable edit -> native reopen canvas"]
fn residual_source_and_current_native_canvas_match_resident_reference() {
    for (folder, pin, name) in cases() {
        let bytes = source(folder, pin, name);
        let mut resident = photocraft_io::import(name, &bytes).unwrap().document;
        let source_reference = expected(&resident);
        let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
            name,
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(matches!(&imported, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        let state = app.session.active().unwrap();
        assert!(state.source_read_only);
        assert!(state.path.is_none());
        let width = state.doc.size.width;
        let height = state.doc.size.height;
        let (display, fee) = backed_snapshot_display_image(&state.doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, source_reference.pixels, "source {name}");
        drop((display, fee));
        let node = &resident.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let enabled = !node.enabled;
        let id = app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        app.session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":enabled})).unwrap();
        resident.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = enabled;
        let current_reference = expected(&resident);
        let state = app.session.active().unwrap();
        let (display, fee) = backed_snapshot_display_image(&state.doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, current_reference.pixels, "disabled current {name}");
        drop((display, fee));
        assert_eq!(state.doc.depth, resident.depth);
        assert_eq!(state.doc.icc_profile, resident.icc_profile);
        assert_eq!(state.doc.working_profile_interpretation, resident.working_profile_interpretation);
        // External harness save buffer; the genuine archive/reader and returned
        // UI display owners have independent actual same-quota accounting.
        let saved = photocraft_format::owned_document::save_current_document_archive(&state.doc, Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            quota,
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "residual.pcraft", archive, &ctl()).unwrap();
        let current = reopened.session.active().unwrap();
        assert!(!current.source_read_only);
        assert!(current.path.is_none());
        let (display, fee) = backed_snapshot_display_image(&current.doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, current_reference.pixels, "native current {name}");
        drop((display, fee));
        reopened.session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":!enabled})).unwrap();
        let (display, fee) = backed_snapshot_display_image(&reopened.session.active().unwrap().doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, source_reference.pixels, "continued edit {name}");
        drop((display, fee));
        assert_eq!(bytes, source(folder, pin, name));
    }
}

#[test]
#[ignore = "UNRUN: cancelled residual canvas leaves current snapshot/history and releases temporary output"]
fn cancelled_residual_display_preserves_current_owner() {
    let (folder, pin, name) = cases()[3];
    let bytes = source(folder, pin, name);
    let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
        name,
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        quota.clone(),
        &ctl(),
    )
    .unwrap();
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
    let current = app.session.active().unwrap();
    // Establish the genuine retained profile/native cache before the baseline.
    let (warm, fee) = backed_snapshot_display_image(&current.doc, current.doc.size.width, current.doc.size.height, None, &Interrupt::NONE).unwrap();
    drop((warm, fee));
    let before = current.doc.clone();
    let history = current.history.clone();
    let usage = quota.usage().unwrap();
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(backed_snapshot_display_image(&current.doc, current.doc.size.width, current.doc.size.height, None, &interrupt).is_err());
    assert!(app.session.active().unwrap().doc.shares_state(&before));
    assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(quota.usage().unwrap(), usage);
}
