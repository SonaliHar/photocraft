//! Controlled native-save transactions retain current/source bytes and atomic cancellation.
use photocraft_doc::{Color, ColorMode, Document, Layer, SampleType, Size};
use photocraft_format::{PreparationLimits, load_from_bytes};
use photocraft_io::{NativeSaveLimits, save_native_stream};
use photocraft_raster::Interrupt;
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
#[test]
fn controlled_native_save_preserves_source_and_current_wide_bits() {
    let mut doc = Document::with_background("Own wide", Size::new(2, 1), ColorMode::Rgb, SampleType::F64, Color::TRANSPARENT);
    let surface = doc.layers[0].surface_mut().unwrap();
    surface.try_write_pixel_f64(0, 0, &[0.5000000000000001, 0.2, 0.3, 1.0]).unwrap();
    doc.metadata.exif = Some(Arc::new(vec![71; 8192]));
    let before = doc.clone();
    let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), NativeSaveLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
    assert_eq!(doc, before);
    assert!(result.retained_payload_bytes >= 8192);
}
#[test]
fn aggregate_retained_payload_and_preparation_share_one_lowered_budget() {
    let mut doc = Document::new("Own budget", Size::new(1, 1), ColorMode::Rgb, SampleType::U32);
    doc.metadata.exif = Some(Arc::new(vec![23; 65536]));
    let limits = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 65535, ..Default::default() }, ..Default::default() };
    let mut output = Cursor::new(Vec::new());
    assert!(save_native_stream(&doc, &mut output, limits, &Interrupt::NONE).is_err());
    assert!(output.get_ref().is_empty());
}
#[test]
fn late_job_token_cancels_preview_or_preparation_before_archive_output() {
    let mut doc = Document::new("Own late cancel", Size::new(512, 512), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(Layer::raster("layer", doc.pixel_format()));
    doc.metadata.exif = Some(Arc::new(vec![19; 1 << 20]));
    let before = doc.clone();
    let polls = AtomicUsize::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 12;
    let mut output = Cursor::new(Vec::new());
    assert!(save_native_stream(&doc, &mut output, Default::default(), &Interrupt::cancel_only(&cancelled)).is_err());
    assert!(output.get_ref().is_empty());
    assert_eq!(doc, before);
    assert!(polls.load(Ordering::Relaxed) > 12);
}

#[test]
fn retained_accounting_deduplicates_shared_tiles_and_blobs() {
    let mut doc = Document::with_background("own shared", Size::new(1, 1), ColorMode::Rgb, SampleType::F64, Color::TRANSPARENT);
    doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.2, 0.3, 0.4, 1.0]).unwrap();
    let blob = Arc::new(vec![17; 8192]);
    doc.metadata.exif = Some(blob.clone());
    let before = photocraft_io::retained_native_payload_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    doc.metadata.psd_resources.push((1000, "own".into(), blob));
    doc.layers.push(doc.layers[0].clone());
    let after = photocraft_io::retained_native_payload_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    // The separately owned default pixel is new; shared tile and blob payloads are not.
    assert_eq!(after - before, doc.layers[0].surface().unwrap().default_bytes().len());
}
#[test]
fn cancellation_preserves_existing_destination() {
    let doc = Document::new("own atomic", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let path = std::env::temp_dir().join(format!("photocraft-sol2-native-cancel-{}.pcraft", std::process::id()));
    std::fs::write(&path, b"previous destination").unwrap();
    assert!(photocraft_io::save_native_path(&doc, &path, Default::default(), &Interrupt::cancel_only(&|| true)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"previous destination");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn forged_retained_tile_origin_is_rejected_before_preview_and_output() {
    let mut doc = Document::with_background("own forged", Size::new(1, 1), ColorMode::Rgb, SampleType::F64, Color::TRANSPARENT);
    let surface = doc.layers[0].surface_mut().unwrap();
    surface.try_write_pixel_f64(0, 0, &[0.2, 0.3, 0.4, 1.0]).unwrap();
    let tile = surface.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap().clone();
    surface.put_tiles([(photocraft_geom::TileCoord::new(i32::MAX, 0), tile)]);
    let before = doc.clone();
    let mut output = Cursor::new(Vec::new());
    assert!(save_native_stream(&doc, &mut output, Default::default(), &Interrupt::NONE).is_err());
    assert!(output.get_ref().is_empty());
    assert_eq!(doc, before);
}

#[test]
fn spare_document_name_capacity_is_charged_before_preview_or_output() {
    let mut doc = Document::new("own", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut name = String::with_capacity(1 << 20);
    name.push_str("own");
    doc.name = name;
    let limits = PreparationLimits { resident_bytes: 65536, ..Default::default() };
    assert!(photocraft_io::retained_native_payload_bytes(&doc, limits, &Interrupt::NONE).is_ok());
    assert!(photocraft_io::retained_native_input_bytes(&doc, limits, &Interrupt::NONE).is_err());
    let mut output = Cursor::new(Vec::new());
    assert!(save_native_stream(&doc, &mut output, NativeSaveLimits { preparation: limits, ..Default::default() }, &Interrupt::NONE).is_err());
    assert!(output.get_ref().is_empty());
    assert_eq!(doc.name, "own");
}
#[test]
fn canonical_graph_spare_slots_and_curve_samples_are_measured_exactly() {
    use photocraft_doc::{XcfState, item_identity::*, xcf_filters::*, xcf_items::*};
    let mut doc = Document::new("own", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let graph = FilterGraph { input: None, nodes: Vec::new(), edges: Vec::new(), output: GraphNodeId(1), original_pipeline: None };
    let mut state = XcfState::default();
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(photocraft_doc::LayerId(1)),
        source: EditableSource::Linked {
            resource: ResourceId(1),
            last_verified_hash: None,
            configuration: vec![FilterArgument { name: "own".into(), value: FilterValue::Graph(graph) }],
            current: None,
        },
        transform: None,
        cache: None,
    });
    doc.xcf_state = Some(state);
    let before = photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    let source = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source;
    let EditableSource::Linked { configuration, .. } = source else { panic!("wrong source") };
    let FilterValue::Graph(graph) = &mut configuration[0].value else { panic!("wrong value") };
    graph.nodes.reserve_exact(4096);
    let extra = graph.nodes.capacity() * std::mem::size_of::<GraphNode>();
    let after = photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(after - before, extra);
    let limits = PreparationLimits { resident_bytes: before + extra - 1, ..Default::default() };
    assert!(photocraft_io::retained_native_input_bytes(&doc, limits, &Interrupt::NONE).is_err());
    let EditableSource::Linked { configuration, .. } = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source else { panic!("wrong source") };
    configuration[0].value = FilterValue::Curve { free: true, points_f64_bits: Vec::new(), point_types: Vec::new(), samples_f64_bits: Vec::new() };
    let before = photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    let EditableSource::Linked { configuration, .. } = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source else { panic!("wrong source") };
    let FilterValue::Curve { samples_f64_bits, .. } = &mut configuration[0].value else { panic!("wrong curve") };
    samples_f64_bits.reserve_exact(16384);
    let extra = samples_f64_bits.capacity() * std::mem::size_of::<u64>();
    let after = photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(after - before, extra);
}
#[test]
fn canonical_capacity_walk_cancels_and_bounds_nested_config() {
    use photocraft_doc::{XcfState, item_identity::*, xcf_filters::*, xcf_items::*};
    let mut value = FilterValue::Bool(true);
    for _ in 0..34 {
        value = FilterValue::Config(vec![FilterArgument { name: "own".into(), value }]);
    }
    let mut state = XcfState::default();
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(photocraft_doc::LayerId(1)),
        source: EditableSource::Linked {
            resource: ResourceId(1),
            last_verified_hash: None,
            configuration: vec![FilterArgument { name: "own".into(), value }],
            current: None,
        },
        transform: None,
        cache: None,
    });
    let mut doc = Document::new("own", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.xcf_state = Some(state);
    assert!(photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).is_err());
    assert!(matches!(
        photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::cancel_only(&|| true)),
        Err(photocraft_io::IoError::Cancelled)
    ));
}
#[test]
fn metadata_and_channel_spare_capacity_is_charged_before_output() {
    let mut doc = Document::new("own", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let before = photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    let mut xmp = String::with_capacity(1 << 20);
    xmp.push_str("own metadata");
    let extra = xmp.capacity();
    doc.metadata.xmp = Some(xmp);
    let after = photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(after - before, extra);
    let before = after;
    doc.channels.reserve_exact(1024);
    let extra = doc.channels.capacity() * std::mem::size_of::<photocraft_doc::AlphaChannel>();
    let after = photocraft_io::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(after - before, extra);
    let mut output = Cursor::new(Vec::new());
    let limits = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: after - 1, ..Default::default() }, ..Default::default() };
    assert!(save_native_stream(&doc, &mut output, limits, &Interrupt::NONE).is_err());
    assert!(output.get_ref().is_empty());
    assert_eq!(doc.metadata.xmp.as_deref(), Some("own metadata"));
    assert!(doc.channels.is_empty());
}
#[test]
fn controlled_byte_export_is_deterministic_and_legacy_contents_match() {
    let mut doc = Document::with_background("own bytes", Size::new(2, 1), ColorMode::Rgb, SampleType::F64, Color::TRANSPARENT);
    doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.20000000000001, -0.0, 0.4, 1.0]).unwrap();
    let before = doc.clone();
    let (first, _) = photocraft_io::save_native_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    let (second, _) = photocraft_io::save_native_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(first, second);
    assert_eq!(load_from_bytes(&first).unwrap(), before);
    let public = photocraft_io::export(&doc, "pcraft", &Default::default()).unwrap();
    assert_eq!(public.bytes, first);
    assert_eq!(doc, before);
    assert!(matches!(photocraft_io::save_native_bytes(&doc, Default::default(), &Interrupt::cancel_only(&|| true)), Err(photocraft_io::IoError::Cancelled)));
}
#[test]
fn byte_output_growth_shares_preparation_and_retained_resident_budget() {
    let mut doc = Document::new("own combined", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.metadata.exif = Some(Arc::new(vec![17; 32 << 20]));
    let archive = photocraft_format::zip::stream::Limits { max_metadata_bytes: 1 << 20, ..Default::default() };
    let limits = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 64 << 20, ..Default::default() }, archive };
    // A file sink needs no archive-sized allocation; the byte sink must account for it
    // together with retained source and prepared manifest/object state.
    let path = std::env::temp_dir().join(format!("photocraft-sol2-native-ledger-{}.pcraft", std::process::id()));
    photocraft_io::save_native_path(&doc, &path, limits, &Interrupt::NONE).unwrap();
    std::fs::remove_file(path).unwrap();
    let result = photocraft_io::save_native_bytes(&doc, limits, &Interrupt::NONE);
    assert!(result.is_err());
    assert_eq!(doc.metadata.exif.as_ref().unwrap().len(), 32 << 20);
    let (bytes, _) = photocraft_io::save_native_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn plain_previews_use_lowered_actual_workspace_instead_of_fixed_allowance() {
    let doc = Document::with_background("Own small plain preview", Size::new(8, 6), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let before = doc.clone();
    let mut limits = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 8 << 20, ..Default::default() }, ..Default::default() };
    // The archive metadata workspace must also fit this deliberately lowered
    // resident cap; default archive limits legitimately require more than 8MiB.
    let mut unchanged = Cursor::new(Vec::new());
    assert!(save_native_stream(&doc, &mut unchanged, limits, &Interrupt::NONE).is_err());
    assert!(unchanged.get_ref().is_empty());
    limits.archive.max_metadata_bytes = 1 << 20;
    let (output, _) = save_native_stream(&doc, Cursor::new(Vec::new()), limits, &Interrupt::NONE).unwrap();
    assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
    assert_eq!(doc, before);
}

