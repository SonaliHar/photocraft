# photocraft-compose

The CPU reference compositor renders PhotoCraft documents. The existing `render`
API produces a derived f32 preview with Photoshop semantics.

Ordinary rendering, reduction, thumbnails, layer shapes, and proxy construction
return checked `Result`s. They reject hidden or nested fallible backing before
producing output, including empty rectangles. `validate_resident_document` offers
the same storage/structure guard without rendering. `RenderBandError` distinguishes
render failures from caller sink failures.

`backed_document::PreparedBackedDocument` consumes the actual document model through
borrowed, quota-owned native storage. Its initial supported path is a flat RGB/Gray
Normal stack with explicit current ICC bytes and sample encoding, common persistent
quota ownership, finite native extents, exact defaults, and COW edits. Bounded f64
bands retain their quota charge until dropped; wide regions stream in columns.
This initial stack admits finite unit colors and alpha; HDR inputs are typed errors.
`render_reduced` streams canvas bands into a charged f64 premultiplied accumulator
and returns charged f32 straight-alpha samples in the explicit current encoding.
Its axes are bounded to 4096; it never creates a full source buffer. Apply the
current ICC interpretation when displaying the derived samples. `into_parts`
transfers the buffer and lease together; a persistent recipient retains the lease.
Unsupported masks, groups, filters, or other operative semantics are errors before
pixel reads. Whole-image sinks must stage publication until streaming succeeds.
This optional path does not enable standard large-document import or native loading.

`gimp_render::try_render` is a checked authoritative f64 document consumer for the
currently implemented GIMP composition slice. It supports RGB/Gray raster layers,
default pixels, simple isolated/pass-through groups, raster masks, Normal mode,
and resolved Union/ClipToBackdrop/ClipToLayer/Intersection operators. It prepares
profile-defined, linear, perceptual, or Lab working spaces once, applies coverage
independently of color transfer, and retains U32 low bits and finite HDR samples.
`WideBuffer::try_display` explicitly derives an f32 buffer and rejects overflow.
`PreparedRenderer` reuses the qualified tree and spaces across bands. Its
`render_reduced` method composes and averages source pixels in f64, then emits only
the derived f32 output pixels, including damage rectangles. The imported RGB/Gray
canvas and Navigator use this path, retain their previous texture on an error, and
report the failure through the existing status area.

Ordered Raster point stacks evaluate the exact stock exposure `1:0`,
brightness-contrast `0:0`, invert-linear `0:0`, and invert-gamma `0:0` members
through checked f64 working-encoding conversions and fixed pixel scratch. Source
surfaces and disabled nodes remain unchanged. Enabled nodes require the observed
v022 Replace 62 / opacity 1 / Adjust 0 envelope, all three raw policy fields
`-261651887` unresolved, exact f32 arguments, qualified empty-selection masks or explicit absence, and no auxiliary input.
Primitive GEGL Levels `0:0` is also supported with four finite f32 scalar
endpoints, increasing input range, and no gamma or implicit clipping. Its two
original source/edit references pass with worst RGBA error 0.462943/255.
Those raw fields remain stored as received; this is a scoped compatibility
witness, not normalization or a universal interpretation of malformed fields.
Selection region 0 and exact empty Gray/U8 saved effect masks are qualified:
mask ids, FilterId ownership, declared extents, native samples/defaults and
metadata remain canonical state. Nonempty or inconsistent masks fail;
absence remains explicit. Exposure black-level is restricted to zero. Other numeric domains follow the
checked algorithm registry. The eight independently authored opaque RGB/U8
implicit-sRGB source/edit references pass the actual document consumer, with
worst full RGBA error 0.500741/255. Gray/HDR/profile mechanics have synthetic
checks; their broader empirical import admission remains separate.

Structured `gimp:levels 0:0` evaluates five ordered exact f64 Value/R/G/B/A
configurations, explicit input/output clamps, and Linear TRC 0 or Nonlinear
TRC 1. The qualified originals use RGB/F32 implicit sRGB, canvas/owner extent
12ï¿½9, v022 Replace 62 / opacity1 / Adjust0 / selection0, and raw policy triple
`-1080065455`. Exact empty Gray/F32 selection masks remain canonical native
state. All 16 source/edit/TRC/clamp references pass with worst full RGBA error
0.001929/255 after explicit source-effective-profile to PNG-profile conversion.
Unclamped selectors produce finite negative/>1 Alpha; the observed stock
projection uses its clipped zero/one coverage while retaining unclamped HDR
color. A stack containing another enabled filter is rejected until intermediate
Alpha semantics are independently qualified. Perceptual TRC, non-default
assigned profiles and other precision/extents remain unsupported. Source Alpha
must be finite unit coverage before evaluation. A streamed 24 MP release seven-
gamma checked Levels kernel measured 4.793 s, excluding surfaces, CMS
preparation and I/O.

The stored FREE256 `gimp:curves 0:0` stage requires exact v022 schema epoch
`xcf-22-stock-curves-replace`, Nonlinear TRC 1, five ordered value/red/green/blue/
alpha tables, empty point geometry, and distinct raw policy fields `-1080065455`.
Its admission domain is implicit-sRGB RGB/U8 Nonlinear, canvas and owner extent
12ï¿½9, with the qualified empty selection mask. The five tables are validated
once and retained as 10 KiB of immutable derived numeric state per filter;
source f64 bits and native source pixels remain canonical. Original source/edit
references pass with worst RGBA error 0.492453/255. Tests check explicit Alpha,
ordered point evaluation, malformed tables/schema, cancellation and balanced
output reservation. SMOOTH regeneration and broader profile/precision/extents
are unsupported; preparation payload still requires outer import-job accounting. A 24 MP release
streamed prepared five-table kernel measured 951.114 ms; this excludes surfaces,
renderer composition, preparation and I/O.

The ordered neighborhood path includes the independently authored Gaussian
`0:0` source/edit slice: opaque implicit-sRGB RGB/U8, exact owner extent
`[0,0,96,80]`, Auto 0, Clamp 1, clip-extent true, f32 std-dev-x 2.25 or 3.75 and
std-dev-y 1.5. Its distinct v022 Replace envelope retains raw policy fields
`-856260015`. Both full/banded references pass, worst RGBA error 0.500508/255.
Point stages before and after neighborhood evaluation preserve order; synthetic
checks compare them against independent stage constructions. Broader Gaussian
alpha/profile/extent/enum/parameter domains remain unsupported.

Intermediate f64 halo surfaces and a seven-entry derived tile cache belong only
to a renderer invocation. A conservative 32 MiB cache/scratch reservation occurs
before allocation, is released on success/failure, and the cache is discarded.
Neighborhood stage depth is capped at 32; individual kernel resident/work limits
are checked. `render_with_interrupt` and `render_reduced_with_interrupt` propagate
a job cancellation/deadline token into the Gaussian kernel, in addition to work
callback checkpoints. Tests cover cancellation during a kernel, reservation
denial, balanced releases, exact source/native DTO preservation, and invalid
mask/profile/extent/domain rejection. This is derived live evaluation, never a
stored cache substituted for editable source state.

Unsupported blend modes, unresolved layer policies, other live filters, editable source
regeneration, floating attachments, vector masks, artboards, clipping, advanced
effects, and mask feathering return errors. This slice does not establish complete
GIMP import/composition parity. Newly admitted LUT profiles must use the checked
consumer: profile-defined encoding is supported, while unsupported separable
transfer overrides fail before evaluation. Legacy `render` does not provide this
error contract.

The caller supplies `WideWork` accounting/cancellation checkpoints. Output
reservation is admitted before allocation and released on failure; successful
returned-buffer reservation stays with the caller until that buffer is dropped.
Current checkpoints use conservative fixed per-layer and per-filter pixel charges. Preparation
allocations and exact per-kernel work accounting require further integration with
the import job budget. Rendering is bounded by raster resident/side limits and
document tree depth.

Hash-bound original offset-mask/operator source/edit pairs additionally qualify
mode 30 explicit Union/ClipToBackdrop/ClipToLayer/Intersection within their
implicit-profile RGB/F32 Nonlinear 16ï¿½12 domain. Their Gray/F32 mask planes are
10ï¿½9 at owner source offset (2,-1) or edit offset (-1,2), enabled and unshown.
Actual stored top fields are -1 / -1 / operators 1ï¿½4; backdrop fields are
0 / -1 / -1. The raw mask source offset is (0,0); its surface is placed at the
owner's global bounds, preserving both pieces of state separately.
Worst full RGBA error is 0.002607/255. Layer-mask metadata is prepared once and
excludes saved effect-mask ids; a canonical regression verifies independent
bounds/defaults for both mask classes on the same layer. No authored unequal-
extent mask or group-filter/mask qualification is established by these cases.

Modern Multiply mode 30 evaluates multiplication in its resolved blend space
and coverage in its independently resolved composite space. The hash-bound v012
implicit-profile RGB/F32 Nonlinear source/edit pair qualifies Linear blend / Linear composite / ClipToBackdrop within that authored
16ï¿½12 two-layer domain. The public GIMP observer reports Auto 0; actual parsed
stored top fields are blend -1 / composite -1 / operator -2, and backdrop fields
0 / -1 / -1. The importer preserves those signed source values separately from
resolved policy. Worst full RGBA error is 0.002071/255 (less than one
16-bit reference code). Synthetic tests cover all four coverage equations, finite
HDR, U32-sized low bits and rejection of nonfinite/overflow. Legacy Multiply,
Lab blend and broader source epochs/domains remain unsupported. A streamed
24 MP release kernel benchmark measured checked f64 918.734 ms versus legacy
f32 379.168 ms (2.423ï¿½); these paths have different coverage semantics and this
measures arithmetic throughput, excluding surfaces, CMS preparation, UI and I/O.

