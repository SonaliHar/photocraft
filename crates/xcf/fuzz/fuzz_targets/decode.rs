#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.input_bytes = 1 << 20;
    options.limits.resident_bytes = 8 << 20;
    options.limits.pixels = 65536;
    options.limits.drawables = 64;
    options.limits.properties = 1024;
    options.limits.visits = 10000;
    let _ = photocraft_xcf::decode(bytes, &options);
});
