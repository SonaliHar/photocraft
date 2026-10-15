//! UNRUN explicit installed-reader capability controls before bytes/metadata.
use super::*;
use std::{cell::Cell, rc::Rc};
#[test]
#[ignore = "UNRUN: explicit owned XCF/native reader receives finite standard work before parse without Box hint, exhausted same allowance refuses publication and never retries ordinary services"]
fn explicit_owned_reader_policy_precedes_content() {
    for native in [false, true] {
        let calls = Rc::new(Cell::new(0usize));
        let seen = calls.clone();
        let mut services = crate::Services::default();
        if native {
            services.automation_owned_native_open = Some(Box::new(move |_, _, control| {
                seen.set(seen.get() + 1);
                // Synthetic service debit, not measured parsing/I/O work.
                control.check(8_000_000_001).map_err(|e| e.to_string())?;
                assert!(control.check(1u64 << 40).is_err());
                Err("reader refused before input".into())
            }));
            services.owned_native_open = Some(Box::new(|_, _, _| panic!("interactive fallback")));
        } else {
            services.automation_owned_current_xcf_open = Some(Box::new(move |_, _, control| {
                seen.set(seen.get() + 1);
                control.check(8_000_000_001).map_err(|e| e.to_string())?;
                assert!(control.check(1u64 << 40).is_err());
                Err("reader refused before input".into())
            }));
            services.automation_owned_xcf_open = Some(Box::new(|_, _, _| panic!("plain retry")));
            services.owned_current_xcf_open = Some(Box::new(|_, _, _| panic!("interactive fallback")));
        }
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let result = if native {
            crate::owned_native_open::invoke_authorized(&mut app, &json!({"path":"explicit.pcraft","nativeProfilePolicy":"keepCurrent"}))
        } else {
            invoke_authorized(&mut app, &json!({"path":"explicit.xcf","xcfProfilePolicy":"keepCurrent"}))
        };
        assert_eq!(result.unwrap_err(), "reader refused before input");
        assert_eq!(calls.get(), 1);
        assert!(app.session.documents().is_empty());
        assert!(app.session.journal.is_empty());
    }
}
