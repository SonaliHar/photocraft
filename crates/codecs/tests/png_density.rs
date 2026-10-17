use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, Metadata, PngDensity, SampleType, encode, encode_png_with_density};
fn crc(bytes: &[u8]) -> u32 {
    let mut c = u32::MAX;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 0 { c >> 1 } else { (c >> 1) ^ 0xedb8_8320 };
        }
    }
    !c
}
fn phys(bytes: &[u8]) -> Option<(u32, u32, u8)> {
    assert_eq!(bytes.get(..8).unwrap(), &[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut p = 8usize;
    let mut found = None;
    while p < bytes.len() {
        let n = usize::try_from(u32::from_be_bytes(bytes.get(p..p + 4).unwrap().try_into().unwrap())).unwrap();
        let end = p.checked_add(n).unwrap().checked_add(12).unwrap();
        let chunk = bytes.get(p + 4..end - 4).unwrap();
        assert_eq!(crc(chunk), u32::from_be_bytes(bytes.get(end - 4..end).unwrap().try_into().unwrap()));
        if chunk.get(..4).unwrap() == b"pHYs" {
            assert!(found.is_none());
            assert_eq!(n, 9);
            found = Some((
                u32::from_be_bytes(chunk.get(4..8).unwrap().try_into().unwrap()),
                u32::from_be_bytes(chunk.get(8..12).unwrap().try_into().unwrap()),
                *chunk.get(12).unwrap(),
            ));
        }
        p = end;
    }
    assert_eq!(p, bytes.len());
    found
}
#[test]
fn density_override_preserves_generic_png_pixels_depth_and_interlacing() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::U8, SampleType::U16] {
            for interlaced in [false, true] {
                let count = 6 * layout.channels();
                let mut image = match sample {
                    SampleType::U8 => Image::from_u8(3, 2, layout, (0..count).map(|i| [0, 1, 127, 128, 254, 255, 63][i % 7]).collect()).unwrap(),
                    SampleType::U16 => {
                        Image::from_u16(3, 2, layout, &(0..count).map(|i| [0, 1, 32767, 32768, 65534, 65535, 12345][i % 7]).collect::<Vec<_>>()).unwrap()
                    }
                    _ => panic!("test sample"),
                };
                image.meta = Metadata { dpi: Some((300.0, 300.0)), text: vec![("Owned test".into(), "Density preserves pixels".into())], ..Default::default() };
                let opts = EncodeOptions { png_interlaced: interlaced, embed_metadata: true, ..Default::default() };
                let ordinary = encode(&image, Format::Png, &opts).unwrap();
                let bytes = encode_png_with_density(&image, &opts, PngDensity::from_dpi(144.25, 287.75).unwrap()).unwrap();
                assert_eq!(phys(&bytes), Some((5679, 11329, 1)));
                let a = photocraft_codecs::decode(&ordinary).unwrap();
                let b = photocraft_codecs::decode(&bytes).unwrap();
                assert_eq!(b.dimensions(), (3, 2));
                assert_eq!(b.sample_type(), sample);
                assert_eq!(b.layout(), layout);
                assert_eq!(a.data(), b.data());
                assert_eq!(b.data(), image.data());
                assert_eq!(a.meta.text, b.meta.text);
                assert_eq!(image.meta.dpi, Some((300.0, 300.0)));
            }
        }
    }
}
#[test]
fn metadata_off_and_palette_encoder_keep_existing_behavior() {
    let image = Image::from_u8(2, 1, ChannelLayout::Rgb, vec![0, 1, 255, 254, 128, 127])
        .unwrap()
        .with_meta(Metadata { dpi: Some((300.0, 300.0)), ..Default::default() });
    let opts = EncodeOptions { embed_metadata: false, ..Default::default() };
    let old = encode(&image, Format::Png, &opts).unwrap();
    let new = encode_png_with_density(&image, &opts, PngDensity::from_dpi(144.25, 287.75).unwrap()).unwrap();
    assert_eq!(old, new);
    assert_eq!(phys(&new), None);
    let bytes = photocraft_codecs::encode_png_indexed(3, 1, &[0, 1, 0], &[[255, 0, 0], [0, 0, 255]], Some(1), None).unwrap();
    assert_eq!(phys(&bytes), None);
    let decoded = photocraft_codecs::decode(&bytes).unwrap().convert(ChannelLayout::Rgba, SampleType::U8);
    assert_eq!(decoded.data(), &[255, 0, 0, 255, 0, 0, 255, 0, 255, 0, 0, 255]);
}
#[test]
fn exact_density_bounds_and_ties_are_encoded_as_png_integers() {
    let image = Image::from_u8(1, 1, ChannelLayout::Gray, vec![127]).unwrap();
    let opts = EncodeOptions { embed_metadata: true, ..Default::default() };
    for (x, y, expected) in [
        (144.25, 287.75, (5679, 11329)),
        (7.9375, 23.8125, (313, 938)),
        (7.9375f64.next_down(), 7.9375f64.next_up(), (312, 313)),
        (0.001, f64::MIN_POSITIVE, (0, 0)),
        (0.0254 * f64::from(i32::MAX), 72.0, (2147483647, 2835)),
    ] {
        let bytes = encode_png_with_density(&image, &opts, PngDensity::from_dpi(x, y).unwrap()).unwrap();
        assert_eq!(phys(&bytes), Some((expected.0, expected.1, 1)));
    }
    for bad in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::MAX, 0.0254 * (f64::from(i32::MAX) + 1.0)] {
        assert!(PngDensity::from_dpi(bad, 72.0).is_err());
        assert!(PngDensity::from_dpi(72.0, bad).is_err());
    }
}
