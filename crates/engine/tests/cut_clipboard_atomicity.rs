//! Cut publishes clipboard pixels only after its document transaction succeeds.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size};
use photocraft_engine::{Session, edit_cmds::Clip};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};
use std::sync::Arc;
fn ordinary(empty: bool) -> Document {
    let mut d = Document::new("Cut source", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut s = Surface::new(d.pixel_format());
    if !empty {
        s.try_write_interleaved(d.bounds(), &[55, 70, 120, 255].repeat(4)).unwrap();
    }
    d.layers.push(Layer::new("Pixels", LayerContent::Raster(s)));
    d
}
fn session(doc: Document, populated: bool) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    s.edit("Seed redo", |d, _| {
        d.name = "Redo seed".into();
        Ok(())
    })
    .unwrap();
    assert!(s.undo());
    if populated {
        let area = Rect::new(0, 0, 1, 1);
        let v = Surface::try_from_interleaved(photocraft_color::PixelFormat::RGBA8, area, &[15, 20, 245, 255]).unwrap();
        s.clipboard = Some(Clip::from_external_srgb(v, area).unwrap());
    }
    s
}
fn clip(s: &Session) -> Value {
    s.clipboard.as_ref().map_or(Value::Null,|c|json!({"bounds":c.bounds,"format":c.surface.format(),"pixels":c.surface.try_read_interleaved(c.bounds).unwrap(),"profile":c.color.icc_profile.as_ref().map(|v|v.as_slice()),"mode":c.color.mode,"encoding":c.color.sample_encoding,"interpretation":c.color.interpretation,"composition":c.composition}))
}
fn refusal(mut s: Session, needle: &str, live: bool) {
    let expected_redo = {
        assert!(s.redo());
        let d = s.active().unwrap().doc.clone();
        assert!(s.undo());
        d
    };
    let state = s.active().unwrap();
    let before = state.doc.clone();
    let revision = state.revision;
    let saved = state.saved_revision;
    let selected = state.selected_layers.clone();
    let active = state.active_layer;
    let anchor = state.layer_anchor;
    let past: Vec<_> = state.history.entry_labels().map(str::to_string).collect();
    let redo: Vec<_> = state.history.redo_labels().map(str::to_string).collect();
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    let oldclip = clip(&s);
    let oldtile = s.clipboard.as_ref().and_then(|c| c.surface.tiles().next().map(|(_, t)| t.clone()));
    let oldicc = s.clipboard.as_ref().and_then(|c| c.color.icc_profile.clone());
    let projection = if live { Some(photocraft_compose::gimp_render::try_render(&before, before.bounds(), &mut |_| Ok(())).unwrap()) } else { None };
    let e = s.execute("edit.cut", json!({})).unwrap_err();
    assert!(e.to_string().contains(needle), "{e}");
    let now = s.active().unwrap();
    assert_eq!(now.doc, before);
    assert_eq!((now.revision, now.saved_revision, now.active_layer, now.layer_anchor), (revision, saved, active, anchor));
    assert_eq!(now.selected_layers, selected);
    assert_eq!(now.history.entry_labels().collect::<Vec<_>>(), past);
    assert_eq!(now.history.redo_labels().collect::<Vec<_>>(), redo);
    assert_eq!(photocraft_format::save_to_bytes(&now.doc, &Default::default()).unwrap(), native);
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *before);
    assert_eq!(clip(&s), oldclip);
    if let Some(old) = oldtile {
        let current = s.clipboard.as_ref().unwrap().surface.tiles().next().unwrap().1;
        assert!(Arc::ptr_eq(&old, current));
    }
    if let Some(old) = oldicc {
        assert!(Arc::ptr_eq(&old, s.clipboard.as_ref().unwrap().color.icc_profile.as_ref().unwrap()));
    }
    if let Some(old) = projection {
        assert_eq!(photocraft_compose::gimp_render::try_render(&now.doc, now.doc.bounds(), &mut |_| Ok(())).unwrap(), old);
    }
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc, expected_redo);
}
#[test]
fn successful_cut_installs_clipboard_after_undoable_pixel_edit() {
    for populated in [false, true] {
        let mut s = session(ordinary(false), populated);
        let before = s.active().unwrap().doc.clone();
        let expected = before.layers[0].surface().unwrap().try_read_interleaved(before.bounds()).unwrap();
        let result = s.execute("edit.cut", json!({})).unwrap();
        assert_eq!(result, json!({"bounds":[0,0,2,2]}));
        assert_eq!(s.clipboard.as_ref().unwrap().surface.try_read_interleaved(before.bounds()).unwrap(), expected);
        assert!(!s.active().unwrap().history.can_redo());
        let after = s.active().unwrap().doc.clone();
        assert!(after.layers[0].surface().unwrap().content_bounds().is_empty());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc, after);
    }
}
#[test]
fn locked_cut_preserves_populated_or_absent_clipboard() {
    for populated in [false, true] {
        let mut d = ordinary(false);
        d.layers[0].locks.pixels = true;
        refusal(session(d, populated), "locked", false);
    }
}
#[test]
fn empty_copy_preparation_preserves_populated_or_absent_clipboard() {
    for populated in [false, true] {
        refusal(session(ordinary(true), populated), "selected area is empty", false);
    }
}
#[test]
fn invalid_selection_copy_preparation_preserves_populated_or_absent_clipboard() {
    for populated in [false, true] {
        let mut d = ordinary(false);
        let fmt = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let bytes: Vec<_> = [f64::NAN; 4].into_iter().flat_map(f64::to_ne_bytes).collect();
        d.selection = Some(Surface::try_from_interleaved(fmt, d.bounds(), &bytes).unwrap());
        refusal(session(d, populated), "invalid selection coverage", false);
    }
}
#[cfg(feature = "corpus")]
fn live_source() -> Document {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-stem-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "ba7b36146b20976846cd334a16f2abda9407894970697fd2d9f6aba56c127bef");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (n, h) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(root.join(&n)).unwrap())), h, "{n}")
    }
    let d = photocraft_io::import("text.xcf", &std::fs::read(root.join("simple/source.xcf")).unwrap()).unwrap().document;
    d.validate_xcf_state().unwrap();
    d
}
#[cfg(feature = "corpus")]
#[test]
fn live_source_late_cut_refusal_preserves_populated_clipboard_native_projection_and_real_redo() {
    refusal(session(live_source(), true), "cannot preserve live imported rendering", true);
}
#[cfg(feature = "corpus")]
#[test]
fn live_source_late_cut_refusal_preserves_absent_clipboard_native_projection_and_real_redo() {
    refusal(session(live_source(), false), "cannot preserve live imported rendering", true);
}
