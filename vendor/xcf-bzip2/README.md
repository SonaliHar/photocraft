# xcf-bzip2

Isolated safe, bounded bzip2 decoder for PhotoCraft's standalone XCF reader.
Only `decode_with_reservation` and its `Reservation` enum are public. Input is already-owned
and bounded; all caps are enforced even when the admission callback accepts.

Poll callbacks enforce the caller's cancellation token and monotonic deadline. Reservation
callbacks admit complete peak requirements, not increments; hold maximum source/state/index/output
reservations until the transaction returns. Smaller repeated requests do not release live memory.
Error denial precedes allocation; errors/cancellation drop partial output. Output growth overlaps
old/new capacities where the enum says OutputOverlap. Caller-owned input is charged separately.

No file I/O, threads, foreign runtime, workspace dependency or unsafe code is used by this package.
Streaming/U64 byte sources and shared job reservation lifetimes belong to application integration.
See ADAPTATION.md and adjacent original notices. Test encoder dependencies are not runtime imports.

The decoder adaptation is MIT OR Apache-2.0; the retained historical randomization table
is separately bzip2-1.0.6, with full LICENSE-randtable notice adjacent. Both notices apply.
