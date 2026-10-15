//! Deferred bounded host service/control tests; injected units are not measured work.
use super::*;
#[path = "../../../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod boxed;
#[path = "../../../../ui-egui/src/pure_point_standard_work_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use photocraft_raster::backing::{BackingError, ReadControl};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-point-service-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: actual purePoint/derived-empty bounded query prepare+encoding ledger, external lower control and oldBox inspection preservation, controlled cold immutable provider native Save finite-work/cancel/failure atomicity; genuine paid synthetic CURRENT, no measured work/largeimage/producer/numerical acceptance"]
fn pure_point_bounded_query_and_native_save_use_one_finite_service() {
    for index in 0..6 {
        for empty in [false, true] {
            let directory = Directory::new();
            let owner = fixture::cold_owner(index, empty);
            let (archive, trigger) = fixture::triggered_archive(&owner);
            let mut h = Headless::trusted_local();
            h.session.add_owned_native_point_archive(archive, "CURRENT", None, &fixture::ctl()).unwrap();
            let before = h.session.active().unwrap().doc.clone();
            let history = h.session.active().unwrap().history.clone();
            let source = h.session.active().unwrap().source.clone();
            let journal = h.session.journal.clone();
            let wire = bounded(before.accounted(), |control| {
                // Probe the ACTUAL shared bounded query helper, then prepare/encode
                // with the same control. This is not a measured metadata work cost.
                control.check(8_000_000_001).map_err(photocraft_engine::EngineError::from)?;
                let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &serde_json::json!({}), false, control)?;
                Ok(photocraft_engine::accounted_filter_inspect::encode_cli(reply, true, crate::budgets::MAX_RESPONSE_BYTES, control)?)
            })
            .unwrap();
            let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
            assert_eq!(value["filters"]["total"], if empty { 0 } else { 1 });
            drop(wire);
            assert!(
                bounded(before.accounted(), |control| {
                    control.check(1).map_err(photocraft_engine::EngineError::from)?;
                    control.check(1u64 << 40).map_err(photocraft_engine::EngineError::from)?;
                    Ok(())
                })
                .is_err()
            );
            let calls = AtomicU64::new(0);
            let deny = |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Err(BackingError::Work)
            };
            let lower = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
            assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &serde_json::json!({}), true, &lower).is_err());
            assert!(calls.load(Ordering::Relaxed) > 0);
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("service query denied".into())));
            assert!(
                h.inspect_owned_command_cli_bounded("xcf.inspect", &serde_json::json!({}), true, crate::budgets::MAX_RESPONSE_BYTES)
                    .unwrap_err()
                    .to_string()
                    .contains("service query denied")
            );
            h.session.authorize = None;
            assert!(h.inspect_owned_command_cli_bounded("xcf.inspect", &serde_json::json!({}), true, 32).is_err());
            let wire = h.inspect_owned_command_rpc_bounded("xcf.inspect", &serde_json::json!({}), false, &serde_json::json!(91)).unwrap();
            let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
            assert_eq!(value["id"], 91);
            assert_eq!(value["ok"], true);
            drop(wire);
            if empty {
                assert!(h.inspect_owned_command_bounded("xcf.filter.inspect", &serde_json::json!({"filterId":"100"}), true).is_err());
            }
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            let output = directory.0.join("saved.pcraft");
            std::fs::write(&output, b"old destination").unwrap();
            trigger.arm(8_000_000_001);
            let old = photocraft_io::accounted_operation_work::OperationWork::legacy(8_000_000_000);
            let debit = |units| old.check(units);
            let external = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
            assert!(h.save_owned_native_active(Some(&output), Some("pcraft"), &external).is_err());
            assert!(trigger.hits.load(Ordering::Relaxed) > 0);
            assert_eq!(std::fs::read(&output).unwrap(), b"old destination");
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.active().unwrap().source, source);
            assert_eq!(h.session.journal, journal);
            let cancelled = || true;
            assert!(h.save_owned_native_active_bounded(Some(&output), Some("pcraft"), &Interrupt::cancel_only(&cancelled)).is_err());
            assert_eq!(std::fs::read(&output).unwrap(), b"old destination");
            trigger.arm(8_000_000_001);
            let hits = trigger.hits.load(Ordering::Relaxed);
            h.save_owned_native_active_bounded(Some(&output), Some("pcraft"), &Interrupt::NONE).unwrap();
            assert!(trigger.hits.load(Ordering::Relaxed) > hits);
            assert_eq!(h.session.active().unwrap().history.past_len(), history.past_len());
            assert_eq!(h.session.active().unwrap().source, source);
            let mut reopened = Headless::trusted_local();
            drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
            fixture::same(&before, &reopened.session.active().unwrap().doc);
            assert!(!h.session.has_jobs());
        }
    }
    // New query service intentionally leaves old Box on its legacy allowance.
    let (box_owner, _) = boxed::fixture(photocraft_doc::SampleType::F32, false, 0);
    assert!(
        bounded(Some(&box_owner), |control| {
            control.check(8_000_000_001).map_err(photocraft_engine::EngineError::from)?;
            Ok(())
        })
        .is_err()
    );
}
