//! File › Scripts › Recompress JPEG as JPEG XL… and Restore Original JPEG from JPEG XL…: a JPEG
//! goes to a smaller JPEG XL file and comes back byte for byte, an existing output is left alone
//! unless asked, and a JPEG XL file not made from a JPEG says so.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use photocraft_engine::Session;
use serde_json::json;

const SOURCE_JPEG: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/exif-xmp-orient6.src.jpg");
const RGB8_JXL: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/rgb8-lossless.jxl");

#[test]
fn a_jpeg_goes_to_jpeg_xl_and_back_exactly() {
    let dir = std::env::temp_dir().join(format!("photocraft-jxl-scripts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jpg = dir.join("photo.jpg");
    std::fs::write(&jpg, SOURCE_JPEG).unwrap();
    let mut s = Session::new();

    let r = s.execute("file.scripts.jpegToJpegXl", json!({"paths": [jpg.to_str().unwrap()]})).unwrap();
    let jxl = dir.join("photo.jxl");
    assert_eq!(r["files"][0]["out"], jxl.to_str().unwrap());
    let jxl_bytes = std::fs::read(&jxl).unwrap();
    assert_eq!(photocraft_codecs::detect(&jxl_bytes), Some(photocraft_codecs::Format::Jxl));
    assert_eq!(r["outBytes"], jxl_bytes.len());

    // Back to photo.jpg, which exists: refused, then replaced on request with the same bytes.
    let back = s.execute("file.scripts.jpegXlToJpeg", json!({"path": jxl.to_str().unwrap()}));
    assert!(back.unwrap_err().to_string().contains("exists already"));
    std::fs::write(&jpg, b"overwritten").unwrap();
    s.execute("file.scripts.jpegXlToJpeg", json!({"path": jxl.to_str().unwrap(), "overwrite": true})).unwrap();
    assert_eq!(std::fs::read(&jpg).unwrap(), SOURCE_JPEG, "the original JPEG, byte for byte");

    // A JPEG XL that was not made from a JPEG has no original to restore.
    let plain = dir.join("plain.jxl");
    std::fs::write(&plain, RGB8_JXL).unwrap();
    let e = s.execute("file.scripts.jpegXlToJpeg", json!({"path": plain.to_str().unwrap()})).unwrap_err();
    assert!(e.to_string().contains("not made from a JPEG"), "{e}");
    // A file that is not a JPEG is not recompressed.
    let e = s.execute("file.scripts.jpegToJpegXl", json!({"path": plain.to_str().unwrap(), "overwrite": true})).unwrap_err();
    assert!(e.to_string().contains("plain.jxl"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
