# photocraft-xcf

Standalone safe Rust reader for GIMP XCF and one gzip/bzip2/XZ transport wrapper. Filesystem and UI integration belong to
`photocraft-io`. The reader never writes XCF or executes linked resources or GEGL effects.

The compatibility API is `is_xcf(bytes)` and `decode(bytes, &DecodeOptions)`. `is_xcf` recognizes
only definitive plain XCF; `classify_container` recognizes outer transport magic without claiming
arbitrary compressed content is XCF. `decode_with_resources` returns original container provenance,
encoded/logical lengths and parsed file under a caller-selected lower-only policy. Failures include
an error category and byte offset. Tight caller budgets and cancellation are supported.

Implementation follows the public [GIMP XCF specification](https://developer.gimp.org/core/standards/xcf/).
Version recognition does not imply support for every feature. Unsupported rendering semantics
are represented explicitly for the document adapter to reject.
Known singleton properties reject duplicates instead of silently overwriting
state; repeated parasites, set declarations/memberships and arguments remain
collections. String sizes count UTF-8 bytes, including the final NUL; embedded
NULs and malformed encodings are errors. Negative guide entries before XCF15
are ignored semantically while their raw source property remains retained.
Effect arguments distinguish an explicitly stored null color from a color with
samples; this does not admit the effect for rendering. GimpText singleton fields
and font metadata reject duplicate keys, while the shared effect configuration
tree retains intentionally repeated channel selectors.

Historically qualified baseline import covers RGB, grayscale and indexed images; U8/U16 and finite F16/F32;
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
representations and are rejected. Parser recognition remains separate from consumer
admission: typed independent X/Y resolution, text/vector/link/filter and attachment
records do not by themselves make their rendering editable. `photocraft-io` and the
compositor admit explicit qualified domains and reject unknown required semantics,
including hidden/disabled activation. Do not interpret the historical baseline as a
blanket rejection of every current special-layer lane, or use a retained stored raster
as the live special-layer result. Arbitrary GEGL operations and unqualified source
contexts remain unsupported.
The parser additionally preserves U32/F64 bytes exactly through uncompressed/RLE/zlib tiles;
adapter, native-migration and editing qualification for these depths is tracked separately.
One gzip/bzip2/XZ wrapper is decoded with reviewed safe Rust backends; checksums, EOF and
unrelated trailing content are checked. Concatenated members form one logical plain XCF stream;
recursive wrapping fails. XCF writes remain outside the importer.

`decode_candidate` performs one strict unwrap and returns `None` only for unrecognized
containers or successfully decoded non-XCF content (for example SVGZ). Malformed actual
XCF and wrapper errors remain errors, preserving content-first routing.

Physical units, custom unit strings/f32 factors and independent X/Y resolution are typed;
the documented unreliable custom-unit payload length is retained alongside actual bytes.
Drawable/effect/image raw properties and named operation/version records are preserved.
Effect envelope state exposes explicit stored presence for enabled, opacity, mode,
spaces/operator and outer clipping; signed out-of-domain values are preserved, never
normalized into Auto. Item state retains tattoos, selected layer/channel markers,
legacy linking, group flags, color tags, mask edit/show state, channel overlay colors,
inert item-set declarations/membership and floating attachment record identities.
Floating target and unequal mask-bound evaluation use separate checked consumer
lanes; retained parser fields alone do not qualify those lanes.
Selected paths have scoped original-file IO/native state tests; that evidence
does not qualify vector-layer regeneration or unknown path geometry epochs.
These fields do not remove unsupported-rendering markers.
Saved vector paths now expose bounded strokes and exact point component bits for
modern standalone records, legacy `PROP_VECTORS`, and legacy `PROP_PATHS` formats
1–3. Selection, tattoo, lock/color state, inert parasites and set membership are
retained. Legacy integer coordinates and point-tag epochs remain explicit rather
than being silently normalized. Path editing and vector-layer evaluation remain
separate consumer work.
Vector-layer tag47 exposes the modified-cache flag, path tattoo, fill/stroke flags,
exact width/miter/dash bits, cap/join styles and encoded colors with original ICC
bytes. Pattern identities remain inert even when present beside a solid-color style.
Unknown required styles or configuration tails return explicit errors. Genuine
unmodified originals may regenerate to different bounds in GIMP; their stored raster
is retained as source cache, never admitted as the live vector result.
Effect argument envelopes expose exact primitives, IEEE bits, config text and color
encoding/sample/ICC bytes. Unknown types retain their original envelopes.
GimpConfig arguments additionally retain an ordered bounded AST with source
spans and numeric spellings; observed Levels/Curves forms expose repeated channel
records, clamps/TRC, exact point/sample arrays and corner types. GEGL chain strings
retain bounded nested syntax, assignments and references as inert records. Graph
IDs, resources and operations are never resolved or executed here; unknown required
configuration shapes remain explicit errors, and effects retain unsupported markers.
Remaining configuration variants, resource resolution and effect evaluation remain
separate pending work.
Linked path/dimension/flag records and finite transform coefficients/interpolation are
typed and retained without resolving paths. The independently observed stock link
payload length discrepancy is handled by its known field layout. Explicit owned-resource
linked rendering/editing has bounded consumer lanes outside this parser.
Both records require XCF 25 or later. Custom-name and monitoring-disabled bits are
exposed as source metadata; unknown flag bits retain a named unsupported marker.
Monitoring metadata never enables filesystem access. Pre-transform offsets, exact
matrix bits and original raster caches remain preserved. Placement/interpolation
commands operate on qualified current owned material without rewriting these originals.
Text parasites retain exact source bytes, bounded config trees/byte spans, requested
font metadata, optional numeric layout fields and original modified-raster flags.
Pango markup is decoded through pinned quick-xml with 4096-byte input polling;
run boundaries are checked UTF8 byte offsets and styles remain ordered inert declarations.
DTD, external/unknown entities and unknown required markup fields fail explicitly.
Unknown text/font configuration fields remain visible in `unknown_fields`. Original
raster caches are retained; text shaping, font substitution, full color/outline/config
domain qualification and editable rendering remain separate pending work.

`Source` provides U64 random access through checked 64 KiB windows. `ByteSource` borrows bytes;
native `FileSource` accepts an already authorized file handle and never resolves paths. It is
not an immutable snapshot: the I/O layer must enforce source stability and race policy.
`ResourceLimits` validates contract limit names and rejects raises/unknown keys before I/O.
`Budget` provides RAII component/resident reservations, monotonic work and cancellation/deadlines.
Sample conversion charges work in at most 4096-unit slices; allocations and wrapper backends poll
cancellation. `native_large` is disabled pending large-file qualification. The current parser
materializes complete planes and wrapper output in memory; private native spooling, whole-job
scratch/cache accounting and measured 50 ms backend responsiveness remain open.

Limits remain enforced even when callers request higher ceilings:
262144 pixels per side, 2^30 cumulative drawable pixels, 10000 drawables, 100000 properties,
1 MiB strings, 16 MiB ICC, 64 MiB individual properties, 256 MiB aggregate metadata,
100 group levels and 20000000 record/work visits. Encoded input is capped at 1 GiB native
or 256 MiB wasm; resident accounting at 2 GiB native or 512 MiB wasm includes borrowed
input, staging, padded output tiles, metadata and indexed provenance. Lower caller
budgets and cancellation apply throughout parsing and mapping. Decoding is serial on
both targets; parallel decoding requires a measured reason before introducing more scratch.

## Current application integration boundary

XCF remains import-only: save continued edits to `.pcraft`, not back to the original
XCF or a renamed/wrapped source alias. The desktop interactive native save service
streams a prepared archive under standard admission, with all-open source exclusions
checked before preparation and atomic commit. Native stream reopen is content-first
and uses the same model/color-policy/source-notice installation as byte import;
negative native candidates preserve the existing importer route. Browser downloads
retain their bounded byte-output path. No route restores serialized filesystem grants.

Current Properties code includes explicit linked Exact recovery/Reduced substitution,
owned-material Refresh, guarded placement/interpolation, lossless string-ID layer
rename and Named item-set controls. They dispatch shared engine commands; the UI
snapshot cache and draft fields confer no resource or rendering authority. Saved
locators remain inert. Exact byte recovery is a resource-level statement, not Full
fidelity for the document. Search-rule authoring, unknown source epochs, arbitrary
operations and unsupported conversions are still refused.

`xcf.filter.graph.inspectPlan` is a read-only structural query for an actual retained
Graph argument. The UI requests bounded node/connection pages on demand and retains
a Weak/snapshot/revision stamp. Planning stored identities/ports does not validate
operation schemas or rendering. The parser's original graph/configuration records
remain unchanged; graph authoring is not provided by this inspector.

These are implementation descriptions. Recent regression drafts are unrun, and no
fresh whole-corpus, native/browser workflow, RSS or performance result is asserted
here. Historical evidence applies only to its recorded sources and domains.
`native_large` remains disabled. Standard ceilings and lower-only overrides still
apply to streaming paths; an archive stream alone does not qualify >4 GiB decode,
editing, ZIP64 continuation or complete resident/scratch accounting.

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

Stored GEGL chain parsing validates the documented operation sequence and nested `aux` syntax
without executing it. Named IDs/references must be unique, resolve to a node, and form an
acyclic graph; duplicate or ownerless ports fail with source-span diagnostics. Other nested
port syntax and reference aliases remain explicitly unsupported pending public schema evidence.
The original chain string remains authoritative source data. Scratch and actual implicit,
reference and auxiliary edges count against the same budgets, with cancellation during topology
construction and traversal. A structurally valid graph does not qualify its operations for rendering.

Color Balance configurations retain their three ordered tonal ranges, all nine exact
f64 parameter bits and the luminosity flag alongside the original configuration text.
Duplicate, incomplete or unknown required fields fail explicitly. This source-schema
decoder does not by itself qualify Color Balance evaluation or editable import.

`decode_candidate_with_budget` reuses the caller's `Budget` for wrapper peaks,
parser allocations, visits and performed sample work. `SourceAccounting::Borrowed`
reserves encoded and resident input bytes; `CallerReserved` requires the caller's
live leases over the exact immutable input. It does not reserve that source twice.
The returned `BudgetDecodedImport` separates the movable decoded fields from
`DecodeReservations`, which must stay alive through mapping. The parser's conservative
retained lease includes charged staging; the caller can adjust it with `resize_retained`
as ownership transfers. Growth is atomic and shrinking remains allowed after cancellation.
This seam does not yet prove complete mapper/kernel work accounting, transient scratch
recycling, large-policy qualification, or worst-case blocking handle-read latency.
