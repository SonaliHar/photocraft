//! UNRUN integration draft: selected owned Point input, paid discovery, current
//! canvas and pathless native continuation. This compares adapters to the existing
//! current renderer; independent GIMP qualification remains a separate gate.
use super::*;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use sha2::{Digest, Sha256};
use std::{io::Cursor, sync::Arc};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}

#[test]
#[ignore = "UNRUN: pinned independent Point originals and complete UI/native continuation"]
fn selected_point_source_paid_inspection_canvas_and_native_reopen_share_current_interpretation() {
    let base = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    for (folder, pin, name) in [
        (
            "point-replace-geometry-evidence",
            "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72",
            "brightness-gray-u16_linear.negative-x.author.xcf",
        ),
        (
            "point-replace-geometry-evidence",
            "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72",
            "alien-hsl-rgb-double_non_linear.one-row.author.xcf",
        ),
        (
            "point-invert-gamma-evidence",
            "6c905ae85375807b57df97b47579461d5986c41c2e008498abd82e4ac56b5daf",
            "invert-gamma-gray-half_perceptual.small.author.xcf",
        ),
    ] {
        let root = base.join(folder);
        let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(hash(&manifest), pin);
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        let asset = manifest["artifacts"].as_array().unwrap().iter().find(|row| row["path"] == name).unwrap();
        let bytes = Arc::new(std::fs::read(root.join(name)).unwrap());
        assert_eq!(hash(&bytes), asset["sha256"].as_str().unwrap());
        let ordinary = photocraft_io::import(name, &bytes).unwrap();
        let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(&ordinary.document).unwrap();
        let reference = renderer.render(ordinary.document.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
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
            &control(),
        )
        .unwrap();
        assert!(matches!(&imported, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &control()).unwrap();
        let current = app.session.active().unwrap();
        assert!(current.source_read_only);
        assert!(current.path.is_none());
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        let node = &current.doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        let project = photocraft_engine::accounted_xcf_inspect::prepare(&app.session, &serde_json::Value::Null, &control()).unwrap();
        assert_eq!(project.command(), "xcf.inspect");
        let inspection = photocraft_engine::accounted_filter_inspect::prepare(&app.session, &serde_json::json!({"filterId":id}), &control()).unwrap();
        assert_eq!(inspection.command(), "xcf.filter.inspect");
        let width = current.doc.size.width;
        let height = current.doc.size.height;
        let (display, fee) = backed_snapshot_display_image(&current.doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels, "source {name}");
        drop((display, fee, inspection, project));
        assert!(backed_display_image(&current.doc, width, height, None, &Interrupt::NONE).is_err());
        // External harness capture, like the fs::read source above. This draft
        // does not assert accounting for Cursor<Vec> writer/provider buffers;
        // production writer custody has separate controlled-save coverage.
        let saved =
            photocraft_format::owned_document::save_current_document_archive(&current.doc, Cursor::new(Vec::new()), Default::default(), &quota, &control())
                .unwrap()
                .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            quota.clone(),
            &control(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "current.pcraft", archive, &control()).unwrap();
        let state = reopened.session.active().unwrap();
        assert!(state.path.is_none());
        assert!(!state.source_read_only);
        assert!(state.doc.accounted().unwrap().is_readonly_point_storage());
        let (actual, fee) = backed_snapshot_display_image(&state.doc, width, height, None, &Interrupt::NONE).unwrap();
        assert_eq!(actual.pixels, expected.pixels, "native {name}");
        drop((actual, fee));
        assert_eq!(hash(&bytes), asset["sha256"].as_str().unwrap());
    }
}
