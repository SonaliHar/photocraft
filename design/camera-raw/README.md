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

## Native bars: rebuilt, pending visual review (2026-10-16)

The next native batch restores the Curve glyph selectors, White balance
combobox and neutral picker, Profile row with three-square/magnifier glyph,
Edit row, persistent metadata row, filled corner clipping indicators, and
top action icons. The Profile row currently identifies the embedded treatment;
additional camera profiles, Auto tone, HDR editing and tutorials remain disabled.
B&W uses the existing saturation adjustment. The neutral picker solves the
existing relative Temperature/Tint gain model in the document encoding; it does
not introduce a Kelvin model or claim Adobe's processing.

After the enabling build, these additional `layout.json` properties can be
tuned live: `histogramHeight` (118), `histogramCornerSize` (16),
`metadataHeight` (26), `editBarHeight` (40), `profileBarHeight` (40),
`barInset` (22), `titleActionSize` (24), `titleActionGap` (6),
`curveSelectorSize` (20), `curveSelectorGap` (6), and
`whiteBalanceLabelWidth` (78). They remain optional in the active JSON; the rebuilt preview accepts them.
The previous preview executable rejects these new keys.
The view-only `whiteBalanceTool` field is then available through the workspace
control JSON as well. Activating the picker and the sampler is mutually exclusive.

Rustfmt and whitespace validation have run. Unit tests were added for metadata
fallbacks, invalid sample rejection, neutral-gain solving and new JSON bounds.
The owner requested the enabling rebuild: cargo build -p photocraft -j 4
passed in 3m08s, and the PS1 launched the new preview. Test execution and
post-build screenshot validation remain pending. Both the section-edge
correction and this bar batch are now in target/debug/photocraft.exe.

## Reference-wide parity resumed (2026-10-16)

`reference-inventory.json` tracks all 19 supplied captures by hash, dimensions,
UI state and current audit status. Every reference remains in scope, including
Remove, Masking, Red Eye, Snapshots, Presets, Info, Crop, menus, dialogs and
full-window states. The seven-section JSON preset is not complete coverage.
Missing native controls cannot be supplied by changing layout JSON alone.

Current local evidence: `target/acr18/parity-20261016/`. The preview uses the
existing debug executable, an isolated profile, a synthetic fixture and control
port 57831. Baseline and after-heights screenshots and inspect replies are kept.
Reference 10 registration: full dock (38,136)-(578,2542); preview dock
(1060,32)-(1330,672), uniformly enlarged 2x to reference width 540. Missing
vertical coverage stays purple. No vertical stretching or photo-content match.

Live JSON histogramHeight 118 -> 124 and metadataHeight 26 -> 24 closed the
8-pixel downstream offset. At normalized width 540 the source separator bands
are 241-245, 296-299, 376-379, 456-459 and 954-957. The captured candidate bands
are 240-245, 296-297, 376-377, 456-457 and 954-955. These cover histogram,
metadata, Edit, Profile and Light/Color boundaries only. Border thickness,
fonts, glyphs, histogram paint and all other elements remain unverified or
visibly different. Both live values were confirmed through ui.inspect.

Next: capture each matching edit-panel state and each non-Edit tab, measure
its own registration, and audit every element against the reference. Keep
unsupported controls and native-only corrections explicit; no 1:1 claim yet.

### Grading native follow-up and repeatable captures

The owner authorized native support for missing reference screens. Existing
Curve, Mixer, Grading, Detail, collapsed and Info states have fresh native
captures in `target/acr18/parity-20261016/wheel-v3`. Before overlays exist for
references 7, 11, 12 and 14. Every observed control group in all 19 references
remains in reference-inventory.json with its individual verification pending.

Grading now has five accessible glyph selectors, display-referred HSV wheel
artwork and compact pointer/keyboard-editable luminance tracks. Enabling builds
passed (3m42s for glyphs; 7m57s for the wheel follow-up). This is development
preview work, with no hosted executable promotion or upstream merge.

Native JSON controls: gradingSelectorSize, gradingSelectorGap,
gradingSelectorStart, gradingSelectorBottomGap, wheelBrightness and
gradingWheelRowGap. Current selector gap below the row is 23 points. The full
reference rail is 80 pixels, so railWidth is corrected from 36 to 40 points.
Consequently the 1366px native capture dock anchor is now x1056..1326.

