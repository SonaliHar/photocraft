//! UNRUN ignored integration drafts over the pinned independently authored XCF
//! precision packet. No synthetic metadata is substituted for source admission.
use super::*;
use photocraft_doc::{ColorMode, LayerContent};
use photocraft_format::{
    LoadOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{load_document_from_owned_archive, save_current_document_archive},
};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
};
use photocraft_xcf::{BudgetKind, DecodeOptions, Encoding, Model, ResourceLimits, Sample};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Fixture {
    imported: photocraft_io::OwnedThresholdAlphaXcfImport,
    bytes: Arc<Vec<u8>>,
    quota: BackingQuota,
    name: String,
    _capture: QuotaLease,
}
fn fixture() -> Fixture {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires pinned full-XCF evidence corpus"))
        .join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits =
        ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)]).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        assert!(name.len() <= 4096 && !name.contains(['/', '\\']));
        let path = root.join(&name);
        let length = std::fs::metadata(&path).unwrap().len();
        assert!(length > 0 && length <= 16 << 20);
        let capture = quota.reserve(QuotaKind::Resident, length.checked_mul(2).unwrap() + 4096).unwrap();
        let bytes = Arc::new(std::fs::read(path).unwrap());
        assert!(bytes.capacity() as u64 <= 2 * length);
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let raw = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
        // Choose the independently authored RGB/F64 nonlinear state whose exact
        // source class is admitted by the existing qualified importer.
        if raw.model != Model::Rgb
            || raw.sample != Sample::F64
            || raw.encoding != Encoding::NonlinearProfile
            || raw.precision != 750
            || !raw
                .layers
                .first()
                .and_then(|layer| layer.effects.first())
                .is_some_and(|effect| effect.operation == "gimp:threshold-alpha" && effect.state.enabled == Some(true))
        {
            continue;
        }
        let imported =
            photocraft_io::import_owned_staged_threshold_alpha_bytes_in_quota(&name, bytes.clone(), &DecodeOptions::default(), &limits, quota.clone(), &ctl())
                .unwrap();
        assert!(imported.document().quota().shares_ledger(&quota));
        assert_eq!(limits.get(BudgetKind::Scratch), 0);
        return Fixture { imported, bytes, quota, name, _capture: capture };
    }
    panic!("pinned packet has no supported RGB/F64 nonlinear Threshold Alpha state")
}
fn pixel_words(owner: &photocraft_doc::accounted_document::AccountedDocument) -> Vec<u8> {
    let doc = owner.document();
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("not native backed") };
    let bpp = layer.format().bytes_per_pixel();
    let mut words = vec![0; 48 * bpp];
    for y in 0..6 {
        for x in 0..8 {
            let at = (y * 8 + x) * bpp;
            layer.checked_view().read_logical_pixel_native_into(x as i32, y as i32, &mut words[at..at + bpp], &ctl()).unwrap();
        }
    }
    words
}
fn canvas(owner: &photocraft_doc::accounted_document::AccountedDocument) -> Vec<[u64; 4]> {
    let profile = match owner.document().mode {
        ColorMode::Rgb => photocraft_cms::Builtin::Srgb.profile(),
        ColorMode::Grayscale => photocraft_cms::Builtin::SGray.profile(),
        _ => panic!(),
    };
    let prepared = photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(owner, profile, Default::default(), &ctl()).unwrap();
    prepared.render_band(owner.document().bounds(), &ctl()).unwrap().pixels().iter().map(|p| p.map(f64::to_bits)).collect()
}

