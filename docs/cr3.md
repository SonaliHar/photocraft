# CR3 support and measured coverage

PhotoCraft develops lossless, legacy C-RAW and adaptive C-RAW CR3 sensor data
to 16-bit ProPhoto RGB. Unsupported coding variants open a camera-rendered JPEG
or, with the `heif` feature, HEVC preview.
Import warnings distinguish development from preview fallback. Unknown CRX coding
variants are explicitly unsupported; malformed supported streams return errors.

## Lossless RAW

The clean-room decoder supports CRX version `0x100`, four Bayer planes, even
tile geometry, up to sixteen tiles, and the `ff01`/`ff02`/`ff03` marker family.
The primary sensor comes from CCTP/CCDT, so the Dual Pixel auxiliary track is
never mistaken for the main image. IAD1 provides sensor and active dimensions
and the recommended crop. Orientation is applied once.

CTMD per-frame maker notes provide ColorData as-shot white balance, per-channel
black levels and specular white. The documented ColorData9–12 layouts are
accepted by version; version 66 retains version 64's layout in the measured
R6 Mark III file. Unknown or invalid layouts use explicit estimates and warnings.
No external camera coefficient tables are imported. Eight camera models have
independently measured, neutral-preserving RGB response calibrations: EOS R,
R6, M6 Mark II, M50, R5, R5 Mark II, R6 Mark III and RP. These are single-illuminant
fits to controlled probes of a reference renderer, rather than spectral or
ColorChecker measurements. They do not reproduce camera picture styles or claim
Canon DPP / Adobe Camera Raw parity. Unknown models retain the explicit
uncalibrated fallback. The current upstream import pipeline can also fit an
embedded JPEG look for an uncalibrated camera; that rendering step is separate
from the exact sensor reconstruction checks. See the [dataset and reproducible method](../crates/raw/data/calibration/README.md).

Current upstream Camera Raw's normal, Before and detail previews apply the document-to-display
ICC transform used by the canvas. ProPhoto document values are never shown as
display RGB directly; monitor/profile changes invalidate the preview textures.

Every full-sensor sample of these CC0 originals matches DNGLab's released binary,
before cropping, white balance or demosaicing:

| raw.pixls.us ID | Camera / mode | Sensor dimensions | FNV-1a, u16 little-endian |
|---|---|---|---|
| 4657 | EOS M50 RAW | 6288×4056 | `62261f0ba81cfcd2` |
| 4693 | EOS R5 RAW, Dual Pixel | 8352×5586 | `1f9a92d870791620` |
| 4694 | EOS R5 RAW | 8352×5586 | `b009f1986c482e28` |
| 7881 | EOS R5 Mark II RAW | 8480×5650 | `ff716fb30248496f` |
| 8961 | EOS R6 Mark III RAW | 7144×4760 | `47a4cb401233cc80` |

The original five lossless fixtures above account for 200,730,112 samples. The
expanded corpus has **48 native sensor oracles, including 22 C-RAW files**, with
**1,342,361,776 samples** independently checked without a single difference.
[The complete per-file table](../xtask/cr3-corpus.tsv) records camera, mode,
dimensions, coding version, preview type, native/fallback status and sensor hash.
This is evidence for those
files and coding variants, rather than a claim that every mode of these cameras
or every CR3 camera has been tested.

The entropy rules were measured from those files: MED prediction, 41-zero / 21-bit
Rice escapes, adaptive Rice thresholds, upper-row lookahead and MELCODE of run
length minus one. A MEL block that exactly reaches the row end advances its state;
an overshooting block does not. The lower nineteen subband flag bits specify
padding **bytes**; at most seven alignment bits remain after decoding them.
Synthetic tests cover these boundaries, truncation, corrupt entropy and size limits.
Native reconstruction decodes at most two independent planes concurrently;
WebAssembly uses one. Working-memory checks include buffer growth and row scratch.

The oracle was the Windows x64 DNGLab 0.8.0 release ZIP, SHA-256
`d3add6b71568bf5f7938468b0b40a0ee1eaec01e0057fe78dcc453188010be9a`.
`dnglab analyze --raw-pixel FILE.CR3` emitted a P5 PGM with big-endian 16-bit
samples. The binary is used only for offline observation, never shipped or linked.
No LibRaw, dcraw, rawler, rawspeed or other RAW decoder implementation was read.

## Legacy C-RAW

