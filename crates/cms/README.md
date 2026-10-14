# photocraft-cms

Pure-Rust ICC colour management for PhotoCraft. No workspace dependencies (L0, publishable),
no C, no `unsafe`. Implemented from the public ICC.1:2010 (v4.3) and ICC.1:2001-04 (v2)
specifications, CIE 15, and Adobe's published black point compensation paper.

## API

```rust
use photocraft_cms::{Builtin, Intent, Profile, Transform};

let src = Profile::parse(&icc_bytes)?;                    // v2/v4: matrix/TRC, gray TRC, mft1/mft2/mAB/mBA
let dst = Builtin::CoatedCmyk.profile();
let t = Transform::new(&src, dst, Intent::RelativeColorimetric, /*bpc*/ true)?;
t.convert_u8(&rgba8, 4, &mut cmyka8, 5, /*copy alpha*/ true);   // also convert_u16 / convert_f32 / apply (in place)
let mut out = [0.0; 4];
t.eval(&[1.0, 0.0, 0.0], &mut out);                        // exact single colour
```

* `Transform::proof(src, proof, display, intent, bpc, simulate_paper)`: soft-proof chain.
* `GamutCheck::new(src, proof, ΔE)`: out-of-gamut test (gamut warning).
* `Lut3d::from_transform(&t, 33).to_rgba16f_bytes()`: display transform as a 3D texture.
* `transform::cached(src, dst, opts)`: process-wide transform cache.
* `Profile::to_bytes()`: original bytes of parsed profiles (byte-exact), or an ICC v4.3 encoding.
* `Profile::same_colors(&other)`: the same colour space whatever the encoding (every sample of
  a device grid within ΔE 0.5 through both), e.g. Photoshop's sRGB IEC61966-2.1 and the built-in
  sRGB. `content_hash()` compares bytes and suits cache keys only.

### XCF source interpretation and wide samples

`xcf_color::SourceColorContext::new` requires the effective source and working ICC bytes
explicitly, alongside optional assigned ICC bytes, color model, source/working `Encoding`,
`SamplePrecision` (U8/U16/U32/F16/F32/F64) and `NonFinitePolicy`. It retains byte-exact original
profiles and assignment absence separately from derived transfer interpretation. RGB matrix/TRC
profiles may use unequal channel curves, sampled tables, identity/gamma or ICC parametric forms
0–4; gray TRCs are supported too. Invalid tag bounds, singular matrices, incompatible models
and invalid curves return errors. The established `effective_profile(None, …)` API still supplies
the known GIMP default; the new context API never chooses a default on behalf of its caller.

`convert_encoding_f64` converts source transfer in the same primaries without changing alpha.
`Transform::try_eval_f64` and `SourceColorContext::to_working_f64` retain f64 arithmetic through
matrix/TRC, CLUT interpolation and analytic PCS stages and commit output only on success.
Existing f32 conversion APIs remain available for their existing paths;
they are not authoritative U32/F64 conversion APIs. ICC sampled table entries retain their ICC
precision while interpolation uses f64, without narrowing the input coordinate. Parsed ICC
integer curves use `Curve::Table64`; parsed CLUTs retain exact normalized integer nodes alongside
the existing f32 preview representation. `Clut::try_new_f64` and `try_eval_f64` expose checked
wide construction/evaluation. RGB uses tetrahedral interpolation, four inputs use two tetrahedra
and linear interpolation, and other dimensions use multilinear interpolation. Original parsed
ICC bytes remain the embedding authority. Public edits to f32 CLUT data override the original
wide node. Valid RGB/gray LUT-only profiles retain their original profile-defined pipeline
under `NonlinearProfile`. Linear/perceptual transfer overrides return an actionable unsupported
error because a LUT does not supply separable tone curves. Same-encoding identity preserves
samples; changing encoding never invents a replacement sRGB curve.

Finite HDR samples are retained unclipped. Nonfinite floating data may be admitted for original
bit preservation only under explicit `PreserveBits`; color evaluation rejects nonfinite values.
CMS does not encode raw samples or rewrite NaN payloads. The storage codec owns exact bits and
quantization, and callers own allocation/work accounting, cancellation and derived display policy.

`Transform::try_convert_bytes_wide` converts native-endian U32 or F64 buffers with dynamic
source/destination channel counts and exact extra-channel copying. Strides count samples.
Each call accepts at most 64 KiB per buffer; callers divide larger tile/transaction staging at
whole-pixel boundaries. Output changes only after successful conversion. The `WideWork`
callback admits staging bytes, accounts sample work, polls cancellation per pixel, and releases
the reservation on success/error. No pixel may exceed 4096 sample-kernel units. U32 samples
normalize through f64 and quantize explicitly on output; out-of-range output fails instead of
implicitly clipping HDR. F64 copied extras retain original bits, including NaN payloads only
under explicit `PreserveBits`; nonfinite evaluated color channels fail.