Reference-12 rim samples at radius 85 of 94 reference pixels have max RGB near
128 and HSV hue approximately equal to screen angle. For a fixed top-wheel
interior ellipse (176,234)-(336,394) in the uniformly registered reference
section, mean absolute RGB differences fell from [46.06,36.53,66.71] to
[7.52,7.05,8.52]. This measures that region only. The full overlay still shows
lower-wheel spacing, label offsets, handles, outer dots, section eyes and
other differences. Exact next measurements are in grading-next-layout.json.

The first Camera Raw test run had 48 passes and 2 failures: selector-height
measurement included root spacing, and the i18n scanner missed labels moved
to the workspace module. The selector test now checks actual accessible glyph
rectangles for one row, the scanner covers the moved module, and Adjust is
translated in all 20 catalogs. Current-source tests are rerunning. All-target
clippy passed for this batch before the offscreen helper flag was added;
current layers/wasm and offscreen rendering remain pending.

`scripts/capture_camera_raw.py` captures named panel states through an existing
isolated preview. It verifies identical photo parameters and document revision
before/after each screenshot, writes capture-report.json, restores layout bytes
and preserves external edits rather than overwriting them. Its six-state run
on control port 57833 passed these checks. Token files are never copied into
capture artifacts. Use a new output directory for each run:

```powershell
python scripts/capture_camera_raw.py --port 57833 `
  --token-file target/acr18/parity-20261016/control-v3.token `
  --output target/acr18/parity-20261016/new-capture
```

The physical monitor limits live captures to 768px height. The snapshot example
now accepts --raw-workspace so taller offscreen captures use the same native
Camera Raw layout, with --size and --scale independent of the monitor. Its
build/render verification is pending; do not claim full-height coverage yet.

### Native Info/Snapshots/layout batch (2026-10-16, building)

The previous batch completed all gates:52 Camera Raw tests, all-target clippy,
30-crate layers and every xtask wasm check passed. Four existing wasm dead-code
warnings remain. Offscreen --raw-workspace was built and produced1440x1400;
12-grading-full-height-before has full vertical coverage, unlike the live capture.

The capture runner originally could accept a previous preview during reload.
It now requires every requested preview field to match. A strict six-state
recapture passed and preserved photo parameters/revision and original layout
bytes. Five images are pixel-identical to the earlier capture; Grading differs
only in the histogram hover/readout region, outside the section overlay.

Latest source adds a camera summary card, file-kind badge, capture date,
filename and Content Credentials area, removing the extra Info title bar.
Unknown metadata is shown as an em dash; no credentials-verification claim.
The Snapshots tab now captures/restores/deletes named settings within the open
dialog; it is not yet durable across closing/switching the photo. Its empty
reference-4 layout is awaiting native capture, not declared matched.

Grading has measured lower-wheel outset/label offsets/track height and dot/handle
paint corrections. Fixed settingsScrollbarGutter and settingsBodyRightPadding
prevent widths changing with scrollbar visibility, while preserving top-bar
metrics. Native sizing additions remain optional: the old live preview does not
accept them. Keep its existing JSON until the new executable is validated.

Latest all-target clippy passed after correcting one test initializer. Build,
current-source tests and overlays for this new batch are in session82463;
active-work.json records exact live handles. New offscreen capture runner
scripts/capture_camera_raw_offscreen.py writes a separate layout per state and
checks --inspect-out against requested view/photo state, without touching live
JSON. Its Python syntax passed; real seven-state run awaits the new executable.

### Current attachment numbering and v6 overlays (2026-10-16)

The active reference set is `c0f524f7-ece8-4a8b-a8f6-7f0f327cf262`.
All 19 content hashes match the earlier inventory, but the ordering changed.
`reference-inventory.json` now uses the current image numbers and preserves
`legacyImage` for earlier comparison folders. In particular Snapshots is now
1, Info 4, Light/Color/Effects 7, Curve/Mixer 8, Grading 9, and collapsed Edit
11. References 17/18 also exchanged their old grid/alternate-Info positions.

