//! UNRUN synthetic decoded fixtures; no source receipts, oracle or numerical claim.
//! Full raw/config/mask authority remains the selected mapper + paid L3 preparation.
use super::super::{Plane, plane::ParsedPlaneDescriptor};
use super::*;
use photocraft_xcf::{Argument, Compression, Drawable, Effect, EffectState, Unit};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn base_file<P>(plane: P, model: Model, sample: Sample, precision: u32, encoding: Encoding, geometry: bool, operation: &str) -> XcfFile<P> {
    let (width, height, owner_width, owner_height, offset) = if geometry { (295, 160, 257, 129, (-13, 13)) } else { (8, 6, 8, 6, (0, 0)) };
    XcfFile {
        paths: Vec::new(),
        tattoo: None,
        item_sets: Vec::new(),
        version: 22,
        width,
        height,
        model,
        precision,
        sample,
        encoding,
        compression: Compression::None,
        icc: Vec::new(),
        dpi: (72., 72.),
        unit: Unit::Inch,
        palette: Vec::new(),
        layers: vec![Drawable {
            width: owner_width,
            height: owner_height,
            model,
            alpha: true,
            offset,
            mode: 28,
            blend_space: 0,
            composite_space: -1,
            composite_mode: -1,
            plane: Some(plane),
            effects: vec![Effect {
                name: "predicate fixture".into(),
                icon: String::new(),
                operation: operation.into(),
                version: Some("0:0".into()),
                properties: Vec::new(),
                arguments: Vec::new(),
                mask: None,
                state: EffectState::default(),
            }],
            ..Default::default()
        }],
        channels: Vec::new(),
        guides: Vec::new(),
        diagnostics: Vec::new(),
        charged_bytes: 0,
        raw_properties: Vec::new(),
    }
}
fn base_plane(model: Model, sample: Sample, geometry: bool) -> Plane {
    let (width, height) = if geometry { (257, 129) } else { (8, 6) };
    let channels = if model == Model::Gray { 2 } else { 4 };
    let raw = match sample {
        Sample::U8 => vec![127],
        Sample::U16 => 32768u16.to_ne_bytes().to_vec(),
        Sample::U32 => 0x80000001u32.to_ne_bytes().to_vec(),
        Sample::F16 | Sample::F32 => 0.5f32.to_ne_bytes().to_vec(),
        Sample::F64 => 0.5f64.to_ne_bytes().to_vec(),
    };
    Plane {
        width,
        height,
        channels,
        sample: if sample == Sample::F16 { Sample::F32 } else { sample },
        bytes: raw.repeat(width as usize * height as usize * channels),
    }
}

