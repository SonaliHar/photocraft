mod common;
use photocraft_xcf::*;

#[test]
fn item_state_is_typed_without_render_admission() {
    let props = vec![
        (2, vec![]),
        (20, 91u32.to_be_bytes().to_vec()),
        (9, 1u32.to_be_bytes().to_vec()),
        (12, 1u32.to_be_bytes().to_vec()),
        (13, 1u32.to_be_bytes().to_vec()),
        (14, 1u32.to_be_bytes().to_vec()),
        (31, 1u32.to_be_bytes().to_vec()),
        (34, 8u32.to_be_bytes().to_vec()),
        (41, 0u32.to_be_bytes().to_vec()),
        (16, vec![12, 34, 56]),
    ];
    let options = common::FixtureOptions { layer_properties: props, ..Default::default() };
    let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    let f = decode(&bytes, &Default::default()).unwrap();
    let d = &f.layers[0];
    assert!(d.active && d.legacy_linked && d.mask_editing && d.mask_shown && d.show_masked);
    assert_eq!(d.tattoo, Some(91));
    assert_eq!(d.group_flags, 1);
    assert_eq!(d.color_tag, Some(8));
    assert_eq!(d.item_sets, [0]);
    assert_eq!(d.channel_color, Some([12.0 / 255.0, 34.0 / 255.0, 56.0 / 255.0]));
    assert!(d.record_offset > 0);
    assert!(d.unsupported.iter().any(|s| s.contains("show-mask")));
}

#[test]
fn invalid_typed_item_fields_reject() {
    for (tag, payload) in [(2, vec![1]), (12, 2u32.to_be_bytes().to_vec()), (38, [f32::NAN.to_be_bytes(); 3].concat())] {
        let options = common::FixtureOptions { layer_properties: vec![(tag, payload)], ..Default::default() };
        let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
        assert!(decode(&bytes, &Default::default()).is_err());
    }
}

#[test]
fn image_item_sets_retain_inert_search_and_tattoo() {
    let mut set = 1u32.to_be_bytes().to_vec();
    set.extend(u32::MAX.to_be_bytes());
    common::u32be(&mut set, 4);
    set.extend(b"Own\0");
    let options = common::FixtureOptions { image_properties: vec![(20, 100u32.to_be_bytes().to_vec()), (40, set)], ..Default::default() };
    let f = decode(&common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).unwrap();
    assert_eq!(f.tattoo, Some(100));
    assert_eq!((f.item_sets[0].item_type, f.item_sets[0].method), (1, u32::MAX));
    assert_eq!(f.item_sets[0].name, "Own");
}

#[test]
fn float_opacity_and_float_overlay_color_override_legacy_values_in_either_order() {
    for reverse in [false, true] {
        let mut properties = vec![
            (6, 128u32.to_be_bytes().to_vec()),
            (33, 0.125f32.to_be_bytes().to_vec()),
            (16, vec![12, 34, 56]),
            (38, [0.25f32.to_be_bytes(), 0.5f32.to_be_bytes(), 0.75f32.to_be_bytes()].concat()),
        ];
        if reverse {
            properties.reverse();
        }
        let options = common::FixtureOptions { layer_properties: properties, ..Default::default() };
        let f = decode(&common::image_custom(12, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).unwrap();
        assert_eq!(f.layers[0].opacity.to_bits(), 0.125f32.to_bits());
        assert_eq!(f.layers[0].channel_color, Some([0.25, 0.5, 0.75]));
        assert_eq!(f.layers[0].raw_properties.len(), 4);
    }
}
fn old_channel(visible: Option<bool>) -> Vec<u8> {
    let mut bytes = b"gimp xcf file\0".to_vec();
    for n in [1, 1, 1] {
        common::u32be(&mut bytes, n);
    }
    common::prop(&mut bytes, 17, &[0]);
    common::prop(&mut bytes, 0, &[]);
    common::u32be(&mut bytes, 0); // no layers
    let channel_pointer = bytes.len();
    common::u32be(&mut bytes, 0);
    common::u32be(&mut bytes, 0);
    let channel = bytes.len();
    for n in [1, 1, 8] {
        common::u32be(&mut bytes, n);
    }
    bytes.extend(b"Channel\0");
    if let Some(visible) = visible {
        common::prop(&mut bytes, 8, &u32::from(visible).to_be_bytes());
    }
    common::prop(&mut bytes, 0, &[]);
    let hierarchy_pointer = bytes.len();
    common::u32be(&mut bytes, 0);
    let hierarchy = bytes.len();
    for n in [1, 1, 1] {
        common::u32be(&mut bytes, n);
    }
    let level_pointer = bytes.len();
    common::u32be(&mut bytes, 0);
    common::u32be(&mut bytes, 0);
    let level = bytes.len();
    for n in [1, 1] {
        common::u32be(&mut bytes, n);
    }
    let tile_pointer = bytes.len();
    common::u32be(&mut bytes, 0);
    common::u32be(&mut bytes, 0);
    let tile = bytes.len();
    bytes.push(77);
    for (at, value) in [(channel_pointer, channel), (hierarchy_pointer, hierarchy), (level_pointer, level), (tile_pointer, tile)] {
        common::patch(&mut bytes, at, value, 0);
    }
    bytes
}
#[test]
fn absent_historical_visibility_defaults_differ_for_layers_and_channels() {
    let layer = decode(&common::image(0, 150, 0, 1, 1, &[1, 2, 3, 255]), &Default::default()).unwrap();
    assert!(layer.layers[0].visible);
    for visible in [None, Some(false), Some(true)] {
        let channel = decode(&old_channel(visible), &Default::default()).unwrap();
        assert_eq!(channel.channels[0].visible, visible.unwrap_or(false));
        assert_eq!(channel.channels[0].plane.as_ref().unwrap().bytes, [77]);
    }
}