Normal overlap/source-over equations follow the public
[GIMP compositing description](https://developer.gimp.org/core/algorithm/compositing/).
Coverage operators select the regions described in the
[GIMP layer composite mode manual](https://docs.gimp.org/3.0/en/gimp-layer-new.html#gimp-layer-composite-mode).
Pass-through groups currently admit Union only; other group operators require
independent qualification rather than assuming isolated-layer semantics.

Tests in `tests/gimp_wide.rs` exercise actual document evaluation and independent
coverage equations. They complement the external GIMP corpus oracles and do not
replace source-rendered acceptance evidence.

The opt-in `corpus` tests bind original XCF, metadata, native-plane and reference
hashes. Set `PHOTOCRAFT_XCF_FULL_CORPUS` to the absolute verified `xcf/full`
directory and run `cargo test -p photocraft-compose --features corpus`. Missing
fixtures fail. `gimp_normal_original` qualifies the two Normal/Auto source/edit
pairs (modern linear/Union and legacy profile/Union); its fixture bottom layer is
modern Normal in both cases. This does not qualify every explicit operator/space
or universal Auto resolution. Publication and pinning remain separate gates.

Release timing on 2026-10-10: 24 MP streamed Normal/source-over kernels took
148.36 ms for the legacy f32 kernel and 563.71 ms for the checked f64 kernel
(3.80Ã—). This excludes document reads, preparation, output allocation, and display
conversion; it is not an end-to-end import/render budget measurement.

The 24 MP release encoded-sRGB exposure point-stack kernel measured 5.150 s
including checked f64 nonlinear/linear transfer in both directions. It uses
fixed pixel scratch and excludes Surface reads, composition, output allocation,
and display conversion. This is a throughput baseline, not a met render budget;
parallel/banded scheduling and exact aggregate job accounting remain open.

Modern basic blend dispatch additionally qualifies Screen (31), Difference (32),
Addition (33), and Subtract (34) through eight hash-bound source/edit references
in `gimp_basic_blends_original`. The exact witness domain is implicit-sRGB RGB
F32 nonlinear, a 16x12 canvas, two raster layers without masks/filters/parents:
foreground 10x9 at (2,-1) source or (-1,2) edit and backdrop 16x12 at (0,0).
Screen/Difference XCF v23 store blend/composite/operator -4/-1/-2, resolved as
Perceptual/Linear/ClipToBackdrop; Addition/Subtract v12 store -1/-1/-2,
resolved as Linear/Linear/ClipToBackdrop. The bottom Normal28 layer stores
0/-1/-1. Fresh public observations report Auto zeros; those are not the stored
signed values. The [public XCF specification](https://developer.gimp.org/core/standards/xcf/)
explicitly defines negative *in-domain* values as the saved Auto mapping.
Malformed out-of-domain effect codes retain unresolved semantics.

Equations follow the public [Lighten mode manual](https://docs.gimp.org/3.0/en/layer-mode-group-lighten.html)
and [Inversion mode manual](https://docs.gimp.org/3.0/en/layer-mode-group-inversion.html).
Modern Addition and Subtract retain HDR overlap values. Only the final PNG
comparison clamps the display derivative to the bounded output format; source
samples and renderer results remain unchanged. All eight RGBA projections are
within one 16-bit code, worst 0.003493/255. Synthetic checks cover signed HDR,
U32-sized low bits, independent coverage, nonfinite/overflow errors, and legacy
mode rejection. Other source epochs, custom profiles, masks/groups and explicit
space combinations are not qualified by these witnesses.

Release arithmetic-only timing on 2026-10-10 for 24 MP in linear f64:
Screen 1.266 s, Difference 1.171 s, Addition 1.370 s, Subtract 1.459 s.
The streamed benchmark excludes surfaces, CMS preparation/transfer, display
conversion, output allocation, UI and I/O; it is not a render budget result.

Exclusion (52) and modern Overlay (23) have also passed four hash-bound source/edit
RGBA references within the same two-layer RGB F32 nonlinear witness domain.
Both XCF v23 top layers store -4/-1/-2 (Perceptual/Linear/ClipToBackdrop).
Worst deviations are Exclusion 0.002606/255 and Overlay 0.001871/255.
Exclusion follows [W3C Compositing and Blending](https://www.w3.org/TR/compositing-1/#blendingexclusion);
Overlay follows the [public GIMP Contrast manual](https://docs.gimp.org/3.0/en/layer-mode-group-contrast.html).
These additional witnesses do not admit legacy Overlay, all explicit spaces,
custom profiles, group/mask combinations, or unverified HDR GIMP projections.

Grain Merge (47) and Linear Light (50) additionally pass four original source/edit
RGBA references in the same v23 Perceptual/Linear/ClipToBackdrop witness domain,
worst 0.002557/255 and 0.002461/255 respectively. Their equations follow the
public Inversion and Contrast manuals above. Grain Merge's operative source
mode lives in `GimpComposition`; there is no matching native BlendMode variant,
so broader editing/legacy compositor compatibility remains a separate gate.
Grain Extract (46) now passes its two original source/edit references with the
bound analytical implicit profile interpretation described below, worst
0.001924/255. Like Grain Merge, its importer admission remains pending an honest
native blend label and edit/export semantics; the checked kernel proof alone
does not clear those gates. No comparison threshold was loosened or projection
substituted for live data.

Modern Hard Light (44) passes both original v23 source/edit references in the
same -4/-1/-2 Perceptual/Linear/ClipToBackdrop domain, worst 0.001782/255.
Its checked branch equations follow the public Contrast manual and
[W3C Hard Light equation](https://www.w3.org/TR/compositing-1/#blendinghardlight).
The blend input domain is explicitly finite [0,1]; signed/HDR blend inputs fail
until independently qualified. This domain check affects the private evaluated
blend samples only and never rewrites the immutable original raster.

The final nine-mode 24 MP linear arithmetic benchmark (same exclusions) measured:
Screen 1.197 s, Difference 1.223 s, Addition 1.257 s, Subtract 1.324 s,
Exclusion 1.423 s, Overlay 1.376 s, Grain Merge 1.390 s,
Linear Light 1.417 s, Hard Light 1.587 s. This includes Hard Light's checked
unit-domain guard. Grain Merge timing does not imply importer admission.

Darken Only (35) and Lighten Only (36) additionally pass four hash-bound original
source/edit RGBA comparisons within the same implicit RGB F32 nonlinear,
16x12 two-raster witness domain. XCF v12 foregrounds store blend/composite/operator
0/-1/-2; the bottom Normal28 layer stores 0/-1/-1. Their public equations choose
component-wise min/max; Profile RGB is the deterministic resolved blend evaluation.
This is not a claim to observe a unique zero/Auto blend space: the authored test
proves Linear, Profile and Perceptual RGB transfer alternatives tie within 1e-12,
as expected for monotone component transfer. It separately rejects nonlinear
composition and Union coverage against original projections. Linear composite
and ClipToBackdrop match, worst Darken 0.002102/255 and Lighten 0.002460/255.
The Darken/Lighten manual links above define the component selection; no luma
selection, legacy source modes, custom profiles, masks or group admission follows.

The retained opt-in `gimp_grain_extract_diagnostic` identifies its strict gate
miss without changing admission or tolerance: analytic implicit sRGB passes both
source/edit references within one 16-bit code (worst 0.001924/255), whereas
reparsed serialized builtin ICC TRCs produce 0.004680/255 at a low-tone source
pixel. ICC s15Fixed16 curve serialization loses the implicit analytical working
transfer precision. Choosing immutable source context after active profile edits
would be incorrect. PreparedRenderer now validates the active
WorkingProfileInterpretation marker, model and BLAKE3 byte binding, then CMS
checks those bytes exactly match the generated implicit ICC for current sample
encoding before returning the analytical profile. Explicit Icc interpretation,
even with identical ICC bytes, retains parsed assigned semantics. Stale,
self-consistent arbitrary-profile and wrong-model markers fail without mutation.
All original compose witness documents now use explicit implicit markers;
structured Linear Levels uses its exact generated effective ICC binding.

24 MP release min/max arithmetic timing on 2026-10-10: Darken Only 1.118 s,
Lighten Only 1.123 s. This streamed linear f64 kernel timing excludes profile
preparation/transfer, surfaces, allocation, display, UI and I/O.


Modern Soft Light (45), Pin Light (49), Color Dodge (42), Color Burn (43),
Vivid Light (48), Divide (41), and Linear Burn (53) now pass fourteen hash-bound
source/edit full-RGBA authored references in `gimp_basic_blends_original`.
The largest difference is 0.002020/255, below one 16-bit reference code.
These witnesses retain the existing two-raster implicit analytical sRGB RGB/F32
nonlinear 16ï¿½12 envelope, foreground 10ï¿½9 with observed source/edit offsets
and opacities, and no masks, groups, assigned profiles, or filters. All except
Divide use v23 stored top blend/composite/operator -4/-1/-2, resolved to
Perceptual/Linear/ClipToBackdrop. Divide uses v12 -1/-1/-2 and
Linear/Linear/ClipToBackdrop. The backdrop Normal28 stores 0/-1/-1. The public
observer reports Auto zeros; these do not replace the exact stored fields.

The equations come from the public GIMP
[Contrast](https://docs.gimp.org/3.0/en/layer-mode-group-contrast.html),
[Lighten](https://docs.gimp.org/3.0/en/layer-mode-group-lighten.html),
[Darken](https://docs.gimp.org/3.0/en/layer-mode-group-darken.html), and
[Inversion](https://docs.gimp.org/3.0/en/layer-mode-group-inversion.html)
manuals. Soft Light uses the GIMP Multiply/Screen weighting, yielding
`b*b + 2*b*s*(1-b)`; its neutral source 0.5 retains the backdrop exactly.
Pin Light selects `min(b,2*s)` below the midpoint and `max(b,2*s-1)` above.
Dodge, Burn, Divide, and Linear Burn retain unbounded overlap values; applying
a bounded W3C Dodge clamp fails its original by 84.528/255. Vivid Light uses
its documented clipped Burn/Dodge branches. Final PNG quantization is a derived
display boundary and never mutates the native planes.

All seven private blend kernels currently require finite unit color inputs.
Dodge additionally requires source below 1, Burn and Divide source above 0,
and Vivid Light source strictly between 0 and 1. The authored overlaps do not
witness the singular endpoints; those return an explicit unsupported error.
Arithmetic overflow and nonfinite color/coverage return checked errors. The
synthetic branch tests cover both thresholds, unclipped outputs, neutral low
bits, unused zero-coverage singular colors, and division overflow. Broader
HDR input, profile, operator, mask, and endpoint domains remain unqualified.


24 MP release streamed arithmetic timing on 2026-10-10: Soft Light 1.302 s,
Pin Light 1.243 s, Color Dodge 1.487 s, Color Burn 1.537 s,
Vivid Light 1.572 s, Divide 1.586 s, Linear Burn 1.478 s. These measure the
checked linear-space kernel including input/domain and output finite checks;
they exclude surface traversal, CMS transfer evaluation, preview allocation,
and file I/O. The ignored `bench_contrast_division_wide_24mp` records the
reproducible scenario. Renderer jobs retain their existing aggregate work,
resident-memory, banding, and cancellation bounds.


Gray/F32 nonlinear implicit SGray Normal28 and Multiply30 now pass four
hash-bound mask interaction source/edit references (`gimp_gray_mask_original`),
worst 0.002014/255. The v12 canvas is 16ï¿½12 with foreground10ï¿½9 at source
(2,-1), edit(-1,2), opacities0.7125/0.4375; the backdrop is16ï¿½12 at0,0 and
opacity1. The foreground mask is Gray/F32,10ï¿½9, parser source offset0,0,
placed at its owner bounds; native mask and owner sample bytes remain exact.
Stored mask visible/enabled/editing are true, shown false; the deprecated source
item-link flag is false. Canonical mask moves-with-owner attachment is true and
is a separate semantic. The public
observer's active mask edit flag is true; it does not replace that stored flag.
The Normal foreground and Normal backdrop store blend/composite/operator
0/-1/-1 and resolve Linear/Linear/Union. Multiply foreground stores -1/-1/-2
and resolves Linear/Linear/ClipToBackdrop. Union instead of ClipToBackdrop
fails Multiply's source reference by85.165/255. There are no assigned profiles,
groups, filters, channels, paths, or floating selections in this witness domain.
This proves the checked existing kernels for this grayscale mask interaction;
it does not establish unequal mask extents or a universal Auto resolution.


The filter-free `group-{28,61}-none-mask` originals qualify isolated Normal28
and pass-through61 group-mask composition. Despite the filename, these groups
have real Gray/F32 masks: source group/mask12ï¿½9 at0,-1, edit12ï¿½11 at-1,0.
The child extents remain independent: Normal child11ï¿½8 at0,0 and Multiply
child10ï¿½9 at2,-1 changing to-1,2. The backdrop is16ï¿½12 at0,0. Native planes
and masks are hash-bound and immutable; all four complete RGBA source/edit
comparisons pass below one16-bit code, worst0.001989/255. These witnesses prove
equal current group/mask extents, not unequal mask geometry. The Gaussian
versions of these groups remain unqualified and are not stripped for testing.

The isolated group's first visible bottom Multiply seeds its source coverage
into the empty isolated stack, while preserving its saved ClipToBackdrop
policy for metadata and subsequent compositing. Applying that operator to the
initial empty backdrop loses the layer; the independent isolated originals
fail by206.549/255. Pass-through evaluates against its real external backdrop
and retains ClipToBackdrop. Forcing a Union seed there fails by206.550/255.
The checked renderer applies the structural seed only to a Normal28 group
carrying persistent canonical epoch `xcf13-isolated-multiply-seed-v1`, never as a per-pixel fallback after some other layer's zero alpha.
Original group/blend/composite/operator policy values are retained, including
all negative Auto codes. The source XCF version is13, not12, and group opacity
is1 in both observed states. Renderer cancellation returns a checked error and
leaves the original document unchanged.

The structural seed is independent of active profile, transfer encoding, and
sample depth. Explicit same-byte profile assignment retains seeded alpha and
coverage; a generic/unqualified group epoch retains the previous semantics.
Importer admission owns the original domain qualification before stamping this
persistent epoch. Source signed policies are never rewritten to fabricate Union.


24 MP release streamed authored-group evaluation on2026-10-10: isolated
Normal28 5.797s, pass-through61 10.115s. The optional
`PHOTOCRAFT_XCF_GROUP_BENCH=1` corpus test reuses one PreparedRenderer over
125,000 bounded16ï¿½12 renders (24million pixels), covering real three-raster
composition, group mask reads, CMS transfers, cancellation/accounting callbacks,
and per-band allocation. It excludes file decoding and one-time preparation;
it is a streamed fixture workload, not a large sparse-surface/cache benchmark.


Multiply30 additionally passes ten hash-bound explicit-space source/edit
references in `gimp_multiply_spaces_original`: ProfileRgb2 or PerceptualRgb4
blend with LinearRgb compositing, and LinearRgb blend with ProfileRgb2,
PerceptualRgb4, or Lab3 compositing. All use ClipToBackdrop, the existing
implicit analytical sRGB RGB/F32 nonlinear16ï¿½12 two-raster domain and observed
foreground10ï¿½9 offsets/opacities; no groups, masks, filters, or assigned profiles.
Perceptual explicit fields require sourcev23; profile and Lab explicit fields
usev12. Worst complete RGBA difference is0.001938/255 (Lab edit), below one
16-bit reference code. This qualifies Lab compositing with a linear RGB blend,
not Lab blending, arbitrary profiles, or all space/operator combinations.
Native pixels remain unchanged. Exact stored Auto fields remain separate from
the public observer's Auto zeros; the importer owns precise stored-tuple audit.


The full XCF original corpus is currently unpublished. Its authored integration
tests are explicitly ignored in ordinary corpus builds; public pinned-corpus
and synthetic tests remain active. Run the reviewed local originals with
`PHOTOCRAFT_XCF_FULL_CORPUS=<full checkout>` and an explicit named-module gate,
for example `cargo test -p photocraft-compose --features corpus --test
 gimp_multiply_spaces_original -- --ignored`. Do not report ordinary skipped
original tests as passed. The ignored Gaussian group oracle is separately
pending live-owner/small-sigma implementation and currently fails when invoked.
Modified XCF text/vector sources render only their current raster when their
typed declaration marks the source modified, their immutable source cache and
complete canonical state validate, and no source transform is pending. Cache
bytes remain provenance and are never substituted for current pixels. Unmodified
sources, linked sources, floating attachments, and required opaque rendering
state still require additional evaluators. Synthetic wide tests distinguish a
black U8 source cache from edited F64 HDR pixels and retain that distinction after
an explicit working profile assignment.
`PreparedRenderer::new_with_interrupt(document, interrupt)` shares the job's
cancellation/deadline checks during source-cache validation (hash chunks are
32 KiB). The original `new` delegates to `Interrupt::NONE`. Export/import callers
should use the interrupt-aware constructor; cancelled validation returns the
checked raster cancellation error and leaves the document unchanged.
Modern Hard Mix (XCF mode 51) uses the GIMP public user-manual channel sum
threshold: sum >= 1 produces 1, otherwise 0. The checked f64 kernel admits
finite unit blend inputs, preserves sub-f32 threshold differences, and rejects
unqualified HDR inputs. Two hash-bound v23 authored RGBF32 implicit-sRGB source /
edit references pass at <= 0.001936/255 in the observed Perceptual blend / Linear
composite / ClipToBackdrop domain; raw stored tuple remains -4/-1/-2 (blend /
composite / operator), separate from public Auto zeros. Release 24 MP kernel
measurement: 1.1588891 s (working space prepared, no file decode/preparation).
Public formula: https://testing.docs.gimp.org/3.2/bg/layer-mode-group-contrast.html

The vector observation distinguishes LIVE BEFORE SAVE native pixels from
reopened native pixels by 208/255; reconstructed reopened native pixels match
the PNG within 0.500776/255. This is not a serialized-cache disproof: the actual
XCF plane matches the reopened orange raster. The importer/render pipeline
mismatch remains separately pending investigation. The generic canonical
current-raster renderer never substitutes source-cache pixels.
Legacy Multiply (3) and Screen (4) have distinct checked dispatch. The public
legacy manual supplies their channel equations; four original source/edit
references qualify the observed ProfileRGB blend and composite spaces with
backdrop-retaining overlap coverage. Exact stored tuple is -2/-2/-2; no absolute
value normalization occurs. Authored modern-coverage substitution fails visibly.
Unit samples, explicit legacy policy, ProfileRGB spaces and ClipToBackdrop are
required; masks/groups/assigned-profile families remain separately unqualified.
Reference maxima <= 0.001915/255. Release 24 MP checked kernel measurements:
LegacyMultiply 1.3969647 s; LegacyScreen 1.3133814 s (prepared spaces, no decode).
Public equations: https://docs.gimp.org/3.0/en/gimp-concepts-layer-modes-legacy.html

Checked floating appearance now supports the observed two-raster Normal attachment: full-canvas immediate target, contained floating footprint, native matching RGB/Gray format, unit opacity, Profile composite space and Union. Normal does not consume blend space; stored Auto remains unresolved source data. Ten independent GIMP source projections cover U8/U16/U32/F32/F64 RGB and Gray. Derived visible RGBA bounds are half an integer output LSB plus 1e-6, or 1e-6 for wide projections; hidden RGB at zero alpha is not an appearance claim. Native surfaces remain immutable and exact. Floating defaults are clipped to the declared footprint; masks, other operators, smaller targets, source resources and complex trees remain unsupported. Run `gimp_floating_original --ignored` with the unpublished full corpus explicitly configured.

Modified vector placement remains unqualified: the serialized source extent starts at (9,11), while GIMP's reopened drawable starts at (10,12). Identical reopened native pixels match the reference at the latter position and reproduce the 208/255 error at the former. This is a placement discrepancy, not a raster-cache validity failure.

Hidden saved alpha channels now receive checked canonical-state validation before imported image projection. The independently authored active-channels source/edit pair preserves both native channel planes while its layer projection matches; visible/spot channel overlays and channel-owned filters remain actionable unsupported errors. This qualifies image projection exclusion only, not channel-view or thumbnail rendering. Run `gimp_channels_original --ignored` against the unpublished full corpus. Compose tests, strict corpus Clippy and wasm checks passed.

Additional checked legacy modes 6/7/8/9/10 (Difference, Addition, Subtract, Darken only, Lighten only) match all ten authored source/edit RGBA projections within 0.001904/255. Exact saved byte witnesses are XCF12 RGB F32 nonlinear without assigned ICC, foreground stored operator/composite/blend -2/-2/-2 and backdrop Normal28 -1/-1/0. Each retains distinct legacy overlap coverage and requires Profile/Profile ClipToBackdrop, finite unit input; Addition clips its blend product at one and Subtract at zero, as the public legacy manual states and these originals confirm. No legacy HDR or unknown operator/space admission follows. Hashes and byte-level assertions live in gimp_legacy_arithmetic_hashes.json / gimp_legacy_arithmetic_original.rs. Release 24MP pre-prepared f64 kernel timings: Difference 1.352s, Addition 1.337s, Subtract 1.520s, Darken 1.572s, Lighten 1.632s. Public equations: https://docs.gimp.org/3.0/en/gimp-concepts-layer-modes-legacy.html .

Qualified isolated Gaussian groups now use the independently verified finite small-Auto kernel: exact f32 x=.8/y=1.3, Clamp/clip true, v22 Replace envelope, source region0/edit region1, empty F32 selection mask, nonlinear implicit-sRGB F32 and matching parent mask. Current children are composed live, converted through checked working-profile f64, filtered, returned to stored encoding, and masked; one bounded job-local group cache is private. Source/edit full RGBA maxima are 0.002141/0.002566 per255 (below the unchanged 1/65535 gate). Pass-through groups, other sigmas, profiles, canvas/extent domains and nonempty selection masks remain unsupported. Shared interrupts cover preparation, mask validation, source collection, kernel and publication. Kernel resident reservation is capped64MiB; source dimensions are12x9 or12x11, canvas16x12.

Independent disabled saved/reopened derivatives in group-disabled-evidence prove retained configuration plus unfiltered live-child appearance; source/edit projection errors remain below4.23e-7 and native children are exact. Local packet publication remains pending. Batched input writes remove repeated COW tile copying:1000 prepared192px group renders improved39.595sâ†’2.208s in release. This measures the qualified small frame, not a24MP document or broad Auto domain.

Legacy SoftLight19 (including the saved Overlay legacy cases which GIMP writes as19) matches four source/edit references within0.001770/255, retains distinct overlap coverage, checked unit/Profile/ClipBackdrop domain and exact half-source neutral bits. Release24MP kernel timing1.349s. Legacy HardLight18 remains unsupported: substituting the modern normalized equation fails the stricter independent reference gate; its public256 normalization requires separate qualification. Divide/Dodge/Burn legacy remain pending.

Legacy modes 15â€“18 remain explicit unsupported after hash-bound original probes: public manual 256-code formulas miss source references by Divide 0.271769/255, Dodge 0.192451/255, Burn 12.388174/255, Hard Light 0.108280/255. Modern Hard Light also misses 0.232372/255. No tolerance was relaxed or candidate kernel retained. The original contrast regression verifies exact saved mode/space metadata and rejection for these cases; only legacy Soft Light mode 19 (including rewritten Overlay originals) is admitted in this slice.

A separate finite raster Gaussian member (schema epoch xcf22-rgb-alpha-finite-gaussian-v1) now uses the actual ordered renderer on ten saved/fresh-reopened 16Ã—12 RGB projects: mixed alpha, finite unclipped HDR, F32/F64 nonlinear samples, implicit sRGB and one own assigned parametric ICC. Exact enabled Replace envelope raw spaces remain unresolved âˆ’1474592175; source Auto (2.25, 1.3f32), edit FIR (3.75, .8f32), Clamp and operation clipping. Exact empty GrayF32/F64 selection masks stay native data. The declared extent is privately converted to working linear F64 and evaluated with checked finite-boundary Gaussian, then re-encoded through the active profile. All full native RGBA and actual renderer comparisons pass the separately documented 2/255 new-slice bound; worst .119474481/255 assigned unit-F32. Existing tight Gaussian references remain unchanged. Tests preserve source/native bits, cancel atomically, release work reservations on denial, reject unknown envelope fields, and retain an F64 2^-40 source-color difference in derived output. Generic GIMP requested format reads convert color space: use native format=None plus assigned ICC for source-code interpretation. Corpus packet full/gaussian-raster-color-evidence manifest SHA952446720a293e00e88ac4076c19f2e0ba07079dec2ef09d99e404802a7adcf1; unpublished/ignored gate is gimp_gaussian_color_original. Larger raster consumer extents and arbitrary Gaussian settings remain outside this member despite separately qualified algorithm kernels.

Release adapter timing: 1,000 isolated authored 192-pixel finite-Gaussian renderer frames took 1.5830343 s (1.58 ms/frame). This measures the actual scoped consumer and its private per-invocation caches; it is not a 24 MP consumer claim. The generalized algorithm kernel's 24 MP release measurement is recorded separately by its owner.

### Checked Box Blur import slice

The ordered renderer admits only the hash-bound Box Blur 0:0 Replace envelopes: radius 4/5, implicit nonlinear RGB U8 at 32Ã—32, or the separately authored 16Ã—12 nonlinear RGB F32/F64 color/alpha/HDR packet. Assigned-profile support is qualified by that packet; other domains remain unsupported. The live source is interpreted through checked CMS into linear F64, filtered on a bounded private surface, and converted back through the active working profile. The source raster is preserved. Expanded operation bounds are distinct from the logical canvas.

Explicit unpublished-corpus gates: `cargo test -p photocraft-compose --features corpus --test gimp_box_original --test gimp_box_color_original -- --ignored`. Original U8 source/edit canvas crops match their expanded drawable PNGs within 0.499827/0.503729 levels per 255. Ten new cases pass a separate 2/255 bound: full expanded premultiplied RGB plus independent alpha worst 0.0645174/255; straight logical-canvas native crop worst 0.062084/255; generic visible projection worst 0.070086/255. GIMP's near-zero-alpha unassociated color instability is retained and not claimed exact. Fixed source bytes, cancellation, balanced denied work, unsupported envelopes and F64 low-bit preservation are checked. The kernel's independent 24 MP Gray F64 measurement is approximately 1.18 s (Sol 2); this is not a 24 MP consumer-domain claim.

The qualified 16Ã—12 Box consumer measured 1,000 isolated logical-canvas frames in 1.632773 s (~1.63 ms/frame) in release on this machine. This is a scoped small-document measurement; no larger consumer domain is inferred.

### Checked quantized Median Blur import slice

The ordered renderer admits only the original hash-bound RGB U8 nonlinear implicit-sRGB 32Ã—32 Median Blur 0:0 Replace envelope: circle neighborhood, radius 3/4, color and alpha percentiles 50, Clamp, high precision disabled, saved empty selection mask. Input operation samples retain their encoded working interpretation; this kernel receives no invented linear transfer override. Alpha-weighted percentile ties use the independently qualified integer alpha-code kernel. Other profiles, depths, neighborhoods and parameters remain unsupported by this import slice.

`cargo test -p photocraft-compose --features corpus --test gimp_median_original -- --ignored` checks both genuine source/edit references. Full RGBA 8-bit output codes match exactly; numerical f64-to-decoded-f32 comparison differs by at most 0.00000757 levels per 255. Source/native bytes stay unchanged. Unknown version, wrong region/operator, high-precision activation and work denial are rejected; cancellation abandons the private result. Always-on schema tests reject duplicate/missing/wrong-type arguments.

The original U8 quantized Median consumer measured 1,000 isolated 1,024-pixel frames in 4.1831395 s (~4.18 ms/frame), release on this machine. The separate 24 MP high-precision reference kernel takes approximately 34.56 s and remains an optimization target; no performance qualification is claimed for it.

### Checked Linear Motion Blur import slice

The ordered renderer admits only the original hash-bound RGB U8 nonlinear implicit-sRGB 32Ã—32 Motion Blur Linear 0:0 Replace envelope: stored F32 length 10/10.125, angle positive zero, saved empty selection mask. Live native samples pass through the active checked CMS into linear F64; the independently qualified associated-alpha line sampler uses transparent abyss and expands the operation extent; output returns through the active profile. Source pixels stay unchanged.

`cargo test -p photocraft-compose --features corpus --test gimp_motion_original -- --ignored` checks genuine source/edit projects. Full expanded 42Ã—32/44Ã—32 drawable premultiplied RGB plus independent alpha errors are 0.573428/0.528399 levels per 255; straight canvas errors are 0.491071/0.488259, all within the fixed 2/255 bound. Unknown version, region, angle, denied work and cancellation are checked. Hidden straight colors at vanishing alpha are not claimed exact. General angles, depth/model/profile families remain outside this consumer slice.

The independent reference kernel's 24 MP Gray F64 measurements are approximately 11.10 s at length 10 and 20.59 s at 10.125; optimization remains open. These are kernel measurements, not a larger admitted consumer domain.

Median's original 24 MP high-precision reference measurement above was 34.56 s. Sol 2 subsequently added a bounded halo cache without changing rank semantics: the latest kernel measurement is approximately 11.23 s (3.08Ã— faster), still an optimization target.

Gray Median has its own four original 16Ã—12 implicit nonlinear F32/F64 alpha/HDR source/edit witnesses, not an inference from RGB. The checked scratch seam reads exactly the runtime channel count, replicates Gray into RGB scratch and preserves Gray alpha in the coverage slot; an always-on negative-origin regression checks this and opaque Gray without alpha. Actual document renderer native gray/alpha error is at most 0.0000135104/255; generic visible RGB is within 0.00102622/255. Source/default/native bytes, cancellation, denied-work rollback and high-precision F64 low bits remain checked. The fixed 2/255 qualification bound is unchanged.

`cargo test -p photocraft-compose --features corpus --test gimp_median_gray_original -- --ignored` runs the explicit unpublished-corpus gate. Assigned Gray profiles, other radii, parameter pairings, sample encodings and model families are still unqualified by this slice.

The Motion consumer's scoped 1,024-pixel canvas measured 1,000 isolated frames in 2.2935737 s (~2.29 ms/frame), release on this machine.

### Checked square Pixelize import slice

The ordered renderer admits only the original hash-bound RGB U8 nonlinear implicit-sRGB 32Ã—32 Pixelize 0:0 Replace envelope: square norm 2, size-x 16/17, size-y 16, local offsets zero, ratios one, exact stored white background Color and saved empty selection mask. Square full-ratio blocks never consult background, but its declared typed value is retained and checked; original encoding, native sample and ICC bytes remain importer provenance. Other norms, ratios, model/profile/depth families remain unqualified.

Live native pixels pass through the active checked CMS to linear F64. Associated-alpha block means use the block's finite input intersection and drawable-local grid; output returns through the active working profile. `cargo test -p photocraft-compose --features corpus --test gimp_pixelize_original -- --ignored` verifies original source/edit full RGBA errors of 0.339066/0.380992 levels per 255, below the fixed 2/255 bound. Source bytes, cancellation, denied-work rollback and unsupported version/region/norm/operator controls are checked. The independent reference kernel's 24 MP Gray F64 measurements are approximately 2.086 s and 2.385 s for 16Ã—16/17Ã—16; these are kernel measurements, not a wider consumer-domain claim.

The qualified original Pixelize consumer measured 1,000 isolated 1,024-pixel canvas frames in 2.6184322 s (~2.62 ms/frame), release on this machine. Always-on schema tests reject duplicate/missing parameters, wrong scalar types, partial ratios and unqualified extents/versions/regions.

### Scoped modern HSV/HSL XCF composition

Modern modes 37/38/39/40 replace HSV hue/saturation/value or HSL hue+saturation using independently implemented min/max color-model definitions. Public behavioral sources are the [GIMP HSV/HSL manual](https://docs.gimp.org/3.2/en_GB/layer-mode-group-hsv.html) and [XCF compositing definitions](https://developer.gimp.org/core/algorithm/compositing/); no implementation code was used. Hue with achromatic source preserves the backdrop, and undefined achromatic hue is red. Current checked domain is three-channel RGB, finite unit colors, ProfileRgb blend, LinearRgb composition, and ClipToBackdrop coverage. Gray, HDR colors, other spaces/operators and unqualified profile interpretations remain unsupported.

`gimp_hsv_original` explicitly runs eight SHA-bound unpublished source/edit references with `--features corpus --test gimp_hsv_original -- --ignored --nocapture`. Originals are XCF12 F32 nonlinear implicit RGB, canvas16x12 with foreground10x9 moved (2,-1) to (-1,2), opacity0.7125 to0.4375. Saved foreground policy is blend/composite/operator -2/-1/-2; independently queried public Auto fields remain zero and are preserved distinctly. Complete RGBA worst error is 0.00190022/255 against the unchanged 1/65535 bound. Source surfaces remain unchanged; cancellation, denied work, unit/HDR and space/operator negatives are tested. This proves the scoped originals, not general stock HSV/HSL compatibility.

Local release benchmark of 24 million checked RGB overlap evaluations (including prepared analytical profile/linear transfers, varied backdrop, fractional alpha and opacity) measured Hue 7.560s, Saturation 8.928s, HSL Color 8.794s, Value 8.621s. This is single-thread kernel throughput, not a full-frame/cache/UI benchmark; it has no additional image allocation and does not establish a frame-time budget. Run `cargo test -p photocraft-compose --release --lib component_composition_24mp_release -- --ignored --nocapture` to reproduce. Optimizing per-pixel transfer and checked dispatch remains open.

### Scoped modern LCh XCF composition

Modern modes24/25/26/27 replace CIE LCh hue/chroma/color/lightness in physical CIE Lab D50 using min/max-free Cartesian chroma directions: C=hypot(a,b). Behavior is specified by the [GIMP LCh manual](https://docs.gimp.org/3.2/en_GB/layer-mode-group-lch.html), with public CIE15/ICC PCS math provided by CMS. No implementation code or fitted coefficients were used. Signed a/b values and chroma beyond normalized ICC Lab bounds remain physical f64 values. Matrix/TRC RGB device-to-PCS pipelines feed physical XYZ-to-Lab and Lab-to-XYZ stages directly; ICC v4 EncodeLabV4 clipping is bypassed. Existing Lab compositing conversion is unchanged. Zero source chroma preserves backdrop hue; zero backdrop chroma remains achromatic for Chroma replacement. These degenerate rules are tested mathematically, but original stock references alone do not establish every near-neutral profile interpretation.

Checked initial domain is RGB finite unit native input, physical Lab blend, LinearRgb composition, ClipToBackdrop. Gray, raw HDR source colors, LUT-based Lab conversion, other spaces/operators and assigned-profile admission remain unsupported/unqualified. Physical reverse conversion retains negative and greater-than-one output RGB without clipping, and sub-F32 lightness changes survive; this does not admit general HDR inputs. `gimp_lch_original` runs eight SHA-bound XCF12 F32 nonlinear implicit RGB source/edit references, foreground10x9 moved(2,-1) to(-1,2) with fractional opacity, saved blend/composite/operator -3/-1/-2 distinct from public Auto zeros. Actual PreparedRenderer fullRGBA maximum0.00223392/255 passes the unchanged1/65535 bound. Source surfaces unchanged; cancelled construction, denied work, unsupported policies, Gray/hiddenHDR and finite checks pass. Run `--features corpus --test gimp_lch_original -- --ignored --nocapture`; unpublished originals stay ignored by default. Existing HSV and explicit Multiply Lab-space references also pass.

Local single-thread release24MP overlap evaluation including prepared physical Lab/CMS conversion measured Hue18.693s, Chroma18.641s, Color19.435s, Lightness18.741s. This has no image allocation and is kernel throughput, not a frame/UI budget. Run `cargo test -p photocraft-compose --release --lib lch_composition_24mp_release -- --ignored --nocapture`. Per-pixel transfer/pipeline dispatch optimization remains open.

### Scoped modern Luma/Luminance XCF composition

Modern modes54/55 select the entire RGB color with lesser/greater CIE luminance, rather than independent channel extrema. Mode56 scales backdrop linear RGB by source-Y/backdrop-Y, as specified in the [GIMP LCh/Luminance manual](https://docs.gimp.org/3.2/en_GB/layer-mode-group-lch.html). Y coefficients come directly from the current RGB profile's ICC PCS D50 colorant matrix. A native-white/unadapted Y hypothesis failed the original Luminance source by4.171/255; that discrepancy is retained in the diagnostic log. No coefficients were fitted. Exact equal-Y/different-color selection ties and zero-backdrop-Y ratios remain unsupported pending independent behavior observations.

The admitted kernel context is three-channel matrix/TRC RGB, finite unit source colors, LinearRgb blend and compositing with ClipToBackdrop. Gray/HDR source colors, LUT profiles, other policies, and assigned-profile importer admission remain unqualified. Output may exceed unit gamut and is retained as finite f64. `gimp_luma_original` explicitly verifies six SHA-bound source/edit XCF12 F32 nonlinear implicit RGB originals, canvas16x12/foreground10x9 moved(2,-1) to(-1,2), fractionalopacity0.7125â†’0.4375, exact saved blend/composite/operator -1/-1/-2 versus queried public Auto zeros. FullRGBA worst0.00208355/255 passes the unchanged1/65535 bound. Source/native bits remain unchanged; cancellation, denied work, model/space/operator/HDR, singular ratios, ties, and sub-F32 samples have regressions. Run `--features corpus --test gimp_luma_original -- --ignored --nocapture`. These originals remain ignored until the full corpus is reviewed and published.

Local single-thread24MP checked RGB overlap plus prepared CMS throughput: LumaDarken14.424s, LumaLighten15.632s, Luminance15.697s. This measures kernel/conversion work without full-frame image allocation or UI caching; it does not meet a frame-time claim. Reproduce with `cargo test -p photocraft-compose --release --lib luminance_composition_24mp_release -- --ignored --nocapture`. Reusing identical prepared working-space samples to avoid redundant inverse/forward transfer remains a possible bounded optimization, requiring exact interpretation identity and regression proof.

### Scoped legacy HSV/HSL composition

Legacy modes11/12/13/14 reuse independently implemented public HSV/HSL component replacement with explicit legacy coverage: overlap=min(backdrop-alpha,source-alpha)*layer-opacity, followed by legacy overlap weighting and retained backdrop alpha. They require legacy policy plus ProfileRgb blend/compositing and ClipToBackdrop; modern component-mode and Photoshop composition operators are unchanged. Gray, HDR source colors, other spaces/operators, assigned-profile or unqualified masks/effects remain outside the scoped importer domain.

`gimp_legacy_hsv_original` runs eight SHA-bound source/edit original XCF12 F32 nonlinear implicit RGB references with saved blend/composite/operator -2/-2/-2, versus independently queried public Auto zeros. CompleteRGBA worst0.00192365/255 passes unchanged1/65535. Native input samples are audited finite0..1 and remain unchanged. Missing legacy identity, wrong policy, cancellation and denied work are rejected; always-on tests prove retained legacy alpha/min-alpha weighting, achromatic/sub-F32 identity and model/HDR bounds. Existing modernHSV8 and legacyMultiply/Screen4 projections pass. Run `--features corpus --test gimp_legacy_hsv_original -- --ignored --nocapture`; originals remain ignored until reviewed corpus publication.

Local single-thread24MP checked overlap throughput: legacyHue1.581s, Saturation1.734s, HSLColor2.236s, Value2.092s. Profile-space kernels require no nonlinear/linear transfer in this measurement; these are not full-frame/UI/cache budget claims. Reproduce with `cargo test -p photocraft-compose --release --lib legacy_hsv_composition_24mp_release -- --ignored --nocapture`.

### Deterministic GIMP alpha modes (2026-10-10)

Checked Erase 58, Merge 59 and Split 60 now use independent, hash-bound saved source/edit references (`gimp_alpha_original`, feature `corpus`, run explicitly with `--ignored`). Scope remains XCF12, implicit RGB F32 nonlinear, the two-layer 16x12 packet, finite unit native samples and its measured geometry/opacities. The six full RGBA comparisons pass the unchanged 1/65535 bound; maximum error is 0.00179524/255. Native source/document data remains unchanged and cancellation/work denial returns an error.

[The public GIMP manual](https://docs.gimp.org/3.0/en/gimp-concepts-layer-modes.html) distinguishes multiplicative Erase coverage, additive Merge coverage, and subtractive Split coverage. Erase and Split copy backdrop RGB exactly, so source blend/composite Auto values remain recorded without a transfer-space inference; Merge uses the independently qualified linear composite and Union. A proposed inverse-premultiplied-color interpretation of Split failed the genuine reference by 206.55/255 (`temp/sol4-alpha-after.log`); it was discarded, with no tolerance change. Split instead preserves backdrop color and subtracts effective source alpha. Source alpha includes layer opacity. Synthetic tests cover sub-F32 color retention, opacity/alpha endpoints, Merge overflow-coverage weighting, unsupported operators/models/HDR, nonfinite input and closed ColorErase/Dissolve dispatch.

ColorErase 57 and coordinate-dependent Dissolve 1 remain unsupported. The current renderer does not imply their correctness from these pure alpha equations. User-visible/native semantic labels and importer predicates remain separate integration work.

### Scoped Color Erase (2026-10-10)

`gimp_color_erase_original` adds two hash-bound Color Erase57 source/edit witnesses. Actual saved mode57 retains raw blend/composite/operator -1/-1/-2; the independently qualified consumer uses current-profile linear RGB and ClipToBackdrop in the same implicit RGB F32 nonlinear 16x12 two-layer domain. Both full RGBA references pass 1/65535 (maximum 0.00192964/255), with unchanged source/native document state, cancellation and denied-work errors.

The implementation derives minimum retained unit-RGB coverage from [public source-over algebra](https://www.w3.org/TR/compositing-1/#simplealphacompositing), rather than copying an editor implementation: solve back=source*(1-retained)+residual*retained with residual components in [0,1], then apply effective source alpha to removed coverage. This is a derivation checked against independent saved [GIMP Color Erase behavior](https://docs.gimp.org/3.0/en/gimp-concepts-layer-modes.html), not a claim that the manual specifies these equations. Stable remaining-coverage arithmetic retains a positive 1e-300 alpha where subtracting rounded removal from one would lose it. Synthetic reconstruction, equal-color transparency, alpha endpoints, transfer/model/operator/nonfinite/HDR rejection and source precision checks remain active. Assigned ICC/Gray/HDR sources and legacy Color Erase22 remain unqualified; no legacy22 original exists in the current composition packet. Dissolve1 remains closed pending coordinate-dependent stochastic semantics.

Alpha/Color Erase final gates: full compose baseline 175 unit tests passed (9 performance tests ignored) plus integrations; explicitly enabled alpha6/ColorErase2/modernHSV8 original comparisons passed, strict corpus all-target clippy and WASM passed. 24MP release checked-kernel measurements: Erase 0.3898s, Merge 5.8828s, Split 0.4350s, ColorErase 5.9654s. These are scalar compositor kernel timings, not complete image/frame timings. Logs: temp/sol4-alpha-{final,baseline,clippy-final,wasm,release}.log and temp/sol4-color-erase-{final,baseline,clippy,wasm,release}.log.

### Dissolve observations and independent Gray alpha/color qualification (2026-10-10)

`gimp_dissolve_original` binds a new independent stock packet with40 fresh loads over two isolated headless processes. Repeated/native projection patterns are exact, alpha0.5 at100% and alpha1 at50% opacity agree, and translated patterns agree at canvas coordinates (zero mismatches) rather than drawable-local coordinates (328/329 mismatches). The existing PhotoCraft hash disagrees on375/768 half-opacity pixels. No GIMP PRNG or seed contract was inferred or copied. The checked mode1 consumer therefore supports only exact zero/full coverage endpoints; four endpoint projections match exactly and16 partial probe/original records return Unsupported without modifying source/native state. Positive products that underflow remain unsupported. Full stochastic Dissolve remains open.

The authorized fallback adds implicit GrayF32 nonlinear stock Color Erase57/Erase58/Merge59/Split60 source/edit eight cases with independent saved/native/profile/PNG observations and16 verified fresh repeat loads. Gray scalar inputs must have identical replicated RGB slots. All eight full Gray PNG YA comparisons pass1/65535 (worst0.00174791/255). Native visible straight Y and premultiplied Y/alpha pass the same bound (worst0.00006379/255). Native composite alpha0 hidden Y differs explicitly, up to206.55/255 forSplit; GIMP PNG clears it, while all original source raster hidden samples are retained exactly. This is visible appearance and original-source preservation qualification, not exact unassociated native projection equality. No positive alpha is omitted from the visible comparison.

New ignored corpus gates: `gimp_dissolve_original`, `gimp_alpha_gray_original`; their checked-in hash manifests bind saved observations and native/render artifacts. Local original packets `xcf/full/dissolve-evidence` and `xcf/full/alpha-gray-evidence` have dual licenses, public-API generators/verifiers, artifact manifests and zero private-path scan hits. Unpublished corpora require `PHOTOCRAFT_XCF_FULL_CORPUS` and explicit `--ignored`. Neither packet changes master corpus pins/publication.

2026-10-10: checked legacy Divide15, Dodge16, Burn17, HardLight18 and GrainExtract20/GrainMerge21 now use independently qualified RGB unit/Profile/Profile/ClipToBackdrop coverage. The saved F32 observations distinguish normalized ratio products from old byte-manual256 approximations; HardLight retains the public128/255 breakpoint/offset and grain midpoint128/255. Singular endpoint decisions, clamp and sub-F32 preservation are independently tested. No Gray, ICC, HDR-input, mask or group admission follows from this slice. Source5 remains rejected (stock public writer saves19); stock3.2.6 rejects public set_mode22, retained separately.

`gimp_legacy_contrast_original`, `gimp_legacy_grain_original`, and `gimp_legacy_basis_original` are ignored unpublished-corpus gates: run with `--features corpus --test <name> -- --ignored --nocapture` and PHOTOCRAFT_XCF_FULL_CORPUS. All source/edit originals pass unchanged1/65535; worst0.0019425913/255. Separate public saved native basis has14freshloads/70hashchecks and maximum8.92901e-8. Native source planes remain exact. Public mathematics: https://docs.gimp.org/3.0/en/gimp-concepts-layer-modes-legacy.html (no editor implementation code used). Release24MP kernel times: Divide1.555s, Dodge1.577s, Burn1.881s, HardLight1.884s, GrainExtract1.509s, GrainMerge1.476s; these are streamed kernel throughput, not whole-document import timings.

Modern Grain46/47 source/edit references are separately bound by `gimp_grain_original` and `gimp_grain_hashes.json` (v23 implicitRGBF32 Nonlinear, Perceptual blend/Linear composite/ClipBackdrop). Four originals pass unchanged1/65535; worst0.002008309/255. `gimp_grain_basis_original` independently observes signed native outputs -0.5..1.5000002384 from unit inputs, so authoritative products remain unclipped; maximum6.55651e-7 across128native contrasts in two isolated fresh processes. Public math: https://docs.gimp.org/3.0/en/layer-mode-group-inversion.html. The previously observed46 mismatch closes after analytic active implicit-profile interpretation; no new fitted formula or tolerance change. Truthful imported Grain UI semantics and IO admission remain separate integration gates.

Exact transfer preparation now recognizes equal analytic sRGB curve definitions for NonlinearProfile/Perceptual identity without a decode/encode round trip. Parsed quantized ICC, unequal curves and table definitions retain their checked conversions; selected working-space semantics remain explicit. Separable operators skip source/backdrop composite conversions whose terms they do not use. Signed-zero/sub-F32 identity regressions and13 independent original-family modules pass, as do full compose tests/strict corpus clippy/WASM. Modern Grain24MP release kernel before/after: Extract19.311sâ†’5.983s(3.23x), Merge21.504sâ†’7.544s(2.85x). These are streamed kernel timings on this local machine, not complete document latency.

Normal28 explicit operators and ordinary masks (2026-10-10): new independent32-file RGB/Gray F32/F64 implicit nonlinear source/edit packet qualifies two-layer16Ã—12, masked10Ã—9 foreground at negative offsets and explicit composite Linear1/operators1â€“4. Public independently captured mask local extent/world offsets/NONE and CLAMP halos bind equal-extent geometry; no unequal-extent inference. Checked Prepared consumer32 passes alpha, premultiplied colors and positive-alpha unassociated colors at unchanged1/65535; native inputs and mask bytes remain exact. Zero-alpha unassociated display payload differences are reported separately, never used as source authority. Material mask disable control, pre/mid cancellation, denied work and unchanged document checks pass. New unpublished originals require explicit ignored gimp_normal_mask_original test with PHOTOCRAFT_XCF_FULL_CORPUS.

Stock PNG default save-transparent=False discards one genuinely positive alpha. Separate save-transparent=True controls/ref PNGs preserve it; default negatives remain bound. Fresh-process source/mask160 native controls are exact,896source/artifact bindings pass; derived mask reads differ at most3.61224e-8, native projection5.96046e-8 and16-bit PNG1LSB. These are observed bounds, not bit-exact derived-projection claims. Full local packet/hash verification and strict compose corpus clippy/WASM pass; root owns actual parserâ†’importer/native pipeline admission/counting.

Normal/mask24MP streamed checked f64 kernel timing (release, fixed sampled mask coverage, no whole-frame allocation): RGB Union5.464s/ClipBackdrop6.437s/ClipLayer.195s/Intersection.206s; Gray3.053s/3.109s/.204s/.206s. These measure existing kernel paths and analytical transfer, not total document upload or mask-surface traversal. No kernel hot-path change was necessary for the new32-case domain. Full compose tests220passed28ignored, plus explicit32-file corpus consumer pass; strict all-target corpus clippy and wasm pass. Logs temp/sol4-normal-mask-{final-stable,clippy-stable,wasm-stable,compose-full,perf}.log.

Additional LINEAR Normal/mask32 (precision600/700) independent originals pass same1/65535 alpha/premult/positive-alpha straight boundary; implicit RGB/Gray F32/F64 profiles and explicit operators1â€“4. Public native format/transfer identity captured separately, never inferred from nonlinear results. PNG preserves linear codes with parsed identityTRC ICC, while public nonlinear-double projection uses explicit checked analytical transfer. Source/mask160 native controls and derived mask/projection/PNG fresh repeats are exact for this packet,896 artifact/source bindings;29,952native samples verified unit-domain,16F64 source+mask sub-F32 markers retained. Stale transfer/implicit-profile binding is explicitly rejected. Material mask control, pre/mid cancellation/deniedwork preserve originalDoc. No HDR, assignedprofile, unequalmask or group admission claim. Separate ignored gimp_normal_mask_linear_original test/hashmanifest; root owns actual importer/native count. Strict compose corpus clippy/WASM pass. DefaultPNG lowpositivealpha loss retained alongside faithful True references.

24MP streamed checked LINEAR Normal kernels with fixed sampled-mask coverage (release, no full-frame allocation or mask traversal): RGB Union.557s/ClipBackdrop.553s/ClipLayer.159s/Intersection.160s; Gray.545s/.545s/.160s/.161s. Existing analytical working-space identity path, no kernel performance change for this evidence slice. Log temp/sol4-normal-mask-linear-perf.log; updated NL+LINEAR64 original consumer/control gate remains green.

Assigned gamma2.2 Normal28 ordinary-mask operators: independent RGB/Gray F32/F64 Nonlinear source/edit32 saved originals now pass separately declared spec SC-002 <=2/255 matched ICC/display acceptance. Own generated RGB624B/Gray448B profiles are assigned after public native writes; all48 before/after planes, source/mask raw bytes, F64 markers and assigned/effective/original ICC are exact. No profile conversion substitutes for assignment. Alpha/native-coded maximum .042281/255, default-space conversion .035687/255, faithful PNG .041635/255. Tight1/65535 failure is retained by an explicit diagnostic test and separate public transfer-roundtrip controls; implicit domain thresholds and composition kernels remain unchanged. Frozen four control families verify source/copy/projection native identity, source-preserving layer/mask activation and non-composition transfer bias; native profile/current/source context stay separate. Tests reject corrupt/wrong-model ICC and denied/mid/pre-cancel work, preserve current source pixels, and show mask activation materially changes output. Equal mask extents/unit source inputs and exact two authored profiles only; arbitrary assigned ICC/HDR/unequal masks remain open. Source tests gimp_normal_mask_assigned_original.rs plus hashes, independent diagnostic gimp_normal_mask_assigned_diagnostic.rs; unpublished full corpus explicitly ignored and run with --ignored. Native format/import mapping remains owned by IO integration.

24MP release assigned-profile Normal/mask fixed-pixel streamed kernel timings (no whole-document allocation/traversal claim): RGB Union3.956s/ClipBackdrop4.678s/ClipLayer.158s/Intersection.187s; Gray2.496/2.517/.214/.218s. These use parsed exact ownGamma ICC profiles rather than implicit built-ins; no kernel optimization or source-fidelity change introduced. Full compose feature-off222 passed/30ignored; own assigned32+diagnostic32 acceptance tests pass2 and own strict tests/profile-author example and wasm checks pass.

Unequal-channel assigned-profile Normal28 mask operators: ownRGBgamma1.6/2.3/2.8 (ICC676B) andGraygamma1.8(ICC436B) independently assigned after literal native writes;32RGB/GrayF32/F64 source/edit files pass checked Prepared actualreference comparisons. Maximum native-coded .5778235943/255, explicitdefault-space display .1069253592/255, faithfulPNG .5770427362/255 under separately declaredSC0022/255; implicit1/65535 bounds untouched. Profiles/source/masks/F64markers exact,48before/afterassignmentplanes exact,160freshnativecontrols,1056bindings, freshPNGexact/projection1.862645149e-6 recorded. Test gimp_normal_mask_channel_trc_original.rs and hashes pin the unpublished original32 packet; --ignored opt-in. Corrupt/wrongmodelprofiles/work/pre/midcancellation fail, maskactivation affectscurrentraster. CurrentICC is parsed by itsdeclaredRGB/Gray model; originalsourceprofile remainsimmutable. ExtraneousRGBTRCs inGrayICC now correctlyignoredforGraytransfer (failing-firstCMSregression), withoutdiscardingbytes/tags. A separately retained initialconflictingGrayauthor packet isnegativeprovenance, not intendedGray1.8 admission. No arbitraryICC/HDRinput/unequalmask claims. Root owns exactIO/native registration.

24MP release parsedactualunequalTRC/Gray1.8 Normal/mask streamedkernel: RGBUnion3.704s/ClipBackdrop3.706s/ClipLayer.155s/Intersection.157s; Gray1.993/2.030/.155/.157s, no whole-document allocation/traversal claim. New32 plus previousimplicitNL32/Linear32/assignedGamma32 references allpass aftermodel-specificcurvefix; fullcompose222pass31ignored; CMS79pass2ignored; strictalltargets/corpusclippy+wasmgreen.

Ordinary-mask unequal geometry remains an admission gap. Four independently authored public GIMP3.2.6 RGBF32Nonlinear probes (masktranslate, scale, scale+translate andpublicGEGLbuffer.set_extent) preserveowner10x9@2,-1/mask10x9sameworldoffset/local0,0 before andafteraction/save/reopen. Two separatefresh-load rounds retain rawowner/mask/projection bytes exactly (24pairs/68hashcontrols), effectiveICC datevariations kept; no transformed-content or tile-extent inference relabels these as unequal-mask witnesses. Frozen private packets mask-geometry-public-probe andmask-buffer-extent-public-probe, publicgenerators and verifier; no newIOadmission/masterledger/publication. No claim that all possible XCFproducers normalize masks; genuinely unequal-source evidence remains required.

Synthetic actualPreparedconsumer64cases (RGB/GrayF32/F64, Linear/Nonlinear, outsideblack/white, operators1..4) verify independently declared negative/unequalmaskbounds versusowner and nonzero masktile/default outside. Literalcoverage equations match, rawsource/maskF64markers/defaults remainexact, denied/midcancel work preservesdoc, nonfiniteoutside policyerrors. Test gimp_mask_geometry.rs is canonicalmodel behavior only, notGIMPformatqualification. Alsofixed preparedWorkingSpace's duplicatecurve selection: validGraywithunrelatedRGBTRCtags usesdeclaredGraykTRC; parsedfailing-first gimp_wide regression nowpassesexact andpreservesprofilebytes. PriorNormal128originals pass; fullcompose224PASS31ignored, strictalltargets/corpus+wasmgreen. No sharedIO/render/format edits.

Own integer Normal/mask96 source/edit originals qualify RGB/Gray U8/U16/U32 Linear/NL implicit profiles with four explicit operators and owner-aligned masks. Source/mask native bytes and U32 low-code markers remain exact; quantized derived native/display/faithfulPNG use a separate2/255 bound (worst .499081/1.090680/.664166 per255), retaining failed Float1/65535 diagnostic. No production math or implicit Float tolerance change. Tests: gimp_normal_mask_integer_original explicitly --ignored with full unpublished corpus. 24MP analytic Normal scalar kernel RGB .164-.561s / Gray .160-.560s, not full integer frame throughput. Unequal-mask admission remains unqualified.

Finite source-alpha Normal has a separate checked WorkingSpace::normal_finite_source API: LinearRgb/full-opacity Union only, transparent or opaque backdrop, finite source colors/alpha. Transparent output preserves current source samples; opaque skips negative alpha and extrapolates alpha over1. Global unit-alpha normal/coverage policies are unchanged. Hash-bound12 public source/projection witnesses (float-alpha-composition-evidence) match transparent bitexact and opaque <=3.14e-7. Partial backdrop, masks, other operators, arbitrary profiles require further qualification and are rejected; root integration must use a scoped persistent policy.

The opt-in unpublished `gimp_normal_mask_half_original` gate now qualifies 32
independently authored RGB/Gray half-float Normal source/edit cases, linear and
nonlinear, with ordinary owner-aligned masks and explicit operators 1â€“4. XCF12
precision 500/550 remains source provenance; storage widens the original half
bits exactly to F32. Original half color/mask markers, native planes, immutable
state, mask activation, current transfer binding and cancellation/work controls
are checked separately from derived output. The maximum native half projection
difference is 0.062256/255 (the unit half rounding quantum); matched nonlinear
display is 0.071344/255 and faithful PNG is 0.070281/255. Its separate SC-002
bound does not change float-domain thresholds; the original 1/65535 diagnostic
failure remains recorded. Hidden RGB at zero alpha is retained, and derived
hidden-color differences are reported separately. Run with `--features corpus
--test gimp_normal_mask_half_original -- --ignored` and the reviewed local
`PHOTOCRAFT_XCF_FULL_CORPUS` packet. This does not qualify half HDR, assigned
profiles, unequal masks or unknown policies.


The ignored `gimp_normal_mask_perceptual_original` gate qualifies 48 independent
RGB/Gray U16/F32/F64 perceptual Normal source/edit cases with implicit profiles,
owner-aligned unit masks and explicit linear-space operators 1-4. XCF12 source
precision 275/675/775 is retained separately from runtime sample type. Public
perceptual native and nonlinear-double reads independently distinguish the
analytical sRGB transfer from linear and gamma2.2 hypotheses (agreement <=1e-6).
Existing checked transfer/composition kernels need no coefficient changes.
Native projection, visible/premultiplied nonlinear output and faithful PNG
maxima are 0.002007/255, 0.002290/255 and 0.002396/255, all within the unchanged
1/65535 gate. Source/mask bytes and current-profile binding remain exact;
hidden-color differences at zero alpha are reported separately. Same-byte
explicit assignment and a changed active linear profile exercise current
interpretation independently of immutable provenance. Work denial, pre/mid
cancellation, stale transfer and material mask activation are covered. Run
with `--features corpus --test gimp_normal_mask_perceptual_original --
--ignored` and `PHOTOCRAFT_XCF_FULL_CORPUS`. This evidence does not qualify
assigned perceptual profiles, HDR/nonunit coverage or unequal mask geometry.


The separate `gimp_normal_mask_perceptual_depth_original` gate qualifies 48
own RGB/Gray U8/U32/half perceptual source/edit cases with the same explicit
Normal28 linear-space operators and unit owner-aligned masks. U8 is XCF11
precision175; U32 and half are XCF12 precision375/575. Native half bits widen
exactly into F32 storage; source precision remains immutable provenance.
144 authored and144 fresh native controls are exact, with zero derived repeat
variation and pixel-exact default/faithful PNG repeats. U8 derived native,
matched nonlinear display and faithful PNG maxima are0.499081/0.664135/0.664167
per255; U32 maxima0.000109/0.000165/0.002396 remain within1/65535. Half maxima
0.062256/0.071384/0.070281 use only its native unit rounding quantum2^-12 plus
1/65535 and separate SC-0022/255 display bound. The rejected tight Float
U8 diagnostic remains recorded; existing Float thresholds and math are intact.
The packet and private consumer preserve low-code/half markers, transfer/current
profile context and material mask controls with cancellation/work bounds.
Run the explicit ignored corpus gate after reviewing the unpublished packet.
No arbitrary profiles, HDR/nonunit coverage or unequal masks are admitted.


Assigned Perceptual Normal and masks have a separate, independently authored
32-file RGB/Gray F32/F64 gamma-2.2 profile qualification. The current effective
Perceptual ICC is used for rendering; the immutable assigned ICC is retained as
source provenance and is the embedded ICC of the faithful PNG references.
Native coded, matched default-space display and faithful PNG maxima are
0.000097, 0.001936 and 0.025422 per 255, respectively. Native and matched display
comparisons retain the 1/65535 bound; faithful PNG uses the separate SC-002
2/255 display bound. Source, mask and ICC bytes remain exact, including F64
markers. The explicit ignored gate is
`gimp_normal_mask_assigned_perceptual_original` with feature `corpus` and
`PHOTOCRAFT_XCF_FULL_CORPUS` pointing to the reviewed unpublished full corpus.
An always-on current-profile regression checks RGB/Gray Linear and Perceptual:
invalid, missing or wrong-model current ICC fails even when source provenance
is valid; malformed original provenance remains inert under valid current ICC.
This domain does not qualify arbitrary assigned profiles, nonunit alpha, HDR,
unequal masks or profile-defined blend/filter transfer semantics.

Assigned gamma-2.2 Linear Normal masks also have 32 independently authored
RGB/Gray F32/F64 source/edit references. Exact native assignment and fresh
reopening controls are preserved. Native coded and matched default-space
display maxima are 0.000026 and 0.000882 per 255 under unchanged 1/65535
bounds; faithful original-profile PNG has maximum 0.018172 per 255 under
the separate SC-002 2/255 display bound. Default/faithful fresh PNG variation
is retained at at most one 16-bit code; it is not described as pixel exact.
Use the explicit ignored `gimp_normal_mask_assigned_linear_original` corpus
gate. The qualified profiles are our exact own RGB624/Gray448 gamma-2.2
definitions; arbitrary profiles and profile-defined blend/filter transfer
semantics remain outside this domain.

Own differing-channel TRC profiles have separate 32-file RGB/Gray F32/F64
Linear and Perceptual Normal-mask qualifications. Native data and ICC bytes
are exact; original RGB676/Gray436 definitions are retained independently
from current effective profiles. Linear native/display/faithful PNG maxima
are 0.000026/0.000882/0.073398 per 255; Perceptual maxima are
0.000097/0.001936/0.056366 per 255. Native and matched default-space display
retain 1/65535; faithful original-profile PNG uses separate SC-002 2/255.
Explicit ignored corpus gates are `gimp_normal_mask_channel_trc_linear_original`
and `gimp_normal_mask_channel_trc_perceptual_original`. These exact-profile
domains do not generalize profile-defined blend/filter transfer semantics.


Own sampled table-TRC Normal masks have a separate 32-file RGB/Gray F32/F64
NonlinearProfile qualification. Source pixels, masks and assigned ICC bytes
are exact, including native-format reopening. Checked consumer and actual
import/native/reopen gates compare both profiles in the same display space:
default display maximum 0.199897 per 255 and faithful embedded-profile PNG
maximum 0.201090 per 255 satisfy SC-002 2/255; derived alpha stays 1/65535.
Derived original-profile code maximum 2.423110 per 255 and the prior tighter
display failure remain executable negative diagnostics. Independent public
unchanged flat/Normal controls demonstrate noninjective ICC inverse-code
roundtrips; no global Curve convention or earlier profile gate changed.
Original evidence and negative records remain frozen; supplementary acceptance
has its own manifest. Run explicit ignored gimp_normal_mask_table_trc_original
and IO xcf_table_trc_normal_original gates. This domain covers only our exact
RGB25264/Gray8668 table ICCs and the measured unit Normal/mask configurations.


Own piecewise ICC type4 RGB/Gray profiles also have 32 independently authored
F32/F64 Nonlinear Normal-mask source/edit observations. Native assignment,
source/mask/ICC, .pcraft reopening and current profile context stay exact.
Default matched display and faithful PNG maxima are 0.205166 and 0.208754
per 255 under the new profile domain's SC-002 2/255 gate; derived alpha has
its separate 1/65535 bound. A fresh public projection repeat difference of
5.848706e-7 is retained; all 160 fresh native controls are exact. These are
original quantified type4 parameters, not fitted curves or copied assets.
Run explicit ignored gimp_normal_mask_parametric_trc_original and actual IO
xcf_parametric_trc_normal_original gates. Admission covers only exact own
RGB764/Gray492 profiles and the measured unit Normal/mask configurations;
there are no new global CMS curve semantics or arbitrary-profile claims.


All 64 table/type4 source and independently edited XCF states also have registered
engine command/history coverage against separately authored public GIMP controls:
layer.translate, layer.setProps opacity, and xcf.composition.set explicit Normal
operators. Source raster/mask pixels, ICC/current context and opaque source records
stay exact; native round trips, failed-edit atomicity, undo and redo are checked.
The command controls preserve mask pixels and do not claim a raw mask replacement
interface. Run ignored engine xcf_profile_normal_edits. Local 24 MP release Normal
kernel measurements: sampled-table RGB Union/ClipBackdrop 1.95/2.14 s, Gray
1.73/1.76 s; type4 RGB 5.95/6.63 s, Gray 3.26/3.21 s. ClipLayer/Intersection
0.19-0.22 s. These profile-specific measurements change no kernel or coefficients.


The same own type4 RGB764/Gray492 ICC definitions now have separate Linear32
and Perceptual32 source/edit packets. Actual native assignment, fresh samples,
mask planes and ICC bytes are exact; the original tight coded/default 1/65535
checks remain unchanged for these two domains. Prepared32 maxima per 255 are
Linear coded/default/faithful PNG 0.0000252/0.000881/0.07918 and Perceptual
0.0000963/0.001936/0.42957. Both actual importer/native-reopen pipelines pass.
A separate frozen 64-state public command-control packet verifies registered
layer.translate, layer.setProps and xcf.composition.set, immutable source/current
profile context, native mask/raster retention, .pcraft reopen, undo/redo and
atomic rejected policy changes. The controls preserve mask pixels and do not
claim a registered raw-mask replacement interface or add admitted file counts.
Run the two parametric_trc_{linear,perceptual}_normal_original IO tests and
xcf_profile_normal_edits with --features corpus -- --ignored. The two original
packets contain 64 unique XCFs with zero sibling-hash overlap; their command
controls are separate evidence. No arbitrary profile, HDR/nonunit coverage,
unequal-mask or profile-defined blend/filter admission follows from this slice.


The assigned-profile Normal/mask consumer now has a semantic analytic-class
qualification rather than a list of profile hashes. CMS defines the bounded
RGB/Gray matrix/Identity/Gamma/para0/3/4 class in its README; source assignment
is limited to NonlinearProfile F32/F64 and the measured two-raster, unit-alpha,
owner-aligned mask/operator geometry. The persistent composition identity
`xcf12-analytic-normal-mask-v1` validates the active current ICC on preparation
and profile changes. Immutable original ICC provenance never overrides current
interpretation. Incompatible assign/convert/discard operations fail before
changing document or history; an Off opening policy keeps the profile and
reports the constraint.

Six independently authored gamma/type3/type4 profiles vary RGB primaries and
RGB/Gray transfer parameters. Their 96 saved source/edit originals pass actual
import, exact native raster/mask/ICC retention, .pcraft reopen and matched PNG
comparisons. Prepared coded/default/faithful maxima per 255 are gamma
0.133521/0.056000/0.133463, type3 0.208423/0.068964/0.208754 and type4
0.203023/0.255803/0.204756. The SC-002 display bound stays 2/255 and alpha stays
1/65535; earlier implicit and exact-profile gates are unchanged. A separate
96-state fresh public control packet verifies registered offset, opacity and
operator edits, undo/redo, source preservation and current-profile negatives.
It does not claim a raw mask replacement command or add admitted file counts.
All 5,908 frozen artifact hashes remain exact; the 96 originals have no overlap
with 4,515 sibling original hashes.

Run `gimp_normal_mask_analytic_class_original` (compose),
`xcf_analytic_class_normal_original` (io), `xcf_profile_normal_edits` and
`xcf_analytic_class_current_profile` (engine) with `--features corpus -- --ignored`.
The release 24MP Normal Union kernel measurements, not full-frame rendering,
are gamma RGB/Gray 4.061/2.680s, type3 6.466/3.222s and type4 6.277/3.163s.
Tables/LUTs, profiles outside the class, Linear/Perceptual source assignment,
HDR/nonunit coverage and unequal mask bounds remain separate qualification work.


A supplementary 32-state public control packet now verifies accepted current
profile assignment and relative-colorimetric conversion (BPC off) from the
analytic gamma sources to independently authored type3 profiles. Registered
commands preserve immutable source ICC provenance, masks and alpha bits;
assignment also preserves all native color bits. Both operations pass matched
projection, .pcraft reopen and undo/redo checks. Conversion legitimately creates
negative current color samples (minimum -1.115929 in the public controls); those
samples are retained. Derived native colors differ from the independent GIMP
conversion by at most 0.000181221 and are not claimed bit-identical. These are
current-command controls, not additional admitted originals or blanket HDR,
profile, rendering-intent or black-point-compensation qualification.

Analytic ICC boundary controls additionally cover 64 independently saved RGB/Gray F32/F64 Normal/mask originals with gamma 0.75/3.5/identity and near-join parametric 3/4 curves. Exact source samples, masks and ICC bytes remain lossless; unchanged SC-002 matched display gates are <=2/255 and alpha <=1/65535. Prepared maxima are default display 0.585429/255 and faithful PNG 1.094429/255. The gamma cohort retains an independent repeat diagnostic: default PNG codes differ by 167/65535 and public projection by 0.005315967, while faithful transparent-preserving PNG repeats exactly. These observations do not change Curve math or earlier profile/implicit test tolerances. Separate public offset/opacity/operator command controls add no original-file count.

Analytic Linear/Perceptual Normal-mask qualification resolves comparison transforms from the active current ICC definition plus declared sample encoding. Canonical analytical transfer and its exact serialized ICC coefficient rounding describe the same stored-sample interpretation; reparsing rounded coefficients as a second transfer is not the comparator. Original ICC is immutable provenance and never a current-profile substitute. New source/native/PNG evidence keeps the existing display and alpha gates unchanged.

### Bounded analytic Normal mask geometry

The persistent `xcf12-analytic-normal-mask-geometry-v1` contract admits the
independently observed two-Raster RGB/Gray F32/F64 nonlinear geometry domain.
Current canvas and declared drawable/mask extents are positive and at most
4096 pixels each, with at most 8192 drawable plus mask pixels in aggregate.
The full-canvas base remains at the origin with unit opacity and Union; the
foreground has an enabled owner-aligned ordinary mask, unit finite current
samples, Normal28 and explicit linear-space composition. Source-local mask
offset zero is retained as canonical world offset equal to its owner origin.
The checked constructor polls every 64 declared pixels without per-pixel
allocation; cancellation or invalid current geometry leaves source surfaces
unchanged. Current ICC and encoding are validated separately from immutable
source provenance. Older fixed-geometry epochs retain their previous contracts.
The corpus gate is `xcf_analytic_geometry_normal_original` plus registered
`xcf_profile_normal_edits` geometry controls and `xcf_analytic_geometry_current`,
using the explicit ignored unpublished-corpus gate. This bounded source/current
contract does not admit 24 MP geometry, unequal masks, HDR or nonunit alpha.

The separately observed Linear and Perceptual geometry counterparts use
`xcf12-analytic-linear-normal-mask-geometry-v1` and
`xcf12-analytic-perceptual-normal-mask-geometry-v1`. They retain the same
4096/8192 pixel limits, unit native samples and owner-aligned masks. Each
epoch binds the current sample encoding and operative analytical ICC class;
current profile stage resets are rejected before history installation.
Independent source ICC bytes remain provenance and faithful-PNG targets.

The bounded analytical Normal-mask geometry contract also has a separate nonlinear integer epoch, 'xcf12-analytic-integer-normal-mask-geometry-v1'. It admits U16/U32 only; the three floating geometry epochs retain F32/F64 only. Authoritative integer source/mask bytes and U32 low-bit markers are exact. Independent derived display and faithful PNG comparisons retain SC-002; source assignment and fresh native controls remain separate from projection rounding. Geometry still caps each plane at 4096 pixels and aggregate at 8192, with checked f64 reads and interruption polling every 64 pixels.

### Accounted ordinary Normal scenes

The separate `xcf12-analytic-ordinary-normal-mask-v1` contract retains the
nonlinear analytical ICC, two-Raster Normal, unit-sample and owner-aligned mask
requirements while replacing the small-scene pixel ceiling with an explicit
job ledger. The independent source/edit witnesses have a 513 by 385 canvas
and a 269 by 193 mask/foreground crossing opposing canvas edges, with
RGB/Gray U16/U32/F32/F64 samples. Larger mathematical bounds are subject to
resident and work denial, not an assertion of independently verified fidelity
at every size. Native large-document saving remains a separate capability.

`PreparedRenderer::new_budgeted` retains both the caller's already-accounted
source lease and a preparation lease through rendering. Its preparation scans
poll and charge work in at most 64-pixel chunks. The unaccounted constructors
refuse this epoch. Import transactions supply their existing shared budget;
loaded-document callers can use `gimp_render::scene_budget::prepare` with
explicit `SceneLimits`. This scoped source census covers retained raster tile
and default payloads plus admitted metadata capacities; unsupported resource
collections fail closed. It does not measure allocator overhead or process RSS.

`ScenePrepared` charges source, preparation and returned bands on one ledger.
`SceneBuffer::into_parts` transfers the buffer with its lease;
`into_buffer` requires the recipient to assume persistent accounting without
cloning. `SceneBudget::reserve_external_bytes` lets an export destination or
codec buffer coexist on the same ledger. Reserve its capacity before allocation
and retain the lease until that capacity drops or transfers to another charged
owner. Observer failures refund only their own output reservation, preserving
unrelated or nested output leases. Cancellation, resident denial and work
denial leave authoritative source pixels unchanged.

Accounting is scoped to preparation and rendering. A canvas upload may retain
only the returned output lease after `ScenePrepared` drops; the caller still
owns its document, and this helper does not claim a process-wide charge for
that persistent owner. Flat export instead retains `ScenePrepared` and the
codec destination lease through every band. Returned output lifetime and
persistent document lifetime are separate accounting responsibilities.

### Streamed current Box Blur

`PreparedBackedBoxContext` and `PreparedBackedBoxDocument` render the admitted
CURRENT/native float Box recipe over checked drawable and canvas geometry.
They share the existing Box arithmetic: taps clamp to the original drawable,
the output expands by the radius, and each canonical 256-pixel tile completes
before an ROI is cropped. Implicit RGB F32/F64 and assigned RGB F32 retain their
existing profile, raw-field and radius contracts. Source/eager constructors
keep their previous admission rules.

Packed input and kernel scratch have a 16 MiB guard. Box preparation, its
current band and overlapping renderer scratch have a 32 MiB guard, including
wrapper headers and profile preparation peaks. Persistent input/provider
caches, previously returned bands and the generic reduced-preview caller's
output/accumulator keep their real shared-quota leases; these are outside
that Box temporary guard. This does not make decoded XCF input lazy or bound
total document/process memory.

Work uses the caller's single cumulative control. Full enabled validation of
24–36 MP currently exceeds the normal registered/open work allowances, so
large-image publication remains unfinished even where individual ROI bands
fit. The new regression groups are authored but remain UNRUN/uncompiled during
the implementation-first pass; no numerical or performance qualification is
claimed.

`LeasedStreamedBoxPipeline::new_geometry_budgeted` is the separate original-XCF
preparation route for `xcf22-observed-box-geometry-v1`. Its implicit RGB F32
nonlinear source envelope requires enabled Box radius4/5, Replace62 at unit
opacity, region/clipping zero, no auxiliary input and an actual canvas-sized
numeric-zero physical mask. Three stored effect-policy words remain opaque
provenance with no resolved enum interpretation. Checked geometry is generic;
the independent GIMP controls establish the observed behavior rather than a
filename or coordinate whitelist. Older source constructors retain their exact
contracts. The old source identity keeps selection priority and final refusal.

This adapter checks retained source tiles/defaults and every stored mask word,
then retains paid profile/working-space preparation. It creates no full filtered
Surface. Promotion still requires complete CURRENT validation of the actual
native owner and expanded output. This observational epoch currently refuses
mutations while supporting viewing and native continuation. Its drafts are
UNRUN/uncompiled, with no independent negative-zero mask qualification.

Box host services now select the existing standard policy work ceiling
(native2^40, wasm2^36) and active-operation deadline (120 seconds), retaining
one used counter, clock and cancellation owner. External caller controls stay
authoritative. Every provider debit remains charged; these are hard maxima,
not sufficient-work or throughput promises. Native/XCF ingress still has its
legacy8-billion bootstrap before the actual Box owner is known. The clock is
retained across it, but the selected deadline is enforced only after selection;
an already expired operation then fails immediately. Existing native logical
extent limits and retained-input costs remain separate large-image work.
