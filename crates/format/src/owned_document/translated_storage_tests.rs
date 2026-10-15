//! UNRUN native continuation of genuine paid translated stores. No source or
//! rendering grant follows. Root physical tile hashes may change when unaligned;
//! logical raw words, exact defaults and operative bounds are the invariants.
use super::*;
use photocraft_doc::{BackedLayer, ColorMode, Layer, LayerContent, PixelFormat, SampleType};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaKind, QuotaLease, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use std::{
    io::{Cursor, Seek, SeekFrom, Write},
    sync::atomic::{AtomicU64, Ordering},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Reads {
    calls: AtomicU64,
    _fee: QuotaLease,
}
#[derive(Debug)]
struct Pattern {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    reads: Arc<Reads>,
    _fee: QuotaLease,
}
fn word(sample: SampleType, index: u64) -> [u8; 8] {
    let mut out = [0; 8];
    match sample {
        SampleType::U8 => out[0] = (index as u8).wrapping_mul(37).wrapping_add(11),
        SampleType::U16 => out[..2].copy_from_slice(&((index as u16).wrapping_mul(1031) ^ 0x9123).to_ne_bytes()),
        SampleType::U32 => out[..4].copy_from_slice(&((index as u32).wrapping_mul(0x10001) ^ 0x81234567).to_ne_bytes()),
        SampleType::F32 => {
            out[..4].copy_from_slice(&[0x80000000u32, 0x7fc01234, 0x3f000001, 1, 0x7f800000, 0x3f7fffff, 0xff800000, 0][index as usize % 8].to_ne_bytes())
        }
        SampleType::F64 => out.copy_from_slice(
            &[0x8000000000000000u64, 0x7ff8000000001234, 0x3fe0000000000001, 1, 0x7ff0000000000000, 0x3fefffffffffffff, 0xfff0000000000000, 0]
                [index as usize % 8]
                .to_ne_bytes(),
        ),
    }
    out
}
impl NativePlaneStore for Pattern {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.calls.fetch_add(1, Ordering::Relaxed);
        if offset.checked_add(out.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        let width = self.descriptor.format().sample.bytes() as u64;
        for (i, b) in out.iter_mut().enumerate() {
            let at = offset + i as u64;
            *b = word(self.descriptor.format().sample, at / width)[(at % width) as usize];
        }
        c.check(1)
    }
}
// Every actual heap owner is preceded by its real same-ledger fee. Fixed small
// fixture model envelope is retained with the Document; it is no source proof.
struct Fixture {
    doc: Document,
    _model: QuotaLease,
}
fn fixture(q: &BackingQuota, format: PixelFormat, rect: Rect) -> (Fixture, Arc<Reads>) {
    let read_fee = q.reserve(QuotaKind::Index, (std::mem::size_of::<Reads>() + 128) as u64).unwrap();
    let reads = Arc::new(Reads { calls: AtomicU64::new(0), _fee: read_fee });
    let mut default = [0; 40];
    let width = format.sample.bytes();
    for (i, part) in default[..format.bytes_per_pixel()].chunks_exact_mut(width).enumerate() {
        part.copy_from_slice(&word(format.sample, 7 + i as u64)[..width]);
    }
    let descriptor = NativePlaneDescriptor::new(format, rect, &default[..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Index, (std::mem::size_of::<Pattern>() + 128) as u64).unwrap();
    let store = Arc::new(Pattern { descriptor, quota: q.clone(), reads: reads.clone(), _fee: fee });
    let native = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
    let layer = BackedLayer::new(native, descriptor, &ctl()).unwrap();
    let model = q.reserve(QuotaKind::Index, 64 << 10).unwrap();
    let mut doc = Document::new("native translated fixture", Size::new(300, 32), format.mode, format.sample);
    doc.layers.push(Layer::new("actual charged leaf", LayerContent::BackedRaster(layer)));
    (Fixture { doc, _model: model }, reads)
}
fn layer(doc: &Document) -> &BackedLayer {
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("native-backed fixture") };
    layer
}
struct Bytes {
    bytes: Vec<u8>,
    _fee: QuotaLease,
}
fn capture(layer: &BackedLayer) -> Bytes {
    let n = usize::try_from(layer.descriptor().logical_bytes()).unwrap();
    let header = (std::mem::size_of::<Bytes>() + 128) as u64;
    let mut fee = layer.quota().reserve(QuotaKind::Resident, 2 * n as u64 + header).unwrap();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(n).unwrap();
    assert!(bytes.capacity() <= 2 * n);
    fee.reconcile_capacity(bytes.capacity() as u64 + header).unwrap();
    bytes.resize(n, 0);
    let r = layer.operative_bounds();
    let bpp = layer.format().bytes_per_pixel();
    for y in 0..r.height() {
        for x in 0..r.width() {
            let at = (y as usize * r.width() as usize + x as usize) * bpp;
            layer.checked_view().read_logical_pixel_native_into(r.x0 + x as i32, r.y0 + y as i32, &mut bytes[at..at + bpp], &ctl()).unwrap();
        }
    }
    Bytes { bytes, _fee: fee }
}
// Bounded paid output is moved, without an unaccounted copy, into the immutable
// archive. This is a test memory capability, never a path or source grant.
struct Writer {
    cursor: Cursor<Vec<u8>>,
    quota: BackingQuota,
    _fee: QuotaLease,
}
#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl Writer {
    fn new(q: &BackingQuota) -> Self {
        let max = 16usize << 20;
        let header = (std::mem::size_of::<Self>() + std::mem::size_of::<Memory>() + 256) as u64;
        let mut fee = q.reserve(QuotaKind::Resident, max as u64 + header).unwrap();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(max).unwrap();
        assert!(bytes.capacity() <= max);
        fee.reconcile_capacity(bytes.capacity() as u64 + header).unwrap();
        Self { cursor: Cursor::new(bytes), quota: q.clone(), _fee: fee }
    }
    fn finish(self) -> Arc<Memory> {
        Arc::new(Memory { bytes: self.cursor.into_inner(), quota: self.quota, _fee: self._fee })
    }
}
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.cursor.position().checked_add(b.len() as u64).is_none_or(|n| n > self.cursor.get_ref().capacity() as u64) {
            return Err(std::io::Error::other("paid output capacity"));
        }
        self.cursor.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for Writer {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(p)
    }
}
impl ImmutableCanonicalArchive for Memory {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let a = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn roundtrip(doc: &Document, q: &BackingQuota) -> OwnedDocumentArchive {
    let memory = save_current_document_archive(doc, Writer::new(q), Default::default(), q, &ctl()).unwrap().finish();
    let weak = Arc::downgrade(&memory);
    let loaded = load_document_from_owned_archive(memory.clone(), &Default::default(), Default::default(), q.clone(), &ctl()).unwrap();
    drop(memory);
    assert!(weak.upgrade().is_some());
    loaded
}
fn exact(layer: &BackedLayer, rect: Rect, default: &[u8], words: &[u8]) {
    assert_eq!(layer.operative_bounds(), rect);
    assert_eq!(layer.default_bytes(), default);
    assert_eq!(capture(layer).bytes, words);
    // Target-grid exterior must be exact default. An unaligned translation may
    // create different physical tile hashes from its root; no equality claimed.
    let bpp = layer.format().bytes_per_pixel();
    for coord in rect.tiles() {
        let tile = coord.rect();
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::DefaultPixel(bytes) => assert_eq!(bytes, default),
            NativeTileBytes::FullTile(bytes) => {
                for y in tile.y0..tile.y1 {
                    for x in tile.x0..tile.x1 {
                        if !rect.contains(x, y) {
                            let at = ((y - tile.y0) as usize * TILE_SIZE as usize + (x - tile.x0) as usize) * bpp;
                            assert_eq!(&bytes[at..at + bpp], default);
                        }
                    }
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN:all five native widths RGB/Gray, unaligned negative/repeated translation and native save/reopen preserve logical words/defaults"]
fn shifted_archive_preserves_exact_current_native_descriptor_and_words() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let q = q();
            let baseline = q.usage().unwrap();
            let (mut f, reads) = fixture(&q, PixelFormat::new(mode, sample, true), Rect::new(-2, 3, 259, 6));
            let original = layer(&f.doc).clone();
            let words = capture(&original);
            let mut default_bytes = [0; 40];
            let length = original.default_bytes().len();
            default_bytes[..length].copy_from_slice(original.default_bytes());
            let default = &default_bytes[..length];
            let shifted = original.try_translated(17, -19, &ctl()).unwrap().try_translated(-1, 2, &ctl()).unwrap();
            let rect = Rect::new(14, -14, 275, -11);
            exact(&shifted, rect, default, &words.bytes);
            f.doc.layers[0].content = LayerContent::BackedRaster(shifted);
            let loaded = roundtrip(&f.doc, &q);
            assert!(layer(loaded.document()).quota().shares_ledger(&q));
            exact(layer(loaded.document()), rect, default, &words.bytes);
            exact(&original, Rect::new(-2, 3, 259, 6), default, &words.bytes);
            drop(loaded);
            drop(words);
            drop(original);
            drop(f);
            drop(reads);
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
}
#[test]
#[ignore = "UNRUN:translated COW affects target coordinate only, original clones remain exact, second archive retains shifted current bounds"]
fn translated_cow_survives_two_native_continuations() {
    let q = q();
    let (mut f, _reads) = fixture(&q, PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(-2, 3, 259, 6));
    let original = layer(&f.doc).clone();
    let old = capture(&original);
    let moved = original.try_translated(17, -19, &ctl()).unwrap();
    let rect = moved.operative_bounds();
    let mut default_bytes = [0; 40];
    let length = moved.default_bytes().len();
    default_bytes[..length].copy_from_slice(moved.default_bytes());
    let default = &default_bytes[..length];
    let edit = Rect::new(rect.x0 + 1, rect.y0 + 1, rect.x0 + 2, rect.y0 + 2);
    let mut replacement = [0u8; 32];
    for (chunk, word) in replacement.chunks_exact_mut(8).zip([0x8000000000000000u64, 0x7ff8000000004321, 0x3ff0000000000001, 1]) {
        chunk.copy_from_slice(&word.to_ne_bytes());
    }
    let changed = moved.try_edited_native(edit, &replacement, &ctl()).unwrap();
    let mut expected = capture(&moved);
    let at = (rect.width() as usize + 1) * 32;
    expected.bytes[at..at + 32].copy_from_slice(&replacement);
    exact(&changed, rect, default, &expected.bytes);
    exact(&moved, rect, default, &old.bytes);
    exact(&original, original.operative_bounds(), default, &old.bytes);
    f.doc.layers[0].content = LayerContent::BackedRaster(changed);
    let loaded = roundtrip(&f.doc, &q);
    exact(layer(loaded.document()), rect, default, &expected.bytes);
    let continued = layer(loaded.document()).try_translated(-5, 7, &ctl()).unwrap();
    let again = continued.operative_bounds();
    exact(&continued, again, default, &expected.bytes);
    let last = Rect::new(again.x1 - 1, again.y1 - 1, again.x1, again.y1);
    let mut final_words = capture(&continued);
    let end = final_words.bytes.len();
    final_words.bytes[end - 32..].copy_from_slice(&replacement);
    let changed_after_reopen = continued.try_edited_native(last, &replacement, &ctl()).unwrap();
    exact(&continued, again, default, &expected.bytes);
    exact(&changed_after_reopen, again, default, &final_words.bytes);
    f.doc.layers[0].content = LayerContent::BackedRaster(changed_after_reopen);
    let loaded_again = roundtrip(&f.doc, &q);
    exact(layer(loaded_again.document()), again, default, &final_words.bytes);
    exact(layer(loaded.document()), rect, default, &expected.bytes);
}
#[test]
#[ignore = "UNRUN:4096 root tiles translated onto4097 target tiles refuse finite native save before reads; no limit raise or source mutation"]
fn shifted_grid_over_existing_finite_limit_is_refused_before_source_read() {
    use photocraft_doc::backed_tiles::{BackedTilePlan, TileEnumerationError, TileEnumerationLimits};
    let q = q();
    let (mut f, reads) = fixture(&q, PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true), Rect::new(0, 0, TILE_SIZE * 4096, 1));
    let limit = TileEnumerationLimits { max_tiles: 4096, max_index_bytes: u64::MAX, index_bytes_per_tile: 0 };
    assert_eq!(BackedTilePlan::new(layer(&f.doc).descriptor(), limit, &ctl()).unwrap().tile_count(), 4096);
    let shifted = layer(&f.doc).try_translated(1, 0, &ctl()).unwrap();
    assert!(matches!(BackedTilePlan::new(shifted.descriptor(), limit, &ctl()), Err(TileEnumerationError::TileLimit)));
    f.doc.layers[0].content = LayerContent::BackedRaster(shifted);
    let before = q.usage().unwrap();
    let calls = reads.calls.load(Ordering::Relaxed);
    assert!(save_current_document_archive(&f.doc, Writer::new(&q), Default::default(), &q, &ctl()).is_err());
    assert_eq!(reads.calls.load(Ordering::Relaxed), calls);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(layer(&f.doc).operative_bounds(), Rect::new(1, 0, 1 + TILE_SIZE * 4096, 1));
}
