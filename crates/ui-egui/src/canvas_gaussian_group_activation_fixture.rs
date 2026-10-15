//! UNRUN activation-only helpers. Existing independent disabled observations
//! remain distinct from edits to the current owned source in these workflows.
use photocraft_doc::Document;
use serde_json::Value;
use std::path::{Path, PathBuf};
#[path = "canvas_gaussian_group_fixture.rs"]
#[allow(dead_code)] // Shared fixture consumers intentionally use different subsets.
mod original;
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    original::ctl()
}
pub(super) fn assets() -> PathBuf {
    original::assets()
}
pub(super) fn payload(before: &Document, current: &Document) {
    assert_eq!(original::fingerprint(before), original::fingerprint(current));
    let mut expected = before.xcf_state.as_ref().unwrap().clone();
    let actual = current.xcf_state.as_ref().unwrap();
    expected.filter_stacks[0].nodes[0].enabled = actual.filter_stacks[0].nodes[0].enabled;
    expected.filter_stacks[0].revision = actual.filter_stacks[0].revision;
    assert_eq!(Some(expected), current.xcf_state);
    assert_eq!(before.icc_profile, current.icc_profile);
    assert_eq!(before.source_color_context, current.source_color_context);
    assert_eq!(before.working_profile_interpretation, current.working_profile_interpretation);
    assert_eq!(before.sample_encoding, current.sample_encoding);
}
pub(super) fn disabled_projection(root: &Path, phase: &str) -> Vec<[f64; 4]> {
    let root = root.parent().unwrap().join("group-disabled-evidence");
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(original::hash(&bytes), "06c29caff71c0da5c64ed8fbc59f7ff5de4a30d7a59d992d344b062f59779bcc");
    let packet: Value = serde_json::from_slice(&bytes).unwrap();
    let row = packet["records"].as_array().unwrap().iter().find(|r| r["phase"] == phase).unwrap();
    assert_eq!(
        row["source_sha256"],
        if phase == "source" {
            "740064c2b008a9785801b9de4425f685dc2618a7fcc938d37728723dd10ccc49"
        } else {
            "bf9f61f61cedbfdd6e3e3c25cb5066e4deef0c46eef16629d6546cfd1018706b"
        }
    );
    let state = &row["reopened"];
    assert_eq!(state["filter_visible"], false);
    assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
    assert!(state["profiles"]["assigned"].is_null());
    let r =
        state["projection"]["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"] == "R'G'B'A double").unwrap();
    let bytes = std::fs::read(root.join(r["file"].as_str().unwrap())).unwrap();
    assert_eq!(original::hash(&bytes), r["sha256"].as_str().unwrap());
    assert_eq!(bytes.len() as u64, r["bytes"].as_u64().unwrap());
    assert_eq!(bytes.len(), 16 * 12 * 32);
    bytes.chunks_exact(32).map(|p| std::array::from_fn(|c| f64::from_le_bytes(p[c * 8..c * 8 + 8].try_into().unwrap()))).collect()
}
