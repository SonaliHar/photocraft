//! UNRUN: authentic 64-case NL/Linear/Perceptual Union geometry host continuation. Source and
//! output Vecs/files are external harness captures, not quota receipts.
use super::*;
use photocraft_doc::{Document, LayerContent};
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn cases() -> Vec<(PathBuf, Value)> {
    let corpus = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut result = Vec::new();
    for (set, pins) in [
        ("holdout", include_str!("../../../compose/tests/gimp_normal_mask_geometry_hashes.json")),
        ("integer", include_str!("../../../compose/tests/gimp_normal_mask_geometry_integer_hashes.json")),
        ("linear", include_str!("../../../compose/tests/gimp_normal_mask_geometry_linear_hashes.json")),
        ("perceptual", include_str!("../../../compose/tests/gimp_normal_mask_geometry_perceptual_hashes.json")),
    ] {
        let pins: Value = serde_json::from_str(pins).unwrap();
        for variant in ["left-top", "right-bottom"] {
            let root = corpus.join(format!("normal-mask-geometry-{set}-evidence/{variant}"));
            for name in [
                "observations.json",
                "fresh/observations.json",
                "faithful-observations.json",
                "faithful-fresh/faithful-observations.json",
                "VERIFICATION.json",
                "assignment-controls.json",
                "batch-manifest.json",
            ] {
                assert_eq!(hash(&std::fs::read(root.join(name)).unwrap()), pins[variant][name].as_str().unwrap());
            }
            let data: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
            for row in data["records"].as_array().unwrap() {
                if row["layers"][0]["spaces"]["composite_mode"] != 1 {
                    continue;
                }
                let expected = match set {
                    "linear" => "_LINEAR",
                    "perceptual" => "_PERCEPTUAL",
                    _ => "_NON_LINEAR",
                };
                assert!(row["precision"].as_str().unwrap().ends_with(expected));
                assert_eq!(hash(&std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap()), row["source_sha256"].as_str().unwrap());
                // Frozen JSON binds these independent captured profile values.
                // Loose .icc files are absent in this corpus layout; below we
                // hash actual imported ICC bytes directly against these pins.
                assert_eq!(row["assigned_icc"]["sha256"], row["effective_icc"]["sha256"]);
                assert_eq!(row["assigned_icc"]["bytes"], row["effective_icc"]["bytes"]);
                result.push((root.clone(), row.clone()));
            }
        }
    }
    assert_eq!(result.len(), 64);
    result
}
// Observation ICC values describe GIMP source/profile roles. The canonical
// current ICC for Linear/Perceptual is the existing CMS derivative of ORIGINAL;
// parsing its rounded serialized curves as original would change the recipe.
fn profiles(doc: &Document, row: &Value) {
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_deref().unwrap();
    assert_eq!(hash(original), row["assigned_icc"]["sha256"].as_str().unwrap());
    assert_eq!(original.len() as u64, row["assigned_icc"]["bytes"].as_u64().unwrap());
    assert_eq!(hash(original), row["effective_icc"]["sha256"].as_str().unwrap());
    assert_eq!(original.len() as u64, row["effective_icc"]["bytes"].as_u64().unwrap());
    let current = doc.icc_profile.as_deref().unwrap();
    let encoding = match doc.sample_encoding.unwrap() {
        photocraft_doc::SampleEncoding::NonlinearProfile => {
            assert_eq!(current, original);
            return;
        }
        photocraft_doc::SampleEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
        photocraft_doc::SampleEncoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
    };
    let space = match doc.mode {
        photocraft_doc::ColorMode::Rgb => photocraft_cms::ColorSpace::Rgb,
        photocraft_doc::ColorMode::Grayscale => photocraft_cms::ColorSpace::Gray,
        _ => panic!("closed RGB/Gray geometry evidence"),
    };
    let expected = photocraft_cms::xcf_color::effective_profile(Some(original), space, encoding).unwrap().to_bytes();
    assert_eq!(current, expected.as_ref());
    assert_ne!(current, original);
}
fn native(doc: &Document) -> Vec<(photocraft_raster::backing::NativePlaneDescriptor, String)> {
    doc.layers
        .iter()
        .map(|layer| {
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("actual native owner") };
            let mut d = Sha256::new();
            d.update(backed.descriptor().default_bytes());
            for coord in backed.descriptor().bounds().tiles() {
                let pin = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
                match pin.encoded() {
                    NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => d.update(b),
                }
            }
            if let Some(mask) = &layer.mask {
                d.update(format!("{:?}", (mask.surface.tile_bounds(), mask.surface.format(), mask.enabled, mask.linked)).as_bytes());
                d.update(mask.density.to_bits().to_ne_bytes());
                d.update(mask.feather.to_bits().to_ne_bytes());
                d.update(mask.surface.default_bytes());
                for (coord, tile) in mask.surface.tiles() {
                    d.update(format!("{coord:?}").as_bytes());
                    d.update(tile.bytes());
                }
            }
            (backed.descriptor(), d.finalize().iter().map(|v| format!("{v:02x}")).collect())
        })
        .collect()
}
fn query(value: &Value, doc: &Document) {
    assert_eq!(value["items"]["total"], 3);
    assert_eq!(value["filters"]["total"], 0);
    assert_eq!(value["filterStacks"]["total"], 0);
    let rows = value["items"]["entries"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let mask = rows.iter().find(|v| v["hasMask"] == true).unwrap();
    let original = doc.xcf_state.as_ref().unwrap().items.iter().find_map(|i| i.mask.as_ref()).unwrap();
    assert_eq!(mask["mask"]["bounds"], serde_json::to_value(original.bounds).unwrap());
    assert_eq!(mask["mask"]["offset"], serde_json::to_value(original.offset).unwrap());
    assert_eq!(mask["mask"]["outsideFloat64Bits"], format!("0x{:016x}", original.outside_f64_bits));
    assert_eq!(mask["mask"]["enabled"], true);
    assert_eq!(mask["mask"]["shown"], false);
    assert_eq!(mask["mask"]["editing"], false);
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-analytic-geometry-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: 64 actual assigned ICC NL/Linear/Perceptual Union source/edit states use paid host/native routes and preserve original masks"]
fn assigned_geometry_host_source_protection_title_save_and_native_continuation() {
    for (root, row) in cases() {
        let original = std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        let dir = Directory::new();
        let source = dir.0.join("source.xcf");
        std::fs::write(&source, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&source, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        profiles(&before, &row);
        let identity = h.session.active().unwrap().source.clone();
        assert!(h.session.active().unwrap().source_read_only);
        before.accounted().unwrap().validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        let raw = native(&before);
        let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        query(reply.value(), &before);
        drop(reply);
        for (id, params) in
            [("select.all", json!({})), ("layer.setProps", json!({"name":"unsupported"})), ("xcf.filter.enable", json!({"filterId":"1","enabled":false}))]
        {
            assert!(h.command_run(id, params).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
        assert!(h.save_owned_native_active(Some(&source), Some("pcraft"), &ctl()).is_err());
        let target = dir.0.join("Geometry current.pcraft");
        h.save_owned_native_active(Some(&target), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_ne!(saved.name, before.name);
        assert_eq!(native(&saved), raw);
        assert_eq!(saved.xcf_state, before.xcf_state);
        assert_eq!(h.session.active().unwrap().source, identity);
        assert!(h.session.active().unwrap().source_read_only);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&target, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(!current.source_read_only);
        current.doc.accounted().unwrap().validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert_eq!(current.doc.name, saved.name);
        assert_eq!(current.doc.xcf_state, before.xcf_state);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(native(&current.doc), raw);
        let reply = reopened.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        query(reply.value(), &current.doc);
        drop(reply);
        assert!(reopened.command_run("select.all", json!({})).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap(), original);
    }
}

#[test]
#[ignore = "UNRUN: authentic assigned geometry host authorizer and preparation control refuse before publication"]
fn assigned_geometry_host_denial_keeps_session_and_source() {
    // One actual source from each retained NL-float/NL-integer/Linear/Perceptual family.
    for (root, row) in cases().into_iter().step_by(16) {
        let original = std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        let dir = Directory::new();
        let source = dir.0.join("source.xcf");
        std::fs::write(&source, &original).unwrap();
        let mut h = Headless::trusted_local();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(photocraft_raster::backing::BackingError::Work) };
        assert!(h.open_owned_xcf_keep_current(&source, &denied).is_err());
        assert!(h.session.active().is_none());
        drop(h.open_owned_xcf_keep_current(&source, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let raw = native(&before);
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
        let error = match h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(error) => error,
            Ok(_) => panic!("supported paid query must pass through the rejecting authorizer"),
        };
        assert!(error.to_string().contains("denied"), "actual supported-query authorizer refusal: {error}");
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(native(&before), raw);
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
}
