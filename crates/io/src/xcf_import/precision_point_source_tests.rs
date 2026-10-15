//! UNRUN frozen eager-predicate equivalence and exact visitor/control drafts.
use super::super::{Plane, plane::ParsedPlaneDescriptor};
use super::*;
use photocraft_xcf::{Argument, Compression, Drawable, Effect, EffectState, Unit};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

fn file<P>(plane: P, model: Model, sample: Sample, precision: u32, encoding: Encoding, geometry: bool, operation: &str) -> XcfFile<P> {
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
fn plane(model: Model, sample: Sample, geometry: bool) -> Plane {
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
fn compare(file: &XcfFile, operation: &str, balance: bool, geometry: bool) {
    assert_eq!(qualify(file, operation, balance, geometry, &Interrupt::NONE, None).unwrap_or(false), legacy_oracle(file, operation, balance, geometry));
}

#[test]
fn original_eager_precision_model_geometry_matrix_has_identical_boolean_eligibility() {
    let precisions = [
        (100, Sample::U8, Encoding::Linear),
        (150, Sample::U8, Encoding::NonlinearProfile),
        (175, Sample::U8, Encoding::Perceptual),
        (200, Sample::U16, Encoding::Linear),
        (250, Sample::U16, Encoding::NonlinearProfile),
        (275, Sample::U16, Encoding::Perceptual),
        (300, Sample::U32, Encoding::Linear),
        (350, Sample::U32, Encoding::NonlinearProfile),
        (375, Sample::U32, Encoding::Perceptual),
        (500, Sample::F16, Encoding::Linear),
        (550, Sample::F16, Encoding::NonlinearProfile),
        (575, Sample::F16, Encoding::Perceptual),
        (600, Sample::F32, Encoding::Linear),
        (650, Sample::F32, Encoding::NonlinearProfile),
        (675, Sample::F32, Encoding::Perceptual),
        (700, Sample::F64, Encoding::Linear),
        (750, Sample::F64, Encoding::NonlinearProfile),
        (775, Sample::F64, Encoding::Perceptual),
    ];
    for model in [Model::Rgb, Model::Gray] {
        for (precision, sample, encoding) in precisions {
            for (operation, balance, geometry) in
                [("gimp:threshold-alpha", false, false), ("gimp:color-balance", true, false), ("gegl:exposure", false, true), ("unlisted-narrow", false, false)]
            {
                let file = file(plane(model, sample, geometry), model, sample, precision, encoding, geometry, operation);
                compare(&file, operation, balance, geometry);
            }
        }
    }
    let valid = file(plane(Model::Rgb, Sample::F32, false), Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, false, "gimp:threshold-alpha");
    assert!(qualify(&valid, "gimp:threshold-alpha", false, false, &Interrupt::NONE, None).unwrap());
}

#[test]
fn eager_negative_metadata_native_nonunit_and_half_exactness_match_frozen_predicate() {
    let base = file(plane(Model::Rgb, Sample::F32, false), Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, false, "gimp:threshold-alpha");
    let mutate: &[fn(&mut XcfFile)] = &[
        |f| f.version = 21,
        |f| f.width += 1,
        |f| f.icc.push(0),
        |f| f.layers[0].visible = false,
        |f| f.layers[0].offset = (1, 0),
        |f| f.layers[0].opacity = 0.5,
        |f| f.layers[0].mode = 0,
        |f| f.layers[0].composite_mode = 1,
        |f| f.layers[0].effects[0].operation = "gegl:exposure".into(),
        |f| f.layers[0].plane.as_mut().unwrap().bytes.truncate(7),
        |f| f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
        |f| f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.001f32.to_ne_bytes()),
    ];
    for change in mutate {
        let mut hostile = base.clone();
        change(&mut hostile);
        assert!(!legacy_oracle(&hostile, "gimp:threshold-alpha", false, false));
        compare(&hostile, "gimp:threshold-alpha", false, false);
    }
    let mut half = file(plane(Model::Rgb, Sample::F16, false), Model::Rgb, Sample::F16, 550, Encoding::NonlinearProfile, false, "gimp:color-balance");
    assert!(legacy_oracle(&half, "gimp:color-balance", true, false));
    half.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.50000006f32.to_ne_bytes());
    assert!(!legacy_oracle(&half, "gimp:color-balance", true, false));
    compare(&half, "gimp:color-balance", true, false);
}

