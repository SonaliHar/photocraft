//! Checked current stock-filter edits never replace immutable source evidence.
use photocraft_doc::{ColorMode, Document, Layer, SampleEncoding, SampleType, Size, XcfState};
use photocraft_doc::{
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{FilterId, ItemRef},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
};
use photocraft_engine::Session;
use serde_json::json;

fn fixture() -> Document {
    let mut doc = Document::new("Own filter edit", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
    doc.sample_encoding = Some(SampleEncoding::Linear);
    doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    let mut layer = Layer::raster("Own HDR raster", doc.pixel_format());
    layer.surface_mut().unwrap().try_write_region_f64(doc.bounds(), &[0.25, 0.5, 0.75, 1.0, 2.0, -0.125, 0.25, 0.5]).unwrap();
    let owner = layer.id;
    doc.layers.push(layer);
    let mut state = XcfState::default();
    state.provenance.format_version = 22;
    state.reconcile_layers(&doc).unwrap();
    let item = state.items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap();
    item.raster_bounds = Some(doc.bounds());
    item.composition = Some(GimpComposition {
        compatibility_epoch: "own-current-filter-policy".into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 1,
        stored_composite_space: 1,
        stored_operator: 1,
        pass_through: false,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
    });
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(owner),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(999_999),
            operation: OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: "xcf-22-stock-point-replace".into() },
            source_display_name: Some("Own exposure source".into()),
            source_icon: Some("own-icon".into()),
            arguments: vec![
                FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.0f32.to_bits())) },
                FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.0f32.to_bits())) },
            ],
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: "xcf-22-stock-point-replace".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -261651887,
                stored_composite_space: -261651887,
                stored_operator: -261651887,
                pass_through: false,
                resolved: None,
            }),
        }],
    });
    doc.xcf_state = Some(state);
    doc
}
fn session(doc: Document) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    s
}
fn patch() -> serde_json::Value {
    json!({"filterId":"999999","arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.5f32.to_bits()}}}]})
}
#[test]
fn stack_enable_move_remove_preserve_source_and_history() {
    let mut before = fixture();
    let state = before.xcf_state.as_mut().unwrap();
    let mut invert = state.filter_stacks[0].nodes[0].clone();
    invert.id = FilterId(999_998);
    invert.operation.name = "gegl:invert-linear".into();
    invert.operation.stored_version = "0:0".into();
    invert.arguments.clear();
    state.filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits()));
    state.filter_stacks[0].nodes.push(invert);
    let mut s = session(before.clone());
    let noop = s.execute("xcf.filter.move", json!({"filterId":"999998","index":1})).unwrap();
    assert_eq!(noop["updated"], false);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
    s.execute("xcf.filter.enable", json!({"filterId":"999998","enabled":false})).unwrap();
    let hidden = (*s.active().unwrap().doc).clone();
    assert_eq!(hidden.layers, before.layers);
    s.execute("xcf.filter.move", json!({"filterId":"999998","index":0})).unwrap();
    let reordered = (*s.active().unwrap().doc).clone();
    assert_eq!(reordered.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id, FilterId(999_998));
    s.execute("xcf.filter.remove", json!({"filterId":"999998"})).unwrap();
    let removed = (*s.active().unwrap().doc).clone();
    assert_eq!(removed.layers, before.layers);
    assert_eq!(removed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 1);
    assert_eq!(removed.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
    let native = photocraft_format::save_to_bytes(&removed, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), removed);
    assert_eq!(s.active().unwrap().history.past_len(), 3);
    for expected in [&reordered, &hidden, &before] {
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(&*s.active().unwrap().doc, expected);
    }
    for _ in 0..3 {
        s.execute("edit.redo", json!({})).unwrap();
    }
    assert_eq!(*s.active().unwrap().doc, removed);
}
#[test]
fn stack_actions_and_opacity_reject_hostile_inputs_without_history() {
    let before = fixture();
    for (cmd, params) in [
        ("xcf.filter.enable", json!({"filterId":"999999","enabled":"false"})),
        ("xcf.filter.enable", json!({"filterId":"999999","enabled":false,"extra":1})),
        ("xcf.filter.move", json!({"filterId":"999999","index":1})),
        ("xcf.filter.move", json!({"filterId":"999999","index":-1})),
        ("xcf.filter.move", json!({"filterId":"999999","index":18446744073709551615_u64})),
        ("xcf.filter.remove", json!({"filterId":"999999","index":0})),
        ("xcf.filter.remove", json!({"filterId":"999998"})),
        ("xcf.filter.set", json!({"filterId":"999999","opacityF64Bits":0.5})),
        ("xcf.filter.set", json!({"filterId":"999999","opacityF64Bits":"0x7ff8000000000000"})),
        ("xcf.filter.set", json!({"filterId":"999999","opacityF64Bits":"0x3fe0000000000000"})),
    ] {
        let mut s = session(before.clone());
        assert!(s.execute(cmd, params).is_err());
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
    let mut locked = before;
    locked.layers[0].locks.pixels = true;
    for cmd in ["xcf.filter.remove", "xcf.filter.move"] {
        let mut s = session(locked.clone());
        let params = if cmd.ends_with("move") { json!({"filterId":"999999","index":0}) } else { json!({"filterId":"999999"}) };
        assert!(s.execute(cmd, params).is_err());
        assert_eq!(*s.active().unwrap().doc, locked);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}
#[test]
fn removed_mask_source_properties_are_archived_and_undo_restores_ownership() {
    use photocraft_doc::{item_identity::MaskId, xcf_filter_masks::FilterMask, xcf_provenance::OpaqueRecord, xcf_state::MaskState};
    let mut before = fixture();
    let bounds = before.bounds();
    let owner = ItemRef::Layer(before.layers[0].id);
    let mask_id = MaskId(123456);
    let state = before.xcf_state.as_mut().unwrap();
    let mut metadata = state.items[0].clone();
    metadata.item = ItemRef::Mask(mask_id);
    metadata.name = "Own selection region".into();
    metadata.composition = None;
    metadata.mask =
        Some(MaskState { owner, bounds, offset: [0, 0], outside_f64_bits: 0.0f64.to_bits(), enabled: true, shown: false, editing: false, linked: false });
    state.items.push(metadata);
    state
        .filter_masks
        .push(FilterMask::try_new(mask_id, FilterId(999999), photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8), bounds).unwrap());
    state.filter_stacks[0].nodes[0].mask = Some(ItemRef::Mask(mask_id));
    let payload = vec![0, 0, 0, 1];
    state.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Mask(mask_id)),
        namespace: "gimp/xcf/source-property".into(),
        identity: "3@5:4".into(),
        source_version: 22,
        order: 1,
        flags: 0,
        blake3: blake3::hash(&payload).to_hex().to_string(),
        bytes: payload.into(),
        rendering_required: false,
    });
    state.selection.active = Some(ItemRef::Mask(mask_id));
    state.selection.mask_target = Some(ItemRef::Mask(mask_id));
    state.selection.selected.push(ItemRef::Mask(mask_id));
    before.validate_xcf_state().unwrap();
    let mut s = session(before.clone());
    s.execute("xcf.filter.remove", json!({"filterId":"999999"})).unwrap();
    let edited = (*s.active().unwrap().doc).clone();
    let state = edited.xcf_state.as_ref().unwrap();
    assert!(state.filter_masks.is_empty());
    assert!(state.filter_stacks[0].nodes.is_empty());
    assert_eq!(state.selection.active, Some(owner));
    assert_eq!(state.selection.mask_target, None);
    let mut expected = before.xcf_state.as_ref().unwrap().opaque[0].clone();
    expected.owner = None;
    expected.identity = "deleted-filter-mask/123456/3@5:4".into();
    assert_eq!(state.opaque[0], expected);
    let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, edited);
    let mut collision = before.clone();
    collision.xcf_state.as_mut().unwrap().opaque.push(expected);
    let mut s = session(collision.clone());
    assert!(s.execute("xcf.filter.remove", json!({"filterId":"999999"})).is_err());
    assert_eq!(*s.active().unwrap().doc, collision);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished GIMP filter stack edit references; run explicitly with corpus"]
