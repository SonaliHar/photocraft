//! Paid synthetic larger CURRENT native storage repeating exact pinned 8x6 inputs.
//! Baselines prove source provenance only; larger geometry is not GIMP evidence.
//! All old family physical effect-mask BYTE-zero/default/padding rules remain unchanged.
//! Seven exact residual/configured epochs; Balance activity remains visible+opacity1.
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
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
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
pub(super) const CASES: [Case; 7] = [
    Case {
        label: "Desaturate",
        path: "corpus/xcf/full/desaturate-precision-evidence/rgb-float_non_linear.xcf",
        hash: "28a60a0263769e778ec426881896d1464635e06a86a4bda1bafc9a3ad3bfdfc2",
        bytes: 2070,
        operation: "gimp:desaturate",
        schema: "xcf-22-stock-desaturate-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "SaturationRgb",
        path: "corpus/xcf/full/saturation-precision-evidence/rgb-float_non_linear.xcf",
        hash: "f67594bceda78f56018702e8536139f5ed7248c80598509d459841884c7ae1eb",
        bytes: 2102,
        operation: "gegl:saturation",
        schema: "xcf-22-stock-saturation-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "SaturationGray",
        path: "corpus/xcf/full/saturation-precision-evidence/gray-float_non_linear.xcf",
        hash: "319b34884dbd66bbec275566dc44097415eb0d4debd316bce90137254777d782",
        bytes: 1903,
        operation: "gegl:saturation",
        schema: "xcf-22-stock-gray-saturation-precision-v1",
        mode: ColorMode::Grayscale,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "SaturationYuv",
        path: "corpus/xcf/full/saturation-precision-evidence/rgb-float_non_linear.cieyuv-half.xcf",
        hash: "9df981ff2f5c3c9d7cb35cadb946df3624929b46a6ed5094c7530664d975c52a",
        bytes: 2102,
        operation: "gegl:saturation",
        schema: "xcf-22-stock-rgb-yuv-saturation-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 650,
    },
    Case {
        label: "ValueInvertPrecision",
        path: "corpus/xcf/full/value-invert-precision-evidence/float_linear.source.xcf",
        hash: "15fc8cb898444d762862f78b5771f6521f020483e7092adad9ee1113be450556",
        bytes: 2165,
        operation: "gegl:value-invert",
        schema: "xcf-22-stock-value-invert-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::Linear,
        precision: 600,
    },
    Case {
        label: "ColorBalance",
        path: "corpus/xcf/full/color-balance-evidence/rgb-double_linear.source.xcf",
        hash: "8dc901b1596f7f534b5e1efce325b0530f54afc3785fde61502595318d2199dd",
        bytes: 2376,
        operation: "gimp:color-balance",
        schema: "xcf-22-stock-color-balance-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F64,
        encoding: SampleEncoding::Linear,
        precision: 700,
    },
    Case {
        label: "ColorBalanceDepth",
        path: "corpus/xcf/full/color-balance-depth-evidence/rgb-float_perceptual.source.xcf",
        hash: "cf5cce8eda9759d2bf8a7704a5e53f3dc7c6741b7b2be4e79f94b02561eb823e",
        bytes: 2391,
        operation: "gimp:color-balance",
        schema: "xcf-22-stock-color-balance-depth-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::Perceptual,
        precision: 675,
    },
];
pub(super) const EXTRA_CASES: [Case; 5] = [
    Case {
        label: "SaturationGrayF64Linear",
        path: "corpus/xcf/full/saturation-precision-evidence/gray-double_linear.xcf",
        hash: "32884a8fd8e8181a4e4952bd8a4c0faf3599568ef98c78468a190f88c147dc77",
        bytes: 1905,
        operation: "gegl:saturation",
        schema: "xcf-22-stock-gray-saturation-precision-v1",
        mode: ColorMode::Grayscale,
        depth: SampleType::F64,
        encoding: SampleEncoding::Linear,
        precision: 700,
    },
    Case {
        label: "SaturationYuvU16NL",
        path: "corpus/xcf/full/saturation-precision-evidence/rgb-u16_non_linear.cieyuv-half.xcf",
        hash: "f608e6f781f49ce652c051ed5d9099c5d07c65036f9184cd89c57537a8581292",
        bytes: 2036,
        operation: "gegl:saturation",
        schema: "xcf-22-stock-rgb-yuv-saturation-precision-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::U16,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 250,
    },
    Case {
        label: "BalanceDepthU32NL",
        path: "corpus/xcf/full/color-balance-depth-evidence/rgb-u32_non_linear.source.xcf",
        hash: "e484aa9c1159ed6b534556451733b2850e4d94ccaa3c8cbc1fd777a7d41c5aa3",
        bytes: 2611,
        operation: "gimp:color-balance",
        schema: "xcf-22-stock-color-balance-depth-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::U32,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 350,
    },
    Case {
        label: "BalanceDepthHalfLinear",
        path: "corpus/xcf/full/color-balance-depth-evidence/rgb-half_linear.source.xcf",
        hash: "458bb490f6f96b8ca1b03d068059389e153ebf2e0fe409e91fc6986fe18c84ae",
        bytes: 2192,
        operation: "gimp:color-balance",
        schema: "xcf-22-stock-color-balance-depth-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::F32,
        encoding: SampleEncoding::Linear,
        precision: 500,
    },
    Case {
        label: "BalanceDepthU8NL",
        path: "corpus/xcf/full/color-balance-depth-evidence/rgb-u8_non_linear.source.xcf",
        hash: "7aabbe4c1a9cdf4f846292ba9c599f123456e21e34709c0fa6c51175e782249b",
        bytes: 2179,
        operation: "gimp:color-balance",
        schema: "xcf-22-stock-color-balance-depth-v1",
        mode: ColorMode::Rgb,
        depth: SampleType::U8,
        encoding: SampleEncoding::NonlinearProfile,
        precision: 150,
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
                if let Some(encoding) = map.get_mut("encoding")
                    && *encoding == "NonlinearProfile"
                {
                    *encoding = serde_json::json!("nonlinear_profile");
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
