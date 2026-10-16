# photocraft-jxl

The optional JPEG XL decoder and encoder of PhotoCraft. A thin, panic-guarded wrapper around
[`jxl-oxide`](https://github.com/tirr-c/jxl-oxide), a pure-Rust decoder of the whole JPEG XL
specification (ISO/IEC 18181, MIT OR Apache-2.0): Modular and VarDCT images, lossless and lossy,
bare codestreams and the container, 1 to 32-bit integer and 16/32-bit float samples, grayscale
and RGB with alpha, the orientation field, ICC profiles and enum colour encodings, animation
(every frame through `decode_frames`), spot colours (rendered in), EXIF and XMP boxes, and
losslessly recompressed JPEGs, whose original JPEG `reconstruct_jpeg` rebuilds byte for byte.

The encoder wraps [`jixel`](https://github.com/awxkee/jixel) (pure Rust, BSD-3-Clause OR
Apache-2.0, a port of libjxl-tiny): lossy VarDCT and lossless Modular, 8/16-bit integer and 32-bit
float samples, gray and RGB with alpha, an ICC profile, EXIF and XMP in the container, effort 1–9
and progressive files. `recompress_jpeg` turns a JPEG into a JPEG XL file about 20% smaller from
which the JPEG can be rebuilt. It writes still images only (no animation).

```rust
let info = photocraft_jxl::probe(&bytes, 8 << 30)?;      // size, layout, depth: check limits first
let img = photocraft_jxl::decode(&bytes, &photocraft_jxl::Options { alloc_limit: 8 << 30 })?;
// img.width, img.height (upright), img.layout, img.sample, img.data, img.icc, img.exif, img.xmp, img.frames
let jxl = photocraft_jxl::encode(&photocraft_jxl::Input { width, height, layout, sample, data, icc, exif, xmp },
                                 &photocraft_jxl::Settings { lossless: false, quality: 90.0, ..Default::default() })?;
let smaller = photocraft_jxl::recompress_jpeg(&jpeg_bytes)?;     // and back: reconstruct_jpeg(&smaller, &opts)?
```

Errors are `Error::Unsupported` (CMYK images), `Error::Limit` (the decoder's own allocations
passed `alloc_limit`) or `Error::Malformed`. It never panics: every call into jxl-oxide or jixel
runs under `catch_unwind`, and a panic inside it becomes `Error::Malformed`. jixel 0.4.2's slower
lossy search panics on a few small sizes (12×9 and 15×9 at effort 5 and up); `encode` then tries
once more at the faster search.

## Why it is a separate, optional crate

Like `photocraft-heif`: `photocraft-codecs` uses this crate only behind its non-default `jxl`
feature, so distributors choose at build time whether a decoder for a still-young format goes into
their build; official PhotoCraft builds and CI enable it. Without it, `.jxl` files are still
recognised and opening one is a clear "JPEG XL support isn't included in this build" error.

The crate depends on no other PhotoCraft crate and knows nothing of `photocraft-codecs`' types;
`codecs` → `jxl` is one of the two allowed dependencies between standalone crates (`cargo xtask
layers`). Tests on real files live in `photocraft-codecs` (`tests/jxl.rs`, feature `jxl`, fixtures
in `tests/fixtures/jxl/` written by `scripts/jxl_fixtures.py` with libjxl's `cjxl`).
