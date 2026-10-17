#![cfg(all(windows, feature = "corpus"))]
//! Actual independent XCF -> TEST-only exact authorized backing capture ->
//! native path -> paid owner -> checked masked renderer. No importer migration.
#[path = "common/accounted_raster_capture.rs"]
mod capture;
use photocraft_compose::{backed_document::PreparedBackedDocument, backed_normal::NormalBandLimits};
use photocraft_doc::{LayerContent, item_identity::ItemRef};
use photocraft_format::owned_document::OwnedDocumentLimits;
use photocraft_geom::Rect;
use photocraft_io::{accounted_native::*, xcf_source::ServiceAuthorizedScratchDirectory};
use photocraft_raster::{
    Interrupt,
    backing::{ReadControl, quota::*},
};
use photocraft_xcf::{Budget, ResourceLimits, ResourcePolicy};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn native_hash(layer: &photocraft_doc::BackedLayer) -> String {
    use photocraft_raster::checked_view::NativeTileBytes;
    let _hash_lease = layer.quota().reserve(QuotaKind::Resident, 512).unwrap();
    let mut hash = Sha256::new();
    let rect = layer.operative_bounds();
    let bpp = layer.format().bytes_per_pixel();
    let tile_size = photocraft_geom::TILE_SIZE;
    let mut y0 = rect.y0;
    while y0 < rect.y1 {
        // This authored medium fixture spans three tile columns. Keep its paid
        // pins for one tile-row stripe rather than thrashing the codec's two-tile
        // cache on every sample row; SHA input remains exact logical row order.
        let mut pins: [Option<(i32, usize, photocraft_raster::checked_view::CheckedTilePin)>; 3] = std::array::from_fn(|_| None);
        let y1 = y0 + (rect.y1 - y0).min(tile_size - y0.rem_euclid(tile_size));
        let mut x = rect.x0;
        let mut column = 0;
        while x < rect.x1 {
            let coord = photocraft_geom::TileCoord::containing(x, y0);
            let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
            let n = (rect.x1 - x).min(tile_size - x.rem_euclid(tile_size)) as usize;
            *pins.get_mut(column).expect("medium fixture has at most three tile columns") = Some((x, n, pin));
            column += 1;
            x += n as i32;
        }
        for y in y0..y1 {
            for (x, n, pin) in pins.iter().flatten() {
                ctl().check((n * bpp) as u64).unwrap();
                match pin.encoded() {
                    NativeTileBytes::FullTile(bytes) => {
                        let start = ((y.rem_euclid(tile_size) * tile_size + x.rem_euclid(tile_size)) as usize) * bpp;
                        hash.update(&bytes[start..start + n * bpp]);
                    }
                    NativeTileBytes::DefaultPixel(pixel) => {
                        for _ in 0..*n {
                            hash.update(pixel);
                        }
                    }
                }
            }
        }
        y0 = y1;
    }
    hash.finalize().iter().map(|v| format!("{v:02x}")).collect()
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-mask-native-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn test_capture_refuses_nondefault_retained_padding_without_erasing_source() {
    let format = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F64, true);
    let bounds = Rect::new(0, 0, 1, 1);
    let mut surface = photocraft_raster::Surface::new(format);
    let bits = [(-0.0f64).to_ne_bytes(), 1.0f64.to_ne_bytes()].concat();
    surface.try_write_interleaved(Rect::new(1, 0, 2, 1), &bits).unwrap();
    assert_eq!(capture::validate_padding(&surface, bounds, &ctl()), Err(photocraft_raster::backing::BackingError::Unsupported));
    assert_eq!(surface.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap(), bits);
}