fn independent_stack_commands_match_gimp_references_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |name: &str| {
        let path = format!("filter-stack-edits-evidence/{name}");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[&path].as_str().unwrap());
        bytes
    };
    let original = read("stack.source.xcf");
    let before = photocraft_io::import("own-stack.xcf", &original).unwrap().document;
    let node = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
    for action in ["disable", "remove", "reverse", "opacity-half"] {
        let reference = photocraft_io::import("own-reference.xcf", &read(&format!("stack.{action}.xcf"))).unwrap().document;
        let mut s = session(before.clone());
        let (cmd, params) = match action {
            "disable" => ("xcf.filter.enable", json!({"filterId":node.0.to_string(),"enabled":false})),
            "remove" => ("xcf.filter.remove", json!({"filterId":node.0.to_string()})),
            "reverse" => ("xcf.filter.move", json!({"filterId":node.0.to_string(),"index":1})),
            _ => ("xcf.filter.set", json!({"filterId":node.0.to_string(),"opacityF64Bits":"0x3fe0000000000000"})),
        };
        s.execute(cmd, params).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert_eq!(edited.layers, before.layers);
        assert_eq!(edited.source_color_context, before.source_color_context);
        let old_state = before.xcf_state.as_ref().unwrap();
        let removed_masks: std::collections::BTreeSet<_> =
            old_state.filter_masks.iter().filter(|mask| action == "remove" && mask.owner == node).map(|mask| ItemRef::Mask(mask.id)).collect();
        let mut expected_opaque = old_state.opaque.clone();
        for record in &mut expected_opaque {
            if let Some(ItemRef::Mask(mask)) = record.owner
                && removed_masks.contains(&ItemRef::Mask(mask))
            {
                record.owner = None;
                record.identity = format!("deleted-filter-mask/{}/{original}", mask.0, original = record.identity);
            }
        }
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, expected_opaque);
        assert_eq!(edited.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
        let current = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        let expected = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(current.px, expected.px);
        let state: serde_json::Value = serde_json::from_slice(&read(&format!("stack.{action}.fresh.state.json"))).unwrap();
        let bytes = read(state["projection"]["file"].as_str().unwrap());
        let mut worst = 0.0f64;
        for (actual, raw) in current.px.iter().zip(bytes.as_chunks::<32>().0.iter()) {
            for (actual, raw) in actual.iter().zip(raw.as_chunks::<8>().0.iter()) {
                worst = worst.max((actual.clamp(0.0, 1.0) - f64::from_le_bytes(*raw)).abs());
            }
        }
        println!("{action}: command stack {} /255", worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
        let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        assert_eq!(std::fs::read(root.join("filter-stack-edits-evidence/stack.source.xcf")).unwrap(), original);
    }
}
#[test]
fn point_parameter_edit_preserves_raw_hdr_native_and_undo() {
    let before = fixture();
    let mut s = session(before.clone());
    let info = s.execute("xcf.filter.inspect", json!({"filterId":"999999"})).unwrap();
    assert_eq!(info["filterId"], "999999");
    assert_eq!(s.active().unwrap().history.past_len(), 0);
    let noop = s.execute("xcf.filter.set", json!({"filterId":"999999","enabled":true})).unwrap();
    assert_eq!(noop["updated"], false);
    assert_eq!(*s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
    s.execute("xcf.filter.set", patch()).unwrap();
    let edited = (*s.active().unwrap().doc).clone();
    assert_eq!(edited.layers, before.layers);
    assert_eq!(edited.source_color_context, before.source_color_context);
    let stack = &edited.xcf_state.as_ref().unwrap().filter_stacks[0];
    assert_eq!(stack.revision, 1);
    assert_eq!(stack.nodes[0].source_display_name.as_deref(), Some("Own exposure source"));
    let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
    let rendered = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
    assert!(rendered.px[1][0] > 2.0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, edited);
}
#[test]
fn activation_and_unknown_typed_arguments_are_atomic() {
    let before = fixture();
    for p in [
        json!(null),
        json!({}),
        json!({"filterId":999999}),
        json!({"filterId":"0999999","enabled":false}),
        json!({"filterId":"999999","enabled":"false"}),
        json!({"filterId":"999999","arguments":[{"name":"unknown","value":{"Bool":true}}]}),
        json!({"filterId":"999999","arguments":[{"name":"exposure","value":{"Number":{"Float64Bits":0}}}]}),
        json!({"filterId":"999999","arguments":[{"name":"exposure","value":{"Number":{"Signed":{"width":32,"value":1}}}}]}),
        json!({"filterId":"999999","arguments":[{"name":"exposure","value":{"Number":{"Signed":{"width":64,"value":1}}}}]}),
        json!({"filterId":"999999","arguments":[{"name":"exposure","value":{"Number":{"Signed":{"width":32,"value":2147483648_i64}}}}]}),
        json!({"filterId":"999999","arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":f32::INFINITY.to_bits()}}}]}),
        json!({"filterId":"999999","arguments":[{"name":"black-level","value":{"Number":{"Float32Bits":0.5f32.to_bits()}}}]}),
    ] {
        let mut s = session(before.clone());
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
    let mut s = session(before.clone());
    s.execute("xcf.filter.set", json!({"filterId":"999999","enabled":false})).unwrap();
    assert!(!s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, before);
    let mut locked = before.clone();
    locked.layers[0].locks.pixels = true;
    let mut s = session(locked.clone());
    assert!(s.execute("xcf.filter.set", patch()).is_err());
    assert_eq!(*s.active().unwrap().doc, locked);
}

