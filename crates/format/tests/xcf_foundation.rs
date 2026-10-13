//! Optional imported foundations must never be lost or treated as runtime authority.
mod common;
use photocraft_doc::*;
use photocraft_doc::{item_identity::*, xcf_filters::*, xcf_provenance::*, xcf_resources::*, xcf_state::*};
use photocraft_format::*;
fn document() -> Document {
    let mut d = Document::with_background("own", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let id = d.layers[0].id;
    let mut s = XcfState::default();
    s.items.push(ItemState {
        item: ItemRef::Layer(id),
        name: "own".into(),
        custom_name: true,
        visible: true,
        locks: ItemLocks { visibility: true, ..Default::default() },
        parent: None,
        source_tattoo: Some(53),
        source_flags: 7,
        raster_bounds: None,
        mask: None,
        composition: None,
    });
    s.provenance = ProjectProvenance {
        format: "xcf".into(),
        format_version: 26,
        container: "plain".into(),
        source_label: "Own.xcf".into(),
        original_locator: Some("inert/Own.xcf".into()),
        ..Default::default()
    };
    s.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own image".into(),
        collection: None,
        internal: false,
        locator: Some("inert/source.png".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    s.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: 9,
        nodes: vec![FilterNode {
            id: FilterId(2),
            operation: OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: "own observed".into() },
            arguments: vec![FilterArgument { name: "exact".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0x7ff8_0000_0000_0042)) }],
            enabled: false,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    let raw = vec![1, 2, 3];
    s.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Layer(id)),
        namespace: "own".into(),
        identity: "note".into(),
        source_version: 26,
        order: 3,
        flags: 9,
        blake3: blake3::hash(&raw).to_hex().to_string(),
        bytes: raw.into(),
        rendering_required: false,
    });
    d.xcf_state = Some(s);
    d.set_physical_resolution(PhysicalResolution {
        x_dpi: 137.5,
        y_dpi: 92.25,
        unit: PhysicalUnit::Custom(CustomUnit {
            name: "Own unit".into(),
            factor: 2.5399999618530273,
            digits: 3,
            symbol: "ou".into(),
            abbreviation: "own".into(),
        }),
    })
    .unwrap();
    d
}
#[test]
fn exact_axes_units_typed_bits_and_inert_state_roundtrip() {
    let d = document();
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    let back = load_from_bytes(&bytes).unwrap();
    assert_eq!(back, d);
    let r = zip::ZipReader::new(&bytes).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&r.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    assert_eq!(v["format_version"], 2);
    assert_eq!(v["minimum_reader_version"], 2);
    assert!(v["document"]["xcf_state"].as_str().is_some());
    assert!(r.entries.iter().any(|e| e.name.starts_with("blobs/")));
}
#[test]
fn native_remapping_covers_all_layer_relationships() {
    let d = document();
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert_ne!(back.layers[0].id, d.layers[0].id);
    let s = back.xcf_state.as_ref().unwrap();
    assert_eq!(s.items[0].item, ItemRef::Layer(back.layers[0].id));
    assert_eq!(s.filter_stacks[0].owner, ItemRef::Layer(back.layers[0].id));
    assert_eq!(s.opaque[0].owner, Some(ItemRef::Layer(back.layers[0].id)));
    back.validate_xcf_state().unwrap();
}
#[test]
fn malformed_relationships_refuse_save() {
    let mut d = document();
    d.xcf_state.as_mut().unwrap().items[0].item = ItemRef::Layer(LayerId(u64::MAX));
    assert!(save_to_bytes(&d, &SaveOptions::default()).is_err());
}
fn bundle() -> (Document, std::path::PathBuf) {
    let d = document();
    let path = common::temp_dir("xcf-foundation");
    PcraftWriter::new().save_dir(&d, &path, &SaveOptions::default()).unwrap();
    (d, path)
}
#[test]
fn legacy_scalar_dpi_migrates_to_equal_axes() {
    let (_, path) = bundle();
    let p = path.join("manifest.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["format_version"] = 1.into();
    v.as_object_mut().unwrap().remove("minimum_reader_version");
    let doc = v["document"].as_object_mut().unwrap();
    doc.remove("physical_resolution");
    doc.remove("xcf_state");
    let legacy = 137.123f32;
    doc.insert("resolution_dpi".into(), serde_json::json!(legacy));
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    let loaded = load_path(&path).unwrap();
    assert_eq!(loaded.effective_resolution(), PhysicalResolution::square(f64::from(legacy)));
    assert!(loaded.xcf_state.is_none());
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn future_minimum_reader_refuses_without_silent_fallback() {
    let (_, path) = bundle();
    let p = path.join("manifest.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["minimum_reader_version"] = 99.into();
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(matches!(load_path(&path), Err(FormatError::TooNew { found: 99, .. })));
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn missing_minimum_reader_cannot_hide_imported_state() {
    let (_, path) = bundle();
    let p = path.join("manifest.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("minimum_reader_version");
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(load_path(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn legacy_and_ordinary_defaults_do_not_gain_optional_state() {
    let d = Document::new("ordinary", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    assert_eq!(load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap(), d);
}
#[test]
fn wide_binary_defaults_and_tiles_are_exact() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut d = Document::new("wide", Size::new(2, 1), ColorMode::Rgb, sample);
        let mut layer = Layer::raster("own", d.pixel_format());
        let s = layer.surface_mut().unwrap();
        let bytes = if sample == SampleType::U32 {
            [u32::MAX, 1, 0, 0x80000001].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
        } else {
            [0x7ff8_0000_0000_0042u64, (-0.0f64).to_bits(), f64::MAX.to_bits(), 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()
        };
        *s = Surface::try_with_default_bytes(s.format(), &bytes).unwrap();
        s.try_write_interleaved(Rect::new(1, 0, 2, 1), &bytes).unwrap();
        d.layers.push(layer);
        let back = load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap();
        assert_eq!(back, d);
    }
}

#[test]
fn editable_indexed_state_roundtrip_and_remap_preserves_duplicates() {
    use std::sync::Arc;
    let mut d = document();
    let owner = d.layers[0].id;
    let raster = EditableIndexedRaster::try_new(Rect::new(-1, 2, 1, 3), Arc::new(vec![0, 1]), None, vec![[20, 40, 60], [20, 40, 60], [90, 80, 70]]).unwrap();
    let original = raster.clone();
    let state = d.xcf_state.as_mut().unwrap();
    state.editable_indexed.push(EditableIndexedLayer { owner, raster });
    state.editable_indexed[0].raster.try_set_pixel(0, 2, 2, 128).unwrap();
    assert_eq!(original.indices().as_slice(), &[0, 1]);
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    let back = load_from_bytes(&bytes).unwrap();
    assert!(back == d);
    let remapped = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let indexed = &remapped.xcf_state.as_ref().unwrap().editable_indexed[0];
    assert_eq!(indexed.owner, remapped.layers[0].id);
    assert_ne!(indexed.owner, owner);
    assert_eq!(indexed.raster.indices().as_slice(), &[0, 2]);
    assert_eq!(indexed.raster.alpha().unwrap().as_slice(), &[255, 128]);
    assert_eq!(indexed.raster.palette()[0], indexed.raster.palette()[1]);
    assert_eq!(indexed.raster.expanded_pixel(0, 2).unwrap(), [90, 80, 70, 128]);
}

#[test]
fn reconciled_clone_and_deletion_preserve_native_state() {
    use std::collections::BTreeMap;
    let mut d = document();
    let source = d.layers[0].id;
    let copy = d.layers[0].duplicate();
    let target = copy.id;
    d.layers.push(copy);
    let mut state = d.xcf_state.take().unwrap();
    state.clone_layer_metadata(source, target, &d, &BTreeMap::new(), &BTreeMap::from([(FilterId(2), FilterId(3))])).unwrap();
    d.layers.remove(0);
    d.layers[0].name = "surviving clone".into();
    state.reconcile_layers(&d).unwrap();
    d.xcf_state = Some(state);
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    let back = load_from_bytes(&bytes).unwrap();
    assert!(back == d);
    let state = back.xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_stacks.len(), 1);
    assert_eq!(state.filter_stacks[0].owner, ItemRef::Layer(target));
    assert_eq!(state.filter_stacks[0].nodes[0].id, FilterId(3));
    assert_eq!(state.items[0].name, "surviving clone");
    assert_eq!(state.provenance.source_label, "Own.xcf");
}

#[test]
fn recursively_cloned_and_translated_indexed_state_survives_native() {
    use std::{collections::BTreeMap, sync::Arc};
    let mut d = document();
    let child = d.layers[0].id;
    d.xcf_state.as_mut().unwrap().editable_indexed.push(EditableIndexedLayer {
        owner: child,
        raster: EditableIndexedRaster::try_new(Rect::new(0, 0, 2, 1), Arc::new(vec![0, 1]), None, vec![[1, 2, 3], [1, 2, 3]]).unwrap(),
    });
    let group = Layer::group("own group", vec![d.layers.remove(0)]);
    let source = group.id;
    d.layers.push(group);
    let mut state = d.xcf_state.take().unwrap();
    state.reconcile_layers(&d).unwrap();
    let copy = d.layers[0].duplicate();
    let target = copy.id;
    let target_child = match &copy.content {
        LayerContent::Group(g) => g.children[0].id,
        _ => unreachable!(),
    };
    d.layers.push(copy);
    state
        .clone_layers_metadata(&BTreeMap::from([(source, target), (child, target_child)]), &d, &BTreeMap::new(), &BTreeMap::from([(FilterId(2), FilterId(3))]))
        .unwrap();
    let copied = state.editable_indexed.iter_mut().find(|i| i.owner == target_child).unwrap();
    copied.raster.translate_bounds(-3, 7).unwrap();
    d.xcf_state = Some(state);
    let back = load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap();
    assert!(back == d);
    let state = back.xcf_state.as_ref().unwrap();
    assert_eq!(state.editable_indexed.iter().find(|i| i.owner == target_child).unwrap().raster.bounds(), Rect::new(-3, 7, -1, 8));
    assert_eq!(state.editable_indexed.iter().find(|i| i.owner == child).unwrap().raster.bounds(), Rect::new(0, 0, 2, 1));
}

#[test]
fn arbitrary_physical_axis_and_custom_factor_bits_roundtrip() {
    let mut d = document();
    for i in 0u64..32 {
        let bits = 0x4050000000000000 | (i.wrapping_mul(0x9e3779b97f4a7c15) & 0x000fffffffffffff);
        let x = f64::from_bits(bits);
        let r = PhysicalResolution {
            x_dpi: x,
            y_dpi: f64::from_bits(bits + 1),
            unit: PhysicalUnit::Custom(CustomUnit {
                name: "own".into(),
                factor: f64::from_bits(bits + 2),
                digits: 3,
                symbol: "o".into(),
                abbreviation: "own".into(),
            }),
        };
        d.set_physical_resolution(r.clone()).unwrap();
        let back = load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap();
        assert_eq!(back.effective_resolution(), r, "DPI/unit bits must not drift at case {i}");
    }
}

#[test]
fn exact_outer_filter_clipping_and_absence_roundtrip() {
    for clipping in [None, Some(0), Some(1), Some(u32::MAX)] {
        let mut d = document();
        d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_clipping = clipping;
        d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = clipping;
        let back = load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap();
        assert_eq!(back.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].source_region, clipping);
        assert_eq!(back.xcf_state.unwrap().filter_stacks[0].nodes[0].source_clipping, clipping);
    }
    let mut value = serde_json::to_value(&document().xcf_state.unwrap().filter_stacks[0].nodes[0]).unwrap();
    value.as_object_mut().unwrap().remove("source_clipping");
    value.as_object_mut().unwrap().remove("source_region");
    let old: FilterNode = serde_json::from_value(value).unwrap();
    assert_eq!(old.source_clipping, None);
    assert_eq!(old.source_region, None);
}

#[test]
fn channel_ids_native_fresh_remap_and_dependent_state_are_exact() {
    use photocraft_doc::xcf_editing::{FloatingAttachment, ItemFamily, ItemSet, ItemSetRule};
    let mut d = document();
    let channel = AlphaChannel::new("Own channel", Surface::new(PixelFormat::GRAY8));
    let old = channel.id;
    d.channels.push(channel);
    let layer = d.layers[0].id;
    let mut state = d.xcf_state.take().unwrap();
    state.reconcile_layers(&d).unwrap();
    state.selection.active = Some(ItemRef::Channel(old));
    state.selection.selected = vec![ItemRef::Channel(old)];
    state.filter_stacks[0].nodes[0].auxiliary.push(("own".into(), ItemRef::Channel(old)));
    state.filter_stacks[0].nodes[0].source_display_name = Some("Own display Ω".into());
    state.filter_stacks[0].nodes[0].source_icon = Some("own-icon".into());
    state.floating = Some(FloatingAttachment { layer, target: ItemRef::Channel(old) });
    state.item_sets.push(ItemSet {
        id: ItemSetId(1),
        name: "Own set".into(),
        family: ItemFamily::Channel,
        rule: ItemSetRule::Named,
        members: vec![ItemRef::Channel(old)],
    });
    state.opaque[0].owner = Some(ItemRef::Channel(old));
    d.xcf_state = Some(state);
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), d);
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let fresh = back.channels[0].id;
    assert_ne!(fresh, old);
    let s = back.xcf_state.as_ref().unwrap();
    assert_eq!(s.selection.active, Some(ItemRef::Channel(fresh)));
    assert_eq!(s.selection.selected, vec![ItemRef::Channel(fresh)]);
    assert_eq!(s.filter_stacks[0].nodes[0].auxiliary[0].1, ItemRef::Channel(fresh));
    assert_eq!(s.item_sets[0].members, vec![ItemRef::Channel(fresh)]);
    assert_eq!(s.opaque[0].owner, Some(ItemRef::Channel(fresh)));
    assert_eq!(s.floating.as_ref().unwrap().target, ItemRef::Channel(fresh));
    back.validate_xcf_state().unwrap();
}
#[test]
fn native_channel_manifest_migrates_missing_id_and_rejects_zero_or_duplicate() {
    let mut d = document();
    d.channels.push(AlphaChannel::new("Own alpha", Surface::new(PixelFormat::GRAY8)));
    let path = common::temp_dir("channel-id-migration");
    PcraftWriter::new().save_dir(&d, &path, &SaveOptions::default()).unwrap();
    let target = path.join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    manifest["document"]["channels"][0].as_object_mut().unwrap().remove("id");
    std::fs::write(&target, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let migrated = load_path(&path).unwrap();
    assert_ne!(migrated.channels[0].id.0, 0);
    assert_eq!(migrated.channels[0].surface, d.channels[0].surface);
    manifest["document"]["channels"][0]["id"] = serde_json::json!(0);
    std::fs::write(&target, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(load_path(&path).is_err());
    manifest["document"]["channels"][0]["id"] = serde_json::json!(d.channels[0].id);
    let duplicate = manifest["document"]["channels"][0].clone();
    manifest["document"]["channels"].as_array_mut().unwrap().push(duplicate);
    std::fs::write(&target, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(load_path(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn selection_identity_survives_empty_pixels_native_remapping_and_rejects_collision() {
    let mut d = document();
    let id = ChannelId::fresh();
    d.selection_channel_id = Some(id);
    let mut state = d.xcf_state.take().unwrap();
    state.reconcile_layers(&d).unwrap();
    state.selection.active = Some(ItemRef::Channel(id));
    state.selection.selected = vec![ItemRef::Channel(id)];
    state.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().raster_bounds = Some(Rect::new(0, 0, 2, 2));
    d.xcf_state = Some(state);
    assert!(d.selection.is_none());
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), d);
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let fresh = back.selection_channel_id.unwrap();
    assert_ne!(fresh, id);
    assert!(back.channels.is_empty() && back.selection.is_none());
    assert_eq!(back.xcf_state.as_ref().unwrap().selection.active, Some(ItemRef::Channel(fresh)));
    assert_eq!(
        back.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Channel(fresh)).unwrap().raster_bounds,
        Some(Rect::new(0, 0, 2, 2))
    );
    let mut collision = AlphaChannel::new("Own", Surface::new(PixelFormat::GRAY8));
    collision.id = id;
    d.channels.push(collision);
    assert!(save_to_bytes(&d, &SaveOptions::default()).is_err());
}

fn document_with_filter_mask() -> Document {
    let mut d = document();
    let owner = ItemRef::Layer(d.layers[0].id);
    let bounds = Rect::new(-2, 3, 0, 5);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut surface = photocraft_raster::Surface::try_with_default_bytes(format, &(-0.0_f64).to_ne_bytes()).unwrap();
    surface.try_write_pixel_f64(-2, 3, &[0.5 + 2.0_f64.powi(-40)]).unwrap();
    let state = d.xcf_state.as_mut().unwrap();
    let mut metadata = state.items[0].clone();
    metadata.item = ItemRef::Mask(MaskId(7));
    metadata.name = "Own filter selection Ω".into();
    metadata.source_tattoo = Some(74);
    metadata.raster_bounds = Some(bounds);
    metadata.mask =
        Some(MaskState { owner, bounds, offset: [-2, 3], outside_f64_bits: (-0.0_f64).to_bits(), enabled: true, shown: true, editing: false, linked: true });
    state.items.push(metadata);
    state.filter_stacks[0].nodes[0].mask = Some(ItemRef::Mask(MaskId(7)));
    state.filter_masks.push(FilterMask::try_new(MaskId(7), FilterId(2), surface, bounds).unwrap());
    d
}

#[test]
fn filter_mask_binary_native_bits_flags_and_fresh_owner_remap_are_exact() {
    let d = document_with_filter_mask();
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), d);
    let reader = zip::ZipReader::new(&bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&reader.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    assert_eq!(manifest["minimum_reader_version"], 2);
    assert_eq!(manifest["document"]["xcf_filter_masks"].as_array().unwrap().len(), 1);
    let state_path = manifest["document"]["xcf_state"].as_str().unwrap();
    let compressed = reader.read_by_name(&format!("blobs/{state_path}.zst"), 1 << 20).unwrap();
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(compressed.as_slice()).unwrap();
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut raw).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert!(json.get("filter_masks").is_none());
    let fresh = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let state = fresh.xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_masks[0], d.xcf_state.as_ref().unwrap().filter_masks[0]);
    assert_eq!(state.items.iter().find(|i| i.item == ItemRef::Mask(MaskId(7))).unwrap().mask.as_ref().unwrap().owner, ItemRef::Layer(fresh.layers[0].id));
    fresh.validate_xcf_state().unwrap();
}

#[test]
fn filter_mask_clone_and_deleted_owner_preserve_or_prune_exact_relationships() {
    use std::collections::BTreeMap;
    let mut d = document_with_filter_mask();
    let source = d.layers[0].id;
    let mut copy = d.layers[0].clone();
    copy.id = LayerId::fresh();
    let target = copy.id;
    d.layers.push(copy);
    let mut state = d.xcf_state.take().unwrap();
    state
        .clone_layers_metadata(
            &BTreeMap::from([(source, target)]),
            &d,
            &BTreeMap::from([(MaskId(7), MaskId(8))]),
            &BTreeMap::from([(FilterId(2), FilterId(3))]),
        )
        .unwrap();
    let cloned = state.filter_masks.iter().find(|m| m.id == MaskId(8)).unwrap();
    assert_eq!(cloned.owner, FilterId(3));
    assert_eq!(cloned.surface, state.filter_masks[0].surface);
    assert_eq!(state.items.iter().find(|i| i.item == ItemRef::Mask(MaskId(8))).unwrap().mask.as_ref().unwrap().owner, ItemRef::Layer(target));
    d.xcf_state = Some(state);
    assert_eq!(load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap(), d);
    d.layers.retain(|layer| layer.id != source);
    let mut state = d.xcf_state.take().unwrap();
    state.reconcile_layers(&d).unwrap();
    assert_eq!(state.filter_masks.len(), 1);
    assert_eq!(state.filter_masks[0].id, MaskId(8));
    assert!(!state.items.iter().any(|i| i.item == ItemRef::Mask(MaskId(7))));
    d.xcf_state = Some(state);
    d.validate_xcf_state().unwrap();
}

#[test]
fn invalid_filter_mask_owner_metadata_or_payload_refuses_native_save() {
    for mutation in 0..4 {
        let mut d = document_with_filter_mask();
        let state = d.xcf_state.as_mut().unwrap();
        match mutation {
            0 => state.filter_masks[0].owner = FilterId(99),
            1 => state.items[1].mask.as_mut().unwrap().bounds.x0 -= 1,
            2 => state.filter_masks.push(state.filter_masks[0].clone()),
            _ => {
                state.filter_masks[0].surface =
                    photocraft_raster::Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &f64::NAN.to_ne_bytes())
                        .unwrap()
            }
        }
        assert!(save_to_bytes(&d, &SaveOptions::default()).is_err());
    }
}

#[test]
fn saved_path_identity_native_remaps_selection_sets_and_editable_vector_dependencies() {
    use photocraft_doc::xcf_items::{EditableSource, SourceItem};
    let mut d = document();
    d.paths.push(NamedPath::new("Own saved Ω", Path::default()));
    let old = d.paths[0].id;
    let mut state = d.xcf_state.take().unwrap();
    state.reconcile_layers(&d).unwrap();
    state.selection.selected_paths = vec![ItemRef::Path(old)];
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(d.layers[0].id),
        source: EditableSource::Vector { path: old, source_modified: true, configuration: vec![] },
        transform: None,
        cache: None,
    });
    d.xcf_state = Some(state);
    let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), d);
    let fresh = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert_ne!(fresh.paths[0].id, old);
    let state = fresh.xcf_state.as_ref().unwrap();
    assert_eq!(state.selection.selected_paths, vec![ItemRef::Path(fresh.paths[0].id)]);
    assert!(matches!(state.source_items[0].source, EditableSource::Vector {path,..} if path == fresh.paths[0].id));
    let before = state.clone();
    let mut invalid = state.clone();
    assert!(invalid.remap_paths(|_| None).is_err());
    assert_eq!(invalid, before);
    let mut deleted = d.clone();
    deleted.paths.clear();
    let mut state = deleted.xcf_state.take().unwrap();
    let before = state.clone();
    assert!(state.reconcile_layers(&deleted).is_err());
    assert_eq!(state, before);
}

