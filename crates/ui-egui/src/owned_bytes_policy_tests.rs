//! UNRUN standard encoded admission versus actual paid provider/Rust overlap.
use super::*;
use photocraft_raster::backing::quota::{QuotaLimits, QuotaUsage};
#[test]
#[ignore = "UNRUN: browser owned XCF/native standard selection is clock-lazy before picker, platform encoded ceiling shared with native store, large metadata refuses actual quota before allocation, one Arc/cancel/meter persists through bounded copy"]
fn explicit_owned_byte_policy_and_real_overlap_remain_separate() {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    assert_eq!(CAP as u64, limits.get(photocraft_xcf::BudgetKind::Encoded));
    for family in [Family::Xcf, Family::Native] {
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 10, index: 16 << 10, scratch: 0 });
        let request = Request::with_family(q.clone(), family).unwrap();
        let before = request.work.meter.diagnostics();
        assert_eq!(before.used, 0);
        assert_eq!(before.policy, Some(limits.policy()));
        assert_eq!(before.limit, limits.get(photocraft_xcf::BudgetKind::Work));
        assert!(!before.deadline_expired);
        let baseline = q.usage().unwrap();
        let name = if family == Family::Native { "actual.pcraft" } else { "actual.xcf" };
        assert!(CAP > 16 << 20);
        match request.clone().begin_capture(name, (16 << 20) + 1) {
            Err(e) => assert!(e.contains("backing quota")),
            Ok(_) => panic!("small real quota cannot admit payload/provider overlap"),
        }
        assert_eq!(q.usage().unwrap(), baseline);
        let started = request.work.meter.diagnostics();
        assert!(started.used > before.used);
        let mut builder = request.clone().begin_capture(name, 2).unwrap();
        assert!(Arc::ptr_eq(&request.work, &builder.request.work));
        builder.append(&[1]).unwrap();
        let used = request.work.meter.diagnostics().used;
        request.work.cancelled.store(true, Ordering::Relaxed);
        assert!(builder.append(&[2]).is_err());
        assert_eq!(request.work.meter.diagnostics().used, used);
        drop(builder);
        assert_eq!(q.usage().unwrap(), baseline);
        drop(request);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
