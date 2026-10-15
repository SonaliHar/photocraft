//! UNRUN actual owned-reader / retained-wire CLI integration drafts.
//! Corpus SHA authority remains in the companion engine evidence tests.
use super::*;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-owned-inspect-cli-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn invoke(args: &[&str]) -> (i32, String, String) {
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = crate::run(&args.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>(), &mut output, &mut error);
    (code, String::from_utf8(output).unwrap(), String::from_utf8(error).unwrap())
}
fn source(histogram: bool) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires independently saved precision packets"));
    let (packet, name) = if histogram {
        ("threshold-precision-evidence", "rgb-double_non_linear.channel0.xcf")
    } else {
        ("alpha-threshold-precision-evidence", "rgb-double_non_linear.half.xcf")
    };
    let root = root.join(packet);
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("SHA256SUMS.json")).unwrap()).unwrap();
    assert_eq!(manifest[name].as_str().unwrap().len(), 64);
    assert!(std::fs::metadata(root.join(name)).unwrap().len() <= 16 << 20);
    std::fs::read(root.join(name)).unwrap()
}
fn owned(histogram: bool, dir: &Directory) -> Headless {
    std::fs::write(dir.0.join("source.xcf"), source(histogram)).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current_bounded(&dir.0.join("source.xcf")).unwrap());
    assert!(h.has_owned_filter_inspection());
    h
}
fn request(h: &Headless) -> Value {
    json!({"filterId":h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}
struct Probe<F, G>(F, G);
impl<F: FnMut(&[u8]) -> std::io::Result<usize>, G: FnMut() -> std::io::Result<()>> Write for Probe<F, G> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        (self.0)(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (self.1)()
    }
}
#[test]
#[ignore = "UNRUN: real owned Alpha/Histogram sources and paid CLI transport"]
fn wire_fee_stays_held_through_each_write_and_failed_output_does_not_mutate_or_leak() {
    for histogram in [false, true] {
        let dir = Directory::new();
        let mut h = owned(histogram, &dir);
        let params = request(&h);
        assert!(handles(&h, "xcf.filter.inspect"));
        assert!(!handles(&h, "xcf.filter.set"));
        let before = h.session.active().unwrap().doc.clone();
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.clone();
        let revision = h.session.active().unwrap().revision;
        let q = before.quota().unwrap().clone();
        let baseline = q.usage().unwrap();
        let mut bytes = Vec::new();
        let mut calls = 0;
        write(
            &mut h,
            &params,
            &mut Probe(
                |chunk: &[u8]| {
                    assert!(q.usage().unwrap().index > baseline.index);
                    calls += 1;
                    bytes.extend_from_slice(chunk);
                    Ok(chunk.len())
                },
                || {
                    assert!(q.usage().unwrap().index > baseline.index);
                    Ok(())
                },
            ),
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(bytes.last(), Some(&b'\n'));
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["command"], "xcf.filter.inspect");
        assert_eq!(value["result"]["filterId"], params["filterId"]);
        assert!(value.get("warnings").is_none());
        assert_eq!(q.usage().unwrap(), baseline);
        let mut wrote = 0;
        assert!(
            write(
                &mut h,
                &params,
                &mut Probe(
                    |chunk: &[u8]| {
                        assert!(q.usage().unwrap().index > baseline.index);
                        wrote += 1;
                        if wrote == 1 { Ok(chunk.len()) } else { Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "draft writer failure")) }
                    },
                    || {
                        assert!(q.usage().unwrap().index > baseline.index);
                        Ok(())
                    }
                )
            )
            .is_err()
        );
        assert_eq!(wrote, 2);
        assert_eq!(q.usage().unwrap(), baseline);
        assert!(
            write(
                &mut h,
                &params,
                &mut Probe(
                    |chunk: &[u8]| Ok(chunk.len()),
                    || {
                        assert!(q.usage().unwrap().index > baseline.index);
                        Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "draft flush failure"))
                    }
                )
            )
            .is_err()
        );
        assert_eq!(q.usage().unwrap(), baseline);
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.journal, journal);
        assert_eq!(h.session.active().unwrap().revision, revision);
        for bad in [Value::Null, json!({}), json!({"filterId":1}), json!({"filterId":"01"}), json!({"filterId":params["filterId"],"extra":true})] {
            let mut destination = Vec::new();
            assert!(write(&mut h, &bad, &mut destination).is_err());
            assert!(destination.is_empty());
            assert_eq!(q.usage().unwrap(), baseline);
        }
        assert!(h.command_run("xcf.filter.inspect", params.clone()).is_err());
        let batch = h.handle("batch", json!({"steps":[{"command":"xcf.filter.inspect","params":params}]})).unwrap();
        assert_eq!(batch["failed"], 1);
        assert_eq!(batch["completed"], 0);
        assert!(h.session.active().unwrap().doc.shares_state(&before));
    }
}
#[test]
#[ignore = "UNRUN: real owned source CLI inspect/edit/history/native persistence"]
fn actual_cli_compact_inspect_edit_undo_save_native_reopen_preserves_recipe_and_original() {
    for histogram in [false, true] {
        let dir = Directory::new();
        let h = owned(histogram, &dir);
        let params = request(&h);
        let filter = params["filterId"].as_str().unwrap();
        let original = std::fs::read(dir.0.join("source.xcf")).unwrap();
        let node = &h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let field = if histogram { "low" } else { "value" };
        let current = serde_json::to_value(node.arguments.iter().find(|a| a.name == field).unwrap()).unwrap();
        let bits = if current["value"]["Number"]["Float32Bits"] == 0.875f32.to_bits() { 0.25f32.to_bits() } else { 0.875f32.to_bits() };
        let inspect = params.to_string();
        let patch = json!({"filterId":filter,"arguments":[{"name":field,"value":{"Number":{"Float32Bits":bits}}}]}).to_string();
        let input = dir.0.join("source.xcf");
        let output = dir.0.join("edited.pcraft");
        let (code, text, error) = invoke(&[
            "run",
            input.to_str().unwrap(),
            "--xcf-profile-policy",
            "keepCurrent",
            "--cmd",
            "xcf.filter.inspect",
            "--params",
            &inspect,
            "--cmd",
            "xcf.filter.set",
            "--params",
            &patch,
            "--cmd",
            "xcf.filter.inspect",
            "--params",
            &inspect,
            "--cmd",
            "edit.undo",
            "--cmd",
            "xcf.filter.inspect",
            "--params",
            &inspect,
            "--cmd",
            "edit.redo",
            "--out",
            output.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{error}");
        assert!(error.is_empty());
        let lines: Vec<Value> = text.lines().map(|v| serde_json::from_str(v).unwrap()).collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0]["result"], lines[4]["result"]);
        assert_eq!(lines[0]["command"], "xcf.filter.inspect");
        assert_eq!(lines[2]["command"], "xcf.filter.inspect");
        assert_ne!(lines[0]["result"]["arguments"], lines[2]["result"]["arguments"]);
        assert_eq!(std::fs::read(&input).unwrap(), original);
        let saved = std::fs::read(&output).unwrap();
        let continued = dir.0.join("continued.pcraft");
        let (code, text, error) = invoke(&[
            "run",
            output.to_str().unwrap(),
            "--native-profile-policy",
            "keepCurrent",
            "--cmd",
            "xcf.filter.inspect",
            "--params",
            &inspect,
            "--out",
            continued.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{error}");
        let reply: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["result"], lines[2]["result"]);
        assert!(reply.get("warnings").is_none());
        assert_eq!(std::fs::read(&output).unwrap(), saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&continued).unwrap());
        assert_eq!(reopened.session.active().unwrap().doc.name, "continued.pcraft");
        assert!(reopened.has_owned_filter_inspection());
        // The unchanged final output route still enforces original identity.
        let alias = dir.0.join("alias.pcraft");
        std::fs::hard_link(&input, &alias).unwrap();
        let (code, _, _) = invoke(&[
            "run",
            input.to_str().unwrap(),
            "--xcf-profile-policy",
            "keepCurrent",
            "--cmd",
            "xcf.filter.inspect",
            "--params",
            &inspect,
            "--out",
            alias.to_str().unwrap(),
        ]);
        assert_ne!(code, 0);
        assert_eq!(std::fs::read(&input).unwrap(), original);
    }
}
#[test]
fn unsupported_empty_owner_does_not_intercept_ordinary_commands() {
    let h = Headless::trusted_local();
    assert!(!handles(&h, "xcf.filter.inspect"));
    assert!(!handles(&h, "file.new"));
}
