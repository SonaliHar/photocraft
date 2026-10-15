//! UNRUN six scalar-domain workflows from independently authored, pinned XCFs.
//! File Vecs and the temporary directory are external harness captures. The
//! actual selected reader, paid replies, native writer and reopened owners
//! retain their production fees; these drafts confer no source authority.
use super::*;
use photocraft_doc::{
    LayerContent,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

struct Case {
    path: &'static str,
    sha: &'static str,
    operation: &'static str,
}
const CASES: [Case; 6] = [
    Case {
        path: "exposure-geometry-evidence/rgb-double_non_linear.negative-x.source.xcf",
        sha: "cc29e9aaae63f97f4f1e01ad49fabc9759b165e9f1496bc70edf728605acf53b",
        operation: "gegl:exposure",
    },
    Case { path: "rgb-clip-hdr-evidence/clip.source.xcf", sha: "33dce24ca99ee3ae8c60b05eb16398498099e7353e7e1698a2efde2e3b3d02a4", operation: "gegl:rgb-clip" },
    Case {
        path: "point-unpremultiply-evidence/unpremultiply-rgb-float_linear.small.source.xcf",
        sha: "21e4faa0915abbf9f0a89ac4c80584392f98849514d8b870ef9db0deb1a9cb79",
        operation: "gegl:unpremultiply",
    },
    Case {
        path: "operations/gegl-mono-mixer/operation-gegl-mono-mixer.xcf",
        sha: "dc7009be42d698b448efd45c6b38a18f80a816a57033a44c031e6fa1d8d5880a",
        operation: "gegl:mono-mixer",
    },
    Case {
        path: "mono-mixer-edits-evidence/mono.source.xcf",
        sha: "32151370e3768e004248e01a59576150ef000572e992d48e52667ff3cb1709b7",
        operation: "gegl:mono-mixer",
    },
    Case {
        path: "operations/gegl-value-invert/operation-gegl-value-invert.xcf",
        sha: "9cd1650e6ddfaba7f3dd8dc9e5ae5ac2cec46b962685973dbe687b6235f6916f",
        operation: "gegl:value-invert",
    },
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-scalar-headless-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn source(case: &Case) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let bytes = std::fs::read(root.join(case.path)).unwrap();
    assert_eq!(hash(&bytes), case.sha);
    bytes
}
fn native(h: &Headless) -> (photocraft_raster::backing::NativePlaneDescriptor, String) {
    let doc = &h.session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("genuine native owner") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    // Include every padded native tile, including off-canvas storage for the
    // independently authored negative-offset exposure geometry.
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(bytes) | NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    (layer.descriptor(), digest.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn node(h: &Headless) -> &photocraft_doc::xcf_filters::FilterNode {
    &h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
}
fn exact_set(h: &Headless) -> Option<Value> {
    let node = node(h);
    if node.arguments.is_empty() {
        return None;
    }
    let arguments: Vec<_> = node.arguments.iter().map(|a| json!({"name":a.name,"value":a.value})).collect();
    Some(json!({"filterId":node.id.0.to_string(),"arguments":arguments}))
}
fn argument_shape(h: &Headless, case: &Case) {
    let node = node(h);
    assert_eq!(node.operation.name, case.operation);
    let expected: &[(&str, bool)] = match case.operation {
        "gegl:exposure" => &[("black-level", false), ("exposure", false)],
        "gegl:rgb-clip" => &[("clip-low", true), ("low-limit", false), ("clip-high", true), ("high-limit", false)],
        "gegl:mono-mixer" => &[("preserve-luminosity", true), ("red", false), ("green", false), ("blue", false)],
        "gegl:unpremultiply" | "gegl:value-invert" => &[],
        _ => panic!("closed fixture operation"),
    };
    assert_eq!(node.arguments.len(), expected.len());
    for (name, boolean) in expected {
        let a = node.arguments.iter().find(|a| a.name == *name).unwrap();
        if *boolean {
            assert!(matches!(a.value, FilterValue::Bool(_)));
        } else {
            assert!(matches!(a.value, FilterValue::Number(ExactNumber::Float32Bits(_))));
        }
    }
}

#[test]
#[ignore = "UNRUN: six exact authored scalar domains, registered activation/noopredo and paid native continuation"]
fn six_scalar_domains_keep_exact_argument_shapes_native_words_and_redo_through_native_reopen() {
    for case in &CASES {
        let dir = Directory::new();
        let original = source(case);
        let path = dir.0.join("original.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        assert!(h.session.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
        argument_shape(&h, case);
        let before = h.session.active().unwrap().doc.clone();
        let raw = native(&h);
        let id = node(&h).id.0.to_string();
        let enabled = node(&h).enabled;
        let exact = exact_set(&h);
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":!enabled})).unwrap()["updated"], true);
        assert_eq!(h.session.active().unwrap().history.past_len(), 1);
        h.command_run("edit.undo", json!({})).unwrap();
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let params = exact.unwrap_or_else(|| json!({"filterId":id,"enabled":enabled}));
        assert_eq!(h.command_run("xcf.filter.set", params).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert_eq!(node(&h).enabled, !enabled);
        let edited = h.session.active().unwrap().doc.clone();
        let inspection = h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        let rows = inspection.value()["arguments"].as_array().unwrap();
        assert_eq!(rows.len(), node(&h).arguments.len());
        for argument in &node(&h).arguments {
            let row = rows.iter().find(|row| row["name"] == argument.name).unwrap();
            assert_eq!(row["state"]["value"], serde_json::to_value(&argument.value).unwrap());
        }
        drop(inspection);
        assert_eq!(native(&h), raw);
        let output = dir.0.join("continued.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        assert!(!current.source_read_only);
        assert_eq!(current.doc.xcf_state, edited.xcf_state);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(native(&reopened), raw);
        argument_shape(&reopened, case);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":enabled})).unwrap()["updated"], true);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
#[ignore = "UNRUN: six typed scalar domains retain synchronous authorizers and reject malformed/unsupported argument shapes"]
fn scalar_nonwaiting_authorization_and_wrong_argument_requests_leave_current_state_unchanged() {
    for case in &CASES {
        let dir = Directory::new();
        let path = dir.0.join("original.xcf");
        let original = source(case);
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        argument_shape(&h, case);
        let before = h.session.active().unwrap().doc.clone();
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        let id = node(&h).id.0.to_string();
        for command in ["layer.setProps", "xcf.filter.set", "xcf.filter.enable"] {
            assert!(
                matches!(h.command_start(command,json!({"malformed":[[[[[0]]]]]}),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
            );
            assert!(!h.session.has_jobs());
        }
        // No-argument operations only support activation; their empty SET
        // cannot invent an argument. Others retain their independent type.
        let bad = match node(&h).arguments.first() {
            None => json!({"filterId":id,"arguments":[]}),
            Some(a) => json!({"filterId":id,"arguments":[{"name":a.name,"value":{"Number":{"Float32Bits":0x7fc00001u32}}}]}),
        };
        assert!(h.command_run("xcf.filter.set", bad).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny scalar Point edit".into())));
        let exact = exact_set(&h).unwrap_or_else(|| json!({"filterId":id,"enabled":node(&h).enabled}));
        for (command, params) in
            [("xcf.filter.set", exact), ("xcf.filter.enable", json!({"filterId":id,"enabled":!node(&h).enabled})), ("layer.setProps", json!({"name":"denied"}))]
        {
            assert!(h.command_run(command, params).is_err());
        }
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
