# Camera Raw live design

The visual target is the owner's supplied **ACR 18.6.0.2698** screenshots.
This is an in-progress native redesign, not a claim of full ACR parity.

Open the live native preview from the repository root:

```powershell
./scripts/preview_camera_raw.ps1
```

The script builds an isolated debug executable once if missing, then opens a
synthetic color fixture. `-Image <path>` opens a specific image instead; `-Build`
rebuilds after changes to native behavior. It uses a separate preview profile.
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
  --source-rect 38 594 550 1514 --candidate-rect 1134 211 1404 696 `
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
