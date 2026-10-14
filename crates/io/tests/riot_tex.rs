use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, TexCompression};
use photocraft_io::{ExportOptions, export, import};

#[test]
fn tex_alpha_mask_hidden_rgb_and_source_encoding_survive_document_round_trip() {
    let pixels: Vec<u8> = (0..64).flat_map(|i| [(i * 3) as u8, (i * 5) as u8, (i * 7) as u8, if i % 2 == 0 { 0 } else { 127 }]).collect();
    let image = Image::from_u8(8, 8, ChannelLayout::Rgba, pixels.clone()).unwrap();
    for mips in [false, true] {
        let data = photocraft_codecs::encode(&image, Format::RiotTex, &EncodeOptions { tex_mipmaps: mips, ..Default::default() }).unwrap();
        let imported = import("skin.tex", &data).unwrap();
        assert!(!imported.source_read_only);
        assert_eq!(imported.document.channels.len(), 1);
        assert_eq!(imported.document.file_encoding.tex_compression, Some(20));
        assert_eq!(imported.document.file_encoding.tex_mipmaps, Some(mips));
        let mut opts = ExportOptions::default();
        photocraft_io::apply_file_encoding(&imported.document, &mut opts.encode);
        let saved = export(&imported.document, "saved.tex", &opts).unwrap();
        assert_eq!(photocraft_codecs::decode(&saved.bytes).unwrap().data(), pixels);
        assert_eq!(saved.bytes, data);
        // The native project stores the TEX preferences too.
        let project = export(&imported.document, "saved.pcraft", &ExportOptions::default()).unwrap();
        let reopened = import("saved.pcraft", &project.bytes).unwrap();
        assert_eq!(reopened.document.file_encoding, imported.document.file_encoding);
        opts.encode.tex_compression = TexCompression::Bc7;
        assert!(export(&reopened.document, "compressed.tex", &opts).is_ok());
    }
}
