# PhotoCraft camera colour response calibrations

`canon.csv` is original numerical data, independently fitted on 2026-10-14 from
controlled RGB input/output measurements of the released DNGLab 0.8.0 Windows
renderer. It contains no copied camera tables, proprietary DCP/ICC assets or
decoder implementation. Its MIT licence is beside the data.

These are single-illuminant, neutral-preserving **reference-renderer response
fits**, not measurements of sensor spectral sensitivity or a claim of Canon DPP
or Adobe picture-style parity. The output document remains 16-bit ProPhoto RGB.
Unknown models retain the explicit uncalibrated-camera warning.

The reference executable came from the release ZIP with SHA-256
`d3add6b71568bf5f7938468b0b40a0ee1eaec01e0057fe78dcc453188010be9a`.
It is an offline test tool, never a PhotoCraft runtime dependency.

## Measurement

1. Preserve a camera container's metadata, IAD1 geometry and primary track pointer.
   Append an independently encoded lossless Bayer sample, set CMP1 wavelet levels
   to zero, and update that track's stsz/offset/header-size fields. The original
   photograph is never changed or shipped. Validate the synthetic sensor with
   PhotoCraft's decoder before asking the reference renderer to develop it.
2. Fill an 16-column by 8-row grid across the entire sensor. Patch `i = row*16 +
   column` has balanced-camera channel `c` equal to `0.025 + ((i/5^c)%5)*0.03`;
   patch 127 is `[0.12,0.12,0.12]`. Encode site values as rounded
   `black + (white-black)*balanced/camera_white_balance` using the file's levels.
3. Observe `dnglab analyze --srgb PROBE.CR3`. Decode its 16-bit TIFF, whose declared
   output is sRGB, and convert to linear sRGB through PhotoCraft's ICC transform.
   Average a 16x16 region at each patch centre, correctly offset by IAD1 crop.
   Reject regions outside the output crop or with clipped reference channels.
4. Fit `rendered ~ M*balanced_camera` in chromaticity space. Constrain each row
   sum of `M` to one, leaving six unknowns. Solve the three cross-product equations
   per patch by pivoted Gaussian elimination and four robust reweighting passes.
   Reserve every fifth independent patch entirely for validation. No response
   matrix or other decoder's implementation is inspected to build the fit.
5. Map `M` into XYZ D50 with PhotoCraft's generated linear-sRGB ICC matrix. Store
   this as the forward matrix, plus the probe's white balance to construct a
   consistent reference-camera matrix. Check finiteness, invertibility and white
   preservation in unit tests. Default and tuned development use the same profile.

| Camera | Probe metadata source | Unclipped patches | Held-out chromaticity RMS |
|---|---|---:|---:|
| EOS R | local acceptance file 0G4A5180.CR3 | 74 | 0.00002429 |
| EOS R6 | local acceptance file 8M9A1601.CR3 | 73 | 0.00001381 |
| EOS M6 Mark II | local acceptance file IMG_5463.CR3 | 84 | 0.00001395 |
| EOS M50 | CC0 raw.pixls.us 4657 | 60 | 0.00001069 |
| EOS R5 | CC0 raw.pixls.us 4694 | 81 | 0.00001605 |
| EOS R5 Mark II (`Canon EOS R5m2`) | CC0 raw.pixls.us 7881 | 72 | 0.00001276 |
| EOS R6 Mark III | CC0 raw.pixls.us 8961 | 77 | 0.00001255 |
| EOS RP | CC0 raw.pixls.us 4671 | 80 | 0.00001172 |

The measurements validate numerical agreement with that renderer's linear colour
response. They do not measure absolute colour-chart accuracy, rendering tone,
noise reduction, gamut mapping or accuracy under every capture illuminant.
PhotoCraft uses its own existing demosaicing and RAW development pipeline.
