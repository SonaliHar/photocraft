//! UNRUN synthetic structural custody only. The bounded ICC bytes below are
//! deliberately not an analytic-profile/source/evaluator qualification witness.
use super::*;
use crate::{BackedLayer, Layer, LayerMask, Size, import_compositing::*, item_identity::MaskId};
use crate::{ColorMode, PixelFormat, SampleEncoding, SampleType, xcf_state::*};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        out.fill(0);
        Ok(())
    }
}
fn fixture(sample: SampleType) -> AccountedDocument {
    fixture_encoding(sample, SampleEncoding::NonlinearProfile)
}
fn fixture_encoding(sample: SampleType, encoding: SampleEncoding) -> AccountedDocument {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 0 });
    // Genuine prospective fee before fixture DTO/maps/profile allocation. All
    // native stores also retain their actual backing allocations on this q.
    let fee = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let native_fee = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut doc = Document::new("masked.pcraft", Size::new(29, 21), ColorMode::Rgb, sample);
    doc.sample_encoding = Some(encoding);
    let profile = Arc::new(vec![0x91; 132]);
    doc.icc_profile = Some(if encoding == SampleEncoding::NonlinearProfile {
        profile.clone()
    } else {
        // Structural-only independent current bytes, not a CMS witness.
        Arc::new(vec![0x92; 133])
    });
    doc.source_color_context = Some(crate::SourceColorContext {
        source_precision: match (sample, encoding) {
            (SampleType::U16, _) => 250,
            (SampleType::U32, _) => 350,
            (SampleType::F32, SampleEncoding::Linear) => 600,
            (SampleType::F64, SampleEncoding::Linear) => 700,
            (SampleType::F32, SampleEncoding::Perceptual) => 675,
            (SampleType::F64, SampleEncoding::Perceptual) => 775,
            (SampleType::F32, _) => 650,
            _ => 750,
        },
        original_icc: Some(profile),
    });
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 12;
    let epoch = match encoding {
        SampleEncoding::Linear => XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        SampleEncoding::Perceptual => XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
        _ if matches!(sample, SampleType::U16 | SampleType::U32) => XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH,
        _ => XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
    };
    for (i, extent) in [doc.bounds(), Rect::new(-3, -4, 4, 9)].into_iter().enumerate() {
        let format = PixelFormat::new(doc.mode, sample, true);
        let descriptor = NativePlaneDescriptor::new(format, extent, &vec![0; format.bytes_per_pixel()]).unwrap();
        let backing = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &ctl()).unwrap();
        let mut layer = Layer::new(format!("layer{i}"), LayerContent::BackedRaster(BackedLayer::new(backing, descriptor, &ctl()).unwrap()));
        layer.import_compositing =
            Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
        if i == 1 {
            layer.opacity = 0.625;
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let default = match sample {
                SampleType::F64 => (-0.0f64).to_ne_bytes().to_vec(),
                SampleType::F32 => (-0.0f32).to_ne_bytes().to_vec(),
                _ => vec![0; format.bytes_per_pixel()],
            };
            let mut surface = Surface::try_with_default_bytes(format, &default).unwrap();
            surface.try_write_pixel_f64(-3, -4, &[0.25]).unwrap();
            surface.try_write_pixel_f64(17, 19, &[0.75]).unwrap(); // Stored off-operative words are retained, not pruned.
            surface.try_write_pixel_f64(18, 20, &[0.10000000000000002]).unwrap();
            let default_tile = surface.try_default_tile().unwrap();
            surface.put_tiles(std::iter::once((TileCoord::new(1, -1), Arc::new(default_tile))));
            layer.mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
            state.items.push(ItemState {
                item: ItemRef::Mask(MaskId(901)),
                name: "mask".into(),
                custom_name: false,
                visible: true,
                locks: Default::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(extent),
                mask: Some(MaskState {
                    owner: ItemRef::Layer(layer.id),
                    bounds: extent,
                    offset: [extent.x0, extent.y0],
                    outside_f64_bits: 0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: true,
                }),
                composition: None,
            });
        }
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(extent),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: epoch.into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: if i == 0 { -1 } else { 1 },
                stored_operator: if i == 0 { -1 } else { 1 },
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                }),
                pass_through: false,
            }),
        });
        doc.layers.push(layer);
    }
    state.selection.active = Some(ItemRef::Layer(doc.layers[1].id));
    state.selection.selected.push(ItemRef::Layer(doc.layers[1].id));
    doc.xcf_state = Some(state);
    AccountedDocument::from_owned(doc, Arc::new((fee, native_fee)), q, &ctl()).unwrap()
}

