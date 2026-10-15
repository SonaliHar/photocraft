//! UNRUN Point routing and real source/native ownership handoff drafts.
use super::*;
use photocraft_doc::{
    ColorMode, Document, LayerId, SampleType, Size, XcfState,
    item_identity::{FilterId, ItemRef},
    xcf_filters::{FilterNode, FilterStack, OperationIdentity},
};
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicU64, Ordering};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hint(name: &str) -> Document {
    let mut doc = Document::new("routing only", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    let mut state = XcfState::default();
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(LayerId(1)),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(2),
            operation: OperationIdentity { name: name.into(), stored_version: "wrong".into(), schema_epoch: "not-authority".into() },
            source_display_name: None,
            source_icon: None,
            arguments: Vec::new(),
            enabled: false,
            opacity_f64_bits: 0,
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: Vec::new(),
            composition: None,
        }],
    });
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn exact_point_names_hint_only_stricter_route_and_propagate_actual_control() {
    for name in [
        "gegl:brightness-contrast",
        "gegl:levels",
        "gegl:exposure",
        "gegl:color-temperature",
        "gegl:sepia",
        "gegl:alien-map",
        "gegl:invert-linear",
        "gegl:invert-gamma",
        "gegl:absolute",
        "gegl:color-exchange",
        "gegl:color-overlay",
        "gegl:channel-mixer",
        "gegl:mono-mixer",
        "gegl:red-eye-removal",
        "gegl:color-rotate",
        "gegl:video-degradation",
        "gegl:color-to-alpha",
        "gegl:component-extract",
        "gegl:alpha-clip",
        "gimp:desaturate",
        "gegl:saturation",
        "gegl:value-invert",
        "gimp:posterize",
        "gimp:color-balance",
        "gimp:levels",
        "gimp:curves",
        "gegl:rgb-clip",
        "gegl:unpremultiply",
    ] {
        let document = hint(name);
        let work = AtomicU64::new(0);
        let debit = |n| {
            work.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        assert_eq!(is_current_point_candidate(&document, &ReadControl { interrupt: &Interrupt::NONE, work: &debit }), Ok(true));
        assert_eq!(work.load(Ordering::Relaxed), 2);
        assert_eq!(
            is_current_point_candidate(&document, &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) }),
            Err(BackingError::Work)
        );
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert_eq!(is_current_point_candidate(&document, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(BackingError::Cancelled));
    }
    for name in ["GEGL:exposure", "gegl:exposure ", "gimp:brightness-contrast", "gimp:levels ", "gimp:threshold", "gegl:gamma", "gegl:rgb-clip ", "GEGL:unpremultiply"] {
        assert_eq!(is_current_point_candidate(&hint(name), &ctl()), Ok(false));
    }
    let empty = Document::new("empty", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert_eq!(is_current_point_candidate(&empty, &ReadControl { interrupt: &interrupt, work: &|_| Err(BackingError::Work) }), Ok(false));
}
#[test]
fn structured_point_errors_do_not_parse_text_or_erase_work_and_cancel() {
    use photocraft_compose::backed_normal::BackedNormalError as E;
    use photocraft_raster::checked_view::RasterViewError as V;
    for error in [E::Backing(BackingError::Cancelled), E::View(V::Control(BackingError::Cancelled)), E::View(V::Backed(BackingError::Cancelled))] {
        assert!(matches!(point_error(error, &ctl()), EngineError::Cancelled));
    }
    assert_eq!(point_error(E::Backing(BackingError::Work), &ctl()).to_string(), EngineError::from(BackingError::Work).to_string());
    assert_eq!(point_error(E::View(V::Descriptor(BackingError::Quota)), &ctl()).to_string(), EngineError::from(BackingError::Quota).to_string());
    assert_eq!(point_error(E::Allocation, &ctl()).to_string(), EngineError::from(BackingError::Allocation).to_string());
}

#[cfg(feature = "corpus")]
mod originals {
    use super::*;
    use photocraft_doc::LayerContent;
    use photocraft_format::{
        LoadOptions,
        canonical_archive::ImmutableCanonicalArchive,
        owned_document::{load_document_from_owned_archive, save_current_document_archive},
    };
    use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits};
    use sha2::{Digest, Sha256};
    use std::{io::Cursor, path::PathBuf, sync::Arc};
    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
    }
    struct Packet {
        imported: photocraft_io::OwnedPointXcfImport,
        _encoded: Arc<Vec<u8>>,
        q: BackingQuota,
        _capture: QuotaLease,
    }
    fn packet() -> Packet {
        let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN: pinned independently saved Point originals required"))
            .join("point-replace-geometry-evidence");
        let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(hash(&manifest), "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72");
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        let artifacts = manifest["artifacts"].as_array().unwrap();
        assert!(artifacts.len() <= 4096);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        for asset in artifacts {
            let path = asset["path"].as_str().unwrap();
            if !path.ends_with(".xcf") {
                continue;
            }
            assert!(!path.contains(['/', '\\']));
            let length = std::fs::metadata(root.join(path)).unwrap().len();
            assert!(length > 0 && length <= 16 << 20);
            let fee = q.reserve(QuotaKind::Resident, 2 * length + 65536).unwrap();
            let bytes = Arc::new(std::fs::read(root.join(path)).unwrap());
            assert!(bytes.capacity() as u64 <= 2 * length);
            assert_eq!(hash(&bytes), asset["sha256"].as_str().unwrap());
            let selected = photocraft_io::import_owned_xcf_bytes_in_quota(path, bytes.clone(), &Default::default(), &limits, q.clone(), &ctl());
            if let Ok(photocraft_io::OwnedXcfImport::Point(imported)) = selected {
                return Packet { imported, _encoded: bytes, q, _capture: fee };
            }
        }
        panic!("pinned manifest contains no admitted initial Point original");
    }
    #[derive(Debug)]
    struct Archive {
        bytes: Vec<u8>,
        q: BackingQuota,
        _fee: QuotaLease,
    }
    impl ImmutableCanonicalArchive for Archive {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
            if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
        }
        fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
            c.check(out.len() as u64)?;
            let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
            let end = start.checked_add(out.len()).ok_or(BackingError::Length)?;
            out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
            Ok(())
        }
    }
    fn archive(p: &Packet) -> Arc<Archive> {
        let bytes =
            save_current_document_archive(p.imported.document().document(), Cursor::new(Vec::new()), Default::default(), &p.q, &ctl()).unwrap().into_inner();
        let fee = p.q.reserve(QuotaKind::Resident, bytes.capacity() as u64 + 128).unwrap();
        Arc::new(Archive { bytes, q: p.q.clone(), _fee: fee })
    }
    fn load(a: Arc<Archive>, p: &Packet) -> OwnedDocumentArchive {
        load_document_from_owned_archive(a, &LoadOptions::default(), Default::default(), p.q.clone(), &ctl()).unwrap()
    }
    fn native(doc: &Document) -> Vec<u8> {
        let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
        let r = layer.operative_bounds();
        let bpp = layer.format().bytes_per_pixel();
        let mut bytes = vec![0; r.width() as usize * r.height() as usize * bpp];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * bpp;
                layer.checked_view().read_logical_pixel_native_into(x, y, &mut bytes[at..at + bpp], &ctl()).unwrap();
            }
        }
        bytes
    }
    #[test]
    #[ignore = "UNRUN: actual pinned Point import/native handoff custody"]
    fn typed_source_and_native_routes_preserve_current_model_words_and_distinct_runtime_identity() {
        let p = packet();
        let original = p.imported.document();
        let raw = native(original.document());
        let mut source = Session::new();
        source.add_owned_point_xcf_import(&p.imported, "selected.xcf", Some("authorized/selected.xcf"), &ctl()).unwrap();
        let state = source.active().unwrap();
        assert!(state.source_read_only);
        assert!(state.path.is_none());
        assert!(state.doc.accounted().unwrap().shares_model(original));
        assert!(!state.history.can_undo());
        let bytes = archive(&p);
        let weak = Arc::downgrade(&bytes);
        let loaded = load(bytes.clone(), &p);
        assert!(is_current_point_candidate(loaded.document(), &ctl()).unwrap());
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(loaded, "current.pcraft", Some("authorized/current.pcraft"), &ctl()).unwrap();
        let state = reopened.active().unwrap();
        assert!(!state.source_read_only);
        assert_eq!(state.path.as_deref(), Some("authorized/current.pcraft"));
        assert_eq!(state.doc.name, original.document().name);
        assert_eq!(state.doc.icc_profile, original.document().icc_profile);
        assert_eq!(state.doc.source_color_context, original.document().source_color_context);
        assert_eq!(state.doc.xcf_state, original.document().xcf_state);
        assert_eq!(native(&state.doc), raw);
        assert!(state.doc.accounted().unwrap().quota().shares_ledger(&p.q));
        assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        drop(bytes);
        assert!(weak.upgrade().is_some());
        drop(reopened);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    #[ignore = "UNRUN: strict Point generic-publication and duplicate fences"]
    fn structural_owner_and_known_native_candidate_cannot_bypass_strict_current_validation() {
        let p = packet();
        // The original owner retains its genuine native cache across Session
        // drops. Establish that same operative cache before the refund baseline.
        validate_current_stock_point_storage(p.imported.document(), &ctl()).unwrap();
        let mut session = Session::new();
        let baseline = p.q.usage().unwrap();
        assert!(session.add_accounted_document(p.imported.document().clone(), None, &ctl()).is_err());
        assert!(session.docs.is_empty());
        let bytes = archive(&p);
        let before = p.q.usage().unwrap();
        assert!(
            session
                .add_owned_native_archive(load(bytes.clone(), &p), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "not-a-legacy-fallback", None, &ctl())
                .is_err()
        );
        assert!(session.docs.is_empty());
        assert_eq!(p.q.usage().unwrap(), before);
        session.add_owned_point_xcf_import(&p.imported, "original", None, &ctl()).unwrap();
        let snapshot = session.active().unwrap().doc.clone();
        assert!(session.add_owned_point_xcf_import(&p.imported, "duplicate", None, &ctl()).is_err());
        assert!(session.active().unwrap().doc.shares_state(&snapshot));
        assert_eq!(session.docs.len(), 1);
        drop(snapshot);
        drop(session);
        drop(bytes);
        assert_eq!(p.q.usage().unwrap(), baseline);
    }
    #[test]
    #[ignore = "UNRUN: actual Point work/cancel/quota failures before publication"]
    fn failed_current_or_archive_preparation_is_private_and_refunds() {
        let p = packet();
        let bytes = archive(&p);
        let baseline = p.q.usage().unwrap();
        let mut session = Session::new();
        let deny = |_| Err(BackingError::Work);
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
        assert!(session.add_owned_point_xcf_import(&p.imported, "denied", None, &control).is_err());
        assert!(session.docs.is_empty());
        assert_eq!(p.q.usage().unwrap(), baseline);
        for at in [0u64, 1, 64, 1024, 65536] {
            let archive = load(bytes.clone(), &p);
            let units = AtomicU64::new(0);
            let cancel = || units.load(Ordering::Relaxed) >= at;
            let interrupt = Interrupt::cancel_only(&cancel);
            let debit = |n| {
                units.fetch_add(n, Ordering::Relaxed);
                Ok(())
            };
            let control = ReadControl { interrupt: &interrupt, work: &debit };
            assert!(session.add_owned_native_point_archive(archive, "cancelled", None, &control).is_err());
            assert!(session.docs.is_empty());
            assert_eq!(p.q.usage().unwrap(), baseline);
        }
        let used = p.q.usage().unwrap().index;
        let block = p.q.reserve(QuotaKind::Index, (32 << 20) - used).unwrap();
        assert!(session.add_owned_point_xcf_import(&p.imported, "full", None, &ctl()).is_err());
        assert!(session.docs.is_empty());
        drop(block);
        assert_eq!(p.q.usage().unwrap(), baseline);
    }
}
