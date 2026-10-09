#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let mut limits = photocraft_xcf::Limits::default();
    limits.input_bytes = 1 << 20;
    limits.resident_bytes = 8 << 20;
    limits.pixels = 65536;
    limits.drawables = 64;
    limits.properties = 1024;
    limits.visits = 10000;
    let _ = photocraft_io::xcf_import::import_with_limits("fuzz.xcf", bytes, &photocraft_raster::Interrupt::NONE, limits);
});
