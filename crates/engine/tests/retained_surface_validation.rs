#[path = "../src/retained_surface_validation.rs"]
mod validation;
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size};
use photocraft_engine::Session;
use photocraft_geom::TileCoord;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use serde_json::json;

fn forged_surface() -> Surface {
    let mut original = Surface::new(photocraft_color::PixelFormat::RGBA8);
    original.try_write_pixel_f64(0, 0, &[1.0; 4]).unwrap();
    let tile = original.tiles().next().unwrap().1.clone();
    let mut forged = Surface::new(original.format());
    forged.put_tiles([(TileCoord::new(i32::MAX, i32::MIN), tile)]);
    forged
}

#[test]
fn predispatched_hostile_document_fails_before_generic_rename_or_history_changes() {
    let mut document = Document::new("Own synthetic hostile surface", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    document.layers.push(Layer::raster("Own layer", document.pixel_format()));
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    session.execute("layer.renameLayer", json!({"name":"Validated identity"})).unwrap();
    assert!(session.is_enabled_with("layer.renameLayer", &json!({"name":"Cached identity"})));
    session.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].content = LayerContent::Raster(forged_surface());
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.past_len();
    assert!(!session.is_enabled_with("layer.renameLayer", &json!({"name":"Must remain disabled"})));
    let result = session.execute("layer.renameLayer", json!({"name":"Should never be published"}));
    assert!(result.is_err());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.past_len(), history);
    let called = std::cell::Cell::new(false);
    assert!(
        session
            .edit("Must not enter closure", |_, _| {
                called.set(true);
                Ok(())
            })
            .is_err()
    );
    assert!(!called.get());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.past_len(), history);
}

#[test]
fn newly_forged_edit_candidate_is_rejected_without_publishing_history() {
    let mut document = Document::new("Own synthetic candidate", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    document.layers.push(Layer::raster("Own layer", document.pixel_format()));
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.past_len();
    let result = session.edit("Own forged candidate", |candidate, _| {
        candidate.layers[0].content = LayerContent::Raster(forged_surface());
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.past_len(), history);
}

#[test]
fn guard_rejects_forged_layer_before_geometry() {
    let mut document = Document::new("Own synthetic hostile surface", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    document.layers.push(Layer::new("Own layer", LayerContent::Raster(forged_surface())));
    assert_eq!(validation::validate_document(&document, &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
}

#[test]
fn all_document_surface_holders_are_checked_including_disabled_caches() {
    use photocraft_doc::{AlphaChannel, Fill, FillCache, LayerMask, Pattern, ShapeLayer, SmartObject, SmartSource, TextLayer};
    use photocraft_geom::Affine;
    let base = Document::new("Own surface holders", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut documents = Vec::new();
    let mut document = base.clone();
    document.selection = Some(forged_surface());
    documents.push(document);
    let mut document = base.clone();
    let mut layer = Layer::raster("Own video frame", document.pixel_format());
    layer.video = Some(photocraft_doc::VideoData::new(vec![forged_surface()], 24.0));
    document.layers.push(layer);
    documents.push(document);
    let mut document = base.clone();
    let mut state = photocraft_doc::XcfState::default();
    state.filter_masks.push(photocraft_doc::FilterMask {
        id: photocraft_doc::item_identity::MaskId(1),
        owner: photocraft_doc::item_identity::FilterId(1),
        surface: forged_surface(),
        bounds: photocraft_geom::Rect::new(0, 0, 2, 2),
    });
    document.xcf_state = Some(state);
    documents.push(document);
    let mut document = base.clone();
    document.channels.push(AlphaChannel::new("Own channel", forged_surface()));
    documents.push(document);
    let mut document = base.clone();
    document.quick_mask = Some(AlphaChannel::new("Own quick mask", forged_surface()));
    documents.push(document);
    let mut document = base.clone();
    document.patterns.push(Pattern { id: "own".into(), name: "Own pattern".into(), width: 2, height: 2, surface: forged_surface() });
    documents.push(document);
    let smart = SmartObject::new(
        SmartSource::Embedded { file_name: "No decoding needed".into(), bytes: std::sync::Arc::new(vec![]) },
        Affine::IDENTITY,
        Some(forged_surface()),
    );
    for content in [
        LayerContent::Text(TextLayer { cache: Some(forged_surface()), ..Default::default() }),
        LayerContent::Shape(ShapeLayer { cache: Some(forged_surface()), ..Default::default() }),
        LayerContent::Smart(smart),
    ] {
        let mut document = base.clone();
        let mut layer = Layer::new("Own hidden holder", content);
        layer.visible = false;
        document.layers.push(layer);
        documents.push(document);
    }
    let mut document = base.clone();
    let mut layer = Layer::raster("Own mask", document.pixel_format());
    let mut mask = LayerMask::reveal_all();
    mask.surface = std::sync::Arc::new(forged_surface());
    mask.enabled = false;
    layer.mask = Some(mask);
    document.layers.push(layer);
    documents.push(document);
    let mut document = base;
    let mut layer = Layer::raster("Own fill cache", document.pixel_format());
    layer.fill_cache = Some(FillCache { fill: Fill::Solid(photocraft_color::Color::BLACK), surface: forged_surface() });
    document.layers.push(layer);
    documents.push(document);
    for (index, document) in documents.iter().enumerate() {
        let before = document.clone();
        assert_eq!(validation::validate_document(document, &Interrupt::NONE), Err(RasterInputError::InvalidBounds), "holder {index}");
        assert_eq!(document, &before);
    }
}

#[test]
fn bounded_group_walk_and_cancellation_are_nonmutating() {
    use photocraft_doc::{Group, MAX_GROUP_DEPTH};
    let base = Document::new("Own depth", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    for (count, expected) in [(MAX_GROUP_DEPTH, Ok(())), (MAX_GROUP_DEPTH + 1, Err(RasterInputError::LimitExceeded))] {
        let mut layer = Layer::raster("Own leaf", base.pixel_format());
        for _ in 0..count {
            layer = Layer::new("Own group", LayerContent::Group(Group { children: vec![layer], expanded: false, artboard: None }));
        }
        let mut document = base.clone();
        document.layers.push(layer);
        assert_eq!(validation::validate_document(&document, &Interrupt::NONE), expected);
    }
    let cancelled = Interrupt::new(&|| true, &|_| {});
    assert_eq!(validation::validate_document(&base, &cancelled), Err(RasterInputError::Cancelled));
}
