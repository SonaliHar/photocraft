//! UNRUN: exact assigned-profile NL/Linear/Perceptual four-operator originals through installed UI routes.
//! Resident output is an adapter reference, not new GIMP evidence. Harness
//! source/archive Vecs are external; production readers retain actual fees.
use super::*;
use photocraft_raster::{
    backing::quota::{BackingQuota, QuotaLimits},
    checked_view::NativeTileBytes,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};

pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) fn cases() -> Vec<(PathBuf, Value)> {
    let corpus = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut result = Vec::new();
    for (set, pins) in [
        ("holdout", include_str!("../../compose/tests/gimp_normal_mask_geometry_hashes.json")),
        ("integer", include_str!("../../compose/tests/gimp_normal_mask_geometry_integer_hashes.json")),
        ("linear", include_str!("../../compose/tests/gimp_normal_mask_geometry_linear_hashes.json")),
        ("perceptual", include_str!("../../compose/tests/gimp_normal_mask_geometry_perceptual_hashes.json")),
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
            let faithful: Value = serde_json::from_slice(&std::fs::read(root.join("faithful-observations.json")).unwrap()).unwrap();
            for row in data["records"].as_array().unwrap() {
                assert!((1..=4).contains(&row["layers"][0]["spaces"]["composite_mode"].as_i64().unwrap()));
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
                let witness = faithful["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
                assert_eq!(witness["source_sha256"], row["source_sha256"]);
                let reference = &witness["reference"];
                let png = std::fs::read(root.join(reference["file"].as_str().unwrap())).unwrap();
                assert_eq!(hash(&png), reference["sha256"].as_str().unwrap());
                assert_eq!(png.len() as u64, reference["bytes"].as_u64().unwrap());
                let mut row = row.clone();
                row["faithful_reference"] = reference.clone();
                result.push((root.clone(), row));
            }
        }
    }
    assert_eq!(result.len(), 256);
    result
}
// Observation ICC values describe GIMP source/profile roles. The canonical
// current ICC for Linear/Perceptual is the existing CMS derivative of ORIGINAL;
// parsing its rounded serialized curves as original would change the recipe.
pub(super) fn profiles(doc: &Document, row: &Value) {
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
pub(super) fn operators(doc: &Document, row: &Value) {
    let explicit: Vec<_> =
        doc.xcf_state.as_ref().unwrap().items.iter().filter_map(|item| item.composition.as_ref()).filter(|policy| policy.stored_operator != -1).collect();
    assert_eq!(explicit.len(), 1);
    let policy = explicit[0];
    assert_eq!((policy.source_mode, policy.stored_blend_space, policy.stored_composite_space), (28, 0, 1));
    let observed = row["layers"][0]["spaces"]["composite_mode"].as_i64().unwrap() as i32;
    assert_eq!(policy.stored_operator, observed);
    let expected = match observed {
        1 => photocraft_doc::import_compositing::GimpOperator::Union,
        2 => photocraft_doc::import_compositing::GimpOperator::ClipToBackdrop,
        3 => photocraft_doc::import_compositing::GimpOperator::ClipToLayer,
        4 => photocraft_doc::import_compositing::GimpOperator::Intersection,
        _ => panic!("closed independently captured operator"),
    };
    assert_eq!(policy.resolved.as_ref().unwrap().operator, expected);
}
pub(super) fn native(doc: &Document) -> Vec<(photocraft_raster::backing::NativePlaneDescriptor, String)> {
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
// Only alpha has an exact zero claim here. GIMP's hidden RGB at alpha zero is
// independently retained and is not equated to the current compositor's RGB.
fn captured_zero_alpha_controls(root: &std::path::Path, row: &Value, reference: &photocraft_compose::Buffer) -> usize {
    let png = photocraft_codecs::decode(&std::fs::read(root.join(row["faithful_reference"]["file"].as_str().unwrap())).unwrap()).unwrap();
    assert_eq!(hash(png.icc.as_deref().unwrap()), row["assigned_icc"]["sha256"].as_str().unwrap());
    let foreground = &row["layers"][0];
    let op = foreground["spaces"]["composite_mode"].as_i64().unwrap();
    let alpha_channel = if row["model"] == "gray" { 1 } else { 3 };
    let x0 = foreground["offset"][0].as_i64().unwrap();
    let y0 = foreground["offset"][1].as_i64().unwrap();
    let width = foreground["size"][0].as_u64().unwrap() as usize;
    let height = foreground["size"][1].as_u64().unwrap() as usize;
    let mask_blob = &foreground["mask"]["native"];
    let mask = std::fs::read(root.join(mask_blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&mask), mask_blob["sha256"].as_str().unwrap());
    assert_eq!(mask.len() as u64, mask_blob["bytes"].as_u64().unwrap());
    let stride = mask.len() / (width * height);
    assert!(matches!(stride, 2 | 4 | 8));
    let canvas_width = row["canvas"][0].as_u64().unwrap() as usize;
    let canvas_height = row["canvas"][1].as_u64().unwrap() as usize;
    let mut exact = 0;
    for y in 0..canvas_height {
        for x in 0..canvas_width {
            let alpha = f64::from(png.get(x as u32, y as u32, alpha_channel));
            let actual = f64::from(reference.px[y * canvas_width + x][3]);
            assert!((actual - alpha).abs() <= 1.0 / 65535.0, "independent captured alpha {}", row["id"]);
            let mx = x as i64 - x0;
            let my = y as i64 - y0;
            let inside = mx >= 0 && my >= 0 && mx < width as i64 && my < height as i64;
            let mask_zero = inside && {
                let at = (my as usize * width + mx as usize) * stride;
                mask[at..at + stride].iter().all(|byte| *byte == 0)
            };
            if matches!(op, 3 | 4) && (!inside || mask_zero) {
                assert_eq!(actual.to_bits(), 0.0f64.to_bits());
                assert_eq!(alpha.to_bits(), 0.0f64.to_bits());
                exact += 1;
            }
        }
    }
    exact
}

#[test]
#[ignore = "UNRUN: 256 pinned NL/Linear/Perceptual four-operator assigned-ICC geometry source/edit states, UI canvas and pathless native continuation"]
fn assigned_geometry_ui_retains_native_masks_profiles_and_paid_title_copy() {
    let mut exact_zero_controls = 0;
    for (root, row) in cases() {
        let name = row["source_file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let resident = photocraft_io::import(name, &bytes).unwrap().document;
        let reference = photocraft_compose::gimp_render::PreparedRenderer::new(&resident)
            .unwrap()
            .render(resident.bounds(), &mut |_| Ok(()))
            .unwrap()
            .try_display()
            .unwrap();
        exact_zero_controls += captured_zero_alpha_controls(&root, &row, &reference);
        let expected = display_image(None, &reference);
        let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
            name,
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(matches!(&imported, photocraft_io::OwnedXcfImport::AnalyticGeometry(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        profiles(&before, &row);
        operators(&before, &row);
        before.accounted().unwrap().validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert!(app.session.active().unwrap().source_read_only);
        assert!(app.session.active().unwrap().path.is_none());
        let query = photocraft_engine::accounted_plain_xcf_inspect::prepare(&app.session, &Value::Null, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 3);
        assert_eq!(query.value()["filters"]["total"], 0);
        assert_eq!(query.value()["filterStacks"]["total"], 0);
        assert_eq!(query.value()["items"]["entries"].as_array().unwrap().iter().filter(|v| v["hasMask"] == true).count(), 1);
        drop(query);
        let raw = native(&before);
        let width = before.size.width;
        let height = before.size.height;
        let (display, fee) = backed_snapshot_display_image(&before, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels);
        drop((display, fee));
        for (id, params) in
            [("select.all", json!({})), ("layer.setProps", json!({"blend":"multiply"})), ("xcf.filter.enable", json!({"filterId":"1","enabled":false}))]
        {
            assert!(app.run(id, params).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
        }
        let renamed = before.accounted().unwrap().try_with_document_name("Assigned geometry saved", &ctl()).unwrap();
        renamed.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert_eq!(native(renamed.document()), raw);
        // In-memory writer output is external harness capture. The subsequent
        // immutable native archive has its actual same-ledger encoded owner.
        let saved =
            photocraft_format::owned_document::save_current_document_archive(renamed.document(), Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
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
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "assigned.pcraft", archive, &ctl()).unwrap();
        let current = reopened.session.active().unwrap();
        assert!(current.path.is_none());
        assert!(!current.source_read_only);
        current.doc.accounted().unwrap().validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert_eq!(current.doc.name, "Assigned geometry saved");
        assert_eq!(current.doc.xcf_state, before.xcf_state);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(native(&current.doc), raw);
        let (display, fee) = backed_snapshot_display_image(&current.doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels);
        drop((display, fee));
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
    }
    assert!(exact_zero_controls > 0);
}

#[test]
#[ignore = "UNRUN: synthetic CURRENT opacity-zero native controls across four operators and stored interpretations; not authored GIMP evidence"]
fn current_zero_opacity_uses_strict_native_adoption_without_relabeling_source() {
    // Four operators from each NL-float/NL-integer/Linear/Perceptual family.
    for (index, (root, row)) in cases().into_iter().enumerate().filter(|(index, _)| index % 64 < 8 && index % 2 == 0) {
        let name = row["source_file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
            name,
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        let photocraft_io::OwnedXcfImport::AnalyticGeometry(source) = &imported else { panic!("genuine source owner") };
        let before = source.document();
        let raw = native(before.document());
        // This Document and resident reference clone are external harness
        // controls. The original paid owner remains alive through native load.
        let mut zero = before.document().clone();
        zero.layers.iter_mut().find(|layer| layer.mask.is_some()).unwrap().opacity = 0.0;
        let mut resident = photocraft_io::import(name, &bytes).unwrap().document;
        resident.layers.iter_mut().find(|layer| layer.mask.is_some()).unwrap().opacity = 0.0;
        let reference = photocraft_compose::gimp_render::PreparedRenderer::new(&resident)
            .unwrap()
            .render(resident.bounds(), &mut |_| Ok(()))
            .unwrap()
            .try_display()
            .unwrap();
        let expected = display_image(None, &reference);
        let saved = photocraft_format::owned_document::save_current_document_archive(&zero, Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
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
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_native_open::adopt_bytes(&mut app, "zero-current.pcraft", archive, &ctl()).unwrap();
        let current = app.session.active().unwrap();
        current.doc.accounted().unwrap().validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert!(current.path.is_none());
        assert!(!current.source_read_only);
        assert_eq!(current.doc.xcf_state, before.document().xcf_state);
        assert_eq!(current.doc.icc_profile, before.document().icc_profile);
        assert_eq!(current.doc.source_color_context, before.document().source_color_context);
        assert_eq!(native(&current.doc), raw);
        assert_eq!(current.doc.layers.iter().find(|layer| layer.mask.is_some()).unwrap().opacity.to_bits(), 0f32.to_bits());
        let (display, fee) = backed_snapshot_display_image(&current.doc, current.doc.size.width, current.doc.size.height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels, "synthetic CURRENT control {index}");
        drop((display, fee));
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
        assert_eq!(native(before.document()), raw);
        assert_ne!(before.document().layers.iter().find(|layer| layer.mask.is_some()).unwrap().opacity.to_bits(), 0f32.to_bits());
    }
}

#[test]
#[ignore = "UNRUN: assigned-ICC canvas cancellation and quota failures preserve actual owner/native/profile"]
fn assigned_geometry_ui_failure_paths_keep_original_owner() {
    // One actual source from each retained NL-float/NL-integer/Linear/Perceptual family.
    for (root, row) in cases().into_iter().step_by(64) {
        let name = row["source_file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let quota = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
            name,
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let raw = native(&before);
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(backed_snapshot_display_image(&before, before.size.width, before.size.height, None, &cancelled).is_err());
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(super::analytic_profile::prepare(before.accounted().unwrap(), &denied).is_err());
        let used = quota.usage().unwrap().resident;
        let held = quota.reserve(QuotaKind::Resident, (128 << 20) - used).unwrap();
        assert!(backed_snapshot_display_image(&before, before.size.width, before.size.height, None, &Interrupt::NONE).is_err());
        drop(held);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(native(&before), raw);
        assert_eq!(app.session.active().unwrap().doc.icc_profile, before.icc_profile);
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
    }
}
