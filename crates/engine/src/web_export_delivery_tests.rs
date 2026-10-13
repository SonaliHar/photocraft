use super::*;
use crate::Session;
use serde_json::json;
use std::sync::{Mutex, PoisonError};
static SERIAL: Mutex<()> = Mutex::new(());
static CAPTURE: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
fn capture(name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
    CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).push((name.into(), bytes.to_vec()));
    Ok(())
}
fn refuse(_: &str, _: &[u8]) -> std::result::Result<(), String> {
    Err("injected delivery failure".into())
}
fn clear() {
    CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).clear();
}
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":24,"height":16,"depth":16,"background":"white"})).unwrap();
    s.edit("Owned physical metadata", |doc, _| {
        doc.physical_resolution = Some(photocraft_doc::PhysicalResolution::square(144.25));
        doc.physical_resolution.as_mut().unwrap().y_dpi = 287.75;
        doc.resolution_dpi = 300.0;
        doc.metadata.xmp = Some("<x:xmpmeta>test</x:xmpmeta>".into());
        Ok(())
    })
    .unwrap();
    s
}
fn params(name: &str) -> serde_json::Value {
    json!({"path":name,"format":"png24","metadata":"all","convertToSrgb":false})
}
#[test]
fn registered_whole_delivery_uses_canonical_density_without_native_write() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let mut s = session();
    s.set_web_export_delivery(Some(capture));
    let source = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let file = format!("photocraft-delivery-whole-{}.png", std::process::id());
    assert!(!std::path::Path::new(&file).exists(), "exclusive regression output must not preexist");
    let result = s.execute("file.export.saveForWebLegacy", params(&file)).unwrap();
    let native_write = std::path::Path::new(&file).exists();
    if native_write {
        std::fs::remove_file(&file).unwrap();
    }
    let captured = CAPTURE.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(captured.len(), 1, "registered command must use explicit byte delivery");
    assert!(!native_write, "capability must not write native files");
    assert_eq!(captured[0].0, file);
    let bytes = &captured[0].1;
    let offset = bytes.windows(4).position(|w| w == b"pHYs").unwrap() + 4;
    assert_eq!(u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()), 5679);
    assert_eq!(u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()), 11329);
    assert_eq!(result["width"], 24);
    assert_eq!(source, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap());
}
#[test]
fn registered_delivery_failure_preserves_preferences_and_native_state() {
    let mut s = session();
    s.set_web_export_delivery(Some(refuse));
    s.file_menu.last_web = Some(json!({"preset":"old"}));
    let old = s.file_menu.last_web.clone();
    let revision = s.active().unwrap().revision;
    let native = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    assert!(s.execute("file.export.saveForWebLegacy", params("failure.png")).is_err());
    assert_eq!(old, s.file_menu.last_web);
    assert_eq!(revision, s.active().unwrap().revision);
    assert_eq!(native, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap());
}
#[test]
fn delivery_bounds_names_seek_and_callback_failures() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let delivery = Delivery { callback: capture, limits: Limits { output: 1024, metadata: 256, entries: 1, name: 16 } };
    assert!(delivery.send("ok.png", &[0; 1024]).is_ok());
    assert!(delivery.send("ok.png", &[0; 1025]).is_err());
    for name in ["", "../x", "/x", "a\\x", "a:x", "a//x", "abcdefghijklmnopq"] {
        assert!(delivery.send(name, &[]).is_err(), "{name}");
    }
    let mut output = BoundedOutput { data: Vec::new(), position: 0, cap: 2 };
    output.write_all(&[1, 2]).unwrap();
    assert!(output.write_all(&[3]).is_err());
    assert!(output.seek(SeekFrom::Current(-3)).is_err());
    assert!(output.seek(SeekFrom::Start(3)).is_err());
    assert!(Delivery::new(refuse).send("x.png", &[]).is_err());
}
#[test]
fn archive_preserves_relative_paths_and_is_delivered_once() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let mut archive = Delivery::new(capture).archive().unwrap();
    archive.add("images/a.png", b"image").unwrap();
    archive.add("page.html", b"<img src=\"images/a.png\">").unwrap();
    archive.finish("page.zip").unwrap();
    let captured = CAPTURE.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(captured.len(), 1);
    let mut reader =
        photocraft_format::zip::stream::Reader::new(Cursor::new(&captured[0].1), ZipLimits::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(reader.read_by_name("images/a.png", 32, &photocraft_raster::Interrupt::NONE).unwrap(), b"image");
    assert_eq!(reader.read_by_name("page.html", 64, &photocraft_raster::Interrupt::NONE).unwrap(), b"<img src=\"images/a.png\">");
}
#[test]
fn archive_limits_fail_before_delivery() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let delivery = Delivery { callback: capture, limits: Limits { output: 512, metadata: 256, entries: 1, name: 32 } };
    let mut archive = delivery.archive().unwrap();
    archive.add("a", b"a").unwrap();
    assert!(archive.add("b", b"b").is_err());
    assert!(archive.finish("x.zip").is_err());
    assert!(CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
}

