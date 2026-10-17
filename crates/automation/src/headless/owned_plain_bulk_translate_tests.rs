//! UNRUN authentic two-layer plain source with paid synthetic CURRENT native
//! multi-selection/reference/lock seeds. External files are harness captures,
//! never GIMP bulk-Move evidence or forged quota/storage proofs.
use super::*;
use photocraft_doc::{Document, LayerContent, Rect, item_identity::ItemRef};
use photocraft_raster::{
    Interrupt,
    backing::{ReadControl, quota::QuotaKind},
};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
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
            for sample in b.as_chunks_mut::<4>().0 {
                sample.reverse();
            }
        }
        raw.push(b);
    }
    (root, row, raw)
}
fn backed(layer: &photocraft_doc::Layer) -> &photocraft_doc::BackedLayer {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("genuine plain native leaf") };
    assert!(layer.mask.is_none());
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
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-bulk-move-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn write_seed(archive: &photocraft_format::owned_document::OwnedDocumentArchive, path: &std::path::Path) {
    // Setup-only exclusive harness file; production guarded native writer is
    // exercised below by save_owned_native_active on the installed real owner.
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
    photocraft_format::owned_document::save_current_document_archive(archive.document(), file, Default::default(), archive.quota(), &ctl()).unwrap();
}
#[test]
#[ignore = "UNRUN:authentic two-layer plain source + paid CURRENT reversed/fallback/locked selections, exact bulk Move/reference metadata, transport/history/native continuation"]
fn plain_bulk_headless_is_synchronous_atomic_and_keeps_source_native_roles() {
    let (root, _, raw) = source();
    let original = std::fs::read(root.join("mode-normal.xcf")).unwrap();
    let dir = Directory::new();
    let source_path = dir.0.join("original.xcf");
    std::fs::write(&source_path, &original).unwrap();
    let mut source_h = Headless::trusted_local();
    drop(source_h.open_owned_xcf_keep_current(&source_path, &ctl()).unwrap());
    let authored = source_h.session.active().unwrap().doc.clone();
    assert_eq!(authored.layers.len(), 2);
    for (i, layer) in authored.layers.iter().enumerate() {
        assert!(layer.mask.is_none());
        assert_eq!(native(layer), raw[i]);
    }
    assert!(source_h.session.active().unwrap().source_read_only);
    assert!(source_h.save_owned_native_active(Some(&source_path), Some("pcraft"), &ctl()).is_err());
    for mode in [0, 1, 2, 4] {
        let archive = seed(&authored, mode);
        let path = dir.0.join(format!("Seed{mode}.pcraft"));
        write_seed(&archive, &path);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let bg = before.layers[0].id;
        let fg = before.layers[1].id;
        let source = h.session.active().unwrap().source.clone();
        assert!(!h.session.active().unwrap().source_read_only);
        assert_eq!(
            h.session.active().unwrap().selection_ids(),
            if mode == 2 { vec![fg] } else { vec![fg, bg] },
            "native opener initializes a genuine paid view; Move does not invent activeNone fallback"
        );
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.clone();
        let revision = h.session.active().unwrap().revision;
        assert!(matches!(h.command_start("layer.translate",json!({"dx":1}),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous")));
        assert!(!h.session.has_jobs());
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal, journal);
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("bulk denied".into())));
        assert!(h.command_start("layer.translate", json!({"dx":1}), true).unwrap_err().to_string().contains("bulk denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        for params in [json!({"dx":2147483647,"dy":1}), json!({"dx":1,"target":"mask"})] {
            assert!(h.command_run("layer.translate", params).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.journal, journal);
            assert_eq!(h.session.active().unwrap().revision, revision);
        }
        if mode == 1 {
            assert!(h.command_run("layer.translate", json!({"dx":1})).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.journal, journal);
            h.command_run("layer.translate", json!({"layer":before.layers[0].name,"dx":1})).unwrap();
            exact(&before, &h.session.active().unwrap().doc, &[(bg, 1, 0)]);
            continue;
        }
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(h.command_start("layer.translate", json!({"dx":1.49,"dy":-0.5,"coalesce":"bulk"}), true).unwrap(), Value::Null);
        let first = h.session.active().unwrap().doc.clone();
        let all = if mode == 2 { vec![(fg, 1, -1)] } else { vec![(bg, 1, -1), (fg, 1, -1)] };
        exact(&before, &first, &all);
        if mode == 4 {
            assert!(h.command_run("layer.translate", json!({"layer":fg.0,"dy":1})).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&first));
            h.command_run("layer.translate", json!({"dy":1,"coalesce":"bulk"})).unwrap();
        } else {
            h.command_run("layer.translate", json!({"layer":fg.0,"dy":1,"coalesce":"bulk"})).unwrap();
        }
        h.command_run("layer.setProps", json!({"layer":fg.0,"opacity":0.5,"coalesce":"bulk"})).unwrap();
        let moved = h.session.active().unwrap().doc.clone();
        let deltas = if mode == 2 {
            vec![(fg, 1, 0)]
        } else if mode == 4 {
            vec![(bg, 1, 0), (fg, 1, 0)]
        } else {
            vec![(bg, 1, -1), (fg, 1, 0)]
        };
        exact(&before, &moved, &deltas);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        h.command_run("layer.translate", json!({"layer":"absent","dx":0.49,"coalesce":"other"})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&moved));
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 2);
        assert_eq!(query.value()["filters"]["total"], 0);
        drop(query);
        let output = dir.0.join(format!("Moved{mode}.pcraft"));
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let saved_name = h.session.active().unwrap().doc.name.clone();
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let state = reopened.session.active().unwrap();
        assert!(!state.source_read_only);
        assert_eq!(state.doc.name, saved_name);
        exact(&before, &state.doc, &deltas);
        assert_eq!(state.doc.layers[1].opacity.to_bits(), 0.5f32.to_bits());
        // Explicit continuation overrides the still-retained multi-selection.
        let continuation = if mode == 4 { bg } else { fg };
        reopened.command_run("layer.translate", json!({"layer":continuation.0,"dx":-1})).unwrap();
        let mut final_deltas = deltas.clone();
        final_deltas.iter_mut().find(|(id, _, _)| *id == continuation).unwrap().1 -= 1;
        exact(&before, &reopened.session.active().unwrap().doc, &final_deltas);
    }
    assert!(source_h.session.active().unwrap().doc.shares_state(&authored));
    assert_eq!(std::fs::read(&source_path).unwrap(), original);
    assert_eq!(std::fs::read(root.join("mode-normal.xcf")).unwrap(), original);
}
