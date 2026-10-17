use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, PhysicalResolution, Size, TextLayer};
use photocraft_geom::Rect;
use sha2::{Digest, Sha256};
use std::sync::Arc;
fn hash(b: &[u8]) -> Vec<u8> {
    Sha256::digest(b).to_vec()
}
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
        t.psd_raw = Some(Arc::new(photocraft_text::psd::build_tysh_with_resolution(&t, &d.effective_resolution(), None).unwrap()));
        t.cache = Some(s);
        d.layers.push(Layer::new("Retained editable type", LayerContent::Text(t)));
    }
    d
}
fn fixture(d: &Document, ext: &str) -> Vec<u8> {
    let native = photocraft_format::save_to_bytes(d, &Default::default()).unwrap();
    let bytes = photocraft_io::export(d, ext, &Default::default()).unwrap().bytes;
    assert_eq!(photocraft_format::save_to_bytes(d, &Default::default()).unwrap(), native, "export mutated model");
    bytes
}
fn public_paired_layout(ext: &str) {
    let d = source(Some([144.0, 288.0]), false, false);
    let bytes = fixture(&d, ext);
    let before = hash(&bytes);
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let (parsed, parsed_warnings) = photocraft_io::psd_to_document(&file);
    let LayerContent::Text(parsed_text) = &parsed.layers[0].content else { panic!("parsed type lost") };
    assert!(parsed_text.cache.is_none(), "fixture unexpectedly retained nonempty pixels");
    assert_eq!([parsed.effective_resolution().x_dpi, parsed.effective_resolution().y_dpi], [144.0, 288.0]);
    assert!(parsed_warnings.is_empty());
    let parsed_native = photocraft_format::save_to_bytes(&parsed, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&parsed_native).unwrap(), parsed);
    // Paired physical-axis layout is now implemented. Public import must retain
    // its point styles and authored TySh while generating the missing cache.
    let imported = photocraft_io::import(ext, &bytes).unwrap().document;
    assert_eq!(hash(&bytes), before);
    let LayerContent::Text(text) = &imported.layers[0].content else { panic!("imported type lost") };
    assert!(text.cache.as_ref().is_some_and(|s| !s.content_bounds().is_empty()));
    assert_eq!(text.char_runs(), parsed_text.char_runs());
    assert_eq!(text.psd_raw, parsed_text.psd_raw);
    assert_eq!(imported.effective_resolution(), parsed.effective_resolution());
}
fn accepted(axes: Option<[f64; 2]>, cached: bool, raster: bool, _label: &str) {
    for ext in ["psd", "psb"] {
        let d = source(axes, cached, raster);
        let bytes = fixture(&d, ext);
        let before = hash(&bytes);
        let r = photocraft_io::import(ext, &bytes).unwrap();
        assert_eq!(hash(&bytes), before);
        let resolution = r.document.effective_resolution();
        let expected = axes.unwrap_or([72.0, 72.0]);
        assert_eq!([resolution.x_dpi, resolution.y_dpi], expected);
        if raster {
            assert_eq!(
                r.document.layers[0].surface().unwrap().try_read_interleaved(d.bounds()).unwrap(),
                d.layers[0].surface().unwrap().try_read_interleaved(d.bounds()).unwrap()
            );
        } else {
            let LayerContent::Text(t) = &r.document.layers[0].content else { panic!("text lost") };
            let LayerContent::Text(original) = &d.layers[0].content else { panic!("invalid fixture") };
            assert!(t.cache.as_ref().is_some_and(|s| !s.content_bounds().is_empty()));
            let raw = t.psd_raw.as_ref().unwrap();
            let old = original.psd_raw.as_ref().unwrap();
            assert!(raw.starts_with(old));
            assert!(raw.len() - old.len() <= 3);
            assert!(raw[old.len()..].iter().all(|b| *b == 0));
            assert_eq!(t.char_runs()[0].style.size_pt, 12.0);
            if cached {
                assert_eq!(
                    t.cache.as_ref().unwrap().try_read_interleaved(d.bounds()).unwrap(),
                    original.cache.as_ref().unwrap().try_read_interleaved(d.bounds()).unwrap()
                );
            }
        }
        let native = photocraft_format::save_to_bytes(&r.document, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), r.document);
    }
}
#[test]
fn public_psd_missing_cache_unequal_uses_paired_layout() {
    public_paired_layout("psd");
}
#[test]
fn public_psb_missing_cache_unequal_uses_paired_layout() {
    public_paired_layout("psb");
}
#[test]
fn square_missing_cache_remains_usable() {
    accepted(Some([144.0, 144.0]), false, false, "square-missing");
}
#[test]
fn legacy_missing_cache_remains_usable() {
    accepted(None, false, false, "legacy-missing");
}
#[test]
fn unequal_raster_remains_usable() {
    accepted(Some([144.0, 288.0]), false, true, "unequal-raster");
}
#[test]
fn unequal_nonempty_cached_type_retains_raw_pixels_and_axes() {
    accepted(Some([144.0, 288.0]), true, false, "unequal-cached");
}
