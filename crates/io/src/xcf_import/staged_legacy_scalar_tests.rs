//! UNRUN bounded legacy scalar identity/native and old-mask-path drafts.
use super::super::super::plane::fixture;
use super::*;
use photocraft_xcf::Effect;
fn file(operation: &str, precision: u32, size: (u32, u32), rgba: &[u8]) -> XcfFile {
    let b = fixture::image(22, precision, 0, size.0, size.1, &rgba.repeat(size.0 as usize * size.1 as usize));
    let mut f = photocraft_xcf::decode(&b, &DecodeOptions::default()).unwrap();
    let d = &mut f.layers[0];
    d.visible = true;
    d.opacity = 1.;
    d.mode = 28;
    d.blend_space = 0;
    d.composite_space = -1;
    d.composite_mode = -1;
    d.effects.push(Effect {
        name: String::new(),
        icon: String::new(),
        operation: operation.into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: None,
        state: Default::default(),
    });
    f
}
#[test]
fn hdr_alpha_bits_and_opaque_originals_preserve_old_source_boundaries() {
    let pixel = [-2.0f32, 3., 0.5, 1.].map(f32::to_be_bytes).concat();
    let mut f = file("gegl:rgb-clip", 600, (8, 6), &pixel);
    assert!(super::super::super::qualified_hdr_clip(&f));
    assert!(LegacyScalarFamily::RgbClipHdr.qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&0.999f32.to_ne_bytes());
    assert!(!super::super::super::qualified_hdr_clip(&f));
    assert!(!LegacyScalarFamily::RgbClipHdr.qualify(&f, &Interrupt::NONE, None).unwrap());
    for (name, raw, expected) in [
        ("gegl:mono-mixer", -261651887, LegacyScalarFamily::MonoOriginal),
        ("gegl:mono-mixer", 107971153, LegacyScalarFamily::MonoEdits),
        ("gegl:value-invert", -261651887, LegacyScalarFamily::ValueOriginal),
    ] {
        let mut f = file(name, 150, (32, 32), &[31, 127, 191, 255]);
        let e = &mut f.layers[0].effects[0];
        e.state.blend_space = Some(raw);
        e.state.composite_space = Some(raw);
        e.state.composite_mode = Some(raw);
        assert_eq!(LegacyScalarFamily::hint(&f), Some(expected));
        assert_eq!(SourceSelectedPolicy::family(&f), SelectedStagedFamily::LegacyScalar(expected));
        assert!(super::super::super::qualified_opaque_neighborhood(&f, name));
        assert!(expected.qualify(&f, &Interrupt::NONE, None).unwrap());
        f.layers[0].plane.as_mut().unwrap().bytes[3] = 254;
        assert!(!expected.qualify(&f, &Interrupt::NONE, None).unwrap());
        let mut domains = admission::LateAdmissionDomains::default();
        expected.set_late(&mut domains);
        assert_eq!(domains.mono_mixer_edits_domain, expected == LegacyScalarFamily::MonoEdits);
    }
}
#[test]
fn legacy_hint_is_bounded_and_real_native_work_cancellation_is_not_refunded() {
    let pixel = [-2.0f32, 3., 0.5, 1.].map(f32::to_be_bytes).concat();
    let f = file("gegl:rgb-clip", 600, (8, 6), &pixel);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(LegacyScalarFamily::RgbClipHdr.qualify(&f, &Interrupt::NONE, Some(&budget)).is_err());
    assert_eq!(budget.used(photocraft_xcf::BudgetKind::Work), 32);
    let stop = || true;
    let control = Interrupt::cancel_only(&stop);
    assert!(matches!(LegacyScalarFamily::RgbClipHdr.qualify(&f, &control, None), Err(IoError::Cancelled)));
    let mut f = file("gegl:value-invert", 150, (32, 32), &[0, 0, 0, 255]);
    let argument =
        photocraft_xcf::Argument { name: "hostile".into(), property_index: 0, value: ArgumentValue::Boolean(false), configuration: None, graph: None };
    f.layers[0].effects[0].arguments.resize(100000, argument);
    assert_eq!(LegacyScalarFamily::hint(&f), None);
}