#[test]
fn saturation_colorspace_any_match_preserves_original_model_and_sample_restrictions() {
    for model in [Model::Rgb, Model::Gray] {
        for (sample, precision) in [(Sample::U16, 250), (Sample::F32, 650), (Sample::F64, 750)] {
            for values in [vec![], vec![0], vec![1], vec![2], vec![3], vec![3, 0]] {
                let mut f = file(plane(model, sample, false), model, sample, precision, Encoding::NonlinearProfile, false, "gegl:saturation");
                f.layers[0].effects[0].arguments = values
                    .into_iter()
                    .map(|value| Argument { name: "colorspace".into(), property_index: 0, value: ArgumentValue::Enum(value), configuration: None, graph: None })
                    .collect();
                compare(&f, "gegl:saturation", false, false);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    Short,
    Extra,
    Misaligned,
    Oversized,
    Error,
    FinalCancel,
}
struct Visitor {
    original: Plane,
    fault: Fault,
    visits: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}
impl ExactParsedPlane for Visitor {
    fn descriptor(&self) -> ParsedPlaneDescriptor {
        self.original.descriptor()
    }
    fn visit_native(&self, control: &Interrupt<'_>, visit: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        self.visits.fetch_add(1, Ordering::SeqCst);
        let stride = self.original.channels * self.original.sample.source_bytes();
        match self.fault {
            Fault::None => self.original.visit_native(control, visit),
            Fault::Short => visit(&self.original.bytes[..self.original.bytes.len() - stride]),
            Fault::Extra => {
                self.original.visit_native(control, visit)?;
                visit(&self.original.bytes[..stride])
            }
            Fault::Misaligned => visit(&self.original.bytes[..stride - 1]),
            Fault::Oversized => visit(&vec![0; stride * 65]),
            Fault::Error => Err(IoError::Unsupported("intentional exact visitor failure".into())),
            Fault::FinalCancel => {
                self.original.visit_native(control, visit)?;
                self.cancel.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
    }
}
fn visitor_file(fault: Fault) -> (XcfFile<Visitor>, Arc<AtomicU64>, Arc<AtomicBool>) {
    let visits = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let visitor = Visitor { original: plane(Model::Rgb, Sample::F32, false), fault, visits: visits.clone(), cancel: cancel.clone() };
    (file(visitor, Model::Rgb, Sample::F32, 650, Encoding::NonlinearProfile, false, "gimp:threshold-alpha"), visits, cancel)
}
#[test]
fn exact_non_eager_visitor_has_same_success_and_malformed_streams_are_typed_failures() {
    let (file, count, _) = visitor_file(Fault::None);
    assert!(qualify(&file, "gimp:threshold-alpha", false, false, &Interrupt::NONE, None).unwrap());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    for fault in [Fault::Short, Fault::Extra, Fault::Misaligned, Fault::Oversized, Fault::Error] {
        let (file, _, _) = visitor_file(fault);
        assert!(qualify(&file, "gimp:threshold-alpha", false, false, &Interrupt::NONE, None).is_err());
    }
}

#[test]
fn actual_cancel_work_quota_and_final_visitor_fence_propagate_without_positive_receipt() {
    let (file, count, flag) = visitor_file(Fault::None);
    flag.store(true, Ordering::SeqCst);
    let stop = || flag.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(qualify(&file, "gimp:threshold-alpha", false, false, &interrupt, None), Err(IoError::Cancelled)));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 40)]).unwrap();
    let budget = photocraft_xcf::Budget::new(limits, None);
    assert!(qualify(&file, "gimp:threshold-alpha", false, false, &Interrupt::NONE, Some(&budget)).is_err());
    assert_eq!(budget.used(photocraft_xcf::BudgetKind::Work), 40);
    let (file, _, flag) = visitor_file(Fault::FinalCancel);
    let stop = || flag.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(qualify(&file, "gimp:threshold-alpha", false, false, &interrupt, None), Err(IoError::Cancelled)));
}

