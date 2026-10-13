//! Synthetic native continuation of atomic XCF layer-identity remapping.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, Layer, LayerContent, LayerId, XcfState,
    item_identity::{FilterId, ItemRef, ItemSetId},
    xcf_editing::{EditingSelection, FloatingAttachment, ItemFamily, ItemSet, ItemSetRule},
    xcf_filters::{FilterNode, FilterStack, OperationIdentity},
    xcf_provenance::OpaqueRecord,
    xcf_state::{ItemLocks, ItemState},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};
use std::{collections::BTreeMap, io::Cursor};

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn document() -> TestResult<Document> {
    let mut doc = Document::new("Own remap continuation", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    let mut state = XcfState::default();
    for id in [1, 2] {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let bytes = [0.25f32, 0.5, -0.0, 1.0, 0.75, 0.0, 0.125, 0.5].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
        surface.try_write_interleaved(Rect::new(0, 0, 2, 1), &bytes)?;
        let mut layer = Layer::new("Own identity", LayerContent::Raster(surface));
        layer.id = LayerId(id);
        doc.layers.push(layer);
        state.items.push(ItemState {
            item: ItemRef::Layer(LayerId(id)),
            name: "Own identity".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(17),
            source_flags: 29,
            raster_bounds: Some(Rect::new(0, 0, 2, 1)),
            mask: None,
            composition: None,
        });
    }
    let one = ItemRef::Layer(LayerId(1));
    let two = ItemRef::Layer(LayerId(2));
    state.selection = EditingSelection { selected: vec![one, two], active: Some(two), selected_paths: vec![], mask_target: None };
    state.item_sets.push(ItemSet { id: ItemSetId(1), name: "Own set".into(), family: ItemFamily::Layer, rule: ItemSetRule::Named, members: vec![two, one] });
    state.filter_stacks.push(FilterStack {
        owner: one,
        revision: 7,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "own:inert".into(), stored_version: "0:0".into(), schema_epoch: "own".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: false,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![("aux".into(), two)],
            composition: None,
        }],
    });
    let bytes = vec![41; 4096];
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
    state.floating = Some(FloatingAttachment { layer: LayerId(2), target: one });
    doc.xcf_state = Some(state);
    doc.validate_xcf_state()?;
    Ok(doc)
}

fn archives(doc: &Document) -> TestResult<(Vec<u8>, Vec<u8>)> {
    let legacy = photocraft_format::save_to_bytes(doc, &Default::default())?;
    let (stream, _) = photocraft_format::PcraftWriter::new().save_zip_stream(
        doc,
        &Default::default(),
        Cursor::new(Vec::new()),
        Default::default(),
        64 << 20,
        &Interrupt::NONE,
    )?;
    Ok((legacy, stream.into_inner()))
}

fn roundtrip(doc: &Document) -> TestResult<()> {
    let (legacy, stream) = archives(doc)?;
    if photocraft_format::load_from_bytes(&legacy)? != *doc
        || photocraft_format::load_from_stream(Cursor::new(stream), &Default::default(), Default::default(), &Interrupt::NONE)? != *doc
    {
        return Err("native remap continuation lost document fields or samples".into());
    }
    Ok(())
}

#[test]
fn typed_layer_references_and_samples_roundtrip_after_remap() {
    let mut doc = document().unwrap();
    roundtrip(&doc).unwrap();
    let before = doc.clone();
    let state = doc.xcf_state.as_mut().unwrap();
    let pointer = state.opaque[0].bytes.as_ptr();
    let mut calls = BTreeMap::new();
    state
        .remap_layers(|id| {
            *calls.entry(id).or_insert(0) += 1;
            Some(LayerId(id.0 + 10))
        })
        .unwrap();
    assert_eq!(calls, BTreeMap::from([(LayerId(1), 1), (LayerId(2), 1)]));
    assert_eq!(state.opaque[0].bytes.as_ptr(), pointer);
    assert_eq!(state.filter_stacks[0].nodes[0].id, FilterId(1));
    assert_eq!(state.item_sets[0].id, ItemSetId(1));
    for layer in &mut doc.layers {
        layer.id = LayerId(layer.id.0 + 10);
    }
    doc.validate_xcf_state().unwrap();
    assert_eq!(doc.xcf_state.as_ref().unwrap().selection.active, Some(ItemRef::Layer(LayerId(12))));
    assert_eq!(doc.xcf_state.as_ref().unwrap().floating.as_ref().unwrap().target, ItemRef::Layer(LayerId(11)));
    for (new, old) in doc.layers.iter().zip(&before.layers) {
        assert_eq!(new.surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), old.surface().unwrap().try_read_interleaved(before.bounds()).unwrap());
    }
    roundtrip(&doc).unwrap();
}

#[test]
fn zero_collision_and_late_missing_mapping_preserve_exact_native_archives() {
    for failure in 0..3 {
        let mut doc = document().unwrap();
        let before = doc.clone();
        let bytes = archives(&doc).unwrap();
        let mut calls = Vec::new();
        let result = doc.xcf_state.as_mut().unwrap().remap_layers(|id| {
            calls.push(id);
            match failure {
                0 => Some(LayerId(0)),
                1 => Some(LayerId(19)),
                _ => (id == LayerId(1)).then_some(LayerId(11)),
            }
        });
        assert!(result.is_err());
        if failure == 2 {
            assert_eq!(calls, vec![LayerId(1), LayerId(2)]);
        }
        assert_eq!(doc, before);
        assert_eq!(archives(&doc).unwrap(), bytes);
        roundtrip(&doc).unwrap();
    }
}
