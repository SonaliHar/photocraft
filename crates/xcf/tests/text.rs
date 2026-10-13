mod common;
use photocraft_xcf::*;
fn parasite(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    common::u32be(&mut out, 16);
    out.extend(b"gimp-text-layer\0");
    common::u32be(&mut out, 1);
    common::u32be(&mut out, text.len() as u32);
    out.extend(text);
    out
}
fn fixture(config: &[u8], flags: u32) -> Vec<u8> {
    common::image_custom(
        22,
        150,
        0,
        1,
        1,
        &[1, 2, 3, 255],
        &common::FixtureOptions { layer_properties: vec![(21, parasite(config)), (26, flags.to_be_bytes().to_vec())], ..Default::default() },
    )
}
#[test]
fn typed_text_preserves_unicode_layout_font_raw_bytes_and_modified_raster() {
    let config=b"(text \"Own \xce\xa9\\nsecond\")\n(font \"GimpFont\" (family \"Own Font\") (style \"Regular\") (index 2))\n(font-size 15.75)\n(font-size-unit pixels)\n(color (color \"RGBA float\" 4 \"\\0\\377\\200\\42\" 0))\n(hinting yes)\0";
    let file = decode(&fixture(config, 3), &Default::default()).unwrap();
    let layer = &file.layers[0];
    let text = layer.text.as_ref().unwrap();
    assert_eq!(text.text.as_deref(), Some("Own Ω\nsecond"));
    assert!(layer.text_source_modified());
    assert_eq!(text.raw, config);
    assert_eq!(text.font.as_ref().unwrap().family.as_deref(), Some("Own Font"));
    assert_eq!(text.font.as_ref().unwrap().index, Some(2));
    assert_eq!(text.layout.font_size_bits, Some(15.75f64.to_bits()));
    assert_eq!(text.layout.font_size_unit.as_deref(), Some("pixels"));
    assert_eq!(text.layout.hinting, Some(true));
    assert_eq!((text.runs[0].start, text.runs[0].end), (0, text.content.len() as u64));
    assert_eq!(layer.plane.as_ref().unwrap().bytes, [1, 2, 3, 255]);
    assert!(text.unknown_fields.is_empty());
    assert_eq!(text.parasite_flags, 1);
    for node in &text.configuration {
        assert!(node.start < node.end);
        assert!(node.end <= config.len() as u64);
    }
}
#[test]
fn unknown_text_settings_are_explicit_and_hostile_configs_fail() {
    let f = decode(&fixture(b"(text \"Own\") (future-required 7)\0", 0), &Default::default()).unwrap();
    assert_eq!(f.layers[0].text.as_ref().unwrap().unknown_fields, ["future-required"]);
    for config in [
        b"(text \"unterminated)".as_slice(),
        b"(text \"\\777\")",
        b"(text \"bad\xff\")",
        b"(text \"ok\") garbage",
        b"(text \"one\") (text \"two\")",
        b"(text \"ok\")\0hidden",
    ] {
        assert!(decode(&fixture(config, 0), &Default::default()).is_err(), "{config:?}");
    }
    let deep = format!("(future {}x{})", "(".repeat(40), ")".repeat(40));
    assert!(decode(&fixture(deep.as_bytes(), 0), &Default::default()).is_err());
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("textBytes", 0)]).unwrap();
    assert_eq!(decode_with_resources(&fixture(b"(text \"Own\")", 0), &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
}
#[test]
fn duplicate_known_text_font_layout_fields_and_escaped_nul_reject() {
    for config in [
        b"(text \"Own\") (font-size 12) (font-size 18)".as_slice(),
        b"(text \"Own\") (base-direction ltr) (base-direction rtl)",
        b"(text \"Own\") (transformation 1 0 0 1) (transformation 2 0 0 2)",
        b"(text \"Own\") (font \"GimpFont\" (family \"Own1\") (family \"Own2\"))",
        b"(text \"Own\") (font \"GimpFont\" (index 1) (index 2))",
        b"(text \"Own\") (language \"en\\0hidden\")",
    ] {
        assert!(decode(&fixture(config, 0), &Default::default()).is_err(), "{config:?}");
    }
    let reordered = decode(&fixture(b"(font-size 18) (base-direction rtl) (text \"Own\") (font-size-unit pixels)", 0), &Default::default()).unwrap();
    let text = reordered.layers[0].text.as_ref().unwrap();
    assert_eq!(text.layout.font_size_bits, Some(18f64.to_bits()));
    assert_eq!(text.layout.direction.as_deref(), Some("rtl"));
    assert_eq!(text.content, "Own");
}
#[test]
fn markup_runs_use_utf8_boundaries_and_preserve_style_declarations() {
    let config = b"(markup \"<markup><span foreground=\\\"#50b0e0\\\">Own \xce\xa9</span> &amp; <b>&#x1f642;</b></markup>\" (markupfont \"font\" (font)))\0";
    let file = decode(&fixture(config, 0), &Default::default()).unwrap();
    let t = file.layers[0].text.as_ref().unwrap();
    assert_eq!(t.content, "Own Ω & 🙂");
    for run in &t.runs {
        assert!(t.content.is_char_boundary(run.start as usize));
        assert!(t.content.is_char_boundary(run.end as usize));
        for id in &run.style_ids {
            assert!(*id < t.styles.len());
        }
    }
    assert!(t.styles.iter().any(|s| s.tag == "span" && s.attributes == [("foreground".into(), "#50b0e0".into())]));
    for xml in [
        "<!DOCTYPE markup [<!ENTITY e SYSTEM 'file:///private'>]><markup>&e;</markup>",
        "<span unknown='7'>x</span>",
        "<span>x</b>",
        "<markup>&unknown;</markup>",
    ] {
        let config = format!("(markup {:?})", xml);
        assert!(decode(&fixture(config.as_bytes(), 0), &Default::default()).is_err());
    }
}
#[test]
fn text_config_limits_and_cancellation_return_no_partial_file() {
    let bytes = fixture(b"(text \"Own\") (font-size 12.5)", 0);
    for (key, value) in [("configEntries", 0), ("configDepth", 0), ("residentBytes", 64)] {
        let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[(key, value)]).unwrap();
        assert_eq!(decode_with_resources(&bytes, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
    }
    let long = format!("(text \"{}\") (font-size 12.5)", "Own Ω ".repeat(20000));
    let bytes = fixture(long.as_bytes(), 0);
    let polls = std::cell::Cell::new(0);
    let cancelled = || {
        polls.set(polls.get() + 1);
        polls.get() >= 30
    };
    let options = DecodeOptions { cancelled: Some(&cancelled), ..Default::default() };
    assert_eq!(decode(&bytes, &options).unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(polls.get(), 30);
    for config in [b"(text \"Own\") (font-size nan)".as_slice(), b"(text \"Own\") (font-size \"future\")", b"(text \"\\0\")"] {
        assert!(decode(&fixture(config, 0), &Default::default()).is_err());
    }
    let f = decode(&fixture(b"(text \"Own\") (font \"GimpFont\" (future-font \"unknown\"))", 0), &Default::default()).unwrap();
    assert_eq!(f.layers[0].text.as_ref().unwrap().unknown_fields, ["future-font"]);
}
