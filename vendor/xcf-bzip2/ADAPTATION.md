# Reviewed decoder adaptation

Adapted from bzip2-rs 0.1.2 (Paolo Barbolini), https://github.com/paolobarbolini/bzip2-rs.
Adjacent LICENSE-MIT and LICENSE-APACHE are retained unchanged. Historical randomization
uses the 512-entry public format table from libbz2-rs-sys 0.2.5 (Trifecta Tech Foundation,
derived from Julian Seward's bzip2); its full LICENSE-randtable (bzip2-1.0.6) is adjacent.

Only lib.rs, bounded.rs, bitreader.rs, crc.rs, huffman.rs, move_to_front.rs, randtable.rs,
block/{mod,bwt,error}.rs and header/{mod,error}.rs are compiled. Legacy decoder/reader
files must not be copied or compiled. The adaptation bypasses the original compressed-block
buffer limit, verifies block and combined stream CRC and zero alignment padding, supports
concatenated and historical randomized blocks, rejects unrelated trailing bytes, uses
fallible allocations, and polls selectors/Huffman/BWT/output every bounded work slice.

Integration exposes decode_with_reservation. Reservations are complete peak
requirements, held until transaction completion, not cumulative deltas. OutputOverlap
includes old and proposed new allocation capacities. Caller-owned encoded input is separately
charged. There is no public unreserved compatibility decode helper. Streaming U64 source/sink infrastructure remains a later integration task.

Exact runtime dependencies: crc32fast 1.5.2 (MIT OR Apache-2.0,
https://github.com/srijs/rust-crc32fast) and tinyvec 1.13.3 (Zlib OR Apache-2.0 OR MIT,
https://github.com/Lokathor/tinyvec). Original dependency licenses remain in Cargo metadata.
Only the audited default configuration is admitted; alternative legacy features fail compilation.
