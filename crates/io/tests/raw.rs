//! Camera raw import: synthetic DNG / CR2 develop into a 16-bit ProPhoto
//! document; unsupported raw variants fall back to the embedded preview.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use photocraft_color::{ColorMode, SampleType};
use photocraft_io::{IoError, import};
use photocraft_raw::testgen::{Cr2Spec, DngSpec, TiffBuilder, Val, mosaic, scene};

#[test]
fn dng_opens_as_16_bit_prophoto() {
    let (w, h) = (40, 24);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535));
    spec.as_shot_neutral = Some([0.6, 1.0, 0.8]);
    let r = import("shot.dng", &spec.build()).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (40, 24));
    assert_eq!(d.mode, ColorMode::Rgb);
    assert_eq!(d.depth, SampleType::U16);
    assert_eq!(d.layers.len(), 1);
    let icc = d.icc_profile.as_ref().expect("profile");
    assert_eq!(icc.as_slice(), &photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes()[..]);
    assert!(r.warnings.iter().any(|w| w.contains("DNG") && w.contains("ProPhoto")), "{:?}", r.warnings);
}

#[test]
fn cr2_opens() {
    let (w, h) = (48, 16);
    let data = mosaic(&scene(w, h), w, [0, 1, 1, 2], 256, 12000);
    let spec =
        Cr2Spec { width: w, height: h, data, precision: 14, components: 2, slices: vec![24, 24], borders: None, wb_rggb: None, orientation: 6, model_id: None };
    let r = import("IMG_0001.CR2", &spec.build()).unwrap();
    // Orientation 6 rotates the 48×16 sensor image to 16×48.
    assert_eq!((r.document.size.width, r.document.size.height), (16, 48));
    assert_eq!(r.document.depth, SampleType::U16);
}

/// A NEF-like file with Nikon's (undocumented) compression and a full-size
/// baseline JPEG preview in IFD0.
fn nef_with_preview() -> Vec<u8> {
    let img = Image::from_u8(32, 20, ChannelLayout::Rgb, vec![180; 32 * 20 * 3]).unwrap();
    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let mut t = TiffBuilder::default();
    let strip = t.blob(vec![0; 64]);
    let preview = t.blob(jpeg.clone());
    let raw = t.ifd(vec![
        (256, Val::Long(vec![8])),
        (257, Val::Long(vec![8])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![34713])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (279, Val::Long(vec![64])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
    ]);
    let ifd0 = t.ifd(vec![
        (271, Val::Ascii("NIKON CORPORATION".into())),
        (330, Val::Ifds(vec![raw])),
        (513, Val::Blobs(vec![preview])),
        (514, Val::Long(vec![jpeg.len() as u32])),
    ]);
    t.chain = vec![ifd0];
    t.build()
}

#[test]
fn unsupported_raw_falls_back_to_the_embedded_preview() {
    let r = import("DSC_0001.NEF", &nef_with_preview()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (32, 20));
    assert!(r.warnings.first().is_some_and(|w| w.contains("Nikon compressed NEF") && w.contains("embedded")), "{:?}", r.warnings);
    let error = photocraft_io::raw_smart::import("DSC_0001.NEF", &nef_with_preview()).unwrap_err();
    assert!(error.to_string().contains("Nikon compressed NEF"), "{error}");
}

#[test]
fn unsupported_raw_without_preview_is_a_clear_error() {
    let mut cr3 = vec![0, 0, 0, 24];
    cr3.extend_from_slice(b"ftypcrx ");
    cr3.extend_from_slice(&[0; 12]);
    match import("IMG_0001.CR3", &cr3) {
        Err(e @ IoError::Raw(_)) => assert!(e.to_string().contains("CR3"), "{e}"),
        Err(e) => panic!("expected a raw error, got {e}"),
        Ok(_) => panic!("CR3 must not decode yet"),
    }
}

#[test]
fn truncated_raws_do_not_panic() {
    let b = nef_with_preview();
    for n in (0..b.len()).step_by(7) {
        let _ = import("x.nef", &b[..n]);
    }
}

/// Camera-space patches, each with a tonal ramp.
fn patches(w: usize, h: usize) -> Vec<[f32; 3]> {
    let mut seed = 99u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let colours: Vec<[f32; 3]> = (0..96).map(|_| [0.12 + 0.5 * rnd(), 0.12 + 0.5 * rnd(), 0.12 + 0.5 * rnd()]).collect();
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let ramp = 0.15 + 0.85 * ((x * 12) % w) as f32 / w as f32;
            colours[(y * 8 / h) * 12 + x * 12 / w].map(|v| v * ramp)
        })
        .collect()
}

