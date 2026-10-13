#![cfg(test)]
use super::*;
use crate::canvas::{ensure_texture, ensure_texture_with_interrupt};
use photocraft_doc::{import_compositing::*, item_identity::*, xcf_state::*, *};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{quota::*, *},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("normal-mask-ordinary-scene-evidence/medium")
}
fn read(root: &Path, value: &Value) -> Vec<u8> {
    let b = fs::read(root.join(value["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&b), value["sha256"]);
    b
}
fn bounds(row: &Value) -> Rect {
    let x = row["offset"][0].as_i64().unwrap() as i32;
    let y = row["offset"][1].as_i64().unwrap() as i32;
    Rect::new(x, y, x + row["size"][0].as_i64().unwrap() as i32, y + row["size"][1].as_i64().unwrap() as i32)
}
#[derive(Debug)]
struct Plane {
    fail: Arc<std::sync::atomic::AtomicBool>,
    bytes: Vec<u8>,
    descriptor: NativePlaneDescriptor,
    _lease: QuotaLease,
}
impl NativePlaneStore for Plane {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        if self.fail.load(std::sync::atomic::Ordering::Acquire) {
            return Err(BackingError::Io);
        }
        let a = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn fixture(
    root: &Path,
    row: &Value,
    fail: Arc<std::sync::atomic::AtomicBool>,
    corrupt_current_icc: bool,
) -> (photocraft_doc::accounted_document::AccountedDocument, BackingQuota, photocraft_cms::Profile) {
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 4 << 20, scratch: 0 });
    // Fixed bounded synthetic source/model retention; not a codec/RSS/map census.
    let retention = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mode = if row["model"] == "gray" { ColorMode::Grayscale } else { ColorMode::Rgb };
    let depth = match row["sample_type"].as_str().unwrap() {
        "u16" => SampleType::U16,
        "f32" => SampleType::F32,
        "f64" => SampleType::F64,
        _ => panic!("outside this oracle slice"),
    };
    let mut doc = Document::new("qualified ordinary scene", Size::new(513, 385), mode, depth);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let icc = read(root, &row["assigned_icc"]);
    let profile = photocraft_cms::Profile::parse(&icc).unwrap();
    doc.icc_profile = Some(Arc::new(icc));
    let mut state = XcfState::default();
    state.provenance.format_version = 12;
    for (index, r) in row["layers"].as_array().unwrap().iter().enumerate().rev() {
        let extent = bounds(r);
        let format = doc.pixel_format();
        let source_charge = q.reserve(QuotaKind::Resident, r["native"]["bytes"].as_u64().unwrap() + 256).unwrap();
        let mut bytes = read(root, &r["native"]);
        if cfg!(target_endian = "big") {
            for chunk in bytes.chunks_exact_mut(depth.bytes()) {
                chunk.reverse();
            }
        }
        let descriptor = NativePlaneDescriptor::new(format, extent, &vec![0; format.bytes_per_pixel()]).unwrap();
        let store = Arc::new(Plane { fail: fail.clone(), bytes, descriptor, _lease: source_charge });
        let backed = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
        let mut layer = Layer::new(r["name"].as_str().unwrap(), LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &ctl()).unwrap()));
        layer.opacity = (r["opacity"].as_f64().unwrap() / 100.) as f32;
        if index == 0 {
            let mask = &r["mask"];
            let mut maskbytes = read(root, &mask["native"]);
            if cfg!(target_endian = "big") {
                for chunk in maskbytes.chunks_exact_mut(depth.bytes()) {
                    chunk.reverse();
                }
            }
            let maskrect = bounds(mask);
            let surface = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, depth, false), maskrect, &maskbytes).unwrap();
            layer.mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
            state.items.push(ItemState {
                item: ItemRef::Mask(MaskId(1)),
                name: "own mask".into(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: None,
                composition: None,
                mask: Some(MaskState {
                    owner: ItemRef::Layer(layer.id),
                    bounds: maskrect,
                    offset: [maskrect.x0, maskrect.y0],
                    outside_f64_bits: 0.0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: true,
                }),
            });
        }
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(extent),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH.into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: if index == 0 { 1 } else { -1 },
                stored_operator: if index == 0 { 1 } else { -1 },
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
    doc.xcf_state = Some(state);
    if corrupt_current_icc {
        doc.icc_profile = Some(Arc::new(vec![0; 132]));
    }
    doc.validate_xcf_state().unwrap();
    (photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new(retention), q.clone(), &ctl()).unwrap(), q, profile)
}

fn data(root: &Path) -> Value {
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_normal_mask_ordinary_scene_hashes.json")).unwrap();
    assert_eq!(hash(&bytes), hashes["medium"]["observations.json"]);
    serde_json::from_slice(&bytes).unwrap()
}
fn native_values(bytes: &[u8], sample: SampleType) -> Vec<f64> {
    bytes
        .chunks_exact(sample.bytes())
        .map(|v| match sample {
            SampleType::U16 => f64::from(u16::from_le_bytes(v.try_into().unwrap())) / 65535.,
            SampleType::F32 => f64::from(f32::from_le_bytes(v.try_into().unwrap())),
            SampleType::F64 => f64::from_le_bytes(v.try_into().unwrap()),
            _ => panic!("outside oracle"),
        })
        .collect()
}