#[test]
fn bounded_styled_previews_share_lowered_native_residency_and_keep_source() {
    let mut doc =
        Document::with_background("Own bounded styled preview", Size::new(48, 36), ColorMode::Rgb, SampleType::U16, Color::rgba(0.21, 0.63, 0.17, 0.71));
    let mut mask = photocraft_doc::LayerMask::hide_all();
    std::sync::Arc::make_mut(&mut mask.surface).fill_rect(photocraft_geom::Rect::new(5, 7, 39, 31), &[0.73]);
    mask.feather = 4.25;
    mask.density = 0.81;
    doc.layers[0].mask = Some(mask);
    let mut vector = photocraft_doc::VectorMask::new(photocraft_vector::shapes::ellipse(2.25, 3.75, 42.5, 29.25));
    vector.feather = 2.25;
    vector.density = 0.79;
    doc.layers[0].vector_mask = Some(vector);
    let before = doc.clone();
    let mut limits = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 8 << 20, ..Default::default() }, ..Default::default() };
    limits.archive.max_metadata_bytes = 1 << 20;
    let (output, _) = save_native_stream(&doc, Cursor::new(Vec::new()), limits, &Interrupt::NONE).unwrap();
    assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
    assert_eq!(doc, before);
}

#[test]
fn styled_preview_cancellation_and_small_cap_leave_destination_unchanged() {
    let mut doc = Document::with_background("Own cancellation", Size::new(48, 36), ColorMode::Rgb, SampleType::U16, Color::rgba(0.21, 0.63, 0.17, 0.71));
    let mut mask = photocraft_doc::LayerMask::hide_all();
    std::sync::Arc::make_mut(&mut mask.surface).fill_rect(photocraft_geom::Rect::new(5, 7, 39, 31), &[0.73]);
    mask.feather = 4.25;
    doc.layers[0].mask = Some(mask);
    let before = doc.clone();
    let count = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 500;
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(save_native_stream(&doc, &mut output, Default::default(), &Interrupt::cancel_only(&cancel)), Err(photocraft_io::IoError::Cancelled)));
    assert!(output.get_ref().is_empty());
    assert_eq!(doc, before);
    let mut limits = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 48 << 10, ..Default::default() }, ..Default::default() };
    limits.archive.max_metadata_bytes = 1024;
    assert!(save_native_stream(&doc, &mut output, limits, &Interrupt::NONE).is_err());
    assert!(output.get_ref().is_empty());
    assert_eq!(doc, before);
    // Spread is now part of the admitted styled-preview route; retain its
    // complete native model through the same controlled save.
    let mut effect = photocraft_doc::Effect::default_drop_shadow();
    if let photocraft_doc::Effect::DropShadow(s) = &mut effect {
        s.spread = 0.2;
    }
    doc.layers[0].effects.items.push(effect);
    assert!(photocraft_compose::styled_preview::styled_preview_supported(&doc));
    let (output, _) = save_native_stream(&doc, Cursor::new(Vec::new()), Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(load_from_bytes(output.get_ref()).unwrap(), doc);
}