fn mask(owner: &AccountedDocument) -> &Surface {
    &owner.document().layers[1].mask.as_ref().unwrap().surface
}

#[test]
fn raw_physical_maps_defaults_and_canonical_bounds_translate_at_all_assigned_depths() {
    for (sample, encoding) in [
        (SampleType::U16, SampleEncoding::NonlinearProfile),
        (SampleType::U32, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::Linear),
        (SampleType::F32, SampleEncoding::Perceptual),
        (SampleType::F64, SampleEncoding::NonlinearProfile),
    ] {
        let original = fixture_encoding(sample, encoding);
        let id = original.document().layers[1].id;
        for (dx, dy) in [(-257, 255), (1, -1), (256, -256)] {
            original.validate_assigned_layer_translation(id, dx, dy, &ctl()).unwrap();
            let moved = original.try_with_assigned_layer_translation(id, dx, dy, &ctl()).unwrap();
            moved.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
            let mut expected = original.document().clone(); // External expected model only.
            expected.layers[1].content = moved.document().layers[1].content.clone();
            expected.layers[1].mask.as_mut().unwrap().surface = moved.document().layers[1].mask.as_ref().unwrap().surface.clone();
            let LayerContent::BackedRaster(leaf) = &moved.document().layers[1].content else { panic!("fixture") };
            let bounds = leaf.operative_bounds();
            for item in &mut expected.xcf_state.as_mut().unwrap().items {
                if item.item == ItemRef::Layer(id) || item.mask.is_some() {
                    item.raster_bounds = Some(bounds);
                }
                if let Some(m) = &mut item.mask {
                    m.bounds = bounds;
                    m.offset = [bounds.x0, bounds.y0];
                }
            }
            assert_eq!(moved.document(), &expected);
            assert_eq!(mask(&moved).default_bytes(), mask(&original).default_bytes());
            let bpp = mask(&original).format().bytes_per_pixel();
            let mut raw = [0; 8];
            let mut expected_tiles = std::collections::BTreeSet::new(); // Harness accounting excluded.
            for (coord, tile) in mask(&original).tiles() {
                let source = tile_rect(*coord).unwrap();
                expected_tiles.extend(shifted(source, i64::from(dx), i64::from(dy)).unwrap().tiles());
                for y in source.y0..source.y1 {
                    for x in source.x0..source.x1 {
                        let offset = local_offset(source, x, y, bpp).unwrap();
                        mask(&moved).try_read_pixel_bytes(x + dx, y + dy, &mut raw[..bpp]).unwrap();
                        assert_eq!(&raw[..bpp], &tile.bytes()[offset..offset + bpp]);
                    }
                }
            }
            assert_eq!(mask(&moved).tile_count(), expected_tiles.len()); // Allocated-default tiles never pruned.
            assert!(moved.quota().shares_ledger(original.quota()));
            assert!(Arc::ptr_eq(moved.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
            assert_eq!(moved.document().source_color_context, original.document().source_color_context);
        }
    }
}

#[test]
fn repeated_moves_use_one_original_mask_and_flat_guard_survives_title_and_metadata_copies() {
    let original = fixture(SampleType::F64);
    let id = original.document().layers[1].id;
    let baseline = original.quota().usage().unwrap();
    let mut moved = original.clone();
    for _ in 0..12 {
        moved = moved.try_with_assigned_layer_translation(id, 1, -1, &ctl()).unwrap();
    }
    let retention = moved.owner.retention.downcast_ref::<TranslationRetention>().unwrap();
    assert!(Arc::ptr_eq(&retention.root, &original.owner.retention));
    assert!(Arc::ptr_eq(&retention.mask.root_surface, &original.document().layers[1].mask.as_ref().unwrap().surface));
    assert_eq!(retention.mask.offset, (12, -12));
    assert!(mask(&moved).tile_count() <= mask(&original).tile_count() * 4);
    let weak = Arc::downgrade(&retention.mask);
    let named = moved.try_with_document_name("continued.pcraft", &ctl()).unwrap();
    let metadata = named.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: Some(false), opacity: Some(-0.0) }, &ctl()).unwrap();
    drop(moved);
    drop(named);
    assert!(weak.upgrade().is_some());
    assert!(Arc::ptr_eq(&metadata.document().layers[1].mask.as_ref().unwrap().surface, &weak.upgrade().unwrap().surface));
    let returned = metadata.try_with_assigned_layer_translation(id, -12, 12, &ctl()).unwrap();
    assert_eq!(mask(&returned), mask(&original));
    assert_eq!(returned.document().layers[1].opacity.to_bits(), (-0.0f32).to_bits());
    drop(returned);
    drop(metadata);
    assert!(weak.upgrade().is_none());
    assert_eq!(original.quota().usage().unwrap(), baseline);
}

