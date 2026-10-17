mod common;
use photocraft_xcf::*;
#[test]
fn wide_native_samples_preserve_extrema_fractional_hdr_and_negative_zero() {
    let integers = [0u32, 1, u32::MAX - 1, u32::MAX];
    let floats = [-0.0f64, f64::from_bits(0x3ff0000000000001), -2.5, 1.0];
    for compression in 0..=2 {
        for precision in [300, 350, 375] {
            let file = decode(
                &common::image(12, precision, compression, 1, 1, &integers.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>()),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, integers.iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>());
        }
        for precision in [700, 750, 775] {
            let file =
                decode(&common::image(12, precision, compression, 1, 1, &floats.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>()), &Default::default())
                    .unwrap();
            assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, floats.iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>());
        }
    }
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            decode(&common::image(12, 750, 2, 1, 1, &[invalid, 0.0, 0.0, 1.0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>()), &Default::default())
                .is_err()
        );
    }
}
#[test]
fn gray_indexed_duplicate_palette_and_offsets() {
    let options = common::FixtureOptions { model: 1, ..Default::default() };
    let bytes = common::image_custom(12, 250, 1, 1, 1, &[0x12, 0x34, 0xff, 0xff], &options);
    let f = decode(&bytes, &Default::default()).unwrap();
    assert_eq!(f.model, Model::Gray);
    assert_eq!(f.layers[0].plane.as_ref().unwrap().channels, 2);
    let mut palette = 3u32.to_be_bytes().to_vec();
    palette.extend([255, 0, 0, 255, 0, 0, 0, 255, 0]);
    let options = common::FixtureOptions {
        model: 2,
        image_properties: vec![(1, palette)],
        layer_properties: vec![(15, [-3i32, 7].iter().flat_map(|n| n.to_be_bytes()).collect())],
        ..Default::default()
    };
    let f = decode(&common::image_custom(12, 150, 2, 3, 1, &[0, 255, 1, 127, 2, 0], &options), &Default::default()).unwrap();
    assert_eq!(f.palette, [[255, 0, 0], [255, 0, 0], [0, 255, 0]]);
    assert_eq!(f.layers[0].offset, (-3, 7));
    assert_eq!(f.layers[0].plane.as_ref().unwrap().bytes, [0, 255, 1, 127, 2, 0]);
}
#[test]
fn raw_rle_zlib_partial_tiles_and_versions() {
    let pixels: Vec<u8> = (0..65 * 67 * 4).map(|n| (n % 251) as u8).collect();
    for v in [0, 1, 2, 3, 7, 10, 11, 12, 18, 20, 22, 23, 26] {
        for compression in 0..=2 {
            let bytes = common::image(v, 150, compression, 65, 67, &pixels);
            let file = decode(&bytes, &DecodeOptions::default()).unwrap();
            assert_eq!(file.version, v);
            assert_eq!(file.layers.len(), 1);
            assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, pixels);
        }
    }
}
#[test]
fn exact_big_endian_integer_float_and_half_samples() {
    for (precision, src, dst) in [
        (250, vec![0, 1, 0x12, 0x34, 0xff, 0xfe, 0xff, 0xff], [1u16, 0x1234, 0xfffe, 0xffff].iter().flat_map(|n| n.to_ne_bytes()).collect::<Vec<_>>()),
        (650, [0.2f32, 0.7, 10.0, 1.0].iter().flat_map(|n| n.to_be_bytes()).collect(), [0.2f32, 0.7, 10.0, 1.0].iter().flat_map(|n| n.to_ne_bytes()).collect()),
        (
            550,
            [0x3555u16, 0x3c00, 0xc000, 0x0001].iter().flat_map(|n| n.to_be_bytes()).collect(),
            [0x3555u16, 0x3c00, 0xc000, 0x0001].iter().flat_map(|n| half::f16::from_bits(*n).to_f32().to_ne_bytes()).collect(),
        ),
    ] {
        for compression in 0..=2 {
            let file = decode(&common::image(12, precision, compression, 1, 1, &src), &DecodeOptions::default()).unwrap();
            assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, dst);
        }
    }
}