#[test]
fn inherited_content_and_activation_visibility_locks_are_respected() {
    for inherited in [false, true] {
        let mut before = fixture();
        let child = before.layers[0].id;
        let owner = if inherited {
            let child = before.layers.pop().unwrap();
            let group = Layer::group("Own locked parent", vec![child]);
            let id = group.id;
            before.layers.push(group);
            let mut state = before.xcf_state.take().unwrap();
            state.reconcile_layers(&before).unwrap();
            before.xcf_state = Some(state);
            id
        } else {
            child
        };
        let item = before.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap();
        if inherited {
            item.locks.content = true;
        } else {
            item.locks.visibility = true;
        }
        let mut s = session(before.clone());
        assert!(s.execute("xcf.filter.set", json!({"filterId":"999999","enabled":false})).is_err());
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[test]
fn cancellable_validation_is_immutable_before_and_during_work() {
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut before = fixture();
    before.size = Size::new(256, 256);
    let bounds = before.bounds();
    before.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(bounds);
    let copy = before.clone();
    assert!(photocraft_engine::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&before, &Interrupt::cancel_only(&|| true)).is_err());
    let checks = AtomicUsize::new(0);
    let cancelled = || checks.fetch_add(1, Ordering::Relaxed) >= 128;
    assert!(photocraft_engine::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&before, &Interrupt::cancel_only(&cancelled)).is_err());
    assert!(checks.load(Ordering::Relaxed) >= 128);
    assert_eq!(before, copy);
}

#[test]
fn oversized_filter_staging_rejects_before_history_or_raster_mutation() {
    let mut before = fixture();
    before.size = Size::new(6001, 6001);
    let mut s = session(before.clone());
    assert!(s.execute("xcf.filter.set", patch()).is_err());
    assert_eq!(*s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires reviewed unpublished original scalar filter source-edit corpus; run explicitly with PHOTOCRAFT_XCF_FULL_CORPUS"]
fn independent_scalar_filter_parameter_edits_match_stock_references() {
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS"));
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let mut count = 0;
    for name in [
        "exposure",
        "brightness-contrast",
        "invert-linear",
        "invert-gamma",
        "levels",
        "gaussian-blur",
        "box-blur",
        "median-blur",
        "motion-blur-linear",
        "pixelize",
        "posterize",
        "posterize-activation",
        "value-invert",
        "value-invert-activation",
        "mono-mixer-activation",
    ] {
        let base = match name {
            "gaussian-blur" => "filter-owner-layer".to_owned(),
            "posterize" => "operations/gimp-posterize/operation-gimp-posterize".to_owned(),
            "posterize-activation" => "operations/gimp-posterize/activation-gimp-posterize".to_owned(),
            "value-invert-activation" => "operations/gegl-value-invert/activation-gegl-value-invert".to_owned(),
            "mono-mixer-activation" => "operations/gegl-mono-mixer/activation-gegl-mono-mixer".to_owned(),
            _ => format!("operations/gegl-{name}/operation-gegl-{name}"),
        };
        let read = |path: &str| {
            let bytes = fs::read(root.join(path)).unwrap();
            assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
            bytes
        };
        let source_bytes = read(&format!("{base}.xcf"));
        let reference_bytes = read(&format!("{base}.edit.xcf"));
        let before = photocraft_io::import("source.xcf", &source_bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &reference_bytes).unwrap().document;
        let old_state = before.xcf_state.as_ref().unwrap();
        let old_node = &old_state.filter_stacks[0].nodes[0];
        let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let mut p = json!({"filterId":old_node.id.0.to_string(),"enabled":target.enabled});
        let changed: Vec<_> = target.arguments.iter().filter(|argument| !old_node.arguments.contains(argument)).collect();
        if !changed.is_empty() {
            p["arguments"] = serde_json::to_value(&changed).unwrap();
        }
        let mut s = session(before.clone());
        let info = s.execute("xcf.filter.inspect", json!({"filterId":old_node.id.0.to_string()})).unwrap();
        assert_eq!(info["clipping"], 0);
        let noop = s.execute("xcf.filter.set", json!({"filterId":old_node.id.0.to_string(),"enabled":old_node.enabled})).unwrap();
        assert_eq!(noop["updated"], false);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        if matches!(name, "box-blur" | "median-blur") {
            assert!(
                s.execute(
                    "xcf.filter.set",
                    json!({"filterId":old_node.id.0.to_string(),"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":32,"value":6}}}}]})
                )
                .is_err()
            );
            assert_eq!(*s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
        }
        let unsupported = match name {
            "motion-blur-linear" => Some(json!({"name":"length","value":{"Number":{"Float32Bits":11.0f32.to_bits()}}})),
            "pixelize" => Some(json!({"name":"size-x","value":{"Number":{"Signed":{"width":32,"value":18}}}})),
            _ => None,
        };
        if let Some(argument) = unsupported {
            assert!(s.execute("xcf.filter.set", json!({"filterId":old_node.id.0.to_string(),"arguments":[argument]})).is_err());
            assert_eq!(*s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
        }
        s.execute("xcf.filter.set", p).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        let state = edited.xcf_state.as_ref().unwrap();
        assert_eq!(state.opaque, old_state.opaque);
        assert_eq!(state.provenance, old_state.provenance);
        assert_eq!(state.filter_masks, old_state.filter_masks);
        assert_eq!(edited.layers, before.layers);
        assert_eq!(state.filter_stacks[0].nodes[0].arguments, target.arguments);
        for original in old_node.arguments.iter().filter(|argument| matches!(argument.value, FilterValue::Color { .. })) {
            assert!(state.filter_stacks[0].nodes[0].arguments.contains(original), "immutable source Color preserved");
        }
        assert_eq!(state.filter_stacks[0].nodes[0].enabled, target.enabled);
        let render = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        let direct = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(render.px, direct.px, "{name} canonical current parameters vs independent edit");
        let expected = photocraft_codecs::decode(&read(&format!("{base}.edit.ref.png"))).unwrap();
        let (reference_x, reference_y) = match name {
            "box-blur" => (5, 5),
            "motion-blur-linear" => (6, 0),
            _ => (0, 0),
        };
        let mut worst = 0.0_f64;
        for (pixel, index) in render.px.iter().zip(0u32..) {
            for (channel, value) in pixel.iter().enumerate() {
                worst = worst.max(
                    (value.clamp(0.0, 1.0)
                        - f64::from(expected.get(index % edited.size.width + reference_x, index / edited.size.width + reference_y, channel)))
                    .abs(),
                );
            }
        }
        assert!(worst <= 2.0 / 255.0, "{name} independent GIMP display delta {worst}");
        println!("{name}: real parameter/activation edit, native/undo, independent display {:.6}/255", worst * 255.0);
        let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), edited);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        read(&format!("{base}.xcf"));
        count += 1;
    }
    assert_eq!(count, 15);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independently authored GIMP false-luminosity mixer edits"]
