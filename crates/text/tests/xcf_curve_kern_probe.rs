//! Private independent source comparison; no XCF admission or renderer changes.
use photocraft_color::{Color, PixelFormat};
use photocraft_doc::{TextLayer, text::AntiAlias};
use photocraft_geom::{Affine, Rect};
use sha2::{Digest, Sha256};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
};
#[test]
#[ignore = "private exact-font curve/multicontour/kerning holdout; expected diagnostic failure"]
fn ordinary_text_pipeline_independent_curve_kern_controls() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-evidence");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let repeat: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let mut report = Vec::new();
    let mut worst = 0u8;
    for (row, second) in data["rows"].as_array().unwrap().iter().zip(repeat["rows"].as_array().unwrap()) {
        assert_eq!(row["source_sha256"], second["source_sha256"]);
        assert_eq!(row["state"]["native"]["sha256"], second["state"]["native"]["sha256"]);
        assert_eq!(row["state"]["projection"]["sha256"], second["state"]["projection"]["sha256"]);
        let case = row["case"].as_str().unwrap();
        let text = row["text"].as_str().unwrap();
        let bytes = std::fs::read(base.join(format!("{case}.xcf"))).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), row["source_sha256"].as_str().unwrap());
        assert!(bytes.windows(64).any(|b| b == b"40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"));
        let size = row["size_px"].as_f64().unwrap() as f32;
        let ascent = face.metrics(Size::new(size), LocationRef::default()).ascent.ceil();
        let aa = if row["antialias"].as_bool().unwrap() { AntiAlias::Smooth } else { AntiAlias::None };
        let layer = TextLayer {
            text: text.into(),
            font_family: "Inter".into(),
            size_pt: size,
            color: Color::WHITE,
            transform: Affine::translate(8., 4. + f64::from(ascent)),
            antialias: aa,
            ..Default::default()
        };
        let layout = photocraft_text::TextEngine::new().layout(&layer, 72.);
        assert!(layout.faces.iter().all(|f| f.font.data.as_ref() == photocraft_text::fonts::INTER_REGULAR.as_slice()));
        let reference = std::fs::read(base.join(format!("{case}.projection.bin"))).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&reference)), row["state"]["projection"]["sha256"].as_str().unwrap());
        for rounded in [false, true] {
            let mut current = layout.clone();
            if rounded {
                let mut previous = 0.;
                let mut x = 0.;
                for g in &mut current.glyphs {
                    let old = g.x;
                    x += (old - previous).round();
                    g.x = x;
                    previous = old;
                }
            }
            let rendered = photocraft_text::render::rasterize(&current, &layer.transform, PixelFormat::RGBA8, aa);
            let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 256, 128)).unwrap();
            let alpha = actual.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()).map(|(a, b)| a[3].abs_diff(b[3])).max().unwrap();
            let over = actual.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()).filter(|(a, b)| a[3].abs_diff(b[3]) > 2).count();
            let glyphs = current.glyphs.iter().map(|g| serde_json::json!({"id":g.id,"x":g.x,"y":g.y})).collect::<Vec<_>>();
            report.push(serde_json::json!({"case":case,"rounded_advances":rounded,"glyphs":glyphs,"alpha_max":alpha,"beyond2":over,"rendered_bounds":format!("{:?}",rendered.rect),"gimp_cache_size":row["state"]["size"]}));
            println!("{case} rounded={rounded}: alpha{alpha}/255 beyond2={over}");
            worst = worst.max(alpha);
        }
    }
    std::fs::write(base.join("ordinary-render-comparison.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(worst <= 2, "ordinary candidate exceeds unchanged bound {worst}");
}
#[test]
fn current_semantic_guard_refuses_curves_and_multicontours_without_fallback() {
    let ctl = photocraft_raster::Interrupt::NONE;
    let font = photocraft_text::xcf::ExactFont::bundled_inter(&ctl).unwrap();
    for text in ["O", "S", "B", "HELLO", "GIMP", "AV", "To", "ffi"] {
        assert!(
            matches!(
                photocraft_text::xcf::qualify_scaled_rectilinear_geometry(
                    &font,
                    photocraft_text::xcf::ScaledText { content: text, origin: [8, 4], size_px: 32. },
                    &ctl
                ),
                Err(photocraft_text::xcf::TextSourceError::Unsupported(_))
            ),
            "{text}"
        );
    }
}
#[test]
#[ignore = "requires frozen independent curve/kerning captures; metadata only, no renderer admission"]
fn independent_original_cache_and_serialized_layout_are_exact() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-evidence");
    let manifest = std::fs::read(base.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "346c44a055c79a327db247f8df2019cef2e00d6a6462bcbb5f55fda0c5960c22");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(base.join(name)).unwrap())), hash);
    }
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let mut reports = Vec::new();
    for row in data["rows"].as_array().unwrap() {
        let case = row["case"].as_str().unwrap();
        let original = std::fs::read(base.join(format!("{case}.xcf"))).unwrap();
        let file = photocraft_xcf::decode(&original, &Default::default()).unwrap();
        let d = &file.layers[0];
        let text = d.text.as_ref().unwrap();
        let cache = d.plane.as_ref().unwrap();
        assert_eq!(file.version, 19);
        assert_eq!(file.precision, 150);
        assert_eq!((d.width, d.height), (row["state"]["size"][0].as_u64().unwrap() as u32, row["state"]["size"][1].as_u64().unwrap() as u32));
        assert_eq!(d.offset, (8, 4));
        assert_eq!(cache.bytes.as_slice(), std::fs::read(base.join(row["state"]["native"]["file"].as_str().unwrap())).unwrap().as_slice());
        assert_eq!(text.content, row["text"].as_str().unwrap());
        assert_eq!(text.runs.len(), 1);
        assert_eq!((text.runs[0].start, text.runs[0].end), (0, text.content.len() as u64));
        assert!(text.runs[0].style_ids.is_empty());
        assert!(text.styles.is_empty());
        assert_eq!(text.font.as_ref().unwrap().hash.as_deref(), Some("40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"));
        assert_eq!(text.layout.offset_x_bits.unwrap_or(0), 0);
        assert_eq!(text.layout.offset_y_bits.unwrap_or(0), 0);
        assert_eq!(text.layout.transformation_bits, None);
        reports.push(serde_json::json!({"case":case,"source_sha256":format!("{:x}",Sha256::digest(&original)),"raw_text_sha256":format!("{:x}",Sha256::digest(&text.raw)),"source_flags":d.text_flags,"parasite_flags":text.parasite_flags,"layout":format!("{:?}",text.layout),"raw_configuration":format!("{:?}",text.configuration),"font":format!("{:?}",text.font),"runs":format!("{:?}",text.runs),"source_cache_native_exact":true}));
        assert_eq!(original, std::fs::read(base.join(format!("{case}.xcf"))).unwrap());
    }
    assert_eq!(reports.len(), 48);
    std::fs::write(base.join("serialized-layout-audit.json"), serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
}