#[test]
fn all_eighteen_precision_tags_keep_encoding_and_exact_native_samples() {
    // Literal tag/sample/encoding expectations are independent of the reader dispatch.
    // No Source admission, colour conversion, or rendering is exercised here.
    let cases = [
        (100, Sample::U8, Encoding::Linear),
        (150, Sample::U8, Encoding::NonlinearProfile),
        (175, Sample::U8, Encoding::Perceptual),
        (200, Sample::U16, Encoding::Linear),
        (250, Sample::U16, Encoding::NonlinearProfile),
        (275, Sample::U16, Encoding::Perceptual),
        (300, Sample::U32, Encoding::Linear),
        (350, Sample::U32, Encoding::NonlinearProfile),
        (375, Sample::U32, Encoding::Perceptual),
        (500, Sample::F16, Encoding::Linear),
        (550, Sample::F16, Encoding::NonlinearProfile),
        (575, Sample::F16, Encoding::Perceptual),
        (600, Sample::F32, Encoding::Linear),
        (650, Sample::F32, Encoding::NonlinearProfile),
        (675, Sample::F32, Encoding::Perceptual),
        (700, Sample::F64, Encoding::Linear),
        (750, Sample::F64, Encoding::NonlinearProfile),
        (775, Sample::F64, Encoding::Perceptual),
    ];
    for (tag, source_sample, encoding) in cases {
        let (stored, expected): (Vec<u8>, Vec<u8>) = match source_sample {
            Sample::U8 => (vec![0, 1, 254, 255], vec![0, 1, 254, 255]),
            Sample::U16 => {
                let values = [0u16, 1, u16::MAX - 1, u16::MAX];
                (values.iter().flat_map(|v| v.to_be_bytes()).collect(), values.iter().flat_map(|v| v.to_ne_bytes()).collect())
            }
            Sample::U32 => {
                let values = [0u32, 1, u32::MAX - 1, u32::MAX];
                (values.iter().flat_map(|v| v.to_be_bytes()).collect(), values.iter().flat_map(|v| v.to_ne_bytes()).collect())
            }
            Sample::F16 => (
                [0x8000u16, 0x0001, 0x3800, 0x3c00].iter().flat_map(|v| v.to_be_bytes()).collect(),
                // IEEE binary16 -0, minimum subnormal 2^-24, 1/2, 1 widened to binary32.
                [0x80000000u32, 0x33800000, 0x3f000000, 0x3f800000].iter().flat_map(|v| v.to_ne_bytes()).collect(),
            ),
            Sample::F32 => {
                let bits = [0x80000000u32, 0x3f000001, 0xc0200000, 0x3f800000];
                (bits.iter().flat_map(|v| v.to_be_bytes()).collect(), bits.iter().flat_map(|v| v.to_ne_bytes()).collect())
            }
            Sample::F64 => {
                let bits = [0x8000000000000000u64, 0x3ff0000000000001, 0xc004000000000000, 0x3ff0000000000000];
                (bits.iter().flat_map(|v| v.to_be_bytes()).collect(), bits.iter().flat_map(|v| v.to_ne_bytes()).collect())
            }
        };
        for compression in 0..=2 {
            let encoded = common::image(12, tag, compression, 1, 1, &stored);
            let file = decode(&encoded, &Default::default()).unwrap();
            assert_eq!((file.version, file.precision, file.sample, file.encoding), (12, tag, source_sample, encoding), "tag {tag}, compression {compression}");
            assert_eq!(file.layers.len(), 1);
            let plane = file.layers[0].plane.as_ref().unwrap();
            assert_eq!((plane.width, plane.height, plane.channels), (1, 1, 4));
            assert_eq!(plane.sample, if source_sample == Sample::F16 { Sample::F32 } else { source_sample });
            assert_eq!(plane.bytes, expected, "tag {tag}, compression {compression}");
        }
    }
}
