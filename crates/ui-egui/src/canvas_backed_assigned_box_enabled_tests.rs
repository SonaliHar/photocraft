//! UNRUN registered assigned Box activation; disabled CURRENT reference uses
//! the existing resident adapter, never a new GIMP-authored disabled claim.
use super::*;
#[path = "canvas_assigned_box_fixture.rs"]
mod fixture;
use fixture::ctl;
use serde_json::{Value, json};
#[test]
#[ignore = "UNRUN: both assigned Box originals through synchronous set/enable, one undo/noopredo, immutable native/profile/mask/source, disabled current canvas and paid native continuation"]
fn assigned_box_ui_activation_is_atomic_and_native_continues() {
    for (path, bytes) in fixture::cases() {
        let name = path.file_name().unwrap().to_str().unwrap();
        let resident = photocraft_io::import(name, &bytes).unwrap().document;
        let mut disabled_ref = resident.clone();
        disabled_ref.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let r = photocraft_compose::gimp_render::PreparedRenderer::new(&disabled_ref).unwrap();
        let expected = display_image(None, &r.render(resident.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap());
        let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
            resident: 256 << 20,
            index: 32 << 20,
            scratch: 0,
        });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let source = photocraft_io::import_owned_xcf_from_reader_in_quota(
            name,
            &mut std::io::Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q,
            &ctl(),
        )
        .unwrap();
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &source, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let past = app.session.active().unwrap().history.past_len();
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("assigned activation denied".into())));
        assert!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap_err().contains("assigned activation denied"));
        app.session.authorize = None;
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.run("xcf.filter.set", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
        assert!(!app.session.has_jobs());
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Edit GIMP Stock Filter"));
        let disabled = app.session.active().unwrap().doc.clone();
        let mut expected_doc: Document = (*before).clone();
        let stack = &mut expected_doc.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.nodes[0].enabled = false;
        stack.revision += 1;
        fixture::exact(&expected_doc, &disabled);
        fixture::observed(&disabled);
        let (image, fee) = backed_snapshot_display_image(&disabled, 16, 12, None, &Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels);
        drop((image, fee));
        app.run("edit.undo", Value::Null).unwrap();
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        assert_eq!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.run("edit.redo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&disabled));
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "disabled.pcraft", fixture::archive(disabled.accounted().unwrap()), &ctl()).unwrap();
        fixture::exact(&disabled, &reopened.session.active().unwrap().doc);
        assert_eq!(reopened.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.run("xcf.filter.set", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        let current = &reopened.session.active().unwrap().doc;
        assert!(current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        fixture::observed(current);
        assert_eq!(current.icc_profile, before.icc_profile);
        assert_eq!(current.source_color_context, before.source_color_context);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
