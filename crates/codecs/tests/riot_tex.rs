use photocraft_codecs::*;

fn pattern(w: u32, h: u32) -> Image {
    let data = (0..w * h).flat_map(|p| [(p * 7) as u8, (p * 11) as u8, (p * 19) as u8, (p * 5) as u8]).collect();
    Image::from_u8(w, h, ChannelLayout::Rgba, data).unwrap()
}

#[test]
fn bgra_round_trip_keeps_hidden_rgb_and_mips_in_riot_order() {
    let img = pattern(8, 4);
    for mipmaps in [false, true] {
        let opts = EncodeOptions { tex_mipmaps: mipmaps, ..Default::default() };
        let bytes = encode(&img, Format::RiotTex, &opts).unwrap();
        assert_eq!(detect(&bytes), Some(Format::RiotTex));
        assert_eq!(from_extension("skin.TEX"), Some(Format::RiotTex));
        assert_eq!(&bytes[..12], &[84, 69, 88, 0, 8, 0, 4, 0, 1, 20, 0, u8::from(mipmaps)]);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.data(), img.data());
        assert_eq!(bytes.len(), 12 + 128 + if mipmaps { 32 + 8 + 4 } else { 0 });
        assert_eq!(&bytes[bytes.len() - 4..], &[77, 85, 217, 155]);
    }
}

#[test]
fn all_compressed_encodings_have_valid_mip_tails_and_channels() {
    let img = Image::from_u8(8, 4, ChannelLayout::Rgba, [64, 128, 192, 96].repeat(32)).unwrap();
    for compression in TexCompression::ALL {
        let source = if compression == TexCompression::Bc1 { img.convert(ChannelLayout::Rgb, SampleType::U8) } else { img.clone() };
        let bytes = encode(&source, Format::RiotTex, &EncodeOptions { tex_compression: compression, ..Default::default() }).unwrap();
        let info = tex_info(&bytes).unwrap();
        assert_eq!(info.compression, compression);
        assert!(info.mipmaps);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.dimensions(), (8, 4));
        for p in decoded.data().as_chunks::<4>().0 {
            assert!(p[0].abs_diff(64) <= 8, "{compression:?}: {p:?}");
            assert!(p[1].abs_diff(128) <= 8, "{compression:?}: {p:?}");
            if compression != TexCompression::Bc5 {
                assert!(p[2].abs_diff(192) <= 8, "{compression:?}: {p:?}");
            }
            let alpha = if matches!(compression, TexCompression::Bc1 | TexCompression::Bc5) { 255 } else { 96 };
            assert!(p[3].abs_diff(alpha) <= 3, "{compression:?}: {p:?}");
        }
        // Extract the smallest mip as a valid standalone 1x1 TEX. This tests padding/cropping.
        let mut tail = bytes[..12].to_vec();
        tail[4..8].copy_from_slice(&[1, 0, 1, 0]);
        tail[11] = 0;
        let n = match compression {
            TexCompression::Bc1 => 8,
            TexCompression::Bgra8 => 4,
            _ => 16,
        };
        tail.extend_from_slice(&bytes[12..12 + n]);
        assert_eq!(decode(&tail).unwrap().dimensions(), (1, 1));
    }
}

#[test]
fn invalid_headers_payloads_and_allocation_limits_are_rejected() {
    let bytes = encode(&pattern(8, 4), Format::RiotTex, &EncodeOptions::default()).unwrap();
    for n in 0..bytes.len() {
        assert!(decode_as(Format::RiotTex, &bytes[..n]).is_err(), "{n}");
    }
    for (offset, value) in [(0, 0), (9, 255), (11, 2)] {
        let mut bad = bytes.clone();
        bad[offset] = value;
        assert!(decode_as(Format::RiotTex, &bad).is_err());
    }
    let mut bad = bytes.clone();
    bad.extend_from_slice(&[0]);
    assert!(decode(&bad).is_err());
    let mut bad = bytes.clone();
    bad[4..8].fill(255);
    assert!(decode(&bad).is_err());
    let opts = DecodeOptions { limits: Limits { max_pixels: 16, ..Default::default() }, ..Default::default() };
    assert!(decode_with(&bytes, &opts).is_err());
    let mut bad = bytes;
    bad[4..6].fill(0);
    assert!(decode(&bad).is_err());
}

#[test]
fn odd_bgra_sizes_and_high_depth_work_but_lossy_alpha_is_never_silent() {
    let img = pattern(7, 3);
    assert_eq!(decode(&encode(&img, Format::RiotTex, &EncodeOptions::default()).unwrap()).unwrap().data(), img.data());
    for compression in [TexCompression::Bc1, TexCompression::Bc3, TexCompression::Bc5, TexCompression::Bc7] {
        assert!(encode(&img, Format::RiotTex, &EncodeOptions { tex_compression: compression, ..Default::default() }).is_err());
    }
    assert!(encode(&pattern(4, 4), Format::RiotTex, &EncodeOptions { tex_compression: TexCompression::Bc1, ..Default::default() }).is_err());
    for depth in [SampleType::U16, SampleType::F32] {
        let high = pattern(4, 4).convert(ChannelLayout::Rgba, depth);
        assert!(fidelity_warnings(&high, Format::RiotTex).iter().any(|w| matches!(w, FidelityWarning::DepthReduced { .. })));
        assert!(encode(&high, Format::RiotTex, &EncodeOptions::default()).is_ok());
    }
}

proptest::proptest! {
    #[test]
    fn arbitrary_compressed_blocks_never_panic(payload in proptest::collection::vec(proptest::num::u8::ANY,16), format in proptest::sample::select(vec![10u8,12,13,14])) {
        let mut data=vec![84,69,88,0,4,0,4,0,1,format,0,0];
        data.extend_from_slice(&payload[..if format==10 {8} else {16}]);
        let _=decode(&data);
    }
}
