//! UNRUN dimension-only storage regression drafts.
//! Geometry evidence and production admission are independent of these fixtures.
//! Synthetic fixtures establish storage ownership, never source/render proof.
use super::*;
use crate::{
    BackedLayer, ColorMode, Layer, PixelFormat, SampleType, Size,
    item_identity::{FilterId, MaskId},
    xcf_filters::{FilterArgument, FilterNode, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaLimits, QuotaUsage},
    },
};
use std::sync::atomic::{AtomicU64, Ordering};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 0, index: 16 << 20 })
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        output.fill(0);
        Ok(())
    }
}
fn fixture(q: &BackingQuota, width: u32, height: u32, mode: ColorMode, sample: SampleType, linear: bool) -> (Document, Arc<dyn Any + Send + Sync>) {
    // Prospectively pay source model/default/map/native fixture construction.
    assert!(width > 0 && width <= 257 && height > 0 && height <= 257);
    let columns = (u64::from(width) + 255) / 256;
    let rows = (u64::from(height) + 255) / 256;
    let payload = columns * rows * 65536 * sample.bytes() as u64;
    let guard: Arc<dyn Any + Send + Sync> = Arc::new(q.reserve(QuotaKind::Resident, payload + (2 << 20)).unwrap());
    let bounds = Rect::new(0, 0, width as i32, height as i32);
    let format = PixelFormat::new(mode, sample, true);
    let descriptor = NativePlaneDescriptor::new(format, bounds, &vec![0; format.bytes_per_pixel()]).unwrap();
    let backing = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    let layer = Layer::new("retained layer", LayerContent::BackedRaster(BackedLayer::new(backing, descriptor, &control()).unwrap()));
    let id = layer.id;
    let mut document = Document::new("old", Size::new(width, height), mode, sample);
    document.layers.push(layer);
    document.sample_encoding = Some(if linear { crate::SampleEncoding::Linear } else { crate::SampleEncoding::NonlinearProfile });
    document.source_color_context = Some(crate::SourceColorContext {
        original_icc: None,
        source_precision: match (sample, linear) {
            (SampleType::U16, true) => 200,
            (SampleType::U16, false) => 250,
            (SampleType::F32, true) => 600,
            (SampleType::F32, false) => 650,
            (SampleType::F64, true) => 700,
            (SampleType::F64, false) => 750,
            _ => unreachable!(),
        },
    });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.provenance.source_hash = Some("a".repeat(64));
    let filter = FilterId(100);
    let mask = MaskId(101);
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: 7,
        nodes: vec![FilterNode {
            id: filter,
            operation: OperationIdentity { name: "gimp:threshold-alpha".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() },
            source_display_name: Some("Original threshold".into()),
            source_icon: Some("retained-icon".into()),
            arguments: vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())) }],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(2),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: None,
        }],
    });
    {
        let mask_format = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let mut default = vec![0; sample.bytes()];
        photocraft_color::try_write_sample_f64(&mut default, sample, 0, -0.0).unwrap();
        let mut surface = Surface::try_with_default_bytes(mask_format, &default).unwrap();
        for ty in 0..rows as i32 {
            for tx in 0..columns as i32 {
                let tile = surface.tile_mut(photocraft_geom::TileCoord::new(tx, ty));
                // Includes the complete padding of edge tiles, not merely ROI words.
                for (index, word) in tile.bytes_mut().chunks_exact_mut(sample.bytes()).enumerate() {
                    let value = if index % 2 == 0 { -0.0 } else { 0.5 + 2f64.powi(-40) };
                    photocraft_color::try_write_sample_f64(word, sample, 0, value).unwrap();
                }
            }
        }
        state.filter_masks.push(FilterMask { id: mask, owner: filter, bounds, surface });
        // Actual shared mapper can emit mask metadata before the layer metadata.
        state.items.push(ItemState {
            item: ItemRef::Mask(mask),
            name: "original selection".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(17),
            source_flags: 0,
            raster_bounds: Some(bounds),
            composition: None,
            mask: Some(MaskState {
                owner: ItemRef::Layer(id),
                bounds,
                offset: [0, 0],
                outside_f64_bits: 0f64.to_bits(),
                enabled: true,
                shown: false,
                editing: false,
                linked: false,
            }),
        });
    }
    state.items.push(ItemState {
        item: ItemRef::Layer(id),
        name: "retained layer".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(18),
        source_flags: 0,
        raster_bounds: Some(bounds),
        mask: None,
        composition: None,
    });
    state.selection.selected.push(ItemRef::Layer(id));
    state.selection.active = Some(ItemRef::Layer(id));
    document.xcf_state = Some(state);
    (document, guard)
}

