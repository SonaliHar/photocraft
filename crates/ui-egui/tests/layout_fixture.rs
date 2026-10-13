//! The benchmark must recover its Unicode text target after a PSD round trip.
#[path = "support/layout_doc.rs"]
mod layout_doc;

use photocraft_doc::{ColorMode, Document, LayerContent, SampleType};
use photocraft_geom::Size;

#[test]
fn product_title_is_resolvable_before_and_after_psd_reopen() {
    let (doc, _) = layout_doc::build(layout_doc::Spec { width: 400, height: 300, cards: 1, tiles: 1 });
    let name = "Product 1 \u{2014} Oak";
    let text = |content: &LayerContent| matches!(content, LayerContent::Text(_));
    let id = layout_doc::find_named(&doc, name, &text).expect("benchmark text target");
    assert_eq!(
        doc.layer(id).and_then(|layer| match &layer.content {
            LayerContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        }),
        Some(name)
    );
    let psd = photocraft_io::export(&doc, "layout.psd", &Default::default()).expect("export").bytes;
    let reopened = photocraft_io::import("layout.psd", &psd).expect("reopen").document;
    let id = layout_doc::find_named(&reopened, name, &text).expect("reopened benchmark text target");
    assert_eq!(
        reopened.layer(id).and_then(|layer| match &layer.content {
            LayerContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        }),
        Some(name)
    );
}

#[test]
fn missing_benchmark_target_returns_error() {
    let doc = Document::new("empty", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    assert!(layout_doc::find_named(&doc, "absent", &|_| true).is_err());
}
