//! UNRUN source predicate, route and real owned-custody regressions.
use super::super::super::plane::fixture;
use super::*;
use photocraft_xcf::Effect;
fn file(op: &str, precision: u32, model: u32) -> XcfFile {
    let size = if precision == 150 { 32 } else { 16 };
    let height = if precision == 150 { 32 } else { 12 };
    let channels = if model == 0 { 4 } else { 2 };
    let word = match precision {
        150 => vec![255],
        650 => 0.5f32.to_be_bytes().to_vec(),
        750 => 0.5f64.to_be_bytes().to_vec(),
        _ => unreachable!(),
    };
    let bytes = fixture::image_custom(
        22,
        precision,
        0,
        size,
        height,
        &word.repeat(size as usize * height as usize * channels),
        &fixture::FixtureOptions { model, layer_type: Some(if model == 0 { 1 } else { 3 }), ..Default::default() },
    );
    let mut file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    let d = &mut file.layers[0];
    d.visible = true;
    d.opacity = 1.0;
    d.mode = 28;
    d.blend_space = 0;
    d.composite_space = -1;
    d.composite_mode = -1;
    d.effects.push(Effect {
        name: String::new(),
        icon: String::new(),
        operation: op.into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: None,
        state: Default::default(),
    });
    // Preserve the old finite Box raw identity, which the newer observational
    // geometry selector deliberately leaves to this exact legacy family.
    if op == "gegl:box-blur" && precision != 150 {
        let state = &mut d.effects[0].state;
        state.blend_space = Some(-1474592175);
        state.composite_space = Some(-1474592175);
        state.composite_mode = Some(-1474592175);
    }
    file
}
#[test]
fn eight_routes_keep_original_domains_and_mask_flags_separate_from_point() {
    use NeighborhoodSourceFamily::*;
    for (op, p, m, family) in [
        ("gegl:box-blur", 150, 0, BoxOriginal),
        ("gegl:box-blur", 650, 0, BoxFinite),
        ("gegl:median-blur", 150, 0, MedianOriginal),
        ("gegl:median-blur", 750, 0, MedianFinite),
        ("gegl:median-blur", 650, 1, MedianGray),
        ("gegl:motion-blur-linear", 150, 0, MotionOriginal),
        ("gegl:pixelize", 150, 0, PixelizeOriginal),
        ("gegl:gaussian-blur", 750, 0, RasterFinite),
    ] {
        let file = file(op, p, m);
        assert_eq!(NeighborhoodSourceFamily::hint(&file), Some(family));
        assert_eq!(SourceSelectedPolicy::family(&file), SelectedStagedFamily::Neighborhood(family));
        assert!(family.qualify(&file, &Interrupt::NONE, None).unwrap());
        let mut d = admission::AdmissionDomains::default();
        family.set_admission(&mut d);
        assert!(d.owned_neighborhood_domain == Some(family.domain()));
        assert!(d.point_replace_domain.is_none());
        assert!(d.owned_point_domain.is_none());
        assert!(
            d.finite_box_domain
                || d.finite_gaussian_domain
                || d.finite_median_domain
                || d.gray_median_domain
                || d.opaque_box_domain
                || d.opaque_median_domain
                || d.opaque_motion_domain
                || d.opaque_pixelize_domain
        );
    }
    let original = file("gegl:gaussian-blur", 150, 0);
    assert_eq!(NeighborhoodSourceFamily::hint(&original), None);
}
#[test]
fn bounded_native_predicates_match_old_scans_and_keep_hdr_alpha_and_profile_intersection() {
    for (op, family) in [
        ("gegl:box-blur", NeighborhoodSourceFamily::BoxOriginal),
        ("gegl:median-blur", NeighborhoodSourceFamily::MedianOriginal),
        ("gegl:motion-blur-linear", NeighborhoodSourceFamily::MotionOriginal),
        ("gegl:pixelize", NeighborhoodSourceFamily::PixelizeOriginal),
    ] {
        let mut f = file(op, 150, 0);
        assert!(super::super::super::qualified_opaque_neighborhood(&f, op));
        assert!(family.qualify(&f, &Interrupt::NONE, None).unwrap());
        f.layers[0].plane.as_mut().unwrap().bytes[3] = 254;
        assert!(!super::super::super::qualified_opaque_neighborhood(&f, op));
        assert!(!family.qualify(&f, &Interrupt::NONE, None).unwrap());
    }
    for (op, model, family) in [
        ("gegl:box-blur", Model::Rgb, NeighborhoodSourceFamily::BoxFinite),
        ("gegl:median-blur", Model::Rgb, NeighborhoodSourceFamily::MedianFinite),
        ("gegl:median-blur", Model::Gray, NeighborhoodSourceFamily::MedianGray),
        ("gegl:gaussian-blur", Model::Rgb, NeighborhoodSourceFamily::RasterFinite),
    ] {
        for p in [650, 750] {
            let mut f = file(op, p, if model == Model::Gray { 1 } else { 0 });
            let word = if p == 750 { (-2f64).to_ne_bytes().to_vec() } else { (-2f32).to_ne_bytes().to_vec() };
            f.layers[0].plane.as_mut().unwrap().bytes[..word.len()].copy_from_slice(&word);
            assert!(super::super::super::qualified_finite_raster_filter(&f, op, model));
            assert!(family.qualify(&f, &Interrupt::NONE, None).unwrap());
            let width = word.len();
            let alpha = (if model == Model::Gray { 2 } else { 4 }) * width - width;
            let bad = if p == 750 { 1.5f64.to_ne_bytes().to_vec() } else { 1.5f32.to_ne_bytes().to_vec() };
            f.layers[0].plane.as_mut().unwrap().bytes[alpha..alpha + width].copy_from_slice(&bad);
            assert!(!super::super::super::qualified_finite_raster_filter(&f, op, model));
            assert!(!family.qualify(&f, &Interrupt::NONE, None).unwrap());
            f.layers[0].plane.as_mut().unwrap().bytes[alpha..alpha + width].copy_from_slice(&if p == 750 {
                0.5f64.to_ne_bytes().to_vec()
            } else {
                0.5f32.to_ne_bytes().to_vec()
            });
            f.icc.push(1);
            assert!(super::super::super::qualified_finite_raster_filter(&f, op, model));
            assert!(!family.qualify(&f, &Interrupt::NONE, None).unwrap());
        }
    }
}
#[test]
fn hostile_selector_and_work_cancel_and_complete_native_length_remain_bounded() {
    let mut f = file("gegl:box-blur", 650, 0);
    let arg = photocraft_xcf::Argument { name: "radius".into(), property_index: 0, value: ArgumentValue::Integer(4), configuration: None, graph: None };
    f.layers[0].effects[0].arguments = vec![arg; 100000];
    assert_eq!(NeighborhoodSourceFamily::hint(&f), None);
    assert!(!NeighborhoodSourceFamily::BoxFinite.qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].effects[0].arguments.clear();
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(NeighborhoodSourceFamily::BoxFinite.qualify(&f, &Interrupt::NONE, Some(&budget)).is_err());
    let stop = || true;
    let ctl = Interrupt::cancel_only(&stop);
    assert!(matches!(NeighborhoodSourceFamily::BoxFinite.qualify(&f, &ctl, None), Err(IoError::Cancelled)));
    f.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(!NeighborhoodSourceFamily::BoxFinite.qualify(&f, &Interrupt::NONE, None).unwrap());
}

