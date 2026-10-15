//! UNRUN actual pinned plain sources -> registered CURRENT Move. Captured native
//! bytes are independent raw harness references; no translated GIMP-source claim.
use super::*;
use photocraft_doc::{Document, LayerContent, Rect};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn cases() -> Vec<(PathBuf, Value, Vec<u8>)> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("precision-matrix");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "fc1bb8ff0635609a890860725d8449d18c0ffbec1a39b329f70fc23a8611a7d5");
    let data: Value = serde_json::from_slice(&bytes).unwrap();
    let ids = ["rgb-u8_non_linear", "rgb-u16_non_linear", "rgb-float_non_linear", "gray-u8_non_linear", "gray-u16_non_linear", "gray-float_non_linear"];
    let rows: Vec<_> = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| ids.contains(&row["id"].as_str().unwrap()))
        .map(|row| {
            let source = std::fs::read(root.join(row["input"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&source), row["input_sha256"].as_str().unwrap());
            let state = &row["state_before"];
            assert!(state["assigned_icc_sha256"].is_null());
            assert_eq!(state["layers"].as_array().unwrap().len(), 1);
            assert!(state["layers"][0]["filters"].as_array().unwrap().is_empty());
            let plane = &state["layers"][0]["native"];
            let raw = std::fs::read(root.join(plane["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&raw), plane["sha256"].as_str().unwrap());
            assert_eq!(raw.len() as u64, plane["bytes"].as_u64().unwrap());
            (root.clone(), row.clone(), raw)
        })
        .collect();
    assert_eq!(rows.len(), 6);
    rows
}
fn layer(doc: &Document) -> &photocraft_doc::BackedLayer {
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.layers[0].mask.is_none());
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("actual imported native owner") };
    backed
}
fn raw_current(doc: &Document) -> Vec<u8> {
    let backed = layer(doc);
    let rect = backed.operative_bounds();
    let bpp = backed.format().bytes_per_pixel();
    let mut raw = vec![0; rect.width() as usize * rect.height() as usize * bpp];
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let at = ((y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize) * bpp;
            backed.checked_view().read_logical_pixel_native_into(x, y, &mut raw[at..at + bpp], &ctl()).unwrap();
        }
    }
    raw
}
fn retained(before: &Document, current: &Document, dx: i32, dy: i32) {
    let a = layer(before);
    let b = layer(current);
    let old = a.operative_bounds();
    let rect = Rect::new(old.x0 + dx, old.y0 + dy, old.x1 + dx, old.y1 + dy);
    assert_eq!(b.operative_bounds(), rect);
    assert_eq!(b.format(), a.format());
    assert_eq!(b.default_bytes(), a.default_bytes());
    assert_eq!(raw_current(current), raw_current(before));
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.sample_encoding, before.sample_encoding);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    assert_eq!(
        (&current.layers[0].name, current.layers[0].visible, current.layers[0].opacity.to_bits()),
        (&before.layers[0].name, before.layers[0].visible, before.layers[0].opacity.to_bits())
    );
    let mut state = before.xcf_state.as_ref().unwrap().clone();
    state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(before.layers[0].id)).unwrap().raster_bounds = Some(rect);
    assert_eq!(current.xcf_state.as_ref().unwrap(), &state);
}
// Explicit CURRENT canvas reference: inverse-map independent original native
// samples and reuse the existing Normal kernel. No old source qualifier is widened.
fn reference(before: &Document, raw: &[u8], dx: i32, dy: i32) -> photocraft_compose::Buffer {
    use photocraft_compose::gimp_composite::{WorkingSpace, encoding};
    let backed = layer(before);
    let format = backed.format();
    let bounds = backed.operative_bounds();
    let current = photocraft_cms::xcf_color::effective_profile(
        None,
        if before.mode == photocraft_doc::ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
        encoding(before.sample_encoding.unwrap()),
    )
    .unwrap();
    let state = before.xcf_state.as_ref().unwrap();
    let policy = state
        .items
        .iter()
        .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(before.layers[0].id))
        .unwrap()
        .composition
        .as_ref()
        .unwrap()
        .resolved
        .as_ref()
        .unwrap();
    let working = WorkingSpace::new(&current, before.sample_encoding.unwrap(), policy.composite_space).unwrap();
    let mut px = Vec::new();
    for y in 0..before.size.height as i32 {
        for x in 0..before.size.width as i32 {
            let (sx, sy) = (x - dx, y - dy);
            let output = if bounds.contains(sx, sy) {
                let at = ((sy - bounds.y0) as usize * bounds.width() as usize + (sx - bounds.x0) as usize) * format.bytes_per_pixel();
                let sample = &raw[at..at + format.bytes_per_pixel()];
                let mut p = [0.; 4];
                for (channel, value) in p.iter_mut().enumerate().take(format.channels()) {
                    *value = photocraft_color::try_read_sample_f64(sample, format.sample, channel).unwrap();
                }
                let source = if before.mode == photocraft_doc::ColorMode::Grayscale { [p[0], p[0], p[0], p[1]] } else { p };
                working.normal([0.; 4], source, f64::from(before.layers[0].opacity), policy.operator).unwrap()
            } else {
                [0.; 4]
            };
            px.push(output.map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: before.bounds(), px }
}
#[test]
#[ignore = "UNRUN:six authored RGB/Gray plain NL sources through registered UI Move, rounding/targets/coalesce/history, inverse-native canvas and native continuation"]
fn plain_move_ui_keeps_raw_samples_and_original_source() {
    for (root, row, mut raw) in cases() {
        let name = row["input"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
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
        assert!(matches!(&imported, photocraft_io::OwnedXcfImport::Plain(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let id = before.layers[0].id;
        if cfg!(target_endian = "big") {
            for sample in raw.chunks_exact_mut(before.depth.bytes()) {
                sample.reverse();
            }
        }
        assert_eq!(raw_current(&before), raw);
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("move denied".into())));
        assert!(app.run("layer.translate", json!({"layer":id.0,"dx":1})).unwrap_err().contains("move denied"));
        app.session.authorize = None;
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let past = app.session.active().unwrap().history.past_len();
        assert_eq!(
            app.run("layer.translate", json!({"layer":before.layers[0].name,"dx":1.49,"dy":-0.5,"units":"px","coalesce":"own-move"})).unwrap(),
            Value::Null
        );
        retained(&before, &app.session.active().unwrap().doc, 1, -1);
        app.run("layer.translate", json!({"layer":id.0,"dx":0,"dy":1,"coalesce":"own-move"})).unwrap();
        let moved = app.session.active().unwrap().doc.clone();
        retained(&before, &moved, 1, 0);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        let expected = display_image(None, &reference(&before, &raw, 1, 0));
        let (actual, fee) = backed_snapshot_display_image(&moved, moved.size.width, moved.size.height, None, &Interrupt::NONE).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
        drop((actual, fee));
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        app.run("layer.translate", json!({"layer":"not a real layer","dx":0.49,"dy":-0.49,"coalesce":"own-move"})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&moved));
        for params in [json!({"layer":id.0,"dx":1,"target":"mask"}), json!({"layer":id.0,"dx":1,"units":"cm"}), json!({"layer":id.0,"dx":null,"units":"pt"})] {
            assert!(app.run("layer.translate", params).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&moved));
        }
        let named = moved.accounted().unwrap().try_with_document_name("Moved native", &ctl()).unwrap();
        let saved =
            photocraft_format::owned_document::save_current_document_archive(named.document(), Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
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
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Moved.pcraft", archive, &ctl()).unwrap();
        let current = reopened.session.active().unwrap();
        assert!(!current.source_read_only);
        assert!(current.path.is_none());
        assert_eq!(current.doc.name, "Moved native");
        retained(&before, &current.doc, 1, 0);
        let (actual, fee) = backed_snapshot_display_image(&current.doc, current.doc.size.width, current.doc.size.height, None, &Interrupt::NONE).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
        drop((actual, fee));
        reopened.run("layer.translate", json!({"dx":-1})).unwrap();
        retained(&before, &reopened.session.active().unwrap().doc, 0, 0);
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
    }
}
