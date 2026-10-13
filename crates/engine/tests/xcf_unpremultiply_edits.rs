#![cfg(feature = "corpus")]
//! Activation/current-state authority for the witnessed zero-argument family.
use photocraft_doc::Document;
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn packet() -> (std::path::PathBuf, Value) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-unpremultiply-evidence");
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(
        Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "f40f522c4e8b7e7bab1e00b4e2d5fd2729c90062fbc36cb93172f362cd406f3d"
    );
    (root, serde_json::from_slice(&bytes).unwrap())
}
fn import(root: &std::path::Path, row: &Value) -> Document {
    let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), row["sha256"]);
    photocraft_io::import("unpremultiply.xcf", &bytes).unwrap().document
}
fn render(doc: &Document) -> Vec<[f64; 4]> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap().px
}
fn untouched(current: &Document, source: &Document) {
    assert_eq!(current.layers, source.layers);
    assert_eq!(current.icc_profile, source.icc_profile);
    assert_eq!(current.source_color_context, source.source_color_context);
    assert_eq!(current.working_profile_interpretation, source.working_profile_interpretation);
    let (a, b) = (current.xcf_state.as_ref().unwrap(), source.xcf_state.as_ref().unwrap());
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.resources, b.resources);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.items, b.items);
}
fn native(doc: &Document) {
    let saved = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), *doc);
    let (stream, _) = photocraft_format::PcraftWriter::new()
        .save_zip_stream(doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
        .unwrap();
    let reopened = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(reopened, *doc);
    assert_eq!(render(&reopened), render(doc));
}
#[test]
#[ignore = "independent source/edit/disabled activation, cancellation, source exactness, native and history"]
fn actual_activation_disable_native_history_and_zero_argument_authority() {
    let (root, report) = packet();
    let rows = report["records"].as_array().unwrap();
    let pick =
        |suffix: &str| rows.iter().find(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR" && r["id"].as_str().unwrap().ends_with(suffix)).unwrap();
    let source = import(&root, pick(".source"));
    let edited = import(&root, pick(".edit"));
    let disabled = import(&root, pick(".disabled"));
    assert_ne!(render(&source), render(&edited));
    assert_eq!(
        source.layers[0].surface().unwrap().try_read_interleaved(source.bounds()).unwrap(),
        disabled.layers[0].surface().unwrap().try_read_interleaved(disabled.bounds()).unwrap()
    );
    assert_ne!(render(&source), render(&disabled));
    let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let mut session = Session::new();
    session.add_document(source.clone(), None).unwrap();
    assert!(
        photocraft_engine::xcf_filter_cmds::enable_with_interrupt(
            &mut session,
            &json!({"filterId":id,"enabled":false}),
            &photocraft_raster::Interrupt::cancel_only(&|| true)
        )
        .is_err()
    );
    assert_eq!(*session.active().unwrap().doc, source);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    for params in [
        json!({"filterId":id,"arguments":[]}),
        json!({"filterId":id,"arguments":[{"name":"invented","value":{"Bool":true}}]}),
        json!({"filterId":id,"arguments":{}}),
    ] {
        assert!(session.execute("xcf.filter.set", params).is_err());
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    for stale in ["0", "01", "18446744073709551615", "18446744073709551616"] {
        assert!(session.execute("xcf.filter.enable", json!({"filterId":stale,"enabled":false})).is_err());
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap();
    let current = (*session.active().unwrap().doc).clone();
    untouched(&current, &source);
    assert_eq!(render(&current), render(&disabled));
    native(&current);
    assert_eq!(session.active().unwrap().history.past_len(), 1);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, source);
    assert!(session.active().unwrap().history.can_redo());
    assert!(
        photocraft_engine::xcf_filter_cmds::enable_with_interrupt(
            &mut session,
            &json!({"filterId":id,"enabled":false}),
            &photocraft_raster::Interrupt::cancel_only(&|| true)
        )
        .is_err()
    );
    assert_eq!(*session.active().unwrap().doc, source);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    assert!(session.active().unwrap().history.can_redo());
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, current);
    session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap();
    let enabled = (*session.active().unwrap().doc).clone();
    let mut expected = source.clone();
    expected.xcf_state.as_mut().unwrap().filter_stacks[0].revision += 2;
    assert_eq!(enabled, expected);
    untouched(&enabled, &source);
    assert_eq!(render(&enabled), render(&source));
    native(&enabled);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, current);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, enabled);
}