#[cfg(feature = "corpus")]
mod originals {
    use super::*;
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    use sha2::{Digest, Sha256};
    fn hash(b: &[u8]) -> String {
        Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
    }
    #[test]
    #[ignore = "UNRUN: eight SHA-pinned original sources through one selected paid completion/native promotion"]
    fn actual_eight_sources_keep_source_and_native_storage_on_the_same_quota() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let fixtures = [
            ("operations/gegl-box-blur/operation-gegl-box-blur.xcf", "0dddbdcee491ce0e24aea521ce7c1de4fa0e5304a2caa9021933917a1de63b15"),
            ("box-raster-color-evidence/implicit-hdr-f64.source.xcf", "11c8158edbab33226b51c201ff167a34eeef52c53efeff601cab242ced6abdc6"),
            ("operations/gegl-median-blur/operation-gegl-median-blur.xcf", "31fcb8d83bb6e574a9dddb8b1e4222de1fa2642a34202696c29e3effa6cbe746"),
            ("median-raster-color-evidence/implicit-hdr-f64.source.xcf", "8f8001f3f31d6b9936f659ab18965a8df57f9e403297497f1d339e3539fe4442"),
            ("median-gray-color-evidence/implicit-gray-hdr-f64.source.xcf", "1149bcbe3cadc2075f4c7546a7eeaa0197f07677ce882a6dc75835483cd57cc7"),
            ("operations/gegl-motion-blur-linear/operation-gegl-motion-blur-linear.xcf", "7fd61309e9ad01f397b5127f0b51f2e2fae27f9d81c53c56305f02b6d24413e8"),
            ("operations/gegl-pixelize/operation-gegl-pixelize.xcf", "a4b746de40767cbd07467cad702cda4611c95e614c718164696ef475b2fe1f34"),
            ("gaussian-raster-color-evidence/implicit-hdr-f64.source.xcf", "e9a17b3deb40ecd066f09a0888fe5332cb54c542afae8ee0be8700ac6bc80d46"),
        ];
        for (path, pin) in fixtures {
            // Original byte Vec/eager reference are external oracle captures, not
            // claimed product-accounted storage or authority.
            let bytes = std::fs::read(root.join(path)).unwrap();
            assert_eq!(hash(&bytes), pin);
            let eager = crate::import(path, &bytes).unwrap().document;
            let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
            let baseline = q.usage().unwrap();
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
                .unwrap()
                .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
                .unwrap();
            let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
            let starts = std::sync::atomic::AtomicU64::new(0);
            let progress = |at: f32| {
                if at.to_bits() == 0.05f32.to_bits() {
                    starts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            };
            let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
            let imported = crate::import_owned_xcf_bytes_in_quota(path, Arc::new(bytes.clone()), &options, &limits, q.clone(), &control).unwrap();
            assert_eq!(starts.load(std::sync::atomic::Ordering::Relaxed), 2);
            let crate::OwnedXcfImport::Point(imported) = imported else { panic!("same structural typed holder") };
            let doc = imported.document();
            assert!(doc.is_readonly_point_storage());
            assert!(doc.quota().shares_ledger(&q));
            let current = doc.document().xcf_state.as_ref().unwrap();
            let old = eager.xcf_state.as_ref().unwrap();
            assert_eq!(current.current_filter_output_precision, old.current_filter_output_precision);
            assert_eq!(current.filter_stacks[0].nodes[0].operation, old.filter_stacks[0].nodes[0].operation);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, old.filter_stacks[0].nodes[0].arguments);
            assert_eq!(current.filter_stacks[0].nodes[0].composition, old.filter_stacks[0].nodes[0].composition);
            assert_eq!(current.filter_masks.len(), old.filter_masks.len());
            for (a, b) in current.filter_masks.iter().zip(&old.filter_masks) {
                assert_eq!(a.bounds, b.bounds);
                assert_eq!(a.surface, b.surface);
            }
            assert_eq!(current.opaque.len(), old.opaque.len());
            for (a, b) in current.opaque.iter().zip(&old.opaque) {
                assert_eq!(a.namespace, b.namespace);
                assert_eq!(a.identity, b.identity);
                assert_eq!(a.bytes, b.bytes);
                assert_eq!(a.blake3, b.blake3);
                assert_eq!(a.flags, b.flags);
                assert_eq!(a.order, b.order);
            }

            assert_eq!(doc.document().source_color_context, eager.source_color_context);
            assert_eq!(doc.document().icc_profile, eager.icc_profile);
            let LayerContent::BackedRaster(layer) = &doc.document().layers[0].content else { panic!("actual paid native store") };
            let expected = eager.layers[0].surface().unwrap().try_read_interleaved(eager.bounds()).unwrap();
            let mut actual = vec![0; expected.len()];
            let view = layer.checked_view();
            let bpp = view.format().bytes_per_pixel();
            for y in 0..doc.document().size.height {
                for x in 0..doc.document().size.width {
                    let at = (y as usize * doc.document().size.width as usize + x as usize) * bpp;
                    view.read_logical_pixel_native_into(x as i32, y as i32, &mut actual[at..at + bpp], &control).unwrap();
                }
            }
            assert_eq!(actual, expected);
            assert!(imported.source_read_only());
            assert_eq!(hash(&bytes), pin);
            drop(imported);
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
}