#[test]
fn disabled_linked_masks_move_and_valid_zero_shares_owner_under_full_quota() {
    let original = fixture(SampleType::U16);
    let id = original.document().layers[1].id;
    let disabled = original.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &ctl()).unwrap();
    let moved = disabled.try_with_assigned_layer_translation(id, -1, 1, &ctl()).unwrap();
    assert!(!moved.document().layers[1].mask.as_ref().unwrap().enabled);
    assert!(!moved.document().xcf_state.as_ref().unwrap().items.iter().find_map(|i| i.mask.as_ref()).unwrap().enabled);
    let q = moved.quota();
    let baseline = q.usage().unwrap();
    let full = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(moved.try_with_assigned_layer_translation(id, 0, 0, &ctl()).unwrap().shares_model(&moved));
    assert!(matches!(moved.try_with_assigned_layer_translation(id, 1, 0, &ctl()), Err(BackingError::Quota)));
    drop(full);
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(original.try_with_assigned_layer_translation(original.document().layers[0].id, 0, 0, &ctl()).is_err());
}

#[test]
fn allocation_stage_cancellation_refunds_and_unlinked_or_hostile_moves_refuse() {
    let original = fixture(SampleType::F32);
    let id = original.document().layers[1].id;
    let q = original.quota();
    let baseline = q.usage().unwrap();
    let work = |_: u64| if q.usage().map_err(|_| BackingError::Integrity)?.index > baseline.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(matches!(
        original.try_with_assigned_layer_translation(id, 1, -1, &ReadControl { interrupt: &Interrupt::NONE, work: &work }),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    // Refuse the second private map-insertion poll after one generated tile
    // already exists. This checks rollback of actual partial payload, rather
    // than only denial of the first prospective metadata reservation.
    let insertions = std::sync::atomic::AtomicUsize::new(0);
    let after_tile = |units: u64| {
        if units == 256
            && q.usage().map_err(|_| BackingError::Integrity)?.resident > baseline.resident
            && insertions.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 1
        {
            return Err(BackingError::Cancelled);
        }
        Ok(())
    };
    assert!(matches!(
        original.try_with_assigned_layer_translation(id, 1, -1, &ReadControl { interrupt: &Interrupt::NONE, work: &after_tile }),
        Err(BackingError::Cancelled)
    ));
    assert!(insertions.load(std::sync::atomic::Ordering::Relaxed) >= 2);
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(original.try_with_assigned_layer_translation(id, i32::MIN, i32::MAX, &ctl()).is_err());
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = original.document().clone(); // Paid synthetic CURRENT refusal fixture, not source evidence.
    doc.layers[1].mask.as_mut().unwrap().linked = false;
    for item in &mut doc.xcf_state.as_mut().unwrap().items {
        if let Some(mask) = &mut item.mask {
            mask.linked = false;
        }
    }
    let unlinked = AccountedDocument::from_owned(doc, Arc::new((original.clone(), fee)), q.clone(), &ctl()).unwrap();
    assert!(unlinked.try_with_assigned_layer_translation(id, 0, 0, &ctl()).is_err());
}

#[test]
fn assigned_scalar_reference_uses_exact_upstream_addition_without_touching_backdrop() {
    let original = fixture(SampleType::F64);
    let fee = original.quota().reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = original.document().clone(); // Paid synthetic CURRENT metadata, not source evidence.
    doc.layers[0].effects.reference = Some((5.0, 6.0));
    doc.layers[1].effects.reference = Some((-0.0, -0.0));
    let id = doc.layers[1].id;
    let owner = AccountedDocument::from_owned(doc, Arc::new((original.clone(), fee)), original.quota().clone(), &ctl()).unwrap();
    let zero = owner.try_with_assigned_layer_translation(id, 0, 0, &ctl()).unwrap();
    assert!(zero.shares_model(&owner));
    assert_eq!(zero.document().layers[1].effects.reference.unwrap().0.to_bits(), (-0.0f64).to_bits());
    let moved = owner.try_with_assigned_layer_translation(id, 0, 3, &ctl()).unwrap();
    let reference = moved.document().layers[1].effects.reference.unwrap();
    assert_eq!(reference.0.to_bits(), 0f64.to_bits());
    assert_eq!(reference.1.to_bits(), 3f64.to_bits());
    assert_eq!(moved.document().layers[0].effects.reference, Some((5.0, 6.0)));
    assert!(Arc::ptr_eq(moved.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
}
