//! UNRUN owned-import/native-reopen canvas integration over pinned independent
//! source files. Equality here checks the current display adapter; it does not
//! establish broad source eligibility or new GIMP rendering acceptance.
use super::*;
use photocraft_engine::{Session, document_snapshot::DocumentSnapshot};
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
#[ignore = "UNRUN: requires pinned independently authored Threshold Alpha source evidence"]
fn actual_owned_sources_and_native_reopens_use_current_canvas_without_resident_fallback() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let name = format!("{id}.xcf");
        let bytes = Arc::new(std::fs::read(root.join(&name)).unwrap());
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let ordinary = photocraft_io::import(&name, &bytes).unwrap();
        let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(&ordinary.document).unwrap();
        let reference = renderer.render(ordinary.document.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let expected = display_image(None, &reference);
        let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let imported =
            photocraft_io::import_owned_staged_threshold_alpha_bytes_in_quota(&name, bytes, &Default::default(), &limits, quota.clone(), &control()).unwrap();
        let snapshot = DocumentSnapshot::from_accounted(imported.document().clone()).unwrap();
        // Source cache residency is independent of the derived preview owner.
        drop(backed_snapshot_display_image(&snapshot, 8, 6, None, &Interrupt::NONE).unwrap());
        let before = quota.usage().unwrap();
        let (image, lease) = backed_snapshot_display_image(&snapshot, 8, 6, None, &Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels, "{id}");
        assert!(quota.usage().unwrap().resident > before.resident);
        drop(image);
        drop(lease);
        assert_eq!(quota.usage().unwrap(), before);
        // A detached bare Document cannot borrow the runtime storage class.
        assert!(backed_display_image(&snapshot, 8, 6, None, &Interrupt::NONE).is_err());
        let saved =
            photocraft_format::owned_document::save_current_document_archive(&snapshot, Cursor::new(Vec::new()), Default::default(), &quota, &control())
                .unwrap()
                .into_inner();
        let _saved_fee = quota.reserve(QuotaKind::Resident, saved.capacity() as u64).unwrap();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            quota.clone(),
            &control(),
        )
        .unwrap();
        let mut session = Session::new();
        session.add_owned_native_threshold_alpha_archive(archive, "current.pcraft", Some("authorized/current.pcraft"), &control()).unwrap();
        let current = &session.active().unwrap().doc;
        assert!(current.accounted().unwrap().is_readonly_threshold_alpha_storage());
        let (reopened, fee) = backed_snapshot_display_image(current, 8, 6, None, &Interrupt::NONE).unwrap();
        assert_eq!(reopened.pixels, expected.pixels, "native {id}");
        drop(reopened);
        drop(fee);
    }
}
