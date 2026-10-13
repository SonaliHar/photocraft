//! Valid channel documents continue exactly through legacy and ZIP64 saves.
use photocraft_doc::{
    AlphaChannel, ColorMode, Document, Layer, LayerContent, LayerId, PixelFormat, Rect, SampleType, Size, XcfState,
    item_identity::{ChannelId, ItemRef, ItemSetId},
    xcf_editing::{EditingSelection, FloatingAttachment, ItemFamily, ItemSet, ItemSetRule},
    xcf_provenance::OpaqueRecord,
    xcf_state::{ItemLocks, ItemState},
};
use photocraft_raster::{Interrupt, Surface};
use std::{collections::BTreeMap, io::Cursor};

fn item(reference: ItemRef, name: &str) -> ItemState {
    ItemState {
        item: reference,
        name: name.into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(17),
        source_flags: 29,
        raster_bounds: Some(Rect::new(0, 0, 2, 1)),
        mask: None,
        composition: None,
    }
}
fn document() -> Document {
    let mut doc = Document::new("Own channels", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    let mut layer = Layer::new("Own floating", LayerContent::Raster(Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true))));
    layer.id = LayerId(10);
    doc.layers.push(layer);
    let mut state = XcfState { items: vec![item(ItemRef::Layer(LayerId(10)), "Own floating")], ..Default::default() };
    for id in [1, 2] {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        let bytes = [0.25f32, -0.0].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
        surface.try_write_interleaved(doc.bounds(), &bytes).unwrap();
        let mut channel = AlphaChannel::new("Own channel", surface);
        channel.id = ChannelId(id);
        state.items.push(item(ItemRef::Channel(channel.id), &channel.name));
        doc.channels.push(channel);
    }
    let one = ItemRef::Channel(ChannelId(1));
    let two = ItemRef::Channel(ChannelId(2));
    state.selection = EditingSelection { selected: vec![one, two], active: Some(two), ..Default::default() };
    state.item_sets.push(ItemSet { id: ItemSetId(1), name: "Own set".into(), family: ItemFamily::Channel, rule: ItemSetRule::Named, members: vec![two, one] });
    state.floating = Some(FloatingAttachment { layer: LayerId(10), target: one });
    let bytes = b"Own inert channel provenance".to_vec();
    state.opaque.push(OpaqueRecord {
        owner: Some(two),
        namespace: "own".into(),
        identity: "own".into(),
        source_version: 22,
        order: 0,
        flags: 3,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    doc
}
fn archives(doc: &Document) -> (Vec<u8>, Vec<u8>) {
    let legacy = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    let (stream, _) = photocraft_format::PcraftWriter::new()
        .save_zip_stream(doc, &Default::default(), Cursor::new(Vec::new()), Default::default(), 64 << 20, &Interrupt::NONE)
        .unwrap();
    (legacy, stream.into_inner())
}
fn roundtrip(doc: &Document) {
    let (legacy, stream) = archives(doc);
    assert_eq!(photocraft_format::load_from_bytes(&legacy).unwrap(), *doc);
    assert_eq!(photocraft_format::load_from_stream(Cursor::new(stream), &Default::default(), Default::default(), &Interrupt::NONE).unwrap(), *doc);
}

#[test]
fn successful_channel_remap_continues_through_both_native_containers() {
    let mut doc = document();
    roundtrip(&doc);
    let original = doc.clone();
    let state = doc.xcf_state.as_mut().unwrap();
    let payload = state.opaque[0].bytes.as_ptr();
    let mut calls = BTreeMap::new();
    state
        .remap_channels(|id| {
            *calls.entry(id).or_insert(0) += 1;
            Some(ChannelId(id.0 + 20))
        })
        .unwrap();
    assert_eq!(calls, BTreeMap::from([(ChannelId(1), 1), (ChannelId(2), 1)]));
    assert_eq!(state.opaque[0].bytes.as_ptr(), payload);
    assert_eq!(state.selection.active, Some(ItemRef::Channel(ChannelId(22))));
    assert_eq!(state.floating.as_ref().unwrap().target, ItemRef::Channel(ChannelId(21)));
    for (channel, before) in doc.channels.iter_mut().zip(&original.channels) {
        channel.id = ChannelId(channel.id.0 + 20);
        assert_eq!(channel.surface, before.surface);
    }
    assert_eq!(doc.layers, original.layers);
    doc.validate_xcf_state().unwrap();
    roundtrip(&doc);
}

#[test]
fn failed_channel_remaps_leave_exact_native_bytes_unchanged() {
    for case in 0..3 {
        let mut doc = document();
        let original = doc.clone();
        let before = archives(&doc);
        let result = doc.xcf_state.as_mut().unwrap().remap_channels(|id| match case {
            0 => Some(ChannelId(0)),
            1 => Some(ChannelId(21)),
            _ => (id != ChannelId(2)).then_some(ChannelId(21)),
        });
        assert!(result.is_err());
        assert_eq!(doc, original);
        assert_eq!(archives(&doc), before);
        roundtrip(&doc);
    }
}
