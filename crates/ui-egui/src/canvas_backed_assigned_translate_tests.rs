//! UNRUN authored assigned Normal source packets -> synthetic CURRENT linked
//! Move controls. Retained canonical linkage is only a CURRENT policy input;
//! no independent GIMP linkage or translation eligibility is inferred.
use super::analytic_geometry_tests::{cases, ctl, operators, profiles};
use super::*;
use photocraft_doc::{Document, LayerContent, Rect, item_identity::ItemRef};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Cursor;
fn backed(layer: &photocraft_doc::Layer) -> &photocraft_doc::BackedLayer {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("actual native leaf") };
    b
}
fn native(layer: &photocraft_doc::Layer) -> Vec<u8> {
    let b = backed(layer);
    let r = b.operative_bounds();
    let n = b.format().bytes_per_pixel();
    let mut out = vec![0; r.width() as usize * r.height() as usize * n];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * n;
            b.checked_view().read_logical_pixel_native_into(x, y, &mut out[at..at + n], &ctl()).unwrap();
        }
    }
    out
}
fn foreground(doc: &Document) -> photocraft_doc::LayerId {
    assert_eq!(doc.layers.len(), 2);
    doc.layers.iter().find(|l| l.mask.is_some()).unwrap().id
}
fn retained(before: &Document, current: &Document, dx: i32, dy: i32) {
    let id = foreground(before);
    let old = before.layer(id).unwrap();
    let new = current.layer(id).unwrap();
    let a = backed(old);
    let b = backed(new);
    let r = a.operative_bounds();
    let moved = Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy);
    assert_eq!(b.operative_bounds(), moved);
    assert_eq!(b.format(), a.format());
    assert_eq!(b.default_bytes(), a.default_bytes());
    assert_eq!(native(old), native(new));
    let ma = old.mask.as_ref().unwrap();
    let mb = new.mask.as_ref().unwrap();
    assert!(ma.linked && mb.linked);
    assert_eq!(ma.enabled, mb.enabled);
    assert_eq!(ma.surface.format(), mb.surface.format());
    assert_eq!(ma.surface.default_bytes(), mb.surface.default_bytes());
    assert_eq!(ma.surface.try_read_interleaved(r).unwrap(), mb.surface.try_read_interleaved(moved).unwrap());
    assert_eq!((ma.density.to_bits(), ma.feather.to_bits()), (mb.density.to_bits(), mb.feather.to_bits()));
    for layer in &before.layers {
        let after = current.layer(layer.id).unwrap();
        assert_eq!(
            (&layer.name, layer.visible, layer.opacity.to_bits(), layer.fill_opacity.to_bits()),
            (&after.name, after.visible, after.opacity.to_bits(), after.fill_opacity.to_bits())
        );
        if layer.id != id {
            assert_eq!(backed(layer).descriptor(), backed(after).descriptor());
            assert_eq!(native(layer), native(after));
        }
    }
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.sample_encoding, before.sample_encoding);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let mut state = before.xcf_state.as_ref().unwrap().clone();
    for item in &mut state.items {
        if item.item == ItemRef::Layer(id) {
            item.raster_bounds = Some(moved);
        }
        if let Some(mask) = item.mask.as_mut().filter(|m| m.owner == ItemRef::Layer(id)) {
            item.raster_bounds = Some(moved);
            mask.bounds = moved;
            mask.offset = [moved.x0, moved.y0];
        }
    }
    assert_eq!(current.xcf_state.as_ref().unwrap(), &state);
}
fn literals(root: &std::path::Path, row: &Value, doc: &Document) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    for layer in row["layers"].as_array().unwrap() {
        for blob in std::iter::once(&layer["native"]).chain(if layer["mask"].is_null() { None } else { Some(&layer["mask"]["native"]) }) {
            let mut bytes = std::fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
            let hash = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
            assert_eq!(hash, blob["sha256"].as_str().unwrap());
            assert_eq!(bytes.len() as u64, blob["bytes"].as_u64().unwrap());
            if cfg!(target_endian = "big") {
                for sample in bytes.chunks_exact_mut(doc.depth.bytes()) {
                    sample.reverse();
                }
            }
            result.push(bytes);
        }
    }
    assert_eq!(result.len(), 3);
    result
}
// Only existing Normal math is reused here. The translated sample lookup is
// independent literal capture indexing, not the new translated view/provider.
fn reference(before: &Document, raw: &[Vec<u8>], dx: i32, dy: i32) -> photocraft_compose::Buffer {
    use photocraft_compose::gimp_composite::{WorkingSpace, encoding};
    fn sample(raw: &[u8], format: photocraft_doc::PixelFormat, r: Rect, x: i32, y: i32) -> [f64; 4] {
        if !r.contains(x, y) {
            return [0.; 4];
        }
        let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * format.bytes_per_pixel();
        let bytes = &raw[at..at + format.bytes_per_pixel()];
        let mut p = [0.; 4];
        for (channel, v) in p.iter_mut().enumerate().take(format.channels()) {
            *v = photocraft_color::try_read_sample_f64(bytes, format.sample, channel).unwrap();
        }
        if format.mode == photocraft_doc::ColorMode::Grayscale && format.alpha { [p[0], p[0], p[0], p[1]] } else { p }
    }
    let id = foreground(before);
    let fg = before.layer(id).unwrap();
    let bg = before.layers.iter().find(|l| l.id != id).unwrap();
    let fr = backed(fg).operative_bounds();
    let br = backed(bg).operative_bounds();
    let format = before.pixel_format();
    let mask = fg.mask.as_ref().unwrap();
    let original = before.source_color_context.as_ref().unwrap().original_icc.as_deref().unwrap();
    let cs = if before.mode == photocraft_doc::ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray };
    let profile = photocraft_cms::xcf_color::effective_profile(Some(original), cs, encoding(before.sample_encoding.unwrap())).unwrap();
    let policy = |id| {
        before.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap().composition.as_ref().unwrap().resolved.as_ref().unwrap()
    };
    let fp = policy(fg.id);
    let bp = policy(bg.id);
    let fw = WorkingSpace::new(&profile, before.sample_encoding.unwrap(), fp.composite_space).unwrap();
    let bw = WorkingSpace::new(&profile, before.sample_encoding.unwrap(), bp.composite_space).unwrap();
    let mf = mask.surface.format();
    let mut px = Vec::new();
    for y in 0..before.size.height as i32 {
        for x in 0..before.size.width as i32 {
            let back = bw.normal([0.; 4], sample(&raw[2], format, br, x, y), f64::from(bg.opacity), bp.operator).unwrap();
            let (sx, sy) = (x - dx, y - dy);
            let coverage = if mask.enabled { sample(&raw[1], mf, fr, sx, sy)[0] } else { 1. };
            let out = fw.normal(back, sample(&raw[0], format, fr, sx, sy), f64::from(fg.opacity) * coverage, fp.operator).unwrap();
            px.push(out.map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: before.bounds(), px }
}
#[test]
#[ignore = "UNRUN:64 representative actual source domains from256 pinned packets; enabled/disabled linked CURRENT Move, raw mask/native/default/profile, canvas/history and native continuation"]
fn linked_assigned_move_ui_preserves_literal_samples_and_current_composition() {
    let selected: Vec<_> = cases().into_iter().filter(|(root, row)| root.ends_with("left-top") && row["id"].as_str().unwrap().ends_with("-source")).collect();
    assert_eq!(selected.len(), 64);
    for (root, row) in selected {
        for disable in [false, true] {
            let name = row["source_file"].as_str().unwrap();
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
            assert!(matches!(&imported, photocraft_io::OwnedXcfImport::AnalyticGeometry(_)));
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
            let original = app.session.active().unwrap().doc.clone();
            let id = foreground(&original);
            assert!(original.layer(id).unwrap().mask.as_ref().unwrap().linked, "actual retained CURRENT link flag, not an independent GIMP linkage claim");
            assert!(original.layer(id).unwrap().mask.as_ref().unwrap().enabled);
            profiles(&original, &row);
            operators(&original, &row);
            let raw = literals(&root, &row, &original);
            assert_eq!(native(original.layer(id).unwrap()), raw[0]);
            let bg = original.layers.iter().find(|l| l.id != id).unwrap();
            assert_eq!(native(bg), raw[2]);
            assert_eq!(
                original
                    .layer(id)
                    .unwrap()
                    .mask
                    .as_ref()
                    .unwrap()
                    .surface
                    .try_read_interleaved(backed(original.layer(id).unwrap()).operative_bounds())
                    .unwrap(),
                raw[1]
            );
            let mask_id = original
                .xcf_state
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find_map(|i| match (i.item, i.mask.as_ref()) {
                    (ItemRef::Mask(mid), Some(m)) if m.owner == ItemRef::Layer(id) => Some(mid),
                    _ => None,
                })
                .unwrap();
            if disable {
                let journal = app.session.journal.len();
                app.run("xcf.mask.enabled.set", json!({"maskId":mask_id.0.to_string(),"enabled":false})).unwrap();
                assert_eq!(app.session.journal.len(), journal);
            }
            let before = app.session.active().unwrap().doc.clone();
            let past = app.session.active().unwrap().history.past_len();
            app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("assigned move denied".into())));
            assert!(app.run("layer.translate", json!({"layer":id.0,"dx":1})).unwrap_err().contains("assigned move denied"));
            app.session.authorize = None;
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            app.run("layer.translate", json!({"layer":before.layer(id).unwrap().name,"dx":1.49,"dy":-0.5,"coalesce":"assigned-move"})).unwrap();
            retained(&before, &app.session.active().unwrap().doc, 1, -1);
            // A linked mask target follows the whole layer, including when disabled.
            app.run("layer.translate", json!({"layer":id.0,"target":"mask","dy":1,"coalesce":"assigned-move"})).unwrap();
            let moved = app.session.active().unwrap().doc.clone();
            retained(&before, &moved, 1, 0);
            profiles(&moved, &row);
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
            app.run("layer.translate", json!({"layer":"absent","dx":0.49,"dy":0.49,"coalesce":"assigned-move"})).unwrap();
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(app.session.active().unwrap().revision, revision);
            app.run("edit.redo", json!({})).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&moved));
            let renamed = moved.accounted().unwrap().try_with_document_name("Assigned moved native", &ctl()).unwrap();
            let saved = photocraft_format::owned_document::save_current_document_archive(
                renamed.document(),
                Cursor::new(Vec::new()),
                Default::default(),
                &quota,
                &ctl(),
            )
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
            crate::owned_native_open::adopt_bytes(&mut reopened, "AssignedMoved.pcraft", archive, &ctl()).unwrap();
            let current = reopened.session.active().unwrap();
            assert!(!current.source_read_only);
            assert!(current.path.is_none());
            assert_eq!(current.doc.name, "Assigned moved native");
            retained(&before, &current.doc, 1, 0);
            profiles(&current.doc, &row);
            let (image, fee) = backed_snapshot_display_image(&current.doc, current.doc.size.width, current.doc.size.height, None, &Interrupt::NONE).unwrap();
            assert_eq!(image.pixels, expected.pixels);
            drop((image, fee));
            reopened.run("layer.translate", json!({"layer":id.0,"dx":-1})).unwrap();
            retained(&before, &reopened.session.active().unwrap().doc, 0, 0);
            assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
        }
    }
}
