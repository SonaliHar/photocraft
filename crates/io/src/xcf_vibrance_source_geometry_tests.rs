//! UNRUN synthetic origin-geometry policy controls; no new producer or numerical oracle claim.
use super::*;
use photocraft_xcf::Plane;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
const INPUTS: [(&str, &str); 2] = [
    (
        "temp/sol1-vibrance-owned-gimp-20261016/f32-author-v1/outputs/f32-neutral-rounding.source.xcf",
        "4e239a92719be5b66fb040b9d815295e5fb2cb9744ff55c8cc18980e80ec311c",
    ),
    (
        "temp/sol1-vibrance-owned-gimp-20261016/f64-author-v1/outputs/f64-neutral-rounding.source.xcf",
        "120c3e4bf2bb0655b9af3637f6bd9eccc5ca272bf6a960ce86e86a22489fc156",
    ),
];
fn file(wide: bool, width: u32, height: u32) -> XcfFile {
    let (path, sha) = INPUTS[usize::from(wide)];
    let b = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)).unwrap();
    assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), sha);
    let mut f = photocraft_xcf::decode(&b, &Default::default()).unwrap();
    f.width = width;
    f.height = height;
    let d = &mut f.layers[0];
    d.width = width;
    d.height = height;
    let p = d.plane.as_mut().unwrap();
    p.width = width;
    p.height = height;
    p.bytes.clear();
    for pixel in 0..u64::from(width) * u64::from(height) {
        for value in [if pixel % 2 == 0 { -0.0f64 } else { 0.5f64 }, 0.25, 1.0, if pixel % 3 == 0 { 0.0 } else { 0.75 }] {
            if wide {
                p.bytes.extend_from_slice(&value.to_ne_bytes());
            } else {
                p.bytes.extend_from_slice(&(value as f32).to_ne_bytes());
            }
        }
    }
    let m = d.effects[0].mask.as_mut().unwrap();
    m.width = width;
    m.height = height;
    let p = m.plane.as_mut().unwrap();
    p.width = width;
    p.height = height;
    p.bytes = vec![0; (width as usize) * (height as usize) * if wide { 8 } else { 4 }];
    f
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
#[test]
#[ignore = "UNRUN: synthetic decoded source origin1x1/tilecross257x259/widerow513x2 F32/F64 exactnative geometry; no producer or math acceptance"]
fn positive_origin_shapes_and_provider_blocks() {
    for wide in [false, true] {
        for (w, h) in [(1, 1), (257, 259), (513, 2)] {
            let f = file(wide, w, h);
            assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
            let p = f.layers[0].plane.as_ref().unwrap();
            let word = if wide { 8 } else { 4 };
            let mut count = 0u64;
            let mut blocks = 0;
            p.visit_native(&Interrupt::NONE, &mut |b| {
                assert!(!b.is_empty() && b.len() <= 64 * 4 * word && b.len() % (4 * word) == 0);
                count += b.len() as u64;
                blocks += 1;
                Ok(())
            })
            .unwrap();
            assert_eq!(count, u64::from(w) * u64::from(h) * 4 * word as u64);
            assert_eq!(p.native_byte_len(), Some(count));
            if w * h > 64 {
                assert!(blocks > 1);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: source geometry/offset/depth/native/mask/raw/activity scope remains closed without fallback"]
fn geometry_and_unchanged_policy_refusals() {
    for wide in [false, true] {
        for bad in 0..13 {
            let mut f = file(wide, 257, 2);
            match bad {
                0 => f.width = 0,
                1 => f.height = 262145,
                2 => {
                    f.width = 262144;
                    f.height = 262144;
                }
                3 => f.layers[0].offset = (1, 0),
                4 => f.layers[0].width = 256,
                5 => f.layers[0].effects[0].mask.as_mut().unwrap().height = 1,
                6 => {
                    f.layers[0].plane.as_mut().unwrap().bytes.pop();
                }
                7 => {
                    let p = f.layers[0].plane.as_mut().unwrap();
                    if wide {
                        p.bytes[..8].copy_from_slice(&2f64.to_ne_bytes());
                    } else {
                        p.bytes[..4].copy_from_slice(&2f32.to_ne_bytes());
                    }
                }
                8 => f.layers[0].plane.as_mut().unwrap().channels = 3,
                9 => f.layers[0].effects[0].state.blend_space = None,
                10 => f.layers[0].opacity = 0.5,
                11 => f.layers[0].effects[0].state.enabled = Some(false),
                _ => f.icc = vec![0; 640],
            }
            assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "{wide}/{bad}");
        }
    }
}
struct Probe {
    plane: Plane,
    mode: u8,
}
impl ExactParsedPlane for Probe {
    fn descriptor(&self) -> crate::xcf_import::plane::ParsedPlaneDescriptor {
        self.plane.descriptor()
    }
    fn native_byte_len(&self) -> Option<u64> {
        self.plane.native_byte_len()
    }
    fn visit_native(&self, c: &Interrupt<'_>, v: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        let stride = 4 * self.plane.sample.source_bytes();
        match self.mode {
            0 => self.plane.visit_native(c, v),
            1 => v(&self.plane.bytes),           // malformed >64pixel callback must still refuse
            2 => v(&self.plane.bytes[..stride]), // advertised count exceeds actual callback count
            3 => {
                self.plane.visit_native(c, v)?;
                v(&self.plane.bytes[..stride])
            }
            4 => v(&self.plane.bytes[..1]),
            _ => v(&[]),
        }
    }
}
#[test]
#[ignore = "UNRUN: exact64pixel callback contract, short/excess/misaligned/empty stream and perblock work/cancellation controls"]
fn native_contract_and_control_boundaries() {
    for wide in [false, true] {
        let f = file(wide, 65, 1);
        let p = f.layers[0].plane.as_ref().unwrap();
        for mode in 0..6 {
            let probe = Probe { plane: p.clone(), mode };
            assert_eq!(native(&probe, f.sample, 65, 1, 4, false, &Interrupt::NONE, Some(&budget())).unwrap(), mode == 0);
        }
        let stop = || true;
        assert!(matches!(native(p, f.sample, 65, 1, 4, false, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 1)]).unwrap();
        let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
        assert!(native(p, f.sample, 65, 1, 4, false, &Interrupt::NONE, Some(&b)).is_err());
    }
}