fn fixture(operation: &str, model: Model, sample: Sample, precision: u32, encoding: Encoding, size: (u32, u32)) -> XcfFile {
    let mut p = base_plane(model, sample, false);
    let word = p.bytes[..p.sample.source_bytes()].to_vec();
    p.width = size.0;
    p.height = size.1;
    p.bytes = word.repeat(size.0 as usize * size.1 as usize * p.channels);
    let mut f = base_file(p, model, sample, precision, encoding, false, operation);
    f.width = size.0;
    f.height = size.1;
    f.layers[0].width = size.0;
    f.layers[0].height = size.1;
    if operation == "gegl:saturation" {
        f.layers[0].effects[0].arguments = vec![Argument {
            name: "colorspace".into(),
            property_index: 0,
            value: ArgumentValue::Enum(if model == Model::Gray { 2 } else { 0 }),
            configuration: None,
            graph: None,
        }];
    }
    f
}
fn origin<P: ExactParsedPlane>(f: &XcfFile<P>, balance: bool) -> Result<bool, IoError> {
    qualify_origin_geometry(f, &f.layers[0].effects[0].operation, balance, &Interrupt::NONE, None)
}
#[test]
#[ignore = "UNRUN: staged-only seven geometry and precision matrix; external decoded harness"]
fn seven_routes_keep_roles_while_frozen_eager_stays_small() {
    use super::super::{staged_configured_point::ConfiguredPointFamily as C, staged_residual_point::ResidualPointFamily as R};
    for size in [(1, 1), (1, 257), (257, 259)] {
        for (op, model, sample, precision, encoding, balance) in [
            ("gimp:desaturate", Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, false),
            ("gegl:saturation", Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, false),
            ("gegl:saturation", Model::Gray, Sample::F64, 700, Encoding::Linear, false),
            ("gegl:value-invert", Model::Rgb, Sample::F32, 600, Encoding::Linear, false),
            ("gimp:color-balance", Model::Rgb, Sample::F64, 700, Encoding::Linear, false),
            ("gimp:color-balance", Model::Rgb, Sample::U32, 350, Encoding::NonlinearProfile, true),
            ("gimp:color-balance", Model::Rgb, Sample::F16, 500, Encoding::Linear, true),
            ("gimp:color-balance", Model::Rgb, Sample::F32, 675, Encoding::Perceptual, true),
        ] {
            let mut f = fixture(op, model, sample, precision, encoding, size);
            assert!(origin(&f, balance).unwrap());
            assert!(!qualify(&f, op, balance, false, &Interrupt::NONE, None).unwrap());
            if op == "gimp:color-balance" {
                assert!(C::hint(&f).unwrap().qualify(&f, &Interrupt::NONE, None).unwrap());
            } else {
                assert!(R::hint(&f).unwrap().qualify(&f, &Interrupt::NONE, None).unwrap());
            }
            f.layers[0].effects[0].state.enabled = Some(false);
            assert!(origin(&f, balance).unwrap()); // PRESENT bool authority remains mapper, not native helper.
            if op == "gegl:saturation" && model == Model::Rgb {
                f.layers[0].effects[0].arguments[0].value = ArgumentValue::Enum(2);
                assert!(R::SaturationYuv.qualify(&f, &Interrupt::NONE, None).unwrap());
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: checked geometry, original role and half-widening refusal boundaries"]
fn bounds_activity_and_native_domain_refuse_without_repair() {
    let base = fixture("gimp:desaturate", Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, (257, 1));
    for change in [
        (|f: &mut XcfFile| f.width = 0) as fn(&mut XcfFile),
        |f| f.width = 262145,
        |f| {
            f.width = 262144;
            f.height = 262144;
        },
        |f| f.layers[0].offset = (1, 0),
        |f| f.layers[0].width -= 1,
        |f| f.layers[0].visible = false,
        |f| f.layers[0].opacity = 0.375,
        |f| f.icc.push(1),
        |f| f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
    ] {
        let mut f = base.clone();
        change(&mut f);
        assert!(!origin(&f, false).unwrap());
    }
    let mut half = fixture("gimp:color-balance", Model::Rgb, Sample::F16, 500, Encoding::Linear, (65, 1));
    half.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.50000006f32.to_ne_bytes());
    assert!(!origin(&half, true).unwrap());
    let yuv64 = fixture("gegl:saturation", Model::Rgb, Sample::F64, 750, Encoding::NonlinearProfile, (65, 1));
    let mut yuv64 = yuv64;
    yuv64.layers[0].effects[0].arguments[0].value = ArgumentValue::Enum(2);
    assert!(!origin(&yuv64, false).unwrap());
    assert!(!origin(&fixture("gegl:value-invert", Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, (65, 1)), false).unwrap());
}
#[derive(Clone, Copy)]
enum Fault {
    None,
    Length,
    Short,
    Extra,
    Oversized,
    Misaligned,
    LateError,
    Cancel,
}
struct Visitor {
    plane: Plane,
    fault: Fault,
    flag: Arc<AtomicBool>,
}
impl ExactParsedPlane for Visitor {
    fn descriptor(&self) -> ParsedPlaneDescriptor {
        self.plane.descriptor()
    }
    fn native_byte_len(&self) -> Option<u64> {
        Some(self.plane.bytes.len() as u64 + u64::from(matches!(self.fault, Fault::Length)))
    }
    fn visit_native(&self, c: &Interrupt<'_>, v: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        let stride = self.plane.channels * self.plane.sample.source_bytes();
        match self.fault {
            Fault::Short => {
                for b in self.plane.bytes[..self.plane.bytes.len() - stride].chunks(stride * 64) {
                    v(b)?;
                }
                Ok(())
            }
            Fault::Extra => {
                self.plane.visit_native(c, v)?;
                v(&self.plane.bytes[..stride])
            }
            Fault::Oversized => v(&vec![0; stride * 65]),
            Fault::Misaligned => v(&self.plane.bytes[..stride - 1]),
            Fault::LateError => {
                self.plane.visit_native(c, v)?;
                Err(bad("late harness error"))
            }
            Fault::Cancel => {
                self.plane.visit_native(c, v)?;
                self.flag.store(true, Ordering::SeqCst);
                Ok(())
            }
            _ => self.plane.visit_native(c, v),
        }
    }
}
fn visitor(fault: Fault) -> (XcfFile<Visitor>, Arc<AtomicBool>) {
    let f = fixture("gimp:desaturate", Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, (65, 1));
    let flag = Arc::new(AtomicBool::new(false));
    let mut out = base_file(
        Visitor { plane: f.layers[0].plane.clone().unwrap(), fault, flag: flag.clone() },
        Model::Rgb,
        Sample::F32,
        650,
        Encoding::NonlinearProfile,
        false,
        "gimp:desaturate",
    );
    out.width = 65;
    out.height = 1;
    out.layers[0].width = 65;
    out.layers[0].height = 1;
    (out, flag)
}
#[test]
#[ignore = "UNRUN: exact declared length, cumulative bytes and hard64 provider contract"]
fn malformed_streams_have_no_positive_qualification() {
    assert!(origin(&visitor(Fault::None).0, false).unwrap());
    assert!(!origin(&visitor(Fault::Length).0, false).unwrap());
    for fault in [Fault::Short, Fault::Extra, Fault::Oversized, Fault::Misaligned, Fault::LateError] {
        assert!(origin(&visitor(fault).0, false).is_err());
    }
}
#[test]
#[ignore = "UNRUN: same cumulative Budget, unchanged native-byte work and final cancellation"]
fn shared_work_and_final_cancel_are_effective() {
    let (f, flag) = visitor(Fault::None);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    let budget = photocraft_xcf::Budget::new(limits, None);
    assert!(qualify_origin_geometry(&f, "gimp:desaturate", false, &Interrupt::NONE, Some(&budget)).unwrap());
    let used = budget.used(photocraft_xcf::BudgetKind::Work);
    assert_eq!(used, 32 + 8 + 65 * 4 * (4 + 1));
    assert!(qualify_origin_geometry(&f, "gimp:desaturate", false, &Interrupt::NONE, Some(&budget)).unwrap());
    assert_eq!(budget.used(photocraft_xcf::BudgetKind::Work), used * 2);
    flag.store(true, Ordering::SeqCst);
    let stop = || flag.load(Ordering::SeqCst);
    assert!(matches!(qualify_origin_geometry(&f, "gimp:desaturate", false, &Interrupt::cancel_only(&stop), Some(&budget)), Err(IoError::Cancelled)));
    let (f, flag) = visitor(Fault::Cancel);
    let stop = || flag.load(Ordering::SeqCst);
    assert!(matches!(qualify_origin_geometry(&f, "gimp:desaturate", false, &Interrupt::cancel_only(&stop), None), Err(IoError::Cancelled)));
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 40)]).unwrap();
    let budget = photocraft_xcf::Budget::new(limits, None);
    assert!(qualify_origin_geometry(&visitor(Fault::None).0, "gimp:desaturate", false, &Interrupt::NONE, Some(&budget)).is_err());
}
