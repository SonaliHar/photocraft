//! Native-only author of the fixed synthetic diagnostic PSD, with no fonts embedded.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size, TextLayer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new("PhotoCraft boot contract", Size::new(128, 64), ColorMode::Rgb, SampleType::U8);
    let text = TextLayer { text: "PhotoCraft boot contract".into(), ..Default::default() };
    doc.layers.push(Layer::new("Editable diagnostic type", LayerContent::Text(text)));
    let bytes = photocraft_io::export(&doc, "boot-contract.psd", &Default::default())?.bytes;
    let reopened = photocraft_io::import("boot-contract.psd", &bytes)?;
    if !reopened
        .document
        .walk()
        .iter()
        .any(|(_, _, layer)| matches!(&layer.content, LayerContent::Text(text) if text.cache.as_ref().is_some_and(|cache| !cache.content_bounds().is_empty())))
    {
        return Err("generated diagnostic PSD did not exercise missing-cache text preparation".into());
    }
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/photocraft-web/src/boot-contract-fixture/synthetic-type.psd");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}