#[test]
#[ignore = "UNRUN: requires independently saved pinned Threshold Alpha precision evidence"]
fn actual_retained_import_session_source_identity_quota_and_canvas_match_resident_consumer() {
    let f = fixture();
    let source_pointer = Arc::downgrade(&f.bytes);
    let raw_before = pixel_words(f.imported.document());
    let mut session = Session::new();
    session.add_owned_threshold_alpha_xcf_import(&f.imported, &f.name, Some("authorized/original.xcf"), &ctl()).unwrap();
    let state = session.active().unwrap();
    let owner = state.doc.accounted().unwrap();
    assert!(owner.is_readonly_threshold_alpha_storage());
    assert!(owner.quota().shares_ledger(&f.quota));
    assert!(state.source_read_only && state.path.is_none());
    assert_eq!(state.source.as_ref().unwrap().label, f.name);
    assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("authorized/original.xcf"));
    assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::xcf());
    assert!(state.source.as_ref().unwrap().check_destination("overwritten.xcf").is_err());
    assert!(!state.history.can_undo());
    let ordinary = photocraft_io::import(&f.name, &f.bytes).unwrap();
    let reference = photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&ordinary.document, &Interrupt::NONE)
        .unwrap()
        .render(ordinary.document.bounds(), &mut |_| Ok(()))
        .unwrap();
    assert_eq!(canvas(owner), reference.px.iter().map(|p| p.map(f64::to_bits)).collect::<Vec<_>>());
    assert_eq!(pixel_words(owner), raw_before);
    let q = f.quota.clone();
    drop(ordinary);
    drop(f);
    assert!(source_pointer.upgrade().is_some());
    drop(session);
    assert!(source_pointer.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[derive(Debug)]
struct Archive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
#[test]
#[ignore = "UNRUN: requires independently saved pinned Threshold Alpha precision evidence"]
fn actual_import_native_save_as_reload_current_class_raw_words_and_canvas_are_retained() {
    let f = fixture();
    let mut session = Session::new();
    session.add_owned_threshold_alpha_xcf_import(&f.imported, &f.name, Some("authorized/original.xcf"), &ctl()).unwrap();
    let source = session.active().unwrap().source.clone();
    let original = session.active().unwrap().doc.clone();
    let expected = pixel_words(original.accounted().unwrap());
    let display = canvas(original.accounted().unwrap());
    let prepared = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&session, "current.pcraft", &ctl()).unwrap();
    let bytes = prepared
        .write_and_publish(&mut session, &ctl(), |snapshot, _| {
            save_current_document_archive(snapshot, std::io::Cursor::new(Vec::new()), Default::default(), &f.quota, &ctl())
                .map(|w| w.into_inner())
                .map_err(|e| EngineError::Other(e.to_string()))
        })
        .unwrap();
    assert_eq!(session.active().unwrap().source, source);
    assert!(session.active().unwrap().source_read_only);
    assert_eq!(session.active().unwrap().path.as_deref(), Some("current.pcraft"));
    assert!(session.active().unwrap().doc.accounted().unwrap().is_readonly_threshold_alpha_storage());
    let fee = f.quota.reserve(QuotaKind::Resident, bytes.capacity() as u64 + 128).unwrap();
    let archive = Arc::new(Archive { bytes, quota: f.quota.clone(), _lease: fee });
    let loaded = load_document_from_owned_archive(archive, &LoadOptions::default(), Default::default(), f.quota.clone(), &ctl()).unwrap();
    let mut reopened = Session::new();
    reopened.add_owned_native_threshold_alpha_archive(loaded, "current.pcraft", Some("authorized/current.pcraft"), &ctl()).unwrap();
    let state = reopened.active().unwrap();
    assert!(state.doc.accounted().unwrap().is_readonly_threshold_alpha_storage());
    assert!(state.doc.quota().unwrap().shares_ledger(&f.quota));
    assert_eq!(pixel_words(state.doc.accounted().unwrap()), expected);
    assert_eq!(canvas(state.doc.accounted().unwrap()), display);
    assert_eq!(state.doc.source_color_context, original.source_color_context);
    assert_eq!(
        state.doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments,
        original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments
    );
    assert!(!state.source_read_only);
    assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
}

#[test]
#[ignore = "UNRUN: requires independently saved pinned Threshold Alpha precision evidence"]
fn duplicate_cancel_and_ineligible_label_leave_no_new_tab_or_fee_leak() {
    let f = fixture();
    // Strict current preparation legitimately fills the retained native cache
    // before late label/duplicate refusal. Warm that actual paid cache first;
    // the equality below concerns leaked temporary/publication fees, not cache IO.
    drop(canvas(f.imported.document()));
    let baseline = f.quota.usage().unwrap();
    for label in ["bad\0label".to_string(), "x".repeat(4097)] {
        let mut s = Session::new();
        assert!(s.add_owned_threshold_alpha_xcf_import(&f.imported, &label, None, &ctl()).is_err());
        assert!(s.docs.is_empty());
        assert_eq!(f.quota.usage().unwrap(), baseline);
    }
    for bound in [0, 1, 128, 4096] {
        let work = AtomicU64::new(0);
        let charge = |n| {
            if work.fetch_add(n, Ordering::Relaxed).saturating_add(n) > bound { Err(BackingError::Cancelled) } else { Ok(()) }
        };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
        let mut s = Session::new();
        assert!(s.add_owned_threshold_alpha_xcf_import(&f.imported, &f.name, None, &c).is_err());
        assert!(s.docs.is_empty());
        assert_eq!(f.quota.usage().unwrap(), baseline);
    }
    let mut s = Session::new();
    s.add_owned_threshold_alpha_xcf_import(&f.imported, &f.name, None, &ctl()).unwrap();
    let snapshot = s.active().unwrap().doc.clone();
    let held = f.quota.usage().unwrap();
    assert!(s.add_owned_threshold_alpha_xcf_import(&f.imported, &f.name, None, &ctl()).is_err());
    assert_eq!(s.docs.len(), 1);
    assert!(s.active().unwrap().doc.shares_state(&snapshot));
    assert_eq!(f.quota.usage().unwrap(), held);
}
