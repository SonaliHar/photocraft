#![cfg(feature = "corpus")]
use photocraft_algo::gimp::curves::StoredCurvesConfiguration;
use std::path::PathBuf;

#[test]
fn exact_stored_free256_source_and_edit_match_independent_projection() {
    compare_stored_tables("free256-rgb-u8", "free");
}

#[test]
fn exact_stored_smooth256_source_and_edit_match_without_point_regeneration() {
    compare_stored_tables("smooth-corners-rgb-u8", "smooth");
}

fn compare_stored_tables(id: &str, curve_type: &str) {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("curve-config-matrix");
    for edit in [false, true] {
        let phase = if edit { "edit-source" } else { "source" };
        let stored = std::fs::read_to_string(root.join(format!("{id}.{}.stored-config.txt", if edit { "edit" } else { "source" }))).unwrap();
        assert!(stored.starts_with("(trc non-linear)"));
        assert_eq!(stored.matches(&format!("(curve-type {curve_type})")).count(), 5);
        let tables: Vec<Vec<f64>> = stored
            .lines()
            .filter_map(|line| line.trim().strip_prefix("(samples 256 "))
            .map(|line| line.trim_end_matches(')').split_whitespace().map(|v| v.parse().unwrap()).collect())
            .collect();
        assert_eq!(tables.len(), 5);
        assert!(tables.iter().all(|v| v.len() == 256));
        let config = StoredCurvesConfiguration { tables: std::array::from_fn(|i| tables[i].as_slice()) }.prepare().unwrap();
        let native = std::fs::read(root.join(format!("{id}.{phase}.0.native.bin"))).unwrap();
        assert_eq!(native.len(), 12 * 9 * 4);
        let reference = photocraft_codecs::decode(&std::fs::read(root.join(format!("{id}.{}", if edit { "edit.ref.png" } else { "ref.png" }))).unwrap())
            .unwrap()
            .convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
        assert_eq!(reference.data().len(), native.len());
        let mut max = [0.0_f64; 4];
        for (raw, expected) in native.as_chunks::<4>().0.iter().zip(reference.data().as_chunks::<4>().0.iter()) {
            let input = raw.iter().map(|v| f64::from(*v) / 255.0).collect::<Vec<_>>();
            let mut out = [0.0; 4];
            config.try_eval_pixel(&input, 3, &mut out).unwrap();
            if out[3] <= 0.0 {
                out[..3].fill(0.0);
            }
            for c in 0..4 {
                max[c] = max[c].max((out[c] * 255.0 - f64::from(expected[c])).abs());
            }
        }
        println!("{id} edit={edit} max255={max:?}");
        assert!(max.iter().all(|v| *v <= 2.0), "{max:?}");
    }
}
