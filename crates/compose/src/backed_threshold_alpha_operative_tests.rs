//! UNRUN bounded scheduling/current-owner validation drafts. Wide rectangles
//! exercise only the private scheduler, not source geometry authority.
use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn scheduler_covers_wide_tall_negative_and_extreme_origins_once_within_lower_limits() {
    for rect in [Rect::new(0, 0, 257, 3), Rect::new(0, 0, 3, 257), Rect::new(-257, -3, 0, 0), Rect::new(i32::MAX - 3, i32::MAX - 3, i32::MAX, i32::MAX)] {
        for limits in [NormalBandLimits::default(), NormalBandLimits { max_rows: 1, max_pixels: 1 }, NormalBandLimits { max_rows: 2, max_pixels: 5 }] {
            let mut seen = vec![0u8; rect.width() as usize * rect.height() as usize];
            visit_operative_bands(rect, limits, &ctl(), |band| {
                assert_eq!(band.intersect(&rect), band);
                assert!(band.height() <= limits.max_rows);
                assert!(u64::from(band.width()) * u64::from(band.height()) <= limits.max_pixels);
                assert_eq!(band.x0.div_euclid(TILE_SIZE), (band.x1 - 1).div_euclid(TILE_SIZE));
                assert_eq!(band.y0.div_euclid(TILE_SIZE), (band.y1 - 1).div_euclid(TILE_SIZE));
                for y in band.y0..band.y1 {
                    for x in band.x0..band.x1 {
                        let index = (y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize;
                        seen[index] += 1;
                    }
                }
                Ok(())
            })
            .unwrap();
            assert!(seen.iter().all(|n| *n == 1));
        }
    }
}

#[test]
fn scheduler_refuses_invalid_limits_and_stops_on_late_visit_work_or_cancellation() {
    for limits in [
        NormalBandLimits { max_rows: 0, max_pixels: 1 },
        NormalBandLimits { max_rows: 1, max_pixels: 0 },
        NormalBandLimits { max_rows: 257, max_pixels: 1 },
        NormalBandLimits { max_rows: 1, max_pixels: 65_537 },
    ] {
        assert!(visit_operative_bands(Rect::new(0, 0, 8, 6), limits, &ctl(), |_| panic!("no invalid visit")).is_err());
    }
    assert!(visit_operative_bands(Rect::EMPTY, NormalBandLimits::default(), &ctl(), |_| panic!("no empty visit")).is_err());
    let tiny = NormalBandLimits { max_rows: 1, max_pixels: 1 };
    let mut visits = 0;
    let error = visit_operative_bands(Rect::new(0, 0, 8, 6), tiny, &ctl(), |_| {
        visits += 1;
        if visits == 3 { Err(BackedPointError::Backing(BackingError::Work)) } else { Ok(()) }
    })
    .unwrap_err();
    assert_eq!(error, BackedPointError::Backing(BackingError::Work));
    assert_eq!(visits, 3);
    let stop = AtomicBool::new(false);
    let cancelled = || stop.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    visits = 0;
    assert_eq!(
        visit_operative_bands(Rect::new(0, 0, 8, 6), tiny, &c, |_| {
            visits += 1;
            stop.store(true, Ordering::Relaxed);
            Ok(())
        }),
        Err(BackedPointError::Backing(BackingError::Cancelled))
    );
    assert_eq!(visits, 1, "poll after the completed owner is dropped");
}

#[test]
fn actual_context_uses_its_one_pixel_limits_and_releases_private_bands_without_model_changes() {
    let (doc, q, guard, reads, p, raw) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let owner = adopt(doc, guard, q.clone());
    let original = owner.clone();
    let tiny = NormalBandLimits { max_rows: 1, max_pixels: 1 };
    let prepared = PreparedBackedThresholdAlphaContext::new(&owner, p, tiny, &ctl()).unwrap();
    assert!(prepared.render_band(owner.document().bounds(), &ctl()).is_err());
    // Warm only the ordinary cache; it remains source-owned, not an output band.
    drop(prepared.render_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap());
    let before = q.usage().unwrap();
    prepared.validate_operative(&ctl()).unwrap();
    assert_eq!(q.usage().unwrap(), before, "each private band fee is refunded");
    assert!(owner.shares_model(&original));
    assert!(reads.load(Ordering::Relaxed) > 0);
    let full = PreparedBackedThresholdAlphaContext::new(&owner, p, Default::default(), &ctl()).unwrap();
    let band = full.render_band(owner.document().bounds(), &ctl()).unwrap();
    for (got, source) in band.bytes().as_chunks::<32>().0.iter().zip(raw.as_chunks::<32>().0.iter()) {
        assert_eq!(&got[..24], &source[..24], "native F64 colors and signed zero unchanged");
    }
}

#[test]
fn quota_denial_is_private_and_canvas_backing_mismatch_is_refused() {
    let (doc, q, guard, reads, p, _) = fixture(ColorMode::Grayscale, SampleType::F32, SampleEncoding::Linear);
    let owner = adopt(doc, guard, q.clone());
    let prepared = PreparedBackedThresholdAlphaContext::new(&owner, p, Default::default(), &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let hold = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert_eq!(prepared.validate_operative(&ctl()), Err(BackedPointError::Backing(BackingError::Quota)));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    drop(hold);
    assert_eq!(q.usage().unwrap(), baseline);
    let (mut doc, q, guard, _, _, _) = fixture(ColorMode::Rgb, SampleType::F32, SampleEncoding::Linear);
    // Changing only the canvas leaves the actual layer descriptor and mask at
    // 8x6. Wider admitted geometry cannot authorize that backing mismatch.
    doc.size = Size::new(257, 3);
    assert!(AccountedDocument::from_owned_readonly_threshold_alpha_storage(doc, guard, q, &ctl()).is_err());
}
