# Bounded gzip facade

Original PhotoCraft Rust implementation from RFC 1952, MIT OR Apache-2.0; adjacent notices apply.
No miniz implementation was copied. Runtime uses miniz_oxide 0.9.1 allocation-free safe inflate
core with default features disabled and crc32fast 1.5.2. The fixed 32768-byte ring and
4096-output/65536-input limits bound each call. Optional headers are scanned without retained
vectors, with bounded field lengths and polling. Validate FHCRC, stream CRC, ISIZE, reserved flags,
concatenation and unrelated trailing bytes. Admission covers fixed state/scratch and output old/new
capacity overlap before fallible growth. Encoder/comparison libraries are test dependencies only.

Dependency provenance: miniz_oxide 0.9.1, https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide,
MIT OR Zlib OR Apache-2.0. Original notices are copied unchanged under
notices/miniz_oxide-0.9.1/ for redistributors; no miniz implementation is copied.
crc32fast 1.5.2 retains its original MIT OR Apache-2.0 dependency metadata/notices.
