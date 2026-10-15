//! Deferred authored storage and CURRENT transactions; producer math remains HOLD.
use super::*;
#[path = "../../../ui-egui/src/vibrance_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-vibrance-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "UNRUN: 12 real authored Vibrance RGB NL F32/F64 source/edit inputs through paid source/native/query/two-F32+enable/history/registered readonly props/title/save, exact native/raw/profile/source roles, auth/waitfalse/stale refusal; RGB/LCh rounding, HDR output, strong F32 producer instability (~.475) and scalar Saturation precision and ICC HOLDs retained, no numerical acceptance"]
fn authored_vibrance_source_and_current_native_lifecycle() {
    for (path, bytes) in fixture::old_sources() {
        let mut refused = Headless::trusted_local();
        assert!(refused.open_owned_xcf_keep_current_bounded(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(refused.session.active().is_err());
    }
    for case in fixture::cases() {
        let directory = Directory::new();
        let mut source = Headless::trusted_local();
        drop(source.open_owned_xcf_keep_current_bounded(&case.path).unwrap());
        let original = source.session.active().unwrap().doc.clone();
        fixture::observed(&original, &case);
        assert!(source.session.active().unwrap().source_read_only);
        let protected = directory.0.join("source-copy.pcraft");
        std::fs::write(&protected, &case.bytes).unwrap();
        assert!(source.save_owned_native_active(Some(&protected), Some("pcraft"), &fixture::ctl()).is_err());
        assert_eq!(std::fs::read(&protected).unwrap(), case.bytes);
        let native = directory.0.join("original.pcraft");
        source.save_owned_native_active(Some(&native), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&native).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::observed(&before, &case);
        fixture::storage(&original, &before);
        let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let next = [(-0.0f32).to_bits(), 1.125f32.to_bits()];
        let params = fixture::wire(&filter, next, Some(false));
        assert!(h.command_start("xcf.filter.set", params.clone(), false).unwrap_err().to_string().contains("synchronous"));
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("vibrance edit denied".into())));
        assert!(h.command_start("xcf.filter.set", params.clone(), true).unwrap_err().to_string().contains("vibrance edit denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(h.command_start("xcf.filter.set", params, true).unwrap()["updated"], true);
        let edited = h.session.active().unwrap().doc.clone();
        fixture::storage(&before, &edited);
        assert_eq!(fixture::args(&edited), next);
        assert!(!edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        h.command_run("edit.undo", Value::Null).unwrap();
        let history = h.session.active().unwrap().history.clone();
        assert_eq!(h.command_run("xcf.filter.set", fixture::wire(&filter, case.args, Some(true))).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&edited));
        let disabled = directory.0.join("disabled.pcraft");
        h.save_owned_native_active(Some(&disabled), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut continued = Headless::trusted_local();
        drop(continued.open_owned_native_keep_current_bounded(&disabled).unwrap());
        fixture::storage(&edited, &continued.session.active().unwrap().doc);
        assert_eq!(fixture::args(&continued.session.active().unwrap().doc), next);
        continued.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":true})).unwrap();
        let anchor = continued.session.active().unwrap().doc.clone();
        for (name, value) in [
            ("vibrance", json!({"Number":{"Float32Bits":101f32.to_bits()}})),
            ("saturation", json!({"Number":{"Float32Bits":10.01f32.to_bits()}})),
            ("saturation", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
            ("vibrance", json!({"Number":{"Float64Bits":"0x0000000000000000"}})),
            ("unknown", json!({"Bool":true})),
        ] {
            assert!(continued.command_run("xcf.filter.set", json!({"filterId":filter,"arguments":[{"name":name,"value":value}]})).is_err());
        }
        assert!(continued.session.active().unwrap().doc.shares_state(&anchor));
        continued.command_run("layer.setProps", json!({"name":"Vibrance CURRENT","visible":true,"opacity":0.375})).unwrap();
        let current = continued.session.active().unwrap().doc.clone();
        fixture::storage(&before, &current);
        for (command, params) in [("layer.translate", json!({"dx":1})), ("layer.setProps", json!({"blend":"multiply"})), ("select.all", Value::Null)] {
            assert!(continued.command_run(command, params).is_err());
        }
        assert!(continued.session.active().unwrap().doc.shares_state(&current));
        let query = continued.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":filter}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gegl:vibrance");
        assert_eq!(query.value()["argumentCount"], 2);
        for (name, bits) in ["vibrance", "saturation"].into_iter().zip(next) {
            let row = query.value()["arguments"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap();
            assert_eq!(row["state"]["value"], json!({"Number":{"Float32Bits":bits}}));
        }
        drop(query);
        let query = continued.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 2);
        assert_eq!(query.value()["filters"]["total"], 1);
        drop(query);
        let titled = current.accounted().unwrap().try_with_document_name("Vibrance native continuation", &fixture::ctl()).unwrap();
        let mut final_session = photocraft_engine::Session::new();
        final_session.add_owned_native_point_archive(fixture::archive(&titled), "continued.pcraft", None, &fixture::ctl()).unwrap();
        fixture::storage(&current, &final_session.active().unwrap().doc);
        assert_eq!(final_session.active().unwrap().doc.name, "Vibrance native continuation");
        assert_eq!(fixture::args(&final_session.active().unwrap().doc), next);
        // Separate derived empty CURRENT continuation, never a producer source claim.
        let mut stack = Headless::trusted_local();
        drop(stack.open_owned_native_keep_current_bounded(&native).unwrap());
        let history = stack.session.active().unwrap().history.clone();
        assert_eq!(stack.command_run("xcf.filter.move", json!({"filterId":filter,"index":0})).unwrap()["updated"], false);
        assert!(stack.session.active().unwrap().history.shares_accounted_history(&history));
        stack.command_run("xcf.filter.remove", json!({"filterId":filter})).unwrap();
        let empty = stack.session.active().unwrap().doc.clone();
        assert!(empty.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        assert!(empty.xcf_state.as_ref().unwrap().filter_masks.is_empty());
        assert_eq!(fixture::native_bytes(&empty), fixture::native_bytes(&before));
        assert_eq!(empty.icc_profile, before.icc_profile);
        assert_eq!(empty.source_color_context, before.source_color_context);
        stack.command_run("edit.undo", Value::Null).unwrap();
        let history = stack.session.active().unwrap().history.clone();
        stack.command_run("xcf.filter.set", fixture::wire(&filter, case.args, None)).unwrap();
        assert!(stack.session.active().unwrap().history.shares_accounted_history(&history));
        stack.command_run("edit.redo", Value::Null).unwrap();
        let removed = directory.0.join("removed.pcraft");
        stack.save_owned_native_active(Some(&removed), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&removed).unwrap());
        assert_eq!(reopened.session.active().unwrap().doc.xcf_state, empty.xcf_state);
        assert_eq!(fixture::native_bytes(&reopened.session.active().unwrap().doc), fixture::native_bytes(&empty));
        assert_eq!(std::fs::read(&case.path).unwrap(), case.bytes);
        assert!(!continued.session.has_jobs());
    }
}
