//! Local diagnostic thumbnails; never writes beside the original photograph.
use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, EncodeOptions, Format, Image};
fn thumbnail(image: &Image, source: &photocraft_cms::Profile) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let width = image.width().min(1000);
    let height = ((u64::from(image.height()) * u64::from(width)) / u64::from(image.width())) as u32;
    let transform = Transform::new(source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false)?;
    let mut input = Vec::new();
    for y in 0..height {
        for x in 0..width {
            for c in 0..3 {
                input.push(image.get(x * image.width() / width, y * image.height() / height, c));
            }
        }
    }
    let mut output = vec![0.0; input.len()];
    transform.convert_f32(&input, 3, &mut output, 3, false);
    let mut image = Image::from_u8(width, height, ChannelLayout::Rgb, output.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect())?;
    image.icc = Some(Builtin::Srgb.profile().to_bytes().as_ref().clone());
    Ok(codecs::encode(&image, Format::Png, &EncodeOptions::default())?)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = std::path::PathBuf::from(args.next().ok_or("output directory")?);
    std::fs::create_dir_all(&output)?;
    for arg in args {
        let path = std::path::PathBuf::from(arg);
        let bytes = std::fs::read(&path)?;
        let id = path.file_stem().and_then(|v| v.to_str()).ok_or("file name")?;
        if let Some(preview) = photocraft_raw::embedded_preview(&bytes) {
            let image = codecs::decode_as(Format::Jpeg, preview.jpeg)?;
            let profile =
                if photocraft_raw::cr3::metadata(&bytes).color_space() == Some(2) { Builtin::AdobeRgbCompat.profile() } else { Builtin::Srgb.profile() };
            std::fs::write(output.join(format!("{id}-camera.png")), thumbnail(&image, profile)?)?;
        }
        let result = photocraft_io::import(id, &bytes)?;
        for warning in result.warnings {
            println!("{id}: {warning}");
        }
        let encoded = photocraft_io::export(&result.document, "image.png", &Default::default())?;
        let image = codecs::decode(&encoded.bytes)?;
        let source = photocraft_cms::Profile::parse(result.document.icc_profile.as_ref().ok_or("profile")?)?;
        std::fs::write(output.join(format!("{id}-native.png")), thumbnail(&image, &source)?)?;
    }
    Ok(())
}