#[test]
fn charged_proxy_save_avoids_whole_document_clone_and_preserves_metadata() {
    let mut doc = Document::with_background("Own proxy metadata Ω", Size::new(4097, 4097), ColorMode::Rgb, SampleType::U8, Color::rgba(0.21, 0.63, 0.17, 0.71));
    doc.layers[0].name = "Own layer λ".repeat(1000);
    doc.metadata.xmp = Some("Own inert metadata Ω".repeat(100000));
    doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(-3, 5, 412, 317), &[0.73, 0.18, 0.51, 0.0]);
    let before = doc.clone();
    let mut limits = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 128 << 20, ..Default::default() }, ..Default::default() };
    limits.archive.max_metadata_bytes = 8 << 20;
    let (output, _) = save_native_stream(&doc, Cursor::new(Vec::new()), limits, &Interrupt::NONE).unwrap();
    assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
    assert_eq!(doc, before);
}

#[test]
fn charged_proxy_cancel_or_aggregate_denial_keeps_native_destination_empty() {
    let doc = Document::with_background("Own proxy denial", Size::new(4097, 4097), ColorMode::Rgb, SampleType::U8, Color::rgba(0.21, 0.63, 0.17, 0.71));
    let before = doc.clone();
    let count = AtomicUsize::new(0);
    let cancel = || count.fetch_add(1, Ordering::Relaxed) >= 1500;
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(save_native_stream(&doc, &mut output, Default::default(), &Interrupt::cancel_only(&cancel)), Err(photocraft_io::IoError::Cancelled)));
    assert!(output.get_ref().is_empty());
    assert_eq!(doc, before);
    let limits = NativeSaveLimits {
        preparation: PreparationLimits { resident_bytes: 8 << 20, ..Default::default() },
        archive: photocraft_format::zip::stream::Limits { max_metadata_bytes: 1 << 20, ..Default::default() },
    };
    assert!(save_native_stream(&doc, &mut output, limits, &Interrupt::NONE).is_err());
    assert!(output.get_ref().is_empty());
    assert_eq!(doc, before);
}

