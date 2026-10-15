//! UNRUN synthetic CURRENT native fixture, explicitly not original source proof.
use super::*;
#[path = "../../../ui-egui/src/canvas_flat_current_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::{ctl, fixture, page, payload};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-general-flat-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN: five sameq synthetic native CURRENT mixed layers, full paginated project+selected slices/auth/wait/control, metadata/native title/reopen, unrelated mutations closed"]
fn general_flat_headless_pages_preserve_order_without_jobs_or_source_authority() {
    for all_normal in [false, true] {
        let dir = Directory::new();
        let (owner, _) = if all_normal { fixture::normal_fixture() } else { fixture() };
        let path = dir.0.join("Synthetic current.pcraft");
        let encoded = photocraft_format::owned_document::save_current_document_archive(
            owner.document(),
            std::io::Cursor::new(Vec::new()),
            Default::default(),
            owner.quota(),
            &ctl(),
        )
        .unwrap()
        .into_inner();
        std::fs::write(&path, &encoded).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::same_quota(before.accounted().unwrap());
        payload(owner.document(), &before);
        assert_eq!(owner.document().xcf_state, before.xcf_state);
        assert!(!h.has_owned_filter_inspection());
        assert!(!h.session.active().unwrap().source_read_only);
        let source = h.session.active().unwrap().source.clone();
        for (params, offset, limit) in [
            (Value::Null, 0, 64),
            (json!({}), 0, 64),
            (json!({"limit":1}), 0, 1),
            (json!({"offset":1,"limit":2}), 1, 2),
            (json!({"offset":4,"limit":128}), 4, 128),
            (json!({"offset":100000,"limit":1}), 100000, 1),
        ] {
            for wait in [true, false] {
                let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &params, wait, &ctl()).unwrap();
                page(reply.value(), &before, offset, limit);
                drop(reply);
            }
        }
        assert!(!h.session.has_jobs());
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.clone();
        let revision = h.session.active().unwrap().revision;
        for params in [json!({"limit":0}), json!({"limit":129}), json!({"offset":100001}), json!({"offset":-1}), json!({"extra":true})] {
            assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &params, true, &ctl()).is_err());
        }
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("general query denied".into())));
        match h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => assert!(e.to_string().contains("general query denied")),
            Ok(_) => panic!("supported paid query honors authorization"),
        };
        h.session.authorize = None;
        let denied = photocraft_raster::backing::ReadControl {
            interrupt: &photocraft_raster::Interrupt::NONE,
            work: &|_| Err(photocraft_raster::backing::BackingError::Work),
        };
        assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &denied).is_err());
        let cancelled = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        assert!(
            h.inspect_owned_command_with_read_control(
                "xcf.inspect",
                &Value::Null,
                false,
                &photocraft_raster::backing::ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }
            )
            .is_err()
        );
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.journal, journal);
        assert_eq!(h.session.active().unwrap().revision, revision);
        let layer = before.layers[3].id;
        assert!(
            matches!(h.command_start("layer.setProps",json!({"layer":layer.0,"visible":true}),false),Err(AutomationError::BadRequest(ref e))if e.contains("synchronous"))
        );
        h.command_run("layer.setProps", json!({"layer":layer.0,"name":"Current native overlay","visible":true,"opacity":0.25})).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        payload(&before, &changed);
        let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":0,"limit":2}), true, &ctl()).unwrap();
        page(reply.value(), &changed, 0, 2);
        drop(reply);
        let output = dir.0.join("General saved title.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap().doc.clone();
        fixture::same_quota(current.accounted().unwrap());
        assert_eq!(current.name, saved.name);
        assert_eq!(current.xcf_state, saved.xcf_state);
        payload(&saved, &current);
        let reply = reopened.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":2,"limit":2}), false, &ctl()).unwrap();
        page(reply.value(), &current, 2, 2);
        drop(reply);
        for (id, p) in [("layer.translate", json!({"dx":1})), ("select.all", Value::Null), ("xcf.filter.enable", json!({"filterId":"1","enabled":false}))] {
            assert!(reopened.command_run(id, p).is_err());
        }
        assert_eq!(std::fs::read(&path).unwrap(), encoded);
    }
    let dir = Directory::new();
    let (bad, _) = fixture::malformed_fixture();
    assert!(photocraft_engine::accounted_native_open::is_current_separable_blend_candidate(bad.document(), &ctl()).unwrap());
    let bytes = photocraft_format::owned_document::save_current_document_archive(
        bad.document(),
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        bad.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner();
    let path = dir.0.join("Malformed matching CURRENT.pcraft");
    std::fs::write(&path, &bytes).unwrap();
    let mut refused = Headless::trusted_local();
    assert!(refused.open_owned_native_keep_current(&path, &ctl()).is_err());
    assert!(refused.session.documents().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
