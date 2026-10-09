# photocraft-xcf

Standalone safe Rust reader for plain GIMP XCF. Filesystem and UI integration belong to
`photocraft-io`. The reader never writes XCF or executes linked resources or GEGL effects.

The public API is `is_xcf(bytes)` and `decode(bytes, &DecodeOptions)`. Decoder failures include
an error category and byte offset. Tight caller budgets and cancellation are supported.

Implementation follows the public [GIMP XCF specification](https://developer.gimp.org/core/standards/xcf/).
Version recognition does not imply support for every feature. Unsupported rendering semantics
are represented explicitly for the document adapter to reject.

Verified import covers RGB, grayscale and indexed images; U8/U16 and finite F16/F32;
linear, profile nonlinear and perceptual samples; uncompressed, RLE and zlib tiles;
normal/source-over layers and isolated/pass-through groups, masks, offsets, opacity,
visibility, supported locks, selections and guides. F16 widens exactly to F32. Indexed
source indices, separate alpha and duplicate palette entries survive native migration.
The io corpus tests use original GIMP 2.8.22, 2.10.32 and 3.2.6 projects and independent
GIMP sample buffers; display comparisons belong to the compositor oracle tests.
The reviewed XCF corpus is currently local; publication and its immutable public pin
are pending. Local validation requires the sibling `photocraft-corpus` authoring clone
(or `PHOTOCRAFT_CORPUS_REPO` pointing to it).

Versions 0–3 and 7–26 are structurally recognized (synthetic tests cover version/pointer
changes). Versions 4–6 and pre-12 high-depth development files have ambiguous sample
representations and are rejected. U32/F64, unequal X/Y DPI, nonfinite samples, unknown
rendering properties, non-normal blend/composite operators, arbitrary GEGL effects,
text/vector/link layers and floating selections are explicit unsupported errors, including
hidden unsupported layers. No special-layer stored-raster fallback is currently verified.
Gzip/bzip2/xz wrappers and XCF writes are outside the importer.

Limits remain enforced even when callers request higher ceilings:
262144 pixels per side, 2^30 cumulative drawable pixels, 10000 drawables, 100000 properties,
1 MiB strings, 16 MiB ICC, 64 MiB individual properties, 256 MiB aggregate metadata,
100 group levels and 20000000 record/work visits. Encoded input is capped at 1 GiB native
or 256 MiB wasm; resident accounting at 2 GiB native or 512 MiB wasm includes borrowed
input, staging, padded output tiles, metadata and indexed provenance. Lower caller
budgets and cancellation apply throughout parsing and mapping. Decoding is serial on
both targets; parallel decoding requires a measured reason before introducing more scratch.

Run reader tests and strict checks with:

```sh
cargo test -p photocraft-xcf
cargo clippy -p photocraft-xcf --all-targets -- -D warnings
cargo xtask corpus --xcf --local
cargo test -p photocraft-io --features corpus --test xcf_corpus
cargo xtask test-corpus --local
cargo run --release -p photocraft-io --example bench_xcf -- input.xcf
cargo +nightly fuzz run decode --fuzz-dir crates/xcf/fuzz --sanitizer address -- -runs=10000
cargo +nightly fuzz run xcf_import --fuzz-dir crates/io/fuzz --sanitizer address -- -runs=10000
```

The fuzz smoke runs use AddressSanitizer and bounded input/resident/work budgets.
On Windows, the MSVC x64 binaries directory must be on PATH for its AddressSanitizer DLL.
Fixture binaries and mutated fuzz inputs are development data and are never committed here.
