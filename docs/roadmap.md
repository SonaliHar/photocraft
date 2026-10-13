# Roadmap

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Updated 2026-10-09.

**Parity metrics.** `cargo xtask parity` measures how much of Photoshop's menu tree is *wired to a
command* and writes [`parity.md`](parity.md). It does **not** measure whether those commands behave
like Photoshop, feel right, or survive real files. Read it next to the PSD composite oracle, the
test count, and the [honest parity assessment](#honest-parity-assessment-2026-10-05) below, which
is the reference answer to "how close are we really". The weighted feature estimate in
[`parity-estimate.md`](parity-estimate.md) (~82%) counts feature surface and overstates user-facing
readiness; treat it as an upper bound.

| M | Status | Where we are |
|---|---|---|
| M0 Skeleton | ✅ | workspace, xtask (layers / wasm / ci / stats / corpus / parity), CI workflow |
| M1 Foundation | ✅ | geom, color (27 blend modes), raster (COW tiles, any depth), doc, ops, cms (ICC) |
| M2 PSD v1 | ✅ | photocraft-psd: 134/135 real files byte-exact round trip |
| M3 Viewer app | ✅ | egui shell (Pro / Studio / Classic themes), 13+ codecs, native and web (trunk) builds |
| M4 Native format + engine | ✅ | 500+ commands, `.pcraft` (incremental, autosave, crash recovery), CLI, persistent preferences |
| M5 GPU compositor | 🟡 | wgpu compositor drives the canvas, layer effects, vector masks, artboards, pattern fills and every clip case included (≤1/255 vs CPU); Multichannel documents fall back to the CPU |
| M6 Paint + select | 🟡 | brush engine, all selection tools, multi-layer selection, snapping + smart guides, free transform + warp; stylus pressure on Windows (WM_POINTER via winit), the web (Pointer Events, with tilt/twist), macOS (AppKit event monitor: pressure, tilt, rotation, eraser end) and Linux X11 (XInput2 raw valuators); on Linux Wayland a pen opens the window through Xwayland (native tablet-v2 still open, #79) |
| M7 Adjust + filters | 🟡 | 16 adjustment layers + destructive-only adjustments, 70+ filters incl. Blur Gallery, Actions record/replay, Fade |
| M8 PSD v2 | 🟡 | adjustments (incl. Selective Color, Color Lookup), fills, effects, patterns, text, shapes, smart objects, alpha channels; oracle 111/170 |
| M9 Text, vector, styles | 🟡 | type engine + Warp Text, shapes / pen / paths, all 10 effects on CPU and GPU (parity ≤1/255, 30/31 corpus effect files on the GPU) |
| M10 Smart features | 🟡 | classical Select Subject / Object, content-aware fill and scale, healing, auto-align / auto-blend; ML backend not started |
| M11 Automation + formats | 🟡 | MCP (headless + live bridge), batch, Image Processor, prefs over MCP; DoD test passes (10 agent tasks over MCP, `automation/tests/agent_tasks.rs`); since 2026-10-07 the CLI rejects unknown flags and answers `<subcommand> --help`, batch runs report same-name outputs instead of overwriting them, MCP `command_batch` steps can start background jobs, and headless saves never flatten over the opened file; JP2 / DICOM / DPX / C2PA pending |
| M12 Pro parity | 🟡 | CMYK / Lab / Indexed / Bitmap / Duotone, ICC + soft proofing, channels + Quick Mask, smart-object stack modes, artboards, layer comps; print, HDR, photomerge, timeline pending |

**Menu parity: 532 / 625 (85.1%)** on 2026-10-01, up from 224 (35.8%) the day before. See [`parity.md`](parity.md).

## Honest parity assessment (2026-10-05)

This is the reference answer to "how close are we to Photoshop parity, really". Agents: read it
before picking work. Update it (with dated measurements) when the numbers move; don't restate the
menu-parity number in its place.

**Measured numbers live in the [scorecard](scorecard.md)** (`cargo xtask scorecard`, kept current
by CI): performance scenarios against their budgets, corpus floors, per-area checklists with
done / partial / missing counts, and the number of settings that do nothing. Where the scorecard
and the estimates below disagree, the scorecard's numbers supersede them. As of 2026-10-05 it
counts 67 of 129 preferences that nothing reads; 3 of the 25 budgeted performance scenarios meet
their #209–#211 targets (first baseline, taken on a heavily loaded machine), and the 150-layer
nudge scenario crashes the GPU compositor; the open area issues' checklists (#203–#220) are
almost entirely missing or partial.

2026-10-07: UI Font Size now applies to interface text, including CJK fallbacks, independently
of display and canvas zoom (#532). The current preference audit drops from 59 to 58 unread
settings out of 135; see the regenerated scorecard.

2026-10-12: the local XCF continuation checkpoint passed 2,792 tests across 351
official corpus suites (556 explicitly ignored), with its bound source cohort
unchanged. The declared validation release overrides and local unpinned corpus
warnings are retained in the [implementation evidence](../specs/002-full-xcf-import/evidence/continued-foundations-20261012.md).
Subsequent native RGB U32/F64 layer-extraction fixes preserve low sample bits;
single optimized synthetic 24MP measurements improved U32 from 2,910 to 550 ms
and F64 from 2,637 to 785 ms. These are scoped extraction measurements, not
general rendering or budget/RSS qualification. Full XCF inventory, wide editing
and two-axis physical-size migration remain incomplete.

2026-10-12 later: the next local official corpus run passed 2,834 tests across
354 suites (556 explicitly ignored, zero failures). Its five tested package
sources were unchanged; two separately verified UI bounds fixes occurred during
the run. The source census exception and validation profile are recorded in the
implementation evidence above. Subsequent Print Size tests and a Vulkan visual
check verify canonical square DPI; unequal-axis Print Size remains unsupported.

2026-10-12, cohort D: the next official local corpus run passed 2,906 tests
across 357 suites (556 explicitly ignored, zero failures), including HEIF.
The 1,809 listed product paths matched before and after the gates. The local
unpinned corpus warnings and validation release overrides remain recorded in
the implementation evidence. Separate engine and UI runs passed 1,326 and
1,277 tests. Actual browser PNG, slice ZIP and square Print Size checks passed,
including native save/reopen; the ordinary HEIF web bundle is still 23,059 bytes
over its unchanged 24 MiB ceiling. Full XCF qualification remains incomplete.

2026-10-13, cohort E: shared edit validation and bounded browser ZIP delivery
bring the actual ordinary HEIF web WASM to 24,621,117 bytes, with 544,707 bytes
headroom under the unchanged 24 MiB ceiling. Fresh engine and UI runs passed
1,339 and 1,277 tests; strict checks, official WASM checks and layers passed.
Fresh browser PNG metadata, slice archives, failure recovery and native Print
Size save/reopen passed their scoped checks. Image Duplicate remaps current
references, but its browser Save/Save As defaults to PSD after source ownership
is cleared; live filters therefore refuse export. The affected selective engine
corpus run subsequently passed 1,356 tests across 130 harnesses (231 ignored),
with its source bindings unchanged. A native save suggestion fix remains
pending. Full XCF qualification is still incomplete; see the implementation
evidence for exact package identities.


2026-10-13: Cohort F preserves full native save names for typed Duplicate and native-memory reopen, and keeps the prior clipboard when Cut fails. Fresh engine tests passed 1,343 with 20 ignored; UI tests passed 1,280 with 71 ignored, with strict, panic, WASM and layer checks passing. The ordinary HEIF web build is 24,622,200 bytes, 543,624 below the unchanged ceiling. Scoped browser Duplicate native save/reopen and physical Windows native save/reopen/rename/Undo/Redo samples preserve native state; independent native peer review passed. One 24MP Cut pair is descriptive, not a speed or memory-budget qualification. Full XCF, T120/T121, native_large, drop/recovery and publication remain open; qualified sources stay 6,789 and parent markers 12/131.

**Bottom line.** Two days after 0.2.0 we had merged ~96 PRs and closed ~48 issues, but **real
Photoshop parity is still well below 50%**. The biggest gaps are AI, missing tools, professional
workflow depth and the plug-in ecosystem. Most fixes since 0.2.0 have passed our tests but have
**not yet been validated by users**, and this week proved our tests miss what users hit:
on first public use they found broken basics (text selection offset, 214 shortcut failures, panels
resizing themselves, an immovable crop frame, folders that wouldn't collapse, lag on layout PSDs)
that all counted as "live" in `parity.md`.

**Overall.** Feature surface: roughly **60–70%** of what a typical Photoshop user touches exists in
some form. "A professional could switch for daily work": roughly **25–35%**. True 1:1 parity is
**many months** of focused work, and some areas need product decisions rather than effort.
Confidence: moderate — the next users of 0.2.x will move these numbers either way.

### By dimension

| Dimension | Measured / evidence (2026-10-05) | Grade | Notes |
|---|---|---|---|
| Menu wiring | 626/626 menu items dispatch a command (`parity.md`) | high but shallow | Says nothing about behaviour. |
| PSD fidelity (rendering) | Corpus oracle 115/170 (68%): 30 differ, 26 have no usable reference, 1 import error. 2026-10-07: io corpus 146/170, psd-tools corpus 236/309 (was 229: Advanced Blending knockouts), Photoshop oracles 132/258 | medium | Push to 170/170 under way (effects/strokes, multi-instance effects, 16/32-bit and colour modes, references for skipped files). |
| PSD round trip | 169/169 re-import identically; every adjustment layer and blend mode round-trips | high (within corpus) | Floors in `crates/io/tests/corpus.rs`; raise, never lower. Corpora: `cargo xtask corpus --all` (ours: https://github.com/Terra854/photocraft-corpus). |
| Smart filters / text / effect shapes in PSDs | Measured on our Photoshop-authored set (https://github.com/Terra854/photocraft-corpus, `corpus/photoshop`, 258 files): see the per-group floors in `crates/io/tests/corpus.rs` and `crates/engine/tests/photoshop_oracles.rs`. Smart objects and smart filters now survive PSD save and open (41 corpus files, 206 smart objects, round trip strict; Photoshop opens our exports with live filters); re-rendering Photoshop's smart filters with ours matches 5/30 (was 1/30) | low–medium | Remaining re-render gaps are filter maths (Gaussian/Motion Blur, Unsharp Mask, Emboss, Add Noise RNG) and bicubic placement. |
| Core editing (layers, masks, selections, adjustments, filters, transforms) | Broad engine coverage; many interaction bugs fixed after 0.2.0 (adjustment dialogs, Curves, crop, Move/Transform modifiers, gesture origin) | medium | Fixes not yet user-validated. |
| UI / UX polish | Shortcut audit 214 → 0 failures; dock, Layers rows and menus reworked; first visual-QA sweep found 14 defects (#147–#157). 2026-10-07: the keyboard-only shortcuts with no menu item (⌥[ ⌥] ⌥, ⌥. layer navigation, ⇧⌥[ ⇧⌥] to extend the selection, 1–0 for opacity and ⇧ for flow or fill, ⇧[ ⇧] hardness, ⌥⌘T to transform a copy and ⌥⇧⌘T to step and repeat), ⌥-click colour sampling with painting tools, double-click a Layers row for Layer Style, File › New from Clipboard, a centred main window and remembered Liquify settings (#352, #417, #350, #368, #419, #418) | low–medium | Needs recurring visual QA with realistic documents. |
| Tools | 2026-10-09: Pencil, Mixer Brush, Patch, Content-Aware Move, Vertical Type, Pattern Stamp and Rotate View are toolbar tools. Remaining missing tools include Red Eye, Art History Brush, Freeform/Curvature Pen, Add/Delete Anchor Point tools, single row/column marquee, Color Sampler, Perspective Crop, type masks and Frame | low–medium | Patch has a live healing preview. Content-Aware Move is partial: Move/Extend, Structure/Color and Sample All Layers exist; Transform On Drop and a live result preview remain missing (`TOOL-213-4` in the scorecard). Magic/Background Eraser added; live gradients in progress (#180). Magnetic Lasso added 2026-10-08 (live-wire edge tracing, Width/Contrast/Frequency, `select.magneticLasso`). Pattern Stamp added 2026-10-08 (S flyout, `paint.patternStamp`, Aligned / Impressionist). Rotate View (2026-10-09) turns the canvas camera around its centre without rewriting pixels; Reset View and Match Rotation copy the angle. |
| Painting | Brush model and Brush Settings panel near Photoshop; .abr/.grd import; persistent presets; pen pressure/tilt on Windows, web, macOS and X11 | medium | Native Wayland pen input open (#79; a pen opens the window through Xwayland meanwhile); X11 pressure confirmed by a user, macOS not yet verified on tablet hardware. |
| Text / typography | Engine works; caret placement and size editing fixed; OpenType features, text-on-path editing, composer parity partial | medium-low | Measure with the Photoshop-authored set. |
| Colour management | Colour-managed canvas (document → monitor), embedded CMYK profiles, linear EXR/HDR, 16-bit float canvas | medium-high | Monitor profile follows only at launch. |
| Performance | 14k+ px on the GPU at ~⅓ the memory; adjustment preview 285 ms → 4–9 ms; font-size edits 297 ms → 4.6 ms; 2026-10-07: 30 MP TIFF open (banded, parallel strip/tile decode) Deflate 345 → 32 ms, LZW 428 → 43 ms, BigTIFF and every IFD readable | medium-high on rasters | Complex layout documents still laggy (#125/#128); >16384 px GPU tiling in progress (#49). |
| Stability | Never-crash lint series, crash guard, `panic_hunt` fuzzing in the gate | medium-high | No field crash data yet. |
| Camera RAW | DNG, CR2, Sony ARW (lossless + compressed), Nikon NEF (lossless + lossy compressed), RW2, uncompressed ORF | medium | CR3, RAF, Nikon "lossy after split" and calibrated colour for non-DNG cameras still open (#50). |
| AI / generative | none | ~0% | Deferred by decision (#41). |
| Ecosystem | Sandboxed WebAssembly plug-ins instead of .8BF; no ExtendScript/UXP/.atn; no Adobe Fonts/Libraries/cloud docs | low | By design for 8BF; scripting compatibility open. |
| Platforms | macOS (notarized), Windows, Linux (AppImage/deb/rpm/Flatpak bundle), web | medium-high | Flathub later (#173); Windows signing material pending. |
| Localisation | 2026-10-07: 10 UI languages; menu, `tl!`, blend mode, preference and brush-section coverage enforced by tests; live switching and scoped Preferences previews | medium | Engine errors/status messages still partly English; CJK web fonts, browser-locale detection, and RTL remain open. |

2026-10-08: Motion Blur adds adaptive FFT convolution for wide streaks while retaining the
row kernel merged in [#902](https://github.com/storytold/photocraft/pull/902) for shorter
streaks and unsupported cases. Native release medians of three paired runs on an Intel
i7-9750H with 12 Rayon workers, synthetic RGBA8 at 30° / distance 1000: 24 MP
22.594 → 9.785 s (2.31×), and 1.5 MP
2.455 → 0.592 s (4.15×).
Full-image comparisons differ by at most one U8 code level; distance 64/256 retain the
byte-identical row output. These measure decode/halo/filter/write/prune, excluding UI proxy
creation, compositing and upload; the UI remains synchronous. FFT work is scheduled within
a conservative 512 MiB working-set estimate, excluding stored Surface tiles and row fallback
memory. The crossover is a heuristic; images dominated by near-cutoff alpha can need extra
scalar work. These measurements do not establish Photoshop filter parity.

2026-10-08: the tools update above is checked against the current toolbar groups in
[`panels.rs`](../crates/ui-egui/src/panels.rs), the Pencil and tool-cycle tests in
[`pencil_tests.rs`](../crates/ui-egui/src/pencil_tests.rs), Patch's live-preview test in
[`patch_preview.rs`](../crates/ui-egui/src/patch_preview.rs), and Vertical Type's point/paragraph
creation and undo test in [`type_tool_tests.rs`](../crates/ui-egui/src/type_tool_tests.rs).
These implementations remove the tools from the missing list; they do not establish Photoshop
behavioural parity. The historical estimates and corpus measurements in this assessment are
unchanged; current measured floors remain in the scorecard.

2026-10-07: Camera Raw PSD mapping covers relative custom white balance, Light/Presence,
parametric and four point curves, HSL, Color Grading, sharpening/noise detail, grain and numeric
post-crop vignette controls. Two revisions of one supplied Photoshop ACR 18.4 PSD preserve their
filter descriptors without edits. The updated revision stays opaque because of active manual
Optics and unverified vignette style; a descriptor projection verifies its supported controls.
Synthetic 8/16/32-bit PSD → edit → `.pcraft` → PSD tests preserve the filter stack, mask and
undo/redo. Unmapped Camera Raw fields/versions remain opaque; Photoshop
acceptance of generated exports and pixel parity are still unverified. Corpus floors above are
unchanged.

2026-10-09: XCF import now has 58 supported GIMP 2.8/2.10/3.x corpus inputs verified
against independent GIMP-exported PNGs after both sides pass through ICC colour management.
The worst colour/alpha difference is 0.8010/255 (the acceptance limit remains 2/255);
five deliberately unsupported cases are counted separately. Coverage includes RGB/Gray
U8/U16/half/F32 linear, nonlinear and perceptual encodings, compatible embedded ICC,
indexed duplicate-palette identity, masks, isolated/pass-through groups and raw/RLE/zlib.
Original samples, ICC and indexed provenance survive native saves and edits. Imported
composition policies explicitly use whole-document CPU fallback, verified on a hardware
GPU adapter. Other blend operators and editable text/vector/GEGL features remain outside
this scope; XCF writing is not supported. These measurements cover the checked corpus,
not arbitrary GIMP projects or interactive desktop acceptance.

2026-10-10: The pinned XCF gate now supports 61 of its 63 inputs, including exact
U32/F64 samples and independent DPI axes; two cases still reject explicitly.
As of 2026-10-11, the additional unpublished local corpus passes 6789 source/edit files through the
actual parser, importer, native reopen and checked rendering for scoped modern
blend modes, mask operators, Gray masked composition and 128 RGB/Gray F32/F64 Normal operator-mask cases across implicit nonlinear/linear encoding and two exact authored ICC-profile pairs, plus 96 implicit RGB/Gray U8/U16/U32 nonlinear/linear Normal operator-mask cases, isolated/pass-through groups,
point filters (including five ordered enable/remove/reorder/opacity states, eight Posterize and U8 Value Invert files, six additional U16/F32/F64 mixed-alpha Value Invert inputs, four false-luminosity Mono Mixer files and three finite HDR RGB Clip inputs and 48 U16/F32/F64 nonlinear/linear RGB/Gray Brightness/Contrast states plus 48 separately qualified GEGL Levels states and 48 Exposure black-level states), Gaussian, Box Blur, Median Blur, horizontal Motion Blur, square Pixelize, Curves and Levels. This includes explicit Multiply spaces,
Hard Mix, fourteen legacy blend modes, ten RGB/Gray floating-selection originals and twenty GIMP floating-action reference projects, hidden saved channels, enabled/disabled isolated Gaussian groups and
one modified painted-text source with authoritative saved pixels, ten finite
Gaussian alpha/HDR/profile cases, twelve Box Blur cases, sixteen Median cases,
two horizontal Motion Blur cases, two square Pixelize cases, thirty modern/legacy HSV/HSL, LCh and luminance cases, sixteen RGB/Gray deterministic alpha/Color Erase cases, twelve editable rectilinear
vector cases plus two even-width AA, four no-AA bevel and four no-AA round-stroke files and six odd-width Butt/Miter stroke files, thirty-two RGB/Gray F32/F64 cubic translation files, sixteen additional exact-nearest/linear translation inputs, twelve additional nearest and twenty-four additional linear/cubic scale/rotation/projective inputs and 48 additional distinct U8/U16/F32 RGB/Gray nonlinear/linear nearest scale/rotation/projective source/edit files and ten RGB/Gray composition-command reference edits. New input increments are deduplicated by saved XCF SHA256; repeated source and reenabled files are counted once. Six independently saved F64 cubic edits with signed/over-one alpha now pass a separate finite-source Normal policy for the qualified implicit-profile, full-opacity, single-raster domain. Matching edit commands preserve those alpha samples without clamping; partial backdrops, masks and other composition domains remain unqualified. These vectors
regenerate from current path geometry while preserving their original cache;
translated geometry, a compound path hole edit and native reopen match independently saved GIMP references. Floating
attachments resolve exact source-record pointers to canonical layer IDs.
Two modified-vector originals reject pending qualification of saved placement:
their saved pixel buffers match GIMP, while serialized and reopened offsets differ.
Active implicit sRGB/sGray interpretation is separate from immutable source
provenance and explicit profile assignment. These are verified increments toward
full import; remaining blend/filter families, general text/vector/link regeneration,
resource recovery and desktop acceptance keep full support incomplete.

The earlier 384 distinct additions are 96 implicit perceptual Normal/mask files,
24 three-range Color Balance states, 48 Color Temperature states, 156 legal-depth
Linear geometry files, 32 U32/F64 low-alpha geometry files, 16 original-half
geometry files and 12 open-axis Butt/Square stroke files. Native source bits,
current edit state and immutable source evidence remain separate; the half geometry
path explicitly rounds generated samples to binary16 before exact F32 widening.
These additions use independently authored GIMP originals and saved edit references.
A further 234 distinct additions comprise 128 assigned Gamma22 and unequal-channel
TRC Normal mask files under Linear/Perceptual storage, 48 Perceptual geometry
files, 36 Desaturate states, 12 off-grid Color Temperature edits, six Round-axis
strokes and four collinear Butt strokes. Each counted slice passes actual import,
native preservation, independent rendering and applicable edit/history/safety gates.
Another 104 distinct additions are 48 integer/Half/Perceptual Color Balance states,
48 fractional Half/Perceptual geometry files and eight collinear Square/Round
stroke files. Color Balance now verifies 72 imported states and 54 real edits,
216 altered source contexts and 123 current-state refusals, preserving exact
source/native/history data. Its new precision states meet the existing 2/255
display limit (maximum 1.553/255).
The implicit Normal mask suite verifies 352 files, with the 128 newly assigned
profile files verified in separate suites. Explicit current ICC
interpretation validates the active profile; immutable original ICC never supplies
stale primaries or replaces a corrupt current profile. Profile-defined blend/filter
transfer after conversion still requires its own current definition and evidence.

Native-container work now includes bounded seekable ZIP64 read/write and atomic
streaming replacement with failure/cancellation protection. Streaming saves now emit
Zstandard raw blocks with a fixed 128 KiB decoder window and bounded 64 KiB
copy/cancel checks, avoiding another object-sized encoded buffer. This trades
compression for bounded copying; ordinary incremental saves keep compression.
Preparation now has fallible residency limits, bounded cache/hash copies and bounded escaped-string serialization. Controlled native Save a Copy uses the shared cancellation token and preserves the prior destination on cancellation. Complete model-heap accounting, deeper legacy preview work and all byte/shell consumers remain under integration. A real 4,294,971,392-byte
entry passed write/read in 17.9 seconds with 5.1 MiB peak process memory, and an
independent Python ZIP reader verified payload size and CRC. This is a container
test; full >4 GiB XCF import/edit/native-save acceptance remains pending and the
native_large policy stays disabled.

2026-10-12 resume: twenty-four independently observed isolated Normal group
controls now pass actual bounded import, native save/reopen and typed
mask/visibility history. Eight linked NoHalo controls retain a maximum1/255
error under the unchanged2/255 bound; qualified current placement/transform
commands preserve original source state and persist with selective native
reader floor 7. One actual 513×385 U16 native hash improved from 4.30s to 10.8ms
after eliminating repeated tile authentication, with exact bytes and existing
quotas/cache size preserved. This test-profile memory-transport diagnostic is
not a disk/RSS or 24–36 MP budget result. The 6789 qualified unpublished input
count is unchanged; full stock/version coverage, native_large and physical GUI
acceptance remain open. Details: [resume evidence](../specs/002-full-xcf-import/evidence/resume-20261012.md).

2026-10-12 continued: Unpremultiply admits the three observed RGB/F32/Linear
singleton envelopes, with native persistence and atomic disabled/no-op/overflow
behavior. A distinct Video Degradation epoch passes 108 observed source/native
states and 36 cyclic edits plus 36 disables across its 36 options; the older
three-state epoch remains unchanged. These bounded increments pass current
precision/profile/mask validation, history, hostile-input, strict, WASM and
panic checks. They do not qualify other source envelopes or release-wide stock
behavior. The inventory gate now requires hash-bound full-epoch and per-subject
proofs; all nine release epochs remain pending. The 6789 input count and existing
floors remain unchanged. Scoped epoch evidence validation now anchors the full
selected release domain without promoting its pending global qualification.
The actual native-memory Save As fix suggests `.pcraft` for native documents
whose internal title ends in `.xcf`; focused tests, strict lint and the official
23-crate WASM plus HEIF checks pass. The latest local full corpus checkpoint
passes 2,773 tests across 348 suites, with 546 framework-ignored tests and no
failures; local unpublished corpus warnings remain explicit.

Web asset delivery remains an isolated prototype. Its final normal HEIF WASM is
25,073,423 bytes, fitting the unchanged 24 MiB limit by 92,401 bytes. Actual
browser checks cover startup, nested paths, warm cache reuse, four readable
network-failure refusals, and native edit/undo/redo/save/reopen. Independent
artifact checks preserve pixels, masks and profile exactly. Physical memory,
cache-denial/corrupt-cache recovery, external cancellation, adoption and full
interface acceptance remain pending; see [GUI evidence](../specs/002-full-xcf-import/evidence/us5-gui.md).

### Where we're going (priority order)

1. **Ship the fixes:** cut 0.2.1 once the current batch lands, so users validate them.
2. **PSD fidelity to 170/170** with round trips, plus the Photoshop-authored reference set for smart
   filters, the text engine and effect shapes (https://github.com/Terra854/photocraft-corpus, fetched by
   `cargo xtask corpus --all`); enforce floors in `corpus.rs`.
3. **Workflow acceptance tests:** 30–50 real tasks (retouch a portrait, social post with text and
   effects, composite with masks and adjustment layers, CMYK print prep…) scripted end to end and
   checked against Photoshop's output on every build. Their pass rate becomes the headline parity
   number.
4. **Tool coverage and workflow depth:** Pen variants (Direct Selection landed, #790) and
   Perspective Crop; finish Content-Aware Move's Transform On Drop and live result preview.
   Pencil, Mixer Brush, Patch, Vertical Type, Pattern Stamp and Rotate View are already available on the toolbar.
5. **Complex-document performance** (#125/#128) and GPU tiling beyond the texture limit (#49).
6. **Recurring visual QA** (`cargo run -p photocraft-engine --example designer_psd`) and fast
   turnaround on user reports (OS, document size, layer count, screenshot).
7. Later / needs decisions: generative AI backend (#41), scripting compatibility (ExtendScript /
   UXP / .atn), Flathub (#173), Wayland pen pressure (#79).

## Current focus (infrastructure before the long tail)

Landed on 2026-10-01:
- multi-layer selection, live smart objects + smart filters, alpha channels + Quick Mask;
- patterns, Warp, preferences, snapping, ~33 filters, the remaining core adjustments;
- Layer Comps and Artboards, GPU layer effects, Liquify / Puppet Warp / Perspective Warp;
- the PSD fidelity pass (oracle 102 → 111), the M11 MCP acceptance test, and the release pipeline
  (`docs/releasing.md`).

Next:
1. **Follow "Where we're going" in the assessment above.** 0.2.0 shipped signed and notarized on
   2026-10-05 (`docs/releasing.md`); Windows code-signing material still needs to be obtained.
2. **Fidelity**: the PSD oracle (113/170). Modern Brightness/Contrast and grayscale Levels
   curves; chisel-soft / stroke-emboss bevel shapes; Photoshop's 8-bit blend rounding; non-Normal
   modes in Lab documents; smart-filter maths against Photoshop's (re-render oracle 5/30).
3. **GPU**: only Multichannel documents and regions over the texture limit fall back to the CPU.
4. **Vanishing Point, Camera Raw / Lens Correction, Face-Aware Liquify** (needs a landmark model).
5. **Panels**: Patterns, Styles, Glyphs, Character/Paragraph Styles, Timeline; Custom Shape tool.
6. Print, Photomerge, Merge to HDR, video layers.

## Milestone definitions

Each milestone has a **definition of done (DoD)** and must leave `main` green on all Tier-1 platforms plus a wasm build.

| M | Name | Scope (key items) | DoD / acceptance |
|---|---|---|---|
| **M0** | Skeleton | Workspace, all crate stubs, lints, `xtask` (layers check, ci), CI matrix, testkit | `cargo test --workspace` green; `cargo check --target wasm32-unknown-unknown -p photocraft-engine -p photocraft-ui-egui` green; layering check passes |
| **M1** | Foundation | geom, color (formats, blend math for all 27 modes, sRGB/linear), raster (sparse COW tiles, U8/U16/F32), doc (full type model incl. CMYK/Lab/adjust/smart), ops (history) | Property tests (proptest) on tile COW and history; blend-mode reference tests against published formulas |
| **M2** | PSD v1 | `photocraft-psd`: header, resources, layer records, channel data (raw/RLE/ZIP/ZIP+pred), masks, groups (lsct), unicode names, unknown-block passthrough, merged image, PSB; writer | Round-trip byte-stability tests; synthetic PSD generator tests; fuzz target; ≥150 unit tests |
| **M3** | Viewer app | codecs (png/jpeg/tiff/webp/gif/bmp/tga/pnm/qoi/exr/hdr, all read+write), CPU compositor, egui shell (menu from registry, canvas pan/zoom, layers panel, history), open/save, native + web build | Opens PNG/JPEG/PSD, shows layers, toggles visibility, undo/redo, saves PNG/PSD; web build loads a file from the browser |
| **M4** | Native format + engine | `.pcraft` bundle, command registry with schemas, jobs/cancellation, snapshots via arc-swap, CLI (`convert`, `run`, `inspect`) | CLI parity tests: every GUI command also runs headless |
| **M5** | GPU compositor | wgpu planner backend, tile residency, parity tests vs CPU (all blend modes), viewport on GPU, mips | GPU vs CPU ≤1/255; pan/zoom 60 fps on a 100 MP / 20-layer document |
| **M6** | Paint + select | platform input (pen: macOS/Windows/Linux/web), brush engine v1, eraser, marquee/lasso/wand, move, free transform, crop | Brush latency <1 frame; selection-restricted paint tests |
| **M7** | Adjust + filters | 16 adjustment layers, first 30 filters, schema-generated dialogs + live preview, actions record/replay | Golden tests at 8/16/32f; action replay determinism |
| **M8** | PSD v2 | adjustment layers, fill layers, lfx2 effects (descriptor parser), text (EngineData) preserve+render, smart objects, vector masks | Composite-oracle pass rate ≥90% on corpus |
| **M9** | Text, vector, styles | parley text layers, shapes/pen/paths, all 10 layer effects on GPU | Visual goldens; PSD text round-trip |
| **M10** | Smart features | `ml` (ort native / ort-web on the web), Select Subject/Object/Sky, Remove BG, Remove tool, content-aware fill, healing, AI denoise, RAW develop | Quality benchmarks on a public dataset; timing budgets |
| **M11** | Automation + formats | MCP server, batch, scripting, remaining formats (JP2, DICOM, DPX…), C2PA | An agent completes 10 scripted edit tasks via MCP |
| **M12** | Pro parity | CMYK/Lab UI, print, HDR display, photomerge/HDR merge, timeline, layer comps, artboards, symmetry, neural filters | `xtask parity` ≥ 90% of Photoshop menu checklist |

2026-10-10: Eight further independently saved open odd-width Butt stroke states
pass actual import, native/cache preservation and registered style/history tests.
Only monotone axis paths with widths3/5 are newly qualified; reversed/clipped
and closed-rectangle controls remain excluded. Current local count2399.

2026-10-10: Another64 distinct exact type4 ICC Normal-mask inputs qualify
Linear/Perceptual RGB/Gray storage. Actual registered layer/composition edits,
native reopen, undo/redo and active-profile negatives pass. No generic profile
class or global Curve change is implied; current local count2399.

2026-10-10: Another60 Gray Saturation inputs pass actual import/native/render
and54registered edits/28changed-context atomic failures. All three colorspace
choices are independently qualified for Gray; RGB CIEYuv remains excluded.
Current local count2399, public pinned61/63 unchanged.

2026-10-10: Another68 distinct inputs add60 RGB/Gray Alpha Threshold states
and8 opaque no-AA monotone cubic arch states. Current alpha and curve controls
regenerate while canonical source colors/cache remain exact. Import/native/render,
registered edits/history, limits/cancellation, strict/wasm/panic gates pass.
Current local count2399; general text/curves/profile/stock union remains open.

2026-10-10: Another96 distinct inputs qualify a bounded analytical assigned ICC
class for nonlinear F32/F64 Normal/mask composition. Actual import/native/reference,
224 registered control states including earlier profile regressions, active-profile
atomic rejection, five-crate strict lint and wasm pass. Local count2399;
text integration and Exposure geometry command gates remain in progress.

2026-10-10: Another72 independently authored Exposure geometry inputs pass actual
import, whole-owner native preservation, independent projection and registered edits.
The local pipeline count is2399. The qualified context has a295x160 canvas,
257x129 owner and two observed offsets; general geometry/filter unions remain open.

2026-10-10: Nine additional unique exact-font text inputs pass import/current
regeneration, registered content edits, native preservation, undo/redo, locks and
atomic cancellation. The local pipeline count is2399. This first text domain covers
unhinted/no-AA Inter Regular32px repeated stems; general text remains incomplete.

2026-10-10: Another64 unique analytical ICC boundary inputs pass import/native/faithful
rendering and registered profile controls. Local pipeline count2399; public pinned
61/63 and floor61 remain unchanged. Control-only fixtures are excluded from this count.

2026-10-10: Another221 unique local pipeline originals pass actual import/native/render
and registered editing/current-state gates:17 rectilinear Inter text,12 RGB U16/F32
Yuv Saturation,192 analytical ICC Linear/Perceptual Normal-mask inputs. Count2399;
control-only projects excluded, public pinned61/63/floor61 unchanged. Original source
and cache/profile provenance remain exact. RGB F64 Yuv stays refused after independent
GIMP repeat discrepancies; general text/geometry/filter coverage remains incomplete.

2026-10-11: Another251 deduplicated local pipeline inputs are qualified:11 antialiased
rectilinear Inter text states,24 Sepia states,24 RGB Alien Map states,64 analytical
nonlinear Normal-mask geometry states and128 corresponding Linear/Perceptual states.
Actual import/native preservation/rendering, registered edits/current-state rejection,
strict lint and wasm gates pass for these scoped classes. Root independently verified
13,191 frozen source/reference/control assets for this increment; controls are excluded
from2650. All existing color/alpha tolerances remain unchanged. General glyphs, HSL Alien
Map, integer geometry and font-size changes are separate work in progress. Physical GUI
acceptance waits while the desktop is busy. Public61/63/floor61 and broad8/131 unchanged;
full XCF import and native_large remain incomplete.

2026-10-11: Another 100 deduplicated local pipeline inputs are qualified: 12
canonical rectilinear text font-size states, 64 U16/U32 nonlinear analytical
Normal-mask geometry states, and 24 HSL Alien Map states. Independent import,
native/source/cache preservation, registered edits and atomic refusals, strict
lint and wasm gates pass. Root verified 4,885 frozen source/reference/control
assets for this increment; controls are excluded from 2750. Twelve automated
Properties tests and UI strict lint pass, including text size and HSL controls.
Current text layout bounds match the independently saved 14 paths; integrating
those bounds into selection and hit testing remains open. RGB/HSL switching,
general glyphs and colors, larger scenes and general filter extents remain
separate work. Existing tolerances, public 61/63/floor 61 and broad 8/131 remain
unchanged. Physical GUI acceptance waits while the desktop is busy; full XCF
import and native_large remain incomplete.

2026-10-11: A further 333 distinct local inputs qualify scoped editable text
color (32), letter spacing (26 new hashes from 36 saved states), generic point
filter geometry (129), U8 point storage (66), and medium analytical Normal-mask
scenes (64 U16/U32/F32/F64 plus 16 Half). Controls remain excluded; root checked
10,290 frozen assets for these packets and byte-deduplicated the Half inputs
against every other local XCF packet. Import, exact native/source preservation,
independent rendering, registered edits/current-state refusal, strict lint,
WASM and panic gates pass for these slices. Fourteen Properties tests and the
actual larger-scene canvas/damage test pass. Current text selection bounds and
glyph picking now follow regenerated layout without rewriting the original
cache; canonical text raster transforms remain unavailable. Latest offscreen
snapshots show the text controls and medium F64 scene after preserving its
embedded profile. These screenshots do not qualify physical desktop acceptance.
The stricter Half RGB cross-process repeat diagnostic remains failed and frozen;
passing existing acceptance bounds do not waive it. No tolerance/oracle changes.
The public corpus stays 61/63 (floor 61), broad task completion 8/131, and full
import/native_large remain incomplete. Latest shared native, all-crate WASM,
public corpus and performance gates are rebuilding after the requested cleanup.
Publication remains deferred. No commit or push was made.

### 2026-10-11 — 3215 distinct inputs; Half/U32 point Replace qualified

Another132 distinct inputs qualify the measured generic point Replace domain,
bringing the unpublished local total to3215. Root verified all3002 frozen packet
assets and independently deduplicated132 selected XCF SHA256 hashes against7367
other sibling XCF files: no collision. The44 author intermediates are excluded.
Actual import/native legacy+ZIP64/source/live projection passes132 states,
worst0.245704/255 under the unchanged2/255 acceptance bound. Registered88 edits,
22 current-F32 controls and528 hostile/current/lock variants pass. Half originals
widen exactly to current F32; retained Half provenance does not quantize current
edits. Current U32 storage rounds before display transfer; its measured Gray
temperature path clamps RGB before luminance. Original alpha/native bits and
disabled controls remain exact. Algo338/compose254 library tests, prior129+66
geometry/U8 references, strict all-targets4crates, WASM and panic hunt pass.
Evidence: temp/sol3-xcf-probes/point-replace-half-u32-final-receipt.json,
temp/root-point-half-u32-integrity-receipt.json and
temp/root-point-half-u32-independent-dedup.json. Earlier receipts recorded these
as unqualified candidates and are retained unchanged.

Zero-spacing UI now exposes0/2/4 and independent transitions pass;15 ignored
Properties corpus tests, strict and visually inspected Vulkan offscreen snapshot
pass. Current text placement has compatible None defaults, bounded coordinates
and native reader floor4 only when an explicit current origin is present. Doc2
and native4 meaningful RED/GREEN tests, full doc+format257 active tests, strict
and WASM pass. Rendering/picking/UI placement qualification remains pending,
so its8 new candidate source files are excluded from3215.

The plain Normal native-preparation HDR regression is reproduced and fixed;
10 contexts/20 frozen preview hashes match the previous path, native18 and prior
88 preview hashes remain exact. Linear/Perceptual preparation adds no new source
count:116 contexts/232 frozen previews and20 small current contexts pass actual
native writer checks, final worker gates are still running.

Public61/63/floor61, broad tasks8/131 and native_large=false remain unchanged.
General text, assigned-profile point filters, stock union/resource recovery and
physical GUI acceptance remain open. Shared full public corpus/performance gates
are rebuilding; those are not passes. The stricter Half RGB repeat diagnostic
remains failed and frozen. No tolerance, reference, publication, commit or push
changes. Cleanup reclaimed about800GiB; active targets stay lean with incremental
and debug data disabled. Desktop control remains on hold.

### 2026-10-11 — Text placement and Perceptual point filters qualified

The unpublished local total is 3421 distinct source/edit files: 8 independently
measured moved text originals and 198 six-precision Perceptual point Replace
states add to the previous 3215. Author intermediates and duplicate controls
are excluded. Root verified all 4498 Perceptual packet assets and independently
compared the 198 selected byte hashes against 7723 other XCF files, without a
collision. The earlier candidate receipts remain unchanged.

Text originals preserve their cache, raw configuration, bounds and resources;
current placement is separate and requires native reader 4 when explicitly
stored. Registered placement tests, 16 Properties tests, 2 current picking/frame
tests, 5 native placement tests, strict lint, WASM and a visually inspected Vulkan
offscreen snapshot pass. The snapshot shows red text at (11,2), matching the
Properties position values and current frame. Legacy modified-pixel text with
no override stays compatible; an unqualified modified-pixel placement is rejected.
References remain exact without AA and within 1/255 with AA.

The 198 Perceptual filter states pass actual import, exact native/source state,
legacy and ZIP64 reopen, and live projection (worst 0.245704/255 under the
unchanged 2/255 bound). Registered edits, current Half-as-F32 controls and atomic
hostile/lock/cancellation cases pass. Algo 338 and compose 263 library tests,
327 prior import references, strict, WASM and panic hunt pass. The first actual
Gray U32 Perceptual failure stdout was overwritten by its successful rerun;
the final receipt discloses this limitation. Separate Unsupported RED logs and
independent frozen ordering analysis remain. No failed stdout was recreated.

Exposure and Box native-preview bridges pass with frozen native state and both
preview PNGs unchanged at 16/48 MiB. Box uses its real 32 MiB transient tile
cache under the same RAII source/preparation/output budget. Cancellation,
concurrent denial and complete refund tests pass. These bridges add no source
count and do not establish large-document or total-process-memory acceptance.

Receipts: temp/sol3-xcf-probes/point-replace-perceptual-final-receipt.json,
temp/root-point-perceptual-independent-dedup.json,
temp/root-text-placement-independent-dedup.json, placement root/worker logs,
temp/sol2-native-preparation/box-before/MIGRATION-RECEIPT.md and
temp/sol4-plain-box-final-snapshot.json.

Public coverage remains 61/63 (floor 61), broad tasks 8/131 and native_large=false.
Multiline evidence and a bounded kernel are prepared, but canonical importing,
editing and UI admission remain pending and its 24 candidates are uncounted.
Full XCF support, general fonts/layouts, stock operation union, resource recovery,
physical GUI acceptance and final shared corpus/performance gates remain open.
The stricter Half repeat diagnostic stays failed and frozen. No tolerances,
references, publication, commit or push changed. Cleanup reclaimed 799.86 GiB;
rebuilds use one lean target per active worker. Desktop control remains on hold.

### 2026-10-11 — Multiline text and Linear Invert qualification

The unpublished local total is3553 distinct source/edit inputs:24 bounded
exact-font two-line text originals and108 six-precision/three-transfer Linear
Invert states add to3421. Root verified frozen272/2459-asset packets and selected
byte hashes independently against8107/7987 other XCF files with no collisions.
Author intermediates and duplicate controls remain excluded.

Multiline actual importing preserves original caches, raster bounds, resources
and whole native documents. Additional line spacing0/2/4 and two-line content
edits use registered commands and undo/redo; zero spacing removes the original
field exactly. Text106, engine27, Properties18, strict lint,WASM and panic hunt
pass. The offscreen preview visually shows HE/FT on separate lines, line spacing
4px and the current selection frame. The first content UI check queried a
single-line AccessKit role; the unchanged real multiline control passes with
the corrected MultilineTextInput query. Genuine missing-line-spacing UI RED is
preserved separately. General curved glyphs/fonts/layouts remain unqualified.

Linear Invert108 actual imports preserve source/native/legacy/ZIP64 state and
live projection (worst0.0621851/255 under unchanged2/255). Registered72 controls,
432 current-state negatives,144 hostile params,36 cancellations,864 consumer
negatives and prior525 references pass. Algo338/compose278, four-crate strict,
WASM and panic hunt pass; root verified all9 final log hashes.

Motion,Median and Gaussian native-preview budget bridges preserve frozen
source/canonical/native documents and both PNGs. Real transient-cache lease,
concurrent denial,cancellation and refunds pass. Gaussian's captured96x80 pair
saves at48MiB. These bridges add zero source inputs and establish no whole-process
RSS or large-document acceptance. Gamma Invert108 is independently verified but
still uncounted pending runtime gates. Public61/63(floor61), broad8/131 and
native_large=false remain unchanged; desktop acceptance and shared corpus/perf
gates remain open. No publication,commit,push or oracle tolerance changed.

Receipts: temp/sol1-xcf-text/multiline-core-receipt.md,
temp/root-multiline-independent-dedup.json,
temp/root-invert-linear-final-verification.json,
temp/sol3-xcf-probes/invert-linear-final-receipt.json,
temp/sol4-plain-gaussian-joint-final-snapshot.json.

### 2026-10-11 — Gamma Invert and Brightness preview bridge

The unpublished local total is3661 distinct source/edit inputs, adding108
independently deduplicated Gamma Invert states to3553. Root verified2459 frozen
artifacts and selected hashes against7987 other XCFs without collision, plus all
final worker log hashes. Actual import/native/legacy/ZIP64/source/mask state,
72 registered activation/native-pixel controls,432 current negatives,144 bad
params,36 valid canceled activations,864 consumer negatives and prior633
references pass. Live projection worst0.0218321/255 under unchanged2/255;
private native max0.0000627122/255. Algo338/compose283, strict,WASM andpanic pass.
Operation0:0 has zero arguments and its observed source schema and composition
triple remain separate. Assigned ICC and other unregistered zero-argument
operations are unsupported. Receipt: temp/sol3-xcf-probes/invert-gamma-final-receipt.json.

Brightness/Contrast captured native pair saves at16MiB with exact source/native
state and allfourPNGs. Valid current contrast1.25 preserves its frozen32MiB
legacy route and16MiB denial. Atomic errors, retained-first-output/second-float
cancellation, concurrent denial/refunds, prior/native/plain tests, strict and
WASM pass. This addszero inputs and no generic/native_large/RSS claim. Joint
receipt: temp/sol4-plain-brightness-joint-final-snapshot.json.

Full UI gate passes1131 tests,0 failures,26 ignored in36.33s with Vulkan
and two test threads. All14 catalogs include870 translated XCF-control rows,
and the1019-literal coverage audit passes. The initial missing-label/device-loss
and intermediate Russian-label failure logs remain preserved. Focused18 XCF
Properties tests and visually inspected multiline/current frame also pass.
Broader native/WASM/corpus/perf gates remain pending. Public61/63(floor61),
broad8/131 andnative_large=false unchanged; desktop remainsbusy, no
publication/commit/push or tolerance change.

### 2026-10-11 — Pinned Linear Invert and import foundations

The public pinned XCF corpus now passes 62 of 63 projects, with one explicit
rejection and a supported floor of 62. The previously unsupported GEGL-effect
project has an independently qualified Linear Invert operation and a zero mask.
Its original native samples and full legacy/ZIP64 native documents are exact;
ICC-matched GIMP display error is 0.00078149/255 under the existing 2/255 bound.
The pinned files, reference images and oracle bounds are unchanged.

A native-save regression is fixed for ordinary 33³ Color Lookup tables: native
preparation retains the immutable shared lookup instead of incorrectly applying
the small adjustment-metadata JSON limit. The focused regression, existing engine
round trip, complete format tests, strict lint and format WASM checks pass.

The additive staged source route passes an identical 2.4 MB budget where the eager
plane-plus-surface route fails, and 13 eligible RGB/Gray U8 projects match eager
import, native reopen and rendering. Default loading is unchanged. Source leases,
handoff and late cancellation remain under qualification; this does not enable
native_large. Independent normal-release 24 MP U16/F32 eager measurements now
include external process working-set observations, separate from component
reservation counters. A same-input staged comparison is in progress.

2026-10-11: Another 306 independently deduplicated inputs qualify: 144 Color
Exchange, 144 Color-to-Alpha and 18 exact-font rectilinear underline states. Their
actual import, independent rendering, registered edits/history and native legacy
and ZIP64 reopen checks pass. Color-to-Alpha preserves current Half output
quantization explicitly; an explicit Float-depth edit removes that quantization
without changing immutable source provenance. Half-to-Float reference controls
and repeated captures are excluded from the input count.

Two Color Exchange, four Color-to-Alpha and four underline Properties tests pass;
the complete UI library passes 1140 tests with no failures and 32 ignored. Corrected
offscreen previews show underline and transparency controls. The snapshot harness
now uses desktop texture sampling for the repeating checkerboard. All 8192 native,
reduced and canvas-upload alpha samples match, and 2648 constant visual interiors
match the independent reference over the checkerboard exactly. This bounded
visual check does not replace the full native/render reference comparisons.

A further720 independently deduplicated originals qualify Component Extract Red,
Green, Blue and Alpha in the observed singleton8x6 implicit RGB/Gray epoch, across
six original precisions and three transfers. Registered edits/history, exact
Half promotion/removal, native legacy/ZIP64, strict, WASM, panic and scoped UI
controls pass. The full UI replay passes1142 tests with0 failures and33 ignored.
Seventeen other Component selectors remain unqualified. The unpublished qualified
total is4813; public support remains62/63 with floor62 and broad tasks remain8/131.
Another126 unique originals qualify ordered unit-domain Alpha Clip across the
observed implicit RGB/Gray six-precision/three-transfer singleton8x6 scene.
Registered controls, native/history, strict/WASM/panic and Properties pass;
fullUI1144/0fail/34ignored and inspected offscreen controls pass. Six promotion
controls add no inputs. HDR/crossed/outside/general-owner domains remain open.
Owned native backing remains an additive foundation and adds no corpus inputs. General text/fonts,
arbitrary stock filters and native_large remain incomplete. The current broad shared corpus rerun passes2674 tests with no failures
(443 explicitly ignored); the repaired normal release quick performance rerun passes allfour benches. The existing sparse PSD memory cap now uses actual layer/mask extents, while canvas auxiliary planes retain their full charge. Cross-machine baseline comparison is skipped and quickmode does not apply full-size budgets. Physical desktop acceptance waits while the
desktop is busy.

2026-10-11: Root adopts1260 unique independently authored CMYK720/YCbCr540 Component originals after frozen core15 sourcefiles/14greenlogs/retaineddiagnostics and all31930packetassets verified against receipts. Latest wholeparent dedup0collisions vs11716otherXCFs. Independent original/native legacy+ZIP64/render,468registered edits,210promotion+210remove histories,720source/864current/432params/72cancel negatives, strict6/WASM6/panic/layers/pinnedcorpus and scoped24MP kernel gatesPASS. Actual UI fivecontrols/full1145PASS35ignored/strict/WASM,14catalogs42keys and both root-inspected offscreen snapshotsPASS via38artifact census. Native/render worstCMYK1.250212104437117/255,YCbCr.9999999701976758/255 under unchanged2/255 bound;12promotedYCbCr tiny repeatdeltas retained. Exactraw−305 triplet singleton8x6 implicitRGB/Gray sixprecisions/threetransfers only. Qualified unpublished4813+1260=6073. FiniteAlpha80/geometry132/polar1800 unqualified; public62/63floor62/broad8/131/NativeLarge disabled unchanged. FullComponent/fullXCF incomplete. Review temp/root-component-expanded-adoption-review.json.

2026-10-11: Root adopts80 unique finite Alpha Clip originals after historical pipeline33, tuple14, UI22 and ZIP64 supplement8 frozen artifacts plus7452 packet assets verified. Existing whole-parent dedup80 unique/zero collisions, exact source preservation,64 registered edits/history, original/current public streaming ZIP64 reopen, source/current refusals, strict/WASM and both root-viewed offscreen controls pass. Exact observed finite threshold epoch only, implicit RGB/Gray F32/F64 unit source/output; no HDR/nonunit-alpha/general-owner widening. Qualified unpublished6073+80=6153; geometry132/polar1800 remain unqualified. Public62/63floor62/broad8/131/NativeLarge disabled unchanged. Optional owned byte/archive19, accounted Writer23 and canonical object Writer join23 frozen artifacts reviewed; B014/B015 foundations complete, no standard Doc/native route or RSS claim. Review temp/root-foundation-adoption-review.json.

2026-10-11: Root adopts132 unique Component Extract geometry originals after frozen core and21 UI artifacts verified. Two exact observed RGB/Gray F32 nonlinear singleton Replace extent/offset classes, selectors0/1/2/8..14/20; exact source/current native legacy+ZIP64/live,88 registered reference endpoints, hostile/cancel controls and52 offscreen Properties state verifications pass. UI1147PASS/37ignored, prior3 controls, strict/WASM and inspected isolated Properties panels pass. Qualified unpublished6153+132=6285. General geometry, HSV/HSL/LabLCh selectors, whole-app desktop acceptance, native-backed Document migration and latest broad corpus rerun remain open. Public62/63floor62 and NativeLarge disabled unchanged. Review temp/root-component-geometry-adoption-review.json.

2026-10-11: Root adopts360 Color Overlay and144 signed Mono Mixer originals after core and28 jointUI artifacts verified;6285+504=6789 qualified unpublished inputs. Exact implicitRGB/Gray sixdepth/threeencoding8x6 singleton Replace scopes; source/current/native legacy+ZIP64/live,registered edits/negative/history/locks pass. Actual publicnativeColorOverlay route fixed bounded literal census; Mono Double focus/Escape exact-preservation bug fixed. UI1150PASS43ignored/prior3/strict/WASM and5 inspected isolated Properties captures pass. Generalgeometry/profiles/HDR, HSV/HSL/LabLCh transfer, fullinventory, broad release finalchain, backedDocument/native_large and physical desktop acceptance remain open. Publication/pins unchanged.


### 2026-10-12 actual web asset integration checkpoint

The declared asset cohort is integrated into the actual source: 69 writes,
5 skips and 77 backups, with no font payload or generated pack committed.
A fresh ordinary HEIF web build is 25,074,009 bytes, SHA256
`65aead58d7bad0530023c7732aabc2f41eeb4ddcba5e275f6fab5a696cf0a10b`:
91,815 bytes below the unchanged 24 MiB limit. The canonical 4,001,124-byte pack
passed 21 range/nine license checks; the normal build has no diagnostic markers.
[Actual package receipt](../temp/sol4-actual-web-build-20261012-v2/actual-normal-bundle-receipt.json).

Pre-format actual native UI passed 1,255 tests/63 ignored and strict (48.44 s), with
legible CPU offscreen output. Pre-format actual engine passed 1,220/20 ignored,
corpus all-target strict and a separate ignored panic-hunt. Actual normal browser
picker Open → Save As → reopen retained whole-byte native identity `1b814e36acd579b97974f4d8b291cc540ea349c7ba7f3037e5f142cb24a06d1d`.
[UI receipt](../temp/root-actual-web-assets-ui-receipt-v1.json),
[engine receipt](../temp/sol2-actual-integration-engine-20261012/receipt-20261012.json),
[actual browser receipt](../temp/root-postformat-web-assets-browser-20261012/receipt.json).
These are separate measured scopes, not whole GUI or physical memory acceptance.

Sealed [V14 local corpus](../temp/sol6-official-corpus-result-v14.json) passed
349 suites/2,777 tests with 546 explicit ignores, with source/pins unchanged.
SHA256 `577a6c879342f177870b5396a6330a8f386e0e8f088b61d2671740d09522f6ea`.
V14 is the integrated pre-format cohort. Six mechanical formatting changes are
now applied: limited 54-file format and xtask 87 passed/2 ignored plus strict are
GREEN; stale generated metadata was refused before preservation/quarantine and
fresh prepare/check/21-range/nine-license verification. Pack bytes are unchanged;
only trusted descriptor Rust text changed. Post-format compilation, focused native
followup and refreshed package/browser receipts are GREEN in their declared
scopes. [Focused native receipt](../temp/sol1-format-followup-gates-20261012/root-final-receipt.json).
Historical V13 and isolated prototype results do not qualify the integrated tree.
Full XCF/all nine release epochs, native_large, full T120/T121, physical RSS,
cache denial and publication remain open. Qualified sources 6,789 and parent
markers 12/131 are unchanged. Detailed receipt hashes and diagnostic limits:
[`resume-20261012.md`](../specs/002-full-xcf-import/evidence/resume-20261012.md).