fn independent_false_luminosity_mixer_weight_edit_native_and_undo() {
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let data = fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&data).to_hex().as_str(), hashes[path].as_str().unwrap());
        data
    };
    let source = photocraft_io::import("source.xcf", &read("mono-mixer-edits-evidence/mono.source.xcf")).unwrap().document;
    let reference = photocraft_io::import("edit.xcf", &read("mono-mixer-edits-evidence/mono.weights.xcf")).unwrap().document;
    let original = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let changed: Vec<_> = target.arguments.iter().filter(|argument| !original.arguments.contains(argument)).collect();
    assert_eq!(changed.len(), 3);
    let mut session = session(source.clone());
    let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let bad = json!({"filterId":original.id.0.to_string(),"arguments":[{"name":"preserve-luminosity","value":{"Bool":true}}]});
    assert!(session.execute("xcf.filter.set", bad).is_err());
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), before);
    session.execute("xcf.filter.set", json!({"filterId":original.id.0.to_string(),"arguments":changed})).unwrap();
    let edited = (*session.active().unwrap().doc).clone();
    assert_eq!(edited.layers, source.layers);
    assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, source.xcf_state.as_ref().unwrap().opaque);
    assert_eq!(edited.xcf_state.as_ref().unwrap().provenance, source.xcf_state.as_ref().unwrap().provenance);
    assert_eq!(edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, target.arguments);
    let render = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
    let direct = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(render.px, direct.px);
    let expected = read("mono-mixer-edits-evidence/mono.weights.fresh.projection.bin");
    assert_eq!(expected.len(), render.px.len() * 32);
    let mut worst = 0.0_f64;
    for (actual, expected) in render.px.iter().zip(expected.as_chunks::<32>().0.iter()) {
        for (actual, expected) in actual.iter().zip(expected.as_chunks::<8>().0.iter()) {
            worst = worst.max((actual - f64::from_le_bytes(*expected)).abs());
        }
    }
    assert!(worst <= 2.0 / 255.0);
    println!("MonoMixer real weight edit native/undo: {} /255", worst * 255.0);
    assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap(), edited);
    assert_eq!(session.active().unwrap().history.past_len(), 1);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, source);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, edited);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires reviewed unpublished finite RGB/Gray Median source-edit corpus"]
fn finite_median_radius_and_precision_edits_match_independent_native() {
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS"));
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let mut count = 0;
    for folder in ["median-raster-color-evidence", "median-gray-color-evidence"] {
        let packet: serde_json::Value = serde_json::from_slice(&read(&format!("{folder}/observations.json"))).unwrap();
        let records = packet["records"].as_array().unwrap();
        for source in records.iter().filter(|record| record["phase"] == "source") {
            let reference_record = records.iter().find(|record| record["case"] == source["case"] && record["phase"] == "edit").unwrap();
            let source_path = format!("{folder}/{}", source["source"]["file"].as_str().unwrap());
            let source_bytes = read(&source_path);
            let before = photocraft_io::import("source.xcf", &source_bytes).unwrap().document;
            let reference =
                photocraft_io::import("reference.xcf", &read(&format!("{folder}/{}", reference_record["source"]["file"].as_str().unwrap()))).unwrap().document;
            let state = before.xcf_state.as_ref().unwrap();
            let node = &state.filter_stacks[0].nodes[0];
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut s = session(before.clone());
            s.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":target.arguments})).unwrap();
            let edited = (*s.active().unwrap().doc).clone();
            assert_eq!(edited.layers, before.layers);
            assert_eq!(edited.source_color_context, before.source_color_context);
            let current = edited.xcf_state.as_ref().unwrap();
            assert_eq!(current.opaque, state.opaque);
            assert_eq!(current.provenance, state.provenance);
            assert_eq!(current.filter_masks, state.filter_masks);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, target.arguments);
            let actual = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
            let direct = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.px, direct.px);
            let descriptor = reference_record["merged_layer"]["reads"]
                .as_array()
                .unwrap()
                .iter()
                .find(|read| {
                    read["region"] == "buffer-extent"
                        && if folder == "median-gray-color-evidence" {
                            read["requested_format"] == "R'G'B'A double"
                        } else {
                            read["requested_format"].is_null()
                        }
                })
                .unwrap();
            let raw = read(&format!("{folder}/{}", descriptor["file"].as_str().unwrap()));
            let expected: Vec<f64> = if folder == "median-gray-color-evidence" || edited.depth == SampleType::F64 {
                raw.as_chunks::<8>().0.iter().map(|bytes| f64::from_le_bytes(*bytes)).collect()
            } else {
                raw.as_chunks::<4>().0.iter().map(|bytes| f64::from(f32::from_le_bytes(*bytes))).collect()
            };
            assert_eq!(expected.len(), actual.px.len() * 4);
            let mut worst = 0.0_f64;
            for (pixel, expected) in actual.px.iter().zip(expected.as_chunks::<4>().0.iter()) {
                for channel in 0..4 {
                    if channel < 3 && pixel[3] == 0.0 && expected[3] == 0.0 {
                        continue;
                    }
                    worst = worst.max((pixel[channel] - expected[channel]).abs());
                }
            }
            assert!(worst <= 2.0 / 255.0, "{}: {}/255", source["case"], worst * 255.0);
            println!("{}: Median radius/precision command native oracle {:.6}/255", source["case"], worst * 255.0);
            let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), edited);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, edited);
            assert_eq!(read(&source_path), source_bytes);
            count += 1;
        }
    }
    assert_eq!(count, 7);
}

#[test]
#[ignore = "manual release-only 24MP current filter staging benchmark"]
fn filter_staging_24mp_release() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let mut before = fixture();
    before.size = Size::new(6000, 4000);
    let bounds = before.bounds();
    before.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(bounds);
    let pixel: Vec<u8> = [0.25_f64, 0.5, 0.75, 1.0].iter().flat_map(|value| value.to_ne_bytes()).collect();
    let format = before.pixel_format();
    *before.layers[0].surface_mut().unwrap() = photocraft_raster::Surface::try_with_default_bytes(format, &pixel).unwrap();
    let mut s = session(before.clone());
    let start = std::time::Instant::now();
    s.execute("xcf.filter.set", patch()).unwrap();
    println!("24MP F64 full-canvas exposure staging: {:.3}s", start.elapsed().as_secs_f64());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, before);
    Ok(())
}

