//! UNRUN synthetic CURRENT raw-tuple control, separate from six actual originals.
use super::*;
#[path = "canvas_observed_box_fixture.rs"]
#[allow(dead_code)]
mod observed;
#[test]
#[ignore = "UNRUN: observed Box preserves opaque raw tuple through paid CURRENT native/title/query/canvas; unrelated mutations and malformed filter edits refuse, no normalization or source proof"]
fn observed_box_current_raw_tuple_is_not_mutation_authority() {
    // These six local originals are independently saved GIMP files. Their
    // source/native bindings are distinct from the synthetic raw control below;
    // the display reference is an existing PhotoCraft adapter, not GIMP proof.
    for (_, bytes) in observed::source_cases() {
        let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
            resident: 256 << 20,
            index: 32 << 20,
            scratch: 0,
        });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let mut options = photocraft_xcf::DecodeOptions::default();
        options.limits.resident_bytes = 64 << 20;
        options.limits.input_bytes = 16 << 20;
        let imported = photocraft_io::import_owned_xcf_bytes_in_quota("observed.xcf", std::sync::Arc::new(bytes), &options, &limits, q, &ctl()).unwrap();
        assert!(matches!(&imported, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "observed.xcf", &imported, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let photocraft_doc::LayerContent::BackedRaster(layer) = &before.layers[0].content else { panic!("source native") };
        let literal = fixture::Literal { bounds: layer.operative_bounds(), raw: fixture::native(&before), sample: SampleType::F32 };
        assert_eq!(observed::raw(&before), [1654948433; 3]);
        assert!(app.session.active().unwrap().source_read_only);
        super::preview(&before, &literal);
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "observed.pcraft", fixture::archive(before.accounted().unwrap()), &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        fixture::exact(&before, current);
        super::preview(current, &literal);
    }
    let (base, literal) = fixture::fixture(SampleType::F32, false, 0);
    let raw = [i32::MIN, 0, i32::MAX];
    let owner = observed::with_raw(&base, raw, &ctl());
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_native_open::adopt_bytes(&mut app, "observed.pcraft", fixture::archive(&owner), &ctl()).unwrap();
    app.background_jobs = true;
    let before = app.session.active().unwrap().doc.clone();
    assert_eq!(observed::raw(&before), raw);
    assert_eq!(fixture::native(&before), literal.raw);
    assert_eq!(before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch, observed::EPOCH);
    super::preview(&before, &literal); // old full Surface algorithm, then crop
    let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let history = app.session.active().unwrap().history.clone();
    let revision = app.session.active().unwrap().revision;
    let journal = app.session.journal.len();
    for (command, params) in [
        ("xcf.filter.enable", json!({"filterId":id,"enabled":null})),
        ("xcf.filter.enable", json!({"filterId":id,"enabled":false,"opacityF64Bits":"0x3ff0000000000000"})),
        ("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":32,"value":1001}}}}]})),
        ("layer.setProps", json!({"name":"closed"})),
        ("layer.translate", json!({"dx":1})),
        ("select.all", Value::Null),
        ("edit.clear", Value::Null),
    ] {
        assert!(app.run(command, params).is_err(), "{command}");
    }
    assert!(app.session.active().unwrap().doc.shares_state(&before));
    assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.journal.len(), journal);
    assert!(!app.session.has_jobs());
    let (request, wake, carrier) =
        crate::control::ControlRequest::with_retained_query("engine.execute", json!({"command":"xcf.inspect","params":null,"wait":false}), json!(7), 1 << 20);
    assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
    assert!(wake.try_recv().unwrap().is_null());
    let wire = carrier.try_recv().unwrap();
    let response: Value = serde_json::from_slice(wire.bytes()).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["filters"]["total"], 1);
    drop(wire);
    let titled = before.accounted().unwrap().try_with_document_name("Observed CURRENT title", &ctl()).unwrap();
    let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &ctl()).unwrap();
    let current = &reopened.session.active().unwrap().doc;
    assert_eq!(current.name, "Observed CURRENT title");
    assert_eq!(observed::raw(current), raw);
    fixture::exact(&before, current);
    super::preview(current, &literal);
}
