# PSD loading performance work

Tracking issue: https://github.com/storytold/photocraft/issues/5392.

Implementation proceeds sequentially on one branch. Keep each measured checkpoint so that
later parser, upload and composition changes can be compared against the immediately prior
version. Generated PSDs and images are local fixtures, not committed assets.

## Validated checkpoints

The Linux experiment uses immutable 7360×4912 three-layer and complex eight-layer PSDs
created from an original generated 1536×1024 image, scaled deterministically. The three-layer
file's BLAKE3 is `497008838e0cd18506e4b2e0cf533ff001f53fe22d1bbc1a9aab41d90df8de0b`.

Ryzen 9 7950X3D / RTX 3090, Vulkan, Rust 1.99, release opt-level 3, LTO off,
16 codegen units, `-C target-cpu=native` for both reference and candidate. The fixed-file
harness times in-memory import, Session admission, full-canvas GPU encode/submit and wait.
Disk, GPU initialization, compositor construction, readback and validation are excluded.
The compositor is fresh per sample. Alternate and reverse version order; keep raw samples.
These are P44-class workloads, not the original CI P44 file or native UI startup measurements.

Historical fork-based evidence: the prior import prototype reduced measured three-layer opening from 621 to 250 ms and
complex opening from 287 to 136 ms. That historical comparison is separate from subsequent
adjacent comparisons; do not combine batches to attribute a gain to a single change.

| Sequential step | Three-layer reference → candidate | Evidence |
|---|---:|---|
| Borrow native tile bytes during GPU upload | 251.3 → 236.0 ms | `perf/experiments/psd-open-linux-step1.json` |

The direct upload step removes temporary copies for native RGBA8/GRAY8 formats while
preserving shared-tile grouping and ordered uploads. CPU preparation changed from 87.3 to
71.1 ms in the paired three-layer run. The complex run was noisy (136.0 → 134.8 ms overall);
no precise complex speedup is claimed for this step.

## Correctness

The retained import prototype passed 35 PSD compression tests and three focused import
integration tests, including malformed channels, deterministic warning order, PSD/PSB,
8/16/32-bit samples, negative bounds, masks and multi-depth round trips. Complete imported
raster/mask/structure signatures match both fixed fixtures. Four representative GPU crops
per fixture pass the CPU oracle within one 8-bit level plus floating-point tolerance; this
is not full-frame GPU bitwise equality. The direct-upload shared-tile/multi-format unit test
also passed on Linux.

The parallel RLE path validates packets in original row order before initializing the full
output; removing that validation caused a malformed-file memory regression in a rejected
trial. Worker-count tuning and fixed packing blocks were rejected because they did not
produce repeatable useful improvements.

## Remaining work

The umbrella issue tracks the remaining mask, sample-format, mip, sparse first-view and lazy
materialization work. Parser borrowing, merged-plane reuse, native upload borrowing, dense
pass fusion and first-open cache deferral have validated checkpoints below. GPU Navigator
reuse already exists upstream. Final lint, wasm and native UI acceptance are pending.

## Upstream checkpoint

Upstream implementation PR: https://github.com/storytold/photocraft/pull/5396, based directly
on upstream main `583425b4df1c57a4515f591fed64fcedf8440cba`. The initially opened fork
PR was closed; unrelated fork changes are excluded.

The upstream control measured 504.0→197.4 ms (three layers) and 216.3→113.3 ms (complex).
See `perf/experiments/psd-open-linux-upstream-total.json`. Borrowed parsing alone in an
adjacent comparison measured 235.2→199.4 ms, with import 119.3→83.5 ms; complex
129.9→112.4 ms. See `perf/experiments/psd-open-linux-borrowed.json`. These pooled medians
use six warm samples in opposite-order batches; phase medians need not sum exactly.

All 211 PSD unit tests and the three import-equivalence tests passed on the upstream
checkpoint. The oracle now includes a distinct parent group mask. Its density uses an exact
PSD-representable 8-bit value, preserving the strict source-versus-import comparison.
Both fixture signatures and representative GPU crops remain valid. Borrowed metadata omits
encoded buffers deliberately; decoding uses PsdView and roundtrip uses to_owned. The owned
PsdFile API and upstream depth-specific global-layer precedence are preserved.

Current upstream already has GPU Navigator reuse (#4518), including initial GPU waiting;
that item needs no duplicate implementation here. Corpus-enabled PSD/IO suites pass; final lint/wasm/UI gates remain pending.

## Raster pass fusion

Dense raster layers without clipping units now combine content/mask sampling and final
blend in one pass. Sparse layers, effects, groups and clipping keep their prior paths.
Full-float accumulation uses the fused path; half-float accumulation deliberately preserves
separate passes and their intermediate rounding without requiring optional shader features.

Adjacent Linux comparison: three-layer 198.4 → 193.0 ms; complex 113.0 → 111.4 ms.
The dense fixture changes from 7 to 4 planned passes with unchanged source uploads. See
`perf/experiments/psd-open-linux-fusion.json`. All GPU unit/integration tests pass, including
fused/unfused masks/blends/depths and half-float fallback. The submission-pressure regression
uses 1,100 layers so that the new fused plan still crosses the 1,000 operation submission limit;
CPU parity, one-chunk and forced-flush assertions are retained.

The PSD and IO packages also pass their corpus-enabled suites on the borrowed-import
checkpoint: 401 PSD and 590 IO tests, no failures.

## First-open cache admission

Fresh supported GPU canvases request a one-shot deferral of lower-stack cache population.
The default compositor behavior and cache budgets remain unchanged; subsequent renders
can populate/reuse the cache. Empty/failed preflight and discardable encode calls do not
consume the request. Stale cache entries are cleared on the deferred render.

End-to-end adjacent comparison, including import: three-layer 190.9 → 181.0 ms;
complex 109.7 → 101.4 ms. See `perf/experiments/psd-open-linux-cache-endtoend.json`.
A separate GPU-only opening-plus-two-64×64-edits experiment measured opening 142.7 → 82.2 ms,
first edit 1.60 → 2.01 ms, second edit 0.45 → 1.22 ms, total 144.8 → 85.4 ms.
That experiment uses separate conditioning and excludes import/admission; its absolute
times and savings must not be added to the end-to-end comparison. Both policies receive
excluded warmup pairs and six alternating measured pairs. See
`perf/experiments/psd-open-linux-cache-policy.json` for all samples and CPU checks.

Initial deferred cache bytes are 0 rather than 512 MiB; after the first small edit the cache
holds 16 MiB and the second edit reuses it. All GPU tests pass (98 cases), including
opening/stale-cache/discarded-encoder/partial-edit regressions across depths. Linux
`cargo check -p photocraft-ui-egui` passed; native visible UI acceptance remains pending.
The full opening harness's `cache-defer` mode requires `--features deferred-cache` and
fresh compositors. Older reference builds remain compatible without that feature.