#[test]
fn filter_actions_cancellation_fences_leave_native_and_history_unchanged() {
    use photocraft_engine::xcf_filter_cmds::{enable_with_interrupt, move_with_interrupt, remove_with_interrupt, set_with_interrupt};
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut before = fixture();
    let stack = &mut before.xcf_state.as_mut().unwrap().filter_stacks[0];
    let mut invert = stack.nodes[0].clone();
    invert.id = FilterId(999998);
    invert.operation.name = "gegl:invert-linear".into();
    invert.operation.stored_version = "0:0".into();
    invert.arguments.clear();
    stack.nodes.push(invert);
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    type Action = fn(&mut Session, &serde_json::Value, &Interrupt<'_>) -> photocraft_engine::Result<serde_json::Value>;
    let actions: [(Action, serde_json::Value); 4] = [
        (set_with_interrupt, patch()),
        (enable_with_interrupt, json!({"filterId":"999999","enabled":false})),
        (move_with_interrupt, json!({"filterId":"999999","index":1})),
        (remove_with_interrupt, json!({"filterId":"999999"})),
    ];
    for (run, params) in actions {
        let checks = AtomicUsize::new(0);
        let poll = || {
            checks.fetch_add(1, Ordering::Relaxed);
            false
        };
        let mut successful = session(before.clone());
        run(&mut successful, &params, &Interrupt::cancel_only(&poll)).unwrap();
        assert_eq!(successful.active().unwrap().history.past_len(), 1);
        let total = checks.load(Ordering::Relaxed);
        assert!(total > 8);
        for cancel_at in [0, total / 2, total - 1] {
            let checks = AtomicUsize::new(0);
            let poll = || checks.fetch_add(1, Ordering::Relaxed) >= cancel_at;
            let mut s = session(before.clone());
            assert!(run(&mut s, &params, &Interrupt::cancel_only(&poll)).is_err());
            assert_eq!(*s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), native);
        }
    }
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independently authored finite HDR RGB Clip edits"]
fn independent_hdr_rgb_clip_limit_and_boolean_edits_native_undo_atomic() {
    use photocraft_raster::Interrupt;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let source_bytes = read("rgb-clip-hdr-evidence/clip.source.xcf");
    let source = photocraft_io::import("source.xcf", &source_bytes).unwrap().document;
    let original = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    for stage in ["limits", "low-disabled"] {
        let reference = photocraft_io::import("edit.xcf", &read(&format!("rgb-clip-hdr-evidence/clip.{stage}.xcf"))).unwrap().document;
        let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let changed: Vec<_> = target.arguments.iter().filter(|argument| !original.arguments.contains(argument)).collect();
        assert_eq!(changed.len(), if stage == "limits" { 2 } else { 1 });
        let params = json!({"filterId":original.id.0.to_string(),"arguments":changed});
        let mut s = session(source.clone());
        let native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        for (name, invalid) in [
            ("clip-high", json!({"Bool":false})),
            ("clip-high", json!({"Number":{"Float32Bits":0}})),
            ("low-limit", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("low-limit", json!({"Number":{"Float32Bits":0.9_f32.to_bits()}})),
        ] {
            assert!(s.execute("xcf.filter.set", json!({"filterId":original.id.0.to_string(),"arguments":[{"name":name,"value":invalid}]})).is_err());
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), native);
        }
        assert!(photocraft_engine::xcf_filter_cmds::set_with_interrupt(&mut s, &params, &Interrupt::cancel_only(&|| true)).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        assert_eq!(*s.active().unwrap().doc, source);
        s.execute("xcf.filter.set", params).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert_eq!(edited.layers, source.layers);
        assert_eq!(edited.source_color_context, source.source_color_context);
        let state = edited.xcf_state.as_ref().unwrap();
        let old = source.xcf_state.as_ref().unwrap();
        assert_eq!(state.opaque, old.opaque);
        assert_eq!(state.provenance, old.provenance);
        assert_eq!(state.filter_masks, old.filter_masks);
        assert_eq!(state.filter_stacks[0].nodes[0].arguments, target.arguments);
        let rendered = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        let direct = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(rendered.px, direct.px);
        let expected = read(&format!("rgb-clip-hdr-evidence/clip.{stage}.fresh.projection.bin"));
        assert_eq!(expected.len(), rendered.px.len() * 32);
        // Renderer returns declared stored linear samples; GIMP's explicit
        // R'G'B'A double reference is nonlinear in the same implicit RGB primaries.
        assert_eq!(edited.sample_encoding, Some(photocraft_doc::SampleEncoding::Linear));
        let curve = photocraft_cms::curve::srgb_trc();
        let mut worst = 0.0_f64;
        for (actual, expected) in rendered.px.iter().zip(expected.as_chunks::<32>().0.iter()) {
            for (channel, (actual, expected)) in actual.iter().zip(expected.as_chunks::<8>().0.iter()).enumerate() {
                let actual = if channel < 3 { curve.eval_inverse64(*actual) } else { *actual };
                worst = worst.max((actual - f64::from_le_bytes(*expected)).abs());
            }
        }
        println!("RGB Clip {stage} actual command: {}/255", worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
        assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap(), edited);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, source);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        assert_eq!(read("rgb-clip-hdr-evidence/clip.source.xcf"), source_bytes);
    }
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independently authored Value Invert precision and alpha originals"]
fn independent_value_invert_precision_alpha_enable_native_and_undo() {
    use photocraft_raster::Interrupt;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let raw = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&raw).to_hex().as_str(), hashes[path].as_str().unwrap());
        raw
    };
    for precision in ["u16_non_linear", "float_linear", "double_non_linear"] {
        for (phase, target_phase, enabled) in [("source", "disabled", false), ("disabled", "reenabled", true)] {
            let input_path = format!("value-invert-precision-evidence/{precision}.{phase}.xcf");
            let input = read(&input_path);
            let source = photocraft_io::import("source.xcf", &input).unwrap().document;
            let target =
                photocraft_io::import("reference.xcf", &read(&format!("value-invert-precision-evidence/{precision}.{target_phase}.xcf"))).unwrap().document;
            let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let params = json!({"filterId":node.id.0.to_string(),"enabled":enabled});
            let mut s = session(source.clone());
            assert!(photocraft_engine::xcf_filter_cmds::enable_with_interrupt(&mut s, &params, &Interrupt::cancel_only(&|| true)).is_err());
            assert_eq!(*s.active().unwrap().doc, source);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            s.execute("xcf.filter.enable", params).unwrap();
            let edited = (*s.active().unwrap().doc).clone();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            let state = edited.xcf_state.as_ref().unwrap();
            let old = source.xcf_state.as_ref().unwrap();
            assert_eq!(state.opaque, old.opaque);
            assert_eq!(state.provenance, old.provenance);
            assert_eq!(state.filter_masks, old.filter_masks);
            assert_eq!(state.filter_stacks[0].nodes[0].enabled, enabled);
            let actual = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
            let direct = photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.px, direct.px);
            let expected = read(&format!("value-invert-precision-evidence/{precision}.{target_phase}.fresh.projection.bin"));
            assert_eq!(expected.len(), actual.px.len() * 32);
            let mut worst = 0.0_f64;
            for (actual, expected) in actual.px.iter().zip(expected.as_chunks::<32>().0.iter()) {
                let actual_alpha = actual[3];
                let expected_alpha = f64::from_le_bytes(expected[24..32].try_into().unwrap());
                for (channel, (actual, expected)) in actual.iter().zip(expected.as_chunks::<8>().0.iter()).enumerate() {
                    let actual = if channel < 3 && edited.sample_encoding == Some(photocraft_doc::SampleEncoding::Linear) {
                        photocraft_cms::curve::srgb_trc().eval_inverse64(*actual)
                    } else {
                        *actual
                    };
                    let expected = f64::from_le_bytes(*expected);
                    // Image projection cannot witness hidden RGB under zero alpha.
                    // Exact hidden samples are covered by the independent baked-native
                    // oracle; compare visible colour here in premultiplied form.
                    let delta = if channel < 3 { (actual * actual_alpha - expected * expected_alpha).abs() } else { (actual - expected).abs() };
                    if channel < 3 && expected_alpha > 0.0 {
                        assert!((actual - expected).abs() <= 2.0 / 255.0, "visible colour");
                    }
                    worst = worst.max(delta);
                }
            }
            println!("{precision} {phase}->{target_phase} actual ValueInvert command{}/255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap(), edited);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, source);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, edited);
            assert_eq!(read(&input_path), input);
        }
    }
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished Value Invert precision admission boundaries"]
fn value_invert_precision_unqualified_contexts_reject_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let path = "value-invert-precision-evidence/float_linear.source.xcf";
    let raw = std::fs::read(root.join(path)).unwrap();
    assert_eq!(blake3::hash(&raw).to_hex().as_str(), hashes[path].as_str().unwrap());
    let original = photocraft_io::import("source.xcf", &raw).unwrap().document;
    // Prove the positive baseline before testing individually changed boundaries.
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
                    photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
            }
            "mask-nonzero" | "mask-negative-zero" => {
                let value = if mutation == "mask-nonzero" { 0.5 } else { -0.0 };
                changed.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[value]).unwrap();
            }
            "mask-owner" => changed.xcf_state.as_mut().unwrap().filter_masks[0].owner = FilterId(u64::MAX),
            "mask-absent" => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
            "epoch" => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "unqualified".into(),
            "argument-type" => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0]
                .arguments
                .push(FilterArgument { name: "unexpected".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0.0_f64.to_bits())) }),
            "hdr" | "nonfinite" => {
                let value = if mutation == "hdr" { 2.0 } else { f64::INFINITY };
                changed.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[value, 0.25, 0.5, 1.0]).unwrap();
            }
            "icc-origin" => changed.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
            "geometry" => changed.size = Size::new(9, 6),
            _ => unreachable!(),
        }
        let id = changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let native = photocraft_format::save_to_bytes(&changed, &Default::default()).ok();
        let mut s = session(changed.clone());
        // Disabling must not bypass unsupported current-state admission.
        assert!(s.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{mutation}");
        assert_eq!(*s.active().unwrap().doc, changed, "{mutation}");
        assert_eq!(s.active().unwrap().history.past_len(), 0, "{mutation}");
        if let Some(native) = native {
            assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), native, "{mutation}");
        }
    }
    assert_eq!(std::fs::read(root.join(path)).unwrap(), raw);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Brightness Contrast precision coupled edits"]
