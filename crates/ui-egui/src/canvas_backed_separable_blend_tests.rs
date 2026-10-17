//! UNRUN actual explicit byte/native UI paths, not default-loader enrollment.
use super::*;
#[path = "canvas_separable_blend_fixture.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::{cases, ctl, observed, retained};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::{Value, json};
use std::{io::Cursor, path::Path};
fn preview(snapshot: &photocraft_engine::DocumentSnapshot, root: &Path, case: &fixture::Case) {
    let png = photocraft_codecs::decode(&std::fs::read(root.join(&case.png)).unwrap()).unwrap();
    let (actual, fee) = backed_snapshot_display_image(snapshot, 16, 12, None, &Interrupt::NONE).unwrap();
    assert_eq!(actual.size, [16, 12]);
    // Independent saved/reopened GIMP PNG, matched to the UI premultiplied
    // display boundary. Exact source/native words are checked separately.
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
#[ignore = "UNRUN: all50 authored flat separable sources/edit byte opens, actual retained project transport, PNG display, title/native continuation and mutation closure"]
fn fifty_flat_blends_keep_native_policy_and_pathless_custody() {
    let (root, cases) = cases();
    for case in cases {
        let input = std::fs::read(root.join(&case.file)).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let source = photocraft_io::import_owned_xcf_from_reader_in_quota(
            &case.file,
            &mut Cursor::new(&input),
            input.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(matches!(&source, photocraft_io::OwnedXcfImport::SeparableBlend(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, &case.file, &source, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let owner = before.accounted().unwrap();
        assert!(owner.is_readonly_flat_composite_storage());
        owner.validate_readonly_flat_composite_storage(&ctl()).unwrap();
        assert!(app.session.active().unwrap().path.is_none());
        assert!(app.session.active().unwrap().source_read_only);
        observed(&root, &before, &case);
        preview(&before, &root, &case);
        let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
            "engine.execute",
            json!({"command":"xcf.inspect","params":null,"wait":false}),
            json!(17),
            1 << 20,
        );
        assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
        assert!(wake.try_recv().unwrap().is_null());
        let wire = carrier.try_recv().unwrap();
        let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["result"]["items"]["total"], 2);
        assert_eq!(value["result"]["filters"]["total"], 0);
        assert_eq!(value["result"]["filterStacks"]["total"], 0);
        drop(wire);
        assert!(photocraft_engine::accounted_filter_inspect::prepare(&app.session, &json!({"filterId":"1"}), &ctl()).is_err());
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        let journal = app.session.journal.clone();
        for (id, params) in [
            ("layer.setProps", json!({"blend":"multiply"})),
            ("layer.translate", json!({"dx":1})),
            ("select.all", Value::Null),
            ("xcf.filter.enable", json!({"filterId":"1","enabled":false})),
        ] {
            assert!(app.run(id, params).is_err());
        }
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.journal, journal);
        let cancel = || true;
        assert!(backed_snapshot_display_image(&before, 16, 12, None, &Interrupt::cancel_only(&cancel)).is_err());
        let title = owner.try_with_document_name("Saved separable title", &ctl()).unwrap();
        retained(&before, title.document());
        // Cursor output is an external harness capture, not post-hoc quota proof.
        let encoded =
            photocraft_format::owned_document::save_current_document_archive(title.document(), Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                .unwrap()
                .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(encoded.as_slice()),
            encoded.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Picker label.pcraft", archive, &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.name, "Saved separable title");
        assert!(current.accounted().unwrap().is_readonly_flat_composite_storage());
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(reopened.session.active().unwrap().path.is_none());
        retained(&before, current);
        observed(&root, current, &case);
        preview(current, &root, &case);
        assert_eq!(std::fs::read(root.join(&case.file)).unwrap(), input);
    }
}
