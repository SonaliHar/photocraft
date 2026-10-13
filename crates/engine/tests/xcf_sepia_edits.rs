#![cfg(feature = "corpus")]
//! Actual independent Sepia Bool/Float32 edits preserve source/native and undo.
use photocraft_doc::{ColorMode, Document, SampleEncoding, SampleType, Size};
use photocraft_doc::{
    item_identity::FilterId,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_engine::Session;
use serde_json::json;
fn session(doc: Document) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    s
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Sepia RGB precision coupled edits"]
fn independent_sepia_rgb_precision_edits_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    use sha2::{Digest, Sha256};
    let binding = std::fs::read(root.join("sepia-precision-evidence/batch-manifest.json")).unwrap();
    assert_eq!(
        Sha256::digest(&binding).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "f12cc00111e3505e351e1a234916cad4b9ea38f577aa01127e3192792386d374"
    );
    let manifest: serde_json::Value = serde_json::from_slice(&binding).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        let name = path.strip_prefix("sepia-precision-evidence/").unwrap();
        let binding = manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == name).unwrap();
        assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), binding["sha256"].as_str().unwrap());
        bytes
    };
    let observations: serde_json::Value = serde_json::from_slice(&read("sepia-precision-evidence/observations.json")).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let source_path = format!("sepia-precision-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut locked = source.clone();
        locked.layers[0].locks.pixels = true;
        let owner = photocraft_doc::item_identity::ItemRef::Layer(locked.layers[0].id);
        locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == owner).unwrap().locks.content = true;
        let mut locked_session = session(locked.clone());
        assert!(
            locked_session
                .execute(
                    "xcf.filter.set",
                    json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"scale","value":{"Number":{"Float32Bits":0.5_f32.to_bits()}}}]})
                )
                .is_err()
        );
        assert_eq!(*locked_session.active().unwrap().doc, locked);
        assert_eq!(locked_session.active().unwrap().history.past_len(), 0);
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("scale", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("srgb", json!({"Number":{"Float32Bits":4.0_f32.to_bits()}})),
            ("scale", json!({"Number":{"Float32Bits":(-1.0_f32).to_bits()}})),
            ("srgb", json!({"Number":{"Float64Bits":1.0_f64.to_bits().to_string()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"scale","value":{"Number":{"Float32Bits":0.0625_f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in ["coupled", "linear", "disabled"] {
            let reference = photocraft_io::import("edit.xcf", &read(&format!("sepia-precision-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            assert_eq!(
                arguments.len(),
                match stage {
                    "disabled" => 0,
                    "coupled" => 1,
                    _ => 2,
                }
            );
            let params = if stage == "disabled" {
                json!({"filterId":node.id.0.to_string(),"enabled":false})
            } else {
                json!({"filterId":node.id.0.to_string(),"arguments":arguments})
            };
            let mut s = session(source.clone());
            let command = if stage == "disabled" { "xcf.filter.enable" } else { "xcf.filter.set" };
            s.execute(command, params).unwrap();
            let edited = (*s.active().unwrap().doc).clone();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            let state = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(state.opaque, original.opaque);
            assert_eq!(state.provenance, original.provenance);
            assert_eq!(state.filter_masks, original.filter_masks);
            assert_eq!(state.filter_stacks[0].nodes[0].arguments, target.arguments);
            assert_eq!(state.filter_stacks[0].nodes[0].enabled, target.enabled);
            let actual = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
            let direct = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.px, direct.px);
            let projection = read(&format!("sepia-precision-evidence/{family}.{stage}.fresh.projection.bin"));
            assert_eq!(projection.len(), 48 * 32);
            let mut worst = 0.0_f64;
            for (pixel, reference) in actual.px.iter().zip(projection.as_chunks::<32>().0.iter()) {
                let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                assert!((pixel[3] - alpha).abs() <= 1.0 / 65535.0);
                for channel in 0..3 {
                    let mut value = if edited.depth == SampleType::U16 { pixel[channel].clamp(0.0, 1.0) } else { pixel[channel] };
                    if edited.sample_encoding == Some(SampleEncoding::Linear) {
                        value = photocraft_cms::curve::srgb_trc().eval_inverse64(value);
                    }
                    let expected = f64::from_le_bytes(reference[channel * 8..channel * 8 + 8].try_into().unwrap());
                    worst = worst.max((value * pixel[3] - expected * alpha).abs());
                    if alpha > 0.0 {
                        assert!((value - expected).abs() <= 2.0 / 255.0);
                    }
                }
            }
            println!("{family} {stage} actual Sepia RGB command {}/255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, source);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, edited);
            assert_eq!(read(&source_path), bytes);
            count += 1;
        }
    }
    assert_eq!(count, 18);
}

#[test]
#[ignore = "local unpublished Sepia changed-current domain negatives"]
fn sepia_precision_changed_contexts_reject_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    use sha2::{Digest, Sha256};
    let binding = std::fs::read(root.join("sepia-precision-evidence/batch-manifest.json")).unwrap();
    assert_eq!(
        Sha256::digest(&binding).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "f12cc00111e3505e351e1a234916cad4b9ea38f577aa01127e3192792386d374"
    );
    let manifest: serde_json::Value = serde_json::from_slice(&binding).unwrap();
    for family in ["rgb-float_linear", "rgb-double_non_linear"] {
        let path = format!("sepia-precision-evidence/{family}.source.xcf");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        let name = path.strip_prefix("sepia-precision-evidence/").unwrap();
        let expected = manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == name).unwrap();
        assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), expected["sha256"].as_str().unwrap());
        let original = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        photocraft_compose::gimp_render::try_render(&original, original.bounds(), &mut |_| Ok(())).unwrap();
        for mutation in [
            "mask-depth",
            "mask-nonzero",
            "mask-negative-zero",
            "mask-owner",
            "mask-absent",
            "epoch",
            "argument-type",
            "hdr",
            "nonfinite",
            "icc-origin",
            "geometry",
        ] {
            let mut changed = original.clone();
            match mutation {
                "mask-depth" => {
                    changed.xcf_state.as_mut().unwrap().filter_masks[0].surface =
                        photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false))
                }
                "mask-nonzero" | "mask-negative-zero" => {
                    let value = if mutation == "mask-nonzero" { 0.5 } else { -0.0 };
                    changed.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[value]).unwrap();
                }
                "mask-owner" => changed.xcf_state.as_mut().unwrap().filter_masks[0].owner = FilterId(u64::MAX),
                "mask-absent" => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                "epoch" => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "unqualified".into(),
                "argument-type" => {
                    changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Float64Bits(1.0_f64.to_bits()))
                }
                "hdr" | "nonfinite" => {
                    let value = if mutation == "hdr" { 2.0 } else { f64::INFINITY };
                    let pixel = if changed.mode == ColorMode::Grayscale { vec![value, 1.0] } else { vec![value, 0.25, 0.5, 1.0] };
                    changed.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &pixel).unwrap();
                }
                "icc-origin" => changed.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                "geometry" => changed.size = Size::new(9, 6),
                _ => unreachable!(),
            }
            let id = changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
            let native = photocraft_format::save_to_bytes(&changed, &Default::default()).ok();
            let mut s = session(changed.clone());
            assert!(s.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{family} {mutation}");
            assert_eq!(*s.active().unwrap().doc, changed);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            if let Some(native) = native {
                assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), native);
            }
        }
        assert_eq!(std::fs::read(root.join(path)).unwrap(), bytes);
    }
}
