use photocraft_doc::{
    item_identity::{ItemRef, ResourceId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
    xcf_items::{EditableSource, SourceItem},
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use photocraft_text::xcf::{ExactFont, StemText, TextSourceError, render_stems};
fn arg(name: &str, value: FilterValue) -> FilterArgument {
    FilterArgument { name: name.into(), value }
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
    if let Some(bits) = text.layout.line_spacing_bits {
        layout.insert(1, arg("line-spacing", FilterValue::Number(ExactNumber::Float64Bits(bits))));
    }
    if let Some(bits) = text.layout.letter_spacing_bits {
        layout.insert(1, arg("letter-spacing", FilterValue::Number(ExactNumber::Float64Bits(bits))));
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

#[test]
#[ignore = "requires frozen independently authored letter-spacing sources"]
fn stored_spacing_semantics_preserve_operative_layout_and_zero_compatibility() {
    use photocraft_text::xcf::{
        RectilinearAntialias, prepare_spaced_rectilinear_with_charge, qualify_colored_rectilinear_source, qualify_spaced_colored_rectilinear_source,
    };
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-spacing-evidence");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for row in data["rows"].as_array().unwrap() {
        let name = row["case"].as_str().unwrap();
        let parsed = photocraft_xcf::decode(&std::fs::read(base.join(format!("{name}.xcf"))).unwrap(), &Default::default()).unwrap();
        assert_eq!((parsed.version, parsed.precision), (19, 150));
        let (item, resource) = canonical(parsed.layers[0].text.as_ref().unwrap());
        let (request, mode, color, spacing) = qualify_spaced_colored_rectilinear_source(&item, &resource, [8, 4], &Interrupt::NONE).unwrap();
        assert_eq!(f64::from(spacing), row["state"]["letter_spacing"].as_f64().unwrap());
        assert_eq!(color.rgba_f32_bits, [1_f32.to_bits(); 4]);
        if spacing != 0. {
            assert!(qualify_colored_rectilinear_source(&item, &resource, [8, 4], &Interrupt::NONE).is_err());
        }
        let prepared = prepare_spaced_rectilinear_with_charge(&font, request, spacing, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        assert_eq!(u64::from(prepared.layout_rect().width()), row["state"]["size"][0].as_u64().unwrap());
        assert_eq!(mode, if row["state"]["antialias"].as_bool().unwrap() { RectilinearAntialias::AreaCoverage } else { RectilinearAntialias::None });
        let actual = prepared
            .render_mode_with_charge(mode, 8 << 20, &Interrupt::NONE, &mut |_| Ok(()))
            .unwrap()
            .surface
            .try_read_interleaved(Rect::new(0, 0, 128, 64))
            .unwrap();
        let expected = std::fs::read(base.join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert!(actual.iter().zip(expected).map(|(a, b)| a.abs_diff(b)).max().unwrap() <= 2, "{name}");
    }
}

#[test]
#[ignore = "requires independent spacing source for canonical atomic failure checks"]
fn spacing_canonical_mismatch_duplicates_budget_and_cancel_are_atomic() {
    use photocraft_text::xcf::{qualify_spaced_colored_rectilinear_source, set_rectilinear_letter_spacing};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-spacing-evidence");
    let original = photocraft_xcf::decode(&std::fs::read(base.join("aa-32-HEFT-source.xcf")).unwrap(), &Default::default()).unwrap();
    let (item, resource) = canonical(original.layers[0].text.as_ref().unwrap());
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for case in 0..4 {
        let mut bad = item.clone();
        let photocraft_doc::xcf_items::EditableSource::Text { configuration, .. } = &mut bad.source else { panic!() };
        let FilterValue::Config(layout) = &mut configuration.iter_mut().find(|a| a.name == "layout").unwrap().value else { panic!() };
        let idx = layout.iter().position(|a| a.name == "letter-spacing").unwrap();
        match case {
            0 => layout[idx].value = FilterValue::Number(ExactNumber::Float64Bits(4_f64.to_bits())),
            1 => layout[idx].value = FilterValue::Number(ExactNumber::Float64Bits(f64::NAN.to_bits())),
            2 => layout.push(layout[idx].clone()),
            _ => layout[idx].name = "unknown-spacing".into(),
        }
        assert!(qualify_spaced_colored_rectilinear_source(&bad, &resource, [8, 4], &Interrupt::NONE).is_err());
        let before = bad.clone();
        assert!(set_rectilinear_letter_spacing(&mut bad, &resource, [8, 4], 4., &font, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).is_err());
        assert_eq!(bad, before);
    }
    for case in 0..3 {
        let mut current = item.clone();
        let cancel = Interrupt::cancel_only(&|| true);
        let ctl = if case == 2 { &cancel } else { &Interrupt::NONE };
        let mut calls = 0;
        assert!(
            set_rectilinear_letter_spacing(&mut current, &resource, [8, 4], 4., &font, if case == 0 { 0 } else { 8 << 20 }, ctl, &mut |_| {
                calls += 1;
                if case == 1 { Err(TextSourceError::Budget) } else { Ok(()) }
            })
            .is_err()
        );
        assert_eq!(current, item);
    }
    let mut current = item.clone();
    assert!(set_rectilinear_letter_spacing(&mut current, &resource, [8, 4], 4., &font, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap());
    let expected = photocraft_xcf::decode(&std::fs::read(base.join("aa-32-HEFT-edit.xcf")).unwrap(), &Default::default()).unwrap();
    let (expected, _) = canonical(expected.layers[0].text.as_ref().unwrap());
    assert_eq!(current.source, expected.source);
    assert_eq!(current.cache, item.cache);
}

#[test]
fn authority_bounds_budget_and_cancel_are_explicit() {
    let ctl = Interrupt::NONE;
    assert!(matches!(ExactFont::authorize(photocraft_text::fonts::INTER_REGULAR.as_slice(), &[0; 32], &ctl), Err(TextSourceError::FontIdentity)));
    let font = ExactFont::bundled_inter(&ctl).unwrap();
    let mut request = StemText { content: "II", origin: [8, 4] };
    assert!(matches!(render_stems(&font, request, 0, &ctl), Err(TextSourceError::Budget)));
    request.content = "fallback";
    assert!(matches!(render_stems(&font, request, u64::MAX, &ctl), Err(TextSourceError::Unsupported(_))));
    request.content = "I";
    request.origin = [i32::MAX, 0];
    assert!(render_stems(&font, request, u64::MAX, &ctl).is_err());
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(ExactFont::bundled_inter(&cancelled), Err(TextSourceError::Cancelled)));
    use sha2::{Digest, Sha256};
    let retained: [u8; 32] = Sha256::digest(photocraft_text::fonts::INTER_REGULAR.as_slice()).into();
    let mut corrupt = photocraft_text::fonts::INTER_REGULAR.as_slice().to_vec();
    corrupt[100] ^= 1;
    assert!(matches!(ExactFont::authorize(&corrupt, &retained, &ctl), Err(TextSourceError::FontIdentity)));
    let mut charged = 0;
    let denied = photocraft_text::xcf::render_stems_with_charge(&font, StemText { content: "III", origin: [-3, -2] }, 2 << 20, &ctl, &mut |bytes| {
        charged = bytes;
        Err(TextSourceError::Budget)
    });
    assert!(matches!(denied, Err(TextSourceError::Budget)));
    assert!(charged > 256 * 256 * 4);
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 4;
    let ctl = Interrupt::cancel_only(&cancel);
    assert!(matches!(render_stems(&font, StemText { content: "IIII", origin: [8, 4] }, 2 << 20, &ctl), Err(TextSourceError::Cancelled)));
}

#[test]
#[ignore = "requires independent public AA source/edit controls"]
fn declared_aa_canonical_content_and_mode_preserved() {
    use photocraft_text::xcf::{
        RectilinearAntialias, qualify_rectilinear_source, qualify_rectilinear_source_with_antialias, render_rectilinear_mode_with_charge,
        set_rectilinear_content_with_antialias,
    };
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-aa-evidence");
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for (case, replacement) in [("shape", "H E"), ("kern", "TL")] {
        let bytes = std::fs::read(base.join(format!("{case}-source.xcf"))).unwrap();
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(file.version, 19);
        assert_eq!(file.layers[0].text.as_ref().unwrap().layout.antialias, Some(true));
        let (mut item, resource) = canonical(file.layers[0].text.as_ref().unwrap());
        assert!(qualify_rectilinear_source(&item, &resource, [8, 4], &Interrupt::NONE).is_err());
        let (request, mode) = qualify_rectilinear_source_with_antialias(&item, &resource, [8, 4], &Interrupt::NONE).unwrap();
        assert_eq!(mode, RectilinearAntialias::AreaCoverage);
        let source = render_rectilinear_mode_with_charge(&font, request, mode, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{case}-source.projection.bin"))).unwrap();
        assert!(source.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap().iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 2));
        let cache = item.cache.clone();
        assert!(
            set_rectilinear_content_with_antialias(
                &mut item,
                &resource,
                StemText { content: replacement, origin: [8, 4] },
                &font,
                8 << 20,
                &Interrupt::NONE,
                &mut |_| Ok(())
            )
            .unwrap()
        );
        let (request, changed_mode) = qualify_rectilinear_source_with_antialias(&item, &resource, [8, 4], &Interrupt::NONE).unwrap();
        assert_eq!(changed_mode, mode);
        assert_eq!(item.cache, cache);
        let edit = render_rectilinear_mode_with_charge(&font, request, mode, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{case}-edit.projection.bin"))).unwrap();
        assert!(edit.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap().iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 2));
        let before = item.clone();
        assert!(
            set_rectilinear_content_with_antialias(
                &mut item,
                &resource,
                StemText { content: "O", origin: [8, 4] },
                &font,
                8 << 20,
                &Interrupt::NONE,
                &mut |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(item, before);
    }
}

#[test]
#[ignore = "requires original isolated public GIMP stem controls"]
fn eight_independent_native_controls_and_second_process_reopens() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/stem-controls");
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for count in 1..=8 {
        let encoded = std::fs::read(root.join(format!("count-{count}.xcf"))).unwrap();
        let decoded = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
        assert_eq!(decoded.version, 19);
        assert_eq!(decoded.layers[0].text_flags, 0);
        let (item, resource) = canonical(decoded.layers[0].text.as_ref().unwrap());
        let request = photocraft_text::xcf::qualify_stem_source(&item, &resource, [8, 4], &Interrupt::NONE).unwrap();
        let content = "I".repeat(count);
        assert_eq!(request.content, content);
        let rendered = render_stems(&font, request, 2 << 20, &Interrupt::NONE).unwrap();
        let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
        let expected = std::fs::read(root.join(format!("count-{count}.projection.bin"))).unwrap();
        let repeated = std::fs::read(root.join(format!("round2/count-{count}.projection.bin"))).unwrap();
        assert_eq!(expected, repeated, "fresh second process count{count}");
        assert_eq!(actual, expected, "current font geometry count{count}");
        let original = item.clone();
        let mut editable = item.clone();
        let next = if count == 8 { "I".to_string() } else { "I".repeat(count + 1) };
        assert!(photocraft_text::xcf::set_stem_content(&mut editable, &resource, [8, 4], &next, &Interrupt::NONE).unwrap());
        let changed = photocraft_text::xcf::qualify_stem_source(&editable, &resource, [8, 4], &Interrupt::NONE).unwrap();
        assert_eq!(changed.content, next);
        let before = editable.clone();
        assert!(photocraft_text::xcf::set_stem_content(&mut editable, &resource, [8, 4], "fallback", &Interrupt::NONE).is_err());
        assert_eq!(editable, before);
        let mut forged = resource.clone();
        forged.name = "fallback".into();
        assert!(photocraft_text::xcf::qualify_stem_source(&original, &forged, [8, 4], &Interrupt::NONE).is_err());
        if let EditableSource::Text { configuration, .. } = &mut editable.source {
            configuration.push(arg("required-unknown", FilterValue::Bool(true)));
        }
        assert!(photocraft_text::xcf::qualify_stem_source(&editable, &resource, [8, 4], &Interrupt::NONE).is_err());
    }
}

#[test]
#[ignore = "requires own public exact-font rectilinear semantic source/edit controls"]
fn rectilinear_canonical_content_edit_matches_independent_gimp_and_preserves_source() {
    use photocraft_text::xcf::{qualify_rectilinear_source, render_rectilinear_with_charge, set_rectilinear_content};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-evidence/rectilinear-pairs");
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for (case, replacement) in [("shape", "H E"), ("kern", "TL")] {
        let source = std::fs::read(base.join(format!("{case}-source.xcf"))).unwrap();
        let file = photocraft_xcf::decode(&source, &Default::default()).unwrap();
        let (mut item, resource) = canonical(file.layers[0].text.as_ref().unwrap());
        let original = item.clone();
        let request = qualify_rectilinear_source(&item, &resource, [8, 4], &Interrupt::NONE).unwrap();
        let rendered = render_rectilinear_with_charge(&font, request, 32 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{case}-source.projection.bin"))).unwrap();
        assert_eq!(rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap(), expected);
        assert!(
            set_rectilinear_content(
                &mut item,
                &resource,
                StemText { content: replacement, origin: [8, 4] },
                &font,
                32 << 20,
                &Interrupt::NONE,
                &mut |_| Ok(())
            )
            .unwrap()
        );
        let request = qualify_rectilinear_source(&item, &resource, [8, 4], &Interrupt::NONE).unwrap();
        let rendered = render_rectilinear_with_charge(&font, request, 32 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{case}-edit.projection.bin"))).unwrap();
        assert_eq!(rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap(), expected);
        assert_eq!(item.owner, original.owner);
        assert_eq!(item.cache, original.cache);
        assert_eq!(item.transform, original.transform);
        let before = item.clone();
        assert!(
            set_rectilinear_content(&mut item, &resource, StemText { content: "O", origin: [8, 4] }, &font, 32 << 20, &Interrupt::NONE, &mut |_| Ok(()))
                .is_err()
        );
        assert_eq!(item, before);
    }
}
#[test]
fn scaled_geometry_size_limits_charges_and_cancellation() {
    use photocraft_text::xcf::{ScaledText, render_scaled_rectilinear_with_charge};
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for size in [f32::NAN, f32::INFINITY, -16., 0., 15., 17., 41., f32::MAX] {
        let mut charges = 0;
        let result =
            render_scaled_rectilinear_with_charge(&font, ScaledText { content: "HEFT", origin: [8, 4], size_px: size }, 8 << 20, &Interrupt::NONE, &mut |_| {
                charges += 1;
                Ok(())
            });
        assert!(result.is_err());
        assert_eq!(charges, 0);
    }
    let mut count = 0;
    let result =
        render_scaled_rectilinear_with_charge(&font, ScaledText { content: "HEFT", origin: [8, 4], size_px: 16. }, 8 << 20, &Interrupt::NONE, &mut |_| {
            count += 1;
            if count == 2 { Err(TextSourceError::Budget) } else { Ok(()) }
        });
    assert!(matches!(result, Err(TextSourceError::Budget)));
    assert_eq!(count, 2);
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 200;
    let mut charges = 0;
    assert!(matches!(
        render_scaled_rectilinear_with_charge(
            &font,
            ScaledText { content: "HEFT", origin: [8, 4], size_px: 16. },
            8 << 20,
            &Interrupt::cancel_only(&cancelled),
            &mut |_| {
                charges += 1;
                Ok(())
            }
        ),
        Err(TextSourceError::Cancelled)
    ));
    assert_eq!(charges, 2);
}
#[test]
#[ignore = "requires frozen independently authored pixel-size originals and public edit"]
fn scaled_original_canonical_semantics_render_and_atomic_size_edit() {
    use photocraft_text::xcf::{ScaledText, qualify_scaled_rectilinear_source, render_scaled_rectilinear_with_charge, set_rectilinear_font_size};
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-size-evidence");
    let manifest = std::fs::read(base.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "caf94a273f103f9ddafdf0090c5a839f1a97d2a24e3f44519913a6bf19d07eb4");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in hashes {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(base.join(&name)).unwrap())), hash);
    }
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let mut admitted = 0;
    for row in observations["rows"].as_array().unwrap() {
        let name = row["case"].as_str().unwrap();
        let file = photocraft_xcf::decode(&std::fs::read(base.join(format!("{name}.xcf"))).unwrap(), &Default::default()).unwrap();
        assert_eq!(file.version, 19);
        assert_eq!(file.precision, 150);
        let (item, res) = canonical(file.layers[0].text.as_ref().unwrap());
        let request = qualify_scaled_rectilinear_source(&item, &res, [8, 4], &Interrupt::NONE);
        if !row["state"]["antialias"].as_bool().unwrap() {
            assert!(request.is_err(), "scaled no-AA must remain unsupported");
            continue;
        }
        let (request, mode) = request.unwrap();
        assert_eq!(mode, photocraft_text::xcf::RectilinearAntialias::AreaCoverage);
        assert_eq!(f64::from(request.size_px), row["state"]["font_size"][0].as_f64().unwrap());
        let prepared = photocraft_text::xcf::prepare_scaled_rectilinear_with_charge(&font, request, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let layout = prepared.layout_rect();
        assert_eq!(
            layout,
            Rect::new(8, 4, 8 + row["state"]["size"][0].as_i64().unwrap() as i32, 4 + row["state"]["size"][1].as_i64().unwrap() as i32),
            "dynamic layout box {name}"
        );
        let rendered = render_scaled_rectilinear_with_charge(&font, request, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{name}.projection.bin"))).unwrap();
        let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
        assert_eq!(actual.len(), expected.len());
        let max = actual.iter().zip(expected).map(|(a, b)| a.abs_diff(b)).max().unwrap();
        assert!(max <= 2, "{name}: {max}/255");
        println!("{name}: max{max}/255");
        admitted += 1;
    }
    assert_eq!(admitted, 14);
    let source = photocraft_xcf::decode(&std::fs::read(base.join("aa-size-source.xcf")).unwrap(), &Default::default()).unwrap();
    let edited = photocraft_xcf::decode(&std::fs::read(base.join("aa-size-edit.xcf")).unwrap(), &Default::default()).unwrap();
    let (mut item, res) = canonical(source.layers[0].text.as_ref().unwrap());
    let (expected, _) = canonical(edited.layers[0].text.as_ref().unwrap());
    let before = item.clone();
    for size in [f32::NAN, 17.] {
        let p = ScaledText { content: "HEFT", origin: [8, 4], size_px: size };
        assert!(set_rectilinear_font_size(&mut item, &res, p, &font, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).is_err());
        assert_eq!(item, before);
    }
    let p = ScaledText { content: "HEFT", origin: [8, 4], size_px: 24. };
    assert!(set_rectilinear_font_size(&mut item, &res, p, &font, 0, &Interrupt::NONE, &mut |_| Ok(())).is_err());
    assert_eq!(item, before);
    assert!(set_rectilinear_font_size(&mut item, &res, p, &font, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap());
    assert_eq!(item.source, expected.source);
    assert_eq!(item.cache, before.cache);
}

#[test]
#[ignore = "requires independently frozen spacing source and control"]
fn zero_spacing_structural_allocation_cancel_and_raw_index_renumber_are_atomic() {
    use photocraft_text::xcf::set_rectilinear_letter_spacing;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-spacing-evidence");
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for (from, to, next) in [("control", "source", 2.), ("source", "control", 0.)] {
        let a = photocraft_xcf::decode(&std::fs::read(root.join(format!("aa-32-HEFT-{from}.xcf"))).unwrap(), &Default::default()).unwrap();
        let b = photocraft_xcf::decode(&std::fs::read(root.join(format!("aa-32-HEFT-{to}.xcf"))).unwrap(), &Default::default()).unwrap();
        let (original, res) = canonical(a.layers[0].text.as_ref().unwrap());
        let (expected, _) = canonical(b.layers[0].text.as_ref().unwrap());
        let mut item = original.clone();
        let mut calls = 0;
        assert!(
            set_rectilinear_letter_spacing(&mut item, &res, [8, 4], next, &font, 8 << 20, &Interrupt::NONE, &mut |_| {
                calls += 1;
                Ok(())
            })
            .unwrap()
        );
        assert_eq!(item.source, expected.source);
        assert_eq!(item.cache, original.cache);
        for deny in 1..=calls {
            let mut item = original.clone();
            let mut call = 0;
            assert!(matches!(
                set_rectilinear_letter_spacing(&mut item, &res, [8, 4], next, &font, 8 << 20, &Interrupt::NONE, &mut |_| {
                    call += 1;
                    if call == deny { Err(TextSourceError::Budget) } else { Ok(()) }
                }),
                Err(TextSourceError::Budget)
            ));
            assert_eq!(item, original, "callback{deny}/{calls}");
        }
        let flag = std::sync::atomic::AtomicBool::new(false);
        let check = || flag.load(std::sync::atomic::Ordering::Relaxed);
        let mut call = 0;
        let mut item = original.clone();
        assert!(matches!(
            set_rectilinear_letter_spacing(&mut item, &res, [8, 4], next, &font, 8 << 20, &Interrupt::cancel_only(&check), &mut |_| {
                call += 1;
                if call == calls {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(())
            }),
            Err(TextSourceError::Cancelled)
        ));
        assert_eq!(item, original);
    }
}

#[test]
#[ignore = "requires independently authored placement originals"]
fn operative_placement_qualifier_keeps_legacy_wrappers_closed_and_source_inert() {
    use photocraft_text::xcf::{
        prepare_spaced_rectilinear_with_charge, qualify_placed_spaced_colored_rectilinear_source, qualify_spaced_colored_rectilinear_source,
    };
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-placement-evidence");
    let manifest = std::fs::read(base.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "b3a3bfc82a7c58d4da8c86698b1c71a74c2afae57d5a2438133906e35888f316");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in hashes {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(base.join(name)).unwrap())), hash);
    }
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for key in ["none-32-HEFT-control", "aa-16-HEFT-source", "aa-32-H-E-edit", "aa-40-II-source"] {
        let file = photocraft_xcf::decode(&std::fs::read(base.join(format!("{key}-origin.xcf"))).unwrap(), &Default::default()).unwrap();
        let (original, resource) = canonical(file.layers[0].text.as_ref().unwrap());
        for (suffix, origin) in [("shift", [11, 2]), ("negative", [-5, 7])] {
            let mut placed = original.clone();
            let mut calls = 0;
            assert!(
                photocraft_text::xcf::set_rectilinear_origin(&mut placed, &resource, [8, 4], origin, &font, 8 << 20, &Interrupt::NONE, &mut |_| {
                    calls += 1;
                    Ok(())
                })
                .unwrap()
            );
            let mut expected_source = original.clone();
            let EditableSource::Text { current_origin, .. } = &mut expected_source.source else { panic!() };
            *current_origin = Some(origin);
            assert_eq!(placed, expected_source);
            for denied in 1..=calls {
                let mut failed = original.clone();
                let mut n = 0;
                assert!(matches!(
                    photocraft_text::xcf::set_rectilinear_origin(&mut failed, &resource, [8, 4], origin, &font, 8 << 20, &Interrupt::NONE, &mut |_| {
                        n += 1;
                        if n == denied { Err(TextSourceError::Budget) } else { Ok(()) }
                    }),
                    Err(TextSourceError::Budget)
                ));
                assert_eq!(failed, original);
            }
            let flag = std::sync::atomic::AtomicBool::new(false);
            let check = || flag.load(std::sync::atomic::Ordering::Relaxed);
            let mut failed = original.clone();
            let mut n = 0;
            assert!(matches!(
                photocraft_text::xcf::set_rectilinear_origin(
                    &mut failed,
                    &resource,
                    [8, 4],
                    origin,
                    &font,
                    8 << 20,
                    &Interrupt::cancel_only(&check),
                    &mut |_| {
                        n += 1;
                        if n == calls {
                            flag.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        Ok(())
                    }
                ),
                Err(TextSourceError::Cancelled)
            ));
            assert_eq!(failed, original);
            assert!(qualify_spaced_colored_rectilinear_source(&placed, &resource, [8, 4], &Interrupt::NONE).is_err());
            assert!(photocraft_text::xcf::qualify_scaled_rectilinear_source(&placed, &resource, [8, 4], &Interrupt::NONE).is_err());
            let (request, mode, _, spacing) = qualify_placed_spaced_colored_rectilinear_source(&placed, &resource, [8, 4], &Interrupt::NONE).unwrap();
            assert_eq!(request.origin, origin);
            let prepared = prepare_spaced_rectilinear_with_charge(&font, request, spacing, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
            let actual = prepared
                .render_mode_with_charge(mode, 8 << 20, &Interrupt::NONE, &mut |_| Ok(()))
                .unwrap()
                .surface
                .try_read_interleaved(Rect::new(0, 0, 128, 64))
                .unwrap();
            let expected = std::fs::read(base.join(format!("{key}-{suffix}.projection.bin"))).unwrap();
            let delta = actual.iter().zip(expected).map(|(a, b)| a.abs_diff(b)).max().unwrap();
            println!("{key}-{suffix}: {delta}/255");
            assert!(delta <= 2);
            assert_eq!(placed.cache, original.cache);
            assert!(
                photocraft_text::xcf::set_rectilinear_origin(&mut placed, &resource, [8, 4], [8, 4], &font, 8 << 20, &Interrupt::NONE, &mut |_| Ok(()))
                    .unwrap()
            );
            assert_eq!(placed, original);
            for invalid in [[i32::MAX, 0], [0, i32::MIN]] {
                let EditableSource::Text { current_origin, .. } = &mut placed.source else { panic!() };
                *current_origin = Some(invalid);
                assert!(qualify_placed_spaced_colored_rectilinear_source(&placed, &resource, [8, 4], &Interrupt::NONE).is_err());
            }
        }
    }
}

#[test]
#[ignore = "requires independent hash-bound exact-font multiline originals"]
fn canonical_multiline_raw_layout_qualification_and_atomic_edits() {
    use photocraft_text::xcf::{
        RectilinearLineLayout, prepare_qualified_rectilinear_with_charge, qualify_placed_layout_colored_rectilinear_source,
        qualify_placed_spaced_colored_rectilinear_source, set_multiline_rectilinear_content, set_rectilinear_line_spacing,
    };
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-multiline-evidence");
    let raw = std::fs::read(base.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), "b399e6e64e4dbb2390ff52a037a0b6eb60071721b32bbfd3ef922552317dfb50");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for row in data["rows"].as_array().unwrap() {
        let name = row["case"].as_str().unwrap();
        let parsed = photocraft_xcf::decode(&std::fs::read(base.join(format!("{name}.xcf"))).unwrap(), &Default::default()).unwrap();
        let (item, resource) = canonical(parsed.layers[0].text.as_ref().unwrap());
        let source = qualify_placed_layout_colored_rectilinear_source(&item, &resource, [8, 4], &Interrupt::NONE).unwrap();
        let spacing = row["state"]["line_spacing"].as_f64().unwrap() as f32;
        assert_eq!(source.layout, RectilinearLineLayout::TwoLines { line_spacing_px: spacing });
        assert!(qualify_placed_spaced_colored_rectilinear_source(&item, &resource, [8, 4], &Interrupt::NONE).is_err());
        let prepared = prepare_qualified_rectilinear_with_charge(&font, source, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        assert_eq!(u64::from(prepared.layout_rect().height()), row["state"]["size"][1].as_u64().unwrap());
        let result = prepared.render_mode_with_charge(source.antialias, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let actual = result.surface.try_read_interleaved(Rect::new(0, 0, 128, 128)).unwrap();
        let expected = std::fs::read(base.join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert!(actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap() <= 2, "{name}");
        let mut current = item.clone();
        for target in [0., 2., 4., spacing] {
            set_rectilinear_line_spacing(&mut current, &resource, [8, 4], target, &font, 16 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
            let target_name = name.rsplit_once("line").unwrap().0.to_string() + "line" + &(target as i32).to_string();
            let expected = photocraft_xcf::decode(&std::fs::read(base.join(format!("{target_name}.xcf"))).unwrap(), &Default::default()).unwrap();
            assert_eq!(current.source, canonical(expected.layers[0].text.as_ref().unwrap()).0.source);
            assert_eq!(current.cache, item.cache);
        }
        assert_eq!(current, item, "returning to original spacing preserves exact raw absence/indexing");
        let next_text = if source.request.content == "I\nI" { "HE\nFT" } else { "I\nI" };
        let next_name = name.replace("two-stems", "REPLACE").replace("mixed-lines", "two-stems").replace("REPLACE", "mixed-lines");
        set_multiline_rectilinear_content(&mut current, &resource, [8, 4], next_text, &font, 16 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = photocraft_xcf::decode(&std::fs::read(base.join(format!("{next_name}.xcf"))).unwrap(), &Default::default()).unwrap();
        assert_eq!(current.source, canonical(expected.layers[0].text.as_ref().unwrap()).0.source);
    }
    let parsed = photocraft_xcf::decode(&std::fs::read(base.join("aa-32-H-E-edit-mixed-lines-line0.xcf")).unwrap(), &Default::default()).unwrap();
    let (item, resource) = canonical(parsed.layers[0].text.as_ref().unwrap());
    let mut probe = item.clone();
    let mut calls = 0;
    set_rectilinear_line_spacing(&mut probe, &resource, [8, 4], 2., &font, 16 << 20, &Interrupt::NONE, &mut |_| {
        calls += 1;
        Ok(())
    })
    .unwrap();
    for denied in 1..=calls {
        let mut current = item.clone();
        let mut at = 0;
        assert!(
            set_rectilinear_line_spacing(&mut current, &resource, [8, 4], 2., &font, 16 << 20, &Interrupt::NONE, &mut |_| {
                at += 1;
                if at == denied { Err(TextSourceError::Budget) } else { Ok(()) }
            })
            .is_err()
        );
        assert_eq!(current, item);
    }
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let check = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&check);
    let mut at = 0;
    let mut current = item.clone();
    assert!(matches!(
        set_rectilinear_line_spacing(&mut current, &resource, [8, 4], 2., &font, 16 << 20, &ctl, &mut |_| {
            at += 1;
            if at == calls {
                cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(())
        }),
        Err(TextSourceError::Cancelled)
    ));
    assert_eq!(current, item);
}