#[test]
fn saved_path_native_id_migration_and_duplicate_identity_are_checked() {
    let mut d = document();
    d.paths.push(NamedPath::new("Own", Path::default()));
    let duplicate = d.paths[0].duplicate();
    assert_ne!(duplicate.id, d.paths[0].id);
    assert_eq!(duplicate.path, d.paths[0].path);
    let path = common::temp_dir("path-id-migration");
    PcraftWriter::new().save_dir(&d, &path, &SaveOptions::default()).unwrap();
    let target = path.join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    manifest["document"]["paths"][0].as_object_mut().unwrap().remove("id");
    std::fs::write(&target, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_ne!(load_path(&path).unwrap().paths[0].id.0, 0);
    manifest["document"]["paths"][0]["id"] = serde_json::json!(0);
    std::fs::write(&target, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(load_path(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn explicit_saved_path_metadata_clone_keeps_provenance_without_copying_selection() {
    let mut d = document();
    d.paths.push(NamedPath::new("Own", Path::default()));
    let source = d.paths[0].id;
    let mut state = d.xcf_state.take().unwrap();
    state.reconcile_layers(&d).unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Path(source)).unwrap().source_tattoo = Some(513);
    state.selection.selected_paths = vec![ItemRef::Path(source)];
    let target = d.paths[0].duplicate();
    let id = target.id;
    d.paths.push(target);
    state.clone_path_metadata(source, id, &d).unwrap();
    assert_eq!(state.items.iter().find(|item| item.item == ItemRef::Path(id)).unwrap().source_tattoo, Some(513));
    assert_eq!(state.selection.selected_paths, vec![ItemRef::Path(source)]);
    let before = state.clone();
    assert!(state.clone_path_metadata(source, id, &d).is_err());
    assert_eq!(state, before);
    d.xcf_state = Some(state);
    assert_eq!(load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap(), d);
}
