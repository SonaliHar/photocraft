//! UNRUN explicit owned Gaussian isolated group source/native UI continuation.
//! Independent PNGs test derived output; native captures remain exact.
use super::*;
#[path = "canvas_gaussian_group_fixture.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, clippy::panic, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::{assets, ctl, observed, retained, state};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::{Value, json};
use std::io::Cursor;
fn import(name: &str, bytes: &[u8], q: BackingQuota) -> photocraft_io::OwnedXcfImport {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    photocraft_io::import_owned_xcf_from_reader_in_quota(name, &mut Cursor::new(bytes), bytes.len() as u64, &Default::default(), &limits, q, &ctl()).unwrap()
}
fn preview(doc: &photocraft_engine::DocumentSnapshot, root: &std::path::Path, phase: &str) {
    let png = photocraft_codecs::decode(&std::fs::read(root.join(format!("group-28-group-mask.fresh-{phase}.ref.png"))).unwrap()).unwrap();
    let (actual, fee) = backed_snapshot_display_image(doc, 16, 12, None, &Interrupt::NONE).unwrap();
    assert_eq!(actual.size, [16, 12]);
    // The established 16-bit independent oracle has <=1/65535 wide error.
    // Match the UI's unmultiplied-to-premultiplied display boundary before
    // applying the separate <=2/255 display bound.
    for y in 0..12 {
        for x in 0..16 {
            let p: [u8; 4] = std::array::from_fn(|c| (png.get(x, y, c).clamp(0., 1.) * 255.).round() as u8);
            let expected = egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]).to_array();
            for (c, expected_component) in expected.iter().enumerate() {
                assert!((i16::from(actual.pixels[(y * 16 + x) as usize].to_array()[c]) - i16::from(*expected_component)).abs() <= 2);
            }
        }
    }
    drop((actual, fee));
}
#[test]
#[ignore = "UNRUN: exact isolated Gaussian mode28 source/edit hierarchy/native/masks/profile/paid inspect/UI preview/native title continuation; mode61 refuses"]
fn gaussian_group_ui_preserves_live_tree_and_masks_through_pathless_native() {
    let root = assets();
    for phase in ["source", "edit"] {
        let file = if phase == "source" { "group-28-group-mask.xcf" } else { "group-28-group-mask.edit.xcf" };
        let bytes = std::fs::read(root.join(file)).unwrap();
        let captured = state(&root, phase);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let carrier = import(file, &bytes, q.clone());
        assert!(matches!(&carrier, photocraft_io::OwnedXcfImport::IsolatedGaussianGroup(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, file, &carrier, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let owner = before.accounted().unwrap();
        assert!(owner.is_readonly_gaussian_group_storage());
        owner.validate_gaussian_group_storage(&ctl()).unwrap();
        assert!(app.session.active().unwrap().source_read_only);
        assert!(app.session.active().unwrap().path.is_none());
        observed(&root, &before, &captured);
        preview(&before, &root, phase);
        assert!(backed_display_image(&before, 16, 12, None, &Interrupt::NONE).is_err());
        let query = photocraft_engine::accounted_xcf_inspect::prepare(&app.session, &Value::Null, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 6);
        assert_eq!(query.value()["filters"]["total"], 1);
        drop(query);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let query = photocraft_engine::accounted_filter_inspect::prepare(&app.session, &json!({"filterId":id}), &ctl()).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gegl:gaussian-blur");
        drop(query);
        // Exercise the actual retained desktop/control selector, rather than
        // only the engine factory; a missing runtime-class hint picks plain.
        for (command, params) in [("xcf.inspect", Value::Null), ("xcf.filter.inspect", json!({"filterId":id}))] {
            let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
                "engine.execute",
                json!({"command":command,"params":params,"wait":false}),
                json!(17),
                1 << 20,
            );
            assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
            assert!(wake.try_recv().unwrap().is_null());
            let wire = carrier.try_recv().unwrap();
            let decoded: Value = serde_json::from_slice(wire.bytes()).unwrap();
            assert_eq!(decoded["id"], 17);
            assert_eq!(decoded["ok"], true);
            if command == "xcf.inspect" {
                assert_eq!(decoded["result"]["items"]["total"], 6);
            } else {
                assert_eq!(decoded["result"]["operation"]["name"], "gegl:gaussian-blur");
            }
            drop(wire);
        }
        // Parameter and layer edits remain outside this exact activation lane.
        assert!(app.run("xcf.filter.set", json!({"filterId":id,"enabled":false,"arguments":[]})).is_err());
        assert!(app.run("layer.setProps", json!({"blend":"multiply"})).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let cancelled = || true;
        let interrupt = Interrupt::cancel_only(&cancelled);
        assert!(backed_snapshot_display_image(&before, 16, 12, None, &interrupt).is_err());
        let named = owner.try_with_document_name("Gaussian group saved", &ctl()).unwrap();
        retained(&before, named.document());
        // External Cursor capture is not an after-the-fact allocation proof.
        let bytes_out =
            photocraft_format::owned_document::save_current_document_archive(named.document(), Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                .unwrap()
                .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(bytes_out.as_slice()),
            bytes_out.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Different picker filename.pcraft", archive, &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.name, "Gaussian group saved");
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(reopened.session.active().unwrap().path.is_none());
        retained(&before, current);
        observed(&root, current, &captured);
        preview(current, &root, phase);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), bytes);
    }
    for file in ["group-61-group-mask.xcf", "group-61-group-mask.edit.xcf"] {
        let bytes = std::fs::read(root.join(file)).unwrap();
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        assert!(
            photocraft_io::import_owned_xcf_from_reader_in_quota(
                file,
                &mut Cursor::new(bytes.as_slice()),
                bytes.len() as u64,
                &Default::default(),
                &limits,
                BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 }),
                &ctl()
            )
            .is_err()
        );
        assert_eq!(std::fs::read(root.join(file)).unwrap(), bytes);
    }
}
