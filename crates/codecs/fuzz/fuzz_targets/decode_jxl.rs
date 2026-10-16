#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_codecs::{decode_as_with, decode_jxl_frames, jpeg_to_jxl, jxl_to_jpeg, DecodeOptions, Format, Limits};

// jxl-oxide parses untrusted codestreams and containers: run with `-timeout=10` so a hang is
// reported, not just a panic. The codec turns a jxl-oxide panic into an error, so jxl-oxide is
// also called directly, where its panics still reach the fuzzer. Both orientation paths are
// covered, and so are every-frame decoding, JPEG reconstruction and (on JPEG-looking input) the
// lossless JPEG recompression of jixel.
fuzz_target!(|data: &[u8]| {
    let keep_orientation = data.first().is_some_and(|b| b & 1 == 1);
    let _ = photocraft_jxl::decode(data, &photocraft_jxl::Options { alloc_limit: 256 << 20 });
    let limits = Limits { max_width: 4096, max_height: 4096, max_pixels: 1 << 22, max_alloc: 256 << 20 };
    let opts = DecodeOptions { limits, keep_orientation };
    let _ = decode_as_with(Format::Jxl, data, &opts);
    let _ = photocraft_jxl::decode_frames(data, &photocraft_jxl::Options { alloc_limit: 256 << 20 }, 64);
    let _ = decode_jxl_frames(data, &opts);
    let _ = jxl_to_jpeg(data, &limits);
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        let _ = photocraft_jxl::recompress_jpeg(data, 256 << 20);
        let _ = jpeg_to_jxl(data, &limits);
    }
});
