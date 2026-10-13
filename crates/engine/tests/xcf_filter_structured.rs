#![cfg(feature = "corpus")]
//! Closed stored configurations, never source proxy inference or numeric normalization.
use photocraft_doc::{
    Document,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_engine::Session;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn session(doc: Document) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    s
}
fn wire(value: &FilterValue) -> Value {
    match value {
        FilterValue::Number(ExactNumber::Float64Bits(bits)) => json!({"Number":{"Float64Bits":format!("0x{bits:016x}")}}),
        FilterValue::Config(fields) => json!({"Config":fields.iter().map(argument).collect::<Vec<_>>()}),
        FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } => json!({"Curve":{
            "free":free,"points_f64_bits":points_f64_bits,"point_types":point_types,
            "samples_f64_bits":samples_f64_bits.iter().map(|bits|format!("0x{bits:016x}")).collect::<Vec<_>>()}}),
        other => serde_json::to_value(other).unwrap(),
    }
}
fn argument(value: &FilterArgument) -> Value {
    json!({"name":value.name,"value":wire(&value.value)})
}

#[test]
#[ignore = "requires reviewed unpublished FREE256/structured Levels source-edit originals"]
fn structured_stock_edits_preserve_source_and_match_independent_reference() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let pins: Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), pins[path].as_str().unwrap());
        bytes
    };
    let mut cases = vec![("free256-rgb-u8".to_owned(), "free256-rgb-u8".to_owned())];
    for trc in [0, 1] {
        for clamps in ["00", "01", "10", "11"] {
            let name = format!("levels-trc{trc}-clamps{clamps}");
            cases.push((name.clone(), name));
        }
    }
    for trc in [0, 1] {
        cases.push((format!("levels-trc{trc}-clamps00"), format!("levels-trc{trc}-clamps11")));
    }
    for (name, reference_name) in cases {
        let base = format!("curve-config-matrix/{name}");
        let source_path = format!("{base}.xcf");
        let original = read(&source_path);
        let before = photocraft_io::import("source.xcf", &original).unwrap().document;
        let reference_base = format!("curve-config-matrix/{reference_name}");
        let reference = photocraft_io::import("reference.xcf", &read(&format!("{reference_base}.edit.xcf"))).unwrap().document;
        let old = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let mut s = session(before.clone());
        let info = s.execute("xcf.filter.inspect", json!({"filterId":old.id.0.to_string()})).unwrap();
        let config_index = old.arguments.iter().position(|argument| argument.name == "config").unwrap();
        assert_eq!(info["arguments"][config_index]["state"]["value"], wire(&old.arguments[config_index].value));
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        let noop = s.execute("xcf.filter.set", json!({"filterId":old.id.0.to_string(),"arguments":[argument(&old.arguments[config_index])]})).unwrap();
        assert_eq!(noop["updated"], false);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        assert_eq!(*s.active().unwrap().doc, before);
        let patches: Vec<_> = target.arguments.iter().filter(|argument| argument.name == "trc" || !old.arguments.contains(argument)).map(argument).collect();
        s.execute("xcf.filter.set", json!({"filterId":old.id.0.to_string(),"arguments":patches})).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert_eq!(edited.layers, before.layers);
        assert_eq!(edited.source_color_context, before.source_color_context);
        let state = edited.xcf_state.as_ref().unwrap();
        let original_state = before.xcf_state.as_ref().unwrap();
        assert_eq!(state.opaque, original_state.opaque);
        assert_eq!(state.provenance, original_state.provenance);
        assert_eq!(state.filter_masks, original_state.filter_masks);
        assert_eq!(state.filter_stacks[0].nodes[0].arguments, target.arguments);
        let mut retained = state.filter_stacks[0].nodes[0].clone();
        retained.arguments = old.arguments.clone();
        assert_eq!(&retained, old);
        let current = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        let direct = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(current.px, direct.px);
        let expected = photocraft_codecs::decode(&read(&format!("{reference_base}.edit.ref.png"))).unwrap();
        let stored = photocraft_cms::Profile::parse(edited.icc_profile.as_ref().unwrap()).unwrap();
        let display = expected
            .icc
            .as_ref()
            .map(|bytes| photocraft_cms::Profile::parse(bytes).unwrap())
            .unwrap_or_else(|| photocraft_cms::Builtin::Srgb.profile().clone());
        let transform = photocraft_cms::Transform::new(&stored, &display, photocraft_cms::Intent::RelativeColorimetric, true).unwrap();
        let mut worst = 0.0_f64;
        for (index, pixel) in current.px.iter().enumerate() {
            let mut rgb = [0.0; 3];
            transform.try_eval_f64(&pixel[..3], &mut rgb).unwrap();
            for (channel, value) in rgb.into_iter().chain([pixel[3]]).enumerate() {
                worst = worst
                    .max((value.clamp(0.0, 1.0) - f64::from(expected.get(index as u32 % edited.size.width, index as u32 / edited.size.width, channel))).abs());
            }
        }
        assert!(worst <= 2.0 / 255.0, "{name} stock display {}/255", worst * 255.0);
        println!("{name}: structured command display {:.6}/255", worst * 255.0);
        let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        assert_eq!(read(&source_path), original);
    }
}

