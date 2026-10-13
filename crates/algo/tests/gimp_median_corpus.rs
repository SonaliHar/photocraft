#![cfg(feature = "corpus")]
use photocraft_algo::gimp::neighborhood::{MedianNeighborhood, MedianParams, try_median_blur_quantized_encoded};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
#[test]
fn original_median_source_and_radius_edit_match_png_reference() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("operations/gegl-median-blur");
    let proof: serde_json::Value = serde_json::from_slice(&fs::read(root.join("probe.json")).unwrap()).unwrap();
    let case = &proof["cases"][0];
    assert_eq!(case["state_reopened"]["layers"][0]["filters"][0]["config"]["radius"], 3);
    assert_eq!(case["state_edited"]["layers"][0]["filters"][0]["config"]["radius"], 4);
    for (file, hash) in [("input", "input_sha256"), ("reference_edit", "reference_edit_sha256")] {
        let original = fs::read(root.join(case[file].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(original)), case[hash].as_str().unwrap());
    }
    let raw = fs::read(root.join(case["state_before"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), case["state_before"]["layers"][0]["native"]["sha256"].as_str().unwrap());
    assert!(raw.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    for (radius, key, hashkey) in [(3, "reference", "reference_sha256"), (4, "reference_edit_render", "reference_edit_render_sha256")] {
        let bytes = fs::read(root.join(case[key].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), case[hashkey].as_str().unwrap());
        let png = photocraft_codecs::decode(&bytes).unwrap();
        let mut samples = Vec::new();
        for pixel in raw.as_chunks::<4>().0.iter() {
            let input = std::array::from_fn::<_, 3, _>(|c| f64::from(pixel[c]) / 255.);
            samples.extend_from_slice(&input);
            samples.push(1.);
        }
        let area = Rect::new(0, 0, 32, 32);
        let mut source = Surface::new(PixelFormat::RGBA64F);
        source.try_write_region_f64(area, &samples).unwrap();
        let output = area;
        let result = try_median_blur_quantized_encoded(
            &source,
            output,
            area,
            MedianParams { radius, neighborhood: MedianNeighborhood::Circle, percentile: 50., alpha_percentile: 50., high_precision: false },
            Default::default(),
            &Interrupt::NONE,
        )
        .unwrap()
        .try_read_region_f64(output)
        .unwrap();
        let expected = png.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
        assert_eq!(result.len(), expected.data().len());
        let mut worst = [0f64; 4];
        for (pixel, reference) in result.as_chunks::<4>().0.iter().zip(expected.data().as_chunks::<4>().0.iter()) {
            for c in 0..4 {
                let actual = pixel[c];
                worst[c] = worst[c].max((actual.clamp(0., 1.) * 255. - f64::from(reference[c])).abs());
            }
        }
        println!("Median radius{radius} actual original PNG maximum per255 {worst:?}");
        assert!(worst.iter().all(|v| *v <= 2.));
    }
}
