#![cfg(feature = "corpus")]
use photocraft_algo::gimp::{
    point::try_apply,
    registry::{WorkingEncoding, point_member},
};
use photocraft_cms::{Builtin, Intent, Profile, Transform};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use std::path::PathBuf;

#[test]
fn authored_point_source_and_edit_full_rgba_display_oracles() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    assert!(root.is_dir(), "stage full XCF corpus or set PHOTOCRAFT_XCF_FULL_CORPUS");
    for (name, version) in
        [("gegl-levels", "0:0"), ("gegl-exposure", "1:0"), ("gegl-brightness-contrast", "0:0"), ("gegl-invert-linear", "0:0"), ("gegl-invert-gamma", "0:0")]
    {
        let dir = root.join("operations").join(name);
        let base = format!("operation-{name}");
        let raw = std::fs::read(dir.join(format!("{base}.source.0.native.bin"))).unwrap();
        assert_eq!(raw.len(), 32 * 32 * 4);
        assert!(raw.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("probe.json")).unwrap()).unwrap();
        let strings = &proof["cases"][0]["serialization"]["tables"][0]["records"][0]["effects"][0]["strings"];
        assert_eq!(strings[2], name.replacen('-', ":", 1));
        assert_eq!(strings[3], version);
        for (stage, suffix) in [("reopened", "ref.png"), ("edited", "edit.ref.png")] {
            let state: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{base}.{stage}_state.json"))).unwrap()).unwrap();
            let filter = &state["layers"][0]["filters"][0];
            let config = filter["config"].as_object().unwrap();
            let args: Vec<(&str, f64)> = config.iter().map(|(n, v)| (n.as_str(), v.as_f64().unwrap())).collect();
            let operation = name.replacen('-', ":", 1);
            let (member, evaluator) = point_member(&operation, Some(version), &args).unwrap();
            let reference = photocraft_codecs::decode(&std::fs::read(dir.join(format!("{base}.{suffix}"))).unwrap()).unwrap();
            let working = Profile::parse(reference.icc.as_ref().unwrap()).unwrap();
            let linear = Builtin::LinearSrgb.profile();
            let to_linear = Transform::new(&working, linear, Intent::RelativeColorimetric, false).unwrap();
            let from_linear = Transform::new(linear, &working, Intent::RelativeColorimetric, false).unwrap();
            let mut samples = Vec::new();
            for px in raw.as_chunks::<4>().0.iter() {
                let encoded = [f64::from(px[0]) / 255.0, f64::from(px[1]) / 255.0, f64::from(px[2]) / 255.0];
                let mut value = encoded;
                if member.encoding == WorkingEncoding::Linear {
                    to_linear.try_eval_f64(&encoded, &mut value).unwrap();
                }
                samples.extend_from_slice(&value);
                samples.push(1.0);
            }
            let area = Rect::new(0, 0, 32, 32);
            let mut source = Surface::new(PixelFormat::RGBA64F);
            source.try_write_region_f64(area, &samples).unwrap();
            let result = if filter["visible"] == false {
                source.clone()
            } else {
                try_apply(&source, area, evaluator, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).unwrap()
            };
            let reference = reference.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            let pixels = result.try_read_region_f64(area).unwrap();
            let mut max = [0.0_f64; 4];
            let mut sum = [0.0_f64; 4];
            for (px, expected) in pixels.as_chunks::<4>().0.iter().zip(reference.data().as_chunks::<4>().0.iter()) {
                let mut encoded = [px[0], px[1], px[2]];
                if member.encoding == WorkingEncoding::Linear {
                    from_linear.try_eval_f64(&px[..3], &mut encoded).unwrap();
                }
                for c in 0..4 {
                    let v = if c == 3 { px[3] } else { encoded[c] };
                    let d = (v.clamp(0.0, 1.0) * 255.0 - f64::from(expected[c])).abs();
                    max[c] = max[c].max(d);
                    sum[c] += d;
                }
            }
            println!("{name}/{stage}: max={max:?} mean={:?}", sum.map(|v| v / 1024.0));
            assert!(max.iter().all(|v| *v <= 2.0), "existing 2/255 contract failed: {name}/{stage} {max:?}");
            assert_eq!(source.try_read_region_f64(area).unwrap(), samples);
        }
    }
}
