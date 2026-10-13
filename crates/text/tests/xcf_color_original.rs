use photocraft_doc::{
    item_identity::{ItemRef, ResourceId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
    xcf_items::{EditableSource, SourceItem},
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use photocraft_text::xcf::ExactFont;
fn arg(name: &str, value: FilterValue) -> FilterArgument {
    FilterArgument { name: name.into(), value }
}

#[test]
#[ignore = "requires independently authored exact-font color originals"]
fn original_color_alpha_and_atomic_canonical_edit() {
    use photocraft_cms::{ColorSpace, Intent, Profile, Transform};
    use photocraft_text::xcf::{TextColorEncoding, qualify_colored_rectilinear_source, set_rectilinear_color};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-color-evidence");
    use sha2::{Digest, Sha256};
    let manifest = std::fs::read(base.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "deef5cdada65df4ffc05e9e2e84deaa75f1ed61cee11e71fd5111e26f52cc086");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(base.join(name)).unwrap())), hash);
    }
    for base in [base.clone(), base.join("alpha-only")] {
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
        let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
        for row in observations["rows"].as_array().unwrap() {
            let name = row["case"].as_str().unwrap();
            let bytes = std::fs::read(base.join(format!("{name}.xcf"))).unwrap();
            let decoded = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let (item, res) = canonical(decoded.layers[0].text.as_ref().unwrap());
            let (request, aa, color) = qualify_colored_rectilinear_source(&item, &res, [8, 4], &Interrupt::NONE).unwrap();
            assert_eq!(color.encoding, TextColorEncoding::Linear);
            let rgba = color.rgba();
            for (v, expected) in rgba.iter().zip(row["requested_rgba"].as_array().unwrap()) {
                assert_eq!(f64::from(*v), expected.as_f64().unwrap());
            }
            let profile = std::fs::read(base.join(row["state"]["effective_icc"]["file"].as_str().unwrap())).unwrap();
            let output = Profile::parse(&profile).unwrap();
            let input = photocraft_cms::xcf_color::effective_profile(Some(&profile), ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Linear).unwrap();
            let transform = Transform::new(&input, &output, Intent::RelativeColorimetric, false).unwrap();
            let mut encoded = rgba;
            transform.apply(&mut encoded, 4);
            assert!(encoded.iter().all(|v| v.is_finite()));
            // The explicit CMS caller performs the U8 output clipping boundary.
            encoded = encoded.map(|v| v.clamp(0., 1.));
            // Independently reopened GIMP text natives retain opaque fill even
            // when the stored color alpha is zero. This is not layer opacity.
            encoded[3] = 1.;
            let prepared = photocraft_text::xcf::prepare_scaled_rectilinear_with_charge(&font, request, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
            let rendered = prepared.render_color_with_charge(aa, encoded, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
            let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
            let expected = std::fs::read(base.join(format!("{name}.projection.bin"))).unwrap();
            assert_eq!(expected, std::fs::read(base.join(format!("round2/{name}.projection.bin"))).unwrap());
            let max = actual
                .as_chunks::<4>().0.iter()
                .zip(expected.as_chunks::<4>().0.iter())
                .map(|(a, b)| {
                    let alpha = a[3].abs_diff(b[3]);
                    let rgb = (0..3).map(|i| (f64::from(a[i]) * f64::from(a[3]) / 255. - f64::from(b[i]) * f64::from(b[3]) / 255.).abs()).fold(0., f64::max);
                    rgb.max(f64::from(alpha))
                })
                .fold(0., f64::max);
            assert!(max <= 2., "{name}: {max}/255");
            println!("{name}: alpha/premult max {max}/255");
            if row["phase"] == "source" {
                let edit_name = name.replace("-source", "-edit");
                let edit = photocraft_xcf::decode(&std::fs::read(base.join(format!("{edit_name}.xcf"))).unwrap(), &Default::default()).unwrap();
                let (expected, _) = canonical(edit.layers[0].text.as_ref().unwrap());
                let (_, _, replacement) = qualify_colored_rectilinear_source(&expected, &res, [8, 4], &Interrupt::NONE).unwrap();
                let mut changed = item.clone();
                assert!(set_rectilinear_color(&mut changed, &res, [8, 4], replacement, &font, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap());
                assert_eq!(changed.source, expected.source);
                assert_eq!(changed.cache, item.cache);
            }
        }
    }
}

#[test]
fn converted_color_validation_budget_and_cancellation() {
    use photocraft_text::xcf::{RectilinearAntialias, ScaledText, TextColor, TextColorEncoding, TextSourceError, prepare_scaled_rectilinear_with_charge};
    for value in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        assert!(TextColor::checked(TextColorEncoding::Linear, [value.to_bits(), 0, 0, 1_f32.to_bits()]).is_err());
    }
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let prepared =
        prepare_scaled_rectilinear_with_charge(&font, ScaledText { content: "HEFT", origin: [8, 4], size_px: 16. }, 8 << 20, &Interrupt::NONE, &mut |_| Ok(()))
            .unwrap();
    assert!(prepared.render_color_with_charge(RectilinearAntialias::AreaCoverage, [f32::NAN, 0., 0., 1.], 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).is_err());
    assert!(matches!(
        prepared.render_color_with_charge(RectilinearAntialias::AreaCoverage, [0., 0., 1., 0.5], 0, &Interrupt::NONE, &mut |_| Ok(())),
        Err(TextSourceError::Budget)
    ));
    assert!(matches!(
        prepared
            .render_color_with_charge(RectilinearAntialias::AreaCoverage, [0., 0., 1., 0.5], 8 << 20, &Interrupt::NONE, &mut |_| Err(TextSourceError::Budget)),
        Err(TextSourceError::Budget)
    ));
    let rendered =
        prepared.render_color_with_charge(RectilinearAntialias::AreaCoverage, [0., 0., 1., 0.5], 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
    let bytes = rendered.surface.try_read_interleaved(rendered.rect).unwrap();
    assert!(bytes.as_chunks::<4>().0.iter().any(|p| p[3] == 128));
    assert!(bytes.as_chunks::<4>().0.iter().all(|p| p[3] <= 128));
    let check = || true;
    let progress = |_| {};
    let ctl = Interrupt::new(&check, &progress);
    assert!(matches!(
        prepared.render_color_with_charge(RectilinearAntialias::AreaCoverage, [0., 0., 1., 0.5], 8 << 20, &ctl, &mut |_| Ok(())),
        Err(TextSourceError::Cancelled)
    ));
}
fn u(name: &str, width: u8, value: u64) -> FilterArgument {
    arg(name, FilterValue::Number(ExactNumber::Unsigned { width, value }))
}
fn ast(nodes: &[photocraft_xcf::ConfigNode]) -> Vec<FilterArgument> {
    nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let (kind, value) = match &node.value {
                photocraft_xcf::ConfigValue::Atom(s) => ("atom", FilterValue::String(s.clone())),
                photocraft_xcf::ConfigValue::Quoted(b) => ("quoted-bytes-hex", FilterValue::String(b.iter().map(|b| format!("{b:02x}")).collect())),
                photocraft_xcf::ConfigValue::List(n) => ("list", FilterValue::Config(ast(n))),
                photocraft_xcf::ConfigValue::Number { literal, bits } => (
                    "number",
                    FilterValue::Config(vec![
                        arg("decimal", FilterValue::String(literal.clone())),
                        arg("f64-bits", FilterValue::Number(ExactNumber::Float64Bits(*bits))),
                    ]),
                ),
            };
            arg(&format!("{kind}-{i}"), value)
        })
        .collect()
}
fn canonical(text: &photocraft_xcf::TextLayer) -> (SourceItem, ResourceReference) {
    let f = text.font.as_ref().unwrap();
    let mut layout = vec![arg("font-size", FilterValue::Number(ExactNumber::Float64Bits(text.layout.font_size_bits.unwrap())))];
    for (key, value) in [
        ("font-size-unit", &text.layout.font_size_unit),
        ("language", &text.layout.language),
        ("base-direction", &text.layout.direction),
        ("justification", &text.layout.justification),
        ("box-mode", &text.layout.box_mode),
        ("box-unit", &text.layout.box_unit),
    ] {
        layout.push(arg(key, FilterValue::String(value.clone().unwrap())));
    }
    for (key, value) in [("antialias", text.layout.antialias), ("hinting", text.layout.hinting)] {
        layout.push(arg(key, FilterValue::Bool(value.unwrap())));
    }
    let mut selectors = Vec::new();
    for (key, value) in [("name", &f.name), ("family", &f.family), ("style", &f.style), ("postscript-name", &f.postscript_name), ("requested-hash", &f.hash)] {
        selectors.push(arg(key, FilterValue::String(value.clone().unwrap())));
    }
    selectors.push(u("index", 32, u64::from(f.index.unwrap())));
    selectors.push(u("version", 32, u64::from(f.version.unwrap())));
    for (key, value) in [("weight", f.weight), ("slant", f.slant), ("width", f.width)] {
        selectors.push(arg(key, FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(value.unwrap()) })));
    }
    let configuration = vec![
        arg("gimp-text-configuration", FilterValue::Config(ast(&text.configuration))),
        u("source-flags", 32, 0),
        u("parasite-flags", 32, u64::from(text.parasite_flags)),
        arg("layout", FilterValue::Config(layout)),
        arg("font-selectors", FilterValue::Config(selectors)),
        arg(
            "runs",
            FilterValue::Config(vec![arg(
                "run-0",
                FilterValue::Config(vec![u("start", 64, 0), u("end", 64, text.content.len() as u64), arg("styles", FilterValue::Config(vec![]))]),
            )]),
        ),
        arg("styles", FilterValue::Config(vec![])),
    ];
    (
        SourceItem {
            owner: ItemRef::Layer(photocraft_doc::LayerId::fresh()),
            source: EditableSource::Text {
                current_origin: None,
                content: text.content.clone(),
                markup: None,
                font: ResourceId(1),
                direction: 0,
                source_modified: false,
                configuration,
            },
            transform: None,
            cache: None,
        },
        ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::Font,
            name: f.name.clone().unwrap(),
            collection: None,
            internal: false,
            locator: None,
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        },
    )
}
