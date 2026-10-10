//! The screen-sized JPEG a host browses with: bounded size, sRGB colours, matted transparency.

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::export_display_jpeg;
use photocraft_raster::Surface;

fn solid(w: u32, h: u32, depth: SampleType, px: [f32; 4]) -> Document {
    let mut d = Document::new("p", Size::new(w, h), ColorMode::Rgb, depth);
    let mut s = Surface::new(d.pixel_format());
    s.fill_rect(Rect::new(0, 0, w as i32, h as i32), &px);
    d.layers.push(Layer::new("bg", LayerContent::Raster(s)));
    d
}

fn decode(bytes: &[u8]) -> photocraft_codecs::Image {
    photocraft_codecs::decode(bytes).expect("the preview decodes")
}

fn centre(img: &photocraft_codecs::Image) -> [u8; 3] {
    let (w, h) = img.dimensions();
    let channels = img.data().len() / (w as usize * h as usize);
    let at = ((h / 2) as usize * w as usize + (w / 2) as usize) * channels;
    [img.data()[at], img.data()[at + 1], img.data()[at + 2]]
}

#[test]
fn long_side_is_bounded_and_aspect_is_kept() {
    let doc = solid(3000, 2000, SampleType::U16, [0.5, 0.5, 0.5, 1.0]);
    let out = export_display_jpeg(&doc, 1024, 85).unwrap();
    assert!(out.bytes.starts_with(&[0xFF, 0xD8]), "not a JPEG");
    let img = decode(&out.bytes);
    assert_eq!(img.dimensions(), (1024, 683));
}

#[test]
fn small_documents_are_not_upscaled() {
    let doc = solid(200, 100, SampleType::U8, [0.2, 0.4, 0.6, 1.0]);
    let img = decode(&export_display_jpeg(&doc, 2560, 90).unwrap().bytes);
    assert_eq!(img.dimensions(), (200, 100));
}

#[test]
fn srgb_documents_keep_their_colours() {
    let doc = solid(64, 64, SampleType::U8, [0.8, 0.4, 0.2, 1.0]);
    let [r, g, b] = centre(&decode(&export_display_jpeg(&doc, 64, 95).unwrap().bytes));
    for (got, want) in [(r, 204.0), (g, 102.0), (b, 51.0)] {
        assert!((f32::from(got) - want).abs() <= 6.0, "{got} vs {want}");
    }
}

#[test]
fn wide_gamut_documents_are_converted_to_srgb() {
    // A fully saturated ProPhoto green is outside sRGB: it must clip to sRGB green rather than
    // be written as the same numbers (which a viewer that ignores profiles would show as a
    // different, duller colour).
    let mut doc = solid(64, 64, SampleType::U16, [0.0, 1.0, 0.0, 1.0]);
    doc.icc_profile = Some(Arc::new(photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes().to_vec()));
    let [r, g, b] = centre(&decode(&export_display_jpeg(&doc, 64, 95).unwrap().bytes));
    assert!(g >= 240 && r <= 20 && b <= 20, "{r} {g} {b}");
    // The same numbers in an sRGB document stay put, so the conversion is what changed them.
    let plain = solid(64, 64, SampleType::U16, [0.5, 0.5, 0.5, 1.0]);
    let mut tagged = plain.clone();
    tagged.icc_profile = Some(Arc::new(photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes().to_vec()));
    let flat = centre(&decode(&export_display_jpeg(&plain, 64, 95).unwrap().bytes))[0];
    let converted = centre(&decode(&export_display_jpeg(&tagged, 64, 95).unwrap().bytes))[0];
    assert_ne!(flat, converted, "a ProPhoto grey and an sRGB grey of the same number are different colours");
}

#[test]
fn transparency_is_matted_over_white() {
    let doc = solid(32, 32, SampleType::U8, [0.0, 0.0, 0.0, 0.0]);
    let [r, g, b] = centre(&decode(&export_display_jpeg(&doc, 32, 95).unwrap().bytes));
    assert!(r >= 250 && g >= 250 && b >= 250, "{r} {g} {b}");
}

#[test]
fn no_metadata_or_profile_is_written() {
    let mut doc = solid(32, 32, SampleType::U8, [0.3, 0.3, 0.3, 1.0]);
    doc.icc_profile = Some(Arc::new(photocraft_cms::Builtin::DisplayP3.profile().to_bytes().to_vec()));
    let bytes = export_display_jpeg(&doc, 32, 90).unwrap().bytes;
    assert!(!bytes.windows(12).any(|w| w == b"ICC_PROFILE\0"), "an ICC profile was embedded");
    assert!(decode(&bytes).icc.is_none());
}
