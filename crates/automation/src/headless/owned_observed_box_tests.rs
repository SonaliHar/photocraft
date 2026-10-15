//! UNRUN exact six local GIMP originals plus distinct synthetic CURRENT tuple.
//! No corpus enrollment or producer geometry/numerical acceptance claim.
use super::*;
#[path = "../../../ui-egui/src/canvas_observed_box_fixture.rs"]
mod observed;
fn refuse_mutations(h: &mut Headless) {
    let before = h.session.active().unwrap().doc.clone();
    let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let history = h.session.active().unwrap().history.clone();
    let revision = h.session.active().unwrap().revision;
    let journal = h.session.journal.len();
    for (command, params) in [
        ("xcf.filter.set", json!({"filterId":id,"enabled":null})),
        ("xcf.filter.enable", json!({"filterId":id,"enabled":false,"opacityF64Bits":"0x3ff0000000000000"})),
        ("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"value":1001,"width":32}}}}]})),
        ("layer.setProps", json!({"blend":"multiply"})),
        ("layer.translate", json!({"target":"mask","dx":1})),
        ("select.all", Value::Null),
        ("edit.clear", Value::Null),
    ] {
        assert!(h.command_start(command, params.clone(), false).is_err(), "nonwaiting {command}");
        assert!(h.command_run(command, params).is_err(), "{command}");
    }
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.journal.len(), journal);
    assert!(!h.session.has_jobs());
}
#[test]
#[ignore = "UNRUN: six SHA-pinned independent local observed Box originals through strict owned source/native/query/persistence and mutation closure; separate paid synthetic CURRENT MIN/0/MAX opaque tuple control, no corpus enrollment"]
fn observed_box_originals_and_current_native_are_read_only() {
    use sha2::{Digest, Sha256};
    for (path, bytes) in observed::source_cases() {
        let dir = Directory::new();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&path).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        assert!(h.session.active().unwrap().source_read_only);
        assert_eq!(observed::raw(&before), [1654948433; 3]);
        assert_eq!(before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch, observed::EPOCH);
        // Independent author seed samples are local f32LE. Hash that precise
        // canonical byte role, not generated display/effective profile bytes.
        let native = fixture::native(&before);
        let little: Vec<u8> = native.chunks_exact(4).flat_map(|v| u32::from_ne_bytes(v.try_into().unwrap()).to_le_bytes()).collect();
        assert_eq!(format!("{:x}", Sha256::digest(&little)), "72fc0fa6db04a4c04240d9db9e270517b5f907a6dae4e681c05173fe8567ec2e");
        let identity = h.session.active().unwrap().source.clone();
        for wait in [false, true] {
            let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, wait, &ctl()).unwrap();
            assert_eq!(query.value()["items"]["total"], 2);
            assert_eq!(query.value()["filters"]["total"], 1);
            drop(query);
        }
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("observed query denied".into())));
        match h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => assert!(e.to_string().contains("observed query denied")),
            Ok(_) => panic!("supported query authorizer"),
        }
        h.session.authorize = None;
        refuse_mutations(&mut h);
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("observed.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, identity);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
        let current = &reopened.session.active().unwrap().doc;
        fixture::exact(&before, current);
        assert_eq!(observed::raw(current), [1654948433; 3]);
        assert!(!reopened.session.active().unwrap().source_read_only);
        refuse_mutations(&mut reopened);
    }
    let dir = Directory::new();
    let (base, literal) = fixture::fixture(SampleType::F32, false, 0);
    let owner = observed::with_raw(&base, [i32::MIN, 0, i32::MAX], &ctl());
    let titled = owner.try_with_document_name("Opaque tuple CURRENT", &ctl()).unwrap();
    let mut seed = Headless::trusted_local();
    seed.session.add_owned_native_point_archive(fixture::archive(&titled), "opaque.pcraft", None, &ctl()).unwrap();
    let path = dir.0.join("opaque.pcraft");
    seed.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).unwrap();
    let mut reopened = Headless::trusted_local();
    drop(reopened.open_owned_native_keep_current_bounded(&path).unwrap());
    let current = &reopened.session.active().unwrap().doc;
    assert_eq!(current.name, "Opaque tuple CURRENT");
    assert_eq!(observed::raw(current), [i32::MIN, 0, i32::MAX]);
    assert_eq!(fixture::native(current), literal.raw);
    fixture::exact(titled.document(), current);
    refuse_mutations(&mut reopened);
}
