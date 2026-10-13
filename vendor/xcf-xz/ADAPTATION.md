# Reviewed XZ decoder adaptation

Source: lzma-rust2 0.21.0, https://github.com/hasenbanck/lzma-rust2, Apache-2.0.
Adjacent LICENSE is original and unchanged. No GIMP/GEGL implementation was copied.

Retained compiled sources: lib.rs, decoder.rs, state.rs, range_dec.rs, crc.rs,
lzma2_reader.rs, lz/{mod,lz_decoder}.rs, xz.rs, xz/reader.rs, filter.rs,
filter/{bcj,delta}.rs, filter/bcj/{arm,ia64,ppc,riscv,sparc,x86}.rs.
Other copied files are not compiled and are not part of the integration whitelist.
Only std+xz features are admitted; encoder/optimization/lzip fail compilation. unsafe is forbidden.

Literal/range/BCJ/filter-holder allocations are fallible. Dictionary initialization polls
at 4096-byte intervals. Per-block admission includes dictionary, range buffer, literal
replacement overlap, every filter holder, BCJ buffers and conservative fixed scratch.
Index records are admitted before allocation. Dummy/invariant error paths return Result.
Raw/multithread/sans-I/O/encoder and recursive concatenation APIs are not admitted.

The public facade combines src/strict_xz.rs and src/bounded.rs. The raw XzReader
and Control trait are crate-private; only decode_with_reservation and Reservation are exposed. Structural
preflight is mandatory and provides complete footer/index/block/chunk/trailing validation;
backend decoding verifies payload CRC32/CRC64/SHA256. Input is already-owned and bounded.
Streaming/U64 infrastructure, shared job reservation ownership, process RSS and browser
qualification remain application integration tasks.

Packaging removes unused encoder/sans-I/O/preset/speculation/reader-introspection helpers,
including the crate-wide inherited dead-code allowance. Five field-level allowances retain
validated parsed block/index/footer metadata; strict preflight checks those fields authoritatively.
Runtime dependency source/license metadata: sha2 0.11.0 https://github.com/RustCrypto/hashes
(MIT OR Apache-2.0), crc32fast 1.5.2 https://github.com/srijs/rust-crc32fast (MIT OR Apache-2.0).
