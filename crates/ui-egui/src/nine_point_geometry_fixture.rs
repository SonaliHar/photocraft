//! Paid synthetic larger CURRENT native storage repeating exact pinned 8x6 inputs.
//! Baselines prove source provenance only; larger geometry is not GIMP evidence.
//! All physical effect-mask words remain BYTE-zero, including padding/default.
use photocraft_doc::{
    BackedLayer, ColorMode, Document, LayerContent, SampleEncoding, SampleType, Size, accounted_document::AccountedDocument, item_identity::ItemRef,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod native;
#[derive(Clone, Copy)]
pub(super) struct Case {
    pub label: &'static str,
    pub path: &'static str,
    pub hash: &'static str,
    pub bytes: usize,
    pub operation: &'static str,
    pub schema: &'static str,
    pub mode: ColorMode,
    pub depth: SampleType,
    pub encoding: SampleEncoding,
    pub precision: u32,
}
pub(super) const CASES: [Case; 9] = [
    Case {
        label: "AbsoluteLinear",
        path: "corpus/xcf/full/point-absolute-evidence/absolute-rgb-float_linear.small.source.xcf",
        hash: "58ed83cb4285068bf0335d9deb5892572ebe5eb44bba7d47d495122381cee0cc",
        bytes: 2060,
        operation: "gegl:absolute",
        schema: "xcf-22-stock-absolute-linear-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::Linear,
        precision: 600,
    },
    Case {
        label: "ChannelMixer",
        path: "corpus/xcf/full/channel-mixer-precision-evidence/rgb.float_non_linear.source.xcf",
        hash: "60d42ae0f8d677550e6b7659939220cbcdaf75b30ed933fc1d069e5859826c91",
        bytes: 2403,
        operation: "gegl:channel-mixer",
        schema: "xcf-22-stock-channel-mixer-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "MonoMixerSigned",
        path: "corpus/xcf/full/mono-mixer-signed-precision-evidence/rgb.float_non_linear.source.xcf",
        hash: "18ea6916f34e8533c29a5803d217c5d1ca66ab1dc8f091e02bfe3b067653144f",
        bytes: 2235,
        operation: "gegl:mono-mixer",
        schema: "xcf-22-stock-mono-mixer-signed-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "ColorOverlay",
        path: "corpus/xcf/full/point-color-overlay-evidence/rgb.float_non_linear.srgb0.source.xcf",
        hash: "22c24f13c38da0c1d41ad8089852fcf44603a0bbb1a420ffc1afa99792da62b5",
        bytes: 2271,
        operation: "gegl:color-overlay",
        schema: "xcf-22-stock-color-overlay-six-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "ComponentExtract",
        path: "corpus/xcf/full/point-component-extract-evidence/enabled/rgb.float_non_linear.rgb-r.invert0.linear0.xcf",
        hash: "0548c336d209e6616aed151284b8e6aeddc2d8c1fa7de7a50b459137536eccc2",
        bytes: 2330,
        operation: "gegl:component-extract",
        schema: "xcf-22-stock-component-extract-four-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "ComponentExtractCmyk",
        path: "corpus/xcf/full/point-component-extract-cmyk-evidence/states/rgb.float_non_linear.cmyk-c.source.xcf",
        hash: "d7b6cce837e8af0ab01e9830f5fac3c2227626e98cc436eedccbefd3daac5709",
        bytes: 2380,
        operation: "gegl:component-extract",
        schema: "xcf-22-stock-component-extract-cmyk-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "ComponentExtractYcbcr",
        path: "corpus/xcf/full/point-component-extract-ycbcr-evidence/states/rgb.float_non_linear.ycbcr-y.source.xcf",
        hash: "3fad22666d3243feaae37d11cb0a69bde7d996eda95ba7f0b1fcf4969c67089d",
        bytes: 2381,
        operation: "gegl:component-extract",
        schema: "xcf-22-stock-component-extract-ycbcr-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "AlphaClip",
        path: "corpus/xcf/full/point-alpha-clip-evidence/rgb-float_non_linear.source.xcf",
        hash: "4bb21ca78a84e2e51748804a44b07a22ce09eeba3da7ad71fc30b32a1107755a",
        bytes: 2237,
        operation: "gegl:alpha-clip",
        schema: "xcf22-stock-alpha-clip-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "AlphaClipFinite",
        path: "corpus/xcf/full/point-alpha-clip-value-domain-nonlinear-controls/rgb-float_non_linear.unit.ordinary.xcf",
        hash: "e2a2a4bc7008cc3f485f29c4aee58e4702cf72d240f9f7a54d1da6056dc9e1c1",
        bytes: 2209,
        operation: "gegl:alpha-clip",
        schema: "xcf22-stock-alpha-clip-finite-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
];
pub(super) const EXTRA_CASES: [Case; 4] = [
    Case {
        label: "AbsoluteLinearGrayF64",
        path: "corpus/xcf/full/point-absolute-evidence/absolute-gray-double_linear.small.source.xcf",
        hash: "fb896b82562e1d257e2a1d9d4ec9cf2b1d5210a6b827f7b84f2453a3e2d123ba",
        bytes: 1877,
        operation: "gegl:absolute",
        schema: "xcf-22-stock-absolute-linear-precision-v1",
        mode: ColorMode::Grayscale,
        depth: SampleType::F64,
        encoding: SampleEncoding::Linear,
        precision: 700,
    },
    Case {
        label: "ChannelMixerGrayF64Perceptual",
        path: "corpus/xcf/full/channel-mixer-precision-evidence/gray.double_perceptual.source.xcf",
        hash: "87adb325e12611b19165c50938d3fab0212a23a6aae94d7d2a446e97df2e79d8",
        bytes: 2388,
        operation: "gegl:channel-mixer",
        schema: "xcf-22-stock-channel-mixer-precision-v1",
        mode: ColorMode::Grayscale,
        depth: SampleType::F64,
        encoding: SampleEncoding::Perceptual,
        precision: 775,
    },
    Case {
        label: "ColorOverlayGrayU16Perceptual",
        path: "corpus/xcf/full/point-color-overlay-evidence/gray.u16_perceptual.srgb0.source.xcf",
        hash: "fc557ae7bb2615c210b8f384640a25105ae0bec9675455a0d400268fa5f362f3",
        bytes: 1914,
        operation: "gegl:color-overlay",
        schema: "xcf-22-stock-color-overlay-six-precision-v1",
        mode: ColorMode::Grayscale,
        depth: SampleType::U16,
        encoding: SampleEncoding::Perceptual,
        precision: 275,
    },
    Case {
        label: "AlphaClipGrayU16",
        path: "corpus/xcf/full/point-alpha-clip-evidence/gray-u16_non_linear.source.xcf",
        hash: "ea863761720ae216805eafc326b651ce7dcdce76c8c463ddaf141ef472e22129",
        bytes: 1963,
        operation: "gegl:alpha-clip",
        schema: "xcf22-stock-alpha-clip-precision-v1",
        mode: ColorMode::Grayscale,
        depth: SampleType::U16,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 250,
    },
];
pub(super) fn ctl() -> ReadControl<'static> {
    native::ctl()
}
pub(super) fn case_count() -> usize {
    CASES.len() + EXTRA_CASES.len()
}
pub(super) fn case(index: usize) -> &'static Case {
    if index < CASES.len() { &CASES[index] } else { &EXTRA_CASES[index - CASES.len()] }
}
pub(super) fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(owner)
}
pub(super) fn encoded(owner: &AccountedDocument) -> Vec<u8> {
    native::encoded(owner)
}
pub(super) fn raw(doc: &Document) -> Vec<u8> {
    native::native(doc)
}
pub(super) fn node(doc: &Document) -> &photocraft_doc::xcf_filters::FilterNode {
    &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
}
pub(super) fn changed(doc: &Document) -> Option<serde_json::Value> {
    let n = node(doc);
    if n.arguments.is_empty() {
        return None;
    }
    let mut arguments = serde_json::to_value(&n.arguments).unwrap();
    fn wire(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(bits) = map.get_mut("Float64Bits") {
                    let n = bits.as_u64().unwrap();
                    *bits = serde_json::json!(format!("0x{n:016x}"));
                }
                if let Some(encoding) = map.get_mut("encoding") {
                    if *encoding == "NonlinearProfile" {
                        *encoding = serde_json::json!("nonlinear_profile");
                    }
                }
                for value in map.values_mut() {
                    wire(value)
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    wire(value)
                }
            }
            _ => {}
        }
    }
    wire(&mut arguments);
    Some(serde_json::json!({"filterId":n.id.0.to_string(),"arguments":arguments,"enabled":!n.enabled}))
}
#[derive(Debug)]
struct Repeated {
    pattern: Vec<u8>,
    descriptor: NativePlaneDescriptor,
    width: u32,
    bpp: usize,
    quota: BackingQuota,
    _pattern: QuotaLease,
    _header: QuotaLease,
}
impl NativePlaneStore for Repeated {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        let end = at.checked_add(out.len() as u64).ok_or(BackingError::Length)?;
        if end > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        c.check(out.len() as u64)?;
        for (i, byte) in out.iter_mut().enumerate() {
            if i % 512 == 0 {
                c.check(0)?
            }
            let n = at as usize + i;
            let pixel = n / self.bpp;
            let x = pixel % self.width as usize;
            let y = pixel / self.width as usize;
            let source = ((y % 6) * 8 + x % 8) * self.bpp + n % self.bpp;
            *byte = *self.pattern.get(source).ok_or(BackingError::Bounds)?;
        }
        Ok(())
    }
}
struct Retention {
    _source: photocraft_io::OwnedPointXcfImport,
    _model: QuotaLease,
    _resident: QuotaLease,
}
pub(super) fn build(index: usize, size: Size) -> AccountedDocument {
    let row = case(index);
    assert!(size.width > 0 && size.height > 0);
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let bytes = std::fs::read(root.join(row.path.strip_prefix("corpus/xcf/full/").unwrap())).unwrap();
    assert_eq!(bytes.len(), row.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), row.hash);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 64 << 20;
    options.limits.input_bytes = 16 << 20;
    let imported = photocraft_io::import_owned_xcf_bytes_in_quota("pinned-baseline.xcf", Arc::new(bytes), &options, &limits, q.clone(), &ctl()).unwrap();
    let photocraft_io::OwnedXcfImport::Point(source) = imported else { panic!("exact Point source") };
    let old = source.document().document();
    assert_eq!(old.size, Size::new(8, 6));
    assert_eq!(old.mode, row.mode);
    assert_eq!(old.depth, row.depth);
    assert_eq!(old.sample_encoding, Some(row.encoding));
    assert_eq!(old.source_color_context.as_ref().unwrap().source_precision, row.precision);
    assert_eq!(node(old).operation.name, row.operation);
    assert_eq!(node(old).operation.schema_epoch, row.schema);
    // Prospective metadata/map/profile retention before deep model clone. Pattern
    // and provider header receive their own actual same-ledger leases below.
    let model = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let LayerContent::BackedRaster(old_layer) = &old.layers[0].content else { panic!("backed") };
    let bpp = old_layer.format().bytes_per_pixel();
    let len = 48 * bpp;
    let mut pattern_fee = q.reserve(QuotaKind::Resident, (len * 2) as u64).unwrap();
    let mut pattern = Vec::new();
    pattern.try_reserve_exact(len).unwrap();
    assert!(pattern.capacity() <= len * 2);
    pattern.resize(len, 0);
    let capture = raw(old);
    assert_eq!(capture.len(), len);
    pattern.copy_from_slice(&capture);
    pattern_fee.reconcile_capacity(pattern.capacity() as u64).unwrap();
    let bounds = photocraft_doc::Rect::from_xywh(0, 0, size.width, size.height);
    let descriptor = NativePlaneDescriptor::new(old_layer.format(), bounds, old_layer.default_bytes()).unwrap();
    let header = q.reserve(QuotaKind::Index, (std::mem::size_of::<Repeated>() + 128) as u64).unwrap();
    let raster = BackedRaster::new(
        Arc::new(Repeated { pattern, descriptor, width: size.width, bpp, quota: q.clone(), _pattern: pattern_fee, _header: header }),
        q.clone(),
        2,
        &ctl(),
    )
    .unwrap();
    let mut doc = old.clone();
    doc.size = size;
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    let layer_id = doc.layers[0].id;
    let state = doc.xcf_state.as_mut().unwrap();
    for mask in &mut state.filter_masks {
        mask.bounds = bounds;
        // Share the genuine BYTE-zero source tile, including its full padding.
        let zero = mask.surface.tiles().next().map(|(_, tile)| Arc::clone(tile));
        if let Some(zero) = zero {
            for coord in bounds.tiles() {
                mask.surface.put_tiles(std::iter::once((coord, zero.clone())));
            }
        }
    }
    for item in &mut state.items {
        match item.item {
            ItemRef::Layer(id) if id == layer_id => item.raster_bounds = Some(bounds),
            ItemRef::Mask(_) => {
                item.raster_bounds = Some(bounds);
                if let Some(mask) = &mut item.mask {
                    mask.bounds = bounds;
                    mask.offset = [0, 0];
                }
            }
            _ => {}
        }
    }
    let measured = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(measured.retained_bytes() <= 16 << 20);
    AccountedDocument::new_readonly_point_storage(doc, Arc::new(Retention { _source: source, _model: model, _resident: resident }), q, &ctl()).unwrap()
}
pub(super) fn same(a: &Document, b: &Document) {
    native::exact(a, b);
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.layers[0].name, b.layers[0].name);
    assert_eq!(a.layers[0].visible, b.layers[0].visible);
    assert_eq!(a.layers[0].opacity.to_bits(), b.layers[0].opacity.to_bits());
}
pub(super) fn retained(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(raw(a), raw(b));
    let (LayerContent::BackedRaster(x), LayerContent::BackedRaster(y)) = (&a.layers[0].content, &b.layers[0].content) else { panic!("native") };
    assert_eq!(x.descriptor(), y.descriptor());
    assert_eq!(x.default_bytes(), y.default_bytes());
    let (x, y) = (a.xcf_state.as_ref().unwrap(), b.xcf_state.as_ref().unwrap());
    assert_eq!(x.opaque, y.opaque);
    assert_eq!(x.provenance, y.provenance);
    assert_eq!(x.filter_masks, y.filter_masks);
    assert_eq!(x.filter_stacks[0].owner, y.filter_stacks[0].owner);
    assert_eq!(node(a).operation, node(b).operation);
    assert_eq!(node(a).composition, node(b).composition);
    assert_eq!(node(a).mask, node(b).mask);
}