#[test]
#[ignore = "requires exact independently authored six scalar originals; UNRUN"]
fn six_pinned_sources_keep_old_qualifiers_distinct_epochs_and_owned_catalog() {
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let fixtures = [
        (
            "exposure-geometry-evidence/rgb-double_non_linear.negative-x.source.xcf",
            "cc29e9aaae63f97f4f1e01ad49fabc9759b165e9f1496bc70edf728605acf53b",
            LegacyScalarFamily::ExposureGeometry,
        ),
        ("rgb-clip-hdr-evidence/clip.source.xcf", "33dce24ca99ee3ae8c60b05eb16398498099e7353e7e1698a2efde2e3b3d02a4", LegacyScalarFamily::RgbClipHdr),
        (
            "point-unpremultiply-evidence/unpremultiply-rgb-float_linear.small.source.xcf",
            "21e4faa0915abbf9f0a89ac4c80584392f98849514d8b870ef9db0deb1a9cb79",
            LegacyScalarFamily::Unpremultiply,
        ),
        (
            "operations/gegl-mono-mixer/operation-gegl-mono-mixer.xcf",
            "dc7009be42d698b448efd45c6b38a18f80a816a57033a44c031e6fa1d8d5880a",
            LegacyScalarFamily::MonoOriginal,
        ),
        ("mono-mixer-edits-evidence/mono.source.xcf", "32151370e3768e004248e01a59576150ef000572e992d48e52667ff3cb1709b7", LegacyScalarFamily::MonoEdits),
        (
            "operations/gegl-value-invert/operation-gegl-value-invert.xcf",
            "9cd1650e6ddfaba7f3dd8dc9e5ae5ac2cec46b962685973dbe687b6235f6916f",
            LegacyScalarFamily::ValueOriginal,
        ),
    ];
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    for (path, sha, family) in fixtures {
        let bytes = Arc::new(std::fs::read(root.join(path)).unwrap());
        assert_eq!(Sha256::digest(&*bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), sha);
        let f = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
        assert!(family.qualify(&f, &Interrupt::NONE, None).unwrap());
        let old = match family {
            LegacyScalarFamily::ExposureGeometry => super::super::super::qualified_precision_point_impl(&f, "gegl:exposure", false, true),
            LegacyScalarFamily::RgbClipHdr => super::super::super::qualified_hdr_clip(&f),
            LegacyScalarFamily::Unpremultiply => super::super::super::unpremultiply::qualifies(&f, &Interrupt::NONE, None).unwrap(),
            LegacyScalarFamily::MonoOriginal | LegacyScalarFamily::MonoEdits => super::super::super::qualified_opaque_neighborhood(&f, "gegl:mono-mixer"),
            LegacyScalarFamily::ValueOriginal => super::super::super::qualified_opaque_neighborhood(&f, "gegl:value-invert"),
        };
        assert!(old);
        let eager = crate::import("old.xcf", &bytes).unwrap().document;
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let baseline = q.usage().unwrap();
        let selected = super::super::super::owned_xcf_source::import_owned_xcf_bytes_in_quota(
            "owned.xcf",
            bytes.clone(),
            &DecodeOptions::default(),
            &limits,
            q.clone(),
            &ctl,
        )
        .unwrap();
        assert!(matches!(&selected, super::super::super::owned_xcf_source::OwnedXcfImport::Point(_)));
        assert!(selected.document().quota().shares_ledger(&q));
        let current = selected.document().document();
        let state = current.xcf_state.as_ref().unwrap();
        let original = eager.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks[0].nodes[0].operation, original.filter_stacks[0].nodes[0].operation);
        assert_eq!(state.filter_stacks[0].nodes[0].arguments, original.filter_stacks[0].nodes[0].arguments);
        assert_eq!(state.filter_stacks[0].nodes[0].composition, original.filter_stacks[0].nodes[0].composition);
        assert_eq!(state.current_filter_output_precision, original.current_filter_output_precision);
        let LayerContent::BackedRaster(layer) = &current.layers[0].content else { panic!() };
        assert_eq!(layer.operative_bounds(), bounds(&f.layers[0]).unwrap());
        let p = f.layers[0].plane.as_ref().unwrap();
        let mut native = vec![0; p.bytes.len()];
        let bpp = layer.format().bytes_per_pixel();
        let rect = layer.operative_bounds();
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let at = ((y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize) * bpp;
                layer.checked_view().read_logical_pixel_native_into(x, y, &mut native[at..at + bpp], &ctl).unwrap();
            }
        }
        assert_eq!(native, p.bytes);
        drop(selected);
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(Sha256::digest(&*bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), sha);
    }
}