The old session82463 handle and its recorded compiler/preview processes no
longer existed on inspection. Current v6 snapshot build and validation passed:
56 Camera Raw library tests, 5 workspace integration tests, shared widget tests,
all-target clippy, 30-crate layers and all xtask wasm checks. Four pre-existing
wasm dead-code warnings remain. The offscreen executable SHA256 is
`f2832da7c1285f296ff7f4ba08c6be72a1c9ef584c9c6373da85f85fbda8ceac`.
This is an offscreen preview build, not a rebuilt/promoted desktop or hosted editor.

This iteration fixes Info/Snapshots left-aligned semibold headings, independent
Info field/divider/credentials spacing, Snapshots heading position and sheet
glyph, Grading selector grouping/separators, wheel antialiasing, lower label
placement, luminance handle/track geometry and footer spacing. The shared numeric
field now honors `valueTextLeftAligned` in its inactive presentation while retaining
its entire edit target. Its regression test measures actual painted digits, then
clicks the right side and types a value. Current egui exposes an editing DragValue
as a SpinButton, so the test verifies focus and the resulting value rather than
requiring a TextInput accessibility role.

Additional live geometry: `gradingSelectorGroupGap`, `gradingFooterGap`,
`infoFieldGap`, `infoDividerTopGap`, `infoCredentialsTopGap`,
`infoHeadingFontSize`, `tabHeadingFontSize`, `tabHeadingInset`,
`tabHeadingOffsetY`. They are optional and validated. Current grading JSON uses
selector start69, bottom gap30.5, footer gap14.5 and lower label offset-10.

Fresh evidence under `target/acr18/parity-20261016/`:

- `native-v6-offscreen`: all11 supported named states, each verified against
  requested view, identical photo parameters and document identity.
- `native-v6-full-window` and `native-v6-ref17/18/19`: Info, grid/Info and Edit
  at full-window and reference-specific dimensions.
- `ref01-snapshots-v6-element-audit`, `ref04-info-v6-element-audit`,
  `ref09-grading-v6-element-audit`: source, candidate, 50% overlay, difference,
  interactive opacity viewer and per-element diagnostics.
- `ref07-edit-v6`, `ref08-curve-mixer-v6`, `ref09-grading-detail-v6`,
  `ref11-collapsed-v6`: combined-panel coverage, including visible missing content.
- `ref16-info-v6`, `ref17-grid-info-v6-exact-size`,
  `ref18-info-v6-exact-size`, `ref19-edit-v6-exact-size`: whole-window overlays.
  OS chrome, photo content/aspect/zoom and workspace chrome differences are
  deliberately visible. These are diagnostic comparisons, not matched-frame claims.

`compare_camera_raw.py --regions <specification.json>` now measures named elements
in the same registered coordinates. Specifications cover38 Grading,21 Info and5
Snapshots regions, including glyphs, labels, handles, tracks and separators.
No control is individually shifted or warped. A missing candidate area cannot
pass even if black padding matches; wrong registration width/rectangle is rejected.
Four Python auditor regression tests pass. Example:

```powershell
python scripts/compare_camera_raw.py --source <current-reference-9.png> `
  --candidate target/acr18/parity-20261016/native-v6-offscreen/grading.png `
  --source-rect 10 536 550 1520 --candidate-rect 1130 260 1400 752 `
  --width 540 --regions design/camera-raw/grading-element-regions.json `
  --output target/acr18/parity-20261016/new-grading-audit
```

The measured regions improve but are not pixel-identical. Reference image content,
metadata and raster noise affect pixel differences; diagnostics do not establish
behavior or whole-reference parity. The full19-reference goal remains incomplete.
Every reference now records supported state and outstanding native coverage.
Next structural gaps include section bypass eyes and Optics/Lens Blur/Calibration,
then Crop, Remove, Masking, Red Eye, Presets and the matching menus/dialogs.
Shared chrome and individual Curve/Mixer/Light/Color/Effects elements also remain.

### v8: ten sections and temporary section bypass (2026-10-16)

The native Edit stack now contains all ten observed headers, including Optics,
Lens Blur and Calibration. Their layouts contain the observed tabs, unchecked
correction options, Defringe row, five Bokeh glyphs, blank disabled Lens Blur
fields, focus strip, Refinement row, process selector and primary-color rows.
These three stages do not yet have matching processing implementations: their
processing controls remain disabled and cannot write fictitious recipe values.
Their enabled/disabled appearance and remaining geometry are not declared matched.

