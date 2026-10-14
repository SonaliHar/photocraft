//! UNRUN closed-current-storage fixtures; no IO/source qualification claim.
use super::*;
use photocraft_doc::{BackedLayer, Layer, PixelFormat, SourceColorContext, XcfState, import_compositing::*, xcf_state::*};
use photocraft_geom::Size;
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, quota::*},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    reads: Arc<AtomicU64>,
    _lease: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(at..at + out.len()).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(mode: ColorMode, implicit: bool, source_mode: i32, composite: GimpSpace) -> (Document, BackingQuota, Arc<AtomicU64>, &'static Profile) {
    fixture_depth(mode, implicit, source_mode, composite, SampleType::U8)
}
fn fixture_depth(
    mode: ColorMode,
    implicit: bool,
    source_mode: i32,
    composite: GimpSpace,
    depth: SampleType,
) -> (Document, BackingQuota, Arc<AtomicU64>, &'static Profile) {
    let profile = if mode == ColorMode::Rgb { photocraft_cms::Builtin::Srgb.profile() } else { photocraft_cms::Builtin::SGray.profile() };
    let mut doc = Document::new("paid plain current", Size::new(2, 1), mode, depth);
    doc.icc_profile = Some(profile.to_bytes());
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let precision = match depth {
        SampleType::U8 => 150,
        SampleType::U16 => 250,
        SampleType::U32 => 350,
        SampleType::F32 => 650,
        SampleType::F64 => 750,
    };
    doc.source_color_context = Some(SourceColorContext { original_icc: if implicit { None } else { doc.icc_profile.clone() }, source_precision: precision });
    if implicit {
        doc.working_profile_interpretation = if mode == ColorMode::Rgb {
            WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
        } else {
            WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
        };
    }
    let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 64 << 20 });
    let reads = Arc::new(AtomicU64::new(0));
    let format = PixelFormat::new(mode, depth, true);
    let values: &[f64] = if mode == ColorMode::Rgb { &[0.25, 0.5, 0.75, 1., 0.75, 0.25, 0.5, 0.5] } else { &[0.25, 1., 0.75, 0.5] };
    let mut raw = Vec::new();
    for &v in values {
        match depth {
            SampleType::U8 => raw.push((v * 255.).round() as u8),
            SampleType::U16 => raw.extend_from_slice(&((v * 65535.).round() as u16).to_ne_bytes()),
            SampleType::U32 => raw.extend_from_slice(&((v * f64::from(u32::MAX)).round() as u32).to_ne_bytes()),
            SampleType::F32 => raw.extend_from_slice(&(v as f32).to_ne_bytes()),
            SampleType::F64 => raw.extend_from_slice(&v.to_ne_bytes()),
        }
    }
    let descriptor = NativePlaneDescriptor::new(format, doc.bounds(), &vec![0; format.bytes_per_pixel()]).unwrap();
    let fee = quota.reserve(QuotaKind::Resident, raw.capacity() as u64 + 256).unwrap();
    let store = Arc::new(Store { descriptor, bytes: raw, reads: reads.clone(), _lease: fee });
    let raster = BackedRaster::new(store, quota.clone(), 1, &ctl()).unwrap();
    let mut layer = Layer::new("native", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
    layer.import_compositing =
        Some(ImportCompositing { blend_encoding: encoding(composite).unwrap(), composite_encoding: encoding(composite).unwrap(), pass_through: false });
    let mut state = XcfState::default();
    state.items.push(ItemState {
        item: ItemRef::Layer(layer.id),
        name: layer.name.clone(),
        custom_name: false,
        visible: layer.visible,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: "xcf-12-normal".into(),
            source_mode,
            legacy: source_mode == 0,
            stored_blend_space: if composite == GimpSpace::LinearRgb { 1 } else { 2 },
            stored_composite_space: if composite == GimpSpace::LinearRgb { 1 } else { 2 },
            stored_operator: 1,
            resolved: Some(ResolvedGimpComposition { blend_space: composite, composite_space: composite, operator: GimpOperator::Union }),
            pass_through: false,
        }),
    });
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    (doc, quota, reads, profile)
}
fn owner(doc: Document, quota: BackingQuota) -> AccountedDocument {
    // Fixture's actual metadata/ICC/model fits this real held reservation.
    let fee = quota.reserve(QuotaKind::Index, 1 << 20).unwrap();
    AccountedDocument::from_owned(doc, Arc::new(fee), quota, &ctl()).unwrap()
}
#[test]
fn genuine_paid_rgb_gray_explicit_and_implicit_normal_policies_render_without_mutation() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for implicit in [false, true] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
                for (source_mode, space) in [(0, GimpSpace::ProfileRgb), (28, GimpSpace::LinearRgb)] {
                    let (doc, q, reads, p) = fixture_depth(mode, implicit, source_mode, space, depth);
                    let expected = doc.xcf_state.clone();
                    let expected_source = doc.source_color_context.clone();
                    let owner = owner(doc, q);
                    assert!(PreparedBackedDocument::new(owner.document(), p, NormalBandLimits::default(), &ctl()).is_err());
                    let prepared = PreparedBackedDocument::new_accounted(&owner, p, NormalBandLimits::default(), &ctl()).unwrap();
                    assert_eq!(reads.load(Ordering::Relaxed), 0);
                    let band = prepared.render_band(Rect::new(0, 0, 2, 1), &ctl()).unwrap();
                    assert_eq!(band.pixels().len(), 2);
                    assert!(band.pixels().iter().flatten().all(|v| v.is_finite()));
                    assert!(reads.load(Ordering::Relaxed) > 0);
                    assert_eq!(owner.document().xcf_state, expected);
                    assert_eq!(owner.document().source_color_context, expected_source);
                }
            }
        }
    }
}
#[test]
fn mismatched_current_profile_marker_tuple_and_hidden_semantics_refuse_before_tiles() {
    for issue in 0..5 {
        let (mut doc, q, reads, p) = fixture(ColorMode::Rgb, true, 28, GimpSpace::LinearRgb);
        match issue {
            0 => doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            1 => {
                let mut bytes = doc.icc_profile.as_ref().unwrap().as_ref().clone();
                bytes[31] ^= 1;
                doc.icc_profile = Some(Arc::new(bytes));
            }
            2 => doc.layers[0].import_compositing.as_mut().unwrap().composite_encoding = SampleEncoding::NonlinearProfile,
            3 => {
                doc.layers[0].visible = false;
                doc.xcf_state.as_mut().unwrap().items[0].visible = false;
                doc.layers[0].effects.psd_raw = Some(Arc::new(vec![1]));
            }
            _ => doc.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().legacy = true,
        }
        let owner = owner(doc, q);
        assert!(PreparedBackedDocument::new_accounted(&owner, p, NormalBandLimits::default(), &ctl()).is_err());
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}
#[test]
fn cancellation_precedes_native_read_and_mixed_composite_spaces_are_closed() {
    let (mut doc, q, reads, p) = fixture(ColorMode::Rgb, false, 28, GimpSpace::LinearRgb);
    let mut second = doc.layers[0].clone();
    second.id = photocraft_doc::LayerId(second.id.0 + 1);
    let mut item = doc.xcf_state.as_ref().unwrap().items[0].clone();
    item.item = ItemRef::Layer(second.id);
    item.composition.as_mut().unwrap().stored_blend_space = 2;
    item.composition.as_mut().unwrap().stored_composite_space = 2;
    let resolved = item.composition.as_mut().unwrap().resolved.as_mut().unwrap();
    resolved.blend_space = GimpSpace::ProfileRgb;
    resolved.composite_space = GimpSpace::ProfileRgb;
    second.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::NonlinearProfile, composite_encoding: SampleEncoding::NonlinearProfile, pass_through: false });
    doc.layers.push(second);
    doc.xcf_state.as_mut().unwrap().items.push(item);
    let owner = owner(doc, q);
    assert!(PreparedBackedDocument::new_accounted(&owner, p, NormalBandLimits::default(), &ctl()).is_err());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(
        PreparedBackedDocument::new_accounted(&owner, p, NormalBandLimits::default(), &control),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[test]
fn oversized_owned_opaque_collection_refuses_before_traversal_or_tile_reads() {
    // from_owned validates backing, not canonical collection ceilings. Keep the
    // actual fixture's empty strings/shared payload and Vec under its paid guard.
    let (mut doc, q, reads, p) = fixture(ColorMode::Rgb, false, 28, GimpSpace::LinearRgb);
    let raw = photocraft_doc::xcf_provenance::OpaqueRecord {
        owner: None,
        namespace: String::new(),
        identity: String::new(),
        source_version: 12,
        order: 0,
        flags: 0,
        bytes: Arc::new(Vec::new()),
        blake3: String::new(),
        rendering_required: false,
    };
    doc.xcf_state.as_mut().unwrap().opaque = vec![raw; 4097];
    let owner = owner(doc, q);
    let calls = AtomicU64::new(0);
    let charge = |_| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    assert!(PreparedBackedDocument::new_accounted(&owner, p, NormalBandLimits::default(), &control).is_err());
    assert!(calls.load(Ordering::Relaxed) < 16);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().opaque.len(), 4097);
}
