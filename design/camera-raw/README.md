# Camera Raw live design

The visual target is the owner's supplied **ACR 18.6.0.2698** screenshots.
This is an in-progress native redesign, not a claim of full ACR parity.

Open the live native preview from the repository root:

```powershell
./scripts/preview_camera_raw.ps1
```

The script reuses an existing debug executable and opens a synthetic color
fixture. Normal launches never compile. If the isolated target/acr18 build is
absent, it reuses target/debug/photocraft.exe. `-Image <path>` opens a specific
image instead; only explicit `-Build` compiles native behavior. A missing
executable reports that one build is needed. It uses a separate preview profile.
It does not replace the editor discovered by Image Triage.

While the preview is open, save either of these files:

- `layout.json`: widths, header/navigation/footer geometry, margins, fonts,
  numeric-field width, section padding, wheel size and panel order.
- `tokens.json`: colors, borders, contrast, highlights and corner radii using
  PhotoCraft's existing `PHOTOCRAFT_THEME_FILE` support.

**No rebuild or restart is needed for these visual edits.** Native debug builds
poll the files every 400 ms. Invalid layout JSON or unsupported values retain
the last good layout and log the error. The layout file is capped at 128 KiB.

`layout.json` also contains an optional `preview` view preset. Change its
`panels.open` list to inspect sections, `grading` to `threeWay`, `shadows`,
`midtones`, `highlights` or `global`, and `mixer` to `color` or `hsl`.
`curveChannel` is `parametric`, `master`, `red`, `green` or `blue`.
`vignetteDetails`, `grainDetails`, `sharpeningDetails` and `noiseDetails` expand
secondary controls. `grid`, `gridSize` and `gridOpacity` control the preview grid.
These are view settings; they do not change photo adjustments. A preview preset
is applied only when that part of the file changes, so ordinary clicks keep
working while colors or spacing are being tuned.

The source-embedded defaults exclude the preview preset. Release and wasm builds
use the validated embedded layout; the external file loader is a native debug
development feature. Ordinary launches without the environment variable never
poll a layout file. Functional tools and image processing remain in Rust and
require rebuilding when their implementation changes.

The host's `ui.viewport` reports the same geometry used by the live layout, so
the loading cover and embedded image area stay aligned when its dimensions change.
`ui.inspect.cameraRawDesign` reports the active visual values. Original screenshot
files and their personal metadata are not bundled with the product.

Slider density is live: `sliderFieldHeight` (18), `sliderTrackHeight` (10),
`sliderInnerGap` (0), `sliderBottomGap` (0) and `rowGap` (0).
`triangleHandles: true` uses upward-pointing handles. Other editor sliders keep
their original metrics and handle shape.

Overlay comparisons use `scripts/compare_camera_raw.py` (Pillow). Supply a
reference screenshot and a fresh `ui.screenshot`, explicit matching panel crop
rectangles and an output directory outside Git. The script keeps one uniform
scale per image, aligns the crop anchors, and writes a 50% overlay, absolute
difference, side-by-side contact sheet and alignment metadata. It never stretches
the vertical axis independently. Compare matching panel states; ignore differing
photo values when interpreting image differences. Source captures stay local.

```powershell
python scripts/compare_camera_raw.py --source reference.png --candidate preview.png `
  --source-rect 38 594 578 1092 --candidate-rect 1134 211 1404 460 --width 540 `
  --output C:/Temp/acr-comparison
```

Those rectangles describe the supplied 13289 capture and the 1440x900 synthetic
preview; remeasure crop anchors for other screenshots. `sectionTextInset`,
`sectionIconInset`, `sliderLabelRowHeight` and `sliderTrackInset` expose alignment
corrections found by this comparison in the live layout JSON.

`sectionVerticalPadding` sets the body top margin; `sectionBottomPadding`
sets its bottom margin independently, so row pitch and section separation can
be matched without moving every control.

Numeric fields use independent live settings: `valueFieldWidth`,
`sliderFieldHeight`, `valueFieldOffsetY`, `valueFontSize`, `valueMonospace`,
`valueTextLeftAligned`, `valueTextPadding` and `valueTextOffsetY`.
`sectionRightPadding` controls the shared right edge of fields and tracks,
independently of the label's left inset. These apply only to Camera Raw;
ordinary editor fields retain their existing typography and centering.

## Full-dock overlay registration (2026-10-15)

Use the complete settings dock as the horizontal anchor, including ACR's
reserved scrollbar gutter. Scaling only the inner controls (512 px in the
expanded reference) to our full 270-point dock enlarges the reference by about
5% and conceals spacing differences. The supplied expanded capture's full dock
is 540 px wide, so its uniform scale to 270 points is exactly 0.5.

The comparison script now writes `index.html`: ACR stays underneath, PhotoCraft
is the adjustable-opacity layer above it. It includes source-only, candidate-only,
50% overlay, blink, uniform zoom, absolute difference and crop/scale metadata.
Before/after captures must retain their actual heights; purple marks absent
coverage rather than stretching an image. Different image data and missing UI
must remain visible.

The laptop live pass adjusted section height/insets, row pitch, right-side gutter
allowance, numeric-box size and numeric typography. At normalized 540 px width,
all six Light track vertical bands now equal the reference: 126–131, 188–193,
250–255, 312–317, 374–379 and 436–441. Before, their offsets grew from 8 to 26 px.
This is a track-placement measurement, not a whole-interface parity claim.
Numeric inactive text is still centered despite valueTextLeftAligned; handle
geometry, bypass eyes, scrollbar treatment and other panel-specific geometry
still need native changes. A global right inset cannot establish Curve/Grading
parity; inspect their own overlays before changing them.
