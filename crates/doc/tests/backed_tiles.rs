use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::backed_tiles::{BackedTilePlan, TileEnumerationError as Error, TileEnumerationLimits as Limits};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, NativePlaneDescriptor, ReadControl},
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
fn descriptor(bounds: Rect) -> NativePlaneDescriptor {
    NativePlaneDescriptor::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), bounds, &[173]).unwrap()
}
fn limits(count: u64) -> Limits {
    Limits { max_tiles: count, max_index_bytes: count * 24, index_bytes_per_tile: 24 }
}
#[test]
fn negative_partial_all_default_extent_row_major_without_reads() {
    let work = AtomicU64::new(0);
    let charge = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let d = descriptor(Rect::new(-257, -1, 1, 257));
    let p = BackedTilePlan::new(d, limits(9), &ctl).unwrap();
    assert_eq!(p.descriptor(), d);
    assert_eq!(p.descriptor().default_bytes(), &[173]);
    assert_eq!(p.tile_count(), 9);
    assert_eq!(p.index_bytes(), 216);
    let mut it = p.iter(&ctl);
    let mut got = Vec::new();
    while let Some(c) = it.next_tile().unwrap() {
        got.push(c);
    }
    assert_eq!(
        got,
        vec![
            TileCoord::new(-2, -1),
            TileCoord::new(-1, -1),
            TileCoord::new(0, -1),
            TileCoord::new(-2, 0),
            TileCoord::new(-1, 0),
            TileCoord::new(0, 0),
            TileCoord::new(-2, 1),
            TileCoord::new(-1, 1),
            TileCoord::new(0, 1)
        ]
    );
    assert!(work.load(Ordering::Relaxed) >= 11);
}
#[test]
fn count_and_index_exact_boundary_one_over_overflow() {
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let d = descriptor(Rect::new(0, 0, 257, 1));
    assert_eq!(BackedTilePlan::new(d, limits(1), &ctl).err(), Some(Error::TileLimit));
    assert_eq!(BackedTilePlan::new(d, Limits { max_tiles: 2, max_index_bytes: 47, index_bytes_per_tile: 24 }, &ctl).err(), Some(Error::IndexLimit));
    assert_eq!(BackedTilePlan::new(d, Limits { max_tiles: 2, max_index_bytes: u64::MAX, index_bytes_per_tile: u64::MAX }, &ctl).err(), Some(Error::IndexLimit));
    assert_eq!(BackedTilePlan::new(d, limits(2), &ctl).unwrap().tile_count(), 2);
}
#[test]
fn extreme_fulltile_corners_checked_before_iteration() {
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    assert_eq!(BackedTilePlan::new(descriptor(Rect::new(i32::MAX - 1, 0, i32::MAX, 1)), limits(1), &ctl).err(), Some(Error::Bounds));
    let d = descriptor(Rect::new(i32::MIN, i32::MIN, i32::MAX - 255, i32::MAX - 255));
    let p = BackedTilePlan::new(d, Limits { max_tiles: u64::MAX, max_index_bytes: u64::MAX, index_bytes_per_tile: 1 }, &ctl).unwrap();
    assert_eq!(p.tile_count(), 16_777_215u64.pow(2));
    assert_eq!(p.iter(&ctl).next_tile().unwrap(), Some(TileCoord::new(-8_388_608, -8_388_608)));
}
#[test]
fn cancellation_and_work_failure_sticky_including_final_eof() {
    let cancel = AtomicBool::new(false);
    let cancelled = || cancel.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    let p = BackedTilePlan::new(descriptor(Rect::new(0, 0, 1, 1)), limits(1), &ctl).unwrap();
    let mut it = p.iter(&ctl);
    assert!(it.next_tile().unwrap().is_some());
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(it.next_tile(), Err(Error::Control(BackingError::Cancelled)));
    cancel.store(false, Ordering::Relaxed);
    assert_eq!(it.next_tile(), Err(Error::Control(BackingError::Cancelled)));
    let calls = AtomicU64::new(0);
    let charge = |_| {
        if calls.fetch_add(1, Ordering::Relaxed) >= 1 { Err(BackingError::Work) } else { Ok(()) }
    };
    let fail = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let mut it = p.iter(&fail);
    assert!(it.next_tile().unwrap().is_some());
    assert_eq!(it.next_tile(), Err(Error::Control(BackingError::Work)));
    assert_eq!(it.next_tile(), Err(Error::Control(BackingError::Work)));
}

#[test]
fn midstream_cancel_and_constructor_work_refusal_are_explicit() {
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancel);
    let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    let plan = BackedTilePlan::new(descriptor(Rect::new(-1, 0, 257, 1)), limits(3), &ctl).unwrap();
    let mut tiles = plan.iter(&ctl);
    assert_eq!(tiles.next_tile().unwrap(), Some(TileCoord::new(-1, 0)));
    cancelled.store(true, Ordering::Relaxed);
    assert_eq!(tiles.next_tile(), Err(Error::Control(BackingError::Cancelled)));
    cancelled.store(false, Ordering::Relaxed);
    assert_eq!(tiles.next_tile(), Err(Error::Control(BackingError::Cancelled)));
    let refuse = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert_eq!(BackedTilePlan::new(plan.descriptor(), limits(3), &refuse).err(), Some(Error::Control(BackingError::Work)));
    let polls = AtomicU64::new(0);
    let late_cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
    let late_interrupt = Interrupt::cancel_only(&late_cancel);
    let late = ReadControl { interrupt: &late_interrupt, work: &|_| Ok(()) };
    assert_eq!(BackedTilePlan::new(plan.descriptor(), limits(3), &late).err(), Some(Error::Control(BackingError::Cancelled)));
}
