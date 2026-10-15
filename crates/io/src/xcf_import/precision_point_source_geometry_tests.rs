//! UNRUN dimension-only sealed Threshold Alpha source qualification drafts.
use super::*;

fn sized(model: Model, sample: Sample, precision: u32, encoding: Encoding, width: u32, height: u32) -> XcfFile {
    let mut f = file(plane(model, sample, false), model, sample, precision, encoding, false, "gimp:threshold-alpha");
    f.width = width;
    f.height = height;
    let d = &mut f.layers[0];
    d.width = width;
    d.height = height;
    let p = d.plane.as_mut().unwrap();
    let stride = p.channels * p.sample.source_bytes();
    p.bytes = p.bytes[..stride].repeat(width as usize * height as usize);
    p.width = width;
    p.height = height;
    f
}

#[test]
fn only_private_threshold_geometry_accepts_dynamic_canvas_with_exact_native_stream() {
    for model in [Model::Rgb, Model::Gray] {
        for (sample, precision, encoding) in [
            (Sample::U16, 200, Encoding::Linear),
            (Sample::U16, 250, Encoding::NonlinearProfile),
            (Sample::F32, 600, Encoding::Linear),
            (Sample::F32, 650, Encoding::NonlinearProfile),
            (Sample::F64, 700, Encoding::Linear),
            (Sample::F64, 750, Encoding::NonlinearProfile),
        ] {
            for (width, height) in [(1, 1), (257, 3), (3, 257), (8, 6)] {
                let f = sized(model, sample, precision, encoding, width, height);
                let raw = f.layers[0].plane.as_ref().unwrap().bytes.clone();
                assert!(qualify_threshold_geometry(&f, &Interrupt::NONE, None).unwrap());
                assert_eq!(qualify(&f, "gimp:threshold-alpha", false, false, &Interrupt::NONE, None).unwrap(), (width, height) == (8, 6));
                assert_eq!(f.layers[0].plane.as_ref().unwrap().bytes, raw);
                let mut other = f.clone();
                other.layers[0].effects[0].operation = "gegl:levels".into();
                assert!(!qualify_threshold_geometry(&other, &Interrupt::NONE, None).unwrap());
                assert_eq!(qualify(&other, "gegl:levels", false, false, &Interrupt::NONE, None).unwrap(), (width, height) == (8, 6));
            }
        }
    }
}

#[test]
fn geometry_refuses_bad_dimensions_offsets_profiles_descriptors_and_stream_lengths() {
    let base = sized(Model::Rgb, Sample::F64, 750, Encoding::NonlinearProfile, 257, 3);
    for change in [
        (|f: &mut XcfFile| f.width = 0) as fn(&mut XcfFile),
        |f| f.height = 262145,
        |f| {
            f.width = 262144;
            f.height = 4097;
        },
        |f| f.layers[0].offset = (-1, 0),
        |f| f.layers[0].width += 1,
        |f| f.layers[0].plane.as_mut().unwrap().width += 1,
        |f| f.icc.push(1),
    ] {
        let mut f = base.clone();
        change(&mut f);
        assert!(!qualify_threshold_geometry(&f, &Interrupt::NONE, None).unwrap());
    }
    for extra in [false, true] {
        let mut f = base.clone();
        let bytes = &mut f.layers[0].plane.as_mut().unwrap().bytes;
        if extra {
            bytes.extend_from_slice(&[0; 32]);
        } else {
            bytes.truncate(bytes.len() - 32);
        }
        assert!(qualify_threshold_geometry(&f, &Interrupt::NONE, None).is_err());
    }
}

#[test]
fn dynamic_qualification_uses_one_cumulative_budget_and_final_cancellation_fence() {
    let f = sized(Model::Gray, Sample::F32, 650, Encoding::NonlinearProfile, 257, 3);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    let counted = photocraft_xcf::Budget::new(limits.clone(), None);
    assert!(qualify_threshold_geometry(&f, &Interrupt::NONE, Some(&counted)).unwrap());
    let work = counted.used(photocraft_xcf::BudgetKind::Work);
    let limited = photocraft_xcf::Budget::new(limits.lowered(&[("sampleWorkUnits", work + 31)]).unwrap(), None);
    assert!(qualify_threshold_geometry(&f, &Interrupt::NONE, Some(&limited)).unwrap());
    assert!(qualify_threshold_geometry(&f, &Interrupt::NONE, Some(&limited)).is_err());
    assert_eq!(limited.used(photocraft_xcf::BudgetKind::Work), work);
    let (f, _, stop) = visitor_file(Fault::FinalCancel);
    let cancelled = || stop.load(Ordering::SeqCst);
    assert!(matches!(qualify_threshold_geometry(&f, &Interrupt::cancel_only(&cancelled), None), Err(IoError::Cancelled)));
}
