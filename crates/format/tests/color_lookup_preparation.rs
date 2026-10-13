use photocraft_doc::{Adjustment, ColorMode, Document, Layer, LayerContent, SampleType, Size};
use std::sync::Arc;

#[test]
fn ordinary_shared_color_lookup_exceeding_64k_json_saves_exactly() {
    let values: Vec<f32> = (0..33_usize.pow(3) * 3)
        .map(|index| match index % 3 {
            0 => -0.0,
            1 => f32::from_bits(0x3eaaaaab),
            _ => 0.75,
        })
        .collect();
    let lut = Arc::new(values);
    let adjustment = Adjustment::ColorLookup { name: "Original Ω lookup".into(), lut: Some(lut.clone()), size: 33, tetrahedral: true, dither: true };
    assert!(serde_json::to_vec(&adjustment).unwrap().len() > 65536);
    let mut document = Document::new("shared lookup", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("lookup", photocraft_doc::PixelFormat::RGBA8);
    layer.content = LayerContent::Adjustment(adjustment);
    document.layers.push(layer);
    let original_refs = Arc::strong_count(&lut);
    let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    assert_eq!(Arc::strong_count(&lut), original_refs);
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(reopened, document);
    let LayerContent::Adjustment(Adjustment::ColorLookup { lut: Some(actual), .. }) = &reopened.layers[0].content else { panic!("lookup state") };
    assert_eq!(actual.iter().map(|value| value.to_bits()).collect::<Vec<_>>(), lut.iter().map(|value| value.to_bits()).collect::<Vec<_>>());
}
