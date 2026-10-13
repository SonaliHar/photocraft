use photocraft_raster::Interrupt;
use photocraft_text::xcf::outline::{CoverageWorkspace, coverage_24_8, quadratic_24_8};

#[test]
fn signed_half_cell_and_reverse_keep_observed_quantization() {
    let mut workspace = CoverageWorkspace::new(&mut |_| Ok(())).unwrap();
    let forward = [[0, 0], [128, 0], [128, 256], [0, 256]];
    let reverse = [[0, 256], [128, 256], [128, 0], [0, 0]];
    assert_eq!(coverage_24_8(&mut workspace, &[&forward], [0, 0], &Interrupt::NONE).unwrap(), 127);
    assert_eq!(coverage_24_8(&mut workspace, &[&reverse], [0, 0], &Interrupt::NONE).unwrap(), 128);
    assert_eq!(coverage_24_8(&mut workspace, &[&forward, &reverse], [0, 0], &Interrupt::NONE).unwrap(), 0);
}

#[test]
fn subunit_negative_area_does_not_fabricate_visible_ink() {
    let mut workspace = CoverageWorkspace::new(&mut |_| Ok(())).unwrap();
    let tiny = [[0, 4], [4, 0], [0, 0]];
    assert_eq!(coverage_24_8(&mut workspace, &[&tiny], [0, 0], &Interrupt::NONE).unwrap(), 0);
}

#[test]
#[ignore = "requires independent 600 signed tiny triangle public API controls"]
fn low_area_public_quantization_controls_are_exact() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-freetype-basis");
    let bytes = std::fs::read(root.join("low-area-quantization.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "5326afb54b0b36d173a39188e7d90b7259eff26da8735849f5ed6d00492b53d6");
    let rows: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut workspace = CoverageWorkspace::new(&mut |_| Ok(())).unwrap();
    for row in rows["rows"].as_array().unwrap() {
        let points: Vec<[i64; 2]> = row["points_26_6"].as_array().unwrap().iter().map(|p| [p[0].as_i64().unwrap() * 4, p[1].as_i64().unwrap() * 4]).collect();
        for pixel in row["public_pixels"].as_array().unwrap() {
            let position = [pixel[0].as_i64().unwrap() as i32, pixel[1].as_i64().unwrap() as i32];
            let actual = coverage_24_8(&mut workspace, &[&points], position, &Interrupt::NONE).unwrap();
            assert_eq!(actual, pixel[2].as_u64().unwrap() as u8, "{points:?} at {position:?}");
        }
    }
    assert_eq!(rows["rows"].as_array().unwrap().len(), 600);
}

#[test]
fn curvature_threshold_uses_promoted_grid_and_uniform_level() {
    let mut charged = 0;
    let points = quadratic_24_8([[0, 0], [256, 132], [512, 0]], &Interrupt::NONE, &mut |bytes| {
        charged += bytes;
        Ok(())
    })
    .unwrap();
    assert_eq!(points.len(), 5);
    assert_eq!(points, [[0, 0], [128, 49], [256, 66], [384, 49], [512, 0]]);
    assert!(charged >= points.len() as u64 * 16);
}

#[test]
fn invalid_huge_coordinates_and_denied_allocation_fail() {
    let mut workspace = CoverageWorkspace::new(&mut |_| Ok(())).unwrap();
    assert!(coverage_24_8(&mut workspace, &[&[[i64::MAX, 0], [0, 1], [1, 1]]], [0, 0], &Interrupt::NONE).is_err());
    assert!(quadratic_24_8([[0, 0], [256, 132], [512, 0]], &Interrupt::NONE, &mut |_| Err(photocraft_text::xcf::TextSourceError::Budget)).is_err());
}

#[test]
fn cancellation_during_subdivision_and_clipping_returns_no_output() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let checks = AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 2;
    let ctl = Interrupt::cancel_only(&cancel);
    assert_eq!(
        quadratic_24_8([[0, 0], [1_000_000, 1_000_000], [2_000_000, 0]], &ctl, &mut |_| Ok(())).unwrap_err(),
        photocraft_text::xcf::TextSourceError::Cancelled
    );
    checks.store(0, Ordering::Relaxed);
    let mut workspace = CoverageWorkspace::new(&mut |_| Ok(())).unwrap();
    let contour: Vec<_> = (0..1024).map(|i| [i * 256, (i % 2) * 256]).collect();
    assert_eq!(coverage_24_8(&mut workspace, &[&contour], [0, 0], &ctl), Err(photocraft_text::xcf::TextSourceError::Cancelled));
}