/// A "camera JPEG" of `cam`: more saturated than sRGB primaries, with a brightening curve.
fn camera_rendering(cam: &[[f32; 3]], w: usize, h: usize) -> Vec<u8> {
    let k = [[1.3f32, -0.2, -0.1], [-0.15, 1.25, -0.1], [0.0, -0.25, 1.25]];
    let px: Vec<u8> = cam
        .iter()
        .flat_map(|c| {
            (0..3).map(move |i| {
                let v = (k[i][0] * c[0] + k[i][1] * c[1] + k[i][2] * c[2]).clamp(0.0, 1.0);
                let v = 1.6 * v / (1.0 + 0.6 * v);
                let e = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                (e * 255.0).round() as u8
            })
        })
        .collect();
    jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, px).unwrap())
}

fn jpeg(img: Image) -> Vec<u8> {
    photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 95, ..Default::default() }).unwrap()
}

/// An uncompressed NEF-like raw of `cam` with an optional JPEG preview in IFD0.
fn nef_uncompressed(cam: &[[f32; 3]], w: usize, h: usize, preview: Option<Vec<u8>>) -> Vec<u8> {
    let data = mosaic(cam, w, [0, 1, 1, 2], 0, 4095);
    let mut t = TiffBuilder::default();
    let strip = t.blob(data.iter().flat_map(|v| v.to_le_bytes()).collect());
    let raw = t.ifd(vec![
        (254, Val::Long(vec![0])),
        (256, Val::Long(vec![w as u32])),
        (257, Val::Long(vec![h as u32])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![h as u32])),
        (279, Val::Long(vec![(data.len() * 2) as u32])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
    ]);
    let mut ifd0 = vec![(271, Val::Ascii("NIKON CORPORATION".into())), (330, Val::Ifds(vec![raw]))];
    if let Some(j) = preview {
        let len = j.len() as u32;
        let b = t.blob(j);
        ifd0.extend([(513, Val::Blobs(vec![b])), (514, Val::Long(vec![len]))]);
    }
    let ifd0 = t.ifd(ifd0);
    t.chain = vec![ifd0];
    t.build()
}

fn fitted(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("fitted to the camera's embedded JPEG"))
}

fn fallback(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("sRGB primaries"))
}

#[test]
fn uncalibrated_raw_takes_its_colour_from_the_camera_jpeg() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let r = import("DSC_0002.NEF", &nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)))).unwrap();
    assert!(fitted(&r.warnings) && !fallback(&r.warnings), "{:?}", r.warnings);
    assert_eq!((r.document.size.width, r.document.size.height), (480, 320));
    assert_eq!(r.document.depth, SampleType::U16);
}

#[test]
fn unusable_previews_keep_the_neutral_fallback() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let mut seed = 5u32;
    let noise: Vec<u8> = (0..w * h * 3)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) as u8
        })
        .collect();
    let mut garbage = jpeg(Image::from_u8(64, 48, ChannelLayout::Rgb, vec![90; 64 * 48 * 3]).unwrap());
    let sos = garbage.windows(2).rposition(|p| p == [0xFF, 0xDA]).unwrap();
    garbage.truncate(sos + 8);
    garbage.extend(std::iter::repeat_n(0xAB, 200));
    let previews = [
        None,
        Some(jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, noise).unwrap())),
        Some(jpeg(Image::from_u8(240, 240, ChannelLayout::Rgb, vec![128; 240 * 240 * 3]).unwrap())),
        Some(jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Gray, vec![128; w * h]).unwrap())),
        Some(garbage),
    ];
    for (i, p) in previews.into_iter().enumerate() {
        let r = import("DSC_0003.NEF", &nef_uncompressed(&cam, w, h, p)).unwrap();
        assert!(fallback(&r.warnings) && !fitted(&r.warnings), "preview {i}: {:?}", r.warnings);
    }
}

#[test]
fn truncated_raws_with_a_fittable_preview_do_not_panic() {
    let (w, h) = (96, 64);
    let cam = patches(w, h);
    let b = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    for n in (0..b.len()).step_by(211) {
        let _ = import("x.nef", &b[..n]);
    }
}

/// Mean of each colour channel of a developed document, 0..1.
fn channel_means(r: &photocraft_io::ImportResult) -> [f64; 3] {
    let img = photocraft_io::document_to_image(&r.document, &mut Vec::new()).unwrap();
    let n = img.layout().channels();
    let values = img.to_normalized();
    let mut sum = [0.0f64; 3];
    for px in values.chunks_exact(n) {
        for (s, v) in sum.iter_mut().zip(px) {
            *s += f64::from(*v);
        }
    }
    sum.map(|s| s / (values.len() / n) as f64)
}

