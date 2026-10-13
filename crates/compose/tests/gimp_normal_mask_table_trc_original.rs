#![cfg(feature = "corpus")]
//! Own saved Normal28 explicit operators with independently observed ordinary masks.
use photocraft_compose::gimp_render::{PreparedRenderer, try_render};
use photocraft_doc::{
    BlendMode, ColorMode, Document, Layer, LayerMask, PixelFormat, Rect, SampleEncoding, SampleType, Size, SourceColorContext, WorkingProfileInterpretation,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{ItemRef, MaskId},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn read(root: &Path, blob: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(bytes.len(), blob["bytes"].as_u64().unwrap() as usize);
    assert_eq!(hash(&bytes), blob["sha256"].as_str().unwrap());
    bytes
}
fn native(bytes: &[u8], wide: bool) -> Vec<u8> {
    if wide {
        bytes.as_chunks::<8>().0.iter().flat_map(|p| f64::from_le_bytes(*p).to_ne_bytes()).collect()
    } else {
        bytes.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect()
    }
}
fn bounds(row: &Value) -> Rect {
    Rect::from_xywh(
        row["offset"][0].as_i64().unwrap() as i32,
        row["offset"][1].as_i64().unwrap() as i32,
        row["size"][0].as_u64().unwrap() as u32,
        row["size"][1].as_u64().unwrap() as u32,
    )
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn independently_table_trc_normal_operator_mask_matrix_matches_prepared_consumer() {
    verify_table_matrix(TableComparison::MatchedDisplay);
}
#[test]
#[ignore = "requires reviewed unpublished table-profile diagnostic corpus"]
#[should_panic(expected = "assigned native projection")]
fn original_profile_code_identity_is_not_matched_display_identity() {
    verify_table_matrix(TableComparison::SourceCodeDiagnostic);
}
#[test]
#[ignore = "requires reviewed unpublished table-profile diagnostic corpus"]
#[should_panic(expected = "tight matched table display")]
fn prior_tight_display_bound_remains_an_explicit_negative() {
    verify_table_matrix(TableComparison::TightDisplayDiagnostic);
}
#[derive(Clone, Copy)]
enum TableComparison {
    MatchedDisplay,
    SourceCodeDiagnostic,
    TightDisplayDiagnostic,
}
fn verify_table_matrix(comparison: TableComparison) {
    let display_probe = !matches!(comparison, TableComparison::SourceCodeDiagnostic);
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("normal-mask-table-trc-evidence");
    let supplement = root.parent().unwrap().join("normal-mask-table-trc-consumer-acceptance");
    let acceptance_manifest = fs::read(supplement.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&acceptance_manifest), "8988ecaf645e7bbae5d92448569a0bdb42550cc76d0843b5b78605166dcba5d6");
    let acceptance: Value = serde_json::from_slice(&acceptance_manifest).unwrap();
    assert_eq!(acceptance["original_manifest_sha256"], "289f517f01787684e29f2ce82b42a1cee1a02e702d9dd6d808d4d3da845e9ecd");
    for artifact in acceptance["artifacts"].as_array().unwrap() {
        read(&supplement, artifact);
    }
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("gimp_normal_mask_table_trc_hashes.json")).unwrap();
    for (name, expected) in hashes.as_object().unwrap() {
        assert_eq!(hash(&fs::read(root.join(name)).unwrap()), expected.as_str().unwrap(), "frozen assigned evidence {name}");
    }
    assert_eq!(hash(&bytes), hashes["observations.json"]);
    for name in ["fresh/observations.json", "faithful-fresh/faithful-observations.json", "VERIFICATION.json", "batch-manifest.json"] {
        assert_eq!(hash(&fs::read(root.join(name)).unwrap()), hashes[name]);
    }
    let data: Value = serde_json::from_slice(&bytes).unwrap();
    let faithful_bytes = fs::read(root.join("faithful-observations.json")).unwrap();
    assert_eq!(hash(&faithful_bytes), hashes["faithful-observations.json"]);
    let faithful: Value = serde_json::from_slice(&faithful_bytes).unwrap();
    let mut count = 0;
    for row in data["records"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let gray = row["model"] == "gray";
        let wide = id.contains("f64");
        let model = if gray { ColorMode::Grayscale } else { ColorMode::Rgb };
        let sample = if wide { SampleType::F64 } else { SampleType::F32 };
        assert!(!row["assigned_icc"].is_null());
        assert_eq!(row["canvas"], serde_json::json!([16, 12]));
        assert_eq!(row["precision"], if wide { "GIMP_PRECISION_DOUBLE_NON_LINEAR" } else { "GIMP_PRECISION_FLOAT_NON_LINEAR" });
        let file = fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&file), row["source_sha256"]);
        assert_eq!(&file[..14], b"gimp xcf v012\0");
        let mut doc = Document::new(id, Size::new(16, 12), model, sample);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let assigned = read(&root, &row["assigned_icc"]);
        let original_profile = fs::read(root.join(row["authored_icc"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&original_profile), row["authored_icc"]["sha256"]);
        assert_eq!(assigned, original_profile, "exact own profile survives assignment/save/reopen");
        let parsed = photocraft_cms::Profile::parse(&assigned).unwrap();
        if gray {
            assert!(parsed.trc.is_none(), "Gray profiles must not inherit extraneous RGB transfer tags");
            assert!((parsed.gray_trc.as_ref().unwrap().eval64(0.5) - 0.5_f64.powf(1.8)).abs() < 0.00002);
        } else {
            let curves = parsed.trc.as_ref().unwrap();
            let values = curves.each_ref().map(|curve| curve.eval64(0.5));
            assert!(values[0] > values[1] && values[1] > values[2], "unequal RGB curves must remain independent");
        }
        assert_eq!(parsed.channels(), if gray { 1 } else { 3 });
        assert_eq!(row["transfer_epoch"], "own-assigned-table-trc-nonlinear-native-v1");
        doc.icc_profile = Some(std::sync::Arc::new(assigned.clone()));
        doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
        doc.source_color_context =
            Some(SourceColorContext { original_icc: Some(std::sync::Arc::new(assigned)), source_precision: if wide { 750 } else { 650 } });
        let mut state = XcfState::default();
        state.provenance.format_version = 12;
        assert_eq!(row["layers"].as_array().unwrap().len(), 2);
        for (i, l) in row["layers"].as_array().unwrap().iter().enumerate().rev() {
            assert_eq!(l["mode"], 28);
            assert_eq!(l["byte_order"], "little");
            assert_eq!(l["visible"], true);
            let rect = bounds(l);
            let raw = read(&root, &l["native"]);
            let ne = native(&raw, wide);
            let mut layer = Layer::raster(l["name"].as_str().unwrap(), doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), rect, &ne).unwrap();
            layer.opacity = (l["opacity"].as_f64().unwrap() / 100.) as f32;
            layer.blend = BlendMode::Normal;
            let op = if i == 0 { l["spaces"]["composite_mode"].as_i64().unwrap() as i32 } else { 1 };
            let operator = match op {
                1 => GimpOperator::Union,
                2 => GimpOperator::ClipToBackdrop,
                3 => GimpOperator::ClipToLayer,
                4 => GimpOperator::Intersection,
                _ => panic!("unknown authored operator"),
            };
            if i == 0 {
                assert_eq!(l["spaces"]["blend_space"], 0);
                assert_eq!(l["spaces"]["composite_space"], 1);
                assert_eq!(l["size"], serde_json::json!([10, 9]));
                let m = &l["mask"];
                assert_eq!(m["offset"], l["offset"]);
                assert_eq!(m["size"], l["size"]);
                assert_eq!(m["buffer_extent"], serde_json::json!([0, 0, 10, 9]));
                assert_eq!(m["owner_apply"], true);
                assert_eq!(m["owner_show"], false);
                assert_eq!(m["owner_edit"], false);
                // Measure outside behavior from the public buffer's expanded read. The
                // canonical world bounds derive from actual public mask offsets, not owner inference.
                let halo = read(&root, &m["halo_none"]);
                assert_eq!(halo.len(), 12 * 11 * 8);
                for y in 0..11 {
                    for x in 0..12 {
                        if x == 0 || x == 11 || y == 0 || y == 10 {
                            assert_eq!(f64::from_le_bytes(halo[(y * 12 + x) * 8..(y * 12 + x) * 8 + 8].try_into().unwrap()), 0.0);
                        }
                    }
                }
                let maskraw = read(&root, &m["native"]);
                let maskrect = bounds(m);
                let surface = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, sample, false), maskrect, &native(&maskraw, wide)).unwrap();
                layer.mask = Some(LayerMask { surface: std::sync::Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
                state.items.push(ItemState {
                    item: ItemRef::Mask(MaskId(900)),
                    name: m["name"].as_str().unwrap().into(),
                    custom_name: false,
                    visible: true,
                    locks: ItemLocks::default(),
                    parent: None,
                    source_tattoo: None,
                    source_flags: 0,
                    raster_bounds: None,
                    composition: None,
                    mask: Some(MaskState {
                        owner: ItemRef::Layer(layer.id),
                        bounds: maskrect,
                        offset: [0, 0],
                        outside_f64_bits: 0.0_f64.to_bits(),
                        enabled: true,
                        shown: false,
                        editing: false,
                        linked: true,
                    }),
                });
            } else {
                assert!(l["mask"].is_null());
                assert_eq!(l["size"], serde_json::json!([16, 12]));
                assert_eq!(l["offset"], serde_json::json!([0, 0]));
            }
            state.items.push(ItemState {
                item: ItemRef::Layer(layer.id),
                name: layer.name.clone(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(rect),
                mask: None,
                composition: Some(GimpComposition {
                    compatibility_epoch: "normal-mask-observation".into(),
                    source_mode: 28,
                    legacy: false,
                    stored_blend_space: 0,
                    stored_composite_space: if i == 0 { 1 } else { -1 },
                    stored_operator: if i == 0 { op } else { -1 },
                    pass_through: false,
                    resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator }),
                }),
            });
            doc.layers.push(layer);
        }
        doc.xcf_state = Some(state);
        doc.validate_xcf_state().unwrap();
        let before = doc.clone();
        let actual_raw = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let native_projection = read(&root, &row["projection"]["native"]);
        let channels = if gray { 1 } else { 3 };
        let count_channels = channels + 1;
        let native_values: Vec<f64> = if wide {
            native_projection.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect()
        } else {
            native_projection.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect()
        };
        let mut native_error = 0.0_f64;
        for (p, reference) in actual_raw.px.iter().zip(native_values.chunks_exact(count_channels)) {
            let alpha = reference[count_channels - 1];
            assert!((p[3] - alpha).abs() <= 1.0 / 65535.0, "table native alpha {id}");
            native_error = native_error.max((p[3] - alpha).abs());
            for c in 0..channels {
                if alpha > 0.0 {
                    native_error = native_error.max((p[c] - reference[c]).abs());
                }
                native_error = native_error.max((p[c] * p[3] - reference[c] * alpha).abs());
            }
        }
        if native_error > 1.0 / 65535.0 {
            println!("table profile channels {}", parsed.channels());
            for (i, (p, r)) in actual_raw.px.iter().zip(native_values.chunks_exact(count_channels)).enumerate() {
                if r[count_channels - 1] > 0.0 && (0..channels).any(|channel| (p[channel] - r[channel]).abs() > 1.0 / 65535.0) {
                    println!("mismatch ({},{}) actual {:?} native {:?}", i % 16, i / 16, p, r);
                    if i > 25 {
                        break;
                    }
                }
            }
        }
        // Assigned-profile domain uses spec SC-002 matched-profile display <=2/255.
        // Exact source/mask/ICC checks stay unchanged; public roundtrip controls retain the tighter discrepancy.
        println!("{id}: assigned-code native visible/premult {}/255", native_error * 255.0);
        if !display_probe {
            assert!(native_error <= 2.0 / 255.0, "assigned native projection {id}: {native_error}");
        }
        let target = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let transform = photocraft_cms::Transform::new(&parsed, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
        let mut actual = actual_raw.clone();
        for p in &mut actual.px {
            let mut converted = [0.0; 3];
            transform.try_eval_f64(&p[..channels], &mut converted[..channels]).unwrap();
            p[..channels].copy_from_slice(&converted[..channels]);
        }

        let reference = read(&root, &row["projection"]["nonlinear_double"]);
        let n = if gray { 2 } else { 4 };
        let mut error = 0.0_f64;
        let mut hidden = 0.0_f64;
        for (p, raw) in actual.px.iter().zip(reference.chunks_exact(n * 8)) {
            let a = f64::from_le_bytes(raw[(n - 1) * 8..n * 8].try_into().unwrap());
            assert!((p[3] - a).abs() <= 1.0 / 65535.0, "table display alpha {id}");
            error = error.max((p[3] - a).abs());
            for c in 0..if gray { 1 } else { 3 } {
                let v = f64::from_le_bytes(raw[c * 8..c * 8 + 8].try_into().unwrap());
                if a > 0. {
                    error = error.max((p[c] - v).abs());
                } else {
                    hidden = hidden.max((p[c] - v).abs());
                }
                error = error.max((p[c] * p[3] - v * a).abs());
            }
        }
        println!("{id}: native visible/premult {}/255; hidden alpha0 {}/255", error * 255., hidden * 255.);
        if matches!(comparison, TableComparison::TightDisplayDiagnostic) {
            assert!(error <= 1.0 / 65535.0, "tight matched table display {id}: {error}");
        } else {
            assert!(error <= 2.0 / 255.0, "{id}native {error}");
        }
        let faithful_row = faithful["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
        let png = photocraft_codecs::decode(&read(&root, &faithful_row["reference"])).unwrap();
        assert_eq!(png.icc.as_ref().unwrap().as_slice(), original_profile.as_slice(), "exported PNG retains exact assigned gamma profile");
        let mut png_error = 0.0_f64;
        let mut png_hidden = 0.0_f64;
        for y in 0..12 {
            for x in 0..16 {
                let p = if display_probe { actual.px[(y * 16 + x) as usize] } else { actual_raw.px[(y * 16 + x) as usize] };
                let mut reference_color = [0.0; 3];
                let mut encoded_color = [0.0; 3];
                for (channel, value) in encoded_color.iter_mut().enumerate().take(channels) {
                    *value = f64::from(png.get(x, y, channel));
                }
                if display_probe {
                    transform.try_eval_f64(&encoded_color[..channels], &mut reference_color[..channels]).unwrap();
                }
                for c in 0..n {
                    let v = if c == n - 1 { p[3] } else { p[c] };
                    let reference_value = if display_probe && c != n - 1 { reference_color[c] } else { f64::from(png.get(x, y, c)) };
                    let reference_alpha = f64::from(png.get(x, y, n - 1));
                    if c == n - 1 {
                        assert!((v - reference_value).abs() <= 1.0 / 65535.0, "table PNG alpha {id}");
                    }
                    if c == n - 1 || reference_alpha > 0.0 {
                        png_error = png_error.max((v - reference_value).abs());
                    } else {
                        png_hidden = png_hidden.max((v - reference_value).abs());
                    }
                    if c != n - 1 {
                        png_error = png_error.max((v * p[3] - reference_value * reference_alpha).abs());
                    }
                }
            }
        }
        println!("{id}: faithful visible/premult PNG {}/255; hidden alpha0 {}/255", png_error * 255., png_hidden * 255.);
        assert!(png_error <= 2. / 255., "{id}PNG {png_error}");
        assert!(PreparedRenderer::new_with_interrupt(&doc, &Interrupt::cancel_only(&|| true)).is_err());
        assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied work"))).is_err());
        let mut corrupt_profile = doc.clone();
        corrupt_profile.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3]));
        assert!(try_render(&corrupt_profile, corrupt_profile.bounds(), &mut |_| Ok(())).is_err(), "no corrupt-profile fallback");
        let mut wrong_model = doc.clone();
        wrong_model.icc_profile =
            Some(if gray { photocraft_cms::Builtin::Srgb.profile().to_bytes() } else { photocraft_cms::Builtin::SGray.profile().to_bytes() });
        assert!(try_render(&wrong_model, wrong_model.bounds(), &mut |_| Ok(())).is_err(), "no wrong-model profile fallback");
        let mut invalid_table = parsed.clone();
        let invalid_curve = photocraft_cms::Curve::Table64(vec![0.0, 1.0, 0.0]);
        if gray {
            invalid_table.gray_trc = Some(invalid_curve);
        } else {
            invalid_table.trc = Some([invalid_curve.clone(), invalid_curve.clone(), invalid_curve]);
        }
        let mut invalid_current = doc.clone();
        invalid_current.icc_profile = Some(invalid_table.with_encoded_bytes().to_bytes());
        assert!(try_render(&invalid_current, invalid_current.bounds(), &mut |_| Ok(())).is_err(), "invalid current table has no source-profile fallback");
        let mut unmasked = doc.clone();
        unmasked.layers.iter_mut().find_map(|layer| layer.mask.as_mut()).unwrap().enabled = false;
        unmasked.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|item| item.mask.as_mut()).unwrap().enabled = false;
        let other = try_render(&unmasked, unmasked.bounds(), &mut |_| Ok(())).unwrap();
        assert!(
            actual_raw.px.iter().zip(&other.px).any(|(a, b)| (a[3] - b[3]).abs() > 1.0 / 255.0 || (a[0] * a[3] - b[0] * b[3]).abs() > 1.0 / 255.0),
            "mask materially changes coverage"
        );
        let mut calls = 0;
        assert!(
            try_render(&doc, doc.bounds(), &mut |_| {
                calls += 1;
                if calls > 8 { Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancel mid render")) } else { Ok(()) }
            })
            .is_err()
        );
        assert_eq!(doc, before);
        count += 1;
    }
    assert_eq!(count, 32);
}

#[test]
#[ignore = "24 MP release Normal/mask kernel measurement; run explicitly"]
fn normal_mask_operator_kernels_24mp() {
    use photocraft_compose::gimp_composite::WorkingSpace;
    use std::{hint::black_box, time::Instant};
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("normal-mask-table-trc-evidence");
    for (name, file) in [("RGB", "own-rgb-table-trc.icc"), ("Gray", "own-gray-table-trc.icc")] {
        let profile = photocraft_cms::Profile::parse(&fs::read(root.join(file)).unwrap()).unwrap();
        let space = WorkingSpace::new(&profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let backdrop = [0.19, 0.47, 0.81, 0.375];
        let source = [0.81, 0.02, 0.47, 0.75];
        for operator in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
            let start = Instant::now();
            for _ in 0..24_000_000 {
                black_box(space.normal(black_box(backdrop), black_box(source), black_box(0.7125 * 0.375), operator).unwrap());
            }
            println!("24 MP {name} Normal/mask {operator:?}: {:?}", start.elapsed());
        }
    }
}
