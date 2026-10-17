//! Native public file.open paired type-cache preparation and invalid geometry preserve existing work.
#![cfg(not(target_arch = "wasm32"))]
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, PhysicalResolution, SampleType, Size, TextLayer};
use photocraft_engine::{
    Session,
    edit_cmds::Clip,
    jobs::{OpenSource, Started},
};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};
use std::sync::Arc;

fn source(axes: Option<[f64; 2]>, cache: bool, raster: bool) -> Document {
    let mut d = Document::new("Missing type cache public import", Size::new(32, 24), ColorMode::Rgb, SampleType::U8);
    d.resolution_dpi = 72.0;
    if let Some([x, y]) = axes {
        d.set_physical_resolution(PhysicalResolution { x_dpi: x, y_dpi: y, unit: photocraft_doc::PhysicalUnit::Inch }).unwrap();
    }
    let mut s = photocraft_raster::Surface::new(d.pixel_format());
    if cache || raster {
        s.try_write_interleaved(Rect::new(1, 1, 2, 2), &[35, 110, 225, 255]).unwrap();
    }
    if raster {
        d.layers.push(Layer::new("Ordinary pixels", LayerContent::Raster(s)));
    } else {
        let mut t = TextLayer { text: "Physical import".into(), ..Default::default() };
        let mut st = t.char_runs()[0].style.clone();
        st.size_pt = 12.0;
        t.runs = vec![photocraft_doc::text::TextRun { len: t.text.len(), style: st }];
        t.sync_summary();
        t.psd_raw = Some(Arc::new(photocraft_text::psd::build_tysh(&t, d.resolution_dpi, None)));
        t.cache = Some(s);
        d.layers.push(Layer::new("Retained editable type", LayerContent::Text(t)));
    }
    d
}
fn encode_fixture(d: &Document, ext: &str) -> Vec<u8> {
    let native = photocraft_format::save_to_bytes(d, &Default::default()).unwrap();
    let bytes = photocraft_io::export(d, ext, &Default::default()).unwrap().bytes;
    assert_eq!(photocraft_format::save_to_bytes(d, &Default::default()).unwrap(), native, "export mutated model");
    bytes
}

fn fixture(axes: [f64; 2], cached: bool, ext: &str) -> Vec<u8> {
    encode_fixture(&source(Some(axes), cached, false), ext)
}
// Build the valid tiny file first, then change only its typed font geometry.
// This avoids asking the exporter to render an intentionally unrepresentable model.
fn unrepresentable_fixture(axes: [f64; 2], ext: &str) -> Vec<u8> {
    let doc = source(Some(axes), false, false);
    let LayerContent::Text(original) = &doc.layers[0].content else { panic!("text fixture") };
    let mut text = original.clone();
    text.runs[0].style.size_pt = 1_000_000_000.0;
    text.sync_summary();
    text.cache = None;
    assert!(text.runs[0].style.size_pt.is_finite());
    let tysh = photocraft_text::psd::build_tysh(&text, doc.resolution_dpi, None);
    assert!(photocraft_text::psd::text_layer_from_tysh_with_resolution(&tysh, &doc.effective_resolution()).is_err());
    let mut file = photocraft_psd::PsdFile::from_bytes(&encode_fixture(&doc, ext)).unwrap();
    let block = file.layers_mut()[0].blocks.iter_mut().find(|block| block.key == *b"TySh").unwrap();
    block.data = tysh;
    block.padding = None;
    let bytes = file.to_bytes().unwrap();
    photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    bytes
}

