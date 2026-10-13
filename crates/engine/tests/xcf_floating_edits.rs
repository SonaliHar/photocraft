//! Persisted attachments anchor raw samples atomically; transient floats stay separate.
use photocraft_doc::import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition};
use photocraft_doc::item_identity::ItemRef;
use photocraft_doc::xcf_editing::FloatingAttachment;
use photocraft_doc::{BlendMode, ColorMode, Document, Layer, LayerContent, PixelFormat, Rect, SampleEncoding, SampleType, Size, XcfState};
use photocraft_engine::Session;
use serde_json::json;

fn original(mode: ColorMode, sample: SampleType) -> Document {
    let format = PixelFormat::new(mode, sample, true);
    let mut doc = Document::new("Own floating anchor", Size::new(4, 3), mode, sample);
    doc.sample_encoding = Some(SampleEncoding::Linear);
    doc.icc_profile = Some(if mode == ColorMode::Grayscale {
        photocraft_cms::builtin::Builtin::SGray.profile().to_bytes()
    } else {
        photocraft_cms::builtin::Builtin::Srgb.profile().to_bytes()
    });
    let mut target = Layer::raster("Own target", format);
    let mut source = Layer::raster("Own floating", format);
    let base = if mode == ColorMode::Grayscale { vec![0.25, 0.5] } else { vec![0.25, 0.5, 0.75, 0.5] };
    let top = if mode == ColorMode::Grayscale { vec![0.75, 0.5] } else { vec![0.75, 0.25, 0.5, 0.5] };
    target.surface_mut().unwrap().try_write_region_f64(Rect::new(0, 0, 4, 3), &base.repeat(12)).unwrap();
    source.surface_mut().unwrap().try_write_region_f64(Rect::new(1, 1, 3, 2), &top.repeat(2)).unwrap();
    let target_id = target.id;
    let floating_id = source.id;
    doc.layers = vec![target, source];
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(if item.item == ItemRef::Layer(floating_id) { Rect::new(1, 1, 3, 2) } else { doc.bounds() });
        item.composition = Some(GimpComposition {
            compatibility_epoch: "own-normal-anchor-test".into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 1,
            stored_composite_space: 1,
            stored_operator: 1,
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
            pass_through: false,
        });
    }
    state.floating = Some(FloatingAttachment { layer: floating_id, target: ItemRef::Layer(target_id) });
    state.selection.selected = vec![ItemRef::Layer(floating_id)];
    state.selection.active = Some(ItemRef::Layer(floating_id));
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    doc
}
fn session(doc: Document) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    s
}
fn native(doc: &Document) {
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *doc);
}

#[test]
fn normal_rgb_gray_anchor_is_native_and_undoable_at_every_supported_depth() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let original = original(mode, depth);
            let target = original.layers[0].id;
            let floating = original.layers[1].id;
            let outside = original.layers[0].surface().unwrap().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
            let mut s = session(original.clone());
            s.execute("xcf.floating.anchor", json!({})).unwrap();
            let edited = (*s.active().unwrap().doc).clone();
            assert!(edited.layer(floating).is_none());
            assert!(edited.xcf_state.as_ref().unwrap().floating.is_none());
            assert_eq!(edited.xcf_state.as_ref().unwrap().selection.active, Some(ItemRef::Layer(target)));
            assert_eq!(s.active().unwrap().active_layer, Some(target));
            assert_eq!(edited.layer(target).unwrap().surface().unwrap().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), outside);
            let values = edited.layer(target).unwrap().surface().unwrap().try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap();
            let alpha = *values.last().unwrap();
            let tolerance = if depth == SampleType::U8 {
                0.01
            } else if depth == SampleType::U16 {
                0.00005
            } else {
                1e-7
            };
            assert!((alpha - 0.75).abs() < tolerance);
            assert!((values[0] - 7.0 / 12.0).abs() < tolerance);
            native(&edited);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, original);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, edited);
        }
    }
}

