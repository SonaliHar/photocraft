//! A native Save As title is serialization metadata, never a mutation of the input.
use photocraft_doc::{ColorMode, Document, SampleType, Size};
use photocraft_format::{FormatError, PcraftWriter};
use photocraft_raster::Interrupt;
use std::io::Cursor;
#[test]
fn destination_title_changes_only_serialized_name_and_unnamed_stream_stays_literal() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let doc = Document::with_background("synthetic-rgb-float.xcf", Size::new(2, 3), ColorMode::Rgb, depth, photocraft_color::Color::WHITE);
        let before = doc.clone(); // external comparison fixture, not a production save clone.
        let (named, _) = PcraftWriter::new()
            .save_zip_stream_named(
                &doc,
                "native-roundtrip.pcraft",
                &Default::default(),
                Cursor::new(Vec::new()),
                Default::default(),
                64 << 20,
                &Interrupt::NONE,
            )
            .unwrap();
        let mut reopened = photocraft_format::load_from_bytes(named.get_ref()).unwrap();
        assert_eq!(reopened.name, "native-roundtrip.pcraft");
        reopened.name = before.name.clone();
        assert_eq!(reopened, before);
        assert_eq!(doc, before);
        let (unnamed, _) =
            PcraftWriter::new().save_zip_stream(&doc, &Default::default(), Cursor::new(Vec::new()), Default::default(), 64 << 20, &Interrupt::NONE).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(unnamed.get_ref()).unwrap(), before);
        let mut output = Cursor::new(Vec::new());
        assert!(matches!(
            PcraftWriter::new().save_zip_stream_named(
                &doc,
                "native-roundtrip.pcraft",
                &Default::default(),
                &mut output,
                Default::default(),
                64 << 20,
                &Interrupt::cancel_only(&|| true)
            ),
            Err(FormatError::Cancelled)
        ));
        assert!(output.get_ref().is_empty());
        assert_eq!(doc, before);
    }
}