CRX `0x100` three-level wavelet Bayer streams with the legacy marker family,
one or two horizontal tiles and measured uniform quantizers 4, 16, 22, 26 and 32
develop natively. The LL band uses MED/Rice/MEL coding; high-frequency bands
use direct signed coefficients, zero-run contexts, a nonzero-symbol offset and
the upper-right **next** Rice parameter. Run coefficients clear that parameter.
Integer Le Gall 5/3 synthesis runs horizontally then vertically, using symmetric
extension and parity-dependent overlap dimensions at each level. Sensor midpoint
and clipping apply only after the final reconstruction.

Public exact C-RAW oracles cover EOS M50, 250D, 90D, M6 Mark II, RP (full sensor
and crop), M200, R100, PowerShot SX70 HS, G5 X Mark II and G7 X Mark III.
An additional private M6 Mark II file matched all 33,729,696 sensor samples; it
is not redistributed or required by CI. Synthetic tests cover lifting phases,
negative-coefficient rounding, entropy truncation, size/working-memory limits,
corrupt headers, native import/tuning and 16-bit persistence.

Unverified legacy quantizers, vertical/middle-tile overlap, rounded Bayer geometry
and other wavelet counts remain unsupported with explicit preview fallback.
Native reconstruction is bounded to
two concurrent planes (one on wasm); allocation checks include all bands,
inverse-transform buffers and temporary capacity growth.

## Adaptive C-RAW

CRX `0x200` three-level, four-plane Bayer streams with a single even-sized tile
and `ff11` / `ff12` / `ff13` headers develop natively. The tile header gives the
size and byte padding of a shared quantization grid; extended band headers give
base/multiplier quantization values and independent padding. Every parent length,
band counter, padding bound and entropy extent is validated.

The grid is `ceil(planeWidth / 8)` by `ceil(planeHeight / 2)`, encoded with MED,
adaptive Rice and a **23-zero / eight-bit escape**, without MEL flags. Higher
wavelet levels average groups of grid rows **before** quantizer conversion,
rounding down and replicating the final row at an odd bottom edge. The scale
has six phases and doubles every six indices; its integer truncation occurs
before the band multiplier is applied. Coefficient entropy and integer 5/3
synthesis reuse the legacy path. Accepted QP indices are bounded to 0–51;
the ten real files independently exercise indices 22–40, mixed quantization,
odd dimensions and multiple scale phases.

All **296,028,288** samples from the ten adaptive originals match the offline
reference: EOS-1D X Mark III, R6, R5 (full and APS-C crop), R7, R8, R50,
R5 Mark II, PowerShot V1 and R6 Mark III. The R5 crop advertises four masked
readout rows beyond its stored raster. Only v0x200's bounded trailing active-area
padding (at most seven rows) is clipped; photo crops and horizontal bounds stay
strict. Camera colour calibration remains model-dependent as described above.

Synthetic adaptive tests use alternating QP blocks and a nonzero high-frequency
band, checking independently calculated boundary pixels, negative rounding,
tuning, orientation, 16-bit persistence and corrupt/truncated extended headers.
The original camera JPEG/HEVC fallback tests force an unsupported sensor version
in memory, so those paths remain covered while all 48 originals develop natively.
Adaptive sensor development does not require HEVC support, including files whose
camera-rendered preview happens to use HEVC. Multi-tile adaptive coding, rounded
planes, alternate encoding families and burst selection remain unsupported.

## Previews and metadata

Single-frame `stsz`/`stsc`/`co64` or `stco` tables and PRVW locate previews;
compressed bytes are never searched for JPEG signatures. Burst sample tables
need a separate picture-selection implementation and are refused.

Both R5 Mark II C-RAW and RAW files in the corpus have HEVC previews rather than
JPEGs. Canon's length-prefixed grid cells are reframed as standard HEIF items for
PhotoCraft's optional pure-Rust decoder. Multiple intra slices per cell, the grid
crop, ten-bit depth and nclx colour declaration survive. Rec.2020 / ST 2084 PQ
previews receive a matching ICC profile and remain 16-bit documents. Unsupported
transfer functions produce errors rather than receiving an assumed sRGB tag.
Builds without `heif` explain the missing decoder when a preview requires it;
lossless sensor decoding does not depend on that feature.

