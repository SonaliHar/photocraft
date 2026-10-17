//! Explicit composition policy edits remain distinct from immutable source evidence.
use photocraft_doc::{BlendMode, ColorMode, Document, Layer, PixelFormat, SampleEncoding, SampleType, Size, XcfState};
use photocraft_doc::{
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::ItemRef,
};
use photocraft_engine::Session;
use serde_json::json;
fn fixture() -> Document {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let mut doc = Document::new("Own policy edit", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(photocraft_cms::builtin::Builtin::Srgb.profile().to_bytes());
    let mut bottom = Layer::raster("Own bottom", format);
    let mut top = Layer::raster("Own top", format);
    bottom.surface_mut().unwrap().try_write_region_f64(doc.bounds(), &[0.25, 0.5, 0.75, 0.5, 2.0, -0.125, 0.25, 0.75]).unwrap();
    top.surface_mut().unwrap().try_write_region_f64(doc.bounds(), &[0.75, 0.25, 0.5, 0.5, 1.25, 0.0625, 0.75, 0.25]).unwrap();
    doc.layers = vec![bottom, top];
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    for item in &mut state.items {
        item.source_flags = 0x8123_4567;
        item.raster_bounds = Some(doc.bounds());
        item.composition = Some(GimpComposition {
            compatibility_epoch: "own-policy-test".into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 2,
            stored_composite_space: 2,
            stored_operator: 1,
            pass_through: false,
            resolved: Some(ResolvedGimpComposition {
                blend_space: GimpSpace::ProfileRgb,
                composite_space: GimpSpace::ProfileRgb,
                operator: GimpOperator::Union,
            }),
        });
    }
    doc.xcf_state = Some(state);
    doc
}
fn params(doc: &Document) -> serde_json::Value {
    json!({"layerId":doc.layers[1].id.0.to_string(),"sourceMode":30,"legacy":false,"blendSpace":"profile-rgb","compositeSpace":"profile-rgb","operator":"union"})
}
#[test]
fn late_composition_cancellation_preserves_document_and_existing_redo() {
    let before = fixture();
    let p = params(&before);
    let mut s = session(before.clone());
    s.execute("xcf.composition.set", p.clone()).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 >= 12;
    let control = photocraft_raster::Interrupt::cancel_only(&cancel);
    assert!(photocraft_engine::xcf_composition_cmds::set_with_interrupt(&mut s, &p, &control).is_err());
    assert_eq!(*s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[1].blend, BlendMode::Multiply);
}
#[test]
fn composition_refusal_preserves_even_a_stale_floating_preview() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":3,"height":2,"background":"white"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
    s.active_mut().unwrap().floating.as_mut().unwrap().revision = u64::MAX;
    let active = s.active().unwrap();
    let before = active.doc.clone();
    let preview = active.floating.clone().unwrap();
    let revision = active.revision;
    let layer = active.active_layer.unwrap();
    let error = s
        .execute(
            "xcf.composition.set",
            json!({"layerId":layer.0.to_string(),"sourceMode":30,"legacy":false,
        "blendSpace":"profile-rgb","compositeSpace":"profile-rgb","operator":"union"}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("floating"));
    let after = s.active().unwrap();
    assert_eq!(after.doc, before);
    assert_eq!(after.revision, revision);
    assert!(std::sync::Arc::ptr_eq(&after.floating.as_ref().unwrap().parts, &preview.parts));
    assert_eq!(after.floating.as_ref().unwrap().revision, preview.revision);
}
fn session(doc: Document) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    s
}
#[test]
fn captured_composition_rejects_stale_or_malformed_pairs_without_history() {
    let mut s = session(fixture());
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let base = params(&before);
    let document = before.id.0.to_string();
    for (id, rev) in [
        (Some(document.clone()), None),
        (None, Some(revision.to_string())),
        (Some(format!("0{document}")), Some(revision.to_string())),
        (Some(document.clone()), Some(format!("0{}", revision))),
        (Some(document.clone()), Some(revision.saturating_add(1).to_string())),
        (Some("18446744073709551616".into()), Some(revision.to_string())),
    ] {
        let mut request = base.clone();
        if let Some(id) = id {
            request["documentId"] = json!(id);
        }
        if let Some(rev) = rev {
            request["expectedRevision"] = json!(rev);
        }
        assert!(s.execute("xcf.composition.set", request).is_err());
        let active = s.active().unwrap();
        assert_eq!(active.doc, before);
        assert_eq!(active.revision, revision);
        assert_eq!(active.history.past_len(), 0);
    }
    let mut request = base;
    request["documentId"] = json!(document);
    request["expectedRevision"] = json!(revision.to_string());
    s.execute("xcf.composition.set", request).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
}
#[test]
fn explicit_policy_edit_inspects_preserves_native_samples_and_undoes() {
    let before = fixture();
    let id = before.layers[1].id;
    let raw = before.layers[1].surface().unwrap().try_read_interleaved(before.bounds()).unwrap();
    let p = params(&before);
    let mut s = session(before.clone());
    let info = s.execute("xcf.composition.inspect", json!({"layerId":id.0.to_string()})).unwrap();
    assert_eq!(info["composition"]["source_mode"], 28);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
    s.execute("xcf.composition.set", p).unwrap();
    let edited = (*s.active().unwrap().doc).clone();
    assert_eq!(edited.layers[1].blend, BlendMode::Multiply);
    let item = edited.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap();
    assert_eq!(item.source_flags, 0x8123_4567);
    assert_eq!(item.composition.as_ref().unwrap().compatibility_epoch, "own-policy-test");
    assert_eq!(edited.layers[1].surface().unwrap().try_read_interleaved(before.bounds()).unwrap(), raw);
    assert_eq!(edited.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap().composition.as_ref().unwrap().source_mode, 30);
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), edited);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, edited);
}
#[test]
fn inspect_reports_unresolved_stored_policy_without_guessing() {
    let mut before = fixture();
    let id = before.layers[1].id;
    let policy = before.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().composition.as_mut().unwrap();
    policy.stored_composite_space = 0;
    policy.resolved = None;
    let mut s = session(before.clone());
    let info = s.execute("xcf.composition.inspect", json!({"layerId": id.0.to_string()})).unwrap();
    assert_eq!(info["composition"]["stored_composite_space"], 0);
    assert!(info["composition"]["resolved"].is_null());
    assert_eq!(*s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
}
#[test]
fn unknown_auto_legacy_ids_locks_and_nonfinite_fail_atomically() {
    let baseline = fixture();
    let good = params(&baseline);
    let mut candidates = vec![json!(null), json!({}), json!({"layerId":1}), json!({"layerId":"18446744073709551616"})];
    for (key, value) in [
        ("blendSpace", json!("auto")),
        ("operator", json!("auto")),
        ("sourceMode", json!(1)),
        ("legacy", json!(true)),
        ("unexpected", json!(true)),
        ("layerId", json!("00")),
    ] {
        let mut p = good.clone();
        p[key] = value;
        candidates.push(p);
    }
    for p in candidates {
        let mut s = session(baseline.clone());
        assert!(s.execute("xcf.composition.set", p).is_err());
        assert_eq!(*s.active().unwrap().doc, baseline);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
    for which in 0..2 {
        let mut before = fixture();
        if which == 0 {
            before.layers[1].locks.pixels = true;
        } else {
            before.layers[1].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[f64::NAN, 0.25, 0.5, 0.5]).unwrap();
        }
        let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let p = params(&before);
        let mut s = session(before);
        assert!(s.execute("xcf.composition.set", p).is_err());
        assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), bytes);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[cfg(feature = "corpus")]
fn stock_multiply_commands(direct_import: bool) {
    use std::{fs, path::PathBuf};
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS")).join("composition-command-evidence");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!("xcf_composition_reference_hashes.json")).unwrap();
    let read = |name: &str| {
        let bytes = fs::read(base.join(name)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[name]);
        bytes
    };
    let report: serde_json::Value = serde_json::from_slice(&read("observations.json")).unwrap();
    let mut count = 0;
    for case in report["records"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let reference_name = case["reference_edit"].as_str().unwrap();
        let before = photocraft_io::import(source, &read(source)).unwrap().document;
        let reference_bytes = read(reference_name);
        let reference = direct_import.then(|| photocraft_io::import(reference_name, &reference_bytes).unwrap().document);
        let name = case["reopened"]["layers"][0]["name"].as_str().unwrap();
        let id = before.layers.iter().find(|l| l.name == name).unwrap().id;
        let p = json!({"layerId":id.0.to_string(),"sourceMode":30,"legacy":false,"blendSpace":"profile-rgb","compositeSpace":"profile-rgb","operator":"union"});
        let mut s = session(before.clone());
        s.execute("xcf.composition.set", p).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        for old in &before.layers {
            assert_eq!(
                edited.layer(old.id).unwrap().surface().unwrap().try_read_interleaved(before.bounds()).unwrap(),
                old.surface().unwrap().try_read_interleaved(before.bounds()).unwrap()
            );
        }
        let rendered = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        if let Some(reference) = &reference {
            let direct = photocraft_compose::gimp_render::try_render(reference, reference.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px, direct.px);
        }
        let projection = read(case["reopened"]["projection"]["file"].as_str().unwrap());
        let expected: Vec<f64> = projection.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect();
        let actual: Vec<f64> = rendered.px.iter().flatten().copied().collect();
        assert_eq!(actual.len(), expected.len());
        // Display color is undefined when both straight-alpha projections have exactly zero coverage.
        // Hidden source RGB is independently preserved bit-exact in the raw native-plane assertions above.
        let mut worst = 0.0_f64;
        let mut hidden_rgb_delta = 0.0_f64;
        for (pixel, reference) in rendered.px.iter().zip(expected.as_chunks::<4>().0.iter()) {
            for component in 0..4 {
                let delta = (pixel[component] - reference[component]).abs();
                if component < 3 && pixel[3] == 0.0 && reference[3] == 0.0 {
                    hidden_rgb_delta = hidden_rgb_delta.max(delta);
                } else {
                    worst = worst.max(delta);
                }
            }
        }
        println!("{} hidden-zero-alpha RGB delta {hidden_rgb_delta}", case["id"]);
        assert!(worst <= 2.0 / 255.0, "{} projection delta {worst}", case["id"]);
        println!("{} stock projection delta {worst}", case["id"]);
        for doc in std::iter::once(&edited).chain(reference.as_ref()) {
            let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *doc);
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        read(source);
        count += 1;
    }
    assert_eq!(count, 10);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires unpublished original composition-command-evidence; run explicitly with PHOTOCRAFT_XCF_FULL_CORPUS"]
fn independent_stock_multiply_edits_import_native_render_and_undo() {
    stock_multiply_commands(true);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires unpublished original composition-command-evidence; kernel witness separately from direct reference IO admission"]
fn independent_stock_multiply_kernel_matches_raw_projection_and_native_undo() {
    stock_multiply_commands(false);
}

#[test]
fn explicit_spaces_and_operators_preserve_policy_distinctions() {
    for space in ["linear-rgb", "profile-rgb", "perceptual-rgb"] {
        for operator in ["union", "clip-to-backdrop", "clip-to-layer", "intersection"] {
            let mut before = fixture();
            before.layers[0].surface_mut().unwrap().try_write_pixel_f64(1, 0, &[0.2, 0.3, 0.4, 0.75]).unwrap();
            before.layers[1].surface_mut().unwrap().try_write_pixel_f64(1, 0, &[0.7, 0.6, 0.5, 0.25]).unwrap();
            let mut p = params(&before);
            p["blendSpace"] = json!(space);
            p["compositeSpace"] = json!(space);
            p["operator"] = json!(operator);
            let id = before.layers[1].id;
            let mut s = session(before.clone());
            s.execute("xcf.composition.set", p).unwrap();
            let edited = (*s.active().unwrap().doc).clone();
            let state = edited.xcf_state.as_ref().unwrap();
            let policy = state.items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap().composition.as_ref().unwrap();
            assert_eq!(
                policy.stored_composite_space,
                match space {
                    "linear-rgb" => 1,
                    "profile-rgb" => 2,
                    _ => 4,
                }
            );
            assert_eq!(
                policy.stored_operator,
                match operator {
                    "union" => 1,
                    "clip-to-backdrop" => 2,
                    "clip-to-layer" => 3,
                    _ => 4,
                }
            );
            let info = s.execute("xcf.composition.inspect", json!({"layerId":id.0.to_string()})).unwrap();
            assert!(info["composition"]["resolved"].is_object());
            let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), edited);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, before);
        }
    }
}

