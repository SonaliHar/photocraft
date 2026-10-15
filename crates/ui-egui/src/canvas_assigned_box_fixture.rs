//! Assigned Box source/edit lifecycle inputs. File/native buffers and Cursor
//! output are external harness captures, not production accounting evidence.
//! Four loose producer ICC captures are absent: no producer-profile equality
//! or new numerical acceptance claim is made by these tests.
//! The producer's "unit" filename is not a unit-RGB assertion: five actual
//! finite RGB words are mildly outside [0,1] and must remain unclamped.
use photocraft_doc::{Document, LayerContent};
use photocraft_raster::{Interrupt, backing::ReadControl};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
pub(super) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) fn cases() -> Vec<(PathBuf, Vec<u8>)> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("box-raster-color-evidence");
    [
        ("source", "e4f31ba998692bb7f57ebc82433a4c8d90a7aeaeb2d67a9276079bb594d4cec8"),
        ("edit", "0720b4beee6ebca7484dc8ef3f152b0c22f1430cf32c31811db5a011ccec22aa"),
    ]
    .into_iter()
    .map(|(phase, pin)| {
        let path = root.join(format!("assigned-unit-f32.{phase}.xcf"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(hash(&bytes), pin);
        let native = std::fs::read(root.join(format!("assigned-unit-f32.{phase}.reopened.layer.buffer-extent.native.none.bin"))).unwrap();
        assert_eq!(hash(&native), "3102cfa5ba9eb6cf718b767257d474fc865e0b5fa93e8b3622adeebd7fefaadf");
        assert_eq!(native.len(), 3072);
        (path, bytes)
    })
    .collect()
}
pub(super) fn native(doc: &Document) -> Vec<u8> {
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("actual native layer") };
    let mut bytes = vec![0; 3072];
    for coord in layer.operative_bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        let part = coord.rect().intersect(&layer.operative_bounds());
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let mut pixel = [0; 16];
                pin.read_pixel_native_into(x, y, &mut pixel, &ctl()).unwrap();
                // Producer native captures explicitly declare little endian.
                for (channel, word) in pixel.chunks_exact(4).enumerate() {
                    let at = (y as usize * 16 + x as usize) * 16 + channel * 4;
                    bytes[at..at + 4].copy_from_slice(&u32::from_ne_bytes(word.try_into().unwrap()).to_le_bytes());
                }
            }
        }
    }
    bytes
}
pub(super) fn observed(doc: &Document) {
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_deref().unwrap();
    // Independent bounded ICC extraction eece3885, from actual saved XCFs;
    // this is not equality to the four absent producer ICC captures.
    assert_eq!(original.len(), 640);
    assert_eq!(hash(original), "abae4941f28e3558cb648775352364b2978e07a3b49e78228836b98a4ac05f46");
    for (offset, gamma) in [(520, 104858i32), (560, 150733), (600, 183501)] {
        assert_eq!(&original[offset..offset + 4], b"para");
        assert_eq!(u16::from_be_bytes(original[offset + 8..offset + 10].try_into().unwrap()), 4);
        let words: Vec<_> = original[offset + 12..offset + 40].chunks_exact(4).map(|b| i32::from_be_bytes(b.try_into().unwrap())).collect();
        assert_eq!(words, [gamma, 65536, 0, 65536, 0, 0, 0]);
    }
    assert!(doc.icc_profile.is_some());
    let native = native(doc);
    assert_eq!(hash(&native), "3102cfa5ba9eb6cf718b767257d474fc865e0b5fa93e8b3622adeebd7fefaadf");
    let outside: Vec<_> = native
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(i, raw)| {
            let bits = u32::from_le_bytes(raw.try_into().unwrap());
            let value = f32::from_bits(bits);
            assert!(value.is_finite());
            if i % 4 == 3 {
                assert!((0.0..=1.0).contains(&value));
            }
            (i % 4 != 3 && !(0.0..=1.0).contains(&value)).then_some((i, bits))
        })
        .collect();
    assert_eq!(outside, [(209, 0x3f80160b), (250, 0xba15c5a6), (290, 0x3f8024b8), (294, 0xbb93e671), (630, 0xb9477e7b)]);
}
pub(super) fn exact(a: &Document, b: &Document) {
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.xcf_state, b.xcf_state);
    assert_eq!(a.layers[0].visible, b.layers[0].visible);
    assert_eq!(a.layers[0].opacity.to_bits(), b.layers[0].opacity.to_bits());
    assert_eq!(a.layers[0].fill_opacity.to_bits(), b.layers[0].fill_opacity.to_bits());
    assert_eq!(a.layers[0].blend, b.layers[0].blend);
    assert_eq!(native(a), native(b));
    let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&a.layers[0].content, &b.layers[0].content) else { panic!("native layers") };
    assert_eq!(a.descriptor(), b.descriptor());
    assert_eq!(a.default_bytes(), b.default_bytes());
    for coord in a.descriptor().bounds().tiles() {
        let left = a.checked_view().pin_tile(coord, &ctl()).unwrap();
        let right = b.checked_view().pin_tile(coord, &ctl()).unwrap();
        use photocraft_raster::checked_view::NativeTileBytes;
        match (left.encoded(), right.encoded()) {
            (NativeTileBytes::FullTile(a), NativeTileBytes::FullTile(b)) | (NativeTileBytes::DefaultPixel(a), NativeTileBytes::DefaultPixel(b)) => {
                assert_eq!(a, b)
            }
            _ => panic!("native storage representation changed"),
        }
    }
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(owner.document(), Cursor::new(Vec::new()), Default::default(), owner.quota(), &ctl())
            .unwrap()
            .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        Default::default(),
        owner.quota().clone(),
        &ctl(),
    )
    .unwrap()
}
