#[path = "../../engine/tests/common/accounted_resource.rs"]
mod accounted_fixture;
use photocraft_automation::Headless;
use photocraft_doc::{ColorMode, Document, SampleType, Size, accounted_document::AccountedDocument};
use photocraft_io::ExportOptions;
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use std::sync::Arc;
#[test]
fn unadopted_headless_consumers_refuse_accounted_models_before_work_or_file_write() {
    let quota = BackingQuota::new(QuotaLimits { resident: 2 << 20, scratch: 0, index: 2 << 20 });
    let retention = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let document = accounted_fixture::document(&quota, &control);
    let owner = AccountedDocument::from_owned(document, Arc::new(retention), quota.clone(), &control).unwrap();
    let mut app = Headless::new();
    app.session.add_accounted_document(owner, None, &control).unwrap();
    let weak = app.session.active().unwrap().doc.downgrade();
    app.session.add_document(Document::new("resident control", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None).unwrap();
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.select(0).is_err());
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.close(Some(1)).is_err());
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.session.set_active(0));
    let before = quota.usage().unwrap();
    let target = std::env::temp_dir().join(format!(
        "photocraft-accounted-refused-{}-{}.pcraft",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    assert!(!target.exists());
    assert!(app.session_list().is_err());
    assert!(app.inspect(None).is_err());
    assert!(app.render_png(None, 2).is_err());
    assert!(app.save(None, Some(&target), None, &ExportOptions::default()).is_err());
    assert!(!target.exists());
    assert!(weak.is_current(&app.session.active().unwrap().doc));
    assert_eq!(quota.usage().unwrap(), before);
    assert!(!app.session.active().unwrap().history.can_undo());
    drop(app);
    let remaining = quota.usage().unwrap();
    assert_eq!(remaining.resident, remaining.index);
    assert!(remaining.index > 0);
    assert!(weak.upgrade().is_none());
    drop(weak);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