fn adopt(document: Document, guard: Arc<dyn Any + Send + Sync>, q: &BackingQuota) -> AccountedDocument {
    AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q.clone(), &control()).unwrap()
}
fn mask(owner: &AccountedDocument) -> &Surface {
    &owner.document().xcf_state.as_ref().unwrap().filter_masks[0].surface
}
fn patch(owner: &AccountedDocument, value: Option<u32>, enabled: Option<bool>) -> ThresholdAlphaPatch {
    let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    ThresholdAlphaPatch {
        layer: owner.document().layers[0].id,
        filter: stack.nodes[0].id,
        expected_stack_revision: stack.revision,
        value_f32_bits: value,
        enabled,
    }
}
fn assert_shared_raw(a: &Surface, b: &Surface) {
    assert_eq!(a.format(), b.format());
    assert_eq!(a.default_bytes(), b.default_bytes());
    assert_eq!(a.tile_count(), b.tile_count());
    for (coordinate, tile) in a.tiles() {
        let other = b.tile(*coordinate).unwrap();
        assert_eq!(tile.bytes(), other.bytes());
        assert!(Arc::ptr_eq(tile, other));
    }
}

#[test]
fn full_canvas_boundaries_preserve_each_native_mask_word_and_map_identity() {
    for (width, height) in [(256, 1), (257, 3), (3, 257), (257, 257)] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
                for linear in [false, true] {
                    let q = quota();
                    let (document, guard) = fixture(&q, width, height, mode, sample, linear);
                    let source = adopt(document, guard, &q);
                    source.validate_readonly_threshold_alpha_storage(&control()).unwrap();
                    let named = source.try_with_document_name("cross-tile.pcraft", &control()).unwrap();
                    assert_shared_raw(mask(&source), mask(&named));
                    assert_eq!(named.document().bounds(), Rect::new(0, 0, width as i32, height as i32));
                    if sample == SampleType::F64 {
                        assert_eq!(mask(&named).default_bytes(), &(-0.0f64).to_ne_bytes());
                        for (_, tile) in mask(&named).tiles() {
                            assert_eq!(&tile.bytes()[..8], &(-0.0f64).to_ne_bytes());
                        }
                    }
                    // Full physical words include the padded area of the last tile.
                    for (_, tile) in mask(&named).tiles() {
                        let last = tile.bytes().chunks_exact(sample.bytes()).last().unwrap();
                        let value = photocraft_color::try_read_sample_f64(last, sample, 0).unwrap();
                        assert_eq!(
                            value,
                            match sample {
                                SampleType::U16 => 32768.0 / 65535.0,
                                SampleType::F32 => 0.5,
                                _ => 0.5 + 2f64.powi(-40),
                            }
                        );
                    }
                    drop(named);
                    drop(source);
                    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
                }
            }
        }
    }
    // Sparse default-only and last-grid-entry maps are valid storage shapes too.
    for sparse in [false, true] {
        let q = quota();
        let (mut document, guard) = fixture(&q, 257, 257, ColorMode::Grayscale, SampleType::F64, false);
        let surface = &mut document.xcf_state.as_mut().unwrap().filter_masks[0].surface;
        let mut replacement = Surface::try_with_default_bytes(surface.format(), surface.default_bytes()).unwrap();
        if sparse {
            let last = photocraft_geom::TileCoord::new(1, 1);
            replacement.put_tiles(std::iter::once((last, surface.tile(last).unwrap().clone())));
        }
        *surface = replacement;
        let owner = adopt(document, guard, &q);
        let renamed = owner.try_with_document_name("sparse.pcraft", &control()).unwrap();
        assert_eq!(mask(&renamed).tile_count(), usize::from(sparse));
        assert_shared_raw(mask(&owner), mask(&renamed));
        drop(renamed);
        drop(owner);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn title_and_current_node_copies_keep_all_tiles_and_old_state_immutable() {
    let q = quota();
    let (document, guard) = fixture(&q, 257, 257, ColorMode::Rgb, SampleType::F64, false);
    let source = adopt(document, guard, &q);
    let named = source.try_with_document_name("new.pcraft", &control()).unwrap();
    let changed = named.try_with_threshold_alpha(patch(&named, Some(0.25f32.to_bits()), Some(false)), &control()).unwrap();
    assert_shared_raw(mask(&source), mask(&changed));
    let original = source.document().xcf_state.as_ref().unwrap();
    let current = changed.document().xcf_state.as_ref().unwrap();
    assert_eq!(original.items, current.items);
    assert_eq!(original.provenance, current.provenance);
    assert_eq!(original.filter_masks, current.filter_masks);
    assert_eq!(original.filter_stacks[0].revision, 7);
    assert_eq!(current.filter_stacks[0].revision, 8);
    assert!(original.filter_stacks[0].nodes[0].enabled);
    assert!(!current.filter_stacks[0].nodes[0].enabled);
    let no_op = changed.try_with_threshold_alpha(patch(&changed, Some(0.25f32.to_bits()), Some(false)), &control()).unwrap();
    assert!(no_op.shares_model(&changed));
    assert!(changed.try_edit(AccountedEdit::LayerVisibility { layer: changed.document().layers[0].id, visible: false }, &control()).is_err());
    assert!(changed.try_replace_selection(None, &control()).is_err());
}

#[test]
fn hostile_coordinates_extents_and_native_lengths_refuse_without_retained_fees() {
    // Allocation-free exact established cap boundary, not a large fixture.
    assert!(canvas_layout(Rect::new(0, 0, 262_144, 4096)).is_ok());
    assert!(canvas_layout(Rect::new(0, 0, 262_144, 4097)).is_err());
    assert!(canvas_layout(Rect::new(0, 0, 262_145, 1)).is_err());
    for alteration in 0..7 {
        let q = quota();
        let (mut document, guard) = fixture(&q, 257, 3, ColorMode::Rgb, SampleType::F64, false);
        match alteration {
            0 | 1 | 2 => {
                let surface = &mut document.xcf_state.as_mut().unwrap().filter_masks[0].surface;
                let tile = surface.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap().clone();
                let coordinate = match alteration {
                    0 => (-1, 0),
                    1 => (2, 0),
                    _ => (i32::MAX, 0),
                };
                surface.put_tiles(std::iter::once((photocraft_geom::TileCoord::new(coordinate.0, coordinate.1), tile)));
            }
            3 => document.size.width = 0,
            4 => document.size.width = u32::MAX,
            5 => document.xcf_state.as_mut().unwrap().filter_masks[0].bounds.x1 -= 1,
            _ => {
                let mut wrong = Surface::new(PixelFormat::GRAY8);
                let tile = wrong.tile_mut(photocraft_geom::TileCoord::new(0, 0)).clone();
                document.xcf_state.as_mut().unwrap().filter_masks[0]
                    .surface
                    .put_tiles(std::iter::once((photocraft_geom::TileCoord::new(0, 0), Arc::new(tile))));
            }
        }
        assert!(AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q.clone(), &control()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn real_quota_work_and_cancellation_denials_leave_source_and_usage_unchanged() {
    let q = quota();
    let (document, guard) = fixture(&q, 257, 257, ColorMode::Grayscale, SampleType::F64, true);
    let source = adopt(document, guard, &q);
    let before = q.usage().unwrap();
    let visited = AtomicU64::new(0);
    let deny = |units| {
        visited.fetch_add(units, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    assert!(source.try_with_document_name("denied.pcraft", &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert!(visited.load(Ordering::Relaxed) > 0);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(source.try_with_threshold_alpha(patch(&source, None, Some(false)), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let held = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(source.try_with_document_name("no-index.pcraft", &control()).is_err());
    drop(held);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(source.document().name, "old");
    assert_eq!(source.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
    assert_eq!(mask(&source).tile_count(), 4);
}

#[test]
fn chained_multitile_copies_keep_original_payload_guard_without_intermediate_chain() {
    let q = quota();
    let (document, guard) = fixture(&q, 257, 3, ColorMode::Rgb, SampleType::F64, false);
    let original = adopt(document, guard, &q);
    let old = original.downgrade();
    let named = original.try_with_document_name("one.pcraft", &control()).unwrap();
    let intermediate = named.downgrade();
    let last = named.try_with_threshold_alpha(patch(&named, Some(0.5f32.to_bits()), None), &control()).unwrap();
    assert_shared_raw(mask(&original), mask(&last));
    drop(original);
    drop(named);
    assert!(old.upgrade().is_some());
    assert!(intermediate.upgrade().is_none());
    assert_eq!(mask(&last).tile_count(), 2);
    last.validate_readonly_threshold_alpha_storage(&control()).unwrap();
    drop(last);
    assert!(old.upgrade().is_none());
    drop(old);
    drop(intermediate);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
