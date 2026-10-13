#![cfg(feature = "corpus")]
use photocraft_doc::Document;
use photocraft_engine::Session;
use serde_json::json;
fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/video-degradation-edit-evidence")
}
fn load(name: &str) -> Document {
    photocraft_io::import("own-video.xcf", &std::fs::read(root().join(format!("{name}.xcf"))).unwrap()).unwrap().document
}
fn unchanged(current: &Document, original: &Document) {
    assert_eq!(current.layers, original.layers);
    assert_eq!(current.source_color_context, original.source_color_context);
    let (a, b) = (current.xcf_state.as_ref().unwrap(), original.xcf_state.as_ref().unwrap());
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.resources, b.resources);
}
fn same_projection(a: &Document, b: &Document) {
    let a = photocraft_compose::gimp_render::try_render(a, a.bounds(), &mut |_| Ok(())).unwrap();
    let b = photocraft_compose::gimp_render::try_render(b, b.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(a.px.len(), 256);
    assert_eq!(a.px, b.px);
}
#[test]
#[ignore = "independent saved source/edit/disabled current command and history controls"]
fn registered_video_edit_disable_native_and_history() {
    let source = load("source");
    let edited = load("edit");
    let disabled = load("disabled");
    let original = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(original));
    let id = original.id.0.to_string();
    let arguments = edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.clone();
    let mut session = Session::new();
    session.add_document(source.clone(), None).unwrap();
    session.execute("xcf.filter.set", json!({"filterId":id,"arguments":arguments})).unwrap();
    let current = (*session.active().unwrap().doc).clone();
    unchanged(&current, &source);
    same_projection(&current, &edited);
    assert_eq!(session.active().unwrap().history.past_len(), 1);
    let saved = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), current);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, source);
    for mutation in 0..4 {
        let mut arguments = original.arguments.clone();
        let pattern = arguments.iter_mut().find(|argument| argument.name == "pattern").unwrap();
        pattern.value = match mutation {
            0 => photocraft_doc::xcf_filters::FilterValue::Enum { domain: "gegl:video-degradation/pattern".into(), value: 9 },
            1 => photocraft_doc::xcf_filters::FilterValue::Enum { domain: "wrong-domain".into(), value: 5 },
            2 => photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(5f32.to_bits())),
            _ => photocraft_doc::xcf_filters::FilterValue::Bool(true),
        };
        let history = session.active().unwrap().history.past_len();
        assert!(session.execute("xcf.filter.set", json!({"filterId":id,"arguments":arguments})).is_err());
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, current);
    let history = session.active().unwrap().history.past_len();
    assert!(session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).is_err());
    assert_eq!(*session.active().unwrap().doc, current);
    assert_eq!(session.active().unwrap().history.past_len(), history);
    session.execute("edit.undo", json!({})).unwrap();
    session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap();
    let current = (*session.active().unwrap().doc).clone();
    unchanged(&current, &source);
    same_projection(&current, &disabled);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, source);
    let mut refused = 0;
    for pattern in 0..9 {
        for additive in [false, true] {
            for rotated in [false, true] {
                if (pattern, additive, rotated) == (5, false, false) || (pattern, additive, rotated) == (2, true, true) {
                    continue;
                }
                let mut arguments = original.arguments.clone();
                for argument in &mut arguments {
                    argument.value = match argument.name.as_str() {
                        "pattern" => photocraft_doc::xcf_filters::FilterValue::Enum { domain: "gegl:video-degradation/pattern".into(), value: pattern },
                        "additive" => photocraft_doc::xcf_filters::FilterValue::Bool(additive),
                        "rotated" => photocraft_doc::xcf_filters::FilterValue::Bool(rotated),
                        _ => unreachable!(),
                    };
                }
                let history = session.active().unwrap().history.past_len();
                assert!(session.execute("xcf.filter.set", json!({"filterId":id,"arguments":arguments})).is_err());
                assert_eq!(*session.active().unwrap().doc, source);
                assert_eq!(session.active().unwrap().history.past_len(), history);
                refused += 1;
            }
        }
    }
    assert_eq!(refused, 34);
    for bad in [
        json!({"filterId":id,"arguments":[{"name":"pattern","value":{"Enum":{"domain":"gegl:video-degradation/pattern","value":9}}}]}),
        json!({"filterId":id,"arguments":[]}),
        json!({"filterId":id,"arguments":"bad"}),
        json!({"filterId":id,"enabled":"bad"}),
    ] {
        let command = if bad.get("enabled").is_some() { "xcf.filter.enable" } else { "xcf.filter.set" };
        let history = session.active().unwrap().history.past_len();
        assert!(session.execute(command, bad).is_err());
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
}
