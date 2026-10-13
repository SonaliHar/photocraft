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
fn native(bytes: &[u8], sample: SampleType, half: bool) -> Vec<u8> {
    if half {
        return bytes.as_chunks::<2>().0.iter().flat_map(|p| photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(*p)).to_ne_bytes()).collect();
    }
    match sample {
        SampleType::U8 => bytes.to_vec(),
        SampleType::U16 => bytes.as_chunks::<2>().0.iter().flat_map(|p| u16::from_le_bytes(*p).to_ne_bytes()).collect(),
        SampleType::U32 => bytes.as_chunks::<4>().0.iter().flat_map(|p| u32::from_le_bytes(*p).to_ne_bytes()).collect(),
        SampleType::F32 => bytes.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect(),
        SampleType::F64 => bytes.as_chunks::<8>().0.iter().flat_map(|p| f64::from_le_bytes(*p).to_ne_bytes()).collect(),
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
fn independently_authored_perceptual_depth_normal_operator_mask_matrix_matches_prepared_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("normal-mask-perceptual-depth-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("gimp_normal_mask_perceptual_depth_hashes.json")).unwrap();
    assert_eq!(hash(&bytes), hashes["observations.json"]);
    for name in ["fresh/observations.json", "faithful-fresh/faithful-observations.json", "VERIFICATION.json", "batch-manifest.json"] {
        assert_eq!(hash(&fs::read(root.join(name)).unwrap()), hashes[name]);
    }
    let data: Value = serde_json::from_slice(&bytes).unwrap();
    let faithful_bytes = fs::read(root.join("faithful-observations.json")).unwrap();
    assert_eq!(hash(&faithful_bytes), hashes["faithful-observations.json"]);
    let faithful: Value = serde_json::from_slice(&faithful_bytes).unwrap();
    let mut count = 0;
    let mut quantization_diagnostics = 0;
    for row in data["records"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let gray = row["model"] == "gray";
        assert_eq!(row["sample_encoding"], "perceptual");
        let linear = false;
        let model = if gray { ColorMode::Grayscale } else { ColorMode::Rgb };
        let half = row["sample_type"] == "half";
        let sample = match row["sample_type"].as_str().unwrap() {
            "u8" => SampleType::U8,
            "half" => SampleType::F32,
            "u32" => SampleType::U32,
            _ => panic!("unknown perceptual depth"),
        };
        assert!(row["assigned_icc"].is_null());
        let public_profile = photocraft_cms::Profile::parse(&read(&root, &row["effective_icc"])).unwrap();
        assert_eq!(public_profile.color_space, if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb });
        assert_eq!(row["canvas"], serde_json::json!([16, 12]));
        assert_eq!(row["precision"], format!("GIMP_PRECISION_{}_{}", row["sample_type"].as_str().unwrap().to_uppercase(), "PERCEPTUAL"));
        let file = fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&file), row["source_sha256"]);
        assert_eq!(&file[..10], b"gimp xcf v");
        let version = std::str::from_utf8(&file[10..13]).unwrap().parse::<u32>().unwrap();
        let precision = u32::from_be_bytes(file[26..30].try_into().unwrap());
        assert_eq!(u64::from(precision), row["source_precision_code"].as_u64().unwrap());
        let mut doc = Document::new(id, Size::new(16, 12), model, sample);
        doc.sample_encoding = Some(SampleEncoding::Perceptual);
        assert_eq!(row["transfer_epoch"], format!("own-perceptual-depth-{}-native-v1", row["sample_type"].as_str().unwrap()));
        doc.icc_profile = Some(
            photocraft_cms::xcf_color::effective_profile(
                None,
                if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
                photocraft_cms::xcf_color::Encoding::Perceptual,
            )
            .unwrap()
            .to_bytes(),
        );
        doc.working_profile_interpretation = if gray {
            WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
        } else {
            WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
        };
        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: precision });
        let mut state = XcfState::default();
        state.provenance.format_version = version;
        assert_eq!(row["layers"].as_array().unwrap().len(), 2);
        for (i, l) in row["layers"].as_array().unwrap().iter().enumerate().rev() {
            assert_eq!(l["mode"], 28);
            assert_eq!(l["byte_order"], "little");
            assert_eq!(l["visible"], true);
            let rect = bounds(l);
            let raw = read(&root, &l["native"]);
            let ne = native(&raw, sample, half);
            // Compare the declared transfer against independent public buffer reads.
            assert!(l["native_format"].as_str().unwrap().contains('~'));
            let observed = read(&root, &l["nonlinear_double"]);
            let channels = if gray { 1 } else { 3 };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let mut transfer_error = 0.0_f64;
            let mut wrong_linear = 0.0_f64;
            let mut wrong_gamma = 0.0_f64;
            for (native_pixel, public_pixel) in ne.chunks_exact((channels + 1) * sample.bytes()).zip(observed.chunks_exact((channels + 1) * 8)) {
                let mut values = [0.0; 3];
                let mut predicted = [0.0; 3];
                for (c, value) in values.iter_mut().take(channels).enumerate() {
                    *value = photocraft_color::try_read_sample_f64(native_pixel, sample, c).unwrap();
                }
                photocraft_cms::xcf_color::convert_encoding_f64(
                    profile,
                    photocraft_cms::xcf_color::Encoding::Perceptual,
                    photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    &values[..channels],
                    &mut predicted[..channels],
                )
                .unwrap();
                for c in 0..channels {
                    let actual = f64::from_le_bytes(public_pixel[c * 8..c * 8 + 8].try_into().unwrap());
                    transfer_error = transfer_error.max((predicted[c] - actual).abs());
                    let encoded = |v: f64| if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                    wrong_linear = wrong_linear.max((encoded(values[c]) - actual).abs());
                    wrong_gamma = wrong_gamma.max((encoded(values[c].powf(2.2)) - actual).abs());
                }
            }
            assert!(transfer_error <= 1e-6, "{id} public perceptual transfer {transfer_error}");
            assert!(wrong_linear > 0.1 && wrong_gamma > 0.001, "literal tones distinguish transfer hypotheses");

            let mut layer = Layer::raster(l["name"].as_str().unwrap(), doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), rect, &ne).unwrap();
            assert_eq!(layer.surface().unwrap().try_read_interleaved(rect).unwrap(), ne);
            if sample == SampleType::U32 {
                assert!(
                    raw.as_chunks::<4>().0.iter().any(|v| u32::from_le_bytes(*v) == 2_147_483_648),
                    "source retains half-range plus one integer code"
                );
            }
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
                let surface =
                    Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, sample, false), maskrect, &native(&maskraw, sample, half)).unwrap();
                assert_eq!(surface.try_read_interleaved(maskrect).unwrap(), native(&maskraw, sample, half));
                if sample == SampleType::U32 {
                    assert!(
                        maskraw.as_chunks::<4>().0.iter().any(|v| u32::from_le_bytes(*v) == 1_610_612_736),
                        "mask retains three-eighths plus one integer code"
                    );
                }
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
        let mut actual = actual_raw.clone();
        let channels = if gray { 1 } else { 3 };
        let source_profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        if linear {
            for pixel in &mut actual.px {
                let mut encoded = [0.0; 3];
                photocraft_cms::xcf_color::convert_encoding_f64(
                    source_profile,
                    photocraft_cms::xcf_color::Encoding::Linear,
                    photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    &pixel[..channels],
                    &mut encoded[..channels],
                )
                .unwrap();
                pixel[..channels].copy_from_slice(&encoded[..channels]);
            }
        }
        // Native projection is independently observed; source planes remain exact.
        let projection = read(&root, &row["projection"]["native"]);
        let quantum = if half { 2 } else { sample.bytes() };
        let native_bound = if half {
            2.0_f64.powi(-12) + 1.0 / 65535.0
        } else if sample == SampleType::U8 {
            2.0 / 255.0
        } else {
            1.0 / 65535.0
        };
        let display_bound = if half || sample == SampleType::U8 { 2.0 / 255.0 } else { 1.0 / 65535.0 };
        let maximum = match sample {
            SampleType::U8 => 255.,
            SampleType::U16 => 65535.,
            SampleType::U32 => 4294967295.,
            SampleType::F32 | SampleType::F64 => 1.,
        };
        let decoded: Vec<f64> = projection.chunks_exact(quantum).map(|v| match sample {
            SampleType::U8 => f64::from(v[0]),
            SampleType::U16 => f64::from(u16::from_le_bytes(v.try_into().unwrap())),
            SampleType::U32 => f64::from(u32::from_le_bytes(v.try_into().unwrap())),
            SampleType::F32 => if half { f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(v.try_into().unwrap())))) } else { f64::from(f32::from_le_bytes(v.try_into().unwrap())) },
            SampleType::F64 => f64::from_le_bytes(v.try_into().unwrap()),
        } / maximum).collect();
        let mut coded_error = 0.0_f64;
        for (pixel, reference) in actual_raw.px.iter().zip(decoded.chunks_exact(if gray { 2 } else { 4 })) {
            let alpha = *reference.last().unwrap();
            coded_error = coded_error.max((pixel[3] - alpha).abs());
            if alpha > 0. {
                for channel in 0..channels {
                    coded_error = coded_error.max((pixel[channel] - reference[channel]).abs());
                }
            }
        }
        println!("{id}: derived native projection {}/255", coded_error * 255.);
        quantization_diagnostics += usize::from(coded_error > 1.0 / 65535.0);
        assert!(coded_error <= native_bound, "{id} native projection {coded_error}");
        let reference = read(&root, &row["projection"]["nonlinear_double"]);
        let n = if gray { 2 } else { 4 };
        let mut error = 0.0_f64;
        let mut hidden = 0.0_f64;
        for (p, raw) in actual.px.iter().zip(reference.chunks_exact(n * 8)) {
            let a = f64::from_le_bytes(raw[(n - 1) * 8..n * 8].try_into().unwrap());
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
        assert!(error <= display_bound, "{id}native {error}");
        let faithful_row = faithful["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
        let png = photocraft_codecs::decode(&read(&root, &faithful_row["reference"])).unwrap();
        let png_profile = photocraft_cms::Profile::parse(png.icc.as_ref().unwrap()).unwrap();
        let curve_is_linear = |curve: &photocraft_cms::curve::Curve| (curve.eval64(0.375) - 0.375).abs() < 1e-9;
        if linear && gray {
            assert!(curve_is_linear(png_profile.gray_trc.as_ref().unwrap()));
        } else if linear {
            assert!(png_profile.trc.as_ref().unwrap().iter().all(curve_is_linear));
        }
        let mut png_error = 0.0_f64;
        let mut png_hidden = 0.0_f64;
        for y in 0..12 {
            for x in 0..16 {
                let p = actual_raw.px[(y * 16 + x) as usize];
                for c in 0..n {
                    let v = if c == n - 1 { p[3] } else { p[c] };
                    let reference_value = f64::from(png.get(x, y, c));
                    let reference_alpha = f64::from(png.get(x, y, n - 1));
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
        assert!(png_error <= display_bound, "{id}PNG {png_error}");
        assert!(PreparedRenderer::new_with_interrupt(&doc, &Interrupt::cancel_only(&|| true)).is_err());
        assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied work"))).is_err());
        let mut stale_transfer = doc.clone();
        stale_transfer.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        assert!(try_render(&stale_transfer, stale_transfer.bounds(), &mut |_| Ok(())).is_err(), "implicit profile must stay bound to actual transfer encoding");
        // Explicit assignment changes active interpretation, never immutable provenance.
        let mut assigned = doc.clone();
        assigned.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        assigned.working_profile_interpretation = WorkingProfileInterpretation::Icc;
        let same_bytes = try_render(&assigned, assigned.bounds(), &mut |_| Ok(())).unwrap();
        assert!(actual_raw.px.iter().zip(&same_bytes.px).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 2.0 / 255.0)));
        assigned.icc_profile = Some(
            photocraft_cms::xcf_color::effective_profile(
                None,
                if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
                photocraft_cms::xcf_color::Encoding::Linear,
            )
            .unwrap()
            .to_bytes(),
        );
        let active_linear = try_render(&assigned, assigned.bounds(), &mut |_| Ok(())).unwrap();
        if id.contains("op1-") || id.contains("op2-") {
            assert!(
                actual_raw.px.iter().zip(&active_linear.px).any(|(a, b)| (a[0] * a[3] - b[0] * b[3]).abs() > 0.01),
                "current assigned profile governs composition despite retained original provenance"
            );
        }
        assert_eq!(assigned.source_color_context, doc.source_color_context);
        assert_eq!(assigned.layers, doc.layers);
        if sample == SampleType::F64 {
            // Canonical current transfer conversion keeps provenance and mask coverage.
            let mut converted = doc.clone();
            let encoding = photocraft_cms::xcf_color::Encoding::Linear;
            let current_profile = photocraft_cms::xcf_color::effective_profile(
                None,
                if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
                encoding,
            )
            .unwrap();
            for layer in &mut converted.layers {
                let rect = before.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).unwrap().raster_bounds.unwrap();
                let mut bytes = layer.surface().unwrap().try_read_interleaved(rect).unwrap();
                for pixel in bytes.chunks_exact_mut((channels + 1) * 8) {
                    let mut input = [0.0; 3];
                    let mut output = [0.0; 3];
                    for (c, v) in input.iter_mut().take(channels).enumerate() {
                        *v = f64::from_ne_bytes(pixel[c * 8..c * 8 + 8].try_into().unwrap());
                    }
                    photocraft_cms::xcf_color::convert_encoding_f64(
                        source_profile,
                        photocraft_cms::xcf_color::Encoding::Perceptual,
                        encoding,
                        &input[..channels],
                        &mut output[..channels],
                    )
                    .unwrap();
                    for (c, v) in output.iter().take(channels).enumerate() {
                        pixel[c * 8..c * 8 + 8].copy_from_slice(&v.to_ne_bytes());
                    }
                }
                *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), rect, &bytes).unwrap();
            }
            converted.sample_encoding = Some(SampleEncoding::Linear);
            converted.icc_profile = Some(current_profile.to_bytes());
            converted.working_profile_interpretation = if gray {
                WorkingProfileInterpretation::implicit_sgray(converted.icc_profile.as_ref().unwrap())
            } else {
                WorkingProfileInterpretation::implicit_srgb(converted.icc_profile.as_ref().unwrap())
            };
            let current = try_render(&converted, converted.bounds(), &mut |_| Ok(())).unwrap();
            for (a, b) in actual_raw.px.iter().zip(&current.px) {
                let mut encoded = [0.0; 3];
                photocraft_cms::xcf_color::convert_encoding_f64(
                    source_profile,
                    encoding,
                    photocraft_cms::xcf_color::Encoding::Perceptual,
                    &b[..channels],
                    &mut encoded[..channels],
                )
                .unwrap();
                assert!((a[3] - b[3]).abs() < 1e-12);
                if a[3] > 0.0 {
                    assert!(a[..channels].iter().zip(&encoded).all(|(x, y)| (x - y).abs() < 1e-10));
                }
            }
            assert_eq!(converted.source_color_context, doc.source_color_context);
            for (a, b) in converted.layers.iter().zip(&doc.layers) {
                assert_eq!(a.mask, b.mask);
            }
        }
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
    assert_eq!(count, 48);
    assert!(quantization_diagnostics > 0, "derived integer/half rounding is separate from implicit float1/65535 acceptance");
}