Kernel measurement (2026-10-10, Windows native release): streaming 24 MP RGB through a cached
17³ identity CLUT took 408.4117 ms on the existing f32 reference and 1.0139503 s on checked f64
interpolation (2.483×). This excludes image allocation and ICC construction and is not a full
import, display or RSS result. Reproduce with `cargo test -p photocraft-cms --release --test bench
bench_wide_rgb_clut_24mp -- --ignored --nocapture`.

### Controlled current implicit-profile binding

`xcf_color::effective_implicit_profile_requirements_with_control(bound_icc, space,
encoding, limits, cancelled)` returns a borrowed prospective `EffectiveProfileRequirements`
envelope for explicit RGB or Gray only. It conservatively admits the bound bytes and the
analytical effective-model construction before input-sized copies. Requirements are not
an equality proof: even unrelated same-sized ICC bytes can pass that admission.

`effective_implicit_profile_with_control` consumes the same planner, constructs the exact
analytical definition for the declared current encoding, and compares its generated ICC
bytes with the binding. A mismatch returns an error. The binding is limited to 16 MiB;
`control::Limits` may only lower the existing storage/work ceilings. Callers retain current
identity-marker validation, any spare input capacity accounting, and original assigned-ICC
absence separately; neither helper infers an implicit marker from source provenance.
Builtin profile `OnceLock` initialization remains a provider boundary with outer cancellation
polls, rather than a claim of controlled initialization or complete process-memory accounting.

### Evaluation

* Exact path: a float stage pipeline (curves, matrices, CLUTs, Lab↔XYZ, PCS encodings, BPC /
  absolute scaling). Float buffers use it by default (`TransformOptions::precise_float`).
* Integer buffers: matrix/TRC chains use per-code input tables + matrix + output tables indexed
  by `x^(1/4)` (accurate near black even for pure gamma curves); chains containing a CLUT are
  sampled into a device link (33³ for 3 inputs, 17⁴ for 4, 4096 for 1) evaluated with
  tetrahedral interpolation (4D: linear on the first input between two tetrahedra); purely
  analytic chains (e.g. Lab ↔ RGB) are evaluated exactly. Buffers ≥ 16k pixels run on rayon
  (native only).
* Rendering intents: AToB0/1/2 and BToA0/1/2 by intent with AToB0/BToA0 fallback; absolute
  colorimetric scales by media white (`wtpt`; v2 display profiles treated as D50; a zero,
  negative or non-finite white is refused with `CmsError::Invalid`); BPC uses
  neutral black points (Y only) mapped linearly in XYZ.

## Built-in profiles (all CC0-1.0, generated by this crate)

| id | description |
|---|---|
| `srgb` | sRGB IEC61966-2.1 (Rec. 709 primaries, D65 Bradford-adapted, sRGB TRC) |
| `display-p3` | Display P3 |
| `adobe-rgb-compat` | Adobe RGB (1998)-compatible (published primaries, gamma 563/256) |
| `prophoto-compat` | ProPhoto-compatible (ROMM primaries, D50, gamma 1.8) |
| `linear-srgb` | sRGB primaries, linear |
| `rec2020` | Rec. 2020 primaries, Rec. 709 OETF |
| `gray-gamma-2.2`, `sgray` | gray gamma 2.2; gray with the sRGB curve (default gray working space) |
| `lab-d50` | Lab identity (v4 encoding) |
| `coated-cmyk` | **Photocraft Coated CMYK (synthetic, 300% TAC, medium GCR)** — default CMYK |

### The CMYK profile

Adobe's CMYK profiles (U.S. Web Coated SWOP, Coated FOGRA39) are proprietary, and the freely
downloadable characterisation-based profiles (ECI/FOGRA, colord, Ghostscript) carry
redistribution conditions or unclear licences, so Photocraft generates its own
(`src/synth.rs`): a Yule–Nielsen modified Neugebauer model (n = 2) over the 16 overprints with
ISO 12647-2 coated-paper Lab aims for paper/C/M/Y/RGB overprints, parabolic dot gain (14 % CMY,
17 % K at 50 %), GCR black generation (starts at 20 % gray component, K max 95 %), total ink
limit 300 %, perceptual tables with L* black-point scaling and soft chroma compression against a
sampled gamut boundary. `profiles/photocraft-coated-cmyk.icc` (237 KB, AToB 11⁴ lut16, BToA
21³ lut16) is that output, dedicated to the public domain (CC0-1.0). Regenerate with

```sh
PHOTOCRAFT_REGEN_PROFILES=1 cargo test -p photocraft-cms --release --test regen
```

