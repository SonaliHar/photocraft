use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use sha2::{Digest, Sha256};

const GEOMETRY_EPOCH: &str = "xcf-22-stock-component-extract-geometry-v1";
const CHOICES: [(&str, &str); 11] = [
    ("rgb-r", "Red"),
    ("rgb-g", "Green"),
    ("rgb-b", "Blue"),
    ("cmyk-c", "Cyan"),
    ("cmyk-m", "Magenta"),
    ("cmyk-y", "Yellow"),
    ("cmyk-k", "Black"),
    ("ycbcr-y", "Y'CbCr Y'"),
    ("ycbcr-cb", "Y'CbCr Cb"),
    ("ycbcr-cr", "Y'CbCr Cr"),
    ("alpha", "Alpha"),
];

#[test]
fn geometry_component_schema_keeps_unqualified_choices_and_malformed_arguments_closed() {
    use photocraft_doc::xcf_filters::{FilterArgument, FilterValue};
    let valid = vec![
        FilterArgument { name: "component".into(), value: FilterValue::Enum { domain: component_extract::DOMAIN.into(), value: 0 } },
        FilterArgument { name: "invert".into(), value: FilterValue::Bool(false) },
        FilterArgument { name: "linear".into(), value: FilterValue::Bool(false) },
    ];
    for value in -1..=21 {
        let mut candidate = valid.clone();
        candidate[0].value = FilterValue::Enum { domain: component_extract::DOMAIN.into(), value };
        assert_eq!(component_extract::valid_arguments_for_epoch(GEOMETRY_EPOCH, &candidate), matches!(value, 0 | 1 | 2 | 8..=14 | 20));
    }
    for index in 0..3 {
        let mut duplicate = valid.clone();
        duplicate.push(valid[index].clone());
        assert!(!component_extract::valid_arguments_for_epoch(GEOMETRY_EPOCH, &duplicate));
        let mut wrong = valid.clone();
        wrong[index].value = FilterValue::String("true".into());
        assert!(!component_extract::valid_arguments_for_epoch(GEOMETRY_EPOCH, &wrong));
        wrong[index] = valid[index].clone();
        wrong[index].name = "unknown".into();
        assert!(!component_extract::valid_arguments_for_epoch(GEOMETRY_EPOCH, &wrong));
    }
    assert!(!component_extract::valid_arguments_for_epoch(GEOMETRY_EPOCH, &valid[..2]));
    assert!(!component_extract::valid_arguments_for_epoch("unknown", &valid));
    let mut wrong_domain = valid;
    wrong_domain[0].value = FilterValue::Enum { domain: "unknown".into(), value: 0 };
    assert!(!component_extract::valid_arguments_for_epoch(GEOMETRY_EPOCH, &wrong_domain));
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn packet() -> std::path::PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("point-component-extract-geometry-evidence")
}
fn original(root: &std::path::Path, stem: &str) -> photocraft_doc::Document {
    let bytes = std::fs::read(root.join(format!("{stem}.xcf"))).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
    let row = manifest["cases"].as_array().unwrap().iter().find(|r| r["id"] == stem).unwrap();
    assert_eq!(row["sha256"], hash(&bytes));
    photocraft_io::import("geometry.xcf", &bytes).unwrap().document
}
fn projection(root: &std::path::Path, stem: &str, doc: &photocraft_doc::Document) {
    use std::io::Read;
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{stem}.fresh.state.json"))).unwrap()).unwrap();
    let desc = &state["projection"];
    let input = std::fs::read(root.join(desc["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&input), desc["sha256"]);
    let len = desc["decoded_bytes"].as_u64().unwrap();
    assert!(len <= 2 << 20);
    let mut expected = Vec::new();
    flate2::read::ZlibDecoder::new(input.as_slice()).take(len + 1).read_to_end(&mut expected).unwrap();
    assert_eq!(expected.len() as u64, len);
    assert_eq!(hash(&expected), desc["decoded_sha256"]);
    let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(rendered.px.len() * 32, expected.len());
    for (pixel, raw) in rendered.px.iter().zip(expected.as_chunks::<32>().0) {
        let values: [f64; 4] = std::array::from_fn(|i| f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap()));
        assert!((pixel[3] - values[3]).abs() <= 1. / 65535.);
        for i in 0..3 {
            assert!((pixel[i] * pixel[3] - values[i] * values[3]).abs() <= 2. / 255., "{stem}");
        }
    }
}
fn verify(root: &std::path::Path, stem: &str, current: &photocraft_doc::Document, source: &photocraft_doc::Document) {
    let reference = original(root, stem);
    assert_eq!(current.layers, source.layers);
    assert_eq!(current.source_color_context, source.source_color_context);
    let state = current.xcf_state.as_ref().unwrap();
    let retained = source.xcf_state.as_ref().unwrap();
    assert_eq!(state.provenance, retained.provenance);
    assert_eq!(state.opaque, retained.opaque);
    assert_eq!(state.filter_masks, retained.filter_masks);
    assert_eq!(state.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
    assert_eq!(state.filter_stacks[0].nodes[0].enabled, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    projection(root, stem, current);
    assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap(), *current);
    let mut archive = std::io::Cursor::new(Vec::new());
    photocraft_io::save_native_stream(current, &mut archive, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    let reopened = photocraft_format::load_from_stream(archive, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(reopened, *current);
    projection(root, stem, &reopened);
}
#[test]
#[ignore = "requires reviewed unpublished independent132 Component geometry originals"]
fn original_component_geometry_widgets_reference_native_history_and_locks() {
    let root = packet();
    assert_eq!(hash(&std::fs::read(root.join("batch-manifest.json")).unwrap()), "fe13672593245e18d8224faec99c18a3bfcd10cd5860427f60494e5d29396e88");
    for model in ["rgb", "gray"] {
        for geometry in ["negative-partial", "tile-crossing"] {
            let prefix = format!("{model}.float_non_linear.{geometry}");
            let stem = format!("{prefix}.rgb-r.source");
            let source_bytes = std::fs::read(root.join(format!("{stem}.xcf"))).unwrap();
            let source = original(&root, &stem);
            let layer = source.layers[0].id;
            let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(node.operation.schema_epoch, GEOMETRY_EPOCH);
            let caption = node.source_display_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| node.operation.name.clone());
            let mut session = photocraft_engine::Session::new();
            session.add_document(source.clone(), None).unwrap();
            let app = PhotocraftApp::new(session, crate::Services::default());
            let builder = Harness::builder().with_size(egui::vec2(480., 760.));
            let builder = if std::env::var_os("PHOTOCRAFT_COMPONENT_GEOMETRY_UI_VISUALS").is_some() {
                let mut setup = egui_kittest::wgpu::default_wgpu_setup();
                if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                    create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
                }
                builder.wgpu_setup(setup)
            } else {
                builder
            };
            let mut h = builder.build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
            h.run_steps(2);
            assert!(!h.get_by_label("Invert").accesskit_node().is_disabled(), "geometry epoch must expose editable controls");
            h.get_by_label("Component").click();
            h.run_steps(2);
            for (_, label) in CHOICES {
                h.get_by_label(label);
            }
            h.get_by_label("Red").click();
            h.run_steps(2);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
            verify(&root, &stem, &h.state().session.active().unwrap().doc, &source);
            for (nick, label) in CHOICES.into_iter().skip(1) {
                h.get_by_label("Component").click();
                h.run_steps(2);
                if matches!(nick, "ycbcr-y" | "ycbcr-cb" | "ycbcr-cr" | "alpha") {
                    // Lower rows are clipped by the eleven-choice popup. Use
                    // the real widget action rather than stale mouse bounds.
                    h.get_by_label(label).scroll_to_me();
                    h.run();
                    h.get_by_label(label).click_accesskit();
                    h.run_steps(2);
                    // AccessKit selection does not emit the pointer click
                    // used by the popup's close-on-click behavior.
                    h.key_press(egui::Key::Escape);
                } else {
                    h.get_by_label(label).click();
                }
                h.run_steps(2);
                assert_eq!(
                    h.state().session.active().unwrap().history.past_len(),
                    1,
                    "{prefix}/{nick}: {} arguments {:?}",
                    h.state().ui.status,
                    h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments
                );
                let current = h.state().session.active().unwrap().doc.clone();
                verify(&root, &format!("{prefix}.{nick}.source"), &current, &source);
                h.state_mut().run("edit.undo", json!({})).unwrap();
                assert_eq!(*h.state().session.active().unwrap().doc, source);
                h.state_mut().run("edit.redo", json!({})).unwrap();
                assert_eq!(*h.state().session.active().unwrap().doc, *current);
                h.state_mut().run("edit.undo", json!({})).unwrap();
                h.run_steps(2);
            }
            h.get_by_label("Invert").click();
            h.run_steps(2);
            h.get_by_label("Linear").click();
            h.run_steps(2);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 2);
            verify(&root, &format!("{prefix}.rgb-r.both"), &h.state().session.active().unwrap().doc, &source);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            h.state_mut().run("edit.undo", json!({})).unwrap();
            h.run_steps(2);
            h.get_by_label(&caption).click();
            h.run_steps(2);
            verify(&root, &format!("{prefix}.rgb-r.disabled"), &h.state().session.active().unwrap().doc, &source);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            h.run_steps(2);
            if let Some(out) = std::env::var_os("PHOTOCRAFT_COMPONENT_GEOMETRY_UI_VISUALS") {
                let out = std::path::PathBuf::from(out);
                std::fs::create_dir_all(&out).unwrap();
                h.render().unwrap().save(out.join(format!("{model}-{geometry}.png"))).unwrap();
            }
            let active = h.state_mut().session.active_mut().unwrap();
            let doc = active.doc.try_resident_mut().unwrap();
            doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
            h.run_steps(2);
            for label in ["Component", "Invert", "Linear", &caption] {
                assert!(h.get_by_label(label).accesskit_node().is_disabled());
            }
            assert_eq!(std::fs::read(root.join(format!("{stem}.xcf"))).unwrap(), source_bytes);
        }
    }
}