#[test]
#[ignore = "independent bounded GIMP masked preview control"]
fn accounted_mask_preview_matches_gimp_and_retains_last_good_on_failure() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = root();
    let rows = data(&root);
    for id in ["rgb-f64-op1-source", "gray-f64-op1-source"] {
        let row = rows["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap();
        let fail = Arc::new(AtomicBool::new(false));
        let (owner, quota, _) = fixture(&root, row, fail.clone(), false);
        let snapshot = photocraft_engine::document_snapshot::DocumentSnapshot::from_accounted(owner).unwrap();
        let baseline = quota.usage().unwrap();
        let (image, lease) = backed_snapshot_display_image(&snapshot, 8, 8, None, &Interrupt::NONE).unwrap();
        let reference = native_values(&read(&root, &row["projection"]["native"]), SampleType::F64);
        let channels = if row["model"] == "gray" { 1 } else { 3 };
        for (i, actual) in image.pixels.iter().enumerate() {
            let (x, y) = (i % 8, i / 8);
            let (x0, x1, y0, y1) = (x * 513 / 8, (x + 1) * 513 / 8, y * 385 / 8, (y + 1) * 385 / 8);
            let mut sum = [0.; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let b = (sy * 513 + sx) * (channels + 1);
                    let a = reference[b + channels];
                    sum[3] += a;
                    for c in 0..channels {
                        sum[c] += reference[b + c] * a;
                    }
                }
            }
            let area = ((x1 - x0) * (y1 - y0)) as f64;
            let a = sum[3] / area;
            let mut rgb = [0u8; 3];
            for (c, value) in rgb.iter_mut().enumerate() {
                let channel = if channels == 1 { 0 } else { c };
                *value = (if sum[3] == 0. { 0. } else { sum[channel] / sum[3] } * 255.).round().clamp(0., 255.) as u8;
            }
            let expected = egui::Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (a * 255.).round() as u8);
            for c in 0..4 {
                assert!((i16::from(actual.to_array()[c]) - i16::from(expected.to_array()[c])).abs() <= 2, "{id} pixel {i} channel {c}");
            }
        }
        drop(image);
        drop(lease);
        let mut session = photocraft_engine::Session::new();
        session.add_accounted_document(snapshot.accounted().unwrap().clone(), None, &ctl()).unwrap();
        let mut app = crate::PhotocraftApp::new(session, Default::default());
        let ctx = egui::Context::default();
        let first = ensure_texture(&mut app, &ctx, 0, None).unwrap();
        ctx.tex_manager().write().take_delta().clear();
        // Keep the physical last source tiles pinned independently of cache
        // eviction, so transient refund equality does not forbid legitimate
        // release of an unpinned cached owner on a failed cache miss.
        let pins: Vec<_> = app
            .session
            .active()
            .unwrap()
            .doc
            .layers
            .iter()
            .filter_map(|layer| {
                let LayerContent::BackedRaster(backed) = &layer.content else { return None };
                let extent = backed.operative_bounds().intersect(&app.session.active().unwrap().doc.bounds());
                Some(backed.checked_view().pin_tile(photocraft_geom::TileCoord::containing(extent.x1 - 1, extent.y1 - 1), &ctl()).unwrap())
            })
            .collect();
        let held = quota.usage().unwrap();
        let source = app.session.active().unwrap().doc.clone();
        fail.store(true, Ordering::Release);
        app.session.active_mut().unwrap().revision += 1;
        assert_eq!(ensure_texture(&mut app, &ctx, 0, None), Some(first));
        assert!(app.ui.status_error);
        assert!(ctx.tex_manager().write().take_delta().set.is_empty());
        assert_eq!(quota.usage().unwrap(), held);
        fail.store(false, Ordering::Release);
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::AcqRel) > 20;
        app.session.active_mut().unwrap().revision += 1;
        assert_eq!(ensure_texture_with_interrupt(&mut app, &ctx, 0, None, &Interrupt::cancel_only(&cancel)), Some(first));
        assert!(ctx.tex_manager().write().take_delta().set.is_empty());
        assert_eq!(quota.usage().unwrap(), held);
        let deny = quota.reserve(QuotaKind::Resident, (128 << 20) - held.resident - 1).unwrap();
        let denied = quota.usage().unwrap();
        app.session.active_mut().unwrap().revision += 1;
        assert_eq!(ensure_texture(&mut app, &ctx, 0, None), Some(first));
        assert!(app.ui.status_error);
        assert!(ctx.tex_manager().write().take_delta().set.is_empty());
        assert_eq!(quota.usage().unwrap(), denied);
        drop(deny);
        assert_eq!(quota.usage().unwrap(), held);
        assert!(app.session.active().unwrap().doc.shares_state(&source));
        drop(source);
        drop(pins);
        drop(app);
        drop(snapshot);
        assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
        assert!(baseline.resident > 0);
    }
}

#[test]
#[ignore = "independent GIMP masked source with hostile current ICC"]
fn accounted_mask_invalid_current_icc_refuses_without_source_or_lease_changes() {
    let root = root();
    let rows = data(&root);
    let row = rows["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-f64-op1-source").unwrap();
    let (owner, quota, _) = fixture(&root, row, Arc::new(std::sync::atomic::AtomicBool::new(false)), true);
    let snapshot = photocraft_engine::document_snapshot::DocumentSnapshot::from_accounted(owner).unwrap();
    let before = snapshot.clone();
    let usage = quota.usage().unwrap();
    assert!(backed_snapshot_display_image(&snapshot, 8, 8, None, &Interrupt::NONE).is_err());
    assert!(snapshot.shares_state(&before));
    assert_eq!(quota.usage().unwrap(), usage);
    drop(before);
    drop(snapshot);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