Every section has a native eye glyph. Holding the eye of an implemented stage
temporarily previews the image with just that stage neutralized. Canonical
parameters, undo state, recipe saving and commit/export remain unchanged. Proxy
and full-resolution detail previews use the same temporary parameters. Release
restores the adjusted preview. Automation can request the same temporary view
with `workspace.bypassSection` (one of the seven implemented section names, or
null to restore). The reply reports `previewBypass`; unsupported names are rejected
atomically. Unsupported stages retain visible, disabled eyes.

New measurements:40 individual collapsed-header regions in
`collapsed-element-regions.json`, and93 lower-panel regions in
`lower-panels-element-regions.json`. The latter registers reference10 at
(34,438)-(574,2530) against native (1130,260)-(1400,1306), at540px width.
After live tuning, Lens Blur and Calibration divider bands start at382 and1252
reference pixels in both images. Edge antialiasing, glyphs, typography, disabled
appearance and individual control offsets still differ. The section eye's measured
mean RGB is about87.04, versus87.05 in the source; matching this region's brightness
does not establish glyph/interaction or full-panel parity.

Additional live JSON: `sectionEyeInset`, `sectionEyeOpacity`,
`sectionSeparatorHeight`, `extraControlGap`, `extraDisclosureHeight`,
`extraDisclosureOutset`, `opticsTabsHeight`, `opticsCheckboxGap`,
`opticsDividerGap`, `bokehSelectorSize`, `calibrationTitleHeight`,
`calibrationTitleLeading`, `calibrationSectionGap`. Current live tuning uses
extraControlGap8.25 and opticsTabsHeight28.75. All values have bounded validation.

Capture ordering was corrected: starting at a later section retains the sections
that follow it, rather than inserting Light/Color/Effects between Curve and Mixer
or between Optics and Lens Blur. The capture runner has a regression test for this.
Combined Curve/Mixer and Grading/Detail views now also open Optics as observed.

v8 passed58 Camera Raw library tests,7 workspace integration tests, all-target
clippy,30-crate layers, wasm (four existing warnings), and5 Python overlay tests.
The native snapshot executable SHA256 is
`27657465fe8e3fe49c7c4164a6aea28545180bb5eca2dbff90eb7e7b31b6dab8`.
All13 supported named views were captured and inspected in native-v8-offscreen;
every requested view matched and photo parameters/document identity were unchanged.
Fresh interactive comparisons are ref11-collapsed-v8-element-audit,
ref10-lens-calibration-v8-element-audit, ref9-grading-v8-element-audit,
ref8-curve-mixer-v8 and ref9-grading-detail-v8. These are incomplete parity evidence,
not complete reference passes. No desktop build or hosted executable promotion.

### Active 19-reference set and Red Eye panel (2026-10-17)

The goal's reference set is now `004b6ab0-f1b0-4278-a478-4890b6fff1d0`.
All 19 hashes match the preceding set; image numbering is different. The current
inventory is authoritative and keeps `previousImage` and `legacyImage`. Current
images: 1 Info, 2 Save Options, 3 Preferences, 4 Light/Color/Effects, 5 Curve/Mixer,
6 Grading/Detail, 7 Optics/Lens Blur/Calibration, 8 collapsed Edit, 9 Crop,
10 Remove, 11 Masking, 12 Red Eye, 13 Snapshots, 14 Presets, 15 Settings menu,
and 16-19 full-window views. Previous comparison directory numbers are historical.

The native workspace now accepts `preview.tab: "redEye"`. It renders the measured
header/reset/preview, Type field, Auto, Pupil Size/Darken fields/tracks/handles,
divider and Overlay checkbox. These controls are unavailable for processing:
PhotoCraft has an existing click-local engine operation, but Camera Raw does not
yet replay that operation inside its editable transaction. No working Auto,
correction or processing-parity claim. Native view navigation must preserve the
photo recipe, preview revision and history.

Vertical geometry is live under layout JSON `redEye` (height, typeTop, pupilTop,
darkenTop, dividerTop, overlayTop), with finite, ordered, bounded validation.
`red-eye-element-regions.json` covers 18 named regions using a single registration:
source image12 rectangle (28,432)-(568,812), native 2x rectangle
(2260,360)-(2800,740), normalized width540. No control moves independently.

