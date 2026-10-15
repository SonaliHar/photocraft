//! UNRUN callback authority, no-default-replacement and quota refusal drafts.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaLimits, QuotaUsage},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
fn explicit_callbacks_do_not_reset_exhausted_catalog_or_fallback_from_absent_root() {
    let q = BackingQuota::new(QuotaLimits { resident: 0, index: 0, scratch: 0 });
    assert!(open("nonexistent.xcf", Some(&q), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let no_root = photocraft_automation::AuthorizedWorkspace::new(None, None).unwrap();
    assert!(rooted(&no_root, "nonexistent.xcf", Some(&q), &ctl()).unwrap_err().contains("read authority is absent"));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let deny = |_: u64| Err(photocraft_raster::backing::BackingError::Cancelled);
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(open("nonexistent.xcf", None, &cancelled).is_err());
}
#[test]
fn native_factory_installs_rooted_callback_only_for_actual_held_read_root() {
    let services = crate::services::native(None);
    assert!(services.owned_xcf_open.is_some());
    assert!(services.automation_owned_xcf_open.is_none());
    // Existing generic Open callback remains installed beside explicit owned XCF.
    assert!(services.import.is_some());
    let no_root = photocraft_automation::AuthorizedWorkspace::new(None, None).unwrap();
    let services = crate::services::native(Some(no_root));
    assert!(services.automation_owned_xcf_open.is_none());
}

#[test]
fn combined_callbacks_keep_actual_catalog_refusals_and_separate_root_authority() {
    let q = BackingQuota::new(QuotaLimits { resident: 0, index: 0, scratch: 0 });
    assert!(open_current("nonexistent.xcf", Some(&q), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let absent = photocraft_automation::AuthorizedWorkspace::new(None, None).unwrap();
    assert!(rooted_current(&absent, "nonexistent.xcf", Some(&q), &ctl()).unwrap_err().contains("read authority is absent"));
    let services = crate::services::native(None);
    assert!(services.owned_current_xcf_open.is_some());
    assert!(services.automation_owned_current_xcf_open.is_none());
    // Existing plain callbacks retain their own exact public entry points.
    assert!(services.owned_xcf_open.is_some());
    assert!(services.automation_owned_xcf_open.is_none());
}
