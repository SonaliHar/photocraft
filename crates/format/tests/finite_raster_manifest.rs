use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_format::{
    FormatError,
    finite_raster::{FiniteRasterLimits, FiniteRasterM},
    manifest::TileRef,
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl},
};

fn limits() -> FiniteRasterLimits {
    FiniteRasterLimits { max_logical_bytes: 1 << 30, max_tiles: 4096 }
}
fn metadata(format: PixelFormat, bounds: Rect) -> FiniteRasterM {
    FiniteRasterM { format, bounds, default: "00".repeat(format.bytes_per_pixel()), tiles: vec![] }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}

#[test]
fn finite_defaults_extent_and_sparse_tiles_roundtrip_all_sample_widths() {
    for sample in SampleType::ALL {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut value = metadata(format, Rect::new(-3, -4, 259, 5));
        // Arbitrary payload bits are preserved; this is transport, not render admission.
        let default: Vec<u8> = (0..format.bytes_per_pixel()).map(|i| (17 + i * 31) as u8).collect();
        let mut little = default.clone();
        if cfg!(target_endian = "big") {
            for part in little.chunks_exact_mut(sample.bytes()) {
                part.reverse();
            }
        }
        value.default = little.iter().map(|b| format!("{b:02x}")).collect();
        value.tiles = vec![TileRef { tx: -1, ty: -1, hash: "01".repeat(32) }, TileRef { tx: 1, ty: 0, hash: "ab".repeat(32) }];
        let bytes = serde_json::to_vec(&value).unwrap();
        let restored: FiniteRasterM = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, value);
        let descriptor = restored.descriptor_with(limits(), &control()).unwrap();
        assert_eq!(descriptor.default_bytes(), default);
        assert_eq!(descriptor.bounds(), value.bounds);
        assert_eq!(descriptor.logical_bytes(), 262 * 9 * format.bytes_per_pixel() as u64);
    }
}

#[test]
fn rejects_invalid_extent_default_hash_order_and_outside_tiles() {
    let original = metadata(PixelFormat::RGBA64F, Rect::new(-3, -4, 259, 5));
    for change in 0..10 {
        let mut value = original.clone();
        match change {
            0 => value.bounds.x1 = value.bounds.x0,
            1 => value.bounds.y1 = value.bounds.y0 - 1,
            2 => value.default.push('0'),
            3 => value.default.replace_range(0..1, "g"),
            4 => value.default.replace_range(0..1, "A"),
            5 => value.tiles.push(TileRef { tx: 2, ty: 0, hash: "00".repeat(32) }),
            6 => value.tiles.push(TileRef { tx: 0, ty: 1, hash: "00".repeat(32) }),
            7 => value.tiles = vec![TileRef { tx: 0, ty: 0, hash: "0".repeat(63) }],
            8 => value.tiles = vec![TileRef { tx: 0, ty: 0, hash: "00".repeat(32) }, TileRef { tx: 0, ty: 0, hash: "00".repeat(32) }],
            _ => value.tiles = vec![TileRef { tx: 1, ty: 0, hash: "00".repeat(32) }, TileRef { tx: -1, ty: 0, hash: "00".repeat(32) }],
        }
        assert!(value.descriptor_with(limits(), &control()).is_err(), "mutation {change}");
    }
    let huge = metadata(PixelFormat::RGBA64F, Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX));
    assert!(huge.descriptor_with(limits(), &control()).is_err());
    assert_eq!(original.bounds, Rect::new(-3, -4, 259, 5));
}

#[test]
fn exact_logical_and_tile_limits_are_checked_before_object_reads() {
    let mut value = metadata(PixelFormat::RGBA32, Rect::new(0, 0, 1, 1));
    value.tiles.push(TileRef { tx: 0, ty: 0, hash: "00".repeat(32) });
    assert!(value.descriptor_with(FiniteRasterLimits { max_logical_bytes: 16, max_tiles: 1 }, &control()).is_ok());
    for cap in [FiniteRasterLimits { max_logical_bytes: 15, max_tiles: 1 }, FiniteRasterLimits { max_logical_bytes: 16, max_tiles: 0 }] {
        assert!(matches!(value.descriptor_with(cap, &control()), Err(FormatError::LimitExceeded(_))));
    }
    value.tiles = (0..4097).map(|tx| TileRef { tx, ty: 0, hash: "00".repeat(32) }).collect();
    value.bounds = Rect::new(0, 0, 4097 * 256, 1);
    assert!(matches!(
        value.descriptor_with(FiniteRasterLimits { max_logical_bytes: u64::MAX, max_tiles: usize::MAX }, &control()),
        Err(FormatError::LimitExceeded(_))
    ));
}

#[test]
fn cancellation_work_and_unknown_semantics_fail_explicitly() {
    let value = metadata(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1));
    let cancelled = Interrupt::cancel_only(&|| true);
    let no_work = |_| Ok(());
    assert!(matches!(value.descriptor_with(limits(), &ReadControl { interrupt: &cancelled, work: &no_work }), Err(FormatError::Cancelled)));
    let denied = |_| Err(BackingError::Work);
    assert!(matches!(value.descriptor_with(limits(), &ReadControl { interrupt: &Interrupt::NONE, work: &denied }), Err(FormatError::LimitExceeded(_))));
    let mut json = serde_json::to_value(&value).unwrap();
    json["outside_pixel"] = serde_json::json!([1, 2, 3, 4]);
    assert!(serde_json::from_value::<FiniteRasterM>(json).is_err());
}
