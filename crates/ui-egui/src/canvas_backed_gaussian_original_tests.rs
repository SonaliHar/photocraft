//! UNRUN original Gaussian source/edit host workflow. Native words are exact;
//! derived display uses the existing independent PNG bound, not byte identity.
use super::*;
use photocraft_doc::{Document, LayerContent};
use photocraft_raster::{
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn assets() -> (PathBuf, Value) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let bytes = include_bytes!("../../compose/tests/gimp_gaussian_hashes.json");
    let hashes: Value = serde_json::from_slice(bytes).unwrap();
    for (path, pin) in hashes.as_object().unwrap() {
        assert_eq!(hash(&std::fs::read(root.join(path)).unwrap()), pin.as_str().unwrap());
    }
    (root, hashes)
}
fn native(doc: &Document) -> (photocraft_raster::backing::NativePlaneDescriptor, Vec<u8>, String) {
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("actual native layer") };
    let r = layer.operative_bounds();
    let mut bytes = vec![0; r.width() as usize * r.height() as usize * layer.format().bytes_per_pixel()];
    let mut physical = Sha256::new();
    physical.update(layer.descriptor().default_bytes());
    for coord in r.tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => physical.update(b),
        }
        let part = coord.rect().intersect(&r);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * 4;
                pin.read_pixel_native_into(x, y, &mut bytes[at..at + 4], &ctl()).unwrap();
            }
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        physical.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            physical.update(tile.bytes());
        }
    }
    (layer.descriptor(), bytes, physical.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn image(snapshot: &photocraft_engine::DocumentSnapshot, root: &std::path::Path, reference: &str) {
    let expected = photocraft_codecs::decode(&std::fs::read(root.join(reference)).unwrap()).unwrap();
    let (actual, lease) = backed_snapshot_display_image(snapshot, 96, 80, None, &Interrupt::NONE).unwrap();
    assert_eq!(actual.size, [96, 80]);
    for y in 0..80 {
        for x in 0..96 {
            for c in 0..4 {
                let target = (expected.get(x, y, c) * 255.0).round() as i32;
                assert!((i32::from(actual.pixels[(y * 96 + x) as usize].to_array()[c]) - target).abs() <= 2, "{reference} ({x},{y})/{c}");
            }
        }
    }
    drop((actual, lease));
}
#[test]
#[ignore = "UNRUN: pinned original Gaussian source/edit, real UI registered edit/undo/derived PNG reference and native continuation"]
fn original_gaussian_ui_source_edit_roundtrip_keeps_native_and_profile_roles() {
    let (root, _) = assets();
    for (file, initial, other, first_ref, other_ref) in [
        ("filter-owner-layer.xcf", 2.25f32, 3.75f32, "filter-owner-layer.ref.png", "filter-owner-layer.edit.ref.png"),
        ("filter-owner-layer.edit.xcf", 3.75f32, 2.25f32, "filter-owner-layer.edit.ref.png", "filter-owner-layer.ref.png"),
    ] {
        let bytes = std::fs::read(root.join(file)).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let carrier = photocraft_io::import_owned_xcf_from_reader_in_quota(
            file,
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(matches!(&carrier, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, file, &carrier, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let words = native(&before);
        assert_eq!(words.1, std::fs::read(root.join("filter-owner-layer.source.0.native.bin")).unwrap());
        assert!(app.session.active().unwrap().source_read_only);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        image(&before, &root, first_ref);
        let query = photocraft_engine::accounted_filter_inspect::prepare(&app.session, &json!({"filterId":id}), &ctl()).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gegl:gaussian-blur");
        drop(query);
        let patch = |value: f32| json!({"filterId":id,"arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":value.to_bits()}}}]});
        assert_eq!(app.run("xcf.filter.set", patch(other)).unwrap()["updated"], true);
        let edited = app.session.active().unwrap().doc.clone();
        assert_eq!(native(&edited), words);
        image(&edited, &root, other_ref);
        assert_eq!(edited.icc_profile, before.icc_profile);
        assert_eq!(edited.source_color_context, before.source_color_context);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let history = app.session.active().unwrap().history.clone();
        assert_eq!(app.run("xcf.filter.set", patch(initial)).unwrap()["updated"], false);
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        // CURRENT disabled identity uses exact original native bytes as its
        // reference; this is not independently saved disabled-source evidence.
        app.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap();
        let disabled = app.session.active().unwrap().doc.clone();
        assert_eq!(native(&disabled), words);
        let (display, fee) = backed_snapshot_display_image(&disabled, 96, 80, None, &Interrupt::NONE).unwrap();
        for (pixel, raw) in display.pixels.iter().zip(words.1.chunks_exact(4)) {
            for (actual, expected) in pixel.to_array().iter().zip(raw) {
                assert!((i16::from(*actual) - i16::from(*expected)).abs() <= 1);
            }
        }
        drop((display, fee));
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        // Cursor output is external harness storage, not a production fee proof.
        let saved = photocraft_format::owned_document::save_current_document_archive(&edited, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Gaussian current.pcraft", archive, &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.xcf_state, edited.xcf_state);
        assert_eq!(native(current), words);
        assert_eq!(current.icc_profile, before.icc_profile);
        assert_eq!(current.source_color_context, before.source_color_context);
        image(current, &root, other_ref);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), bytes);
    }
}
