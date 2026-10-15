//! Deferred host-policy service tests, not engine counter or producer math tests.
use super::*;
#[path = "pure_point_standard_work_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use crate::control::{ControlRequest, Outcome};
use std::{cell::RefCell, rc::Rc};

#[test]
#[ignore = "UNRUN: actual purePoint/derived-empty preview cancellation, UI Save service selected finite work and cumulative writer refusal, retained query RPC/deadline/auth/cap/disconnect custody; synthetic CURRENT fixtures, no measured work/largeimage/numerical/visual acceptance"]
fn pure_point_standard_preview_save_and_retained_query_preserve_host_fences() {
    for index in 0..6 {
        for empty in [false, true] {
            let owner = fixture::owner(index, empty);
            let capture = Rc::new(RefCell::new(None));
            let output = capture.clone();
            let mut app = PhotocraftApp::new(
                photocraft_engine::Session::new(),
                Services {
                    accounted_native_save: Some(Box::new(move |snapshot, _, _, control| {
                        // Injected policy probe, not measured writer work or a fit guarantee.
                        control.check(8_000_000_001).map_err(|e| e.to_string())?;
                        let bytes = photocraft_format::owned_document::save_current_document_archive(
                            snapshot,
                            std::io::Cursor::new(Vec::new()),
                            Default::default(),
                            snapshot.accounted().unwrap().quota(),
                            control,
                        )
                        .map_err(|e| e.to_string())?
                        .into_inner();
                        *output.borrow_mut() = Some(bytes);
                        Ok(())
                    })),
                    ..Default::default()
                },
            );
            crate::owned_native_open::adopt_bytes(&mut app, "CURRENT.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.clone();
            let display = app.session.color.canvas_display_for(&before, None).unwrap();
            let cancelled = || true;
            assert!(
                crate::canvas_backed_preview::backed_snapshot_display_image(
                    &before,
                    1,
                    257,
                    Some(display.as_ref()),
                    &photocraft_raster::Interrupt::cancel_only(&cancelled)
                )
                .is_err()
            );
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            let (image, fee) =
                crate::canvas_backed_preview::backed_snapshot_display_image(&before, 1, 257, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE)
                    .unwrap();
            assert_eq!(image.pixels.len(), 257);
            drop((image, fee));
            app.save_to("policy-saved.pcraft".into(), None).unwrap();
            assert_eq!(app.session.active().unwrap().history.past_len(), history.past_len());
            fixture::same(&before, &app.session.active().unwrap().doc);
            let bytes = capture.borrow_mut().take().unwrap(); // External service capture.
            let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
                &mut std::io::Cursor::new(bytes.as_slice()),
                bytes.len() as u64,
                Default::default(),
                owner.quota().clone(),
                &fixture::ctl(),
            )
            .unwrap();
            let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut reopened, "saved.pcraft", archive, &fixture::ctl()).unwrap();
            fixture::same(&before, &reopened.session.active().unwrap().doc);
            app.services.accounted_native_save = Some(Box::new(|_, _, _, c| c.check(1u64 << 40).map_err(|e| e.to_string())));
            let state = app.session.active().unwrap();
            let model = state.doc.clone();
            let history = state.history.clone();
            let path = state.path.clone();
            let saved = state.saved_revision;
            let source = state.source.clone();
            let journal = app.session.journal.clone();
            assert!(app.save_to("refused.pcraft".into(), None).is_err());
            let state = app.session.active().unwrap();
            assert!(state.doc.shares_state(&model));
            assert!(state.history.shares_accounted_history(&history));
            assert_eq!((state.path.clone(), state.saved_revision, state.source.clone()), (path, saved, source));
            assert_eq!(app.session.journal, journal);
            assert!(capture.borrow().is_none());
            let baseline = owner.quota().usage().unwrap();
            let outer = json!({"command":"xcf.inspect","params":{},"wait":false});
            let (req, wake, wire) = ControlRequest::with_retained_query("engine.execute", outer.clone(), json!(71), 1 << 20);
            assert!(matches!(crate::owned_filter_inspect::handle(&mut app, &req, "xcf.inspect"), Some(Outcome::Deferred)));
            assert!(wake.try_recv().unwrap().is_null());
            let reply = wire.try_recv().unwrap();
            let value: Value = serde_json::from_slice(reply.bytes()).unwrap();
            assert_eq!(value["id"], 71);
            assert_eq!(value["ok"], true);
            assert_eq!(value["result"]["filters"]["total"], if empty { 0 } else { 1 });
            assert!(owner.quota().usage().unwrap().index > baseline.index);
            drop(reply);
            assert_eq!(owner.quota().usage().unwrap(), baseline);
            // Expired RPC/control prepass remains before authorizers and preserves id:null.
            app.services.automation_command = Some(Box::new(|_, _| Err("query denied".into())));
            let (mut req, _, wire) = ControlRequest::with_retained_query("engine.execute", outer.clone(), json!(72), 1 << 20);
            req.deadline = Some(std::time::Instant::now());
            let Some(Outcome::Done(value)) = crate::owned_filter_inspect::handle(&mut app, &req, "xcf.inspect") else { panic!("deadline refused") };
            assert_eq!(value["id"], Value::Null);
            assert_eq!(value["ok"], false);
            assert!(wire.try_recv().is_err());
            let (req, _, wire) = ControlRequest::with_retained_query("engine.execute", outer.clone(), json!(73), 1 << 20);
            let Some(Outcome::Done(value)) = crate::owned_filter_inspect::handle(&mut app, &req, "xcf.inspect") else { panic!("authorization refused") };
            assert!(value["error"].as_str().unwrap().contains("query denied"));
            assert!(wire.try_recv().is_err());
            app.services.automation_command = None;
            for limit in [32, 1 << 20] {
                let (req, _, wire) = ControlRequest::with_retained_query("engine.execute", outer.clone(), json!(74), limit);
                if limit == 1 << 20 {
                    drop(wire);
                    let outcome = crate::owned_filter_inspect::handle(&mut app, &req, "xcf.inspect");
                    assert!(matches!(outcome, Some(Outcome::Done(_))));
                } else {
                    assert!(matches!(crate::owned_filter_inspect::handle(&mut app, &req, "xcf.inspect"), Some(Outcome::Done(_))));
                    assert!(wire.try_recv().is_err());
                }
                assert_eq!(owner.quota().usage().unwrap(), baseline);
            }
            assert!(app.session.active().unwrap().doc.shares_state(&model));
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            assert!(!app.session.has_jobs());
        }
    }
}
