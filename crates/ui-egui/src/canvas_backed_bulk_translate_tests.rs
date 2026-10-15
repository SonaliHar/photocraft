//! UNRUN authentic two-layer Normal source + paid synthetic CURRENT native
//! selection/reference/lock controls. Neither those controls nor translation
//! are described as independently GIMP-authored source evidence.
use super::*;
use photocraft_doc::{Document, LayerContent, Rect, item_identity::ItemRef};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, sync::Arc};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn source() -> (PathBuf, Value, Vec<Vec<u8>>) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("composition-matrix");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "fdcdf1579e7a9e4fa8b4ea3e5f1df722cf7032f24b8c942670ea42e88bad4310");
    let data: Value = serde_json::from_slice(&manifest).unwrap();
    let row = data["cases"].as_array().unwrap().iter().find(|r| r["id"] == "mode-normal").unwrap().clone();
    let bytes = std::fs::read(root.join("mode-normal.xcf")).unwrap();
    assert_eq!(hash(&bytes), "1cfb20a92da5a2180fa9ff8d77156c06e9bcb312922d488a50fdfdaab2e6d5c3");
    assert_eq!(bytes.len(), 5425);
    let state = &row["state_before"];
    assert!(state["assigned_icc_sha256"].is_null());
    assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
    assert_eq!(state["layers"].as_array().unwrap().len(), 2);
    let mut raw = Vec::new();
    for r in state["layers"].as_array().unwrap().iter().rev() {
        assert_eq!(r["type"], "Layer");
        assert!(r["filters"].as_array().unwrap().is_empty());
        assert!(r.get("mask").is_none());
        let blob = &r["native"];
        let mut b = std::fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&b), blob["sha256"].as_str().unwrap());
        assert_eq!(b.len() as u64, blob["bytes"].as_u64().unwrap());
        if cfg!(target_endian = "big") {
            for sample in b.chunks_exact_mut(4) {
                sample.reverse();
            }
        }
        raw.push(b);
    }
    (root, row, raw)
}
fn backed(layer: &photocraft_doc::Layer) -> &photocraft_doc::BackedLayer {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("genuine plain native leaf") };
    b
}
fn native(layer: &photocraft_doc::Layer) -> Vec<u8> {
    let b = backed(layer);
    let r = b.operative_bounds();
    let n = b.format().bytes_per_pixel();
    let mut out = vec![0; r.width() as usize * r.height() as usize * n];
    for coord in r.tiles() {
        let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
        let part = coord.rect().intersect(&r);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * n;
                pin.read_pixel_native_into(x, y, &mut out[at..at + n], &ctl()).unwrap();
            }
        }
    }
    out
}
fn reference_bits(layer: &photocraft_doc::Layer) -> Option<(u64, u64)> {
    layer.effects.reference.map(|(x, y)| (x.to_bits(), y.to_bits()))
}
fn exact(before: &Document, current: &Document, deltas: &[(photocraft_doc::LayerId, i32, i32)]) {
    assert_eq!(before.layers.len(), current.layers.len());
    let mut state = before.xcf_state.as_ref().unwrap().clone();
    for old in &before.layers {
        let new = current.layer(old.id).unwrap();
        let (_, dx, dy) = deltas.iter().find(|(id, _, _)| *id == old.id).copied().unwrap_or((old.id, 0, 0));
        let r = backed(old).operative_bounds();
        let shifted = Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy);
        assert_eq!(backed(new).operative_bounds(), shifted);
        assert_eq!(backed(old).format(), backed(new).format());
        assert_eq!(backed(old).default_bytes(), backed(new).default_bytes());
        assert_eq!(native(old), native(new));
        let expected = old
            .effects
            .reference
            .map(|(x, y)| if (dx, dy) == (0, 0) { (x.to_bits(), y.to_bits()) } else { ((x + f64::from(dx)).to_bits(), (y + f64::from(dy)).to_bits()) });
        assert_eq!(reference_bits(new), expected);
        state.items.iter_mut().find(|i| i.item == ItemRef::Layer(old.id)).unwrap().raster_bounds = Some(shifted);
    }
    assert_eq!(current.xcf_state.as_ref().unwrap(), &state);
    assert_eq!(before.icc_profile, current.icc_profile);
    assert_eq!(before.source_color_context, current.source_color_context);
    assert_eq!(before.sample_encoding, current.sample_encoding);
    assert_eq!(before.working_profile_interpretation, current.working_profile_interpretation);
}
// Prepaid synthetic CURRENT metadata only. The original genuine owner remains
// retained through every copied native/mask/source object and all failure paths.
fn seed(snapshot: &photocraft_engine::DocumentSnapshot, mode: usize) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let original = snapshot.accounted().unwrap().clone();
    let q = original.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = original.document().clone();
    assert_eq!(doc.layers.len(), 2);
    let bg = doc.layers[0].id;
    let fg = doc.layers[1].id;
    for (index, layer) in doc.layers.iter_mut().enumerate() {
        layer.effects.reference = Some((1.25 + index as f64, 2.5 + index as f64));
    }
    if mode == 1 {
        doc.layers[1].locks.position = true;
    }
    if mode == 4 {
        doc.layers[1].visible = false;
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(fg)).unwrap().visible = false;
    }
    if mode != 3 {
        let selection = &mut doc.xcf_state.as_mut().unwrap().selection;
        if mode == 2 {
            selection.selected.clear();
            selection.active = None;
        } else {
            selection.selected = vec![ItemRef::Layer(fg), ItemRef::Layer(bg)];
            selection.active = Some(ItemRef::Layer(if mode == 4 { bg } else { fg }));
        }
    }
    let owner = photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new((original, fee)), q.clone(), &ctl()).unwrap();
    let bytes = photocraft_format::owned_document::save_current_document_archive(owner.document(), Cursor::new(Vec::new()), Default::default(), &q, &ctl())
        .unwrap()
        .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(bytes.as_slice()), bytes.len() as u64, Default::default(), q, &ctl())
        .unwrap()
}
fn imported(bytes: &[u8]) -> photocraft_io::OwnedXcfImport {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    photocraft_io::import_owned_xcf_from_reader_in_quota(
        "mode-normal.xcf",
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &ctl(),
    )
    .unwrap()
}
fn reference(original: &Document, current: &Document, raw: &[Vec<u8>]) -> photocraft_compose::Buffer {
    use photocraft_compose::gimp_composite::{WorkingSpace, encoding};
    let profile = photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Rgb, encoding(current.sample_encoding.unwrap())).unwrap();
    let spaces: Vec<_> = current
        .layers
        .iter()
        .map(|layer| {
            let p = current
                .xcf_state
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find(|i| i.item == ItemRef::Layer(layer.id))
                .unwrap()
                .composition
                .as_ref()
                .unwrap()
                .resolved
                .as_ref()
                .unwrap();
            WorkingSpace::new(&profile, current.sample_encoding.unwrap(), p.composite_space).unwrap()
        })
        .collect();
    let mut px = Vec::new();
    for y in 0..current.size.height as i32 {
        for x in 0..current.size.width as i32 {
            let mut out = [0.; 4];
            for (index, layer) in current.layers.iter().enumerate() {
                if !layer.visible {
                    continue;
                }
                let old = original.layer(layer.id).unwrap();
                let r = backed(layer).operative_bounds();
                let old_r = backed(old).operative_bounds();
                let source = if r.contains(x, y) {
                    let at = ((y - r.y0) as usize * old_r.width() as usize + (x - r.x0) as usize) * 16;
                    let mut p = [0.; 4];
                    for (i, v) in p.iter_mut().enumerate() {
                        *v = photocraft_color::try_read_sample_f64(&raw[index][at..at + 16], photocraft_doc::SampleType::F32, i).unwrap();
                    }
                    p
                } else {
                    [0.; 4]
                };
                let p = current
                    .xcf_state
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .find(|i| i.item == ItemRef::Layer(layer.id))
                    .unwrap()
                    .composition
                    .as_ref()
                    .unwrap()
                    .resolved
                    .as_ref()
                    .unwrap();
                let working = &spaces[index];
                out = working.normal(out, source, f64::from(layer.opacity), p.operator).unwrap();
            }
            px.push(out.map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: current.bounds(), px }
}
fn compare_canvas(original: &Document, current: &photocraft_engine::DocumentSnapshot, raw: &[Vec<u8>]) {
    let expected = display_image(None, &reference(original, current, raw));
    let (image, fee) = backed_snapshot_display_image(current, current.size.width, current.size.height, None, &Interrupt::NONE).unwrap();
    assert_eq!(image.pixels, expected.pixels);
    drop((image, fee));
}
#[test]
#[ignore = "UNRUN:authentic two-layer plain source; paid CURRENT reversed multi-selection/effect refs, explicit override, Move/key/history/noopredo/native and atomic lock/overflow"]
fn bulk_plain_ui_moves_exact_selected_native_leaves_as_one_current_transaction() {
    let (root, _, raw) = source();
    let bytes = std::fs::read(root.join("mode-normal.xcf")).unwrap();
    let carrier = imported(&bytes);
    assert!(matches!(&carrier, photocraft_io::OwnedXcfImport::Plain(_)));
    let mut source_app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_xcf_open::adopt_bytes(&mut source_app, "mode-normal.xcf", &carrier, &ctl()).unwrap();
    let authored = source_app.session.active().unwrap().doc.clone();
    assert_eq!(authored.layers.len(), 2);
    for (i, l) in authored.layers.iter().enumerate() {
        assert!(l.mask.is_none());
        assert_eq!(native(l), raw[i]);
    }
    for mode in [0, 1, 2, 4] {
        let archive = seed(&authored, mode);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "Bulk current.pcraft", archive, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let bg = before.layers[0].id;
        let fg = before.layers[1].id;
        // Empty canonical selection is normalized by native opener initialization;
        // Move consumes this genuine paid view, never invents an active fallback.
        assert_eq!(app.session.active().unwrap().selection_ids(), if mode == 2 { vec![fg] } else { vec![fg, bg] });
        let history = app.session.active().unwrap().history.clone();
        let journal = app.session.journal.clone();
        let revision = app.session.active().unwrap().revision;
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("bulk move denied".into())));
        assert!(app.run("layer.translate", json!({"dx":1})).unwrap_err().contains("bulk move denied"));
        app.session.authorize = None;
        for params in [json!({"dx":2147483647,"dy":1}), json!({"dx":1,"target":"mask"})] {
            assert!(app.run("layer.translate", params).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(app.session.journal, journal);
            assert_eq!(app.session.active().unwrap().revision, revision);
        }
        if mode == 1 {
            assert!(app.run("layer.translate", json!({"dx":1})).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(app.session.journal, journal);
            // Explicit name overrides multi-selection and the locked peer.
            app.run("layer.translate", json!({"layer":before.layers[0].name,"dx":1})).unwrap();
            exact(&before, &app.session.active().unwrap().doc, &[(bg, 1, 0)]);
            continue;
        }
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.translate", json!({"dx":1.49,"dy":-0.5,"coalesce":"bulk"})).unwrap();
        let first = app.session.active().unwrap().doc.clone();
        let all = if mode == 2 { vec![(fg, 1, -1)] } else { vec![(bg, 1, -1), (fg, 1, -1)] };
        exact(&before, &first, &all);
        if mode == 4 {
            assert!(app.run("layer.translate", json!({"layer":fg.0,"dy":1})).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&first));
            app.run("layer.translate", json!({"dy":1,"coalesce":"bulk"})).unwrap();
        } else {
            app.run("layer.translate", json!({"layer":fg.0,"dy":1,"coalesce":"bulk"})).unwrap();
        }
        app.run("layer.setProps", json!({"layer":fg.0,"opacity":0.5,"coalesce":"bulk"})).unwrap();
        let moved = app.session.active().unwrap().doc.clone();
        let deltas = if mode == 2 {
            vec![(fg, 1, 0)]
        } else if mode == 4 {
            vec![(bg, 1, 0), (fg, 1, 0)]
        } else {
            vec![(bg, 1, -1), (fg, 1, 0)]
        };
        exact(&before, &moved, &deltas);
        compare_canvas(&authored, &moved, &raw);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let history = app.session.active().unwrap().history.clone();
        app.run("layer.translate", json!({"layer":"absent","dx":0.49,"coalesce":"other"})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&moved));
        let q = moved.quota().unwrap();
        let saved = photocraft_format::owned_document::save_current_document_archive(&moved, Cursor::new(Vec::new()), Default::default(), q, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Bulk saved.pcraft", archive, &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        exact(&before, current, &deltas);
        assert_eq!(current.layers[1].opacity.to_bits(), 0.5f32.to_bits());
        compare_canvas(&authored, current, &raw);
    }
    assert_eq!(std::fs::read(root.join("mode-normal.xcf")).unwrap(), bytes);
}
#[test]
#[ignore = "UNRUN:assigned finite scalar effects.reference through genuine CURRENT native adoption, linked Move and native persistence; not GIMP-authored scalar metadata"]
fn assigned_reference_scalar_remains_exact_through_current_native_move() {
    let (root, row) = super::analytic_geometry_tests::cases()
        .into_iter()
        .find(|(root, row)| root.ends_with("holdout-evidence/left-top") && row["id"] == "rgb-f32-op1-source")
        .unwrap();
    let bytes = std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
    let carrier = imported(&bytes);
    assert!(matches!(&carrier, photocraft_io::OwnedXcfImport::AnalyticGeometry(_)));
    let mut original = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_xcf_open::adopt_bytes(&mut original, "assigned.xcf", &carrier, &ctl()).unwrap();
    let authored = original.session.active().unwrap().doc.clone();
    let archive = seed(&authored, 3);
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_native_open::adopt_bytes(&mut app, "Scalar.pcraft", archive, &ctl()).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let fg = before.layers[1].id;
    let bg_ref = reference_bits(&before.layers[0]);
    let r = backed(&before.layers[1]).operative_bounds();
    let words = native(&before.layers[1]);
    let mask = before.layers[1].mask.as_ref().unwrap().surface.try_read_interleaved(r).unwrap();
    app.run("layer.translate", json!({"layer":fg.0,"dx":2,"dy":-1})).unwrap();
    let moved = app.session.active().unwrap().doc.clone();
    assert_eq!(reference_bits(&moved.layers[1]), Some((4.25f64.to_bits(), 2.5f64.to_bits())));
    assert_eq!(reference_bits(&moved.layers[0]), bg_ref);
    assert_eq!(native(&moved.layers[1]), words);
    assert_eq!(moved.layers[1].mask.as_ref().unwrap().surface.try_read_interleaved(r.translate(2, -1)).unwrap(), mask);
    assert_eq!(moved.icc_profile, before.icc_profile);
    assert_eq!(moved.source_color_context, before.source_color_context);
    let q = moved.quota().unwrap();
    let saved =
        photocraft_format::owned_document::save_current_document_archive(&moved, Cursor::new(Vec::new()), Default::default(), q, &ctl()).unwrap().into_inner();
    let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut Cursor::new(saved.as_slice()),
        saved.len() as u64,
        Default::default(),
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_native_open::adopt_bytes(&mut reopened, "Scalar saved.pcraft", archive, &ctl()).unwrap();
    assert_eq!(reference_bits(&reopened.session.active().unwrap().doc.layers[1]), reference_bits(&moved.layers[1]));
    assert_eq!(reopened.session.active().unwrap().doc.xcf_state, moved.xcf_state);
    assert_eq!(std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap(), bytes);
}