#[test]
fn bad_params_locks_and_unqualified_attachments_are_atomic() {
    let original = original(ColorMode::Rgb, SampleType::F64);
    for params in [json!({"target":999}), json!([]), json!(true)] {
        let mut s = session(original.clone());
        assert!(s.execute("xcf.floating.anchor", params).is_err());
        assert_eq!(*s.active().unwrap().doc, original);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
    for kind in 0..4 {
        let mut doc = original.clone();
        match kind {
            0 => doc.layers[0].locks.pixels = true,
            1 => doc.layers[0].locks.transparency = true,
            2 => doc.layers[1].blend = BlendMode::Multiply,
            _ => {
                doc.layers[0].content = LayerContent::Group(photocraft_doc::Group { children: vec![], expanded: true, artboard: None });
            }
        }
        let mut s = session(doc.clone());
        assert!(s.execute("xcf.floating.anchor", json!({})).is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[test]
fn opaque_copy_preserves_wide_low_bits_and_transparent_pixels_preserve_target_payloads() {
    let mut doc = original(ColorMode::Rgb, SampleType::F64);
    let target = doc.layers[0].id;
    let src = [-0.0, 0.5 + 2.0_f64.powi(-40), 1.25, 1.0];
    doc.layers[1].surface_mut().unwrap().try_write_pixel_f64(1, 1, &src).unwrap();
    doc.layers[1].surface_mut().unwrap().try_write_pixel_f64(2, 1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    let raw = doc.layers[1].surface().unwrap().try_read_interleaved(Rect::new(1, 1, 2, 2)).unwrap();
    let untouched = doc.layers[0].surface().unwrap().try_read_interleaved(Rect::new(2, 1, 3, 2)).unwrap();
    let mut s = session(doc.clone());
    s.execute("xcf.floating.anchor", json!({})).unwrap();
    let edited = &s.active().unwrap().doc;
    assert_eq!(edited.layer(target).unwrap().surface().unwrap().try_read_interleaved(Rect::new(1, 1, 2, 2)).unwrap(), raw);
    assert_eq!(edited.layer(target).unwrap().surface().unwrap().try_read_interleaved(Rect::new(2, 1, 3, 2)).unwrap(), untouched);
    native(edited);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, doc);
}

#[test]
fn target_extent_clips_anchor_and_inherited_lock_errors_are_atomic() {
    let mut doc = original(ColorMode::Rgb, SampleType::F64);
    let target = doc.layers[0].id;
    doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(target)).unwrap().raster_bounds = Some(Rect::new(0, 0, 2, 3));
    let clipped = doc.layers[0].surface().unwrap().try_read_interleaved(Rect::new(2, 1, 3, 2)).unwrap();
    let mut s = session(doc.clone());
    s.execute("xcf.floating.anchor", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(target).unwrap().surface().unwrap().try_read_interleaved(Rect::new(2, 1, 3, 2)).unwrap(), clipped);
    native(&s.active().unwrap().doc);
    let mut locked = original(ColorMode::Rgb, SampleType::F64);
    let children = std::mem::take(&mut locked.layers);
    let mut group = Layer::new("Own locked parent", LayerContent::Group(photocraft_doc::Group { children, expanded: true, artboard: None }));
    group.locks.pixels = true;
    locked.layers.push(group);
    let mut state = locked.xcf_state.take().unwrap();
    state.reconcile_layers(&locked).unwrap();
    locked.xcf_state = Some(state);
    let mut s = session(locked.clone());
    assert!(s.execute("xcf.floating.anchor", json!({})).is_err());
    assert_eq!(*s.active().unwrap().doc, locked);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
}

#[test]
fn malformed_alpha_nonfinite_and_unqualified_masks_leave_history_unchanged() {
    for kind in 0..4 {
        let mut doc = original(ColorMode::Rgb, SampleType::F64);
        match kind {
            0 => doc.layers[1].surface_mut().unwrap().try_write_pixel_f64(1, 1, &[0.2, 0.3, 0.4, 1.2]).unwrap(),
            1 => doc.layers[1].surface_mut().unwrap().try_write_pixel_f64(1, 1, &[f64::NAN, 0.3, 0.4, 0.5]).unwrap(),
            2 => doc.layers[1].mask = Some(photocraft_doc::LayerMask::reveal_all()),
            _ => doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().resolved = None,
        }
        // NaN payloads compare through exact native bytes, rather than PartialEq float semantics.
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let mut s = session(doc);
        assert!(s.execute("xcf.floating.anchor", json!({})).is_err());
        assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), before);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires unpublished original floating-anchor-evidence corpus; run named suite with --ignored and PHOTOCRAFT_XCF_FULL_CORPUS"]
fn independent_gimp_anchors_match_normal_math_with_recorded_native_rounding() {
    use std::{fs, path::PathBuf};
    let base = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("floating-anchor-evidence");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!("xcf_floating_reference_hashes.json")).unwrap();
    let read = |name: &str| {
        let raw = fs::read(base.join(name)).expect("set PHOTOCRAFT_XCF_FULL_CORPUS to the unpublished original corpus/full");
        assert_eq!(blake3::hash(&raw).to_hex().as_str(), hashes[name]);
        raw
    };
    let manifest: serde_json::Value = serde_json::from_slice(&read("observations.json")).unwrap();
    let mut count = 0;
    for case in manifest["records"].as_array().unwrap() {
        let name = case["id"].as_str().unwrap();
        read(case["input"].as_str().unwrap());
        read(case["reference_edit"].as_str().unwrap());
        let mode = if name.starts_with("gray-") { ColorMode::Grayscale } else { ColorMode::Rgb };
        let sample = if name.contains("-u8_") {
            SampleType::U8
        } else if name.contains("-u16_") {
            SampleType::U16
        } else if name.contains("-u32_") {
            SampleType::U32
        } else if name.contains("-float_") {
            SampleType::F32
        } else {
            SampleType::F64
        };
        let format = PixelFormat::new(mode, sample, true);
        let input = case["input"].as_str().unwrap();
        let doc = photocraft_io::import(input, &read(input)).unwrap().document;
        assert_eq!(doc.mode, mode);
        assert_eq!(doc.depth, sample);
        let attached = doc.xcf_state.as_ref().unwrap().floating.as_ref().unwrap();
        let ItemRef::Layer(target) = attached.target else { panic!("expected qualified raster target") };
        let source_id = attached.layer;
        native(&doc);
        let before = doc.clone();
        let mut s = session(doc);
        s.execute("xcf.floating.anchor", json!({})).unwrap();
        let actual = s.active().unwrap().doc.layer(target).unwrap().surface().unwrap();
        let mut reference = photocraft_raster::Surface::new(format);
        reference.try_write_interleaved(Rect::new(0, 0, 8, 6), &read(case["anchor_reopened"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        let actual = actual.try_read_region_f64(Rect::new(0, 0, 8, 6)).unwrap();
        let expected = reference.try_read_region_f64(Rect::new(0, 0, 8, 6)).unwrap();
        let worst = actual.iter().zip(expected).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        let bound = match sample {
            SampleType::U8 => 1.0 / 255.0,
            SampleType::U16 => 1.0 / 65535.0,
            _ => 1e-6,
        };
        assert!(worst <= bound, "{name}: native numerical delta {worst}");
        assert!(s.active().unwrap().doc.layer(source_id).is_none());
        assert!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().floating.is_none());
        let edited = (*s.active().unwrap().doc).clone();
        native(&edited);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        read(input);
        count += 1;
    }
    assert_eq!(count, 10);
}

#[test]
fn to_layer_and_delete_preserve_or_remove_source_explicitly_with_native_undo() {
    for command in ["xcf.floating.toLayer", "xcf.floating.delete"] {
        let doc = original(ColorMode::Rgb, SampleType::F64);
        let target = doc.layers[0].id;
        let source = doc.layers[1].id;
        let raw = doc.layers[1].surface().unwrap().try_read_interleaved(Rect::new(1, 1, 3, 2)).unwrap();
        let source_item = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(source)).unwrap().clone();
        let target_raw = doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap();
        let mut s = session(doc.clone());
        s.execute(command, json!({})).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert!(edited.xcf_state.as_ref().unwrap().floating.is_none());
        assert_eq!(edited.layer(target).unwrap().surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), target_raw);
        if command.ends_with("toLayer") {
            assert_eq!(edited.layer(source).unwrap().surface().unwrap().try_read_interleaved(Rect::new(1, 1, 3, 2)).unwrap(), raw);
            assert_eq!(edited.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(source)).unwrap(), &source_item);
            assert_eq!(s.active().unwrap().active_layer, Some(source));
        } else {
            assert!(edited.layer(source).is_none());
            assert_eq!(s.active().unwrap().active_layer, Some(target));
        }
        photocraft_compose::gimp_render::PreparedRenderer::new(&edited).unwrap();
        native(&edited);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, doc);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
    }
}