#[test]
#[ignore = "requires reviewed unpublished FREE256 source/edit for hidden and hostile engine staging"]
fn hidden_configurations_locks_and_hostile_forms_are_atomic() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let pins: Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |name: &str| {
        let bytes = fs::read(root.join(name)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), pins[name].as_str().unwrap());
        bytes
    };
    let source = photocraft_io::import("source.xcf", &read("curve-config-matrix/free256-rgb-u8.xcf")).unwrap().document;
    let reference = photocraft_io::import("reference.xcf", &read("curve-config-matrix/free256-rgb-u8.edit.xcf")).unwrap().document;
    let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let config = target.arguments.iter().find(|arg| arg.name == "config").unwrap();
    let valid = wire(&config.value);
    let mut s = session(source.clone());
    s.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"enabled":false})).unwrap();
    let disabled = (*s.active().unwrap().doc).clone();
    let mut invalid = Vec::new();
    let mut bad = valid.clone();
    bad["Config"][0]["value"]["Curve"]["samples_f64_bits"][0] = json!(0_u64);
    invalid.push(bad);
    let mut bad = valid.clone();
    bad["Config"][0]["value"]["Curve"]["samples_f64_bits"][0] = json!("0x7ff8000000000001");
    invalid.push(bad);
    let mut bad = valid.clone();
    bad["Config"][0]["value"]["Curve"]["samples_f64_bits"][0] = json!("0x4000000000000000");
    invalid.push(bad);
    let mut bad = valid.clone();
    bad["Config"][0]["value"]["Curve"]["samples_f64_bits"] = json!([]);
    invalid.push(bad);
    let mut bad = valid.clone();
    bad["Config"][0]["value"]["Curve"]["free"] = json!(false);
    invalid.push(bad);
    let mut bad = valid.clone();
    bad["Config"][1]["name"] = json!("value");
    invalid.push(bad);
    let mut bad = valid.clone();
    bad["Config"][1]["name"] = json!("unknown");
    invalid.push(bad);
    let mut bad = valid.clone();
    bad["Config"][0]["value"]["Curve"].as_object_mut().unwrap().remove("point_types");
    invalid.push(bad);
    invalid.push(json!({"Config":[{"name":"value","value":{"Graph":{}}}]}));
    for value in invalid {
        assert!(s.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"config","value":value}]})).is_err());
        assert_eq!(*s.active().unwrap().doc, disabled);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
    }
    let mut locked = disabled.clone();
    locked.layers[0].locks.pixels = true;
    let mut locked_session = session(locked.clone());
    assert!(locked_session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"config","value":valid}]})).is_err());
    assert_eq!(*locked_session.active().unwrap().doc, locked);
    assert_eq!(locked_session.active().unwrap().history.past_len(), 0);
    s.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"config","value":valid}]})).unwrap();
    let hidden = (*s.active().unwrap().doc).clone();
    assert!(!hidden.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    assert_eq!(hidden.layers, source.layers);
    s.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"enabled":true})).unwrap();
    let activated = (*s.active().unwrap().doc).clone();
    let actual = photocraft_compose::gimp_render::try_render(&activated, activated.bounds(), &mut |_| Ok(())).unwrap();
    let expected = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(actual.px, expected.px);
    let native = photocraft_format::save_to_bytes(&activated, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), activated);
    for expected in [&hidden, &disabled, &source] {
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(&*s.active().unwrap().doc, expected);
    }
    for _ in 0..3 {
        s.execute("edit.redo", json!({})).unwrap();
    }
    assert_eq!(*s.active().unwrap().doc, activated);
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    assert!(photocraft_engine::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&source, &Interrupt::cancel_only(&|| true)).is_err());
    let checks = AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 16;
    assert!(photocraft_engine::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&source, &Interrupt::cancel_only(&cancel)).is_err());
    assert!(checks.load(Ordering::Relaxed) >= 16);
}