fn legacy_oracle(file: &XcfFile, operation: &str, balance_depth: bool, exposure_geometry: bool) -> bool {
    let canvas = if exposure_geometry { (295, 160) } else { (8, 6) };
    let drawable = if exposure_geometry { (257, 129) } else { (8, 6) };
    let wide = matches!(
        operation,
        "gegl:brightness-contrast"
            | "gegl:levels"
            | "gegl:exposure"
            | "gimp:color-balance"
            | "gegl:color-temperature"
            | "gimp:desaturate"
            | "gegl:saturation"
            | "gegl:sepia"
            | "gegl:alien-map"
            | "gimp:threshold"
            | "gimp:threshold-alpha"
    );
    let width = match (file.precision, file.sample, file.encoding) {
        (100, Sample::U8, Encoding::Linear) | (150, Sample::U8, Encoding::NonlinearProfile) | (175, Sample::U8, Encoding::Perceptual) if balance_depth => 1,
        (300, Sample::U32, Encoding::Linear) | (350, Sample::U32, Encoding::NonlinearProfile) | (375, Sample::U32, Encoding::Perceptual) if balance_depth => 4,
        (500, Sample::F16, Encoding::Linear) | (550, Sample::F16, Encoding::NonlinearProfile) | (575, Sample::F16, Encoding::Perceptual) if balance_depth => 4,
        (275, Sample::U16, Encoding::Perceptual) if balance_depth => 2,
        (675, Sample::F32, Encoding::Perceptual) if balance_depth => 4,
        (775, Sample::F64, Encoding::Perceptual) if balance_depth => 8,
        (250, Sample::U16, Encoding::NonlinearProfile) if !balance_depth => 2,
        (600, Sample::F32, Encoding::Linear) if !balance_depth => 4,
        (750, Sample::F64, Encoding::NonlinearProfile) if !balance_depth => 8,
        (200, Sample::U16, Encoding::Linear) if wide && !balance_depth => 2,
        (650, Sample::F32, Encoding::NonlinearProfile) if wide && !balance_depth => 4,
        (700, Sample::F64, Encoding::Linear) if wide && !balance_depth => 8,
        _ => return false,
    };
    file.version == 22
        && (file.model == Model::Rgb
            || wide
                && !matches!(operation, "gimp:color-balance" | "gimp:desaturate" | "gimp:threshold" | "gegl:sepia" | "gegl:alien-map")
                && file.model == Model::Gray)
        && file.icc.is_empty()
        && (file.width, file.height) == canvas
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == file.model
                && (d.width, d.height) == drawable
                && if exposure_geometry { matches!(d.offset, (-13, 13) | (17, -11)) } else { d.offset == (0, 0) }
                && d.mode == 28
                && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && d.mask.is_none()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.floating_attachment.is_none()
                && d.effects.len() == 1
                && d.effects.first().is_some_and(|effect| {
                    effect.operation == operation
                        && (operation != "gegl:saturation"
                            || effect.arguments.iter().any(|argument| {
                                argument.name == "colorspace"
                                    && (matches!(argument.value, ArgumentValue::Enum(0 | 1))
                                        || argument.value == ArgumentValue::Enum(2)
                                            && (file.model == Model::Gray || matches!(file.sample, Sample::U16 | Sample::F32)))
                            }))
                })
                && d.plane.as_ref().is_some_and(|plane| {
                    plane.sample == if file.sample == Sample::F16 { Sample::F32 } else { file.sample }
                        && plane.channels == if file.model == Model::Gray { 2 } else { 4 }
                        && plane.width == drawable.0
                        && plane.height == drawable.1
                        && plane.bytes.len() == drawable.0 as usize * drawable.1 as usize * plane.channels * width
                        && plane.bytes.chunks_exact(width).all(|bytes| match file.sample {
                            Sample::U8 | Sample::U16 | Sample::U32 => true,
                            Sample::F16 | Sample::F32 => {
                                let Ok(raw) = <[u8; 4]>::try_from(bytes) else { return false };
                                let value = f32::from_ne_bytes(raw);
                                value.is_finite() && (0.0..=1.0).contains(&value) && (file.sample != Sample::F16 || exact_half_unit_f32(value))
                            }
                            Sample::F64 => {
                                let Ok(raw) = <[u8; 8]>::try_from(bytes) else { return false };
                                let value = f64::from_ne_bytes(raw);
                                value.is_finite() && (0.0..=1.0).contains(&value)
                            }
                        })
                })
        })
}

#[path = "precision_point_source_geometry_tests.rs"]
mod geometry;
