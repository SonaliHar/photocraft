#![cfg(feature = "corpus")]
//! Current zero-argument Absolute activation remains distinct from native pixel edits.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn import(root: &std::path::Path, row: &Value) -> photocraft_doc::Document {
    let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    let hash = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hash, row["sha256"]);
    photocraft_io::import("absolute.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "independent Linear12 source/disabled pairs and current native/history refusals"]
fn registered_absolute_activation_native_history_and_zero_argument_authority() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-absolute-evidence");
    let report: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = report["records"].as_array().unwrap();
    let mut count = 0;
    for row in rows.iter().filter(|r| {
        r["id"].as_str().is_some_and(|s| s.ends_with(".source"))
            && r["precision"].as_str().is_some_and(|s| s.ends_with("_LINEAR") && !s.ends_with("_NON_LINEAR"))
    }) {
        let source = import(&root, row);
        let family = row["id"].as_str().unwrap().strip_suffix(".source").unwrap();
        let disabled = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.disabled")).unwrap());
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        let mut s = Session::new();
        s.add_document(source.clone(), None).unwrap();
        assert!(
            photocraft_engine::xcf_filter_cmds::enable_with_interrupt(
                &mut s,
                &json!({"filterId":id,"enabled":false}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            )
            .is_err()
        );
        assert_eq!(*s.active().unwrap().doc, source);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        let mut locked = source.clone();
        locked
            .xcf_state
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(locked.layers[0].id))
            .unwrap()
            .locks
            .content = true;
        let mut locked_session = Session::new();
        locked_session.add_document(locked.clone(), None).unwrap();
        assert!(locked_session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).is_err());
        assert_eq!(*locked_session.active().unwrap().doc, locked);
        assert_eq!(locked_session.active().unwrap().history.past_len(), 0);

        assert!(s.execute("xcf.filter.set", json!({"filterId":id,"arguments":[]})).is_err());
        for stale in ["0", "01", "18446744073709551615", "18446744073709551616"] {
            assert!(s.execute("xcf.filter.enable", json!({"filterId":stale,"enabled":false})).is_err());
            assert_eq!(*s.active().unwrap().doc, source);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
        }

        assert_eq!(s.active().unwrap().history.past_len(), 0);
        for params in [
            json!({"filterId":id,"arguments":[photocraft_doc::xcf_filters::FilterArgument {name:"unknown".into(),value:photocraft_doc::xcf_filters::FilterValue::Bool(true)}]}),
            json!({"filterId":id,"arguments":{}}),
            json!({"filterId":id,"unknown":true,"arguments":[]}),
        ] {
            assert!(s.execute("xcf.filter.set", params).is_err());
            assert_eq!(*s.active().unwrap().doc, source);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
        }
        s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap();
        let current = (*s.active().unwrap().doc).clone();
        assert_eq!(current.layers, source.layers);
        assert_eq!(current.source_color_context, source.source_color_context);
        assert_eq!(current.xcf_state.as_ref().unwrap().opaque, source.xcf_state.as_ref().unwrap().opaque);
        let actual = photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap();
        let expected = photocraft_compose::gimp_render::try_render(&disabled, disabled.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(actual.px, expected.px);
        let saved = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), current);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, source);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, current);
        s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap();
        let mut expected_reenabled = source.clone();
        expected_reenabled.xcf_state.as_mut().unwrap().filter_stacks[0].revision += 2;
        assert_eq!(*s.active().unwrap().doc, expected_reenabled);
        count += 1;
    }
    assert_eq!(count, 12);
}

#[test]
#[ignore = "independent public Half-to-Float promotion, LinearRGB/Gray current policy and native undo"]
fn absolute_half_linear_promotion_uses_current_policy_and_undo() {
    let base = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let main = base.join("point-absolute-evidence");
    let promotion = base.join("point-absolute-half-promotion-evidence");
    let report: Value = serde_json::from_slice(&std::fs::read(promotion.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in report["records"].as_array().unwrap().iter().filter(|r| r["precision"] == "FLOAT_LINEAR") {
        let original = std::fs::read(main.join(row["original_file"].as_str().unwrap())).unwrap();
        let source = photocraft_io::import("absolute-half.xcf", &original).unwrap().document;
        assert_eq!(source.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Half);
        let expected = import(&promotion, row);
        let mut removed_session = Session::new();
        removed_session.add_document(source.clone(), None).unwrap();
        let fid = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        removed_session.execute("xcf.filter.remove", json!({"filterId":fid})).unwrap();
        let removed = (*removed_session.active().unwrap().doc).clone();
        assert_eq!(removed.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        assert!(removed.xcf_state.as_ref().unwrap().filter_stacks.iter().all(|stack| stack.nodes.is_empty()));
        assert_eq!(removed.layers, source.layers);
        assert_eq!(removed.source_color_context, source.source_color_context);
        let removed_native = photocraft_format::save_to_bytes(&removed, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&removed_native).unwrap(), removed);
        removed_session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*removed_session.active().unwrap().doc, source);
        removed_session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*removed_session.active().unwrap().doc, removed);

        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("image.mode.bits32", json!({})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        assert_eq!(current.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        assert_eq!(current.source_color_context, source.source_color_context);
        assert_eq!(current.layers, source.layers);
        let actual = photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap();
        let target = photocraft_compose::gimp_render::try_render(&expected, expected.bounds(), &mut |_| Ok(())).unwrap();
        for (a, b) in actual.px.iter().zip(&target.px) {
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() <= 1e-6);
            }
        }
        let saved = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), current);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        count += 1;
    }
    assert_eq!(count, 2);
}

#[test]
#[ignore = "independent Linear current profile/encoding/extent/mask/version/alpha hostile states"]
fn absolute_current_domain_refusals_are_atomic() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-absolute-evidence");
    let report: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in report["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["id"].as_str().is_some_and(|s| s.ends_with(".source")) && matches!(r["precision"].as_str(), Some("FLOAT_LINEAR" | "DOUBLE_LINEAR")))
    {
        let source = import(&root, row);
        let fid = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        for mutation in 0..10 {
            let mut bad = source.clone();
            match mutation {
                0 => bad.size = photocraft_geom::Size::new(9, 6),
                1 => bad.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile),
                2 => bad.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                3 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "future".into(),
                4 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "future".into(),
                5 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0,
                6 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                7 => bad.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.5]).unwrap(),
                _ => {
                    let channels = bad.pixel_format().mode.color_channels();
                    let mut values = vec![0.5; channels + 1];
                    values[if mutation == 8 { 0 } else { channels }] = if mutation == 8 { f64::INFINITY } else { 1.125 };
                    bad.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &values).unwrap();
                }
            }
            assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&bad).is_err(), "{}/mutation{mutation}", row["id"]);
            let mut session = Session::new();
            session.add_document(bad.clone(), None).unwrap();
            assert!(session.execute("xcf.filter.enable", json!({"filterId":fid,"enabled":false})).is_err(), "{}/mutation{mutation}", row["id"]);
            assert_eq!(*session.active().unwrap().doc, bad);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            count += 1;
        }
    }
    assert_eq!(count, 40);
}
