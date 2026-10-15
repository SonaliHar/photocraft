//! UNRUN genuine-source residual Point continuation drafts. Source and archive
//! Cursor buffers are external harness captures, not claims of paid production
//! buffer ownership. The reader, typed model and native provider remain paid.
use crate::Session;
use photocraft_doc::{LayerContent, xcf_filters::FilterValue};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn cases() -> [(&'static str, &'static str, &'static str); 6] {
    [
        ("desaturate-precision-evidence", "ff2d1ce5981edb0b3d4b5c22e01e8af015945160062d9b057d997e720f231603", "rgb-u16_linear.average.xcf"),
        ("saturation-precision-evidence", "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c", "rgb-float_non_linear.native-half.xcf"),
        ("saturation-precision-evidence", "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c", "gray-double_linear.cieyuv-half.xcf"),
        ("saturation-precision-evidence", "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c", "rgb-float_linear.cieyuv-half.xcf"),
        ("value-invert-precision-evidence", "8a75ccbb88e61611d3c3523db8145f922b63676601986c124d2d8493a27e73a1", "double_non_linear.source.xcf"),
        ("operations/gimp-posterize", "4eac864e360186bb24174f091b7b84d424e804177fe75c0330d6e888b801968a", "operation-gimp-posterize.xcf"),
    ]
}
fn source(folder: &str, pin: &str, name: &str) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(folder);
    let proof_name = if folder.starts_with("operations/") { "probe.json" } else { "batch-manifest.json" };
    let proof = std::fs::read(root.join(proof_name)).unwrap();
    assert_eq!(hash(&proof), pin);
    let proof: Value = serde_json::from_slice(&proof).unwrap();
    let expected = if proof_name == "probe.json" {
        assert_eq!(proof["cases"][0]["input"], name);
        proof["cases"][0]["input_sha256"].as_str().unwrap()
    } else {
        proof["artifacts"].as_array().unwrap().iter().find(|v| v["path"] == name).unwrap()["sha256"].as_str().unwrap()
    };
    let bytes = std::fs::read(root.join(name)).unwrap();
    assert_eq!(hash(&bytes), expected);
    bytes
}
fn imported(name: &str, bytes: &[u8], quota: &BackingQuota) -> photocraft_io::OwnedPointXcfImport {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    match photocraft_io::import_owned_xcf_from_reader_in_quota(
        name,
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        quota.clone(),
        &ctl(),
    )
    .unwrap()
    {
        photocraft_io::OwnedXcfImport::Point(owner) => owner,
        _ => panic!("pinned residual source must select Point without fallback"),
    }
}
fn native(session: &Session) -> String {
    let doc = &session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("paid native raster") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => digest.update(b),
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    digest.finalize().iter().map(|v| format!("{v:02x}")).collect()
}
fn argument_edit(session: &Session) -> Option<(Value, Value)> {
    let node = &session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let argument = node.arguments.first()?;
    let next = match (&*node.operation.name, &argument.value) {
        ("gimp:desaturate", FilterValue::Enum { domain, value }) => json!({"Enum":{"domain":domain,"value":if *value == 0 {1} else {0}}}),
        ("gegl:saturation", _) => json!({"Number":{"Float32Bits":0.75f32.to_bits()}}),
        ("gimp:posterize", _) => json!({"Number":{"Signed":{"width":32,"value":4}}}),
        _ => panic!("unexpected recorded operation"),
    };
    let name = if node.operation.name == "gegl:saturation" { "scale" } else { &argument.name };
    let original = node.arguments.iter().find(|v| v.name == name).unwrap();
    Some((
        json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":next}]}),
        json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":original.value}]}),
    ))
}

#[test]
#[ignore = "UNRUN: genuine residual original -> registered typed edits -> undo/no-op/redo -> native continuation"]
fn residual_typed_edits_and_activation_keep_native_profile_depth_and_history_contracts() {
    for (folder, pin, name) in cases() {
        let bytes = source(folder, pin, name);
        let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let owner = imported(name, &bytes, &quota);
        let mut session = Session::new();
        session.add_owned_point_xcf_import(&owner, name, None, &ctl()).unwrap();
        let initial = session.active().unwrap().doc.clone();
        let raw = native(&session);
        let node = &initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        let (command, changed, unchanged) = if let Some((next, old)) = argument_edit(&session) {
            ("xcf.filter.set", next, old)
        } else {
            ("xcf.filter.enable", json!({"filterId":id,"enabled":!node.enabled}), json!({"filterId":id,"enabled":node.enabled}))
        };
        assert_eq!(session.execute(command, changed.clone()).unwrap()["updated"], true, "{name}");
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&initial), "one undo restores original {name}");
        let history = session.active().unwrap().history.clone();
        let revision = session.active().unwrap().revision;
        assert_eq!(session.execute(command, unchanged).unwrap()["updated"], false);
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().history.shares_accounted_history(&history));
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.execute(command, changed).unwrap()["updated"], false);
        assert_eq!(native(&session), raw);
        let current = session.active().unwrap().doc.clone();
        assert_eq!(current.depth, initial.depth);
        assert_eq!(current.icc_profile, initial.icc_profile);
        assert_eq!(current.working_profile_interpretation, initial.working_profile_interpretation);
        assert_eq!(current.source_color_context, initial.source_color_context);
        // External harness writer buffer. Production archive internals and the
        // reopen reader/provider retain actual same-quota fees independently.
        let saved = photocraft_format::owned_document::save_current_document_archive(&current, Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive, "continued.pcraft", None, &ctl()).unwrap();
        assert_eq!(reopened.active().unwrap().doc.xcf_state, current.xcf_state);
        assert_eq!(reopened.active().unwrap().doc.depth, initial.depth);
        assert_eq!(reopened.active().unwrap().doc.icc_profile, initial.icc_profile);
        assert_eq!(native(&reopened), raw);
        let enabled = reopened.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled;
        assert_eq!(reopened.execute("xcf.filter.enable", json!({"filterId":id,"enabled":!enabled})).unwrap()["updated"], true);
        assert_eq!(native(&reopened), raw);
        assert_eq!(hash(&bytes), hash(&source(folder, pin, name)));
    }
}

#[test]
#[ignore = "UNRUN: residual wrong raw type and cancelled current edit preserve model, history and quota"]
fn residual_refused_edit_preserves_current_state_and_fees() {
    for (folder, pin, name) in cases() {
        let bytes = source(folder, pin, name);
        let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let owner = imported(name, &bytes, &quota);
        let mut session = Session::new();
        session.add_owned_point_xcf_import(&owner, name, None, &ctl()).unwrap();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.clone();
        let used = quota.usage().unwrap();
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let argument = node.arguments.first().map(|v| v.name.as_str()).unwrap_or("absent");
        assert!(session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":argument,"value":{"Bool":true}}]})).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(quota.usage().unwrap(), used);
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        let enable = json!({"filterId":node.id.0.to_string(),"enabled":!node.enabled});
        assert!(crate::accounted_commands::dispatch(&mut session, "xcf.filter.enable", &enable, &control).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(quota.usage().unwrap(), used);
    }
}