#[test]
fn planner_grid_and_merge_work_are_checked_before_algorithm_calls() {
    let canvas = photocraft_geom::Rect::new(0, 0, 100, 100);
    let taken = [photocraft_geom::Rect::new(20, 20, 60, 60)];
    let mut exact = PlanBudget { work_left: 90, runs_left: 9 };
    assert_eq!(exact.before_grid(canvas, &mut taken.iter().copied(), 1).unwrap(), 9);
    assert_eq!(exact.work_left, 0);
    assert_eq!(exact.runs_left, 0);
    assert!(PlanBudget { work_left: 89, runs_left: 9 }.before_grid(canvas, &mut taken.iter().copied(), 1).is_err());
    assert!(PlanBudget { work_left: 90, runs_left: 8 }.before_grid(canvas, &mut taken.iter().copied(), 1).is_err());
    let crossing: Vec<_> = (0..128).map(|i| photocraft_geom::Rect::new(i * 2, i * 2, i * 2 + 1, i * 2 + 1)).collect();
    assert!(PlanBudget::default().before_grid(photocraft_geom::Rect::new(0, 0, 1024, 1024), &mut crossing.iter().copied(), crossing.len()).is_err());
}
#[test]
fn preflight_source_refuses_labels_metadata_numbers_and_stored_counts_before_copy() {
    let mut doc =
        photocraft_doc::Document::new("synthetic", photocraft_doc::Size::new(100, 100), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let d = lower(4096, 256, 4, 32);
    let p = serde_json::json!({});
    d.preflight_source(&doc, &p).unwrap();
    doc.slices.list.push(photocraft_doc::slices::Slice::default());
    doc.slices.list[0].name = "x".repeat(901);
    assert!(d.preflight_source(&doc, &p).is_err());
    doc.slices.list[0].name.clear();
    doc.slices.list[0].cell_text = "x".repeat(43);
    assert!(d.preflight_source(&doc, &p).is_err());
    doc.slices.list[0].cell_text.clear();
    assert!(d.preflight_source(&doc, &serde_json::json!({"numbers":[1,2,3,4,5]})).is_err());
    assert!(d.preflight_source(&doc, &serde_json::json!({"numbers":["oversized ignored string"]})).is_err());
    doc.slices.list.resize(5, photocraft_doc::slices::Slice::default());
    assert!(d.preflight_source(&doc, &p).is_err());
}

#[test]
fn registered_crossing_slice_preflight_refuses_before_source_copy_or_delivery() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let mut s = Session::new();
    s.execute("file.new", json!({"width":1024,"height":1024,"background":"transparent"})).unwrap();
    s.edit("Owned crossing slices", |doc, _| {
        for i in 0..128 {
            doc.slices.list.push(photocraft_doc::slices::Slice {
                id: i as u32 + 1,
                rect: photocraft_geom::Rect::new(i * 2, i * 2, i * 2 + 1, i * 2 + 1),
                ..Default::default()
            });
        }
        Ok(())
    })
    .unwrap();
    s.set_web_export_delivery(Some(capture));
    s.file_menu.last_web = Some(json!({"preset":"old"}));
    let before = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let prefs = s.file_menu.last_web.clone();
    let error = s.execute("file.export.saveForWebLegacy", json!({"path":"crossing.zip","dir":".","slices":"all","html":true})).unwrap_err();
    assert!(error.to_string().contains("slice grid work/run budget"), "{error}");
    assert!(CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
    assert_eq!(prefs, s.file_menu.last_web);
    assert_eq!(before, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap());
}
fn lower(output: usize, metadata: u64, entries: u64, name: usize) -> Delivery {
    assert!(output <= 64 << 20 && metadata <= 1 << 20 && entries <= 4096 && name <= 1024);
    Delivery { callback: capture, limits: Limits { output, metadata, entries, name } }
}
#[test]
fn stacking_work_cap_below_at_above_is_precharged_before_staging() {
    // Two selected slices, three stored slices:2*3*(2*2+1)=30 worst comparisons.
    let mut below = PlanBudget { work_left: 29, runs_left: 4096 };
    let mut source_copy = false;
    if below.charge_stacking_scans(2, 3).is_ok() {
        source_copy = true;
    }
    assert!(!source_copy);
    assert_eq!(below.work_left, 29);
    let mut at = PlanBudget { work_left: 30, runs_left: 4096 };
    at.charge_stacking_scans(2, 3).unwrap();
    assert_eq!(at.work_left, 0);
    let mut above = PlanBudget { work_left: 31, runs_left: 4096 };
    above.charge_stacking_scans(2, 3).unwrap();
    assert_eq!(above.work_left, 1);
    assert!(PlanBudget::default().charge_stacking_scans(usize::MAX, 1).is_err());
    let mut zero = PlanBudget { work_left: 0, runs_left: 0 };
    zero.charge_stacking_scans(0, 4096).unwrap();
}
#[test]
fn selected_upper_preserves_numbers_precedence_over_user_slices() {
    assert_eq!(PlanBudget::selected_upper(5, 1, &serde_json::json!({"slices":"user"})), 1);
    assert_eq!(PlanBudget::selected_upper(5, 1, &serde_json::json!({"slices":"user","numbers":[1,2,3,4,5]})), 5);
    assert_eq!(PlanBudget::selected_upper(5, 1, &serde_json::json!({"slices":"user","numbers":[1,2]})), 2);
    assert_eq!(PlanBudget::selected_upper(5, 1, &serde_json::json!({"numbers":[]})), 0);
    assert_eq!(PlanBudget::selected_upper(5, 1, &serde_json::json!({"slices":"all"})), 5);
}

fn patterned_session() -> Session {
    let mut s = session();
    s.edit("Owned pixel pattern", |doc, active| {
        let fmt = doc.pixel_format();
        let mut layer = photocraft_doc::Layer::raster("Owned pattern", fmt);
        let surface = layer.surface_mut().unwrap();
        for y in 0..16 {
            for x in 0..24 {
                surface.fill_rect(
                    photocraft_geom::Rect::new(x, y, x + 1, y + 1),
                    &photocraft_raster::from_rgba(&fmt, [x as f32 / 23.0, y as f32 / 15.0, ((x + y) % 7) as f32 / 6.0, 1.0]),
                );
            }
        }
        let id = doc.insert_above(None, layer);
        *active = Some(id);
        Ok(())
    })
    .unwrap();
    s
}
#[test]
fn registered_slice_zip_matches_native_outputs_and_preserves_state() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let mut s = patterned_session();
    s.execute("slice.new", json!({"rect":[2,3,8,6],"name":"tile"})).unwrap();
    s.execute("slice.new", json!({"rect":[13,5,22,14],"name":"tile"})).unwrap();
    let native = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let revision = s.active().unwrap().revision;
    let history = s.active().unwrap().history.past_len();
    let source = format!("{:?}", s.active().unwrap().source);
    let dir = std::env::temp_dir().join(format!("photocraft-native-web-delivery-{}", std::process::id()));
    assert!(!dir.exists(), "exclusive native oracle directory must not preexist");
    let dir_name = dir.to_string_lossy().into_owned();
    let path = dir.join("web.html").to_string_lossy().into_owned();
    let native_result = s
        .execute(
            "file.export.saveForWebLegacy",
            json!({"path":path,"dir":dir_name,"format":"png24","metadata":"all","convertToSrgb":false,"slices":"user","html":true}),
        )
        .unwrap();
    s.set_web_export_delivery(Some(capture));
    let web = s
        .execute(
            "file.export.saveForWebLegacy",
            json!({"path":"web.zip","dir":".","format":"png24","metadata":"all","convertToSrgb":false,"slices":"user","html":true}),
        )
        .unwrap();
    let calls = CAPTURE.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "web.zip");
    assert_eq!(web["download"]["bytes"], calls[0].1.len());
    let mut zip = photocraft_format::zip::stream::Reader::new(Cursor::new(&calls[0].1), ZipLimits::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    for file in native_result["files"].as_array().unwrap() {
        let file = std::path::Path::new(file.as_str().unwrap());
        let relative = file.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
        let bytes = std::fs::read(file).unwrap();
        assert_eq!(zip.read_by_name(&relative, 1 << 20, &photocraft_raster::Interrupt::NONE).unwrap(), bytes);
    }
    assert_eq!(zip.read_by_name("web.html", 1 << 20, &photocraft_raster::Interrupt::NONE).unwrap(), std::fs::read(dir.join("web.html")).unwrap());
    assert_eq!(
        zip.read_by_name("images/spacer.gif", 1 << 20, &photocraft_raster::Interrupt::NONE).unwrap(),
        std::fs::read(dir.join("images/spacer.gif")).unwrap()
    );
    assert_eq!(native, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap());
    assert_eq!(revision, s.active().unwrap().revision);
    assert_eq!(history, s.active().unwrap().history.past_len());
    assert_eq!(source, format!("{:?}", s.active().unwrap().source));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn registered_metadata_formats_and_legacy_boundary_have_no_extra_delivery() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let mut s = patterned_session();
    s.set_web_export_delivery(Some(capture));
    for format in ["png24", "png8", "gif", "jpeg", "wbmp"] {
        s.execute("file.export.saveForWebLegacy", json!({"path":format!("format.{format}"),"format":format,"metadata":"none","convertToSrgb":false})).unwrap();
    }
    {
        let calls = CAPTURE.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(calls.len(), 5);
        assert!(!calls[0].1.windows(4).any(|x| x == b"pHYs"));
    }
    s.edit("Owned hostile legacy boundary", |doc, _| {
        doc.physical_resolution = None;
        doc.resolution_dpi = f32::from_bits(1280316329);
        Ok(())
    })
    .unwrap();
    let prefs = s.file_menu.last_web.clone();
    let before = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    assert!(s.execute("file.export.saveForWebLegacy", params("boundary.png")).is_err());
    assert_eq!(CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).len(), 5);
    assert_eq!(prefs, s.file_menu.last_web);
    assert_eq!(before, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap());
    let result = s.execute("file.export.saveForWebLegacy", json!({"format":"png24","metadata":"none"})).unwrap();
    assert!(result["bytes"].as_u64().unwrap() > 0);
    assert_eq!(CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).len(), 5);
}
fn deny_export(id: &str, _: &serde_json::Value) -> crate::Result<()> {
    if id == "file.export.saveForWebLegacy" { Err(crate::EngineError::Other("injected authorization refusal".into())) } else { Ok(()) }
}
#[test]
fn registered_actions_authorization_events_and_delivery_failure_share_one_path() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    clear();
    let mut s = session();
    s.set_web_export_delivery(Some(refuse));
    s.execute("file.scripts.scriptEventsManager", json!({"enabled":true,"add":{"event":"export","name":"Owned empty export event","steps":[]}})).unwrap();
    s.file_menu.last_web = Some(json!({"preset":"old"}));
    let prefs = s.file_menu.last_web.clone();
    let events = s.file_menu.event_log.len();
    let before = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    assert!(s.execute("file.export.saveForWebLegacy", params("action.png")).is_err());
    assert_eq!(events, s.file_menu.event_log.len());
    assert_eq!(prefs, s.file_menu.last_web);
    s.set_web_export_delivery(Some(capture));
    s.authorize = Some(deny_export);
    assert!(s.execute("file.export.saveForWebLegacy", params("action.png")).is_err());
    assert_eq!(events, s.file_menu.event_log.len());
    assert!(CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
    s.authorize = None;
    s.execute("actions.record", json!({"name":"Owned Web Delivery"})).unwrap();
    s.execute("file.export.saveForWebLegacy", params("action.png")).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let play = s.execute("actions.play", json!({"action":"Owned Web Delivery"})).unwrap();
    assert!(play.get("failed").is_none(), "{play}");
    assert_eq!(CAPTURE.lock().unwrap_or_else(PoisonError::into_inner).len(), 2);
    assert_eq!(s.file_menu.event_log.len(), events + 2);
    assert_eq!(before, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap());
}

#[test]
fn direct_zip64_matches_old_stream_writer_exact_bytes() {
    let large: Vec<u8> = (0..65543).map(|i| (i % 251) as u8).collect();
    let cases: Vec<Vec<(&str, &[u8])>> = vec![
        vec![],
        vec![("empty.bin", b"")],
        vec![("images/é.png", b"synthetic")],
        vec![("images/a.png", &large), ("page.html", b"<img src=\"images/a.png\">"), ("empty.bin", b"")],
    ];
    for entries in cases {
        let limits = Limits::default();
        let mut direct = Zip64Bytes::new(limits);
        let mut stream = Writer::new(
            Cursor::new(Vec::new()),
            ZipLimits { max_archive_bytes: limits.output as u64, max_metadata_bytes: limits.metadata, max_entries: limits.entries },
        )
        .unwrap();
        for (name, bytes) in entries {
            direct.add(name, bytes).unwrap();
            stream.add(name, bytes.len() as u64, &mut Cursor::new(bytes), &photocraft_raster::Interrupt::NONE).unwrap();
        }
        assert_eq!(direct.finish().unwrap(), stream.finish(&photocraft_raster::Interrupt::NONE).unwrap().into_inner());
    }
}