#[test]
fn bounded_eight_byte_shape_precharges_and_denies_before_output() {
    use photocraft_text::xcf::{ExactFont, ScaledText, TextSourceError, outline::prepare_outline_with_charge};
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for content in ["BBBBBBBB", "WWWWWWWW", "@@@@@@@@", "SSSSSSSS", "ffi AVTo"] {
        let mut charged = 0_u64;
        let prepared = prepare_outline_with_charge(
            &font,
            ScaledText { content, origin: [8, 4], size_px: 40. },
            8 * 1024 * 1024,
            &Interrupt::NONE,
            &mut |bytes| {
                charged += bytes;
                Ok(())
            },
            &mut |_| Ok(()),
        )
        .unwrap();
        assert!(charged <= 8 * 1024 * 1024);
        let before = charged;
        assert_eq!(
            prepared
                .render_with_charge(
                    8 * 1024 * 1024,
                    &Interrupt::NONE,
                    &mut |bytes| {
                        charged += bytes;
                        Ok(())
                    },
                    &mut |_| Err(TextSourceError::Budget)
                )
                .err(),
            Some(TextSourceError::Budget)
        );
        assert_eq!(charged, before, "work denial allocated output");
    }
    for content in ["123456789", "\0", "é", "        "] {
        assert!(
            prepare_outline_with_charge(
                &font,
                ScaledText { content, origin: [8, 4], size_px: 32. },
                8 * 1024 * 1024,
                &Interrupt::NONE,
                &mut |_| Ok(()),
                &mut |_| Ok(())
            )
            .is_err()
        );
    }
    let mut charged = 0;
    assert!(
        prepare_outline_with_charge(
            &font,
            ScaledText { content: "O", origin: [8, 4], size_px: 32. },
            2 * 1024 * 1024 - 1,
            &Interrupt::NONE,
            &mut |bytes| {
                charged += bytes;
                Ok(())
            },
            &mut |_| Ok(())
        )
        .is_err()
    );
    assert_eq!(charged, 0);
}

#[test]
#[ignore = "requires immutable independently captured public raster controls"]
fn arbitrary_threshold_reversed_translated_and_sloped_public_controls() {
    use sha2::{Digest, Sha256};
    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-integer-reference");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "cea51183de512247f4a2c4022c20362fde9050e1c68878f371ead88a087ac826");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, expected) in hashes {
        assert_eq!(hash(&std::fs::read(root.join(name)).unwrap()), expected);
    }
    let controls: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("controls.json")).unwrap()).unwrap();
    let mut workspace = CoverageWorkspace::new(&mut |_| Ok(())).unwrap();
    let mut worst = 0;
    for row in controls["rows"].as_array().unwrap() {
        let points: Vec<[i64; 2]> = row["points_26_6"].as_array().unwrap().iter().map(|p| [p[0].as_i64().unwrap() * 4, p[1].as_i64().unwrap() * 4]).collect();
        let contour = if row["kind"] == "conic" { quadratic_24_8(points.try_into().unwrap(), &Interrupt::NONE, &mut |_| Ok(())).unwrap() } else { points };
        let reference = std::fs::read(root.join(row["reference"].as_str().unwrap())).unwrap();
        let mut maximum = 0;
        for y in 0..32 {
            for x in 0..32 {
                let actual = coverage_24_8(&mut workspace, &[&contour], [x, y], &Interrupt::NONE).unwrap();
                maximum = maximum.max(actual.abs_diff(reference[((31 - y) * 32 + x) as usize]));
            }
        }
        worst = worst.max(maximum);
        assert!(maximum <= 2, "{}: {maximum}/255", row["reference"]);
    }
    assert_eq!(controls["rows"].as_array().unwrap().len(), 300);
    println!("300 independent public integer controls: worst {worst}/255");
}

