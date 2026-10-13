#![cfg(feature = "corpus")]
//! Stage-only cross-space proof using independently saved GIMP Normal originals.
//! Opacity is evaluated explicitly into a derived F64 source for this mask-free
//! full-opacity API; original native bytes/resources are retained and rehashed.
use photocraft_cms::Builtin;
use photocraft_color::PixelFormat;
use photocraft_compose::backed_normal::{NormalBandLimits, PreparedBackedNormal};
use photocraft_doc::{BackedLayer, Rect, SampleEncoding, import_compositing::GimpSpace};
use photocraft_raster::Interrupt;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use photocraft_raster::backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, sync::Arc};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    native: Vec<u8>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        let start = usize::try_from(offset).map_err(|_| BackingError::Integrity)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Integrity)?;
        out.copy_from_slice(self.native.get(start..end).ok_or(BackingError::Integrity)?);
        Ok(())
    }
}
fn work(_: u64) -> Result<(), BackingError> {
    Ok(())
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn actual_normal_auto_pair_cross_space_stage_matches_independent_png() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("explicit local corpus required")).join("composition-matrix");
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    for (phase, state_hash, ref_hash) in [
        ("source", "0029a07cb179e5f23c6f2f4bc11ee79f0f5b4c46b690d90aa52eca6482a44f0c", "8904b88277967ad41ced1c0f20142c2a78131e3178da4fbdc71324ce88cf7951"),
        ("edit", "0feebb02fc74668506f441941b1ec7478b5facf143716c1d710d63c7ddcac161", "d8124386132a04084c6043378bef2bccd9f010376b2bfddd6cc9eb77fb77f4df"),
    ] {
        let (source_name, source_hash) = if phase == "source" {
            ("mode-normal.xcf", "1cfb20a92da5a2180fa9ff8d77156c06e9bcb312922d488a50fdfdaab2e6d5c3")
        } else {
            ("mode-normal.edit.xcf", "8147ec1edd907e1e63f751303bf8a7b882807576658de72d10cd612c3ae70f9d")
        };
        let source = fs::read(root.join(source_name)).unwrap();
        assert_eq!(hash(&source), source_hash);
        let state_bytes = fs::read(root.join(format!("mode-normal.fresh-{phase}_state.json"))).unwrap();
        assert_eq!(hash(&state_bytes), state_hash);
        let state: Value = serde_json::from_slice(&state_bytes).unwrap();
        assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
        assert!(state["assigned_icc_sha256"].is_null());
        let canvas = Rect::new(0, 0, 16, 12);
        let mut back = vec![[0.0; 4]; 192];
        let q = BackingQuota::new(QuotaLimits { resident: 8 << 20, scratch: 8 << 20, index: 8 << 20 });
        for row in state["layers"].as_array().unwrap().iter().rev() {
            assert!(row["mask"].is_null());
            assert!(row["parent"].is_null());
            assert_eq!(row["composition"]["mode"], 28);
            let raw = fs::read(root.join(row["native"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&raw), row["native"]["sha256"]);
            let raw_before = raw.clone();
            let opacity = (row["opacity"].as_f64().unwrap() / 100.0) as f32;
            let bounds = Rect::from_xywh(
                row["offset"][0].as_i64().unwrap() as i32,
                row["offset"][1].as_i64().unwrap() as i32,
                row["size"][0].as_u64().unwrap() as u32,
                row["size"][1].as_u64().unwrap() as u32,
            );
            let mut derived = Vec::new();
            for p in raw.as_chunks::<16>().0 {
                for c in 0..4 {
                    let mut v = f64::from(f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()));
                    if c == 3 {
                        v *= f64::from(opacity);
                    }
                    derived.extend(v.to_ne_bytes());
                }
            }
            let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA64F, bounds, &[0; 32]).unwrap();
            let store = Arc::new(Store { descriptor, native: derived });
            let layer = BackedLayer::new(BackedRaster::new(store.clone(), q.clone(), 2, &ctl).unwrap(), descriptor, &ctl).unwrap();
            let prepared = PreparedBackedNormal::new_with_composite_space(
                &layer,
                Builtin::Srgb.profile(),
                SampleEncoding::NonlinearProfile,
                GimpSpace::LinearRgb,
                descriptor,
                NormalBandLimits::default(),
                &ctl,
            )
            .unwrap();
            let output = prepared.render_band_over(canvas, &back, &ctl).unwrap();
            back = output.pixels().to_vec();
            assert_eq!(raw, raw_before);
            assert_eq!(hash(&raw), row["native"]["sha256"]);
        }
        let ref_bytes = fs::read(root.join(format!("mode-normal.fresh-{phase}.ref.png"))).unwrap();
        assert_eq!(hash(&ref_bytes), ref_hash);
        let reference = photocraft_codecs::decode(&ref_bytes).unwrap();
        let mut worst = 0.0_f64;
        for y in 0..12 {
            for x in 0..16 {
                for (c, value) in back[(y * 16 + x) as usize].iter().enumerate() {
                    worst = worst.max((*value - f64::from(reference.get(x, y, c))).abs());
                }
            }
        }
        assert!(worst <= 1.0 / 65535.0, "{phase}: error {worst}");
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(fs::read(root.join(source_name)).unwrap(), source);
        println!("independent {phase} Normal cross-space stage: worst {}/255", worst * 255.0);
    }
}
