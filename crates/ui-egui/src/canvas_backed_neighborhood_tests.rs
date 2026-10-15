//! UNRUN genuine neighborhood source -> typed UI/current canvas -> native
//! continuation. Resident rendering is an existing-adapter reference, not a new
//! GIMP oracle. Source/reference/archive Vecs are external harness captures.
use super::*;
use photocraft_doc::LayerContent;
use photocraft_raster::{
    backing::quota::{BackingQuota, QuotaLimits},
    checked_view::NativeTileBytes,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, &str); 8] = [
    ("operations/gegl-box-blur/operation-gegl-box-blur.xcf", "0dddbdcee491ce0e24aea521ce7c1de4fa0e5304a2caa9021933917a1de63b15"),
    ("box-raster-color-evidence/implicit-hdr-f64.source.xcf", "11c8158edbab33226b51c201ff167a34eeef52c53efeff601cab242ced6abdc6"),
    ("operations/gegl-median-blur/operation-gegl-median-blur.xcf", "31fcb8d83bb6e574a9dddb8b1e4222de1fa2642a34202696c29e3effa6cbe746"),
    ("median-raster-color-evidence/implicit-hdr-f64.source.xcf", "8f8001f3f31d6b9936f659ab18965a8df57f9e403297497f1d339e3539fe4442"),
    ("median-gray-color-evidence/implicit-gray-hdr-f64.source.xcf", "1149bcbe3cadc2075f4c7546a7eeaa0197f07677ce882a6dc75835483cd57cc7"),
    ("operations/gegl-motion-blur-linear/operation-gegl-motion-blur-linear.xcf", "7fd61309e9ad01f397b5127f0b51f2e2fae27f9d81c53c56305f02b6d24413e8"),
    ("operations/gegl-pixelize/operation-gegl-pixelize.xcf", "a4b746de40767cbd07467cad702cda4611c95e614c718164696ef475b2fe1f34"),
    ("gaussian-raster-color-evidence/implicit-hdr-f64.source.xcf", "e9a17b3deb40ecd066f09a0888fe5332cb54c542afae8ee0be8700ac6bc80d46"),
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn native(doc: &Document) -> (photocraft_raster::backing::NativePlaneDescriptor, String) {
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("genuine native owner") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(bytes) | NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    (layer.descriptor(), digest.finalize().iter().map(|v| format!("{v:02x}")).collect())
}

#[test]
#[ignore = "UNRUN: eight real neighborhood domains share UI paid canvas/commands and pathless native continuation"]
fn neighborhood_ui_canvas_matches_existing_resident_adapter_and_preserves_native_source_on_continuation() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    for (relative, pin) in CASES {
        let bytes = std::fs::read(root.join(relative)).unwrap();
        assert_eq!(hash(&bytes), pin);
        let name = relative.rsplit('/').next().unwrap();
        let resident = photocraft_io::import(name, &bytes).unwrap().document;
        let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(&resident).unwrap();
        let reference = renderer.render(resident.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let expected = display_image(None, &reference);
        let mut disabled_reference = resident.clone();
        disabled_reference.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(&disabled_reference).unwrap();
        let reference = renderer.render(disabled_reference.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let expected_disabled = display_image(None, &reference);
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
        let before = app.session.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_point_storage());
        assert!(app.session.active().unwrap().source_read_only);
        assert!(app.session.active().unwrap().path.is_none());
        let raw = native(&before);
        let width = before.size.width;
        let height = before.size.height;
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        assert!(node.enabled);
        let (display, fee) = backed_snapshot_display_image(&before, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels, "source {relative}");
        drop((display, fee));
        // A bare Document cannot bypass the actual typed source owner.
        assert!(backed_display_image(&before, width, height, None, &Interrupt::NONE).is_err());
        let query = photocraft_engine::accounted_xcf_inspect::prepare(&app.session, &Value::Null, &ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 1);
        drop(query);
        let inspection = photocraft_engine::accounted_filter_inspect::prepare(&app.session, &json!({"filterId":id}), &ctl()).unwrap();
        assert_eq!(inspection.value()["operation"]["name"], node.operation.name);
        drop(inspection);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        app.run("layer.setProps", json!({"name":"Neighborhood UI current"})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        assert_eq!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.run("edit.redo", json!({})).unwrap();
        let edited = app.session.active().unwrap().doc.clone();
        assert_eq!(native(&edited), raw);
        let (display, fee) = backed_snapshot_display_image(&edited, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels);
        drop((display, fee));
        assert_eq!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
        let disabled = app.session.active().unwrap().doc.clone();
        assert_eq!(native(&disabled), raw);
        let (display, fee) = backed_snapshot_display_image(&disabled, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected_disabled.pixels, "disabled {relative}");
        drop((display, fee));
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&disabled));
        // Warm retained source/profile caches before checking cancellation;
        // real output fees must return without changing model/history.
        let baseline = quota.usage().unwrap();
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert!(backed_snapshot_display_image(&disabled, width, height, None, &interrupt).is_err());
        assert_eq!(quota.usage().unwrap(), baseline);
        // Cursor output remains an external harness capture; the reader's
        // immutable native archive and adopted owner have actual same-q fees.
        let saved = photocraft_format::owned_document::save_current_document_archive(&disabled, Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "neighborhood.pcraft", archive, &ctl()).unwrap();
        let current = reopened.session.active().unwrap();
        assert!(current.path.is_none());
        assert!(!current.source_read_only);
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        assert_eq!(current.doc.xcf_state, disabled.xcf_state);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(native(&current.doc), raw);
        let (display, fee) = backed_snapshot_display_image(&current.doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected_disabled.pixels, "disabled native {relative}");
        drop((display, fee));
        assert_eq!(reopened.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        let current = &reopened.session.active().unwrap().doc;
        let (display, fee) = backed_snapshot_display_image(current, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels, "reenabled native {relative}");
        drop((display, fee));
        assert_eq!(native(current), raw);
        assert_eq!(hash(&bytes), pin);
        assert_eq!(std::fs::read(root.join(relative)).unwrap(), bytes);
    }
}