/// Camera Raw's Open re-develops a raw with the photographer's white balance. The colour fitted to the camera's JPEG
/// is the camera's own rendering, so it must be fitted at the as-shot white balance: a matrix fitted to a changed
/// white balance absorbs the change and the control would do nothing.
#[test]
fn a_changed_white_balance_is_not_cancelled_by_the_fitted_colour() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let bytes = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    let develop = |white_balance| {
        let opts = photocraft_raw::DevelopOptions { white_balance, ..Default::default() };
        photocraft_io::raw::import_raw_with("DSC_0004.NEF", &bytes, &opts).unwrap()
    };
    let as_shot = develop(photocraft_raw::WhiteBalance::AsShot);
    let warm = develop(photocraft_raw::WhiteBalance::Multipliers([1.5, 1.0, 0.65]));
    let cool = develop(photocraft_raw::WhiteBalance::Multipliers([0.7, 1.0, 1.4]));
    for r in [&as_shot, &warm, &cool] {
        assert!(fitted(&r.warnings), "{:?}", r.warnings);
    }
    let red_over_blue = |r: &photocraft_io::ImportResult| channel_means(r)[0] / channel_means(r)[2];
    let (before, warmer, cooler) = (red_over_blue(&as_shot), red_over_blue(&warm), red_over_blue(&cool));
    // Cancelled, the ratio stays within about 1% of the as-shot one (0.99 measured before the fix).
    assert!(warmer > before * 1.15, "red/blue {before:.3} as shot, {warmer:.3} with a warmer white balance");
    assert!(cooler < before / 1.15, "red/blue {before:.3} as shot, {cooler:.3} with a cooler white balance");
}

/// The look does not depend on the exposure asked for either: exposure is applied after it.
#[test]
fn exposure_still_scales_a_fitted_raw() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let bytes = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    let develop = |exposure| {
        let opts = photocraft_raw::DevelopOptions { exposure, ..Default::default() };
        photocraft_io::raw::import_raw_with("DSC_0005.NEF", &bytes, &opts).unwrap()
    };
    let (flat, bright) = (develop(0.0), develop(1.0));
    assert!(fitted(&flat.warnings) && fitted(&bright.warnings));
    let (a, b) = (channel_means(&flat), channel_means(&bright));
    assert!(b[1] > a[1] * 1.2, "green {:.3} at 0 EV, {:.3} at +1 EV", a[1], b[1]);
}

/// A raw as browsed in Camera Raw: a fraction of the size, the full development's colour.
#[test]
fn a_browsed_raw_is_reduced_in_size_but_not_in_colour() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let bytes = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    let reduced = photocraft_io::raw::import_raw_browse_reduced_to("DSC_0010.NEF", &bytes, 100).unwrap().expect("a Bayer raw reduces");
    assert_eq!(reduced.full_size, (480, 320));
    let (rw, rh) = (reduced.import.document.size.width, reduced.import.document.size.height);
    assert!(rw < 480 && rh < 320, "reduced: {rw}x{rh}");
    assert_eq!(rw as usize * h, rh as usize * w, "the same shape: {rw}x{rh}");
    assert!(fitted(&reduced.import.warnings), "the colour fitted to the camera's JPEG is kept: {:?}", reduced.import.warnings);
    assert!(reduced.import.warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "still what the dialog looks for");
    let full = photocraft_io::raw::import_raw_with("DSC_0010.NEF", &bytes, &photocraft_raw::DevelopOptions::default()).unwrap();
    let (a, b) = (channel_means(&full), channel_means(&reduced.import));
    for k in 0..3 {
        assert!((a[k] - b[k]).abs() < 0.03, "channel {k}: full {:.3}, reduced {:.3}", a[k], b[k]);
    }
}

#[test]
fn a_calibrated_dng_reduces_with_its_own_colour() {
    let (w, h) = (96, 64);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535));
    spec.as_shot_neutral = Some([0.6, 1.0, 0.8]);
    let reduced = photocraft_io::raw::import_raw_browse_reduced_to("shot.dng", &spec.build(), 20).unwrap().expect("a DNG reduces");
    assert_eq!(reduced.full_size, (96, 64));
    assert!(!fitted(&reduced.import.warnings), "a calibrated file keeps its calibration: {:?}", reduced.import.warnings);
}

#[test]
fn a_raw_the_decoder_cannot_read_is_left_to_the_full_import() {
    assert!(photocraft_io::raw::import_raw_browse_reduced_to("x.NEF", &nef_with_preview(), 10).unwrap().is_none());
}

#[test]
fn truncated_raws_do_not_panic_when_reduced() {
    let (w, h) = (96, 64);
    let cam = patches(w, h);
    let b = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    for n in (0..b.len()).step_by(211) {
        let _ = photocraft_io::raw::import_raw_browse_reduced_to("x.nef", &b[..n], 20);
    }
}

/// Photos at or under twice the browsing size are cheap to develop in full: reducing them would only lose pixels.
#[test]
fn a_photo_that_is_already_small_is_not_reduced() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let bytes = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    assert!(photocraft_io::raw::import_raw_browse_reduced("DSC_0011.NEF", &bytes).unwrap().is_none(), "the browsing size is far above 480 px");
    assert!(photocraft_io::raw::import_raw_browse_reduced_to("DSC_0011.NEF", &bytes, 241).unwrap().is_none(), "just under twice the target");
    assert!(photocraft_io::raw::import_raw_browse_reduced_to("DSC_0011.NEF", &bytes, 240).unwrap().is_some(), "twice the target");
}