The HEIF dependency is pinned to the immutable heic-rs 0.1.4 commit containing
the Table 8-3 correction in [upstream PR 10](https://github.com/tbraun96/heic-rs/pull/10).
Released versions before that correction cannot decode the measured Canon 4:2:2
previews. Re-run the HEIF and CR3 corpora before changing the pin.

CMT1–4 and per-frame CTMD TIFF entries are rebuilt with relocated value and IFD
offsets. Camera information, exposure, ISO, maker notes (including ColorData), GPS
and unknown scalar tags survive. XMP is retained. Metadata limits and recursive
IFD cycle checks guard damaged files. JPEG previews without an ICC receive sRGB
or Adobe RGB only when the file declares it; uncalibrated EXIF does not imply Adobe RGB.
Developed documents keep metadata while resetting orientation to upright.

## Reproducing the checks

```sh
cargo run -j 12 -p xtask -- corpus --cr3
cargo test -j 12 -p photocraft-raw --features corpus
cargo test -j 12 -p photocraft-io --features corpus,heif --test cr3_corpus
cargo test -j 12 -p photocraft-io --features corpus --test cr3_corpus
cargo run -j 12 -p photocraft-raw --example cr3info -- corpus/cr3/7882.CR3
```

`xtask/cr3.sha256` pins **48 unmodified originals across 25 camera models**,
totalling **1,267,264,089 bytes (1,208.6 MiB)**.
Tests fail when fixtures are absent. `cargo run -j 12 -p xtask -- corpus --all` fetches them and
`cargo run -j 12 -p xtask -- test-corpus` runs the CR3 sensor-reference tests through the I/O
corpus gate. The existing multi-format pixls RAW corpus remains opt-in.
It includes DSLR, mirrorless and PowerShot cameras, full/cropped sensors,
aspect-ratio metadata, Dual Pixel, RAW/C-RAW and JPEG/HEVC PQ previews. Public
input hashes are taken from the raw.pixls.us catalogue and verified after download.
Photos and oracle pixels stay gitignored and are never committed. Tests use
recorded native hashes that were independently verified against the release
binary, not self-generated round-trip expectations.

Remaining native work: additional legacy uniform quantizers and tile layouts,
adaptive multi-tile/rounded-plane geometry, additional bit depths,
burst / small-RAW modes and broader camera colour/illuminant coverage.

On a local Windows x64 Ryzen 7 9700X release build, three-run medians were
247.8 ms to reconstruct the 25.5 MP M50 sensor and 438.9 ms for the 46.7 MP R5
sensor, with zero oracle differences. Full default AHD development/import of
the 24 MP M50 image took 622.2 ms. File reads and reference comparison are
outside those timings. Native CR3 development was unsupported before this change;
these are initial measurements, not a speedup claim or a published performance budget.

After enabling C-RAW, full default AHD development/import of a 32.3 MP M6
Mark II image took **931.2 ms** (three-run median, release with LTO disabled for
the corpus build). Native C-RAW development was previously unsupported; the
earlier JPEG fallback was a different, lower-precision operation. File reads are
excluded and no preview-to-native speedup is claimed.

For adaptive C-RAW, three-run release AHD import medians were **1,168.0 ms**
for the 32.3 MP EOS R7 file and **1,147.6 ms** for the 32.3 MP R6 Mark III file
(LTO disabled, file reads excluded; concurrent corpus validation was running).
These coding variants previously used camera previews rather than native sensor
development. The quick performance suite completed; cross-machine regression
comparison was skipped because the published baseline uses a different CPU/OS.

## Public sources

- [Laurent Clévy's CR3 format description](https://github.com/lclevy/canon_cr3/blob/master/readme.md): BMFF, CMP1, IAD1, CTMD and marker layouts (prose only).
- [Canon US20160323602A1](https://patents.google.com/patent/US20160323602A1/en): prediction, wavelets and adaptive Rice coding.
- [Canon JP2017192077A](https://patents.google.com/patent/JP2017192077A/en): zero-run detection and MELCODE.
- [ITU-T T.87](https://www.itu.int/rec/T-REC-T.87): MELCODE table.
- [ITU-T T.800](https://www.itu.int/rec/T-REC-T.800): reversible Le Gall 5/3 lifting and symmetric extension.
- [ExifTool Canon tag documentation](https://exiftool.org/TagNames/Canon.html): ColorData9–12 metadata layouts.
- [raw.pixls.us](https://raw.pixls.us/): CC0 inputs, pinned by SHA-256. DPReview galleries were considered; no restricted gallery photos are committed or redistributed.
