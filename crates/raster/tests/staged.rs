use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{
    Interrupt, RasterInputError, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
use std::{cell::Cell, rc::Rc};

struct Lease {
    live: Rc<Cell<u64>>,
    bytes: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.live.set(self.live.get() - self.bytes);
    }
}
struct Budget {
    live: Rc<Cell<u64>>,
    cap: u64,
    work: u64,
    deny_work: u64,
    reserves: usize,
}
impl Budget {
    fn new() -> Self {
        Self { live: Rc::new(Cell::new(0)), cap: 128 << 20, work: 0, deny_work: u64::MAX, reserves: 0 }
    }
}
impl StagingBudget for Budget {
    type Lease = Lease;
    fn reserve(&mut self, bytes: u64) -> Result<Lease, RasterInputError> {
        if bytes > self.cap - self.live.get() {
            return Err(RasterInputError::LimitExceeded);
        }
        self.reserves += 1;
        self.live.set(self.live.get() + bytes);
        Ok(Lease { live: self.live.clone(), bytes })
    }
    fn work(&mut self, n: u64) -> Result<(), RasterInputError> {
        if n > self.deny_work.saturating_sub(self.work) {
            return Err(RasterInputError::LimitExceeded);
        }
        self.work += n;
        Ok(())
    }
}
fn encoded(sample: SampleType, index: usize) -> Vec<u8> {
    match sample {
        SampleType::U8 => vec![(index.wrapping_mul(37) % 256) as u8],
        SampleType::U16 => (0x8001_u16.wrapping_add(index as u16)).to_ne_bytes().to_vec(),
        SampleType::U32 => (0x80000001_u32.wrapping_add(index as u32)).to_ne_bytes().to_vec(),
        SampleType::F32 => [(-0_f32).to_bits(), 0x3f000001, 0x3f400000][index % 3].to_ne_bytes().to_vec(),
        SampleType::F64 => [(-0_f64).to_bits(), 0x3fe0000000000001, 0x3fe8000000000000][index % 3].to_ne_bytes().to_vec(),
    }
}
fn cells(bounds: Rect) -> Vec<Rect> {
    let mut cells = Vec::new();
    for y in (bounds.y0..bounds.y1).step_by(64) {
        for x in (bounds.x0..bounds.x1).step_by(64) {
            cells.push(Rect::new(x, y, (x + 64).min(bounds.x1), (y + 64).min(bounds.y1)));
        }
    }
    cells
}
fn crop(bounds: Rect, cell: Rect, bpp: usize, bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    for y in cell.y0..cell.y1 {
        let start = ((y - bounds.y0) as usize * bounds.width() as usize + (cell.x0 - bounds.x0) as usize) * bpp;
        result.extend_from_slice(&bytes[start..start + cell.width() as usize * bpp]);
    }
    result
}
#[test]
fn exact_samples_defaults_keys_negative_offsets_and_straddles() {
    let bounds = Rect::new(-17, -9, 310, 289);
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for alpha in [false, true] {
                let format = PixelFormat::new(mode, sample, alpha);
                let default = encoded(sample, 0).repeat(format.channels());
                let raw: Vec<u8> = (0..bounds.width() as usize * bounds.height() as usize * format.channels()).flat_map(|i| encoded(sample, i)).collect();
                let mut expected = Surface::try_with_default_bytes(format, &default).unwrap();
                expected.try_write_interleaved(bounds, &raw).unwrap();
                let mut budget = Budget::new();
                let mut builder = StagedSurfaceBuilder::new(format, &default, bounds, &mut budget, &Interrupt::NONE).unwrap();
                for cell in cells(bounds).into_iter().rev() {
                    builder.push_region(cell, &crop(bounds, cell, format.bytes_per_pixel(), &raw), &mut budget, &Interrupt::NONE).unwrap();
                }
                assert_eq!(builder.allocated_tile_count(), expected.tile_count());
                let result = builder.finalize(&mut budget, &Interrupt::NONE).unwrap();
                assert_eq!(result.surface(), &expected, "{sample:?}/{mode:?}/{alpha}");
                assert_eq!(result.surface().try_read_interleaved(bounds).unwrap(), raw);
                assert_eq!(budget.reserves, 1);
                assert!(budget.live.get() > 0);
                let (surface, lease) = result.into_parts();
                assert!(budget.live.get() > 0);
                drop(surface);
                drop(lease);
                assert_eq!(budget.live.get(), 0);
            }
        }
    }
}
#[test]
fn exact_zero_tiles_are_retained_and_each_destination_is_created_once() {
    let bounds = Rect::new(0, 0, 256, 256);
    let mut budget = Budget::new();
    let mut builder = StagedSurfaceBuilder::new(PixelFormat::RGBA8, &[0; 4], bounds, &mut budget, &Interrupt::NONE).unwrap();
    for cell in cells(bounds) {
        builder.push_region(cell, &vec![0; 64 * 64 * 4], &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(builder.allocated_tile_count(), 1);
    }
    let result = builder.finalize(&mut budget, &Interrupt::NONE).unwrap();
    assert_eq!(result.surface().tile_count(), 1);
    assert_eq!(result.surface(), &Surface::try_from_interleaved(PixelFormat::RGBA8, bounds, &vec![0; 256 * 256 * 4]).unwrap());
    drop(result);
    assert_eq!(budget.live.get(), 0);
}
#[test]
fn malformed_overlap_incomplete_and_work_denial_refund_without_publication() {
    let bounds = Rect::new(-3, -5, 125, 123);
    for case in 0..5 {
        let mut budget = Budget::new();
        let mut builder = StagedSurfaceBuilder::new(PixelFormat::GRAY8, &[17], bounds, &mut budget, &Interrupt::NONE).unwrap();
        let first = cells(bounds)[0];
        builder.push_region(first, &[9; 4096], &mut budget, &Interrupt::NONE).unwrap();
        if case == 4 {
            assert!(matches!(builder.finalize(&mut budget, &Interrupt::NONE), Err(RasterInputError::LengthMismatch)));
        } else {
            let (cell, raw) = match case {
                0 => (first, vec![9; 4096]),
                1 => (Rect::new(0, 0, 64, 64), vec![9; 4096]),
                2 => (cells(bounds)[1], vec![9; 4095]),
                _ => {
                    budget.deny_work = budget.work;
                    (cells(bounds)[1], vec![9; 4096])
                }
            };
            assert!(builder.push_region(cell, &raw, &mut budget, &Interrupt::NONE).is_err());
            assert_eq!(builder.allocated_tile_count(), 0);
            assert_eq!(budget.live.get(), 0);
            assert!(builder.finalize(&mut budget, &Interrupt::NONE).is_err());
        }
        assert_eq!(budget.live.get(), 0);
    }
    let mut budget = Budget::new();
    budget.cap = 1;
    assert!(matches!(StagedSurfaceBuilder::new(PixelFormat::GRAY8, &[0], bounds, &mut budget, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
    assert_eq!(budget.live.get(), 0);
    assert_eq!(budget.reserves, 0);
}
#[test]
fn initialization_late_copy_and_publication_cancellation_refund() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let bounds = Rect::new(0, 0, 128, 64);
    for late in [false, true] {
        let mut budget = Budget::new();
        let mut builder = StagedSurfaceBuilder::new(PixelFormat::RGBA8, &[0; 4], bounds, &mut budget, &Interrupt::NONE).unwrap();
        if late {
            builder.push_region(Rect::new(0, 0, 64, 64), &[7; 16384], &mut budget, &Interrupt::NONE).unwrap();
        }
        let calls = AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, Ordering::Relaxed) >= 5;
        assert_eq!(
            builder.push_region(
                if late { Rect::new(64, 0, 128, 64) } else { Rect::new(0, 0, 64, 64) },
                &[7; 16384],
                &mut budget,
                &Interrupt::cancel_only(&stop)
            ),
            Err(RasterInputError::Cancelled)
        );
        assert_eq!(builder.allocated_tile_count(), 0);
        assert_eq!(budget.live.get(), 0);
    }
    let mut budget = Budget::new();
    let mut builder = StagedSurfaceBuilder::new(PixelFormat::GRAY8, &[0], Rect::new(0, 0, 64, 64), &mut budget, &Interrupt::NONE).unwrap();
    builder.push_region(Rect::new(0, 0, 64, 64), &[0; 4096], &mut budget, &Interrupt::NONE).unwrap();
    let calls = AtomicUsize::new(0);
    let stop = || calls.fetch_add(1, Ordering::Relaxed) >= 2;
    assert!(matches!(builder.finalize(&mut budget, &Interrupt::cancel_only(&stop)), Err(RasterInputError::Cancelled)));
    assert_eq!(budget.live.get(), 0);
}
#[test]
fn old_incremental_ingestion_replaces_shared_destination_each_region() {
    let mut surface = Surface::new(PixelFormat::GRAY8);
    surface.try_write_interleaved(Rect::new(0, 0, 64, 64), &[1; 4096]).unwrap();
    let first = surface.tile(TileCoord::new(0, 0)).unwrap().bytes().as_ptr() as usize;
    surface.try_write_interleaved(Rect::new(64, 0, 128, 64), &[2; 4096]).unwrap();
    assert_ne!(first, surface.tile(TileCoord::new(0, 0)).unwrap().bytes().as_ptr() as usize);
}
#[test]
fn exact_prepaid_limit_and_hostile_constructor_reject_before_allocation() {
    let bounds = Rect::new(254, 254, 318, 318);
    let mut budget = Budget::new();
    let builder = StagedSurfaceBuilder::new(PixelFormat::RGBA64F, &[0; 32], bounds, &mut budget, &Interrupt::NONE).unwrap();
    let required = budget.live.get();
    assert!(required > 4 * 256 * 256 * 32);
    drop(builder);
    assert_eq!(budget.live.get(), 0);
    budget.cap = required - 1;
    assert!(matches!(StagedSurfaceBuilder::new(PixelFormat::RGBA64F, &[0; 32], bounds, &mut budget, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
    assert_eq!(budget.live.get(), 0);
    budget.cap = required;
    let mut builder = StagedSurfaceBuilder::new(PixelFormat::RGBA64F, &[0; 32], bounds, &mut budget, &Interrupt::NONE).unwrap();
    builder.push_region(bounds, &vec![0; 64 * 64 * 32], &mut budget, &Interrupt::NONE).unwrap();
    assert_eq!(builder.allocated_tile_count(), 4);
    let output = builder.finalize(&mut budget, &Interrupt::NONE).unwrap();
    assert_eq!(budget.live.get(), required);
    drop(output);
    assert_eq!(budget.live.get(), 0);
    for invalid in [Rect::EMPTY, Rect::new(2, 0, 1, 1), Rect::new(i32::MIN, 0, i32::MAX, 1), Rect::new(0, 0, 300000, 1)] {
        assert!(StagedSurfaceBuilder::new(PixelFormat::GRAY8, &[0], invalid, &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.live.get(), 0);
    }
    let mut budget = Budget::new();
    assert!(matches!(
        StagedSurfaceBuilder::new(PixelFormat::GRAY8, &[0], bounds, &mut budget, &Interrupt::cancel_only(&|| true)),
        Err(RasterInputError::Cancelled)
    ));
    assert_eq!(budget.reserves, 0);
}
#[test]
#[ignore = "manual24MP release staging comparison; no IO/native_large qualification"]
fn staged_24mp_release_comparison() {
    let bounds = Rect::new(0, 0, 6000, 4000);
    let format = PixelFormat::RGBA8;
    let cells = cells(bounds);
    let raw = [19, 89, 173, 211].repeat(64 * 64);
    let crop_cell = |cell: Rect| {
        let mut out = Vec::with_capacity(cell.width() as usize * cell.height() as usize * 4);
        for _ in 0..cell.height() {
            out.extend_from_slice(&raw[..cell.width() as usize * 4]);
        }
        out
    };
    let start = std::time::Instant::now();
    let mut eager = Surface::new(format);
    for cell in &cells {
        eager.try_write_interleaved(*cell, &crop_cell(*cell)).unwrap();
    }
    let eager_seconds = start.elapsed().as_secs_f64();
    let mut budget = Budget::new();
    let start = std::time::Instant::now();
    let mut builder = StagedSurfaceBuilder::new(format, &[0; 4], bounds, &mut budget, &Interrupt::NONE).unwrap();
    let charged = budget.live.get();
    for cell in &cells {
        builder.push_region(*cell, &crop_cell(*cell), &mut budget, &Interrupt::NONE).unwrap();
    }
    let staged = builder.finalize(&mut budget, &Interrupt::NONE).unwrap();
    let staged_seconds = start.elapsed().as_secs_f64();
    assert_eq!(staged.surface(), &eager);
    println!(
        "STAGED24MP eager_seconds={eager_seconds:.6} staged_seconds={staged_seconds:.6} tile_count={} prepaid_bytes={charged} work_units={} input_scratch_max_bytes=16384",
        staged.surface().tile_count(),
        budget.work
    );
    drop(staged);
    assert_eq!(budget.live.get(), 0);
}