#[test]
fn checked_noisy_shadow_native_previews_share_lowered_budget_and_preserve_source() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::with_background("Own noisy shadow", Size::new(82, 63), ColorMode::Rgb, depth, Color::rgba(0.3, 0.2, 0.4, 0.73));
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_doc::Rect::new(3, 7, 38, 57), &[0.6, 0.2, 0.1, 0.]);
        let mut effect = photocraft_doc::Effect::default_drop_shadow();
        let photocraft_doc::Effect::DropShadow(shadow) = &mut effect else {
            panic!("default shadow");
        };
        shadow.spread = 0.;
        shadow.noise = 0.27;
        shadow.size = 4.;
        shadow.distance = 3.;
        shadow.contour = photocraft_doc::Contour::Linear;
        let inner = photocraft_doc::Effect::InnerShadow(shadow.clone());
        doc.layers[0].effects.items = vec![effect, inner];
        assert!(photocraft_compose::styled_preview::styled_preview_supported(&doc));
        let before = doc.clone();
        let limits = NativeSaveLimits {
            preparation: PreparationLimits { resident_bytes: 16 << 20, ..Default::default() },
            archive: photocraft_format::zip::stream::Limits { max_metadata_bytes: 1 << 20, ..Default::default() },
        };
        let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), limits, &Interrupt::NONE).unwrap();
        assert!(result.warnings.is_empty());
        assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
        let mut archive = photocraft_format::zip::stream::Reader::new(Cursor::new(output.get_ref()), limits.archive, &Interrupt::NONE).unwrap();
        for (path, side) in [("thumb.png", 256), ("composite/preview.png", 1024)] {
            let png = archive.read_by_name(path, 8 << 20, &Interrupt::NONE).unwrap();
            assert_eq!(photocraft_codecs::decode(&png).unwrap().to_rgba8(), photocraft_compose::thumbnail(&doc, side).unwrap().pixels);
        }
        let mut destination = Cursor::new(Vec::new());
        for stop_at in [1, 30, 100] {
            let count = AtomicUsize::new(0);
            let cancel = || count.fetch_add(1, Ordering::Relaxed) >= stop_at;
            assert!(matches!(save_native_stream(&doc, &mut destination, limits, &Interrupt::cancel_only(&cancel)), Err(photocraft_io::IoError::Cancelled)));
            assert!(destination.get_ref().is_empty());
            assert_eq!(doc, before);
        }
        let denied = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 1 << 20, ..limits.preparation }, ..limits };
        assert!(save_native_stream(&doc, &mut destination, denied, &Interrupt::NONE).is_err());
        assert!(destination.get_ref().is_empty());
        assert_eq!(doc, before);
    }
}
