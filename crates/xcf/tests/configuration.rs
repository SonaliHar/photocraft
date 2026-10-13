mod common;
use photocraft_xcf::*;
fn string(v: &mut Vec<u8>, s: &str) {
    common::u32be(v, s.len() as u32 + 1);
    v.extend(s.as_bytes());
    v.push(0);
}
fn fixture(operation: &str, config: &str, kind: u32) -> Vec<u8> {
    let mut effect = Vec::new();
    for s in ["Own", "icon", operation, "0:0"] {
        string(&mut effect, s);
    }
    let mut arg = Vec::new();
    string(&mut arg, if kind == 6 { "config" } else { "string" });
    common::u32be(&mut arg, kind);
    string(&mut arg, config);
    common::prop(&mut effect, 45, &arg);
    common::prop(&mut effect, 0, &[]);
    common::pointer(&mut effect, 0, 22);
    let options = common::FixtureOptions { effect_records: vec![effect], ..Default::default() };
    common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options)
}
#[test]
fn levels_preserve_repeated_selector_order_exact_numbers_and_raw_text() {
    let text =
        "(time 0)(trc linear)(clamp-input yes)(clamp-output no)(channel red)(low-input 0.125)(high-input 0.875)(gamma 1.5)(low-output -0.0)(high-output 1.2)";
    let f = decode(&fixture("gimp:levels", text, 6), &Default::default()).unwrap();
    let arg = &f.layers[0].effects[0].arguments[0];
    assert_eq!(arg.value, ArgumentValue::Config(text.into()));
    let TypedConfiguration::Levels(cfg) = arg.configuration.as_ref().unwrap().typed.as_ref().unwrap() else { panic!("levels") };
    assert_eq!(cfg.channels[0].channel, "red");
    assert_eq!(cfg.channels[0].low_output_bits, (-0f64).to_bits());
    assert_eq!(cfg.channels[0].gamma_bits, 1.5f64.to_bits());
    assert_eq!(cfg.clamp_output, Some(false));
}
#[test]
fn curve_shapes_and_hostile_configuration_limits_are_explicit() {
    let text = "(channel value)(curve (curve-type free)(points 0)(point-types 0)(n-samples 3)(samples 3 0 0.5 1))";
    let bytes = fixture("gimp:curves", text, 6);
    let f = decode(&bytes, &Default::default()).unwrap();
    let TypedConfiguration::Curves(cfg) = f.layers[0].effects[0].arguments[0].configuration.as_ref().unwrap().typed.as_ref().unwrap() else { panic!("curve") };
    assert_eq!(cfg.channels[0].curve.samples_bits, [0f64.to_bits(), 0.5f64.to_bits(), 1f64.to_bits()]);
    for text in [
        "(channel value)(curve (unknown 1))",
        "(channel value)(curve (curve-type free)(points 0)(point-types 0)(n-samples 3)(samples 2 0 1))",
        "(time NaN)",
        "(time 1",
        "(time 1)(required-unknown 1)",
    ] {
        assert!(decode(&fixture("gimp:curves", text, 6), &Default::default()).is_err());
    }
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("configEntries", 2)]).unwrap();
    assert_eq!(decode_with_resources(&bytes, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
}

fn color_balance_text() -> &'static str {
    "(time 0)(range shadows)(cyan-red -0.0)(magenta-green 0.125)(yellow-blue -0.25)(range midtones)(cyan-red 0.18)(magenta-green -0.12)(yellow-blue 0.23)(range highlights)(cyan-red 0.375)(magenta-green -0.5)(yellow-blue 0.625)(preserve-luminosity no)"
}
#[test]
fn color_balance_preserves_three_ordered_ranges_and_exact_source_scalars() {
    let text = color_balance_text();
    let file = decode(&fixture("gimp:color-balance", text, 6), &Default::default()).unwrap();
    let argument = &file.layers[0].effects[0].arguments[0];
    assert_eq!(argument.value, ArgumentValue::Config(text.into()));
    let TypedConfiguration::ColorBalance(config) = argument.configuration.as_ref().unwrap().typed.as_ref().unwrap() else { panic!("color balance") };
    assert_eq!(config.time_bits, Some(0.0f64.to_bits()));
    assert_eq!(config.preserve_luminosity, Some(false));
    assert_eq!(config.ranges.iter().map(|range| range.range.as_str()).collect::<Vec<_>>(), ["shadows", "midtones", "highlights"]);
    assert_eq!(config.ranges[0].cyan_red_bits, (-0.0f64).to_bits());
    assert_eq!(config.ranges[1].cyan_red_bits, 0.18f64.to_bits());
    assert_eq!(config.ranges[1].magenta_green_bits, (-0.12f64).to_bits());
    assert_eq!(config.ranges[2].yellow_blue_bits, 0.625f64.to_bits());
    assert!(file.layers[0].unsupported.iter().any(|item| item.contains("effects")));
}
#[test]
fn color_balance_unknown_duplicate_missing_and_nonfinite_fields_reject() {
    let text = color_balance_text();
    for invalid in [
        text.replace("(range midtones)", "(range shadows)"),
        text.replace("(range shadows)", "(range unknown)"),
        text.replace("(cyan-red -0.0)", "(cyan-red -0.0)(cyan-red 0.25)"),
        text.replace("(cyan-red -0.0)", ""),
        text.replace("(range shadows)", ""),
        text.replace("(time 0)", "(time 0)(time 1)"),
        text.replace("(time 0)", ""),
        text.replace("(preserve-luminosity no)", ""),
        text.replace("(preserve-luminosity no)", "(preserve-luminosity no)(preserve-luminosity yes)"),
        text.replace("(preserve-luminosity no)", "(preserve-luminosity unknown)"),
        text.replace("(cyan-red 0.18)", "(cyan-red NaN)"),
        text.replace("(cyan-red 0.18)", "(cyan-red inf)"),
        format!("{text}(unknown-setting 1)"),
    ] {
        assert!(decode(&fixture("gimp:color-balance", &invalid, 6), &Default::default()).is_err(), "{invalid}");
    }
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("configEntries", 2)]).unwrap();
    assert_eq!(decode_with_resources(&fixture("gimp:color-balance", text, 6), &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
}
#[test]
fn nested_graph_syntax_is_bounded_ordered_and_inert() {
    let text = "id=own exposure exposure=0.5 over aux=[ ref=own gaussian-blur std-dev-x=1.5 ] text string='Own Ω'";
    let bytes = fixture("gegl:gegl", text, 4);
    let f = decode(&bytes, &Default::default()).unwrap();
    let graph = f.layers[0].effects[0].arguments[0].graph.as_ref().unwrap();
    assert_eq!(graph.nodes, 4);
    assert!(graph.edges >= 3);
    assert!(graph.entries.iter().any(|e| matches!(e,GraphEntry::Property{name,value:GraphValue::Chain(_),..}if name=="aux")));
    assert!(f.layers[0].unsupported.iter().any(|s| s.contains("effects")));
    for text in ["over aux=[ exposure", "text string='unterminated", "exposure exposure=NaN", "over ]"] {
        assert!(decode(&fixture("gegl:gegl", text, 4), &Default::default()).is_err());
    }
    for (key, value) in [("graphNodes", 1), ("graphEdges", 0), ("configDepth", 0)] {
        let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[(key, value)]).unwrap();
        assert_eq!(decode_with_resources(&bytes, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
    }
}
#[test]
#[ignore = "explicitly emits original synthetic seeds into the owned fuzz corpus"]
fn emit_owned_configuration_fuzz_seeds() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/decode");
    std::fs::create_dir_all(&root).unwrap();
    for (name, operation, text, kind) in [
        (
            "own-config-levels",
            "gimp:levels",
            "(time 0)(trc linear)(clamp-input yes)(clamp-output no)(channel red)(low-input 0.125)(high-input 0.875)(gamma 1.5)(low-output -0.0)(high-output 1.2)",
            6,
        ),
        ("own-config-curve", "gimp:curves", "(channel value)(curve (curve-type free)(points 0)(point-types 0)(n-samples 3)(samples 3 0 0.5 1))", 6),
        ("own-config-graph", "gegl:gegl", "id=own exposure exposure=0.5 over aux=[ ref=own gaussian-blur std-dev-x=1.5 ] text string='Own Ω'", 4),
    ] {
        let bytes = fixture(operation, text, kind);
        assert!(decode(&bytes, &Default::default()).is_ok());
        std::fs::write(root.join(name), bytes).unwrap();
    }
}

#[test]
fn graph_references_ports_cycles_and_fanout_are_checked() {
    for (source, diagnostic) in [
        ("id=a exposure id=a", "duplicate graph id"),
        ("over aux=[ ref=missing exposure ]", "dangling graph ref"),
        ("over id=a aux=[ ref=a exposure ]", "cycle in stored graph"),
        ("over aux=[ exposure ] aux=[ exposure ]", "duplicate graph aux port"),
        ("over aux2=[ exposure ]", "unqualified graph port"),
        ("aux=[ exposure ]", "missing owner"),
        ("over aux=[ exposure ref=a ]", "ref must occur once"),
    ] {
        let error = decode(&fixture("gegl:gegl", source, 4), &Default::default()).unwrap_err();
        assert!(error.message.contains(diagnostic), "{source}: {error}");
    }
    // A reference can name a later declaration; resolving it must still prove a DAG.
    let forward = "over aux=[ ref=a exposure ] exposure id=a";
    assert!(decode(&fixture("gegl:gegl", forward, 4), &Default::default()).unwrap_err().message.contains("cycle"));
    let mut fanout = String::from("id=own");
    for _ in 0..256 {
        fanout.push_str(" over aux=[ ref=own exposure ]");
    }
    let bytes = fixture("gegl:gegl", &fanout, 4);
    assert_eq!(decode(&bytes, &Default::default()).unwrap().layers[0].effects[0].arguments[0].graph.as_ref().unwrap().nodes, 512);
    let calls = std::cell::Cell::new(0usize);
    let observe = || {
        calls.set(calls.get() + 1);
        false
    };
    decode(&bytes, &DecodeOptions { cancelled: Some(&observe), ..Default::default() }).unwrap();
    let completed_polls = calls.get();
    assert!(completed_polls > 1000);
    calls.set(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > completed_polls - 1000
    };
    assert_eq!(decode(&bytes, &DecodeOptions { cancelled: Some(&stop), ..Default::default() }).unwrap_err().kind, ErrorKind::Cancelled);
    let deep = format!("{}exposure{}", "over aux=[ ".repeat(66), "]".repeat(66));
    assert_eq!(decode(&fixture("gegl:gegl", &deep, 4), &Default::default()).unwrap_err().kind, ErrorKind::Limit);
}
