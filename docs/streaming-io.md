# Native PSD/EXR streaming

Native path opens inspect a seekable source before allocating decoded pixels. Supported layered
PSD/PSB files and flat scanline EXR files decode bounded bands directly into guarded raster
tiles. Their document dimensions and samples remain at the source resolution; scratch residency
can move cold tiles to disk. The byte decoder remains available for web and unsupported layouts,
subject to working-memory admission rather than a file-sized allocation without a budget.

PSD indexing retains bounded metadata and channel offsets without copying all channel data or
decoding the stored merged image. Independent seekable channel handles feed the existing document
mapping, preserving layers, masks, groups, metadata, colour models and depth. Unsupported nesting
and metadata limits produce errors. The native PSB writer uses the same metadata mapper with
deferred pixel sources and writes original-depth channel bands to a seekable output.

EXR block decoding supports the flat scanline layouts accepted by the streaming decoder, including
the RGB HALF ZIP test image `65536 × 32768`. HALF samples are stored as F32, with a warning; this
does not reduce the document dimensions. Unsupported EXR layouts fall back to the normal codec
when its limits allow them. Chunk sizes, source bounds and transient buffers are checked before
allocation. Cancellation is checked between bands, and progress describes completed work.

Control and CLI opens use the same importer. Authorized control opens create independent handles
under the held read-directory capability, including channel reopens. They do not gain ambient
filesystem access. Path opens check that file size and modification time remain unchanged. Read-only
and preview import flags survive the native and authorized routes, and templates keep an untitled
document without a source save path.

The seekable output API accepts a caller-owned writer; atomic path output encodes into a temporary
file beside the destination, syncs it using the shared platform helper, then renames it. Encoder
failure, cancellation or unwinding removes the temporary and preserves the old destination. Its
integration with the desktop background-save service is documented in [streamed saves](streaming-save.md).

Synthetic tests compare streamed import with the byte decoder across PSD/PSB versions, depths,
colour modes and compression, and verify masks, metadata, cancellation, EXR chunk handling,
source protection and capability bounds. Large-file runtime checks use the public fixtures at
[vistaero.es/photocraft](https://vistaero.es/photocraft/).
