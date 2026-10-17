//! UNRUN assigned Box byte/native/read-only UI consumers. Resident output is an
//! existing adapter reference, not independent GIMP numerical acceptance.
use super::*;
#[path = "canvas_assigned_box_fixture.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::ctl;
use serde_json::{Value, json};
use std::io::Cursor;
#[test]
#[ignore = "UNRUN: two pinned assigned F32 Box originals, paid typed byte/native adoption, current canvas/reference, retained project/filter replies, closed mutations and exact profile/native/source/title continuation; absent producer ICC captures remain HOLD"]
fn assigned_box_ui_uses_paid_profile_owner_and_read_only_transport() {
    for (path, bytes) in fixture::cases() {
        let name = path.file_name().unwrap().to_str().unwrap();
        let resident = photocraft_io::import(name, &bytes).unwrap().document;
        let old = photocraft_compose::gimp_render::PreparedRenderer::new(&resident).unwrap();
        let expected = display_image(None, &old.render(resident.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap());
        let quota = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
            resident: 256 << 20,
            index: 32 << 20,
            scratch: 0,
        });
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
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let owner = before.accounted().unwrap();
        assert!(owner.is_readonly_assigned_point_storage());
        assert!(!owner.is_readonly_point_storage());
        assert!(quota.shares_ledger(owner.quota()));
        assert!(app.session.active().unwrap().path.is_none());
        assert!(app.session.active().unwrap().source_read_only);
        fixture::observed(&before);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        for (command, params) in [("xcf.inspect", Value::Null), ("xcf.filter.inspect", json!({"filterId":id}))] {
            let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
                "engine.execute",
                json!({"command":command,"params":params,"wait":false}),
                json!(1),
                1 << 20,
            );
            assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
            assert!(wake.try_recv().unwrap().is_null());
            let wire = carrier.try_recv().unwrap();
            let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
            assert_eq!(value["ok"], true);
            if command == "xcf.inspect" {
                assert_eq!(value["result"]["filters"]["total"], 1);
            } else {
                assert_eq!(value["result"]["operation"]["name"], "gegl:box-blur");
            }
            drop(wire);
        }
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("assigned query denied".into())));
        let (request, _wake, _carrier) =
            crate::control::ControlRequest::with_retained_query("engine.execute", json!({"command":"xcf.inspect","params":null}), json!(2), 1 << 20);
        let crate::control::Outcome::Done(denied) = crate::control::handle(&mut app, &egui::Context::default(), &request) else {
            panic!("authorization refuses synchronously")
        };
        assert!(denied.to_string().contains("assigned query denied"));
        app.session.authorize = None;
        let (image, fee) = backed_snapshot_display_image(&before, 16, 12, None, &Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels);
        drop((image, fee));
        let stop = || true;
        assert!(backed_snapshot_display_image(&before, 16, 12, None, &Interrupt::cancel_only(&stop)).is_err());
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        let journal = app.session.journal.len();
        app.background_jobs = false;
        for (command, params) in [
            ("layer.setProps", json!({"name":"closed"})),
            ("layer.translate", json!({"dx":1})),
            ("xcf.filter.set", json!({"filterId":id,"enabled":false,"arguments":[]})),
            ("select.all", Value::Null),
            ("edit.clear", Value::Null),
        ] {
            assert!(app.run(command, params).is_err(), "{command}");
        }
        assert!(!app.session.has_jobs());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.journal.len(), journal);
        let titled = owner.try_with_document_name("Assigned Box saved title", &ctl()).unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "label.pcraft", fixture::archive(&titled), &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.name, "Assigned Box saved title");
        assert!(current.accounted().unwrap().is_readonly_assigned_point_storage());
        assert!(quota.shares_ledger(current.accounted().unwrap().quota()));
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(reopened.session.active().unwrap().path.is_none());
        fixture::exact(&before, current);
        let (image, fee) = backed_snapshot_display_image(current, 16, 12, None, &Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels);
        drop((image, fee));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