#[test]
#[ignore = "mutable current depth/encoding/profile/geometry/raw flags/masks/native hostile controls stay atomic"]
fn current_hostile_domains_refuse_without_history_or_source_mutation() {
    let (root, report) = packet();
    let row = report["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR" && r["id"].as_str().unwrap().ends_with(".source"))
        .unwrap();
    let source = import(&root, row);
    let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    for mutation in 0..26 {
        let mut bad = source.clone();
        let bounds = bad.bounds();
        match mutation {
            0 => bad.depth = photocraft_doc::SampleType::F64,
            1 => bad.sample_encoding = Some(photocraft_doc::SampleEncoding::Perceptual),
            2 => bad.mode = photocraft_doc::ColorMode::Grayscale,
            3 => bad.icc_profile = None,
            4 => bad.working_profile_interpretation = Default::default(),
            5 => bad.size = photocraft_doc::Size::new(9, 6),
            6 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "future".into(),
            7 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "xcf-22-stock-absolute-linear-precision-v1".into(),
            8 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_blend_space = 1574142545,
            9 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_composite_space = 1574142545,
            10 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 1574142545,
            11 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_clipping = Some(1),
            12 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
            13 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits = 0.5f64.to_bits(),
            14 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0]
                .arguments
                .push(photocraft_doc::xcf_filters::FilterArgument { name: "invented".into(), value: photocraft_doc::xcf_filters::FilterValue::Bool(true) }),
            15 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
            16 => {
                bad.xcf_state
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(bad.layers[0].id))
                    .unwrap()
                    .locks
                    .content = true
            }
            17 => {
                let mut bytes = bad.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
                bytes[..4].copy_from_slice(&f32::INFINITY.to_ne_bytes());
                bad.layers[0].surface_mut().unwrap().try_write_interleaved(bounds, &bytes).unwrap();
            }
            18 => {
                let mut bytes = bad.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
                bytes[12..16].copy_from_slice(&2f32.to_ne_bytes());
                bad.layers[0].surface_mut().unwrap().try_write_interleaved(bounds, &bytes).unwrap();
            }
            19 => {
                let mask = &mut bad.xcf_state.as_mut().unwrap().filter_masks[0];
                let mut bytes = mask.surface.try_read_interleaved(mask.bounds).unwrap();
                bytes[..4].copy_from_slice(&(-0f32).to_ne_bytes());
                mask.surface.try_write_interleaved(mask.bounds, &bytes).unwrap();
            }
            20 => {
                let mask = &mut bad.xcf_state.as_mut().unwrap().filter_masks[0];
                let mut bytes = mask.surface.try_read_interleaved(mask.bounds).unwrap();
                bytes[..4].copy_from_slice(&0.5f32.to_ne_bytes());
                mask.surface.try_write_interleaved(mask.bounds, &bytes).unwrap();
            }
            21 => bad.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.mask.is_some()).unwrap().mask.as_mut().unwrap().shown = true,
            22 => {
                let composition = bad.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].composition.clone();
                bad.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.mask.is_some()).unwrap().composition = composition;
            }
            23 => {
                bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch =
                    "xcf-22-stock-point-replace-fields-v1".into()
            }
            24 => {
                bad.xcf_state
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(bad.layers[0].id))
                    .unwrap()
                    .raster_bounds = Some(photocraft_doc::Rect::new(0, 0, 7, 6))
            }
            _ => {
                let mut bytes = bad.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
                bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes());
                bad.layers[0].surface_mut().unwrap().try_write_interleaved(bounds, &bytes).unwrap();
            }
        }
        let mut session = Session::new();
        session.add_document(bad.clone(), None).unwrap();
        assert!(session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).is_err(), "mutation{mutation}");
        assert_eq!(*session.active().unwrap().doc, bad);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
}

#[test]
#[ignore = "Surface API current native preparation against independent edit; enable overflow is atomic"]
fn actual_surface_api_current_native_and_activation_overflow() {
    let (root, report) = packet();
    let rows = report["records"].as_array().unwrap();
    let row = rows.iter().find(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR" && r["id"].as_str().unwrap().ends_with(".disabled")).unwrap();
    let source = import(&root, row);
    let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let edit_row = rows.iter().find(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR" && r["id"].as_str().unwrap().ends_with(".edit")).unwrap();
    let edited = import(&root, edit_row);
    let mut current = source.clone();
    let bounds = current.bounds();
    let edit_bits = edited.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    let original_bits = source.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    assert_ne!(edit_bits, original_bits);
    // Surface API current preparation, not generic paint/pixel command qualification.
    current.layers[0].surface_mut().unwrap().try_write_interleaved(bounds, &edit_bits).unwrap();
    assert_eq!(source.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), original_bits);
    let mut current_session = Session::new();
    current_session.add_document(current.clone(), None).unwrap();
    current_session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap();
    let enabled_current = (*current_session.active().unwrap().doc).clone();
    assert_eq!(render(&enabled_current), render(&edited));
    untouched(&enabled_current, &current);
    native(&enabled_current);
    assert_eq!(enabled_current.xcf_state.as_ref().unwrap().provenance, source.xcf_state.as_ref().unwrap().provenance);
    current_session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*current_session.active().unwrap().doc, current);
    current_session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*current_session.active().unwrap().doc, enabled_current);
    let mut overflow = source.clone();
    let bounds = overflow.bounds();
    let mut bits = overflow.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    bits[..4].copy_from_slice(&f32::MAX.to_ne_bytes());
    bits[12..16].copy_from_slice(&0.5f32.to_ne_bytes());
    overflow.layers[0].surface_mut().unwrap().try_write_interleaved(bounds, &bits).unwrap();
    assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&overflow).is_ok());
    let mut session = Session::new();
    session.add_document(overflow.clone(), None).unwrap();
    assert!(session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).is_err());
    assert_eq!(*session.active().unwrap().doc, overflow);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    assert_eq!(session.active().unwrap().doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), bits);
}
