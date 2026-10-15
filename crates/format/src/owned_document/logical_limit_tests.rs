//! UNRUN sparse logical-size continuation, not dense-input or performance proof.
use super::*;
use photocraft_raster::backing::NativeTileVisitor;

#[derive(Debug)]
struct Constant {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Constant {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        if offset.checked_add(output.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        c.check(output.len() as u64)?;
        let pixel = self.descriptor.default_bytes();
        for (i, byte) in output.iter_mut().enumerate() {
            *byte = pixel[((offset + i as u64) % pixel.len() as u64) as usize];
        }
        Ok(())
    }
    fn visit_native_tile(&self, _coord: TileCoord, c: &ReadControl<'_>, _stage: &mut NativeTileVisitor<'_>) -> std::result::Result<bool, BackingError> {
        c.check(1)?;
        Ok(true)
    }
}
fn constant(q: &BackingQuota, sample: SampleType, height: u32) -> Fixture {
    let model = q.reserve(QuotaKind::Index, 64 << 10).unwrap();
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let mut default = [0; 40];
    for (channel, value) in [0.125, -0.0, 0.75, 1.0].into_iter().enumerate() {
        photocraft_color::try_write_sample_f64(&mut default, sample, channel, value).unwrap();
    }
    let rect = Rect::new(0, 0, 6000, height as i32);
    let descriptor = NativePlaneDescriptor::new(format, rect, &default[..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Index, (std::mem::size_of::<Constant>() + 128) as u64).unwrap();
    let store = Arc::new(Constant { descriptor, quota: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
    let native = BackedLayer::new(raster, descriptor, &ctl()).unwrap();
    let mut doc = Document::new("sparse logical-size control", Size::new(6000, height), ColorMode::Rgb, sample);
    doc.layers.push(Layer::new("constant", LayerContent::BackedRaster(native)));
    Fixture { doc, _model: model }
}

#[test]
#[ignore = "UNRUN: sparse24/36MP F32/F64 save/reopen preserves logical bounds and exact defaults with small actual storage"]
fn logical_size_above_old_cap_does_not_require_a_full_resident_plane() {
    for sample in [SampleType::F32, SampleType::F64] {
        for height in [4000, 6000] {
            let q = q();
            let baseline = q.usage().unwrap();
            let fixture = constant(&q, sample, height);
            let expected = 6000u64 * u64::from(height) * layer(&fixture.doc).format().bytes_per_pixel() as u64;
            assert!(expected > 256 << 20);
            let loaded = roundtrip(&fixture.doc, &q);
            let native = layer(loaded.document());
            assert_eq!(native.descriptor().logical_bytes(), expected);
            assert_eq!(native.operative_bounds(), fixture.doc.bounds());
            assert_eq!(native.default_bytes(), layer(&fixture.doc).default_bytes());
            let mut pixel = [0; 40];
            let length = native.format().bytes_per_pixel();
            native.checked_view().read_logical_pixel_native_into(5999, height as i32 - 1, &mut pixel[..length], &ctl()).unwrap();
            assert_eq!(&pixel[..length], native.default_bytes());
            // This is an actual retained-quota observation of a sparse default
            // provider, not a claim about dense24/36MP images or process RSS.
            assert!(q.usage().unwrap().resident < 64 << 20);
            drop(loaded);
            drop(fixture);
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
}

#[test]
#[ignore = "UNRUN: lowered logical limits constrain both writer and reader; raises and zero stay finite"]
fn logical_allowance_is_shared_lower_only_and_does_not_bypass_actual_quota_or_control() {
    let q = q();
    let fixture = constant(&q, SampleType::F64, 6000);
    let needed = layer(&fixture.doc).descriptor().logical_bytes();
    let limits = OwnedDocumentLimits { max_finite_logical_bytes: needed - 1, ..Default::default() };
    assert!(save_current_document_archive(&fixture.doc, Writer::new(&q), limits, &q, &ctl()).is_err());
    let memory = save_current_document_archive(&fixture.doc, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    assert!(load_document_from_owned_archive(memory.clone(), &Default::default(), limits, q.clone(), &ctl()).is_err());
    let exact = OwnedDocumentLimits { max_finite_logical_bytes: needed, ..Default::default() };
    assert!(load_document_from_owned_archive(memory.clone(), &Default::default(), exact, q.clone(), &ctl()).is_ok());
    let zero = OwnedDocumentLimits { max_finite_logical_bytes: 0, ..Default::default() };
    assert!(load_document_from_owned_archive(memory.clone(), &Default::default(), zero, q.clone(), &ctl()).is_err());
    let raised = OwnedDocumentLimits { max_finite_logical_bytes: STANDARD_MAX_FINITE_LOGICAL_BYTES + 1, ..Default::default() };
    assert!(raised.validate().is_err());
    let before = q.usage().unwrap();
    let held = q.reserve(QuotaKind::Resident, (256u64 << 20) - before.resident - 1024).unwrap();
    assert!(load_document_from_owned_archive(memory.clone(), &Default::default(), Default::default(), q.clone(), &ctl()).is_err());
    drop(held);
    assert_eq!(q.usage().unwrap(), before, "logical admission never supplies extra resident capacity");
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(load_document_from_owned_archive(memory, &Default::default(), Default::default(), q, &deny).is_err());
}
