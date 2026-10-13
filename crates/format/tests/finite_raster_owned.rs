use photocraft_color::PixelFormat;
use photocraft_format::{
    FormatError,
    finite_raster::{FiniteRasterLimits, FiniteRasterM, OwnedFiniteRasterMetadata},
    manifest::TileRef,
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::sync::atomic::{AtomicU64, Ordering};
fn input() -> Vec<u8> {
    serde_json::to_vec(&FiniteRasterM {
        format: PixelFormat::RGBA64F,
        bounds: Rect::new(-3, -4, 259, 5),
        default: "01".repeat(32),
        tiles: vec![TileRef { tx: -1, ty: -1, hash: "00".repeat(32) }, TileRef { tx: 1, ty: 0, hash: "ab".repeat(32) }],
    })
    .unwrap()
}
fn limits() -> FiniteRasterLimits {
    FiniteRasterLimits { max_tiles: 4, max_logical_bytes: 1 << 30 }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 8 << 20, index: 1 << 20, scratch: 0 })
}

#[test]
fn owned_metadata_outlives_input_and_refunds_only_after_drop() {
    let quota = quota();
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let parsed = {
        let data = input();
        OwnedFiniteRasterMetadata::parse_with(&data, data.len(), limits(), quota.clone(), &ctl).unwrap()
    };
    assert_eq!(parsed.descriptor().bounds(), Rect::new(-3, -4, 259, 5));
    assert_eq!(parsed.descriptor().default_bytes(), &[1; 32]);
    assert_eq!(parsed.metadata().tiles[1].hash, "ab".repeat(32));
    assert!(parsed.quota().shares_ledger(&quota));
    let usage = quota.usage().unwrap();
    assert!(usage.resident > 0 && usage.index > 0);
    drop(parsed);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn source_size_and_zero_quota_fail_before_json_decoding() {
    let zero = BackingQuota::new(QuotaLimits::default());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    assert!(matches!(OwnedFiniteRasterMetadata::parse_with(b"not JSON", 99, limits(), zero.clone(), &ctl), Err(FormatError::LimitExceeded(_))));
    let data = input();
    assert!(matches!(OwnedFiniteRasterMetadata::parse_with(&data, data.len() - 1, limits(), quota(), &ctl), Err(FormatError::LimitExceeded(_))));
    assert_eq!(zero.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn rejects_extra_duplicate_and_oversized_fields_without_retained_charge() {
    let original = input();
    let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let mut candidates = vec![];
    json["default"] = serde_json::json!("0".repeat(81));
    candidates.push(serde_json::to_vec(&json).unwrap());
    json = serde_json::from_slice(&original).unwrap();
    json["tiles"][0]["hash"] = serde_json::json!("0".repeat(65));
    candidates.push(serde_json::to_vec(&json).unwrap());
    json = serde_json::from_slice(&original).unwrap();
    json["path"] = serde_json::json!("ambient-source");
    candidates.push(serde_json::to_vec(&json).unwrap());
    json = serde_json::from_slice(&original).unwrap();
    json["tiles"][0]["payload"] = serde_json::json!([1, 2]);
    candidates.push(serde_json::to_vec(&json).unwrap());
    let text = String::from_utf8(original.clone()).unwrap();
    candidates.push(text.replacen("{", "{\"default\":\"00\",", 1).into_bytes());
    json = serde_json::from_slice(&original).unwrap();
    let tile = json["tiles"][0].clone();
    json["tiles"] = serde_json::json!([tile, tile, tile, tile, tile]);
    candidates.push(serde_json::to_vec(&json).unwrap());
    candidates.push([original, b" trailing".to_vec()].concat());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    for data in candidates {
        let q = quota();
        assert!(OwnedFiniteRasterMetadata::parse_with(&data, 1 << 20, limits(), q.clone(), &ctl).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn mid_parse_cancel_and_work_denial_are_explicit_and_refund() {
    let bytes = input();
    let q = quota();
    let polls = AtomicU64::new(0);
    let cancel = || polls.load(Ordering::SeqCst) > 10;
    let interrupt = Interrupt::cancel_only(&cancel);
    let work = |_| {
        polls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    assert!(matches!(
        OwnedFiniteRasterMetadata::parse_with(&bytes, 1 << 20, limits(), q.clone(), &ReadControl { interrupt: &interrupt, work: &work }),
        Err(FormatError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let work = |_| Err(BackingError::Work);
    assert!(matches!(
        OwnedFiniteRasterMetadata::parse_with(&bytes, 1 << 20, limits(), q.clone(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }),
        Err(FormatError::LimitExceeded(_))
    ));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn empty_index_escaped_hex_and_nested_semantics_have_exact_boundaries() {
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let data = br#"{"format":{"mode":"Rgb","sample":"U32","alpha":true},"bounds":{"x0":0,"y0":0,"x1":1,"y1":1},"default":"\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031\u0030\u0031","tiles":[]}"#;
    let q = quota();
    let parsed = OwnedFiniteRasterMetadata::parse_with(data, data.len(), FiniteRasterLimits { max_tiles: 0, max_logical_bytes: 16 }, q.clone(), &ctl).unwrap();
    assert_eq!(parsed.descriptor().default_bytes(), &[1; 16]);
    assert!(parsed.metadata().tiles.is_empty());
    drop(parsed);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let original = input();
    for field in ["format", "bounds"] {
        let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
        json[field]["future_semantics"] = serde_json::json!(true);
        let data = serde_json::to_vec(&json).unwrap();
        assert!(OwnedFiniteRasterMetadata::parse_with(&data, 1 << 20, limits(), q.clone(), &ctl).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let too_large = vec![b' '; (1 << 20) + 1];
    assert!(matches!(OwnedFiniteRasterMetadata::parse_with(&too_large, usize::MAX, limits(), q.clone(), &ctl), Err(FormatError::LimitExceeded(_))));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn quota_callbacks_are_reentrant_and_temporaries_are_refunded() {
    let q = quota();
    let data = input();
    let work = |_| {
        let token = q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, 1).map_err(|_| BackingError::Quota)?;
        drop(token);
        Ok(())
    };
    let parsed = OwnedFiniteRasterMetadata::parse_with(&data, 1 << 20, limits(), q.clone(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    drop(parsed);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    // The owner/index could fit, but the prepaid parser scratch cannot.
    let small = BackingQuota::new(QuotaLimits { resident: 4096, index: 4096, scratch: 0 });
    assert!(matches!(
        OwnedFiniteRasterMetadata::parse_with(&data, 1 << 20, limits(), small.clone(), &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }),
        Err(FormatError::LimitExceeded(_))
    ));
    assert_eq!(small.usage().unwrap(), QuotaUsage::default());
}