#[test]
fn release_actions_reject_bad_params_and_locked_deletion_atomically() {
    for command in ["xcf.floating.toLayer", "xcf.floating.delete"] {
        for params in [json!(null), json!([]), json!({"unexpected":true})] {
            let before = original(ColorMode::Rgb, SampleType::U8);
            let mut s = session(before.clone());
            assert!(s.execute(command, params).is_err());
            assert_eq!(*s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
        }
    }
    for which in 0..5 {
        let mut before = original(ColorMode::Rgb, SampleType::F64);
        let source = before.layers[1].id;
        match which {
            0 => before.layers[1].locks.pixels = true,
            1 => before.layers[1].locks.position = true,
            2 => before.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(source)).unwrap().locks.content = true,
            3 => before.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(source)).unwrap().locks.position = true,
            _ => before.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(source)).unwrap().locks.visibility = true,
        }
        let mut s = session(before.clone());
        assert!(s.execute("xcf.floating.delete", json!({})).is_err());
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires unpublished original floating-actions-evidence; run explicitly with PHOTOCRAFT_XCF_FULL_CORPUS"]
fn independent_gimp_release_actions_match_exact_native_planes() {
    use std::{fs, path::PathBuf};
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS")).join("floating-actions-evidence");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!("xcf_floating_action_reference_hashes.json")).unwrap();
    let read = |name: &str| {
        let bytes = fs::read(base.join(name)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[name]);
        bytes
    };
    let manifest: serde_json::Value = serde_json::from_slice(&read("observations.json")).unwrap();
    let mut count = 0;
    for record in manifest["records"].as_array().unwrap() {
        let input = record["source"].as_str().unwrap();
        let before = photocraft_io::import(input, &read(input)).unwrap().document;
        let reference_name = record["reference_edit"].as_str().unwrap();
        let reference = photocraft_io::import(reference_name, &read(reference_name)).unwrap().document;
        native(&reference);
        let reference_render = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
        let mut s = session(before.clone());
        let action = record["action"].as_str().unwrap();
        s.execute(if action == "delete" { "xcf.floating.delete" } else { "xcf.floating.toLayer" }, json!({})).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert!(edited.xcf_state.as_ref().unwrap().floating.is_none());
        let edited_render = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(edited_render.px, reference_render.px, "{} {} direct reference display", record["id"], record["action"]);
        let expected = record["reopened"]["layers"].as_array().unwrap();
        assert_eq!(edited.layers.len(), expected.len());
        for layer in expected {
            let name = layer["name"].as_str().unwrap();
            let actual = edited.layers.iter().find(|l| l.name == name).unwrap();
            let imported_reference = reference.layers.iter().find(|l| l.name == name).unwrap();
            let x = layer["offset"][0].as_i64().unwrap() as i32;
            let y = layer["offset"][1].as_i64().unwrap() as i32;
            let w = layer["size"][0].as_i64().unwrap() as i32;
            let h = layer["size"][1].as_i64().unwrap() as i32;
            assert_eq!(actual.surface().unwrap().try_read_interleaved(Rect::new(x, y, x + w, y + h)).unwrap(), read(layer["native"]["file"].as_str().unwrap()));
            assert_eq!(
                imported_reference.surface().unwrap().try_read_interleaved(Rect::new(x, y, x + w, y + h)).unwrap(),
                read(layer["native"]["file"].as_str().unwrap())
            );
        }
        photocraft_compose::gimp_render::PreparedRenderer::new(&edited).unwrap();
        native(&edited);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        read(input);
        count += 1;
    }
    assert_eq!(count, 20);
}

