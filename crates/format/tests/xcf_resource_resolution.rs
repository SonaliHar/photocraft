//! Current resource resolutions must survive native persistence, never grants.
mod common;
use photocraft_doc::{
    Color, ColorMode, Document, SampleType, Size,
    item_identity::ResourceId,
    xcf_resources::{ResourceFidelity, ResourceKind, ResourceReference, ResourceResolution},
    xcf_state::XcfState,
};
use photocraft_format::{SaveOptions, load_from_bytes, save_to_bytes, zip::ZipReader};
fn document(fidelity: ResourceFidelity) -> Document {
    let mut doc = Document::with_background("owned current resource", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    let current = vec![19, 3, 7, 255];
    let observed = blake3::hash(&current).to_hex().to_string();
    let requested = if fidelity == ResourceFidelity::Exact { observed.clone() } else { blake3::hash(b"original requested bytes").to_hex().to_string() };
    state.resources.push(ResourceReference {
        id: ResourceId(5),
        kind: ResourceKind::LinkedImage,
        name: "current embedded value".into(),
        collection: None,
        internal: false,
        locator: Some("inert/requested-image".into()),
        content_hash: Some(requested),
        owned_bytes: Some(current.into()),
        resolution: Some(ResourceResolution { fidelity, payload_hash: observed }),
    });
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn current_exact_and_substituted_payloads_require_reader_six_and_preserve_requested_identity() {
    for fidelity in [ResourceFidelity::Exact, ResourceFidelity::Substituted] {
        let doc = document(fidelity);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let zip = ZipReader::new(&bytes).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
        assert_eq!(wire["minimum_reader_version"], 6);
        assert_eq!(wire["format_version"], 6);
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    }
}
#[test]
fn inconsistent_resolution_is_refused_before_native_output() {
    for exact in [false, true] {
        let mut doc = document(if exact { ResourceFidelity::Exact } else { ResourceFidelity::Substituted });
        let resource = &mut doc.xcf_state.as_mut().unwrap().resources[0];
        if exact {
            resource.content_hash = Some("0".repeat(64));
        } else {
            resource.resolution.as_mut().unwrap().payload_hash = "0".repeat(64);
        }
        assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
    }
}

#[test]
fn forged_reader_five_cannot_hide_current_resolution_after_state_decode() {
    for fidelity in [ResourceFidelity::Exact, ResourceFidelity::Substituted] {
        let doc = document(fidelity);
        let path = common::temp_dir("resource-resolution-reader");
        photocraft_format::PcraftWriter::new().save_dir(&doc, &path, &SaveOptions::default()).unwrap();
        let file = path.join("manifest.json");
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        value["minimum_reader_version"] = 5.into();
        std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
        let result = photocraft_format::load_path(&path);
        std::fs::remove_dir_all(path).unwrap();
        assert!(result.is_err(), "lower advertised floor must not erase operative resolution");
    }
}
#[test]
fn accounted_current_resource_save_uses_same_selective_floor_and_exact_bytes() {
    use photocraft_format::owned_document::{OwnedDocumentLimits, save_current_document_archive};
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    };
    for fidelity in [ResourceFidelity::Exact, ResourceFidelity::Substituted] {
        let doc = document(fidelity);
        let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 64 << 20, index: 16 << 20 });
        let bytes = save_current_document_archive(
            &doc,
            std::io::Cursor::new(Vec::new()),
            OwnedDocumentLimits::default(),
            &quota,
            &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Ok(()) },
        )
        .unwrap()
        .into_inner();
        let wire = photocraft_format::read_manifest(&bytes).unwrap();
        assert_eq!(wire.minimum_reader_version, 6);
        #[derive(Debug)]
        struct Archive(Vec<u8>, BackingQuota);
        impl photocraft_format::canonical_archive::ImmutableCanonicalArchive for Archive {
            fn len(&self) -> u64 {
                self.0.len() as u64
            }
            fn validate_quota(&self, quota: &BackingQuota) -> Result<(), photocraft_raster::backing::BackingError> {
                if self.1.shares_ledger(quota) { Ok(()) } else { Err(photocraft_raster::backing::BackingError::Quota) }
            }
            fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), photocraft_raster::backing::BackingError> {
                use photocraft_raster::backing::BackingError;
                ctl.check(out.len() as u64)?;
                let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
                let end = at.checked_add(out.len()).ok_or(BackingError::Bounds)?;
                out.copy_from_slice(self.0.get(at..end).ok_or(BackingError::Io)?);
                Ok(())
            }
        }
        // Test capability payload is retained/charged before the actual owned loader.
        let input = quota.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, bytes.capacity() as u64).unwrap();
        let source = std::sync::Arc::new(Archive(bytes, quota.clone()));
        let loaded = photocraft_format::owned_document::load_document_from_owned_archive(
            source.clone(),
            &photocraft_format::LoadOptions::default(),
            OwnedDocumentLimits::default(),
            quota.clone(),
            &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Ok(()) },
        )
        .unwrap();
        assert_eq!(loaded.document(), &doc);
        drop(loaded);
        drop(source);
        drop(input);
        assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    }
}
