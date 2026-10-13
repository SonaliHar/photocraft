use photocraft_doc::{ColorMode, Document, Layer, MAX_GROUP_DEPTH, PixelFormat, SampleType, Size};
use photocraft_raster::RasterInputError;

#[test]
fn structural_backing_guard_preserves_the_existing_native_group_depth_boundary() {
    for depth in [MAX_GROUP_DEPTH, MAX_GROUP_DEPTH + 1] {
        let mut layer = Layer::raster("original resident leaf", PixelFormat::RGBA16);
        for _ in 0..depth {
            layer = Layer::group("original resident group", vec![layer]);
        }
        let mut doc = Document::new("native depth boundary", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
        doc.layers.push(layer);
        let expected = if depth == MAX_GROUP_DEPTH { Ok(false) } else { Err(RasterInputError::LimitExceeded) };
        assert_eq!(doc.has_backed_rasters(), expected);
        assert_eq!(doc.layers[0].has_backed_rasters(), expected);
    }
}