Initial native capture and element audit are in
`target/acr18/parity-20261017/native-red-eye-baseline` and
`ref12-red-eye-baseline`. Requested view/photo invariants passed. Overlay inspection
identified incorrect bright track gradients, checkbox accent/placement and numeric
field offsets. Source revisions address these; final validation/recapture pending.
The baseline is diagnostic evidence, not a parity pass. Six Python auditor tests
pass, including rejecting a mismatched reference hash before writing evidence.
All existing region specifications now identify their reference SHA256 as well.

Full every-element/every-reference parity remains incomplete. Missing native tool
screens/dialogs and shared chrome remain in the inventory; per-element region
coverage is not full-window coverage. Hosted builds have not been promoted.

Validation/evidence follow-up: nine workspace tests pass, including unavailable
Red Eye controls and recipe/history invariants. The initial test query matched
the unrelated toolbar Type button; corrected by selecting the disabled node.
Native14-state font11 capture passed every requested view/photo invariant.
`scripts/audit_camera_raw_reference_views.py` generated235 region audits, listing
all19 references and explicitly recording13 references without individual audits.
It verifies all source hashes and keeps each panel's fixed registration. It does
not claim unlisted/shared-element or whole-reference completeness.

JSON panelFontSize11 matches measured reference ink widths: Pupil Size99px,
Darken71px (previously108/77px). Type/Overlay are within about1px of reference
widths; offsets still differ. Grading whole-crop error fell from about6.90 to6.74;
Info from13.36 to13.32 (diagnostic RGB means, not parity scores).

A further Grading footer gap14.5 moves footer rows20px down and reduces that crop's
RGB mean to about5.05, but also moves the following Detail header20px down. The
source now adds independent gradingBottomPadding, set13 instead of23, to preserve
that boundary. This combined correction awaits a rebuilt capture; do not treat
the footer-only capture as a successful overall change. Final pipeline session
53266 builds/captures/audits current source and runs workspace/library/clippy/
layers/wasm gates. Exact source hashes, live handles and next steps are in
`target/acr18/parity-20261017/active-work.json`. No hosted promotion or full parity.

### Curve/Mixer individual audits and native region strip (2026-10-17)

The preceding Grading padding build was captured in all14 states. Inspection of
`reference-audit-padding-final` verifies that footer gap14.5 plus bottom padding13
moves the Grading footer20px down while retaining the Detail boundary. Grading RGB
mean is about5.05, versus6.74 before; Detail remains7.16. Neither is pixel-identical.

New source specifications measure114 regions in reference4 and98 in reference5.
The complete current report contains447 named regions. References2,3,9,10,11,14,
15,16,17,18,19 still have no individual audits. Histogram/metadata/photo values,
shared rail/footer and unavailable behavior remain unverified even where a region
is measured. The Curve sub-element graph regions overlap intentionally.

Those overlays exposed a narrow/early Curve graph and absent parametric region
strip, which also pulled Mixer upward. Source now gives Curve independent
left/right/bottom padding and selector/graph/strip spacing, all live JSON. The
three strip handles edit existing curveSplits parameters, remain1percent apart,
accept drag/arrow keys and reset by double-click. A workspace drag test checks
60percent after dragging the middle handle,74percent before its75percent neighbor,
unchanged other parameters and no committed history. Graph painting now uses a
continuous histogram mesh, square frame, and theme-derived graph/grid/curve ink.
These source changes await their new native build; do not treat the old Curve
capture as evidence for them.

White Balance's measured track is7px early in the prior overlay. The source now
exposes whiteBalanceBottomGap (JSON9.5 instead of hard-coded6 native points) and
uses the existing chevron glyph. Native recapture must confirm the correction.

The preceding library run passed58 tests and failed the Camera Raw catalog scan
for missing Not available. Added its cameraRaw translation in all20 catalogs.
Nine workspace tests passed on the preceding source. Final Curve source uses
pipeline95549: snapshot,14-state capture,447-region audit and focused tests plus
clippy/layers/wasm;80389 is its queued check,53266 is the preceding padding gate
pipeline. Active handles and exact source hashes are in active-work.json. Full
19-reference/every-element parity remains incomplete. No hosted promotion or push.
