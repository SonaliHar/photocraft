#[path = "common/xcf.rs"]
mod common;

fn effect(enabled: bool, fields: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut record = Vec::new();
    for text in ["Own invert", "own-icon", "gegl:invert-gamma", "0:0"] {
        common::u32be(&mut record, text.len() as u32 + 1);
        record.extend(text.as_bytes());
        record.push(0);
    }
    for (tag, bytes) in [
        (8, u32::from(enabled).to_be_bytes().to_vec()),
        (7, 62_i32.to_be_bytes().to_vec()),
        (33, 1.0_f32.to_be_bytes().to_vec()),
        (35, 1306165841_i32.to_be_bytes().to_vec()),
        (36, 1306165841_i32.to_be_bytes().to_vec()),
        (37, 1306165841_i32.to_be_bytes().to_vec()),
        (46, 0_u32.to_be_bytes().to_vec()),
        (44, 0_u32.to_be_bytes().to_vec()),
    ] {
        common::prop(&mut record, tag, &bytes);
    }
    for (tag, bytes) in fields {
        common::prop(&mut record, *tag, bytes);
    }
    common::prop(&mut record, 0, &[]);
    common::pointer(&mut record, 0, 22);
    record
}

// Pure synthetic coverage of the measured singleton Replace structure. A
// saved effect has its own same-depth, canvas-sized, empty Gray mask; omitting
// it does not represent the source contract qualified by the original corpus.
fn qualified_image(enabled: bool, fields: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let options = common::FixtureOptions {
        layer_properties: vec![
            (7, 28_i32.to_be_bytes().to_vec()),
            (35, (-1_i32).to_be_bytes().to_vec()),
            (36, (-1_i32).to_be_bytes().to_vec()),
            (37, 0_i32.to_be_bytes().to_vec()),
        ],
        effect_records: vec![effect(enabled, fields)],
        ..Default::default()
    };
    let mut bytes = common::image_custom(22, 150, 0, 1, 1, &[20, 70, 120, 255], &options);
    let mask_pointer = bytes.len() - 8;
    let mask = bytes.len();
    common::patch(&mut bytes, mask_pointer, mask, 22);
    common::u32be(&mut bytes, 1);
    common::u32be(&mut bytes, 1);
    common::u32be(&mut bytes, 5);
    bytes.extend(b"Mask\0");
    common::prop(&mut bytes, 8, &1_u32.to_be_bytes());
    common::prop(&mut bytes, 0, &[]);
    let hierarchy_pointer = bytes.len();
    common::pointer(&mut bytes, 0, 22);
    let hierarchy = bytes.len();
    common::patch(&mut bytes, hierarchy_pointer, hierarchy, 22);
    for value in [1, 1, 1] {
        common::u32be(&mut bytes, value);
    }
    let level_pointer = bytes.len();
    common::pointer(&mut bytes, 0, 22);
    common::pointer(&mut bytes, 0, 22);
    let level = bytes.len();
    common::patch(&mut bytes, level_pointer, level, 22);
    common::u32be(&mut bytes, 1);
    common::u32be(&mut bytes, 1);
    let tile_pointer = bytes.len();
    common::pointer(&mut bytes, 0, 22);
    common::pointer(&mut bytes, 0, 22);
    let tile = bytes.len();
    common::patch(&mut bytes, tile_pointer, tile, 22);
    bytes.push(0);
    bytes
}

#[test]
fn import_point_stack_executes_flat_export_and_preserves_native_state() {
    for enabled in [false, true] {
        let imported = photocraft_io::import("invert.xcf", &qualified_image(enabled, &[])).unwrap();
        let doc = &imported.document;
        let before = doc.clone();
        let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(node.enabled, enabled);
        assert_eq!(node.source_display_name.as_deref(), Some("Own invert"));
        assert_eq!(node.source_icon.as_deref(), Some("own-icon"));
        assert_eq!(node.source_clipping, Some(0));
        let flat = photocraft_io::document_to_image(doc, &mut Vec::new()).unwrap();
        let expected = if enabled { [235, 185, 135] } else { [20, 70, 120] };
        assert_eq!(&flat.data()[..3], &expected);
        let native = photocraft_io::export(doc, "invert.pcraft", &Default::default()).unwrap();
        assert!(native.warnings.is_empty());
        let reopened = photocraft_format::load_from_bytes(&native.bytes).unwrap();
        assert_eq!(reopened.xcf_state, doc.xcf_state);
        assert_eq!(photocraft_io::document_to_image(&reopened, &mut Vec::new()).unwrap().data(), flat.data());
        if enabled {
            assert!(photocraft_io::export(doc, "invert.psd", &Default::default()).is_err());
            assert!(photocraft_io::export(doc, "invert.tiff", &photocraft_io::ExportOptions { tiff_layers: true, ..Default::default() }).is_err());
        }
        assert_eq!(doc, &before);
    }
}

#[test]
fn unqualified_envelopes_are_rejected_even_when_disabled() {
    for enabled in [false, true] {
        // The original incomplete fixture remains a negative control.
        let incomplete = common::FixtureOptions { effect_records: vec![effect(enabled, &[])], ..Default::default() };
        assert!(photocraft_io::import("incomplete.xcf", &common::image_custom(22, 150, 0, 1, 1, &[20, 70, 120, 255], &incomplete)).is_err());
        for fields in [vec![(46, 1_u32.to_be_bytes().to_vec())], vec![(9001, vec![1])]] {
            assert!(photocraft_io::import("unknown.xcf", &qualified_image(enabled, &fields)).is_err());
        }
    }
}