It is a plausible coated-offset profile, not a measured characterisation: use your printer's
profile for production print.

## Not implemented / approximate

* v4.4 `mpet` (multiProcessElements, D2Bx/B2Dx) tags, named colour, device link and abstract
  profiles as transform endpoints, `chad`-based adaptation of v2 absolute colorimetry.
* BPC destination black uses the BToA/AToB round trip directly (Adobe's curve fit for
  ill-behaved LUT profiles is not implemented); v2/v4 perceptual reference black mixing is not
  modelled beyond the PRMG black for v4 LUT profiles.
* Profile ID (MD5) is not computed when writing (left zero, which the spec allows).

Active implicit working-profile identity is separate from immutable source
provenance. `xcf_color::implicit_source_profile(bound_icc, space, encoding)`
returns the analytical nonlinear sRGB/sGray source basis;
`effective_implicit_profile` returns that implicit profile with the current
stored encoding applied analytically. Both require the supplied current ICC
bytes to match the CMS-generated effective implicit ICC exactly and reject
mismatches/model errors. The caller must also validate the document's active
interpretation marker and current byte binding; an absent original assignment
alone cannot establish current implicit status after editing.

These APIs retain analytical transfer constants which ICC s15Fixed16
serialization rounds. Tests prove exact low-tone RGB/Gray transfer through all
three sample encodings, unchanged bound bytes, and rejection of stale/arbitrary
profile bytes. Normal `Icc` interpretation continues to parse the assigned
bytes as authoritative; no profile is automatically relabeled implicit.

`xcf_color::effective_profile_from_definition(&Profile, Encoding)` applies
sample interpretation without a serialize/reparse round trip. It validates
finite/model/curve semantics before cloning, caps tables at the ICC reader's
1,048,576-node limit, preserves parsed assigned table definitions and bytes for
NonlinearProfile, and keeps analytical implicit matrices/transfers intact.
Checked prepared composition uses this seam, including its Lab transform path;
a regression distinguishes analytical Lab output from a serialized/reparsed
profile while proving unchanged assigned unequal tables.

ICC transfer curve selection now follows the declared device model: Gray uses kTRC, RGB uses its channel curves, even if a valid profile retains unrelated curve tags. The parsed profile bytes/tags remain untouched. A failing-first regression verifies parsed Gray with extraneousRGBtags, exact f64 expectedGray conversion/roundtrip, checkedf32/legacyf32 agreement, unequalRGB curves with extraneousGraytags and profile identity. Complete CMS79tests pass, strictalltargetsclippy andwasm pass.


`xcf_color::qualify_analytic_profile(&Profile)` is an additive, semantic
qualification predicate for a bounded analytic ICC class. It compares no hash
or profile name and neither rewrites source bytes nor changes `Curve` math.
The initial class is RGB/Gray Display, XYZ PCS, D50 white, versions2/4 without
LUTs or nonzero black offsets. Parsed headers must declare their actual length
(132..65536 bytes) and XYZ explicitly. RGB colorants must be nonnegative,
D50-normalized within four s15Fixed16 LSBs, determinant >=0.01, and inverse
absolute row sum <=16. Curves are Identity, Gamma or ICC para0/3/4, with gamma
0.75..3.5; piecewise a0.25..2, b/e/f-0.25..0.25, d0..0.25 and c0..4. A real
lower branch needs positive c>=1/65536. The actual branch join must agree within
four fixed-point LSBs; stored rounding in either direction is retained.
Zero/one endpoints must agree within two/four LSBs. Analytic checks inspect
actual branch locations, including narrower branches than a sampling grid.
Tables/LUTs and definitions outside these bounds remain valid possible ICCs,
but this predicate returns actionable unsupported rather than a transfer fallback.

The equations follow [ICC parametricCurveType](https://www.color.org/specification/ICC.1-2022-05.pdf)
and its [parameter guidance](https://www.color.org/whitepapers/). New synthetic
regressions cover tiny plateaus/jumps, malformed headers, non-D50/singular
matrices, model-specific Gray kTRC, immutable definitions and analytic inverse
boundaries. Public GIMP gamma/type3/type4 holdouts vary parameters and primaries;
consumer qualification is recorded separately from this mathematical predicate.

`xcf_color::qualify_effective_analytic_profile(profile, encoding)` checks active current ICC interpretation without immutable source substitution. It keeps the existing semantic D50 matrix/shaper class. Linear accepts only operative identity transfer (Identity, Gamma(1), or parametric type0 gamma1); Perceptual accepts only canonical sRGB parametric type3 coefficients, analytical or their exact ICC s15Fixed16-rounded array. One additional coefficient LSB is outside this exact effective interpretation. Gray uses only kTRC; unrelated RGB tags are inert. This helper does not admit new source epochs or change Curve evaluation.
