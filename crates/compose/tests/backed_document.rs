//! Native Document consumer regression tests.
//! Independent equation/error tests for the first flat Normal/Union consumer.
use photocraft_cms::Builtin;
use photocraft_color::{ColorMode, PixelFormat, SampleType, blend::BlendMode, try_write_sample_f64};
use photocraft_compose::{
    backed_document::PreparedBackedDocument,
    backed_normal::{BackedNormalError, NormalBandLimits},
};
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent, SampleEncoding, WorkingProfileInterpretation};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 0, index: 1 << 20 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: AtomicU64,
    fail: AtomicBool,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        for (i, b) in output.iter_mut().enumerate() {
            *b = self.descriptor.default_bytes()[((offset + i as u64) % self.descriptor.format().bytes_per_pixel() as u64) as usize];
        }
        Ok(())
    }
}
fn layer(q: &BackingQuota, mode: ColorMode, values: &[f64], rect: Rect) -> (Layer, Arc<Store>) {
    let format = PixelFormat::new(mode, SampleType::F64, true);
    let mut bytes = vec![0; format.bytes_per_pixel()];
    for (i, value) in values.iter().enumerate() {
        try_write_sample_f64(&mut bytes, SampleType::F64, i, *value).unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, rect, &bytes).unwrap();
    let store = Arc::new(Store { descriptor, reads: AtomicU64::new(0), fail: AtomicBool::new(false) });
    let raster = BackedRaster::new(store.clone(), q.clone(), 1, &control()).unwrap();
    (Layer::new("owned finite", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap())), store)
}
fn document(mode: ColorMode, layers: Vec<Layer>) -> Document {
    let mut doc = Document::new("checked finite", Size::new(4, 2), mode, SampleType::F64);
    doc.layers = layers;
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::SGray }.profile().to_bytes());
    doc
}
#[test]
fn explicit_document_bottom_to_top_negative_extents_match_independent_normal_equation() {
    let q = quota();
    let (bottom, bs) = layer(&q, ColorMode::Rgb, &[1., 0., 0., 0.5], Rect::new(-1, 0, 2, 1));
    let (mut top, ts) = layer(&q, ColorMode::Rgb, &[0., 1., 0., 0.5], Rect::new(0, 0, 3, 1));
    top.opacity = 0.5;
    let doc = document(ColorMode::Rgb, vec![bottom, top]);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    assert_eq!(bs.reads.load(Ordering::Relaxed) + ts.reads.load(Ordering::Relaxed), 0, "preparation read source pixels");
    let output = prepared.render_band(Rect::new(-2, 0, 4, 1), &control()).unwrap();
    // Coverage=.25+.5*(1-.25)=.625; RGB is straight alpha after Normal.
    let expected = [[0.; 4], [1., 0., 0., 0.5], [0.6, 0.4, 0., 0.625], [0.6, 0.4, 0., 0.625], [0., 1., 0., 0.25], [0.; 4]];
    for (got, want) in output.pixels().iter().zip(expected) {
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 1.0e-14, "{got:?} != {want:?}");
        }
    }
    drop((output, prepared));
    drop((doc, bs, ts));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn hidden_unsupported_semantics_and_mixed_resident_content_refuse_before_io() {
    let q = quota();
    let (source, store) = layer(&q, ColorMode::Rgb, &[0.2, 0.4, 0.6, 1.], Rect::new(0, 0, 2, 1));
    let doc = document(ColorMode::Rgb, vec![source]);
    let before = q.usage().unwrap();
    for kind in 0..8 {
        let mut current = doc.clone();
        current.layers[0].visible = false;
        match kind {
            0 => current.layers[0].blend = BlendMode::Multiply,
            1 => current.layers[0].opacity = f32::NAN,
            2 => current.layers[0].fill_opacity = 0.5,
            3 => current.layers[0].clipped = true,
            4 => current.sample_encoding = None,
            5 => current.icc_profile = Some(Builtin::SGray.profile().to_bytes()),
            6 => current.layers.push(Layer::raster("resident", PixelFormat::RGBA32F)),
            _ => current.layers[0].excluded_channels = 1,
        }
        assert!(PreparedBackedDocument::new(&current, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).is_err());
        assert_eq!(store.reads.load(Ordering::Relaxed), 0);
        assert_eq!(q.usage().unwrap(), before);
    }
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn late_second_layer_io_never_delivers_first_layer_and_preserves_source() {
    let q = quota();
    let (first, fs) = layer(&q, ColorMode::Rgb, &[1., 0., 0., 0.5], Rect::new(0, 0, 2, 1));
    let (last, ls) = layer(&q, ColorMode::Rgb, &[0., 1., 0., 0.5], Rect::new(0, 0, 2, 1));
    let doc = document(ColorMode::Rgb, vec![first, last]);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    ls.fail.store(true, Ordering::Relaxed);
    let before = q.usage().unwrap();
    assert!(matches!(prepared.render_band(Rect::new(0, 0, 2, 1), &control()), Err(BackedNormalError::View(_))));
    assert!(fs.reads.load(Ordering::Relaxed) > 0 && ls.reads.load(Ordering::Relaxed) > 0);
    // Caches may retain verified reads; no returned partial output exists.
    assert!(q.usage().unwrap().resident >= before.resident);
    drop(prepared);
    drop((doc, fs, ls));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn gray_current_profile_repetition_and_work_failures_release_derived_owners() {
    let q = quota();
    let (source, store) = layer(&q, ColorMode::Grayscale, &[0.3, 0.5], Rect::new(-1, 0, 2, 1));
    let doc = document(ColorMode::Grayscale, vec![source]);
    let before = q.usage().unwrap();
    let exhausted = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(
        PreparedBackedDocument::new(&doc, Builtin::SGray.profile(), NormalBandLimits::default(), &exhausted),
        Err(BackedNormalError::Backing(BackingError::Work))
    ));
    assert_eq!(q.usage().unwrap(), before);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::SGray.profile(), NormalBandLimits::default(), &control()).unwrap();
    assert!(matches!(prepared.render_band(Rect::new(0, 0, 2, 1), &exhausted), Err(BackedNormalError::Backing(BackingError::Work))));
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    let band = prepared.render_band(Rect::new(0, 0, 2, 1), &control()).unwrap();
    for p in band.pixels() {
        assert_eq!(p, &[0.3, 0.3, 0.3, 0.5]);
    }
    drop((band, prepared));
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn ordinary_public_consumers_refuse_hidden_nested_backing_before_empty_output_or_reads() {
    use photocraft_compose as compose;
    let q = quota();
    let (mut source, store) = layer(&q, ColorMode::Rgb, &[0.2, 0.4, 0.6, 1.], Rect::new(0, 0, 2, 1));
    source.visible = false;
    let nested = Layer::group("hidden owned pixels", vec![source]);
    let doc = document(ColorMode::Rgb, vec![nested]);
    let before = q.usage().unwrap();
    let error = photocraft_raster::RasterInputError::UnsupportedConversion;
    assert_eq!(compose::validate_resident_document(&doc), Err(error));
    assert_eq!(compose::render(&doc, Rect::EMPTY), Err(error));
    assert_eq!(compose::render_tiled(&doc, Rect::EMPTY, 1), Err(error));
    assert_eq!(compose::flatten(&doc), Err(error));
    assert_eq!(compose::flatten_to_surface(&doc, PixelFormat::RGBA32F, None), Err(error));
    assert_eq!(compose::render_layer(&doc.layers[0], Rect::EMPTY), Err(error));
    assert_eq!(compose::thumbnail(&doc, 1), Err(error));
    assert_eq!(compose::thumbnail_buffer(&doc, 1), Err(error));
    assert_eq!(compose::render_reduced(&doc, 1, 1), Err(error));
    assert_eq!(compose::render_reduced_rect(&doc, Rect::EMPTY, 1, 1), Err(error));
    assert_eq!(compose::render_reduced_damage(&doc, 1, 1, Rect::EMPTY), Err(error));
    assert_eq!(compose::proxy::proxy_document(&doc, 1), Err(error));
    assert_eq!(compose::layer_shape(&doc, &doc.layers[0], Rect::EMPTY), Err(error));
    assert!(matches!(compose::PreparedResidentRegion::new(&doc, Rect::EMPTY), Err(e) if e==error));
    let mut delivered = 0;
    assert_eq!(
        compose::render_bands(&doc, Rect::EMPTY, 1, |_| {
            delivered += 1;
            Ok::<_, ()>(())
        }),
        Err(compose::RenderBandError::Render(error))
    );
    assert_eq!(compose::render_with_interrupt(&doc, Rect::EMPTY, &Interrupt::NONE), Err(error));
    assert_eq!(
        compose::render_bands_with_interrupt(&doc, Rect::EMPTY, 1, &Interrupt::NONE, |_| {
            delivered += 1;
            Ok(())
        }),
        Err(error)
    );
    assert_eq!(compose::render_reduced_with_interrupt(&doc, 1, 1, &Interrupt::NONE), Err(error));
    assert_eq!(compose::thumbnail_with_interrupt(&doc, 1, &Interrupt::NONE), Err(error));
    assert!(compose::gimp_render::PreparedRenderer::new(&doc).is_err());
    assert_eq!(delivered, 0);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), before);
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn streaming_splits_wide_regions_and_keeps_complete_owner_charges_through_sink_failures() {
    use photocraft_compose::backed_document::BackedDocumentSinkError;
    let q = quota();
    let (source, store) = layer(&q, ColorMode::Rgb, &[0.2, 0.4, 0.6, 0.5], Rect::new(-1, 0, 7, 3));
    let doc = document(ColorMode::Rgb, vec![source]);
    let limits = NormalBandLimits { max_rows: 2, max_pixels: 4 };
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), limits, &control()).unwrap();
    let mut regions = Vec::new();
    let mut pixels = 0;
    prepared
        .render_bands(Rect::new(-1, 0, 7, 3), &control(), |band| {
            assert!(band.pixels().len() <= 4);
            assert!(q.usage().unwrap().resident > 0);
            for pixel in band.pixels() {
                assert_eq!(*pixel, [0.2, 0.4, 0.6, 0.5]);
            }
            pixels += band.pixels().len();
            regions.push(band.rect());
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(pixels, 24);
    assert_eq!(regions.len(), 9);
    assert_eq!(regions[0], Rect::new(-1, 0, 0, 1));
    assert_eq!(regions[1], Rect::new(0, 0, 4, 1));
    let error = prepared.render_bands(Rect::new(-1, 0, 7, 3), &control(), |_| Err("sink refused"));
    assert_eq!(error, Err(BackedDocumentSinkError::Sink("sink refused")));
    let mut deliveries = 0;
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert_eq!(
        prepared.render_bands(Rect::EMPTY, &work, |_| {
            deliveries += 1;
            Ok::<_, ()>(())
        }),
        Err(BackedDocumentSinkError::Render(BackedNormalError::Backing(BackingError::Work)))
    );
    assert_eq!(deliveries, 0);
    drop(prepared);
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn reduction_averages_premultiplied_canvas_samples_independent_of_band_shape() {
    let q = quota();
    // Half the canvas is covered. Empty pixels carry no color into averaging.
    let (source, store) = layer(&q, ColorMode::Rgb, &[0.2, 0.4, 0.6, 0.5], Rect::new(-1, 0, 2, 2));
    let doc = document(ColorMode::Rgb, vec![source]);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits { max_rows: 1, max_pixels: 1 }, &control()).unwrap();
    let result = prepared.render_reduced(1, 1, &control()).unwrap();
    assert_eq!(result.rect(), Rect::new(0, 0, 1, 1));
    assert_eq!(result.pixels(), &[[0.2, 0.4, 0.6, 0.25]]);
    let larger = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    let second = larger.render_reduced(1, 1, &control()).unwrap();
    assert_eq!(result.pixels(), second.pixels());
    let charged = q.usage().unwrap();
    let (transferred, lease) = second.into_parts();
    assert_eq!(transferred.px, result.pixels());
    assert_eq!(q.usage().unwrap(), charged);
    drop(transferred);
    drop(lease);
    drop((result, larger));
    let before = q.usage().unwrap();
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(prepared.render_reduced(1, 1, &work), Err(BackedNormalError::Backing(BackingError::Work))));
    assert!(matches!(prepared.render_reduced(4097, 1, &control()), Err(BackedNormalError::Invalid(_))));
    assert_eq!(q.usage().unwrap(), before);
    drop(prepared);
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn reduction_rejects_quota_before_source_reads_and_refunds_late_io_failure() {
    let q = quota();
    let (source, store) = layer(&q, ColorMode::Rgb, &[0.2, 0.4, 0.6, 0.5], Rect::new(0, 0, 4, 2));
    let mut doc = document(ColorMode::Rgb, vec![source]);
    doc.size = Size::new(4096, 4096);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(prepared.render_reduced(4096, 4096, &control()), Err(BackedNormalError::Quota(_))));
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), before);
    drop(prepared);
    doc.size = Size::new(4, 2);
    store.fail.store(true, Ordering::Relaxed);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(
        prepared.render_reduced(1, 1, &control()),
        Err(BackedNormalError::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Io)))
    ));
    assert_eq!(q.usage().unwrap(), before);
    drop(prepared);
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn reduction_propagates_unqualified_hdr_before_emitting_display_samples() {
    let q = quota();
    let (source, store) = layer(&q, ColorMode::Rgb, &[1.0e39, 0.4, 0.6, 1.0], Rect::new(0, 0, 4, 2));
    let doc = document(ColorMode::Rgb, vec![source]);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    assert!(matches!(prepared.render_band(doc.bounds(), &control()), Err(BackedNormalError::Unsupported("finite unit colors/alpha only"))));
    let before = q.usage().unwrap();
    assert!(matches!(prepared.render_reduced(1, 1, &control()), Err(BackedNormalError::Unsupported("finite unit colors/alpha only"))));
    assert_eq!(q.usage().unwrap(), before);
    drop(prepared);
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
#[ignore = "24 MP release performance probe"]
fn backed_document_reduction_24mp_release() {
    let q = quota();
    let (source, store) = layer(&q, ColorMode::Rgb, &[0.2, 0.4, 0.6, 0.5], Rect::new(0, 0, 6000, 4000));
    let mut doc = document(ColorMode::Rgb, vec![source]);
    doc.size = Size::new(6000, 4000);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    let start = std::time::Instant::now();
    let output = prepared.render_reduced(1200, 800, &control()).unwrap();
    eprintln!("backed Document 24MP to 1200x800: {:?}; resident={} bytes", start.elapsed(), q.usage().unwrap().resident);
    assert_eq!(output.pixels().len(), 960_000);
    assert!(output.pixels().iter().all(|pixel| *pixel == [0.2, 0.4, 0.6, 0.5]));
    drop(output);
    drop(prepared);
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn wide_reduction_reads_each_source_tile_row_once_with_one_tile_cache() {
    let q = quota();
    let (source, store) = layer(&q, ColorMode::Rgb, &[0.2, 0.4, 0.6, 0.5], Rect::new(0, 0, 600, 300));
    let mut doc = document(ColorMode::Rgb, vec![source]);
    doc.size = Size::new(600, 300);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    let output = prepared.render_reduced(3, 3, &control()).unwrap();
    assert!(output.pixels().iter().all(|pixel| *pixel == [0.2, 0.4, 0.6, 0.5]));
    // Each row has two full 256px tile columns (two 4KiB IO slices each) and
    // one 88px tail (one slice): five slices × 300 rows. No repeated tile load.
    assert_eq!(store.reads.load(Ordering::Relaxed), 1500);
    drop(output);
    drop(prepared);
    drop((doc, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn nonuniform_cross_tile_reduction_matches_independent_area_and_coverage_equation() {
    let q = quota();
    let (bottom, bs) = layer(&q, ColorMode::Rgb, &[0.1, 0.3, 0.7, 0.25], Rect::new(0, 0, 520, 260));
    let foreground = Rect::new(255, 128, 258, 260);
    let (top, ts) = layer(&q, ColorMode::Rgb, &[0.9, 0.2, 0.1, 0.5], foreground);
    let mut doc = document(ColorMode::Rgb, vec![bottom, top]);
    doc.size = Size::new(520, 260);
    let prepared = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    let reduced = prepared.render_reduced(7, 3, &control()).unwrap();
    for y in 0..3 {
        for x in 0..7 {
            let rect = Rect::new(x * 520 / 7, y * 260 / 3, (x + 1) * 520 / 7, (y + 1) * 260 / 3);
            let area = f64::from(rect.width()) * f64::from(rect.height());
            let overlap = rect.intersect(&foreground);
            let covered = f64::from(overlap.width()) * f64::from(overlap.height());
            // Independent source-over premultiplied products, then area average.
            let alpha = 0.25 * (area - covered) + (0.5 + 0.25 * 0.5) * covered;
            let colors = [(0.1, 0.9), (0.3, 0.2), (0.7, 0.1)].map(|(bg, fg)| ((bg * 0.25 * (area - covered) + (fg * 0.5 + bg * 0.25 * 0.5) * covered) / alpha) as f32);
            assert_eq!(reduced.pixels()[(y * 7 + x) as usize], [colors[0], colors[1], colors[2], (alpha / area) as f32]);
        }
    }
    let narrow = PreparedBackedDocument::new(&doc, Builtin::Srgb.profile(), NormalBandLimits { max_rows: 23, max_pixels: 4096 }, &control()).unwrap();
    let other = narrow.render_reduced(7, 3, &control()).unwrap();
    assert_eq!(reduced.pixels(), other.pixels());
    drop((other, narrow, reduced, prepared)); drop((doc, bs, ts));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