#[test]
#[ignore = "requires reviewed unpublished ordinary-mask medium corpus; no default import migration"]
fn six_actual_masked_native_paths_preserve_source_current_metadata_projection_and_lifetime() {
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let input = full.join("normal-mask-ordinary-scene-evidence/medium");
    let observations = fs::read(input.join("observations.json")).unwrap();
    let pins: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_normal_mask_ordinary_scene_hashes.json")).unwrap();
    assert_eq!(hash(&observations), pins["medium"]["observations.json"]);
    let observations: Value = serde_json::from_slice(&observations).unwrap();
    let mut count = 0;
    for record in observations["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["id"].as_str().unwrap().ends_with("op1-source") && matches!(r["sample_type"].as_str().unwrap(), "u16" | "f32" | "f64"))
    {
        count += 1;
        let source_path = input.join(record["source_file"].as_str().unwrap());
        let source = fs::read(&source_path).unwrap();
        assert_eq!(hash(&source), record["source_sha256"]);
        let mut doc = photocraft_io::xcf_import::import("independent masked source", &source, &Interrupt::NONE).unwrap().document;
        let original_state = doc.xcf_state.clone();
        let original_profile = doc.icc_profile.clone();
        let original_masks: Vec<_> = doc.layers.iter().map(|l| l.mask.clone()).collect();
        let temp = Temp::new();
        let scratch = temp.0.join("scratch");
        fs::create_dir(&scratch).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, scratch: 256 << 20, index: 16 << 20 });
        {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            let state = doc.xcf_state.as_ref().unwrap();
            for layer in &mut doc.layers {
                let bounds = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).unwrap().raster_bounds.unwrap();
                let LayerContent::Raster(surface) = &layer.content else { panic!("ordinary source not raster") };
                let row = record["layers"].as_array().unwrap().iter().find(|r| r["name"] == layer.name).unwrap();
                let expected = fs::read(input.join(row["native"]["file"].as_str().unwrap())).unwrap();
                assert_eq!(hash(&expected), row["native"]["sha256"]);
                // Independent authored native bytes establish logical sample order;
                // validate_padding separately establishes exact physical retention.
                let actual = surface.try_read_interleaved(bounds).unwrap();
                assert_eq!(actual, expected, "{} {} original native", record["id"], layer.name);
                let backed = capture::capture(surface, bounds, &service, &q, &ctl()).unwrap();
                assert_eq!(backed.descriptor().default_bytes(), surface.default_bytes());
                let mut one = vec![0; surface.format().bytes_per_pixel()];
                backed.checked_view().read_logical_pixel_native_into(bounds.x0, bounds.y0, &mut one, &ctl()).unwrap();
                assert_eq!(one, expected[..one.len()]);
                layer.content = LayerContent::BackedRaster(backed);
            }
        }
        assert_eq!(doc.xcf_state, original_state);
        assert_eq!(doc.icc_profile, original_profile);
        for (layer, before) in doc.layers.iter().zip(&original_masks) {
            assert_eq!(layer.mask, *before);
            if let (Some(after), Some(before)) = (&layer.mask, before) {
                assert!(std::sync::Arc::ptr_eq(&after.surface, &before.surface));
            }
        }
        let path = temp.0.join("current.pcraft");
        let write = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &ctl()).unwrap();
        save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &ctl(), |_, _| Ok(())).unwrap();
        let saved_hash = hash(&fs::read(&path).unwrap());
        let gates = AtomicUsize::new(0);
        let usage = q.usage().unwrap();
        assert!(
            save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &ctl(), |_, _| {
                if gates.fetch_add(1, Ordering::SeqCst) == 0 { Ok(()) } else { Err(photocraft_raster::backing::BackingError::Cancelled.into()) }
            })
            .is_err()
        );
        assert_eq!(hash(&fs::read(&path).unwrap()), saved_hash);
        assert_eq!(q.usage().unwrap(), usage);
        // Explicit write authority cannot override detected original XCF identity.
        let protected = temp.0.join("original.xcf");
        fs::write(&protected, &source).unwrap();
        let protected_cap = ServiceAuthorizedNativeWritePath::from_platform_authorized(protected.clone(), &q, &ctl()).unwrap();
        assert!(save_accounted_native_path(&doc, &protected_cap, OwnedDocumentLimits::default(), &q, &ctl(), |_, _| Ok(())).is_err());
        assert_eq!(fs::read(&protected).unwrap(), source);
        drop(protected_cap);
        let read = ServiceAuthorizedNativeReadPath::from_platform_authorized(path.clone(), &q, &ctl()).unwrap();
        let loaded = {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            open_accounted_native(&read, &service, OwnedDocumentLimits::default(), q.clone(), &ctl()).unwrap()
        };
        assert_eq!(loaded.document().xcf_state, original_state);
        assert_eq!(loaded.document().icc_profile, original_profile);
        for (layer, before) in loaded.document().layers.iter().zip(&original_masks) {
            assert_eq!(layer.mask, *before);
        }
        drop(doc);
        drop(read);
        drop(write);
        fs::remove_file(path).unwrap();
        let owner = loaded.into_accounted_document(&ctl()).unwrap();
        for layer in &owner.document().layers {
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("native layer erased") };
            let row = record["layers"].as_array().unwrap().iter().find(|r| r["name"] == layer.name).unwrap();
            assert_eq!(native_hash(backed), row["native"]["sha256"], "{} exact reopened native", record["id"]);
        }
        let profile = photocraft_cms::Profile::parse(owner.document().icc_profile.as_ref().unwrap()).unwrap();
        let prepared = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
        let band = prepared.render_band(Rect::new(0, 0, 128, 64), &ctl()).unwrap();
        let projection = fs::read(input.join(record["projection"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&projection), record["projection"]["native"]["sha256"]);
        let channels = owner.document().mode.color_channels();
        let sample = owner.document().depth;
        for (index, pixel) in band.pixels().iter().enumerate() {
            let offset = ((index / 128) * 513 + index % 128) * (channels + 1);
            let alpha = photocraft_color::try_read_sample_f64(&projection, sample, offset + channels).unwrap();
            assert!((pixel[3] - alpha).abs() <= 1. / 65535., "{} alpha", record["id"]);
            for (c, value) in pixel.iter().enumerate().take(channels) {
                let expected = photocraft_color::try_read_sample_f64(&projection, sample, offset + c).unwrap();
                assert!((value * pixel[3] - expected * alpha).abs() <= 2. / 255., "{} projection", record["id"]);
            }
        }
        let history = owner.clone();
        let id = owner.document().layers[0].id;
        // This ordinary masked native is the genuine Legacy data owner, not
        // the separate assigned-geometry compound-metadata programme.
        assert!(owner.is_legacy_storage());
        let renamed = owner.try_edit(photocraft_doc::accounted_document::AccountedEdit::LayerName { layer: id, name: "paid native rename" }, &ctl()).unwrap();
        assert_eq!(history.document().xcf_state, original_state);
        assert_eq!(renamed.document().layers[0].name, "paid native rename");
        let resave_path = temp.0.join("renamed.pcraft");
        let resave = ServiceAuthorizedNativeWritePath::from_platform_authorized(resave_path.clone(), &q, &ctl()).unwrap();
        save_accounted_native_path(renamed.document(), &resave, OwnedDocumentLimits::default(), &q, &ctl(), |_, _| Ok(())).unwrap();
        drop(resave);
        let reopen = ServiceAuthorizedNativeReadPath::from_platform_authorized(resave_path.clone(), &q, &ctl()).unwrap();
        let reopened = {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            open_accounted_native(&reopen, &service, OwnedDocumentLimits::default(), q.clone(), &ctl()).unwrap()
        };
        assert_eq!(reopened.document().xcf_state, renamed.document().xcf_state);
        assert_eq!(reopened.document().layers[0].name, "paid native rename");
        for (before, after) in renamed.document().layers.iter().zip(&reopened.document().layers) {
            let (LayerContent::BackedRaster(before), LayerContent::BackedRaster(after)) = (&before.content, &after.content) else {
                panic!("current native erased")
            };
            assert_eq!(native_hash(before), native_hash(after));
        }
        drop(reopen);
        drop(reopened);
        fs::remove_file(resave_path).unwrap();
        assert_eq!(hash(&fs::read(source_path).unwrap()), record["source_sha256"]);
        drop(band);
        drop(prepared);
        drop(history);
        drop(renamed);
        drop(owner);
        drop(original_masks);
        drop(original_state);
        drop(original_profile);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        assert_eq!(fs::read_dir(scratch).unwrap().count(), 0);
    }
    assert_eq!(count, 6);
}
