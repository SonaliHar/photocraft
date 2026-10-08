use photocraft_doc::{LayerContent, RawDevelopment, RawWhiteBalance, SmartSource};
use photocraft_engine::Session;
use photocraft_raw::testgen::{DngSpec, mosaic, scene};
use serde_json::json;

fn fixture() -> (Session, Vec<u8>) {
    let (w, h) = (32, 24);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    let bytes = spec.build();
    let doc = photocraft_io::raw_smart::import("camera.dng", &bytes).unwrap().document;
    let mut s = Session::new();
    s.add_document(doc, None);
    (s, bytes)
}

fn contents(s: &Session) -> (&RawDevelopment, &[u8]) {
    let state = s.active().unwrap();
    let LayerContent::Smart(sm) = &state.doc.layer(state.active_layer.unwrap()).unwrap().content else { panic!("not smart") };
    let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("not embedded") };
    (sm.raw_development.as_ref().unwrap(), bytes)
}

#[test]
fn sensor_settings_redevelop_and_roundtrip_without_changing_original_bytes() {
    let (mut s, bytes) = fixture();
    let before = photocraft_compose::flatten(&s.active().unwrap().doc).px;
    let settings = RawDevelopment { exposure: -1.5, white_balance: RawWhiteBalance::Multipliers { rgb: [1.7, 1.0, 1.2] }, ..Default::default() };
    s.execute("layer.rawDevelopment.set", json!({"settings": settings})).unwrap();
    let after = photocraft_compose::flatten(&s.active().unwrap().doc).px;
    assert_ne!(before, after);
    assert_eq!(contents(&s), (&settings, bytes.as_slice()));
    let fresh = photocraft_io::raw_smart::develop("camera.dng", &bytes, &settings).unwrap().document;
    let fresh = photocraft_compose::flatten(&fresh).px;
    let max_diff = fresh.iter().zip(&after).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0f32, f32::max);
    assert!(max_diff < 0.002, "redevelopment differs from fresh sensor rendering: {max_diff}");
    assert!(s.undo());
    assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, before);
    assert!(s.redo());
    assert_eq!(contents(&s).1, bytes);
    let project = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&project).unwrap();
    s.close(0);
    s.add_document(doc, None);
    assert_eq!(contents(&s), (&settings, bytes.as_slice()));
    assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, after);
    assert!(s.execute("layer.smartObjects.editContents", json!({})).is_err(), "headless generic Edit Contents must not bake RAW into a child project");
    let warnings = photocraft_io::export(&s.active().unwrap().doc, "raw.psd", &Default::default()).unwrap().warnings;
    assert!(warnings.iter().any(|w| w.contains("development settings require .pcraft")));
}

#[test]
fn invalid_settings_fail_without_altering_document() {
    let (mut s, bytes) = fixture();
    let original = s.active().unwrap().revision;
    for settings in [
        json!({}),
        json!({"version":2,"exposure":0,"whiteBalance":{"mode":"asShot"},"demosaic":"ahd"}),
        json!({"version":1,"exposure":100,"whiteBalance":{"mode":"asShot"},"demosaic":"ahd"}),
        json!({"version":1,"exposure":0,"whiteBalance":{"mode":"multipliers","rgb":[0,1,1]},"demosaic":"ahd"}),
    ] {
        assert!(s.execute("layer.rawDevelopment.set", json!({"settings":settings})).is_err());
        assert_eq!(s.active().unwrap().revision, original);
        assert_eq!(contents(&s).1, bytes);
    }
}

#[test]
fn redevelopment_respects_document_sample_depth() {
    for depth in [photocraft_color::SampleType::U8, photocraft_color::SampleType::U16, photocraft_color::SampleType::F32] {
        let (mut s, bytes) = fixture();
        s.edit("depth", |doc, _| {
            doc.depth = depth;
            Ok(())
        })
        .unwrap();
        s.execute("layer.rawDevelopment.set", json!({"settings":RawDevelopment { exposure:-0.5, ..Default::default() }})).unwrap();
        let state = s.active().unwrap();
        let LayerContent::Smart(sm) = &state.doc.layers[0].content else { panic!("not smart") };
        assert_eq!(sm.cache.as_ref().unwrap().format().sample, depth);
        assert_eq!(contents(&s).1, bytes);
    }
}

#[test]
fn unpacked_sensor_and_original_nef_remain_distinct_after_save_and_redevelopment() {
    let (_, sensor) = fixture();
    let original = b"original NEF bytes remain authoritative";
    let mut s = Session::new();
    let doc = photocraft_io::raw_smart::import_with_sensor("camera.NEF", original, Some(("sensor.dng", &sensor))).unwrap().document;
    s.add_document(doc, None);
    let settings = RawDevelopment { exposure: -1.0, ..Default::default() };
    s.execute("layer.rawDevelopment.set", json!({"settings":settings})).unwrap();
    assert_eq!(contents(&s), (&settings, original.as_slice()));
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut restored = photocraft_format::load_from_bytes(&bytes).unwrap();
    let id = restored.layers[0].id;
    assert!(photocraft_engine::smart_cmds::refresh(&mut restored, id).unwrap());
    let LayerContent::Smart(sm) = &restored.layers[0].content else { panic!("not smart") };
    let Some(SmartSource::Embedded { bytes, .. }) = &sm.raw_decode_source else { panic!("missing sensor") };
    assert_eq!(bytes.as_slice(), sensor);
    let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("missing original") };
    assert_eq!(bytes.as_slice(), original);
}