#[test]
fn release_actions_reconcile_named_members_and_private_provenance_without_clearing_selection() {
    use photocraft_doc::{
        item_identity::ItemSetId,
        xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
        xcf_provenance::OpaqueRecord,
    };
    for command in ["xcf.floating.toLayer", "xcf.floating.delete"] {
        let mut before = original(ColorMode::Grayscale, SampleType::U16);
        let target = before.layers[0].id;
        let source = before.layers[1].id;
        let bytes = vec![1, 2, 3, 4];
        let state = before.xcf_state.as_mut().unwrap();
        state.item_sets.push(ItemSet {
            id: ItemSetId(700),
            name: "Own named set".into(),
            family: ItemFamily::Layer,
            rule: ItemSetRule::Named,
            members: vec![ItemRef::Layer(source), ItemRef::Layer(target)],
        });
        state.opaque.push(OpaqueRecord {
            owner: Some(ItemRef::Layer(source)),
            namespace: "own-test".into(),
            identity: "inert-source".into(),
            source_version: 22,
            order: 0,
            flags: 0,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes: bytes.into(),
            rendering_required: false,
        });
        let mut mask = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
        mask.try_write_pixel_f64(0, 0, &[0.5]).unwrap();
        before.selection = Some(mask);
        let mut s = session(before.clone());
        s.execute(command, json!({})).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert_eq!(edited.selection, before.selection);
        let state = edited.xcf_state.as_ref().unwrap();
        if command.ends_with("delete") {
            assert_eq!(state.item_sets[0].members, vec![ItemRef::Layer(target)]);
            assert!(state.opaque.is_empty());
        } else {
            assert_eq!(state.item_sets, before.xcf_state.as_ref().unwrap().item_sets);
            assert_eq!(state.opaque, before.xcf_state.as_ref().unwrap().opaque);
        }
        native(&edited);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, before);
    }
}
