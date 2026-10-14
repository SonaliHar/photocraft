//! Create frozen high-depth variants outside measured runs; refuses replacement.
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Layer, LayerContent};
use photocraft_io::{ExportOptions, export, import};
use std::{error::Error, path::Path};
fn convert_layers(layers: &mut [Layer], depth: SampleType) {
    for layer in layers {
        match &mut layer.content {
            LayerContent::Raster(s) => {
                let f = s.format();
                *s = s.convert(PixelFormat::new(f.mode, depth, f.alpha));
            }
            LayerContent::Group(g) => convert_layers(&mut g.children, depth),
            _ => (),
        }
        if let Some(m) = &mut layer.mask {
            let f = m.surface.format();
            m.surface = m.surface.convert(PixelFormat::new(f.mode, depth, f.alpha));
        }
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let source = args.first().ok_or("source PSD required")?;
    let target = args.get(1).ok_or("output PSD required")?;
    if Path::new(target).exists() {
        return Err("frozen fixture already exists".into());
    }
    let depth = match args.get(2).map(String::as_str) {
        Some("16") => SampleType::U16,
        Some("32") => SampleType::F32,
        _ => return Err("depth must be16or32".into()),
    };
    let bytes = std::fs::read(source)?;
    let mut doc = import("source.psd", &bytes)?.document;
    convert_layers(&mut doc.layers, depth);
    doc.depth = depth;
    let output = export(&doc, "depth.psd", &ExportOptions::default())?;
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(target)?;
    use std::io::Write;
    (&file).write_all(&output.bytes)?;
    println!("{} {} bytes source={} output={}", target, output.bytes.len(), blake3::hash(&bytes), blake3::hash(&output.bytes));
    Ok(())
}