fn independent_brightness_contrast_coupled_precision_edits_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let observations: serde_json::Value = serde_json::from_slice(&read("brightness-contrast-precision-evidence/observations.json")).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let source_path = format!("brightness-contrast-precision-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("brightness", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("brightness", json!({"Number":{"Float32Bits":4.0_f32.to_bits()}})),
            ("contrast", json!({"Number":{"Float32Bits":6.0_f32.to_bits()}})),
            ("contrast", json!({"Number":{"Float64Bits":1.0_f64.to_bits().to_string()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"brightness","value":{"Number":{"Float32Bits":0.125_f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in ["coupled", "reduced", "disabled"] {
            let reference = photocraft_io::import("edit.xcf", &read(&format!("brightness-contrast-precision-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            assert_eq!(arguments.len(), if stage == "disabled" { 0 } else { 2 });
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
            let projection = read(&format!("brightness-contrast-precision-evidence/{family}.{stage}.fresh.projection.bin"));
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
            println!("{family} {stage} actual Brightness Contrast command {}/255", worst * 255.0);
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
    assert_eq!(count, 36);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished Brightness Contrast RGB and Gray admission boundaries"]
fn brightness_contrast_precision_changed_contexts_reject_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    for family in ["rgb-float_linear", "gray-double_non_linear"] {
        let path = format!("brightness-contrast-precision-evidence/{family}.source.xcf");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[&path].as_str().unwrap());
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

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent GEGL Levels precision coupled edits"]
fn independent_gegl_levels_coupled_precision_edits_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let observations: serde_json::Value = serde_json::from_slice(&read("gegl-levels-precision-evidence/observations.json")).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let source_path = format!("gegl-levels-precision-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("in-low", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("in-high", json!({"Number":{"Float32Bits":0.125_f32.to_bits()}})),
            ("out-low", json!({"Number":{"Float32Bits":f32::INFINITY.to_bits()}})),
            ("out-high", json!({"Number":{"Float64Bits":1.0_f64.to_bits().to_string()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"in-low","value":{"Number":{"Float32Bits":0.125_f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in ["coupled", "reduced", "disabled"] {
            let reference = photocraft_io::import("edit.xcf", &read(&format!("gegl-levels-precision-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            assert_eq!(arguments.len(), if stage == "disabled" { 0 } else { 4 });
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
            let projection = read(&format!("gegl-levels-precision-evidence/{family}.{stage}.fresh.projection.bin"));
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
            println!("{family} {stage} actual GEGL Levels command {}/255", worst * 255.0);
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
    assert_eq!(count, 36);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished GEGL Levels RGB and Gray admission boundaries"]
fn gegl_levels_precision_changed_contexts_reject_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    for family in ["rgb-float_linear", "gray-double_non_linear"] {
        let path = format!("gegl-levels-precision-evidence/{family}.source.xcf");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[&path].as_str().unwrap());
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

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Exposure black-level precision coupled edits"]
fn independent_exposure_black_coupled_precision_edits_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let observations: serde_json::Value = serde_json::from_slice(&read("exposure-black-level-precision-evidence/observations.json")).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let source_path = format!("exposure-black-level-precision-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("black-level", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("exposure", json!({"Number":{"Float32Bits":4.0_f32.to_bits()}})),
            ("black-level", json!({"Number":{"Float32Bits":0.125_f32.to_bits()}})),
            ("exposure", json!({"Number":{"Float64Bits":1.0_f64.to_bits().to_string()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"black-level","value":{"Number":{"Float32Bits":0.0625_f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in ["coupled", "reduced", "disabled"] {
            let reference =
                photocraft_io::import("edit.xcf", &read(&format!("exposure-black-level-precision-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            assert_eq!(arguments.len(), if stage == "disabled" { 0 } else { 2 });
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
            let projection = read(&format!("exposure-black-level-precision-evidence/{family}.{stage}.fresh.projection.bin"));
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
            println!("{family} {stage} actual Exposure black-level command {}/255", worst * 255.0);
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
    assert_eq!(count, 36);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished Exposure black-level RGB and Gray admission boundaries"]
fn exposure_black_precision_changed_contexts_reject_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    for family in ["rgb-float_linear", "gray-double_non_linear"] {
        let path = format!("exposure-black-level-precision-evidence/{family}.source.xcf");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[&path].as_str().unwrap());
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

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Color Temperature precision coupled edits"]
fn independent_color_temperature_coupled_precision_edits_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let observations: serde_json::Value = serde_json::from_slice(&read("color-temperature-precision-evidence/observations.json")).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let source_path = format!("color-temperature-precision-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("original-temperature", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("intended-temperature", json!({"Number":{"Float32Bits":999.0_f32.to_bits()}})),
            ("original-temperature", json!({"Number":{"Float32Bits":12001.0_f32.to_bits()}})),
            ("intended-temperature", json!({"Number":{"Float64Bits":1.0_f64.to_bits().to_string()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"original-temperature","value":{"Number":{"Float32Bits":6500.0_f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in ["coupled", "reduced", "disabled"] {
            let reference = photocraft_io::import("edit.xcf", &read(&format!("color-temperature-precision-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            assert_eq!(arguments.len(), if stage == "disabled" { 0 } else { 2 });
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
            let projection = read(&format!("color-temperature-precision-evidence/{family}.{stage}.fresh.projection.bin"));
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
            println!("{family} {stage} actual Color Temperature command {}/255", worst * 255.0);
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
    assert_eq!(count, 36);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished Color Temperature RGB and Gray admission boundaries"]
fn color_temperature_precision_changed_contexts_reject_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    for family in ["rgb-float_linear", "gray-double_non_linear"] {
        let path = format!("color-temperature-precision-evidence/{family}.source.xcf");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[&path].as_str().unwrap());
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

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Color Temperature precision coupled edits"]
fn independent_color_temperature_off_grid_edits_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let packet: serde_json::Value = serde_json::from_slice(&read("color-temperature-offgrid-evidence/observations.json")).unwrap();
    let observations = serde_json::json!({"records":packet["records"].as_array().unwrap().iter().map(|row| serde_json::json!({"id":format!("{}.source",row["id"].as_str().unwrap())})).collect::<Vec<_>>()});
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let source_path = format!("color-temperature-offgrid-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("original-temperature", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("intended-temperature", json!({"Number":{"Float32Bits":999.0_f32.to_bits()}})),
            ("original-temperature", json!({"Number":{"Float32Bits":12001.0_f32.to_bits()}})),
            ("intended-temperature", json!({"Number":{"Float64Bits":1.0_f64.to_bits().to_string()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"original-temperature","value":{"Number":{"Float32Bits":6500.0_f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in ["edit"] {
            let reference = photocraft_io::import("edit.xcf", &read(&format!("color-temperature-offgrid-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            assert_eq!(arguments.len(), if stage == "disabled" { 0 } else { 2 });
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
            let projection = read(&format!("color-temperature-offgrid-evidence/{family}.{stage}.fresh.projection.bin"));
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
            println!("{family} {stage} actual Color Temperature command {}/255", worst * 255.0);
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
    assert_eq!(count, 12);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Desaturate precision coupled edits"]
fn independent_desaturate_modes_precision_edits_and_native_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let observations: serde_json::Value = serde_json::from_slice(&read("desaturate-precision-evidence/observations.json")).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".luminance")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".luminance").unwrap();
        let source_path = format!("desaturate-precision-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("mode", json!({"Enum":{"domain":"GimpDesaturateMode","value":-1}})),
            ("mode", json!({"Enum":{"domain":"GimpDesaturateMode","value":5}})),
            ("mode", json!({"Enum":{"domain":"OtherMode","value":3}})),
            ("mode", json!({"Number":{"Float32Bits":3.0_f32.to_bits()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"mode","value":{"Enum":{"domain":"GimpDesaturateMode","value":3}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in ["lightness", "luma", "average", "value", "disabled"] {
            let reference = photocraft_io::import("edit.xcf", &read(&format!("desaturate-precision-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            assert_eq!(arguments.len(), if stage == "disabled" { 0 } else { 1 });
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
            let projection = read(&format!("desaturate-precision-evidence/{family}.{stage}.fresh.projection.bin"));
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
            println!("{family} {stage} actual Desaturate command {}/255", worst * 255.0);
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
    assert_eq!(count, 30);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished Desaturate RGB admission boundaries"]
fn desaturate_precision_changed_contexts_reject_atomically() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    for family in ["rgb-float_linear", "rgb-double_non_linear"] {
        let path = format!("desaturate-precision-evidence/{family}.luminance.xcf");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[&path].as_str().unwrap());
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
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Saturation precision coupled edits"]
fn independent_saturation_modes_precision_edits_and_native_undo() {
    check_saturation_precision_edits(false, 0);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent Gray Saturation coupled edits"]
fn independent_gray_saturation_modes_precision_edits_and_native_undo() {
    check_saturation_precision_edits(true, 0);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent RGB U16/F32 Native to Yuv references"]
fn independent_rgb_float_native_to_yuv_edits_and_native_undo() {
    check_saturation_precision_edits(false, 1);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent RGB U16/F32 Yuv source edit references"]
fn independent_rgb_float_yuv_source_choices_edits_and_native_undo() {
    check_saturation_precision_edits(false, 2);
}
#[cfg(feature = "corpus")]
fn check_saturation_precision_edits(gray_scope: bool, yuv_scope: u8) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    let read = |path: &str| {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[path].as_str().unwrap());
        bytes
    };
    let observations: serde_json::Value = serde_json::from_slice(&read("saturation-precision-evidence/observations.json")).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| {
        let id = row["id"].as_str().unwrap();
        id.starts_with(if gray_scope { "gray-" } else { "rgb-" })
            && id.ends_with(if yuv_scope == 2 { ".cieyuv-half" } else { ".native-half" })
            && (yuv_scope == 0 || !id.contains("double"))
    }) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(if yuv_scope == 2 { ".cieyuv-half" } else { ".native-half" }).unwrap();
        let source_path = format!("saturation-precision-evidence/{id}.xcf");
        let bytes = read(&source_path);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let native_before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        if yuv_scope != 0 {
            let mut locked = source.clone();
            locked.layers[0].locks.pixels = true;
            let owner = photocraft_doc::item_identity::ItemRef::Layer(locked.layers[0].id);
            locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == owner).unwrap().locks.content = true;
            let locked_native = photocraft_format::save_to_bytes(&locked, &Default::default()).unwrap();
            let mut locked_session = session(locked.clone());
            assert!(
                locked_session.execute(
                    "xcf.filter.set",
                    json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"colorspace","value":{"Enum":{"domain":"GeglSaturationType","value":if yuv_scope == 1 {2} else {1}}}}]}),
                ).is_err()
            );
            assert_eq!(*locked_session.active().unwrap().doc, locked);
            assert_eq!(locked_session.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&locked_session.active().unwrap().doc, &Default::default()).unwrap(), locked_native);
        }
        let mut rejected = session(source.clone());
        for (name, value) in [
            ("colorspace", json!({"Enum":{"domain":"GeglSaturationType","value":-1}})),
            ("colorspace", json!({"Enum":{"domain":"GeglSaturationType","value":9}})),
            ("colorspace", json!({"Enum":{"domain":"OtherMode","value":3}})),
            ("colorspace", json!({"Number":{"Float32Bits":3.0_f32.to_bits()}})),
        ] {
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":value}]})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&rejected.active().unwrap().doc, &Default::default()).unwrap(), native_before);
        }
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut rejected,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"colorspace","value":{"Enum":{"domain":"GeglSaturationType","value":1}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            )
            .is_err()
        );
        assert_eq!(*rejected.active().unwrap().doc, source);
        assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        for stage in [
            "native-half",
            "native-threequarter",
            "native-onehalf",
            "cielab-half",
            "cielab-threequarter",
            "cielab-onehalf",
            "cieyuv-half",
            "cieyuv-threequarter",
            "cieyuv-onehalf",
            "disabled",
        ] {
            if yuv_scope == 0 && (stage == "native-half" || stage.starts_with("cieyuv") && !gray_scope)
                || yuv_scope == 1 && !stage.starts_with("cieyuv")
                || yuv_scope == 2 && stage == "cieyuv-half"
            {
                continue;
            }
            let reference = photocraft_io::import("edit.xcf", &read(&format!("saturation-precision-evidence/{family}.{stage}.xcf"))).unwrap().document;
            let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let arguments: Vec<_> = target.arguments.iter().filter(|argument| !node.arguments.contains(argument)).collect();
            if stage == "disabled" {
                // The independently authored disabled projection uses Native.
                // Disabling our Yuv source preserves its own typed arguments;
                // the no-effect projection is the independent pixel oracle.
                if yuv_scope != 2 {
                    assert!(arguments.is_empty());
                }
            } else {
                assert!((1..=2).contains(&arguments.len()));
            }
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
            let expected_arguments = if stage == "disabled" && yuv_scope == 2 { &node.arguments } else { &target.arguments };
            assert_eq!(&state.filter_stacks[0].nodes[0].arguments, expected_arguments);
            assert_eq!(state.filter_stacks[0].nodes[0].enabled, target.enabled);
            let actual = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
            let direct = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.px, direct.px);
            let projection = read(&format!("saturation-precision-evidence/{family}.{stage}.fresh.projection.bin"));
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
            println!("{family} {stage} actual Saturation command {}/255", worst * 255.0);
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
    assert_eq!(
        count,
        if yuv_scope == 1 {
            12
        } else if yuv_scope == 2 {
            36
        } else if gray_scope {
            54
        } else {
            36
        }
    );
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished Saturation RGB admission boundaries"]
fn saturation_precision_changed_contexts_reject_atomically() {
    check_saturation_changed_contexts(false, false);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished Gray Saturation admission boundaries"]
fn gray_saturation_precision_changed_contexts_reject_atomically() {
    check_saturation_changed_contexts(true, false);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "local unpublished independent RGB Yuv U16/F32 admission controls"]
fn rgb_float_yuv_changed_contexts_reject_atomically() {
    check_saturation_changed_contexts(false, true);
}
#[cfg(feature = "corpus")]
fn check_saturation_changed_contexts(gray_scope: bool, yuv_scope: bool) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: serde_json::Value = serde_json::from_str(include_str!("xcf_filter_reference_hashes.json")).unwrap();
    for family in if gray_scope {
        ["gray-float_linear", "gray-double_non_linear"]
    } else if yuv_scope {
        ["rgb-float_linear", "rgb-u16_non_linear"]
    } else {
        ["rgb-float_linear", "rgb-double_non_linear"]
    } {
        let stage = if yuv_scope { "cieyuv-half" } else { "native-half" };
        let path = format!("saturation-precision-evidence/{family}.{stage}.xcf");
        let bytes = std::fs::read(root.join(&path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hashes[&path].as_str().unwrap());
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
            "enum-domain",
            "enum-range",
            "enum-number",
            "yuv-depth",
            "current-f64",
        ] {
            if mutation == "yuv-depth" && (gray_scope || original.depth != SampleType::F64) {
                continue;
            }
            if mutation == "current-f64" && !yuv_scope || original.depth == SampleType::U16 && matches!(mutation, "mask-negative-zero" | "hdr" | "nonfinite") {
                continue;
            }
            let mut changed = original.clone();
            match mutation {
                "mask-depth" => {
                    changed.xcf_state.as_mut().unwrap().filter_masks[0].surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(
                        ColorMode::Grayscale,
                        if original.depth == SampleType::U16 { SampleType::F32 } else { SampleType::U16 },
                        false,
                    ))
                }
                "current-f64" => {
                    let converted = changed.layers[0].surface().unwrap().try_convert(photocraft_color::PixelFormat::RGBA64F).unwrap();
                    *changed.layers[0].surface_mut().unwrap() = converted;
                    let mask = &mut changed.xcf_state.as_mut().unwrap().filter_masks[0];
                    mask.surface = mask.surface.try_convert(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false)).unwrap();
                    changed.depth = SampleType::F64;
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
                "enum-domain" | "enum-range" | "enum-number" | "yuv-depth" => {
                    let value = match mutation {
                        "enum-domain" => FilterValue::Enum { domain: "Unknown".into(), value: 0 },
                        "enum-range" => FilterValue::Enum { domain: "GeglSaturationType".into(), value: 3 },
                        "enum-number" => FilterValue::Number(ExactNumber::Float32Bits(0.0_f32.to_bits())),
                        _ => FilterValue::Enum { domain: "GeglSaturationType".into(), value: 2 },
                    };
                    changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments.iter_mut().find(|a| a.name == "colorspace").unwrap().value = value;
                }
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
