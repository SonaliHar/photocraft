//! UNRUN synthetic CURRENT geometry/native consumers; no producer geometry or
//! new source eligibility oracle. The reference uses existing public CMS curves,
//! old Surface Box entry and pure Normal over transparent, as external harness.
use super::*;
#[path = "canvas_streamed_box_fixture.rs"]
mod fixture;
use fixture::ctl;
use photocraft_doc::{
    SampleType,
    import_compositing::{GimpOperator, GimpSpace},
    xcf_filters::{ExactNumber, FilterValue},
};
use serde_json::{Value, json};
fn reference(doc: &Document, literal: &fixture::Literal) -> photocraft_compose::Buffer {
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_deref();
    let profile = original.map_or_else(|| photocraft_cms::Builtin::Srgb.profile().clone(), |bytes| photocraft_cms::Profile::parse(bytes).unwrap());
    let curves = profile.trc.as_ref().unwrap();
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let FilterValue::Number(ExactNumber::Signed { width: 32, value: radius }) = node.arguments[0].value else { panic!("radius") };
    let mut linear = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Rgb, SampleType::F64, true));
    for y in literal.bounds.y0..literal.bounds.y1 {
        for x in literal.bounds.x0..literal.bounds.x1 {
            let p = fixture::sample(literal, x, y);
            let p = [curves[0].eval64(p[0]), curves[1].eval64(p[1]), curves[2].eval64(p[2]), p[3]];
            linear.try_write_pixel_f64(x, y, &p).unwrap();
        }
    }
    let extent = if node.enabled { photocraft_algo::gimp::neighborhood::box_blur_extent(literal.bounds, radius as u32).unwrap() } else { literal.bounds };
    let filtered = if node.enabled {
        Some(photocraft_algo::gimp::neighborhood::try_box_blur(&linear, extent, literal.bounds, radius as u32, Default::default(), &Interrupt::NONE).unwrap())
    } else {
        None
    };
    let normal =
        photocraft_compose::gimp_composite::WorkingSpace::new(&profile, photocraft_doc::SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    let opacity = if doc.layers[0].visible { f64::from(doc.layers[0].opacity) } else { 0.0 };
    let mut px = Vec::new();
    for y in 0..doc.size.height as i32 {
        for x in 0..doc.size.width as i32 {
            let p = if extent.contains(x, y) {
                if let Some(filtered) = &filtered {
                    let mut p = [0.0; 4];
                    filtered.try_read_pixel_f64(x, y, &mut p).unwrap();
                    [curves[0].eval_inverse64(p[0]), curves[1].eval_inverse64(p[1]), curves[2].eval_inverse64(p[2]), p[3]]
                } else {
                    fixture::sample(literal, x, y)
                }
            } else {
                [0.0; 4]
            };
            px.push(normal.normal([0.0; 4], p, opacity, GimpOperator::Union).unwrap().map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: doc.bounds(), px }
}
fn preview(snapshot: &photocraft_engine::DocumentSnapshot, literal: &fixture::Literal) {
    let expected = display_image(None, &reference(snapshot, literal));
    let (actual, fee) = backed_snapshot_display_image(snapshot, snapshot.size.width, snapshot.size.height, None, &Interrupt::NONE).unwrap();
    assert_eq!(actual.pixels.len(), expected.pixels.len());
    for (a, b) in actual.pixels.iter().zip(&expected.pixels) {
        for c in 0..4 {
            assert!((i16::from(a.to_array()[c]) - i16::from(b.to_array()[c])).abs() <= 1);
        }
    }
    drop((actual, fee));
}
#[test]
#[ignore = "UNRUN: paid synthetic float Box CURRENT arbitrary canvas/drawable, negative/tile-cross/wholly offcanvas, existing Surface+CMS reference, radius/activation/history/native/query/control; no GIMP geometry/source or low-input-memory claim"]
fn current_streamed_box_ui_native_geometry_and_existing_transactions() {
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for variant in 0..2 {
            let (owner, literal) = fixture::fixture(sample, assigned, variant);
            let q = owner.quota().clone();
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut app, "CURRENT.pcraft", fixture::archive(&owner), &ctl()).unwrap();
            app.background_jobs = true;
            let before = app.session.active().unwrap().doc.clone();
            assert_eq!(fixture::native(&before), literal.raw);
            assert!(!app.session.active().unwrap().source_read_only);
            assert!(app.session.active().unwrap().path.is_none());
            assert!(q.shares_ledger(before.accounted().unwrap().quota()));
            assert_eq!(before.accounted().unwrap().is_readonly_assigned_point_storage(), assigned);
            preview(&before, &literal);
            let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
            let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
                "engine.execute",
                json!({"command":"xcf.filter.inspect","params":{"filterId":id},"wait":false}),
                json!(7),
                1 << 20,
            );
            assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
            assert!(wake.try_recv().unwrap().is_null());
            let wire = carrier.try_recv().unwrap();
            let v: Value = serde_json::from_slice(wire.bytes()).unwrap();
            assert_eq!(v["ok"], true);
            drop(wire);
            let stop = || true;
            assert!(backed_snapshot_display_image(&before, before.size.width, before.size.height, None, &Interrupt::cancel_only(&stop)).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("streamed Box denied".into())));
            assert!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap_err().contains("streamed Box denied"));
            app.session.authorize = None;
            let changed = if assigned {
                assert_eq!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
                app.session.active().unwrap().doc.clone()
            } else {
                assert_eq!(
                    app.run("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":32,"value":5}}}}]}))
                        .unwrap()["updated"],
                    true
                );
                app.session.active().unwrap().doc.clone()
            };
            assert!(!app.session.has_jobs());
            assert_eq!(fixture::native(&changed), literal.raw);
            preview(&changed, &literal);
            app.run("edit.undo", Value::Null).unwrap();
            let history = app.session.active().unwrap().history.clone();
            assert_eq!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            app.run("edit.redo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&changed));
            let titled = changed.accounted().unwrap().try_with_document_name("Streamed Box CURRENT saved title", &ctl()).unwrap();
            let mut native = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut native, "other.pcraft", fixture::archive(&titled), &ctl()).unwrap();
            let current = &native.session.active().unwrap().doc;
            assert_eq!(current.name, "Streamed Box CURRENT saved title");
            fixture::exact(&changed, current);
            preview(current, &literal);
        }
    }
}