#[test]
#[ignore = "requires original frozen GIMP exact-font whole-string controls"]
fn whole_string_prepared_current_geometry_matches_all_aa_controls() {
    use photocraft_text::xcf::{ExactFont, ScaledText, outline::prepare_outline_with_charge};
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "346c44a055c79a327db247f8df2019cef2e00d6a6462bcbb5f55fda0c5960c22");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, digest) in hashes {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(root.join(name)).unwrap())), digest);
    }
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let mut count = 0;
    let mut boundary_rows = Vec::new();
    for row in observations["rows"].as_array().unwrap().iter().filter(|row| row["antialias"] == true) {
        let mut resident = 0;
        let mut work = 0;
        let mut charge = |bytes| {
            resident += bytes;
            Ok(())
        };
        let mut consume = |units| {
            work += units;
            Ok(())
        };
        let prepared = prepare_outline_with_charge(
            &font,
            ScaledText { content: row["text"].as_str().unwrap(), size_px: row["size_px"].as_f64().unwrap() as f32, origin: [8, 4] },
            8 * 1024 * 1024,
            &Interrupt::NONE,
            &mut charge,
            &mut consume,
        )
        .unwrap();
        let rendered = prepared.render_with_charge(8 * 1024 * 1024, &Interrupt::NONE, &mut charge, &mut consume).unwrap();
        let pixels = rendered.surface.try_read_interleaved(rendered.rect).unwrap();
        let expected = std::fs::read(root.join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        let mut maximum = 0;
        let mut straight_maximum = 0;
        let mut zero_boundary_disagreements = 0;
        let mut boundaries = Vec::new();
        let native = std::fs::read(root.join(row["state"]["native"]["file"].as_str().unwrap())).unwrap();
        let cache_width = row["state"]["size"][0].as_i64().unwrap() as i32;
        let cache_height = row["state"]["size"][1].as_i64().unwrap() as i32;
        for y in 0..128 {
            for x in 0..256 {
                let actual = if x >= rendered.rect.x0 && x < rendered.rect.x1 && y >= rendered.rect.y0 && y < rendered.rect.y1 {
                    pixels[(((y - rendered.rect.y0) as usize * rendered.rect.width() as usize + (x - rendered.rect.x0) as usize) * 4) + 3]
                } else {
                    0
                };
                maximum = maximum.max(actual.abs_diff(expected[((y * 256 + x) * 4 + 3) as usize]));
                let at = ((y * 256 + x) * 4) as usize;
                zero_boundary_disagreements += usize::from((actual == 0) != (expected[at + 3] == 0));
                if (actual == 0) != (expected[at + 3] == 0) {
                    let source_rgba = if x >= 8 && x < 8 + cache_width && y >= 4 && y < 4 + cache_height {
                        native[(((y - 4) * cache_width + (x - 8)) * 4) as usize..(((y - 4) * cache_width + (x - 8)) * 4 + 4) as usize].to_vec()
                    } else {
                        vec![0; 4]
                    };
                    boundaries.push(serde_json::json!({"x":x,"y":y,"new_rgba":if actual==0 {vec![0;4]} else {vec![255,255,255,actual]},"projection_rgba":&expected[at..at+4],"original_native_rgba":source_rgba}));
                }
                for channel in 0..3 {
                    let rgb: u8 = if actual == 0 { 0 } else { 255 };
                    straight_maximum = straight_maximum.max(rgb.abs_diff(expected[at + channel]));
                }
            }
        }
        println!(
            "{}: alpha {maximum}/255, straightRGB {straight_maximum}/255, zero-boundary disagreements {zero_boundary_disagreements}, resident {resident}, work {work}, bounds {:?}",
            row["case"], rendered.rect
        );
        assert!(maximum <= 2);
        assert!(resident <= 8 * 1024 * 1024);
        assert!(work > 4096);
        boundary_rows.push(serde_json::json!({"case":row["case"],"alpha_max":maximum,"straight_rgb_max":straight_maximum,"zero_boundary_disagreements":zero_boundary_disagreements,"boundaries":boundaries}));
        count += 1;
    }
    assert_eq!(count, 24);
    std::fs::write(
        root.parent().unwrap().join("curve-kern-freetype-basis/current-native-projection-boundaries.json"),
        serde_json::to_vec_pretty(&boundary_rows).unwrap(),
    )
    .unwrap();
}
