use photocraft_compose::{PreparedResidentRegion, render};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_geom::Rect;
use photocraft_raster::RasterInputError;

#[test]
fn prepared_resident_sampling_preserves_rgba_and_refuses_unprepared_content() {
    let mut doc = Document::with_background("owned resident sampler", Size::new(4, 3), ColorMode::Rgb, SampleType::U16, Color::WHITE);
    doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(-2, 1, 2, 2), &[0.31, 0.57, 0.73, 0.63]);
    let before = doc.clone();
    let region = Rect::new(-2, 1, 3, 2);
    let prepared = PreparedResidentRegion::new(&doc, region).unwrap();
    assert_eq!(prepared.bounds(), region);
    assert_eq!(prepared.rgba_region(region).unwrap(), render(&doc, region).unwrap().px);
    assert_eq!(prepared.rgba_region(Rect::new(-1, 1, 1, 2)).unwrap(), render(&doc, Rect::new(-1, 1, 1, 2)).unwrap().px);
    assert_eq!(prepared.rgba(3, 1), None);
    assert_eq!(prepared.rgba_region(Rect::new(-3, 1, 3, 2)), Err(RasterInputError::InvalidBounds));
    assert_eq!(prepared.rgba_region(Rect::new(0, 0, 0, 0)), Err(RasterInputError::InvalidBounds));
    assert_eq!(doc, before);
}