#[test]
fn oversized_validation_and_inherited_content_locks_reject_before_history_changes() {
    let mut oversized = fixture();
    oversized.size = Size::new(8192, 8192);
    let p = params(&oversized);
    let mut s = session(oversized.clone());
    assert!(s.execute("xcf.composition.set", p).is_err());
    assert_eq!(*s.active().unwrap().doc, oversized);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
    for inherited in [false, true] {
        let mut before = fixture();
        let p = params(&before);
        let top = before.layers[1].id;
        let owner = if inherited {
            let child = before.layers.pop().unwrap();
            let parent = Layer::group("Own locked parent", vec![child]);
            let id = parent.id;
            before.layers.push(parent);
            let mut state = before.xcf_state.take().unwrap();
            state.reconcile_layers(&before).unwrap();
            before.xcf_state = Some(state);
            id
        } else {
            top
        };
        before.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().locks.content = true;
        let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), before);
        let mut s = session(before.clone());
        assert!(s.execute("xcf.composition.set", p).is_err());
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[test]
#[ignore = "24MP composition command release performance gate; run explicitly in --release"]
fn composition_policy_24mp_release() {
    let mut before = fixture();
    before.size = Size::new(6000, 4000);
    before.depth = SampleType::U8;
    let format = PixelFormat::RGBA8;
    for (index, layer) in before.layers.iter_mut().enumerate() {
        layer.content = photocraft_doc::LayerContent::Raster(photocraft_raster::Surface::with_default(
            format,
            if index == 0 { &[0.25, 0.5, 0.75, 0.5] } else { &[0.75, 0.25, 0.5, 0.5] },
        ));
    }
    let bounds = before.bounds();
    for item in &mut before.xcf_state.as_mut().unwrap().items {
        item.raster_bounds = Some(bounds);
    }
    let p = params(&before);
    let mut s = session(before.clone());
    let started = std::time::Instant::now();
    s.execute("xcf.composition.set", p).unwrap();
    println!("24MP U8 two-layer Multiply policy checked and committed {:.3}s", started.elapsed().as_secs_f64());
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(s.active().unwrap().doc.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
    assert_eq!(s.active().unwrap().doc.layers[1].surface().unwrap(), before.layers[1].surface().unwrap());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, before);
}