fn existing() -> Session {
    let mut d = Document::new("Existing dirty work", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    let mut surface = Surface::new(d.pixel_format());
    let bytes = [0.25f32, -0.0, 0.75, 1.0, 0.5, 0.25, 0.125, 0.5].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
    surface.try_write_interleaved(d.bounds(), &bytes).unwrap();
    d.layers.push(Layer::new("Existing pixels", LayerContent::Raster(surface)));
    let mut s = Session::new();
    s.add_document(d, None).unwrap();
    s.edit("Committed dirty change", |d, _| {
        d.layers[0].name.push_str(" dirty");
        Ok(())
    })
    .unwrap();
    s.edit("Real redo seed", |d, _| {
        d.layers[0].name.push_str(" redone");
        Ok(())
    })
    .unwrap();
    assert!(s.undo());
    let area = Rect::new(0, 0, 1, 1);
    let v = Surface::try_from_interleaved(photocraft_doc::PixelFormat::RGBA8, area, &[15, 20, 245, 255]).unwrap();
    s.clipboard = Some(Clip::from_external_srgb(v, area).unwrap());
    s
}
fn clipboard(s: &Session) -> Value {
    let c = s.clipboard.as_ref().unwrap();
    json!({"bounds":c.bounds,"format":c.surface.format(),"pixels":c.surface.try_read_interleaved(c.bounds).unwrap(),"profile":c.color.icc_profile.as_ref().map(|v|v.as_slice()),"mode":c.color.mode,"encoding":c.color.sample_encoding,"interpretation":c.color.interpretation,"composition":c.composition})
}
fn open(s: &mut Session, name: &str, bytes: &[u8], background: bool) -> photocraft_engine::Result<Value> {
    if !background {
        return photocraft_engine::file_cmds::open_bytes_as(s, name, bytes, None, None);
    }
    match s.start_open(name, OpenSource::Bytes(Arc::new(bytes.to_vec())))? {
        Started::Job(id) => s.wait_job(id),
        Started::Done(value) => Ok(value),
    }
}
fn check(ext: &str, background: bool, axes: [f64; 2], cached: bool, refuse: bool) {
    let bytes = if refuse { unrepresentable_fixture(axes, ext) } else { fixture(axes, cached, ext) };
    let retained = bytes.clone();
    let mut s = existing();
    assert_eq!(s.documents().len(), 1);
    assert!(s.active().unwrap().history.can_redo());
    assert!(s.redo());
    let expected_redo = s.active().unwrap().doc.clone();
    assert!(s.undo());
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let active_index = s.active_index();
    let meta = (st.revision, st.saved_revision, st.active_layer, st.layer_anchor, st.source.clone(), st.path.clone());
    assert_ne!(st.revision, st.saved_revision, "existing work must be dirty");
    let selected = st.selected_layers.clone();
    let past = st.history.entry_labels().map(str::to_string).collect::<Vec<_>>();
    let redo = st.history.redo_labels().map(str::to_string).collect::<Vec<_>>();
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    let oldclip = clipboard(&s);
    let oldjournal = s.journal.clone();
    let tile = s.clipboard.as_ref().unwrap().surface.tiles().next().unwrap().1.clone();
    let profile = s.clipboard.as_ref().unwrap().color.icc_profile.clone();
    let result = open(&mut s, &format!("own-small.{ext}"), &bytes, background);
    if refuse {
        let error = result.expect_err("finite unrepresentable type geometry must refuse before publication");
        assert!(error.to_string().contains("unrepresentable geometry") && error.to_string().contains("text"), "{error}");
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.active_index(), active_index);
    } else {
        result.unwrap();
        assert_eq!(s.documents().len(), 2);
        assert_eq!(s.active_index(), Some(1));
        let opened = &s.active().unwrap().doc;
        let r = opened.effective_resolution();
        assert_eq!([r.x_dpi, r.y_dpi], axes);
        let LayerContent::Text(text) = &opened.layers[0].content else { panic!("text retained") };
        assert!(text.cache.as_ref().is_some_and(|s| !s.content_bounds().is_empty()));
        if !cached {
            // Independent public paired-resolution rendering of the decoded model,
            // never an expected surface copied from the actual prepared cache.
            let mut engine = photocraft_text::TextEngine::new();
            let (_, expected) = engine.render_with_resolution(text, &opened.effective_resolution(), opened.pixel_format()).unwrap();
            assert_eq!(text.cache.as_ref(), Some(&expected.surface));
        }
        if cached {
            assert_eq!(
                text.cache.as_ref().unwrap().try_read_interleaved(opened.bounds()).unwrap(),
                source(Some(axes), true, false).layers[0].surface().unwrap().try_read_interleaved(opened.bounds()).unwrap()
            );
        }
        let b = photocraft_format::save_to_bytes(opened, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&b).unwrap(), **opened);
        assert!(s.set_active(0));
    }
    let now = s.active().unwrap();
    assert!(now.doc.shares_state(&before));
    assert_eq!(now.doc, before);
    assert_eq!((now.revision, now.saved_revision, now.active_layer, now.layer_anchor, now.source.clone(), now.path.clone()), meta);
    assert_eq!(now.selected_layers, selected);
    assert_eq!(now.history.entry_labels().collect::<Vec<_>>(), past);
    assert_eq!(now.history.redo_labels().collect::<Vec<_>>(), redo);
    assert_eq!(photocraft_format::save_to_bytes(&now.doc, &Default::default()).unwrap(), native);
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *before);
    assert_eq!(clipboard(&s), oldclip);
    assert!(Arc::ptr_eq(&tile, s.clipboard.as_ref().unwrap().surface.tiles().next().unwrap().1));
    if let Some(old) = profile {
        assert!(Arc::ptr_eq(&old, s.clipboard.as_ref().unwrap().color.icc_profile.as_ref().unwrap()));
    }
    assert_eq!(s.journal, oldjournal);
    assert_eq!(bytes, retained);
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc, expected_redo);
}
#[test]
fn native_psd_open_paired_missing_cache_preserves_existing_work() {
    check("psd", true, [144., 288.], false, false);
}
#[test]
fn native_psb_open_paired_missing_cache_preserves_existing_work() {
    check("psb", true, [144., 288.], false, false);
}
#[test]
fn synchronous_psd_open_paired_missing_cache_preserves_existing_work() {
    check("psd", false, [144., 288.], false, false);
}
#[test]
fn synchronous_psb_open_paired_missing_cache_preserves_existing_work() {
    check("psb", false, [144., 288.], false, false);
}
#[test]
fn cached_unequal_and_square_missing_open_controls_preserve_existing_work() {
    for ext in ["psd", "psb"] {
        for background in [false, true] {
            check(ext, background, [144., 288.], true, false);
            check(ext, background, [144., 144.], false, false);
        }
    }
}

#[test]
fn background_psd_unrepresentable_finite_type_geometry_is_atomic() {
    check("psd", true, [144., 288.], false, true);
}
#[test]
fn background_psb_unrepresentable_finite_type_geometry_is_atomic() {
    check("psb", true, [144., 288.], false, true);
}
#[test]
fn synchronous_psd_unrepresentable_finite_type_geometry_is_atomic() {
    check("psd", false, [144., 288.], false, true);
}
#[test]
fn synchronous_psb_unrepresentable_finite_type_geometry_is_atomic() {
    check("psb", false, [144., 288.], false, true);
}
