//! Editable import of admitted XCF rendering semantics. Nothing executes GIMP resources.
use crate::{ImportOrigin, ImportResult, IoError};
use photocraft_doc::{
    AlphaChannel, BlendMode, ColorMode, ColorTable, Document, ImportCompositing, IndexedSource, Layer, LayerContent, LayerMask, PixelFormat, Rect,
    SampleEncoding, SampleType, Size, SourceColorContext,
};
use photocraft_raster::{Interrupt, Surface};
use photocraft_xcf::{ArgumentValue, DecodeOptions, Drawable, Encoding, ErrorKind, Limits, Model, Plane, Sample, XcfError, XcfFile};
use std::sync::Arc;

#[path = "xcf_import/text_preparation.rs"]
mod text_preparation;

#[path = "xcf_import/plane.rs"]
mod plane;
use plane::{ExactParsedPlane, MappingPlane};

#[path = "xcf_import/admission.rs"]
mod admission;

#[path = "xcf_import/accounted_group.rs"]
mod accounted_group;

#[path = "xcf_import/unpremultiply.rs"]
pub(crate) mod unpremultiply;

#[path = "xcf_import/provenance.rs"]
mod provenance;

#[path = "xcf_import/staged_source.rs"]
mod staged_source;
pub use staged_source::import_staged_plain_source_with_budget;

#[path = "xcf_import/source.rs"]
mod source;
pub use source::import_plain_source_with_budget;

#[path = "xcf_import/stored_channel_candidate.rs"]
mod stored_channel_candidate;
pub use stored_channel_candidate::{assemble_owned_stored_channel, AssembledStoredChannel, NamespaceBacking, StoredChannelCorrespondence, CorrespondenceRequirements};
pub use stored_channel_candidate::{CurrentProfilePolicy, SelectedCurrentChannel, SelectedLimits, SelectedProfileRequirements, SelectedRequirements, select_current, select_current_with_profile_policy, selected_profile_requirements};

#[path = "xcf_import/layer_selection_candidate.rs"]
mod layer_selection_candidate;
pub use layer_selection_candidate::{
    LayerOuterPolicy, LayerSelectionIds, LayerSelectionCandidate, LayerSelectionLimits, LayerSelectionNamespace, LayerSelectionRequirements,
    LAYER_SELECTION_EPOCH, map_layer_selection, map_layer_selection_with_outer_policy,
};

#[path = "xcf_import/layer_selection_owner.rs"]
mod layer_selection_owner;
pub use layer_selection_owner::{
    AssembledLayerSelection, LayerCorrespondenceRequirements, LayerCurrentLimits, LayerCurrentRequirements, LayerOriginalPlanesProof,
    LayerProfilePolicy, LayerProfileRequirements, SelectedCurrentLayer, assemble_owned_layer_selection,
    assemble_owned_layer_selection_with_outer_policy, layer_profile_requirements, select_layer_current, select_layer_current_with_profile_policy,
};

#[path = "xcf_import/layer_selection_current.rs"]
mod layer_selection_current;
pub use layer_selection_current::{
    LayerStorageRequirements, PrivateLayerCurrent, assemble_private_layer_current, layer_storage_requirements,
};

#[cfg(all(test, feature = "corpus"))]
mod linked_custom_name_mapping_tests {
    use super::*;
    #[test]
    fn linked_name_flag_maps_without_changing_pixels_or_monitoring() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../photocraft-corpus/xcf/full/transform-interpolation");
        let original = std::fs::read(root.join("link-none.xcf")).unwrap();
        let decoded = photocraft_xcf::decode(&original, &Default::default()).unwrap();
        let position = decoded.layers.iter().position(|layer| layer.link.is_some()).unwrap();
        let pixels = decoded.layers[position].plane.as_ref().unwrap().bytes.clone();
        for flags in 0..4u32 {
            // Synthetic metadata controls in the already admitted linked domain;
            // these are not independent render or new source-family evidence.
            let mut file = decoded.clone();
            file.layers[position].link.as_mut().unwrap().flags = flags;
            for property in &mut file.layers[position].raw_properties {
                if property.tag == 48 {
                    property.payload[..4].copy_from_slice(&flags.to_be_bytes());
                }
            }
            let imported = map("linked name mapping", file, &Interrupt::NONE, 128 << 20).unwrap().0;
            let doc = imported.document;
            let state = doc.xcf_state.as_ref().unwrap();
            let source = state.source_items.iter().find(|source| matches!(source.source,
                photocraft_doc::xcf_items::EditableSource::Linked { .. })).unwrap();
            let item = state.items.iter().find(|item| item.item == source.owner).unwrap();
            assert_eq!(item.custom_name, flags & 1 != 0);
            assert_eq!(item.name, decoded.layers[position].name);
            assert_eq!(source.cache.as_ref().unwrap().bytes.as_slice(), pixels.as_slice());
            let photocraft_doc::xcf_items::EditableSource::Linked { configuration, .. } = &source.source else {
                panic!("linked source")
            };
            assert!(configuration.iter().any(|argument| argument.name == "flags" &&
                matches!(argument.value, photocraft_doc::xcf_filters::FilterValue::Number(
                    photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value })
                    if value == u64::from(flags))));
            let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), doc);
        }
        assert_eq!(std::fs::read(root.join("link-none.xcf")).unwrap(), original);
    }
}

/// Common import contract, including cancellation and a conservative shared resident budget.
pub fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult, IoError> {
    import_with_limits(name, bytes, ctl, Limits::default())
}
/// The same staged import with additive machine-readable failure details.
/// Successful results retain the existing contract; this API makes no expanded
/// full-fidelity or resource-recovery claim.
pub fn import_with_diagnostics(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult, crate::xcf_assessment::ImportFailure> {
    import(name, bytes, ctl).map_err(Into::into)
}
/// Content-first wrapper dispatch. A valid non-XCF wrapper is left for its
/// actual loader; malformed wrappers and contained XCF failures stay errors.
pub fn import_candidate(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<ImportResult>, IoError> {
    let cancelled = || ctl.cancelled();
    let progress = |v| ctl.progress(v * 0.7);
    let limits = photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default())?;
    let policy = crate::xcf_policy::for_resources(&limits)?;
    let options = DecodeOptions { limits: policy.parser, cancelled: Some(&cancelled), progress: Some(&progress), allow_finite_drawable_alpha: true };
    let budget = photocraft_xcf::Budget::new(policy.resources, Some(&cancelled));
    import_candidate_with_budget(name, bytes, ctl, &options, &budget, photocraft_xcf::SourceAccounting::Borrowed)
}

/// Caller-owned transaction accounting. Already captured inputs must keep their
/// encoded/resident source reservations alive through this call. The returned
/// document leaves transaction accounting when it is transferred to its owner.
pub fn import_candidate_with_budget(
    name: &str,
    bytes: &[u8],
    ctl: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    shared: &photocraft_xcf::Budget<'_>,
    accounting: photocraft_xcf::SourceAccounting,
) -> Result<Option<ImportResult>, IoError> {
    import_candidate_accounted(name, bytes, ctl, options, shared, accounting, None)
}
fn import_candidate_accounted(
    name: &str,
    bytes: &[u8],
    ctl: &Interrupt<'_>,
    options: &DecodeOptions<'_>,
    shared: &photocraft_xcf::Budget<'_>,
    accounting: photocraft_xcf::SourceAccounting,
    decoded_phase: Option<&dyn Fn()>,
) -> Result<Option<ImportResult>, IoError> {
    // Mapping below validates the qualified finite-alpha domain before publishing a document.
    let mapping_options =
        DecodeOptions { limits: options.limits.clone(), allow_finite_drawable_alpha: true, cancelled: options.cancelled, progress: options.progress };
    let decoded = match photocraft_xcf::decode_candidate_with_budget(bytes, &mapping_options, shared, accounting) {
        Err(e) if e.kind == ErrorKind::Cancelled => return Err(IoError::Cancelled),
        other => other?,
    };
    if let Some(phase) = decoded_phase {
        phase();
    }
    decoded
        .map(|decoded| {
            let photocraft_xcf::BudgetDecodedImport { decoded, reservations } = decoded;
            let source_bytes = decoded.encoded_bytes;
            let previous = std::cell::Cell::new(decoded.file.charged_bytes);
            let lease = std::cell::RefCell::new(reservations);
            let account = |used: usize| {
                // Conservative materialization work: one unit per newly charged
                // output/metadata byte, including repeated temporary buffers.
                if used > previous.get() {
                    shared.consume_work((used - previous.get()) as u64, "mapping materialization").map_err(accounting_error)?;
                }
                let retained = (used as u64).saturating_sub(source_bytes);
                lease
                    .try_borrow_mut()
                    .map_err(|_| IoError::Unsupported("XCF mapping accounting is already borrowed".into()))?
                    .resize_retained(retained, "XCF mapping")
                    .map_err(accounting_error)?;
                previous.set(used);
                Ok(())
            };
            let maximum = options.limits.resident_bytes.min(
                usize::try_from(shared.limits().get(photocraft_xcf::BudgetKind::Resident))
                    .map_err(|_| IoError::Unsupported("XCF resident policy exceeds address space".into()))?,
            );
            let (mut result, _) = map_accounted(name, decoded.file, ctl, maximum, Some(&account), Some((shared, &lease)))?;
            shared.consume_work(bytes.len() as u64, "source provenance hash").map_err(accounting_error)?;
            stamp_source(&mut result, bytes, decoded.container, ctl)?;
            shared.poll("mapped document completed").map_err(
                |error| {
                    if error.kind == ErrorKind::Cancelled { IoError::Cancelled } else { IoError::Xcf(error) }
                },
            )?;
            Ok(result)
        })
        .transpose()
}
fn accounting_error(error: XcfError) -> IoError {
    if error.kind == ErrorKind::Cancelled { IoError::Cancelled } else { IoError::Xcf(error) }
}
fn stamp_source(result: &mut ImportResult, bytes: &[u8], container: photocraft_xcf::SourceContainer, ctl: &Interrupt<'_>) -> Result<(), IoError> {
    if let Some(state) = &mut result.document.xcf_state {
        let mut hash = blake3::Hasher::new();
        for chunk in bytes.chunks(4096) {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            hash.update(chunk);
        }
        state.provenance.source_hash = Some(hash.finalize().to_hex().to_string());
        state.provenance.container = match container {
            photocraft_xcf::SourceContainer::Plain => "plain",
            photocraft_xcf::SourceContainer::Gzip => "gzip",
            photocraft_xcf::SourceContainer::Bzip2 => "bzip2",
            photocraft_xcf::SourceContainer::Xz => "xz",
        }
        .into();
        if let Some(assessment) = &mut result.assessment {
            assessment.container.clone_from(&state.provenance.container);
        }
    }
    Ok(())
}
/// Same admission rules with tighter budgets, useful for untrusted-input tests and callers.
pub fn import_with_limits(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limits: Limits) -> Result<ImportResult, IoError> {
    let base = photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default())?;
    let policy = crate::xcf_policy::from_parser_limits(limits, &base)?;
    let cancelled = || ctl.cancelled();
    let progress = |v| ctl.progress(v * 0.7);
    let options = DecodeOptions { limits: policy.parser, cancelled: Some(&cancelled), progress: Some(&progress), allow_finite_drawable_alpha: true };
    let budget = photocraft_xcf::Budget::new(policy.resources, Some(&cancelled));
    import_candidate_with_budget(name, bytes, ctl, &options, &budget, photocraft_xcf::SourceAccounting::Borrowed)?
        .ok_or_else(|| IoError::Unsupported("input does not contain GIMP XCF".into()))
}
/// Development measurement of the same parser/adapter, without a different import path.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct ImportStats {
    pub decode: std::time::Duration,
    pub mapping: std::time::Duration,
    /// High-water mark of successful shared resident charges, including borrowed
    /// input and live component leases. This is not process RSS or a measurement
    /// of backend allocations not represented by reservations.
    pub peak_charged_bytes: usize,
}
#[cfg(not(target_arch = "wasm32"))]
pub fn import_with_stats(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<(ImportResult, ImportStats), IoError> {
    let cancelled = || ctl.cancelled();
    let progress = |v| ctl.progress(v * 0.7);
    let limits = photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default())?;
    let policy = crate::xcf_policy::for_resources(&limits)?;
    let options = DecodeOptions { limits: policy.parser, cancelled: Some(&cancelled), progress: Some(&progress), allow_finite_drawable_alpha: true };
    let budget = photocraft_xcf::Budget::new(policy.resources, Some(&cancelled));
    let before = std::time::Instant::now();
    let decode = std::cell::Cell::new(std::time::Duration::ZERO);
    let phase = || decode.set(before.elapsed());
    let result = import_candidate_accounted(name, bytes, ctl, &options, &budget, photocraft_xcf::SourceAccounting::Borrowed, Some(&phase))?
        .ok_or_else(|| IoError::Unsupported("input does not contain GIMP XCF".into()))?;
    let elapsed = before.elapsed();
    let peak_charged_bytes = usize::try_from(budget.peak(photocraft_xcf::BudgetKind::Resident))
        .map_err(|_| IoError::Unsupported("XCF charged peak exceeds address space".into()))?;
    Ok((result, ImportStats { decode: decode.get(), mapping: elapsed.saturating_sub(decode.get()), peak_charged_bytes }))
}
struct Budget<'a> {
    used: usize,
    max: usize,
    peak: usize,
    shared: Option<&'a dyn Fn(usize) -> Result<(), IoError>>,
}
impl Budget<'_> {
    fn release(&mut self, n: usize) -> Result<(), IoError> {
        self.used = self.used.saturating_sub(n);
        if let Some(shared) = self.shared {
            shared(self.used)?;
        }
        Ok(())
    }
    fn charge(&mut self, n: usize) -> Result<(), IoError> {
        self.used = self.used.checked_add(n).ok_or_else(|| IoError::Unsupported("XCF mapping memory overflow".into()))?;
        self.peak = self.peak.max(self.used);
        if self.used > self.max {
            return Err(IoError::Xcf(XcfError {
                kind: ErrorKind::Limit,
                offset: 0,
                item: String::new(),
                message: "XCF mapping/provenance resident memory budget exceeded".into(),
            }));
        }
        if let Some(shared) = self.shared {
            shared(self.used)?;
        }
        Ok(())
    }
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        self.charge(n)?;
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| IoError::Unsupported("XCF mapping allocation failed".into()))?;
        v.resize(n, 0);
        Ok(v)
    }
}
fn unsupported<P>(d: &Drawable<P>, reason: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("XCF item '{}': {reason}", short(&d.name)))
}
fn short(s: &str) -> String {
    s.chars().take(80).collect()
}
fn encoding(e: Encoding) -> SampleEncoding {
    match e {
        Encoding::Linear => SampleEncoding::Linear,
        Encoding::NonlinearProfile => SampleEncoding::NonlinearProfile,
        Encoding::Perceptual => SampleEncoding::Perceptual,
    }
}
// GimpColorTag numeric order from the retained public GIMP 3.2.6
// libgimpbase/gimpbaseenums.h. Labels preserve stored identity, not swatch RGB.
fn layer_color_tag(tag: Option<u32>) -> std::result::Result<photocraft_doc::LabelColor, &'static str> {
    use photocraft_doc::LabelColor;
    match tag {
        None | Some(0) => Ok(LabelColor::None),
        Some(1) => Ok(LabelColor::Blue),
        Some(2) => Ok(LabelColor::Green),
        Some(3) => Ok(LabelColor::Yellow),
        Some(4) => Ok(LabelColor::Orange),
        Some(5) => Ok(LabelColor::Brown),
        Some(6) => Ok(LabelColor::Red),
        Some(7) => Ok(LabelColor::Violet),
        Some(8) => Ok(LabelColor::Gray),
        Some(_) => Err("unknown stored color tag has no exact PhotoCraft layer label"),
    }
}

fn physical_unit(unit: &photocraft_xcf::Unit) -> photocraft_doc::PhysicalUnit {
    use photocraft_doc::{CustomUnit, PhysicalUnit};
    match unit {
        photocraft_xcf::Unit::Inch => PhysicalUnit::Inch,
        photocraft_xcf::Unit::Millimeter => PhysicalUnit::Millimeter,
        photocraft_xcf::Unit::Point => PhysicalUnit::Point,
        photocraft_xcf::Unit::Pica => PhysicalUnit::Pica,
        photocraft_xcf::Unit::Custom(unit) => PhysicalUnit::Custom(CustomUnit {
            // Before XCF21 `name` is the unit's internal identifier; the plural
            // string is its public display name. Original strings stay in the
            // retained source property independently of this effective unit.
            name: unit.plural.as_ref().unwrap_or(&unit.name).clone(),
            factor: f64::from(unit.factor),
            digits: unit.digits,
            symbol: unit.symbol.clone(),
            abbreviation: unit.abbreviation.clone(),
        }),
    }
}
fn preserve_properties(
    state: &mut photocraft_doc::XcfState,
    properties: Vec<photocraft_xcf::RawProperty>,
    owner: Option<photocraft_doc::item_identity::ItemRef>,
    version: u32,
    budget: &mut Budget,
) -> Result<(), IoError> {
    state.opaque.try_reserve_exact(properties.len()).map_err(|_| IoError::Unsupported("XCF source-record allocation failed".into()))?;
    for (order, property) in properties.into_iter().enumerate() {
        budget.charge(512)?;
        let order = u32::try_from(order).map_err(|_| IoError::Unsupported("XCF property order overflow".into()))?;
        let hash = blake3::hash(&property.payload).to_hex().to_string();
        // This namespace is source evidence. Known semantics are also decoded
        // into their typed fields; these bytes do not replace evaluation.
        state.opaque.push(photocraft_doc::xcf_provenance::OpaqueRecord {
            owner,
            namespace: "gimp/xcf/source-property".into(),
            identity: format!("{}@{}:{}", property.tag, property.offset, property.declared_bytes),
            source_version: version,
            order,
            flags: 0,
            bytes: std::sync::Arc::new(property.payload),
            blake3: hash,
            rendering_required: false,
        });
    }
    Ok(())
}
fn working<P>(d: &Drawable<P>, n: i32) -> Result<SampleEncoding, IoError> {
    match n.checked_abs() {
        Some(0) if matches!(d.mode, 28 | 30) => Ok(SampleEncoding::Linear),
        Some(0) if d.mode == 0 => Ok(SampleEncoding::NonlinearProfile),
        // Component min/max commute with the qualified monotone RGB curves.
        // This is equivalent evaluation, not a unique inferred Auto space.
        Some(0) if matches!(d.mode, 35 | 36) => Ok(SampleEncoding::NonlinearProfile),
        // These qualified alpha modes do not use their Auto blend space;
        // Split also leaves backdrop color unchanged in its Auto composite space.
        Some(0) if matches!(d.mode, 58..=60) => Ok(SampleEncoding::NonlinearProfile),
        Some(0) if d.mode == 61 && d.composite_space != 0 => working(d, d.composite_space),
        Some(1) => Ok(SampleEncoding::Linear),
        Some(2) => Ok(SampleEncoding::NonlinearProfile),
        Some(4) => Ok(SampleEncoding::Perceptual),
        _ => Err(unsupported(d, format!("unsupported blend/composite space {n}"))),
    }
}
fn gimp_space(encoding: SampleEncoding) -> photocraft_doc::import_compositing::GimpSpace {
    use photocraft_doc::import_compositing::GimpSpace;
    match encoding {
        SampleEncoding::Linear => GimpSpace::LinearRgb,
        SampleEncoding::NonlinearProfile => GimpSpace::ProfileRgb,
        SampleEncoding::Perceptual => GimpSpace::PerceptualRgb,
    }
}
fn admit<P>(
    d: &Drawable<P>,
    effects_candidate: bool,
    blend_domain: bool,
    normal_mask_domain: bool,
    modified: Option<crate::xcf_modified_sources::ModifiedSourceKind>,
) -> Result<(), IoError> {
    if let Some(reason) = d.unsupported.iter().find(|reason| {
        !(effects_candidate && reason.as_str() == "GEGL layer effects"
            || modified == Some(crate::xcf_modified_sources::ModifiedSourceKind::Text) && reason.as_str() == "editable GIMP text layer"
            || modified == Some(crate::xcf_modified_sources::ModifiedSourceKind::Vector) && reason.as_str() == "editable GIMP vector layer"
            || modified == Some(crate::xcf_modified_sources::ModifiedSourceKind::Linked) && reason.as_str() == "linked layer/transform")
    }) {
        return Err(unsupported(d, reason));
    }
    if !(matches!(d.mode, 0 | 28) || matches!(d.mode, 3 | 4 | 6..=21 | 23..=27 | 30..=40 | 41..=60) && blend_domain || d.group && d.mode == 61) {
        return Err(unsupported(d, format!("blend mode {} is unsupported; no verified stored-raster fallback", d.mode)));
    }
    if !(d.composite_mode.checked_abs() == Some(1)
        || normal_mask_domain && d.mode == 28 && matches!(d.composite_mode, 1..=4)
        || matches!(d.mode, 3 | 4 | 6..=21 | 23..=27 | 30..=40 | 41..=60) && blend_domain && matches!(d.composite_mode, -2 | 1..=4)
        || d.composite_mode == 0 && matches!(d.mode, 0 | 28))
    {
        return Err(unsupported(d, format!("composite mode {} is unsupported", d.composite_mode)));
    }
    if !(blend_domain && d.mode == 30 && d.composite_space == 3) {
        working(d, d.composite_space)?;
    }
    if !(blend_domain && matches!(d.mode, 24..=27) && d.blend_space == -3) {
        working(d, d.blend_space)?;
    }
    if let Some(m) = &d.mask {
        if !m.unsupported.is_empty() {
            return Err(unsupported(m, "unsupported layer mask semantics"));
        }
        if m.width != d.width || m.height != d.height {
            return Err(unsupported(d, "mask/layer dimensions differ"));
        }
    }
    Ok(())
}
pub(crate) fn bounds<P>(d: &Drawable<P>) -> Result<Rect, IoError> {
    let x1 = d
        .offset
        .0
        .checked_add(i32::try_from(d.width).map_err(|_| unsupported(d, "width exceeds coordinates"))?)
        .ok_or_else(|| unsupported(d, "horizontal bounds overflow"))?;
    let y1 = d
        .offset
        .1
        .checked_add(i32::try_from(d.height).map_err(|_| unsupported(d, "height exceeds coordinates"))?)
        .ok_or_else(|| unsupported(d, "vertical bounds overflow"))?;
    let r = Rect::new(d.offset.0, d.offset.1, x1, y1);
    if [r.x0, r.x1, r.y0, r.y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) {
        return Err(unsupported(d, "offsets exceed safe raster coordinates"));
    }
    Ok(r)
}
fn depth(s: Sample) -> SampleType {
    match s {
        Sample::U8 => SampleType::U8,
        Sample::U16 => SampleType::U16,
        Sample::U32 => SampleType::U32,
        Sample::F64 => SampleType::F64,
        Sample::F16 | Sample::F32 => SampleType::F32,
    }
}

/// Own source/edit observations bind these masks' local extent, world placement,
/// zero outside value and explicit Normal operators at qualified integer/float depths.
pub(super) fn exact_half_unit_f32(value: f32) -> bool {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return false;
    }
    let bits = value.to_bits();
    if bits & 0x7fff_ffff == 0 {
        return true;
    }
    let exponent = (bits >> 23) & 255;
    match exponent {
        103..=112 => {
            let significand = (bits & 0x7f_ffff) | (1 << 23);
            let shift = 126 - exponent;
            significand & ((1u32 << shift) - 1) == 0
        }
        113..=127 => bits & 0x1fff == 0,
        _ => false,
    }
}
fn qualified_known_normal_profile(file: &XcfFile) -> bool {
    match (file.model, file.icc.len()) {
        (Model::Rgb, 624) => {
            matches!(file.encoding, Encoding::NonlinearProfile | Encoding::Perceptual | Encoding::Linear)
                && blake3::hash(&file.icc).to_hex().as_str() == "7d1917bc93b917b82acf448b194b6d42fb700be7966e5432cd4c61c31855278b"
        }
        (Model::Gray, 448) => {
            matches!(file.encoding, Encoding::NonlinearProfile | Encoding::Perceptual | Encoding::Linear)
                && blake3::hash(&file.icc).to_hex().as_str() == "38a450310b9d79e489a2d65ac5797cd49d8990fad78df53d8cd39e204b1b1b86"
        }
        (Model::Rgb, 676) => {
            matches!(file.encoding, Encoding::NonlinearProfile | Encoding::Perceptual | Encoding::Linear)
                && blake3::hash(&file.icc).to_hex().as_str() == "a4f856836dc373d70a40f292cad416e038960fad2eee3e6b6c1136f499981018"
        }
        (Model::Gray, 436) => {
            matches!(file.encoding, Encoding::NonlinearProfile | Encoding::Perceptual | Encoding::Linear)
                && blake3::hash(&file.icc).to_hex().as_str() == "227efe782313ad03c3c87ccbb8440de0b720a0a9ab377e8cc1b339d722058677"
        }
        (Model::Rgb, 25264) => {
            file.encoding == Encoding::NonlinearProfile
                && blake3::hash(&file.icc).to_hex().as_str() == "237cf1a7da0ef9ee3071c9017a14804ae700739aeb27526444a581d9476995dd"
        }
        (Model::Gray, 8668) => {
            file.encoding == Encoding::NonlinearProfile
                && blake3::hash(&file.icc).to_hex().as_str() == "50e9b5018d011d123f46871fe2e46da7fc1402ad6a4b8d886389a5149a0a03bb"
        }
        (Model::Rgb, 764) => {
            matches!(file.encoding, Encoding::NonlinearProfile | Encoding::Linear | Encoding::Perceptual)
                && blake3::hash(&file.icc).to_hex().as_str() == "2e36900308b993673310d2336c6376b305c0e283ab2c05b78e5f63a7ab2a1a6b"
        }
        (Model::Gray, 492) => {
            matches!(file.encoding, Encoding::NonlinearProfile | Encoding::Linear | Encoding::Perceptual)
                && blake3::hash(&file.icc).to_hex().as_str() == "37e49cb13479811d7475d92861067e5eeba73e6b1ededb355a6a2554cfcae140"
        }
        _ => false,
    }
}

pub(crate) fn qualified_analytic_normal_profile(file: &XcfFile) -> bool {
    if file.version != 12
        || !matches!(
            (file.precision, file.sample, file.encoding),
            (250, Sample::U16, Encoding::NonlinearProfile)
                | (350, Sample::U32, Encoding::NonlinearProfile)
                | (650, Sample::F32, Encoding::NonlinearProfile)
                | (750, Sample::F64, Encoding::NonlinearProfile)
                | (600, Sample::F32, Encoding::Linear)
                | (700, Sample::F64, Encoding::Linear)
                | (675, Sample::F32, Encoding::Perceptual)
                | (775, Sample::F64, Encoding::Perceptual)
        )
        || !(132..=65536).contains(&file.icc.len())
    {
        return false;
    }
    qualified_analytic_normal_icc(file)
}

/// Semantic profile qualification after the caller validates its source epoch.
pub(crate) fn qualified_analytic_normal_icc(file: &XcfFile) -> bool {
    if !(132..=65536).contains(&file.icc.len()) {
        return false;
    }
    let expected = match file.model {
        Model::Rgb => photocraft_cms::ColorSpace::Rgb,
        Model::Gray => photocraft_cms::ColorSpace::Gray,
        _ => return false,
    };
    photocraft_cms::xcf_color::parse_bounded_profile(&file.icc, expected)
        .is_ok_and(|profile| photocraft_cms::xcf_color::qualify_analytic_profile(&profile).is_ok())
}

fn plane_matches(d: &Drawable, sample: Sample, channels: usize) -> bool {
    d.plane.as_ref().is_some_and(|plane| {
        let bytes = match sample {
            Sample::U8 => 1,
            Sample::U16 => 2,
            Sample::U32 => 4,
            Sample::F16 | Sample::F32 => 4,
            Sample::F64 => 8,
        };
        let expected = usize::try_from(d.width)
            .ok()
            .and_then(|width| usize::try_from(d.height).ok().and_then(|height| width.checked_mul(height)))
            .and_then(|pixels| pixels.checked_mul(channels))
            .and_then(|samples| samples.checked_mul(bytes));
        plane.sample == if sample == Sample::F16 { Sample::F32 } else { sample }
            && plane.channels == channels
            && plane.width == d.width
            && plane.height == d.height
            && expected == Some(plane.bytes.len())
            && plane.bytes.chunks_exact(bytes).all(|raw| {
                if matches!(sample, Sample::U8 | Sample::U16 | Sample::U32) {
                    return true;
                }
                let value = if sample == Sample::F64 {
                    let Ok(raw) = raw.try_into() else { return false };
                    f64::from_ne_bytes(raw)
                } else {
                    let Ok(raw) = raw.try_into() else { return false };
                    f64::from(f32::from_ne_bytes(raw))
                };
                value.is_finite() && (0.0..=1.0).contains(&value) && (sample != Sample::F16 || exact_half_unit_f32(value as f32))
            })
    })
}
fn qualified_normal_mask(file: &XcfFile) -> bool {
    matches!((file.version, file.sample), (11, Sample::U8) | (12, Sample::U16 | Sample::U32 | Sample::F16 | Sample::F32 | Sample::F64))
        && matches!(
            (file.precision, file.sample, file.encoding),
            (100, Sample::U8, Encoding::Linear)
                | (150, Sample::U8, Encoding::NonlinearProfile)
                | (175, Sample::U8, Encoding::Perceptual)
                | (375, Sample::U32, Encoding::Perceptual)
                | (575, Sample::F16, Encoding::Perceptual)
                | (200, Sample::U16, Encoding::Linear)
                | (250, Sample::U16, Encoding::NonlinearProfile)
                | (275, Sample::U16, Encoding::Perceptual)
                | (675, Sample::F32, Encoding::Perceptual)
                | (775, Sample::F64, Encoding::Perceptual)
                | (300, Sample::U32, Encoding::Linear)
                | (350, Sample::U32, Encoding::NonlinearProfile)
                | (500, Sample::F16, Encoding::Linear)
                | (550, Sample::F16, Encoding::NonlinearProfile)
                | (650, Sample::F32, Encoding::NonlinearProfile)
                | (750, Sample::F64, Encoding::NonlinearProfile)
                | (600, Sample::F32, Encoding::Linear)
                | (700, Sample::F64, Encoding::Linear)
        )
        && matches!(file.model, Model::Rgb | Model::Gray)
        && (file.icc.is_empty()
            || matches!(file.sample, Sample::F32 | Sample::F64) && (qualified_known_normal_profile(file) || qualified_analytic_normal_profile(file)))
        && (file.width, file.height) == (16, 12)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 2
        && file.layers.iter().all(|d| {
            !d.group
                && d.path.len() <= 1
                && d.effects.is_empty()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.visible
                && d.floating_attachment.is_none()
                && d.model == file.model
                && d.alpha
                && d.mode == 28
                && d.blend_space == 0
                && plane_matches(d, file.sample, if file.model == Model::Gray { 2 } else { 4 })
        })
        && file.layers.first().is_some_and(|d| {
            d.composite_space == 1
                && matches!(d.composite_mode, 1..=4)
                && (d.width, d.height) == (10, 9)
                && ((d.offset == (-2, -1) && d.opacity.to_bits() == 0.7125_f32.to_bits())
                    || (d.offset == (-1, 2) && d.opacity.to_bits() == 0.4375_f32.to_bits()))
                && d.mask_enabled
                && !d.mask_shown
                && !d.mask_editing
                && d.mask.as_ref().is_some_and(|mask| {
                    (mask.width, mask.height) == (10, 9)
                        && mask.offset == (0, 0)
                        && mask.model == Model::Gray
                        && !mask.alpha
                        && mask.visible
                        && mask.effects.is_empty()
                        && mask.text.is_none()
                        && mask.vector.is_none()
                        && mask.link.is_none()
                        && mask.transform.is_none()
                        && mask.floating_attachment.is_none()
                        && plane_matches(mask, file.sample, 1)
                })
        })
        && file.layers.get(1).is_some_and(|d| {
            (d.width, d.height) == (16, 12)
                && d.offset == (0, 0)
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && d.mask.is_none()
                && (d.composite_space, d.composite_mode) == (-1, -1)
        })
}
fn qualified_analytic_normal_geometry(file: &XcfFile) -> bool {
    fn bounded(width: u32, height: u32) -> bool {
        width > 0 && height > 0 && width.checked_mul(height).is_some_and(|pixels| pixels <= 4096)
    }
    fn plain(d: &Drawable) -> bool {
        !d.group
            && d.path.len() <= 1
            && d.effects.is_empty()
            && d.text.is_none()
            && d.vector.is_none()
            && d.link.is_none()
            && d.transform.is_none()
            && d.visible
            && d.floating_attachment.is_none()
    }
    file.version == 12
        && matches!(
            (file.precision, file.sample, file.encoding),
            (250, Sample::U16, Encoding::NonlinearProfile)
                | (350, Sample::U32, Encoding::NonlinearProfile)
                | (650, Sample::F32, Encoding::NonlinearProfile)
                | (750, Sample::F64, Encoding::NonlinearProfile)
                | (600, Sample::F32, Encoding::Linear)
                | (700, Sample::F64, Encoding::Linear)
                | (675, Sample::F32, Encoding::Perceptual)
                | (775, Sample::F64, Encoding::Perceptual)
        )
        && matches!(file.model, Model::Rgb | Model::Gray)
        && qualified_analytic_normal_profile(file)
        && bounded(file.width, file.height)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 2
        && file.layers.iter().all(|d| {
            plain(d)
                && d.model == file.model
                && d.alpha
                && d.mode == 28
                && d.blend_space == 0
                && bounded(d.width, d.height)
                && bounds(d).is_ok()
                && plane_matches(d, file.sample, if file.model == Model::Gray { 2 } else { 4 })
        })
        && file.layers.first().is_some_and(|d| {
            d.composite_space == 1
                && matches!(d.composite_mode, 1..=4)
                && d.opacity.is_finite()
                && (0.0..=1.0).contains(&d.opacity)
                && file
                    .width
                    .checked_mul(file.height)
                    .and_then(|canvas| d.width.checked_mul(d.height).and_then(|owner| owner.checked_mul(2)).and_then(|owners| canvas.checked_add(owners)))
                    .is_some_and(|pixels| pixels <= 8192)
                && d.mask_enabled
                && !d.mask_shown
                && !d.mask_editing
                && d.mask.as_ref().is_some_and(|mask| {
                    plain(mask)
                        && (mask.width, mask.height) == (d.width, d.height)
                        && mask.offset == (0, 0)
                        && mask.model == Model::Gray
                        && !mask.alpha
                        && plane_matches(mask, file.sample, 1)
                })
        })
        && file.layers.get(1).is_some_and(|d| {
            (d.width, d.height) == (file.width, file.height)
                && d.offset == (0, 0)
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && d.mask.is_none()
                && (d.composite_space, d.composite_mode) == (-1, -1)
        })
}
fn qualified_finite_normal_source(file: &XcfFile) -> bool {
    file.version == 12
        && file.precision == 750
        && file.sample == Sample::F64
        && file.encoding == Encoding::NonlinearProfile
        && matches!(file.model, Model::Rgb | Model::Gray)
        && file.icc.is_empty()
        && (file.width, file.height) == (16, 14)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.model == file.model
                && d.alpha
                && d.visible
                && d.mode == 28
                && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && matches!((d.width, d.height, d.offset), (9, 8, (-2, 1)) | (11, 11, (-3, 0)) | (8, 8, (-1, 1)))
                && d.mask.is_none()
                && d.effects.is_empty()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.floating_attachment.is_none()
                && d.plane.as_ref().is_some_and(|p| {
                    let channels = if file.model == Model::Gray { 2 } else { 4 };
                    p.sample == Sample::F64
                        && p.channels == channels
                        && p.width == d.width
                        && p.height == d.height
                        && usize::try_from(d.width)
                            .ok()
                            .and_then(|w| usize::try_from(d.height).ok().and_then(|h| w.checked_mul(h)))
                            .and_then(|pixels| pixels.checked_mul(channels))
                            .and_then(|samples| samples.checked_mul(8))
                            == Some(p.bytes.len())
                        && p.bytes.as_chunks::<8>().0.iter().enumerate().all(|(index, raw)| {
                            let value = f64::from_ne_bytes(*raw);
                            value.is_finite() && (index % channels == channels - 1 || (0.0..=1.0).contains(&value))
                        })
                })
        })
}
fn validate_drawable_alpha<P: ExactParsedPlane>(d: &Drawable<P>, allow: bool, coverage_plane: bool, ctl: &Interrupt<'_>, depth: usize) -> Result<(), IoError> {
    if depth > 100 {
        return Err(unsupported(d, "alpha validation depth"));
    }
    if (d.alpha || coverage_plane)
        && let Some(p) = &d.plane
    {
        let bytes = match p.descriptor().sample {
            Sample::F32 => 4,
            Sample::F64 => 8,
            _ => 0,
        };
        if bytes != 0 {
            let stride = p.descriptor().channels.checked_mul(bytes).filter(|n| *n > 0).ok_or_else(|| unsupported(d, "alpha sample stride"))?;
            p.visit_native(ctl, &mut |block| {
                for raw in block.chunks_exact(stride) {
                    let alpha =
                        if coverage_plane { raw.get(..bytes) } else { raw.get(stride - bytes..) }.ok_or_else(|| unsupported(d, "alpha sample bytes"))?;
                    let alpha = if bytes == 8 {
                        f64::from_ne_bytes(alpha.try_into().map_err(|_| unsupported(d, "double alpha bytes"))?)
                    } else {
                        f64::from(f32::from_ne_bytes(alpha.try_into().map_err(|_| unsupported(d, "float alpha bytes"))?))
                    };
                    if !alpha.is_finite() || !allow && !(0.0..=1.0).contains(&alpha) {
                        return Err(unsupported(d, "source alpha outside qualified coverage domain"));
                    }
                }
                Ok(())
            })?;
        }
    }
    if let Some(mask) = &d.mask {
        validate_drawable_alpha(mask, false, true, ctl, depth + 1)?;
    }
    for effect in &d.effects {
        if let Some(mask) = &effect.mask {
            validate_drawable_alpha(mask, false, true, ctl, depth + 1)?;
        }
    }
    Ok(())
}
fn qualified_gray_mask(file: &XcfFile) -> bool {
    file.version == 12
        && file.model == Model::Gray
        && file.sample == Sample::F32
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 16
        && file.height == 12
        && file.layers.len() == 2
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.iter().all(|d| {
            !d.group
                && d.path.len() <= 1
                && d.effects.is_empty()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.visible
                && d.floating_attachment.is_none()
                && d.model == Model::Gray
                && d.alpha
                && d.plane.as_ref().is_some_and(|p| p.sample == Sample::F32 && p.channels == 2)
        })
        && file.layers.first().is_some_and(|d| {
            ((d.mode == 30 && d.blend_space == -1 && d.composite_mode == -2) || (d.mode == 28 && d.blend_space == 0 && d.composite_mode == -1))
                && d.composite_space == -1
                && d.width == 10
                && d.height == 9
                && ((d.offset == (2, -1) && d.opacity.to_bits() == 0.7125_f32.to_bits())
                    || (d.offset == (-1, 2) && d.opacity.to_bits() == 0.4375_f32.to_bits()))
                && d.mask_enabled
                && !d.mask_shown
                && d.mask_editing
                && d.mask.as_ref().is_some_and(|m| {
                    m.width == 10
                        && m.height == 9
                        && m.offset == (0, 0)
                        && m.model == Model::Gray
                        && !m.alpha
                        && m.plane.as_ref().is_some_and(|p| p.sample == Sample::F32 && p.channels == 1)
                })
        })
        && file.layers.get(1).is_some_and(|d| {
            d.mode == 28
                && d.width == 16
                && d.height == 12
                && d.offset == (0, 0)
                && d.opacity == 1.0
                && d.mask.is_none()
                && d.blend_space == 0
                && d.composite_space == -1
                && d.composite_mode == -1
        })
}
fn qualified_masked_group(file: &XcfFile) -> bool {
    file.version == 13 && qualified_group_geometry(file, false)
}
fn qualified_gaussian_group(file: &XcfFile) -> bool {
    file.version == 22 && qualified_group_geometry(file, true)
}
fn qualified_group_geometry(file: &XcfFile, gaussian: bool) -> bool {
    if file.model != Model::Rgb
        || file.sample != Sample::F32
        || file.encoding != Encoding::NonlinearProfile
        || !file.icc.is_empty()
        || file.width != 16
        || file.height != 12
        || file.layers.len() != 4
        || !file.channels.is_empty()
        || !file.paths.is_empty()
    {
        return false;
    }
    file.layers.iter().enumerate().all(|(index, d)| {
        let effects_qualified = if gaussian && index == 0 {
            d.effects.len() == 1 && d.effects.first().is_some_and(|e| e.operation == "gegl:gaussian-blur" && e.version.as_deref() == Some("0:0"))
        } else {
            d.effects.is_empty()
        };
        if !d.visible || !effects_qualified || d.floating_attachment.is_some() {
            return false;
        }
        if index == 0 {
            d.group
                && (d.mode == 28 || !gaussian && d.mode == 61)
                && d.path.is_empty()
                && d.opacity == 1.0
                && d.blend_space == 0
                && d.composite_space == -1
                && d.composite_mode == -1
                && ((d.width == 12 && d.height == 9 && d.offset == (0, -1)) || (d.width == 12 && d.height == 11 && d.offset == (-1, 0)))
                && d.mask_enabled
                && !d.mask_shown
                && d.mask_editing
                && d.mask.as_ref().is_some_and(|m| {
                    m.width == d.width
                        && m.height == d.height
                        && m.offset == (0, 0)
                        && m.model == Model::Gray
                        && !m.alpha
                        && m.plane.as_ref().is_some_and(|p| p.sample == Sample::F32 && p.channels == 1)
                })
        } else {
            !d.group
                && d.model == Model::Rgb
                && d.alpha
                && d.mask.is_none()
                && d.plane.as_ref().is_some_and(|p| p.sample == Sample::F32 && p.channels == 4)
                && d.composite_space == -1
                && match index {
                    1 => {
                        d.path == [0, 0]
                            && d.mode == 28
                            && d.blend_space == 0
                            && d.composite_mode == -1
                            && d.width == 11
                            && d.height == 8
                            && d.offset == (0, 0)
                            && d.opacity == 1.0
                    }
                    2 => {
                        d.path == [0, 1]
                            && d.mode == 30
                            && d.blend_space == -1
                            && d.composite_mode == -2
                            && d.width == 10
                            && d.height == 9
                            && ((d.offset == (2, -1) && d.opacity.to_bits() == 0.7125_f32.to_bits())
                                || (d.offset == (-1, 2) && d.opacity.to_bits() == 0.4375_f32.to_bits()))
                    }
                    3 => {
                        d.path.is_empty()
                            && d.mode == 28
                            && d.blend_space == 0
                            && d.composite_mode == -1
                            && d.width == 16
                            && d.height == 12
                            && d.offset == (0, 0)
                            && d.opacity == 1.0
                    }
                    _ => false,
                }
        }
    })
}
fn qualified_multiply_spaces(file: &XcfFile) -> bool {
    file.model == Model::Rgb
        && file.sample == Sample::F32
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 16
        && file.height == 12
        && file.layers.len() == 2
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.iter().all(|d| {
            !d.group
                && d.path.len() <= 1
                && d.effects.is_empty()
                && d.visible
                && d.floating_attachment.is_none()
                && d.mask.is_none()
                && d.model == Model::Rgb
                && d.alpha
                && d.plane.as_ref().is_some_and(|p| p.sample == Sample::F32 && p.channels == 4)
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
        })
        && file.layers.first().is_some_and(|d| {
            d.mode == 30
                && d.composite_mode == -2
                && matches!((file.version, d.blend_space, d.composite_space), (12, 2, -1) | (23, 4, -1) | (12, -1, 2) | (23, -1, 4) | (12, -1, 3))
                && d.width == 10
                && d.height == 9
                && ((d.offset == (2, -1) && d.opacity.to_bits() == 0.7125_f32.to_bits())
                    || (d.offset == (-1, 2) && d.opacity.to_bits() == 0.4375_f32.to_bits()))
        })
        && file.layers.get(1).is_some_and(|d| {
            d.mode == 28
                && d.width == 16
                && d.height == 12
                && d.offset == (0, 0)
                && d.opacity == 1.0
                && d.blend_space == 0
                && d.composite_space == -1
                && d.composite_mode == -1
        })
}
/// Native/profile/alpha/HDR source and regeneration witnesses qualify this bounded
/// raster family. Every source sample is checked before a document can be committed.
fn qualified_finite_raster_filter(file: &XcfFile, operation: &str, model: Model) -> bool {
    file.version == 22
        && file.model == model
        && matches!((file.sample, file.precision), (Sample::F32, 650) | (Sample::F64, 750))
        && file.encoding == Encoding::NonlinearProfile
        && file.width == 16
        && file.height == 12
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == model
                && d.width == 16
                && d.height == 12
                && d.offset == (0, 0)
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
                && d.effects.first().is_some_and(|e| e.operation == operation)
                && d.plane.as_ref().is_some_and(|p| {
                    let width = if file.sample == Sample::F64 { 8 } else { 4 };
                    let channels = if model == Model::Gray { 2 } else { 4 };
                    p.sample == file.sample
                        && p.channels == channels
                        && p.bytes.len() == 16 * 12 * channels * width
                        && p.bytes.chunks_exact(width).enumerate().all(|(index, bytes)| {
                            let value = if width == 8 {
                                let Ok(raw) = <[u8; 8]>::try_from(bytes) else { return false };
                                f64::from_ne_bytes(raw)
                            } else {
                                let Ok(raw) = <[u8; 4]>::try_from(bytes) else { return false };
                                f64::from(f32::from_ne_bytes(raw))
                            };
                            value.is_finite() && (index % channels != channels - 1 || (0.0..=1.0).contains(&value))
                        })
                })
        })
}
fn qualified_opaque_neighborhood(file: &XcfFile, operation: &str) -> bool {
    file.version == 22
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 32
        && file.height == 32
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == Model::Rgb
                && d.width == 32
                && d.height == 32
                && d.offset == (0, 0)
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
                && d.effects.first().is_some_and(|e| e.operation == operation)
                && d.plane.as_ref().is_some_and(|p| {
                    p.sample == Sample::U8
                        && p.channels == 4
                        && p.bytes.len() == 32 * 32 * 4
                        && p.bytes.as_chunks::<4>().0.iter().all(|pixel| pixel.last() == Some(&255))
                })
        })
}

/// Independently authored finite HDR clipping witnesses, separate from U8 point effects.
fn qualified_hdr_clip(file: &XcfFile) -> bool {
    file.version == 22
        && file.precision == 600
        && file.sample == Sample::F32
        && file.model == Model::Rgb
        && file.encoding == Encoding::Linear
        && file.icc.is_empty()
        && (file.width, file.height) == (8, 6)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == Model::Rgb
                && (d.width, d.height) == (8, 6)
                && d.offset == (0, 0)
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
                && d.effects.first().is_some_and(|effect| effect.operation == "gegl:rgb-clip")
                && d.plane.as_ref().is_some_and(|plane| {
                    plane.sample == Sample::F32
                        && plane.channels == 4
                        && plane.bytes.len() == 8 * 6 * 16
                        && plane.bytes.as_chunks::<4>().0.iter().enumerate().all(|(index, bytes)| {
                            let value = f32::from_ne_bytes(*bytes);
                            value.is_finite() && (index % 4 != 3 || value.to_bits() == 1.0_f32.to_bits())
                        })
                })
        })
}

/// Independent enable/disable originals bind unit-valued mixed-alpha samples
/// at three declared precisions. Signed HDR and other encodings remain separate.
fn qualified_precision_value_invert(file: &XcfFile) -> bool {
    qualified_precision_point(file, "gegl:value-invert")
}
fn qualified_precision_brightness_contrast(file: &XcfFile) -> bool {
    qualified_precision_point(file, "gegl:brightness-contrast")
}
fn qualified_precision_gegl_levels(file: &XcfFile) -> bool {
    qualified_precision_point(file, "gegl:levels")
}
fn qualified_precision_exposure_black(file: &XcfFile) -> bool {
    qualified_precision_point(file, "gegl:exposure")
}
fn qualified_precision_point(file: &XcfFile, operation: &str) -> bool {
    qualified_precision_point_impl(file, operation, false, false)
}
fn qualified_precision_point_impl(file: &XcfFile, operation: &str, balance_depth: bool, exposure_geometry: bool) -> bool {
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
/// Saved-source and edit witnesses qualify this varying-alpha, noncommuting stack.
/// Its opaque envelope is deliberately distinct from the ordinary point epoch.
fn qualified_point_stack_edits(file: &XcfFile) -> bool {
    file.version == 22
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && (file.width, file.height) == (8, 6)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == Model::Rgb
                && (d.width, d.height) == (8, 6)
                && d.offset == (0, 0)
                && d.mode == 28
                && (d.blend_space, d.composite_space, d.composite_mode) == (0, 2, 1)
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && d.mask.is_none()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.floating_attachment.is_none()
                && matches!(d.effects.len(), 1..=2)
                && d.effects.iter().all(|e| matches!(e.operation.as_str(), "gegl:exposure" | "gegl:invert-linear"))
                && d.plane.as_ref().is_some_and(|p| p.sample == Sample::U8 && p.channels == 4 && p.bytes.len() == 8 * 6 * 4)
        })
}
fn qualified_rectilinear_text_context(file: &XcfFile) -> bool {
    file.version == 19
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && matches!((file.width, file.height), (128, 64) | (128, 128))
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == Model::Rgb
                && matches!(d.offset, (8, 4) | (11, 2) | (-5, 7))
                && match file.height {
                    64 => matches!(d.height, 20 | 30 | 39 | 49),
                    128 => d.offset == (8, 4) && matches!(d.height, 40 | 42 | 44 | 78 | 80 | 82 | 98 | 100 | 102),
                    _ => false,
                }
                && (1..=512).contains(&d.width)
                && d.mode == 28
                && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
                && d.opacity.to_bits() == 1_f32.to_bits()
                && d.mask.is_none()
                && d.effects.is_empty()
                && d.text.is_some()
                && d.text_flags == 0
                && !d.text_source_modified()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.floating_attachment.is_none()
                && d.plane.as_ref().is_some_and(|plane| {
                    plane.sample == Sample::U8
                        && plane.channels == 4
                        && plane.width == d.width
                        && plane.height == d.height
                        && usize::try_from(d.width)
                            .ok()
                            .and_then(|width| usize::try_from(d.height).ok().and_then(|height| width.checked_mul(height)))
                            .and_then(|pixels| pixels.checked_mul(4))
                            == Some(plane.bytes.len())
                })
        })
}

/// Separate authored underline context; the existing rectilinear guard stays unchanged.
fn qualified_underline_text_context(file: &XcfFile) -> bool {
    file.version == 19
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && (file.width, file.height) == (128, 64)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == Model::Rgb
                && d.offset == (8, 4)
                && matches!(d.height, 20 | 39)
                && (1..=512).contains(&d.width)
                && d.mode == 28
                && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
                && d.opacity.to_bits() == 1_f32.to_bits()
                && d.mask.is_none()
                && d.effects.is_empty()
                && d.text.is_some()
                && matches!(d.text_flags, 0 | 1)
                && !d.text_source_modified()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.floating_attachment.is_none()
                && d.plane.as_ref().is_some_and(|p| {
                    p.sample == Sample::U8
                        && p.channels == 4
                        && p.width == d.width
                        && p.height == d.height
                        && (d.width as usize).checked_mul(d.height as usize).and_then(|n| n.checked_mul(4)) == Some(p.bytes.len())
                })
        })
}

fn qualified_vector_context(file: &XcfFile) -> bool {
    file.version == 24
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.channels.is_empty()
        && file.width == 64
        && file.height == 48
        && file.layers.len() == 1
        && file.paths.len() == 1
        && file.layers.first().is_some_and(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == Model::Rgb
                && d.mode == 28
                && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && d.mask.is_none()
                && d.effects.is_empty()
                && d.text.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.floating_attachment.is_none()
                && d.vector.as_ref().is_some_and(|v| !v.source_modified)
                && d.plane.as_ref().is_some_and(|p| p.sample == Sample::U8 && p.channels == 4)
        })
}
fn qualified_composition_command(file: &XcfFile) -> bool {
    matches!(file.model, Model::Rgb | Model::Gray)
        && matches!(
            (file.version, file.sample, file.precision),
            (11, Sample::U8, 150) | (12, Sample::U16, 250) | (12, Sample::U32, 350) | (12, Sample::F32, 650) | (12, Sample::F64, 750)
        )
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 8
        && file.height == 6
        && file.layers.len() == 2
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.iter().all(|d| {
            !d.group
                && d.path.len() <= 1
                && d.visible
                && d.alpha
                && d.model == file.model
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && d.mask.is_none()
                && d.effects.is_empty()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.floating_attachment.is_none()
                && d.plane.as_ref().is_some_and(|p| p.sample == file.sample && p.channels == if file.model == Model::Gray { 2 } else { 4 })
        })
        && file.layers.first().is_some_and(|d| {
            d.width == 3 && d.height == 2 && d.offset == (2, 1) && d.mode == 30 && (d.blend_space, d.composite_space, d.composite_mode) == (2, 2, 1)
        })
        && file.layers.get(1).is_some_and(|d| {
            d.width == 8 && d.height == 6 && d.offset == (0, 0) && d.mode == 28 && (d.blend_space, d.composite_space, d.composite_mode) == (0, 2, 1)
        })
}
fn surface(b: &mut Budget, plane: &Plane, r: Rect, mode: ColorMode, alpha: bool, bytes: &[u8], ctl: &Interrupt) -> Result<Surface, IoError> {
    let fmt = PixelFormat::new(mode, depth(plane.sample), alpha);
    let ts = photocraft_geom::TILE_SIZE as usize;
    // Includes padded tiles, tile/tree overhead, and the ingestion API's temporary tile buffer.
    let cols = (r.x1 - 1).div_euclid(ts as i32) - r.x0.div_euclid(ts as i32) + 1;
    let rows = (r.y1 - 1).div_euclid(ts as i32) - r.y0.div_euclid(ts as i32) + 1;
    let tiles = (cols as usize).checked_mul(rows as usize).ok_or_else(|| IoError::Unsupported("XCF tile count overflow".into()))?;
    let tile_bytes =
        ts.checked_mul(ts).and_then(|n| n.checked_mul(fmt.bytes_per_pixel())).ok_or_else(|| IoError::Unsupported("XCF tile bytes overflow".into()))?;
    b.charge(
        tiles.checked_mul(tile_bytes + 128).and_then(|n| n.checked_add(tile_bytes)).ok_or_else(|| IoError::Unsupported("XCF resident tile overflow".into()))?,
    )?;
    Surface::try_from_interleaved_with(fmt, r, bytes, ctl).map_err(|e| {
        if matches!(e, photocraft_raster::RasterInputError::Cancelled) {
            IoError::Cancelled
        } else {
            IoError::Unsupported(format!("XCF raster ingestion: {e}"))
        }
    })
}
fn raster<P>(d: &Drawable<P>, plane: &Plane, palette: &[[u8; 3]], b: &mut Budget, ctl: &Interrupt) -> Result<(Surface, Option<IndexedSource>), IoError> {
    let r = bounds(d)?;
    let mode = match d.model {
        Model::Rgb => ColorMode::Rgb,
        Model::Gray => ColorMode::Grayscale,
        Model::Indexed => ColorMode::Rgb,
    };
    if d.model == Model::Indexed {
        let n = (d.width as usize).checked_mul(d.height as usize).ok_or_else(|| unsupported(d, "indexed plane overflow"))?;
        let mut expanded = b.bytes(n.checked_mul(4).ok_or_else(|| unsupported(d, "expanded indexed size overflow"))?)?;
        let mut indices = b.bytes(n)?;
        let mut alpha = if d.alpha { Some(b.bytes(n)?) } else { None };
        for (i, source) in plane.bytes.chunks_exact(plane.channels).enumerate() {
            if i.is_multiple_of(4096) {
                ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            let index = source.first().copied().ok_or_else(|| unsupported(d, "missing palette index"))?;
            let color = palette.get(index as usize).ok_or_else(|| unsupported(d, format!("palette index {index} out of range")))?;
            let output = expanded.get_mut(i * 4..i * 4 + 4).ok_or_else(|| unsupported(d, "indexed output bounds"))?;
            output.get_mut(..3).ok_or_else(|| unsupported(d, "indexed RGB"))?.copy_from_slice(color);
            let coverage = if d.alpha { source.get(1).copied().ok_or_else(|| unsupported(d, "missing indexed alpha"))? } else { 255 };
            *output.get_mut(3).ok_or_else(|| unsupported(d, "indexed alpha output"))? = coverage;
            *indices.get_mut(i).ok_or_else(|| unsupported(d, "index sidecar bounds"))? = index;
            if let Some(a) = alpha.as_mut() {
                *a.get_mut(i).ok_or_else(|| unsupported(d, "alpha sidecar bounds"))? = coverage;
            }
        }
        let s = surface(b, plane, r, mode, true, &expanded, ctl)?;
        b.charge(palette.len() * 3 + (256 * 256 * 4))?;
        let side = IndexedSource::new_with_interrupt(Arc::new(indices), alpha.map(Arc::new), palette.to_vec(), r, &s, ctl)
            .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { unsupported(d, e) })?;
        let temporary = expanded.len();
        drop(expanded);
        b.release(temporary + (256 * 256 * 4))?;
        Ok((s, Some(side)))
    } else if d.alpha {
        Ok((surface(b, plane, r, mode, true, &plane.bytes, ctl)?, None))
    } else {
        let source_stride = plane.channels * plane.sample.source_bytes();
        let target_stride = (plane.channels + 1) * plane.sample.source_bytes();
        let n = plane.bytes.len() / source_stride;
        let mut expanded = b.bytes(n.checked_mul(target_stride).ok_or_else(|| unsupported(d, "opaque plane overflow"))?)?;
        for (i, (src, dst)) in plane.bytes.chunks_exact(source_stride).zip(expanded.chunks_exact_mut(target_stride)).enumerate() {
            if i.is_multiple_of(4096) {
                ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            dst.get_mut(..source_stride).ok_or_else(|| unsupported(d, "opaque output color"))?.copy_from_slice(src);
            let a = dst.get_mut(source_stride..).ok_or_else(|| unsupported(d, "opaque alpha"))?;
            match plane.sample {
                Sample::U8 => a.fill(255),
                Sample::U16 => a.copy_from_slice(&u16::MAX.to_ne_bytes()),
                Sample::U32 => a.copy_from_slice(&u32::MAX.to_ne_bytes()),
                Sample::F64 => a.copy_from_slice(&1.0f64.to_ne_bytes()),
                Sample::F16 | Sample::F32 => a.copy_from_slice(&1.0f32.to_ne_bytes()),
            }
        }
        let s = surface(b, plane, r, mode, true, &expanded, ctl)?;
        let temporary = expanded.len();
        drop(expanded);
        b.release(temporary)?;
        Ok((s, None))
    }
}
fn insert(layers: &mut Vec<Layer>, path: &[u32], layer: Layer) -> Result<(), IoError> {
    let Some((&first, rest)) = path.split_first() else {
        return Err(IoError::Unsupported("XCF empty layer path".into()));
    };
    if rest.is_empty() {
        if first as usize != layers.len() {
            return Err(IoError::Unsupported("XCF duplicate/noncontiguous item path".into()));
        }
        layers.try_reserve_exact(1).map_err(|_| IoError::Unsupported("XCF group allocation failed".into()))?;
        layers.push(layer);
        return Ok(());
    }
    let parent = layers.get_mut(first as usize).ok_or_else(|| IoError::Unsupported("XCF item path references missing parent".into()))?;
    let LayerContent::Group(g) = &mut parent.content else {
        return Err(IoError::Unsupported("XCF item path parent is not a group".into()));
    };
    insert(&mut g.children, rest, layer)
}
fn reverse(layers: &mut [Layer]) {
    layers.reverse();
    for l in layers {
        if let LayerContent::Group(g) = &mut l.content {
            reverse(&mut g.children);
        }
    }
}
#[cfg(test)]
fn map(name: &str, file: XcfFile, ctl: &Interrupt<'_>, resident: usize) -> Result<(ImportResult, usize), IoError> {
    map_accounted(name, file, ctl, resident, None, None::<(&photocraft_xcf::Budget<'_>, ())>)
}
// Appearance and anchoring have independent original-file gates. This admits
// only the observed plain Normal/Union floating source domain, with record
// pointers resolved to freshly minted canonical IDs below.
fn qualified_floating(file: &XcfFile) -> Option<(u64, u64)> {
    if !matches!(file.model, Model::Rgb | Model::Gray)
        || file.sample == Sample::F16
        || file.encoding != Encoding::NonlinearProfile
        || !file.icc.is_empty()
        || (file.width, file.height) != (8, 6)
        || file.layers.len() != 2
        || !file.channels.is_empty()
        || !file.paths.is_empty()
    {
        return None;
    }
    let floating = file.layers.first()?;
    let target = file.layers.get(1)?;
    if floating.record_offset == 0
        || target.record_offset == 0
        || floating.record_offset == target.record_offset
        || floating.floating_attachment != Some(target.record_offset)
        || target.floating_attachment.is_some()
        || (floating.width, floating.height, floating.offset) != (3, 2, (2, 1))
        || (target.width, target.height, target.offset) != (8, 6, (0, 0))
        || !file.layers.iter().all(|d| {
            !d.group
                && d.path.len() <= 1
                && d.model == file.model
                && d.alpha
                && d.visible
                && d.opacity == 1.0
                && d.mode == 28
                && d.blend_space == 0
                && d.composite_space == 2
                && d.composite_mode == 1
                && d.mask.is_none()
                && d.effects.is_empty()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.plane.as_ref().is_some_and(|p| p.sample == file.sample && p.channels == if file.model == Model::Gray { 2 } else { 4 })
        })
    {
        return None;
    }
    Some((floating.record_offset, target.record_offset))
}

struct SharedPreparation<'b, 'a> {
    shared: &'b photocraft_xcf::Budget<'a>,
    failure: Option<XcfError>,
}
impl<'b, 'a> photocraft_compose::gimp_render::PreparationBudget for SharedPreparation<'b, 'a> {
    type Lease = photocraft_xcf::Reservation<'b, 'a>;
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> photocraft_compose::gimp_composite::Result<Self::Lease> {
        self.shared.reserve(photocraft_xcf::BudgetKind::Resident, bytes, "ordinary renderer preparation").map_err(|error| {
            let raster = if error.kind == ErrorKind::Cancelled {
                photocraft_raster::RasterInputError::Cancelled
            } else {
                photocraft_raster::RasterInputError::LimitExceeded
            };
            self.failure = Some(error);
            raster.into()
        })
    }
    fn consume_preparation_work(&mut self, units: u64) -> photocraft_compose::gimp_composite::Result<()> {
        self.shared.consume_work(units, "ordinary renderer validation").map_err(|error| {
            let raster = if error.kind == ErrorKind::Cancelled {
                photocraft_raster::RasterInputError::Cancelled
            } else {
                photocraft_raster::RasterInputError::LimitExceeded
            };
            self.failure = Some(error);
            raster.into()
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn map_accounted<P: MappingPlane, S>(
    name: &str,
    mut file: XcfFile<P>,
    ctl: &Interrupt<'_>,
    resident: usize,
    shared: Option<&dyn Fn(usize) -> Result<(), IoError>>,
    preparation: Option<(&photocraft_xcf::Budget<'_>, S)>,
) -> Result<(ImportResult, usize), IoError> {
    let admission::AdmissionDomains {
        point_replace_domain,
        unpremultiply_domain,
        finite_normal_domain,
        floating_records,
        gray_mask_domain,
        analytic_geometry_domain,
        ordinary_geometry_domain,
        normal_mask_domain,
        analytic_normal_domain,
        gaussian_group_domain,
        accounted_group_domain,
        finite_gaussian_domain,
        finite_box_domain,
        finite_median_domain,
        gray_median_domain,
        opaque_box_domain,
        opaque_median_domain,
        opaque_motion_domain,
        opaque_pixelize_domain,
        vector_domain,
        stem_text_domain,
        masked_group_domain,
        multiply_spaces_domain,
        composition_command_domain,
    } = P::admissions(&file, ctl, preparation.as_ref().map(|(budget, _)| *budget))?;
    let mut floating_ids = (None, None);
    let mut budget = Budget { used: file.charged_bytes, max: resident, peak: file.charged_bytes, shared };
    budget.charge(
        file.icc.len().checked_mul(4).and_then(|n| n.checked_add(1 << 20)).ok_or_else(|| IoError::Unsupported("XCF ICC allocation overflow".into()))?,
    )?;
    let space = if file.model == Model::Gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
    let ce = match file.encoding {
        Encoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
        Encoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        Encoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
    };
    let effective = photocraft_cms::xcf_color::effective_profile(if file.icc.is_empty() { None } else { Some(&file.icc) }, space, ce)
        .map_err(|e| IoError::Unsupported(format!("XCF source profile/encoding: {e}")))?;
    let mode = match file.model {
        Model::Rgb => ColorMode::Rgb,
        Model::Gray => ColorMode::Grayscale,
        Model::Indexed => ColorMode::Indexed,
    };
    let mut doc = Document::new(name, Size::new(file.width, file.height), mode, depth(file.sample));
    let mut state = photocraft_doc::XcfState {
        current_filter_output_precision: if file.sample == Sample::F16
            && matches!(
                point_replace_domain,
                Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(
                    crate::xcf_filters::PointReplaceFamily::ColorToAlpha
                        | crate::xcf_filters::PointReplaceFamily::ColorOverlay
                        | crate::xcf_filters::PointReplaceFamily::MonoMixerSigned
                        | crate::xcf_filters::PointReplaceFamily::ChannelMixer
                        | crate::xcf_filters::PointReplaceFamily::AbsoluteLinear
                        | crate::xcf_filters::PointReplaceFamily::RedEye
                        | crate::xcf_filters::PointReplaceFamily::ComponentExtract
                        | crate::xcf_filters::PointReplaceFamily::ComponentExtractCmyk
                        | crate::xcf_filters::PointReplaceFamily::ComponentExtractYcbcr
                        | crate::xcf_filters::PointReplaceFamily::AlphaClip
                ))
            ) {
            photocraft_doc::FilterOutputPrecision::Half
        } else {
            photocraft_doc::FilterOutputPrecision::Canonical
        },
        provenance: photocraft_doc::xcf_provenance::ProjectProvenance {
            format: "xcf".into(),
            format_version: file.version,
            container: "plain".into(),
            source_label: name.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    preserve_properties(&mut state, std::mem::take(&mut file.raw_properties), None, file.version, &mut budget)?;
    doc.sample_encoding = Some(encoding(file.encoding));
    doc.source_color_context =
        Some(SourceColorContext { original_icc: if file.icc.is_empty() { None } else { Some(Arc::new(file.icc.clone())) }, source_precision: file.precision });
    doc.icc_profile = Some(effective.to_bytes());
    if file.icc.is_empty() {
        let bytes = doc.icc_profile.as_ref().ok_or_else(|| IoError::Unsupported("missing effective XCF profile".into()))?;
        doc.working_profile_interpretation = if file.model == Model::Gray {
            photocraft_doc::WorkingProfileInterpretation::implicit_sgray(bytes)
        } else {
            photocraft_doc::WorkingProfileInterpretation::implicit_srgb(bytes)
        };
    }
    doc.set_physical_resolution(photocraft_doc::PhysicalResolution {
        x_dpi: f64::from(file.dpi.0),
        y_dpi: f64::from(file.dpi.1),
        unit: physical_unit(&file.unit),
    })
    .map_err(|e| IoError::Unsupported(format!("XCF resolution: {e}")))?;
    if mode == ColorMode::Indexed {
        budget.charge(file.palette.len() * 3)?;
        doc.color_table = Some(ColorTable { colors: file.palette.clone(), transparent: None });
    }
    let admission::LateAdmissionDomains {
        point_stack_edits_domain,
        posterize_domain,
        value_invert_domain,
        hdr_clip_domain,
        value_precision_domain,
        brightness_precision_domain,
        levels_precision_domain,
        exposure_geometry_domain,
        exposure_black_domain,
        color_balance_depth_domain,
        color_balance_domain,
        temperature_domain,
        desaturate_domain,
        saturation_yuv_domain,
        saturation_domain,
        saturation_gray_domain,
        threshold_domain,
        threshold_alpha_domain,
        sepia_domain,
        alien_map_domain,
        alien_map_hsl_domain,
        mono_mixer_domain,
        mono_mixer_edits_domain,
        point_context,
        levels_context,
        multiply_domain,
        basic_blend_domain,
    } = P::late_admissions(&file);
    let mut warnings = file.diagnostics;
    if file.sample == Sample::F16 {
        warnings.push("XCF finite F16 samples widened exactly to F32; original precision retained".into());
    }
    let layer_total = file.layers.len();
    let mut next_mask_id =
        u64::try_from(layer_total).ok().and_then(|n| n.checked_add(1)).ok_or_else(|| IoError::Unsupported("XCF mask identity overflow".into()))?;
    let mut next_filter_id = 1_u64;
    let mut set_members = std::collections::BTreeMap::<(u32, u32), Vec<photocraft_doc::item_identity::ItemRef>>::new();
    let mut identities = Vec::<(Vec<u32>, photocraft_doc::LayerId)>::new();
    identities.try_reserve_exact(layer_total).map_err(|_| IoError::Unsupported("XCF identity allocation failed".into()))?;
    state.items.try_reserve_exact(layer_total).map_err(|_| IoError::Unsupported("XCF item state allocation failed".into()))?;
    // Mint canonical paths once, before resolving vector source dependencies.
    let mut mapped_paths = crate::xcf_paths::map_paths(std::mem::take(&mut file.paths), file.version, ctl, &mut |bytes| budget.charge(bytes))?;
    let mut next_resource_id = 1_u64;
    for (i, mut drawable) in file.layers.drain(..).enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let layer_id = photocraft_doc::LayerId::fresh();
        if let Some((floating_record, target_record)) = floating_records {
            if drawable.record_offset == floating_record {
                drawable.unsupported.retain(|reason| reason != "floating selection");
            }
            // The prequalified unique source records are the only identity key.
            if drawable.record_offset == floating_record {
                floating_ids.0 = Some(layer_id);
            }
            if drawable.record_offset == target_record {
                floating_ids.1 = Some(layer_id);
            }
        }
        let d = &drawable;
        let modified_context = if d.text.is_some() || d.vector.is_some() || d.link.is_some() {
            Some(crate::xcf_modified_sources::ModifiedSourceContext {
                mapping: crate::xcf_sources::SourceContext {
                    owner: photocraft_doc::item_identity::ItemRef::Layer(layer_id),
                    vector_path: d.vector.as_ref().and_then(|v| mapped_paths.tattoos.iter().find(|(tattoo, _)| *tattoo == v.path_tattoo).map(|(_, id)| *id)),
                    format: PixelFormat::RGBA8,
                    source_version: file.version,
                },
                source_precision: file.precision,
                source_encoding: file.encoding,
                assigned_profile_present: !file.icc.is_empty(),
            })
        } else {
            None
        };
        let modified = if stem_text_domain {
            Some(crate::xcf_modified_sources::ModifiedSourceKind::Text)
        } else if vector_domain {
            Some(crate::xcf_modified_sources::ModifiedSourceKind::Vector)
        } else {
            modified_context.as_ref().map(|context| crate::xcf_modified_sources::validate_modified_source_with_storage(d, context, ctl)).transpose()?
        };
        let effects_candidate = !d.effects.is_empty()
            && (point_replace_domain.is_some()
                || unpremultiply_domain
                || point_stack_edits_domain
                || finite_gaussian_domain
                || finite_box_domain
                || finite_median_domain
                || gray_median_domain
                || opaque_motion_domain
                || opaque_pixelize_domain
                || gaussian_group_domain && d.group
                || hdr_clip_domain
                || value_precision_domain
                || brightness_precision_domain
                || levels_precision_domain
                || exposure_black_domain
                || exposure_geometry_domain
                || color_balance_domain
                || temperature_domain
                || desaturate_domain
                || saturation_domain
                || saturation_yuv_domain
                || sepia_domain
                || alien_map_domain
                || saturation_gray_domain
                || threshold_domain
                || threshold_alpha_domain
                || P::legacy_point_candidate(d, point_context, levels_context, doc.size)?);

        admit(
            d,
            effects_candidate,
            multiply_domain || basic_blend_domain || gray_mask_domain || masked_group_domain || multiply_spaces_domain || composition_command_domain,
            normal_mask_domain,
            modified,
        )?;
        budget.charge(1024 + d.name.len() + d.path.len() * 4 + d.diagnostics.iter().map(|v| v.len() + std::mem::size_of::<String>()).sum::<usize>())?;
        let (mut drawable, mut l, side) = P::current_owner(drawable, &file.palette, &mut budget, ctl)?;
        l.id = layer_id;
        if let Some(context) = modified_context {
            let plane = drawable.plane.take().ok_or_else(|| IoError::Unsupported("XCF modified source is missing its raster".into()))?.into_original()?;
            let mut fresh = || {
                let id = photocraft_doc::item_identity::ResourceId(next_resource_id);
                next_resource_id = next_resource_id.saturating_add(1);
                id
            };
            let (mut mapped, warning) = if stem_text_domain {
                let mut mapped =
                    crate::xcf_sources::map_source_with_storage(&drawable, Some(plane), context.mapping, ctl, &mut fresh, &mut |bytes| budget.charge(bytes))?
                        .ok_or_else(|| IoError::Unsupported("XCF qualified text source disappeared".into()))?;
                text_preparation::validate(&mapped, &drawable, doc.size, &mut budget, preparation.as_ref().map(|(budget, _)| *budget), ctl)?;
                for record in &mut mapped.opaque {
                    if record.namespace == "gimp/xcf/text-parasite" && record.identity == "gimp-text-layer" && record.source_version == 19 {
                        record.rendering_required = false;
                    }
                }
                (mapped, None)
            } else if vector_domain {
                let mut mapped =
                    crate::xcf_sources::map_source_with_storage(&drawable, Some(plane), context.mapping, ctl, &mut fresh, &mut |bytes| budget.charge(bytes))?
                        .ok_or_else(|| IoError::Unsupported("XCF qualified vector source disappeared".into()))?;
                // Tag47 has been decoded into the current canonical path and complete
                // typed fill/stroke configuration. Retain its bytes as provenance;
                // the final renderer qualifier checks every operative field below.
                for record in &mut mapped.opaque {
                    if record.namespace == "gimp/xcf/source-property" && record.identity == "47" && record.source_version == 24 {
                        record.rendering_required = false;
                    }
                }
                (mapped, None)
            } else {
                let mapped = crate::xcf_modified_sources::map_modified_source_with_storage(&drawable, plane, context, ctl, &mut fresh, &mut |bytes| {
                    budget.charge(bytes)
                })?;
                (mapped.mapped, Some(mapped.warning))
            };
            budget.charge(std::mem::size_of::<photocraft_doc::xcf_items::SourceItem>())?;
            state.source_items.try_reserve_exact(1).map_err(|_| IoError::Unsupported("XCF source item allocation failed".into()))?;
            state.resources.try_reserve_exact(mapped.resources.len()).map_err(|_| IoError::Unsupported("XCF source resource allocation failed".into()))?;
            state.opaque.try_reserve_exact(mapped.opaque.len()).map_err(|_| IoError::Unsupported("XCF source evidence allocation failed".into()))?;
            state.source_items.push(mapped.item);
            state.resources.append(&mut mapped.resources);
            state.opaque.append(&mut mapped.opaque);
            if let Some(warning) = warning {
                warnings.push(warning.into());
            }
        }
        let d = &drawable;
        l.indexed_source = side;
        l.opacity = d.opacity;
        l.visible = d.visible;
        match layer_color_tag(d.color_tag) {
            Ok(label) => l.label = label,
            Err(reason) => warnings.push(format!(
                "XCF layer color tag {}: {reason}; original source tag retained, current label unavailable",
                d.color_tag.unwrap_or_default()
            )),
        }
        l.locks.transparency = d.lock_alpha;
        l.locks.pixels = d.lock_content;
        l.locks.position = d.lock_position;
        let pass = d.group && d.mode == 61;
        l.blend = if pass {
            BlendMode::PassThrough
        } else {
            match d.mode {
                23 => BlendMode::Overlay,
                3 | 30 => BlendMode::Multiply,
                4 | 31 => BlendMode::Screen,
                6 | 32 => BlendMode::Difference,
                7 | 33 => BlendMode::LinearDodge,
                8 | 34 => BlendMode::Subtract,
                9 | 35 => BlendMode::Darken,
                10 | 36 => BlendMode::Lighten,
                41 => BlendMode::Divide,
                42 => BlendMode::ColorDodge,
                43 => BlendMode::ColorBurn,
                44 => BlendMode::HardLight,
                19 | 45 => BlendMode::SoftLight,
                48 => BlendMode::VividLight,
                49 => BlendMode::PinLight,
                50 => BlendMode::LinearLight,
                51 => BlendMode::HardMix,
                52 => BlendMode::Exclusion,
                53 => BlendMode::LinearBurn,
                _ => BlendMode::Normal,
            }
        };
        // The earlier encoding-only policy cannot express a Lab composite.
        // Its authoritative, resolved space is retained in the typed policy below.
        l.import_compositing = if accounted_group_domain || d.composite_space == 3 || matches!(d.mode, 24..=27) {
            None
        } else {
            Some(ImportCompositing { blend_encoding: working(d, d.blend_space)?, composite_encoding: working(d, d.composite_space)?, pass_through: pass })
        };
        warnings.extend(d.diagnostics.iter().cloned());
        if let Some(m) = &d.mask {
            let p = m.plane.as_ref().ok_or_else(|| unsupported(m, "missing mask plane"))?.eager_original()?;
            let r = bounds(d)?;
            let s = surface(&mut budget, p, r, ColorMode::Grayscale, false, &p.bytes, ctl)?;
            budget.charge(std::mem::size_of::<Surface>() + 128)?;
            l.mask = Some(LayerMask { surface: std::sync::Arc::new(s), enabled: d.mask_enabled, linked: true, density: 1.0, feather: 0.0 });
        }
        let path = if d.path.is_empty() { vec![doc.layers.len() as u32] } else { d.path.clone() };
        if path.len() > photocraft_doc::MAX_GROUP_DEPTH + 1 {
            return Err(unsupported(d, "group nesting exceeds document cap"));
        }
        let parent = if path.len() > 1 {
            let prefix = path.get(..path.len() - 1).ok_or_else(|| unsupported(d, "invalid parent path"))?;
            Some(photocraft_doc::item_identity::ItemRef::Layer(
                identities.iter().find(|(path, _)| path == prefix).map(|(_, id)| *id).ok_or_else(|| unsupported(d, "missing parent identity"))?,
            ))
        } else {
            None
        };
        if let Some(source) = &l.indexed_source {
            budget.charge(source.palette().len() * 3 + 512)?;
            let raster =
                photocraft_doc::EditableIndexedRaster::try_new(source.bounds(), source.indices().clone(), source.alpha().clone(), source.palette().to_vec())
                    .map_err(|e| unsupported(d, e))?;
            state.editable_indexed.try_reserve_exact(1).map_err(|_| unsupported(d, "current index allocation failed"))?;
            state.editable_indexed.push(photocraft_doc::EditableIndexedLayer { owner: l.id, raster });
        }
        let owner = photocraft_doc::item_identity::ItemRef::Layer(l.id);
        record_memberships(&mut set_members, &d.item_sets, owner, &mut budget)?;
        if !d.effects.is_empty() {
            if !effects_candidate {
                return Err(unsupported(d, "unqualified point-filter source color, alpha or owner domain"));
            }
            let metadata = d
                .effects
                .iter()
                .try_fold(0_usize, |total, effect| {
                    total
                        .checked_add(1024)?
                        .checked_add(effect.name.len())?
                        .checked_add(effect.icon.len())?
                        .checked_add(effect.operation.len())?
                        .checked_add(effect.version.as_ref().map_or(0, String::len))?
                        .checked_add(effect.arguments.iter().try_fold(0_usize, |n, a| {
                            n.checked_add(128)?.checked_add(a.name.len())?.checked_add(if let photocraft_xcf::ArgumentValue::Config(text) = &a.value {
                                text.len().checked_mul(8)?
                            } else {
                                0
                            })
                        })?)
                })
                .ok_or_else(|| unsupported(d, "filter metadata overflow"))?;
            budget.charge(metadata)?;
            let masks = effect_masks(
                d,
                owner,
                next_filter_id,
                &mut next_mask_id,
                &mut state,
                &mut budget,
                ctl,
                gaussian_group_domain && d.group,
                finite_gaussian_domain
                    || finite_box_domain
                    || finite_median_domain
                    || gray_median_domain
                    || opaque_box_domain
                    || opaque_median_domain
                    || opaque_motion_domain
                    || opaque_pixelize_domain
                    || unpremultiply_domain
                    || hdr_clip_domain
                    || value_precision_domain
                    || brightness_precision_domain
                    || levels_precision_domain
                    || exposure_black_domain
                    || color_balance_domain
                    || temperature_domain
                    || desaturate_domain
                    || saturation_domain
                    || saturation_yuv_domain
                    || sepia_domain
                    || alien_map_domain
                    || saturation_gray_domain
                    || threshold_domain
                    || threshold_alpha_domain,
                value_precision_domain
                    || brightness_precision_domain
                    || levels_precision_domain
                    || exposure_black_domain
                    || color_balance_domain
                    || temperature_domain
                    || desaturate_domain
                    || saturation_domain
                    || saturation_yuv_domain
                    || sepia_domain
                    || alien_map_domain
                    || saturation_gray_domain
                    || threshold_domain
                    || threshold_alpha_domain,
                color_balance_depth_domain,
                exposure_geometry_domain,
                point_replace_domain.map(|_| doc.bounds()),
            )?;
            let neighborhood_domain = if let Some(domain) = point_replace_domain {
                domain
            } else if unpremultiply_domain {
                crate::xcf_filters::NeighborhoodDomain::UnpremultiplyRgbFloatLinear
            } else if alien_map_hsl_domain {
                crate::xcf_filters::NeighborhoodDomain::AlienMapHslPrecision
            } else if alien_map_domain {
                crate::xcf_filters::NeighborhoodDomain::AlienMapPrecision
            } else if sepia_domain {
                crate::xcf_filters::NeighborhoodDomain::SepiaPrecision
            } else if threshold_alpha_domain {
                crate::xcf_filters::NeighborhoodDomain::ThresholdAlphaPrecision
            } else if saturation_yuv_domain {
                crate::xcf_filters::NeighborhoodDomain::SaturationRgbYuvPrecision
            } else if saturation_gray_domain {
                crate::xcf_filters::NeighborhoodDomain::SaturationGrayPrecision
            } else if threshold_domain {
                crate::xcf_filters::NeighborhoodDomain::ThresholdPrecision
            } else if saturation_domain {
                crate::xcf_filters::NeighborhoodDomain::SaturationPrecision
            } else if desaturate_domain {
                crate::xcf_filters::NeighborhoodDomain::DesaturatePrecision
            } else if temperature_domain {
                crate::xcf_filters::NeighborhoodDomain::ColorTemperaturePrecision
            } else if color_balance_depth_domain {
                crate::xcf_filters::NeighborhoodDomain::ColorBalanceDepth
            } else if color_balance_domain {
                crate::xcf_filters::NeighborhoodDomain::ColorBalancePrecision
            } else if exposure_geometry_domain {
                crate::xcf_filters::NeighborhoodDomain::ExposureGeometry
            } else if exposure_black_domain {
                crate::xcf_filters::NeighborhoodDomain::ExposureBlackPrecision
            } else if levels_precision_domain {
                crate::xcf_filters::NeighborhoodDomain::GeglLevelsPrecision
            } else if brightness_precision_domain {
                crate::xcf_filters::NeighborhoodDomain::BrightnessContrastPrecision
            } else if hdr_clip_domain {
                crate::xcf_filters::NeighborhoodDomain::RgbClipHdr
            } else if value_precision_domain {
                crate::xcf_filters::NeighborhoodDomain::ValueInvertPrecision
            } else if posterize_domain {
                crate::xcf_filters::NeighborhoodDomain::PosterizeOriginal
            } else if value_invert_domain {
                crate::xcf_filters::NeighborhoodDomain::ValueInvertOriginal
            } else if mono_mixer_edits_domain {
                crate::xcf_filters::NeighborhoodDomain::MonoMixerEditsOriginal
            } else if mono_mixer_domain {
                crate::xcf_filters::NeighborhoodDomain::MonoMixerOriginal
            } else if point_stack_edits_domain {
                crate::xcf_filters::NeighborhoodDomain::PointStackEdits
            } else if opaque_pixelize_domain {
                crate::xcf_filters::NeighborhoodDomain::PixelizeOriginal
            } else if gray_median_domain {
                crate::xcf_filters::NeighborhoodDomain::MedianGray
            } else if finite_median_domain {
                crate::xcf_filters::NeighborhoodDomain::MedianFinite
            } else if opaque_motion_domain {
                crate::xcf_filters::NeighborhoodDomain::MotionOriginal
            } else if opaque_median_domain {
                crate::xcf_filters::NeighborhoodDomain::MedianOriginal
            } else if finite_box_domain {
                crate::xcf_filters::NeighborhoodDomain::BoxFinite
            } else if opaque_box_domain {
                crate::xcf_filters::NeighborhoodDomain::BoxOriginal
            } else if gaussian_group_domain && d.group {
                crate::xcf_filters::NeighborhoodDomain::IsolatedSmallAuto
            } else if finite_gaussian_domain {
                crate::xcf_filters::NeighborhoodDomain::RasterFinite
            } else if doc.size == Size::new(96, 80) && bounds(d)? == Rect::new(0, 0, 96, 80) {
                crate::xcf_filters::NeighborhoodDomain::RasterOriginal
            } else {
                crate::xcf_filters::NeighborhoodDomain::Unqualified
            };
            let stack = crate::xcf_filters::qualified_stack(&d.effects, &masks, owner, next_filter_id, file.version, neighborhood_domain)?;
            next_filter_id = next_filter_id
                .checked_add(u64::try_from(stack.nodes.len()).map_err(|_| unsupported(d, "filter count overflow"))?)
                .ok_or_else(|| unsupported(d, "filter identity overflow"))?;
            state.filter_stacks.try_reserve_exact(1).map_err(|_| unsupported(d, "filter stack allocation failed"))?;
            state.filter_stacks.push(stack);
        }
        state.items.push(photocraft_doc::xcf_state::ItemState {
            item: owner,
            name: l.name.clone(),
            custom_name: d.link.as_ref().is_some_and(|link| link.custom_name()),
            visible: l.visible,
            locks: photocraft_doc::xcf_state::ItemLocks {
                content: d.lock_content,
                position: d.lock_position,
                alpha: d.lock_alpha,
                visibility: d.lock_visibility,
            },
            parent,
            source_tattoo: d.tattoo,
            source_flags: d.group_flags,
            raster_bounds: if d.group && !gaussian_group_domain && !accounted_group_domain { None } else { Some(bounds(d)?) },
            mask: None,
            composition: Some(photocraft_doc::import_compositing::GimpComposition {
                compatibility_epoch: if accounted_group_domain {
                    accounted_group::EPOCH.into()
                } else if ordinary_geometry_domain {
                    photocraft_doc::import_compositing::XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH.into()
                } else if analytic_geometry_domain && matches!(file.sample, Sample::U16 | Sample::U32) {
                    photocraft_doc::import_compositing::XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH.into()
                } else if analytic_geometry_domain {
                    match file.encoding {
                        Encoding::NonlinearProfile => photocraft_doc::import_compositing::XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
                        Encoding::Linear => photocraft_doc::import_compositing::XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
                        Encoding::Perceptual => photocraft_doc::import_compositing::XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
                    }
                    .into()
                } else if analytic_normal_domain {
                    match file.encoding {
                        Encoding::NonlinearProfile => photocraft_doc::import_compositing::XCF12_ANALYTIC_NORMAL_MASK_EPOCH,
                        Encoding::Linear => photocraft_doc::import_compositing::XCF12_ANALYTIC_LINEAR_NORMAL_MASK_EPOCH,
                        Encoding::Perceptual => photocraft_doc::import_compositing::XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_EPOCH,
                    }
                    .into()
                } else if finite_normal_domain {
                    photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH.into()
                } else if masked_group_domain && d.group && d.mode == 28 {
                    photocraft_doc::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH.into()
                } else {
                    format!(
                        "xcf-{}-{}",
                        file.version,
                        if matches!(d.mode, 35 | 36) {
                            "authored-equivalent-rgb-space"
                        } else if matches!(d.mode, 3 | 4 | 6..=21 | 23..=27 | 30..=34 | 37..=60) {
                            "authored-blend-auto"
                        } else {
                            "normal"
                        }
                    )
                },
                source_mode: d.mode,
                legacy: matches!(d.mode, 0 | 3 | 4 | 6..=21),
                stored_blend_space: d.blend_space,
                stored_composite_space: d.composite_space,
                stored_operator: d.composite_mode,
                resolved: Some(photocraft_doc::import_compositing::ResolvedGimpComposition {
                    blend_space: if matches!(d.mode, 24..=27) && d.blend_space == -3 {
                        photocraft_doc::import_compositing::GimpSpace::Lab
                    } else {
                        gimp_space(working(d, d.blend_space)?)
                    },
                    composite_space: if d.composite_space == 3 {
                        photocraft_doc::import_compositing::GimpSpace::Lab
                    } else {
                        gimp_space(working(d, d.composite_space)?)
                    },
                    operator: if normal_mask_domain || matches!(d.mode, 3 | 4 | 6..=21 | 23..=27 | 30..=40 | 41..=60) {
                        match d.composite_mode {
                            -1 if d.mode == 28 => photocraft_doc::import_compositing::GimpOperator::Union,
                            -1 if d.mode == 59 => photocraft_doc::import_compositing::GimpOperator::Union,
                            1 => photocraft_doc::import_compositing::GimpOperator::Union,
                            3 => photocraft_doc::import_compositing::GimpOperator::ClipToLayer,
                            4 => photocraft_doc::import_compositing::GimpOperator::Intersection,
                            _ => photocraft_doc::import_compositing::GimpOperator::ClipToBackdrop,
                        }
                    } else {
                        photocraft_doc::import_compositing::GimpOperator::Union
                    },
                }),
                pass_through: pass,
            }),
        });
        let mask_owner = if let Some(mask) = &d.mask {
            let mask_id = u64::try_from(i).ok().and_then(|v| v.checked_add(1)).ok_or_else(|| unsupported(d, "mask identity overflow"))?;
            let item = photocraft_doc::item_identity::ItemRef::Mask(photocraft_doc::item_identity::MaskId(mask_id));
            budget.charge(512 + mask.name.len())?;
            state.items.try_reserve_exact(1).map_err(|_| unsupported(d, "mask state allocation failed"))?;
            state.items.push(photocraft_doc::xcf_state::ItemState {
                item,
                name: mask.name.clone(),
                custom_name: false,
                visible: mask.visible,
                locks: photocraft_doc::xcf_state::ItemLocks {
                    content: mask.lock_content,
                    position: mask.lock_position,
                    alpha: mask.lock_alpha,
                    visibility: mask.lock_visibility,
                },
                parent: None,
                source_tattoo: mask.tattoo,
                source_flags: 0,
                raster_bounds: Some(bounds(d)?),
                composition: None,
                mask: Some(photocraft_doc::xcf_state::MaskState {
                    owner,
                    bounds: bounds(d)?,
                    offset: [d.offset.0, d.offset.1],
                    outside_f64_bits: 0.0_f64.to_bits(),
                    enabled: d.mask_enabled,
                    shown: d.mask_shown,
                    editing: d.mask_editing,
                    linked: true,
                }),
            });
            Some(item)
        } else {
            None
        };
        if d.active {
            state.selection.selected.try_reserve_exact(1).map_err(|_| unsupported(d, "selection allocation failed"))?;
            state.selection.selected.push(owner);
            if d.mask_editing {
                state.selection.mask_target = mask_owner;
            }
        }
        identities.push((path.clone(), l.id));
        insert(&mut doc.layers, &path, l)?;
        let released =
            d.plane.as_ref().map_or(0, |p| p.retained_source_bytes()) + d.mask.as_ref().and_then(|m| m.plane.as_ref()).map_or(0, |p| p.retained_source_bytes());
        preserve_properties(&mut state, std::mem::take(&mut drawable.raw_properties), Some(owner), file.version, &mut budget)?;
        for (ordinal, effect) in drawable.effects.iter_mut().enumerate() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            preserve_properties(&mut state, std::mem::take(&mut effect.properties), Some(owner), file.version, &mut budget)?;
            for (argument_order, argument) in effect.arguments.iter().enumerate() {
                if let photocraft_xcf::ArgumentValue::Color { encoding, samples, icc } = &argument.value {
                    // Original format spelling and native color bytes remain inert
                    // provenance, separate from the checked current typed color.
                    let length = encoding
                        .len()
                        .checked_add(samples.len())
                        .and_then(|n| n.checked_add(icc.len()))
                        .and_then(|n| n.checked_add(12))
                        .ok_or_else(|| IoError::Unsupported("XCF color provenance size overflow".into()))?;
                    budget.charge(length.checked_add(512).ok_or_else(|| IoError::Unsupported("XCF color provenance size overflow".into()))?)?;
                    let mut bytes = Vec::new();
                    bytes.try_reserve_exact(length).map_err(|_| IoError::Unsupported("XCF color provenance allocation failed".into()))?;
                    for part in [encoding.as_bytes(), samples.as_slice(), icc.as_slice()] {
                        bytes.extend_from_slice(
                            &u32::try_from(part.len()).map_err(|_| IoError::Unsupported("XCF color provenance size overflow".into()))?.to_be_bytes(),
                        );
                        bytes.extend_from_slice(part);
                    }
                    state.opaque.try_reserve_exact(1).map_err(|_| IoError::Unsupported("XCF color provenance allocation failed".into()))?;
                    state.opaque.push(photocraft_doc::xcf_provenance::OpaqueRecord {
                        owner: Some(owner),
                        namespace: "gimp/xcf/source-filter-color".into(),
                        identity: format!("{ordinal}/{}", argument.name),
                        source_version: file.version,
                        order: u32::try_from(argument_order).map_err(|_| IoError::Unsupported("XCF argument order overflow".into()))?,
                        flags: 0,
                        blake3: blake3::hash(&bytes).to_hex().to_string(),
                        bytes: std::sync::Arc::new(bytes),
                        rendering_required: false,
                    });
                }
                if let photocraft_xcf::ArgumentValue::Config(text) = &argument.value {
                    budget.charge(text.len().checked_add(512).ok_or_else(|| IoError::Unsupported("XCF config allocation overflow".into()))?)?;
                    let mut bytes = Vec::new();
                    bytes.try_reserve_exact(text.len()).map_err(|_| IoError::Unsupported("XCF config allocation failed".into()))?;
                    bytes.extend_from_slice(text.as_bytes());
                    state.opaque.try_reserve_exact(1).map_err(|_| IoError::Unsupported("XCF config metadata allocation failed".into()))?;
                    state.opaque.push(photocraft_doc::xcf_provenance::OpaqueRecord {
                        owner: Some(owner),
                        namespace: "gimp/xcf/source-filter-config".into(),
                        identity: format!("{ordinal}/{}", argument.name),
                        source_version: file.version,
                        order: u32::try_from(argument_order).map_err(|_| IoError::Unsupported("XCF argument order overflow".into()))?,
                        flags: 0,
                        blake3: blake3::hash(&bytes).to_hex().to_string(),
                        bytes: std::sync::Arc::new(bytes),
                        rendering_required: false,
                    });
                }
            }
            if let Some(mask) = effect.mask.as_mut() {
                let mask_owner = state.filter_stacks.last().and_then(|stack| stack.nodes.get(ordinal)).and_then(|node| node.mask);
                preserve_properties(&mut state, std::mem::take(&mut mask.raw_properties), mask_owner, file.version, &mut budget)?;
            }
        }
        if let Some(mask) = drawable.mask.as_mut() {
            preserve_properties(&mut state, std::mem::take(&mut mask.raw_properties), mask_owner, file.version, &mut budget)?;
        }
        drop(drawable);
        budget.release(released)?;
        ctl.progress(0.7 + 0.25 * ((i + 1) as f32) / (layer_total.max(1) as f32));
    }
    reverse(&mut doc.layers);
    if floating_records.is_some() {
        let layer = floating_ids.0.ok_or_else(|| IoError::Unsupported("XCF floating source identity missing".into()))?;
        let target = floating_ids.1.ok_or_else(|| IoError::Unsupported("XCF floating target identity missing".into()))?;
        state.floating = Some(photocraft_doc::xcf_editing::FloatingAttachment { layer, target: photocraft_doc::item_identity::ItemRef::Layer(target) });
    }
    for mut d in file.channels {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if !d.unsupported.is_empty() {
            return Err(unsupported(&d, "unsupported saved channel semantics"));
        }
        let p = d.plane.as_ref().ok_or_else(|| unsupported(&d, "missing channel plane"))?.eager_original()?;
        let s = surface(&mut budget, p, bounds(&d)?, ColorMode::Grayscale, false, &p.bytes, ctl)?;
        let mut c = AlphaChannel::new(&d.name, s);
        c.opacity = d.opacity;
        if let Some(rgb) = d.channel_color {
            c.color = photocraft_color::Color::rgb(rgb[0], rgb[1], rgb[2]);
        }
        c.indicates = if d.show_masked { photocraft_doc::ColorIndicates::MaskedAreas } else { photocraft_doc::ColorIndicates::SelectedAreas };
        let owner = photocraft_doc::item_identity::ItemRef::Channel(c.id);
        record_memberships(&mut set_members, &d.item_sets, owner, &mut budget)?;
        state.items.try_reserve_exact(1).map_err(|_| unsupported(&d, "channel state allocation failed"))?;
        budget.charge(512 + d.name.len())?;
        state.items.push(photocraft_doc::xcf_state::ItemState {
            item: owner,
            name: d.name.clone(),
            custom_name: false,
            visible: d.visible,
            locks: photocraft_doc::xcf_state::ItemLocks {
                content: d.lock_content,
                position: d.lock_position,
                alpha: d.lock_alpha,
                visibility: d.lock_visibility,
            },
            parent: None,
            source_tattoo: d.tattoo,
            source_flags: 0,
            raster_bounds: Some(bounds(&d)?),
            mask: None,
            composition: None,
        });
        if d.active {
            state.selection.selected.try_reserve_exact(1).map_err(|_| unsupported(&d, "channel selection allocation failed"))?;
            state.selection.selected.push(owner);
        }
        preserve_properties(&mut state, std::mem::take(&mut d.raw_properties), Some(owner), file.version, &mut budget)?;
        if d.selection {
            if doc.selection.is_some() || doc.selection_channel_id.is_some() {
                return Err(unsupported(&d, "duplicate selection channels"));
            }
            doc.selection_channel_id = Some(c.id);
            doc.selection = Some(c.surface);
        } else {
            doc.channels.push(c);
        }
    }
    doc.paths = mapped_paths.paths;
    state.selection.selected_paths = mapped_paths.selected;
    state.items.try_reserve_exact(mapped_paths.items.len()).map_err(|_| IoError::Unsupported("XCF path registry allocation failed".into()))?;
    state.items.append(&mut mapped_paths.items);
    state.opaque.try_reserve_exact(mapped_paths.opaque.len()).map_err(|_| IoError::Unsupported("XCF path evidence allocation failed".into()))?;
    state.opaque.append(&mut mapped_paths.opaque);
    for (index, owner) in mapped_paths.memberships {
        record_memberships(&mut set_members, &[index], owner, &mut budget)?;
    }
    // Canonical item metadata retains tattoos for future vector bindings.
    let _source_path_ids = mapped_paths.tattoos;
    let mut family_ordinals = [0_u32; 3];
    for (index, set) in file.item_sets.into_iter().enumerate() {
        use photocraft_doc::{
            item_identity::ItemSetId,
            xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
        };
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let index = u32::try_from(index).map_err(|_| IoError::Unsupported("XCF set identity overflow".into()))?;
        let family = match set.item_type {
            0 => ItemFamily::Layer,
            1 => ItemFamily::Channel,
            2 => ItemFamily::Path,
            _ => return Err(IoError::Unsupported("XCF unknown item set family".into())),
        };
        let rule = match set.method {
            0 => ItemSetRule::Text(set.name.clone()),
            1 => ItemSetRule::Regex(set.name.clone()),
            2 => ItemSetRule::Glob(set.name.clone()),
            u32::MAX => ItemSetRule::Named,
            _ => return Err(IoError::Unsupported("XCF unknown item set rule".into())),
        };
        budget.charge(512 + set.name.len().saturating_mul(2))?;
        // Stock canonical files independently retain Layer and Channel declaration
        // lists whose respective first members both use index zero.
        let ordinal = family_ordinals.get_mut(set.item_type as usize).ok_or_else(|| IoError::Unsupported("XCF unknown item set family".into()))?;
        let members = set_members.remove(&(set.item_type, *ordinal)).unwrap_or_default();
        *ordinal = ordinal.checked_add(1).ok_or_else(|| IoError::Unsupported("XCF set family index overflow".into()))?;
        if members.iter().any(|member| {
            !matches!(
                (family, member),
                (ItemFamily::Layer, photocraft_doc::item_identity::ItemRef::Layer(_))
                    | (ItemFamily::Channel, photocraft_doc::item_identity::ItemRef::Channel(_))
                    | (ItemFamily::Path, photocraft_doc::item_identity::ItemRef::Path(_))
            )
        }) {
            return Err(IoError::Unsupported("XCF item set member family mismatch".into()));
        }
        state.item_sets.try_reserve_exact(1).map_err(|_| IoError::Unsupported("XCF item set allocation failed".into()))?;
        state.item_sets.push(ItemSet { id: ItemSetId(u64::from(index) + 1), name: set.name, family, rule, members });
    }
    if !set_members.is_empty() {
        return Err(IoError::Unsupported("XCF item set membership references a missing declaration".into()));
    }
    for g in file.guides {
        if (f64::from(g.position)) as i32 != g.position {
            warnings.push(format!("Guide at {} omitted; position is not exactly representable", g.position));
            continue;
        }
        if g.horizontal {
            doc.guides.horizontal.push(f64::from(g.position));
        } else {
            doc.guides.vertical.push(f64::from(g.position));
        }
    }
    if state.selection.selected.len() == 1 {
        state.selection.active = state.selection.selected.first().copied();
    }
    doc.xcf_state = Some(state);
    doc.validate_xcf_state_with(ctl).map_err(
        |e| {
            if ctl.cancelled() { IoError::Cancelled } else { IoError::Unsupported(format!("XCF imported state: {e}")) }
        },
    )?;
    if ordinary_geometry_domain {
        let (shared, source_lease) = preparation.ok_or_else(|| IoError::Unsupported("ordinary scene requires transaction accounting".into()))?;
        let mut adapter = SharedPreparation { shared, failure: None };
        let checked = photocraft_compose::gimp_render::PreparedRenderer::new_budgeted(&doc, ctl, &mut adapter, source_lease);
        if let Err(error) = checked {
            return Err(adapter
                .failure
                .map(accounting_error)
                .unwrap_or_else(|| if ctl.cancelled() { IoError::Cancelled } else { IoError::Unsupported(format!("XCF rendering consumer: {error}")) }));
        }
    } else if !doc.channels.is_empty()
        || doc.xcf_state.as_ref().is_some_and(|state| !state.filter_stacks.is_empty() || !state.source_items.is_empty() || state.floating.is_some())
    {
        photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&doc, ctl)
            .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { IoError::Unsupported(format!("XCF rendering consumer: {e}")) })?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    budget.charge(crate::xcf_assessment::allocation_bound(&doc)?)?;
    let assessment = Some(crate::xcf_assessment::after_qualified_import(&doc, ctl)?);
    Ok((ImportResult { document: doc, warnings, origin: ImportOrigin::xcf(), assessment, source_read_only: true, preview_only: false}, budget.peak))
}

#[allow(clippy::too_many_arguments)]
fn effect_masks<P: MappingPlane>(
    drawable: &Drawable<P>,
    owner: photocraft_doc::item_identity::ItemRef,
    first_filter: u64,
    next_mask: &mut u64,
    state: &mut photocraft_doc::XcfState,
    budget: &mut Budget,
    ctl: &Interrupt<'_>,
    canvas_group_mask: bool,
    finite_raster_mask: bool,
    value_precision_mask: bool,
    color_balance_depth_mask: bool,
    exposure_geometry_mask: bool,
    point_replace_canvas: Option<Rect>,
) -> Result<Vec<Option<photocraft_doc::item_identity::ItemRef>>, IoError> {
    use photocraft_doc::{
        item_identity::{FilterId, ItemRef, MaskId},
        xcf_state::{ItemLocks, ItemState, MaskState},
    };
    let mut mapped = Vec::new();
    mapped.try_reserve_exact(drawable.effects.len()).map_err(|_| unsupported(drawable, "filter mask allocation failed"))?;
    for (ordinal, effect) in drawable.effects.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let Some(mask) = &effect.mask else {
            mapped.push(None);
            continue;
        };
        let rect = bounds(mask)?;
        let plane = mask.plane.as_ref().ok_or_else(|| unsupported(mask, "missing filter mask plane"))?.eager_original()?;
        if !mask.unsupported.is_empty()
            || !(plane.sample == Sample::U8
                || point_replace_canvas.is_some() && matches!(plane.sample, Sample::U16 | Sample::U32 | Sample::F32 | Sample::F64)
                || color_balance_depth_mask && plane.sample == Sample::U32
                || (value_precision_mask || exposure_geometry_mask) && plane.sample == Sample::U16
                || (effect.operation == "gimp:levels" || canvas_group_mask) && plane.sample == Sample::F32
                || (finite_raster_mask || exposure_geometry_mask)
                    && matches!(plane.sample, Sample::F32 | Sample::F64)
                    && drawable.plane.as_ref().is_some_and(|p| p.descriptor().sample == plane.sample))
            || (finite_raster_mask || exposure_geometry_mask || point_replace_canvas.is_some())
                && !drawable.plane.as_ref().is_some_and(|p| p.descriptor().sample == plane.sample)
            || plane.channels != 1
            || rect
                != if let Some(canvas) = point_replace_canvas {
                    canvas
                } else if exposure_geometry_mask {
                    Rect::new(0, 0, 295, 160)
                } else if canvas_group_mask {
                    Rect::new(0, 0, 16, 12)
                } else {
                    bounds(drawable)?
                }
            || (finite_raster_mask || exposure_geometry_mask || point_replace_canvas.is_some())
                && (mask.model != Model::Gray
                    || mask.alpha
                    || !mask.visible
                    || !mask.mask_enabled
                    || mask.mask_shown
                    || mask.mask_editing
                    || mask.legacy_linked
                    || mask.mode != 0
                    || (mask.blend_space, mask.composite_space, mask.composite_mode) != (2, 2, 1)
                    || !mask.effects.is_empty())
        {
            return Err(unsupported(mask, "unqualified filter selection mask domain"));
        }
        let surface = surface(budget, plane, rect, ColorMode::Grayscale, false, &plane.bytes, ctl)?;
        let id = MaskId(*next_mask);
        *next_mask = next_mask.checked_add(1).ok_or_else(|| unsupported(mask, "filter mask identity overflow"))?;
        let filter =
            FilterId(u64::try_from(ordinal).ok().and_then(|n| first_filter.checked_add(n)).ok_or_else(|| unsupported(mask, "filter identity overflow"))?);
        let current = photocraft_doc::xcf_filter_masks::FilterMask { id, owner: filter, surface, bounds: rect };
        if !current.is_empty_selection_with(ctl).map_err(|e| if e.code == "cancelled" { IoError::Cancelled } else { unsupported(mask, e) })? {
            return Err(unsupported(mask, "nonempty filter selection mask evaluation is unqualified"));
        }
        budget.charge(1024 + mask.name.len())?;
        state.filter_masks.try_reserve_exact(1).map_err(|_| unsupported(mask, "filter mask state allocation failed"))?;
        state.items.try_reserve_exact(1).map_err(|_| unsupported(mask, "filter mask item allocation failed"))?;
        let item = ItemRef::Mask(id);
        state.items.push(ItemState {
            item,
            name: mask.name.clone(),
            custom_name: false,
            visible: mask.visible,
            locks: ItemLocks { content: mask.lock_content, position: mask.lock_position, alpha: mask.lock_alpha, visibility: mask.lock_visibility },
            parent: None,
            source_tattoo: mask.tattoo,
            source_flags: mask.group_flags,
            raster_bounds: Some(rect),
            composition: None,
            mask: Some(MaskState {
                owner,
                bounds: rect,
                offset: [mask.offset.0, mask.offset.1],
                outside_f64_bits: 0.0_f64.to_bits(),
                enabled: mask.mask_enabled,
                shown: mask.mask_shown,
                editing: mask.mask_editing,
                linked: mask.legacy_linked,
            }),
        });
        state.filter_masks.push(current);
        mapped.push(Some(item));
    }
    Ok(mapped)
}

fn record_memberships(
    sets: &mut std::collections::BTreeMap<(u32, u32), Vec<photocraft_doc::item_identity::ItemRef>>,
    source: &[u32],
    owner: photocraft_doc::item_identity::ItemRef,
    budget: &mut Budget,
) -> Result<(), IoError> {
    let family = match owner {
        photocraft_doc::item_identity::ItemRef::Layer(_) => 0,
        photocraft_doc::item_identity::ItemRef::Channel(_) => 1,
        photocraft_doc::item_identity::ItemRef::Path(_) => 2,
        photocraft_doc::item_identity::ItemRef::Mask(_) => return Err(IoError::Unsupported("XCF mask item-set membership".into())),
    };
    for index in source {
        budget.charge(64)?;
        let members = sets.entry((family, *index)).or_default();
        if members.contains(&owner) {
            return Err(IoError::Unsupported("XCF duplicate item set membership".into()));
        }
        members.try_reserve_exact(1).map_err(|_| IoError::Unsupported("XCF item set member allocation failed".into()))?;
        members.push(owner);
    }
    Ok(())
}

#[cfg(test)]
mod unit_tests {
    // Synthetic decoded state in the existing Normal-mask admission envelope.
    // It tests mapping/serialization only, not a new source rendering domain.
    fn mask_lock_fixture(bits: u8) -> photocraft_xcf::XcfFile {
        use photocraft_xcf::{Compression, Drawable, Encoding, Model, Plane, RawProperty, Sample, Unit, XcfFile};
        let flags = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0];
        let mask = Drawable {
            name: "retained mask".into(),
            width: 10,
            height: 9,
            model: Model::Gray,
            lock_content: flags[0],
            lock_position: flags[1],
            lock_alpha: flags[2],
            lock_visibility: flags[3],
            tattoo: Some(91),
            raw_properties: [28, 32, 10, 42].into_iter().zip(flags).map(|(tag, flag)| RawProperty {
                tag,
                offset: u64::from(tag),
                declared_bytes: 4,
                payload: u32::from(flag).to_be_bytes().to_vec(),
            }).collect(),
            plane: Some(Plane { width: 10, height: 9, channels: 1, sample: Sample::U8, bytes: vec![127; 90] }),
            ..Default::default()
        };
        let owner = Drawable {
            name: "owner".into(),
            width: 10,
            height: 9,
            alpha: true,
            offset: (-2, -1),
            opacity: 0.7125,
            mode: 28,
            blend_space: 0,
            composite_space: 1,
            composite_mode: 1,
            mask: Some(Box::new(mask)),
            plane: Some(Plane { width: 10, height: 9, channels: 4, sample: Sample::U8, bytes: [12, 34, 56, 255].repeat(90) }),
            ..Default::default()
        };
        let backdrop = Drawable {
            name: "backdrop".into(),
            width: 16,
            height: 12,
            alpha: true,
            mode: 28,
            blend_space: 0,
            composite_space: -1,
            composite_mode: -1,
            plane: Some(Plane { width: 16, height: 12, channels: 4, sample: Sample::U8, bytes: [78, 90, 123, 255].repeat(192) }),
            ..Default::default()
        };
        XcfFile {
            paths: Vec::new(), tattoo: None, item_sets: Vec::new(),
            version: 11, width: 16, height: 12, model: Model::Rgb,
            precision: 150, sample: Sample::U8, encoding: Encoding::NonlinearProfile,
            compression: Compression::None, icc: Vec::new(), dpi: (72.0, 72.0),
            unit: Unit::Inch, palette: Vec::new(), layers: vec![owner, backdrop],
            channels: Vec::new(), guides: Vec::new(), diagnostics: Vec::new(),
            charged_bytes: 0, raw_properties: Vec::new(),
        }
    }

    #[test]
    fn layer_color_tags_map_exact_shared_labels_and_retain_unmapped_source_tags_in_native() {
        use photocraft_doc::{LabelColor, item_identity::ItemRef};
        use photocraft_raster::Interrupt;
        use photocraft_xcf::RawProperty;
        assert_eq!(super::layer_color_tag(None), Ok(LabelColor::None));
        for (tag, expected, limitation) in [
            (0, LabelColor::None, None),
            (1, LabelColor::Blue, None),
            (2, LabelColor::Green, None),
            (3, LabelColor::Yellow, None),
            (4, LabelColor::Orange, None),
            (5, LabelColor::Brown, None),
            (6, LabelColor::Red, None),
            (7, LabelColor::Violet, None),
            (8, LabelColor::Gray, None),
            (u32::MAX, LabelColor::None, Some("unknown stored color tag")),
        ] {
            let mut file = mask_lock_fixture(0);
            file.layers[0].color_tag = Some(tag);
            let payload = tag.to_be_bytes().to_vec();
            file.layers[0].raw_properties.push(RawProperty {
                tag: 34, offset: 123, declared_bytes: 4, payload: payload.clone(),
            });
            let original_pixels = file.layers[0].plane.as_ref().unwrap().bytes.clone();
            assert!(super::qualified_normal_mask(&file));
            let mapped = super::map("color tags", file.clone(), &Interrupt::NONE, 128 << 20).unwrap().0;
            assert_eq!(mapped.document.layers[0].label, expected);
            assert_eq!(mapped.document.layers[1].label, LabelColor::None);
            if let Some(reason) = limitation {
                assert!(mapped.warnings.iter().any(|warning| warning.contains(reason) && warning.contains("original source tag retained")));
            } else {
                assert!(!mapped.warnings.iter().any(|warning| warning.contains("current label unavailable")));
            }
            let state = mapped.document.xcf_state.as_ref().unwrap();
            let record = state.opaque.iter().find(|record| {
                record.owner == Some(ItemRef::Layer(mapped.document.layers[0].id)) && record.identity == "34@123:4"
            }).unwrap();
            assert_eq!(record.bytes.as_slice(), payload.as_slice());
            assert_eq!(record.blake3, blake3::hash(&payload).to_hex().as_str());
            assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, original_pixels);
            let before = mapped.document.clone();
            let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.layers, before.layers);
            assert_eq!(reopened.xcf_state, before.xcf_state);
            assert_eq!(mapped.document, before);
        }
    }

    #[test]
    fn layer_mask_mapping_preserves_each_decoded_lock_without_changing_owner_or_samples() {
        use photocraft_doc::{item_identity::{ItemRef, MaskId}, xcf_state::ItemLocks};
        use photocraft_raster::Interrupt;
        let baseline = super::map("locks", mask_lock_fixture(0), &Interrupt::NONE, 128 << 20).unwrap().0.document;
        for bits in 0..16 {
            let file = mask_lock_fixture(bits);
            let original_payloads = file.layers[0].mask.as_ref().unwrap().raw_properties.clone();
            assert!(super::qualified_normal_mask(&file));
            let mapped = super::map("locks", file.clone(), &Interrupt::NONE, 128 << 20).unwrap().0.document;
            let state = mapped.xcf_state.as_ref().unwrap();
            let mask = state.items.iter().find(|item| item.item == ItemRef::Mask(MaskId(1))).unwrap();
            assert_eq!(mask.locks, ItemLocks {
                content: bits & 1 != 0, position: bits & 2 != 0,
                alpha: bits & 4 != 0, visibility: bits & 8 != 0,
            });
            assert_eq!(mask.mask.as_ref().unwrap().owner, ItemRef::Layer(mapped.layers[0].id));
            assert_eq!(mask.source_tattoo, Some(91));
            for (layer, original) in mapped.layers.iter().zip(&baseline.layers) {
                assert_eq!(layer.surface(), original.surface());
                assert_eq!(layer.mask, original.mask);
            }
            for (order, property) in original_payloads.iter().enumerate() {
                let record = state.opaque.iter().find(|record| {
                    record.owner == Some(mask.item) && record.order == order as u32
                }).unwrap();
                assert_eq!(record.identity, format!("{}@{}:{}", property.tag, property.offset, property.declared_bytes));
                assert_eq!(record.bytes.as_slice(), property.payload.as_slice());
                assert_eq!(record.blake3, blake3::hash(&property.payload).to_hex().as_str());
            }
            let before = mapped.clone();
            let bytes = photocraft_format::save_to_bytes(&mapped, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.xcf_state, before.xcf_state);
            assert_eq!(reopened.layers, before.layers);
            assert_eq!(mapped, before);
        }
    }

    #[test]
    fn half_unit_domain_accepts_exact_widening_and_rejects_unrepresentable_floats() {
        for code in 0..=u16::MAX {
            let value = f32::from_bits(photocraft_color::widen_f16_to_f32_bits(code));
            assert_eq!(super::exact_half_unit_f32(value), value.is_finite() && (0.0..=1.0).contains(&value), "half {code:04x}");
        }
        for value in [f32::from_bits(1), f32::from_bits(0.5f32.to_bits() + 1), 0.123456, f32::NAN, f32::INFINITY, -0.25, 1.25] {
            assert!(!super::exact_half_unit_f32(value), "{value}");
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished Mono Mixer originals"]
    fn mono_mixer_rejects_unqualified_types_envelopes_and_owners() {
        use photocraft_xcf::ArgumentValue;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-edits-evidence");
        let bytes = std::fs::read(root.join("mono.source.xcf")).unwrap();
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        super::map("qualified", file.clone(), &photocraft_raster::Interrupt::NONE, photocraft_xcf::Limits::default().resident_bytes).unwrap();
        for change in 0..18 {
            let mut invalid = file.clone();
            let layer = &mut invalid.layers[0];
            match change {
                0 => layer.effects[0].arguments.iter_mut().find(|a| a.name == "preserve-luminosity").unwrap().value = ArgumentValue::Boolean(true),
                1 => layer.effects[0].arguments.iter_mut().find(|a| a.name == "preserve-luminosity").unwrap().value = ArgumentValue::Float32Bits(0),
                2 => layer.effects[0].arguments.iter_mut().find(|a| a.name == "preserve-luminosity").unwrap().value = ArgumentValue::Integer(0),
                3 => layer.effects[0].arguments.iter_mut().find(|a| a.name == "red").unwrap().value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
                4 => layer.effects[0].arguments.iter_mut().find(|a| a.name == "red").unwrap().value = ArgumentValue::Float32Bits(1.0_f32.to_bits()),
                5 => layer.effects[0].arguments.iter_mut().find(|a| a.name == "red").unwrap().value = ArgumentValue::Integer(0),
                6 => {
                    let duplicate = layer.effects[0].arguments[0].clone();
                    layer.effects[0].arguments.push(duplicate);
                }
                7 => {
                    layer.effects[0].arguments.pop();
                }
                8 => layer.effects[0].state.blend_space = Some(1),
                9 => layer.effects[0].version = Some("9:0".into()),
                10 => layer.offset = (1, 0),
                11 => layer.opacity = 0.5,
                12 => layer.plane.as_mut().unwrap().bytes[3] = 254,
                13 => invalid.width = 33,
                14 => invalid.encoding = photocraft_xcf::Encoding::Linear,
                15 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().to_vec(),
                16 => layer.effects[0].state.region = Some(1),
                _ => layer.effects[0].arguments.iter_mut().find(|a| a.name == "blue").unwrap().value = ArgumentValue::Float32Bits((-0.01_f32).to_bits()),
            }
            assert!(
                super::map("unqualified", invalid, &photocraft_raster::Interrupt::NONE, photocraft_xcf::Limits::default().resident_bytes).is_err(),
                "variant {change}"
            );
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished Brightness/Contrast originals"]
    fn brightness_precision_rejects_changed_owner_and_selection_contexts() {
        precision_point_context_rejections("brightness-contrast-precision-evidence", "gegl:brightness-contrast");
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished GEGL Levels originals"]
    fn gegl_levels_precision_rejects_changed_owner_and_selection_contexts() {
        precision_point_context_rejections("gegl-levels-precision-evidence", "gegl:levels");
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished GEGL Exposure black-level originals"]
    fn exposure_black_precision_rejects_changed_owner_and_selection_contexts() {
        precision_point_context_rejections("exposure-black-level-precision-evidence", "gegl:exposure");
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished Color Balance originals"]
    fn color_balance_precision_rejects_changed_owner_and_selection_contexts() {
        precision_point_context_rejections("color-balance-evidence", "gimp:color-balance");
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished integer half perceptual Color Balance originals"]
    fn color_balance_depth_rejects_changed_owner_and_selection_contexts() {
        precision_point_context_rejections("color-balance-depth-evidence", "gimp:color-balance");
    }
    #[cfg(feature = "corpus")]
    fn precision_point_context_rejections(packet: &str, operation: &str) {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(packet);
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        let mut count = 0;
        for row in proof["checks"].as_array().unwrap().iter().filter(|row| row["id"].as_str().unwrap().ends_with(".source")) {
            let id = row["id"].as_str().unwrap();
            let bytes = std::fs::read(root.join(format!("{id}.xcf"))).unwrap();
            let digest = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            if operation == "gimp:color-balance" {
                let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("SHA256SUMS.json")).unwrap()).unwrap();
                assert_eq!(digest, manifest[format!("{id}.xcf")]);
            } else {
                let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
                let asset = manifest["artifacts"].as_array().unwrap().iter().find(|asset| asset["path"] == format!("{id}.xcf")).unwrap();
                assert_eq!(digest, asset["sha256"]);
            }
            let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert_eq!(original.layers[0].effects[0].operation, operation);
            assert!(super::map("precision point", original.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
            for change in 0..12 {
                let mut invalid = original.clone();
                match change {
                    0 => invalid.version = 23,
                    1 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone(),
                    2 => invalid.layers[0].offset = (1, 0),
                    3 => invalid.layers[0].opacity = 0.5,
                    4 => invalid.layers[0].effects[0].mask = None,
                    5 => {
                        let sample = &mut invalid.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample;
                        *sample = if *sample == photocraft_xcf::Sample::U8 { photocraft_xcf::Sample::U16 } else { photocraft_xcf::Sample::U8 };
                    }
                    6 => invalid.layers[0].effects[0].mask.as_mut().unwrap().offset = (-1, 0),
                    7 => invalid.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                    8 => invalid.layers[0].effects[0].version = Some(if operation == "gegl:exposure" { "0:0" } else { "1:0" }.into()),
                    9 => invalid.layers[0].effects[0].arguments[0].value = photocraft_xcf::ArgumentValue::Float64Bits(0.25_f64.to_bits()),
                    10 => invalid.layers[0].effects[0].state.composite_mode = Some(-2103934383),
                    _ => invalid.layers[0].effects[0].arguments[0].name = "unknown-parameter".into(),
                }
                assert!(super::map("unqualified brightness", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20).is_err(), "{id} variant {change}");
            }
            count += 1;
        }
        assert_eq!(count, if operation == "gimp:color-balance" && packet == "color-balance-evidence" { 6 } else { 12 });
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished finite-alpha originals"]
    fn finite_alpha_import_rejects_unobserved_context_without_clamping() {
        use sha2::{Digest, Sha256};
        let base = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let packet: serde_json::Value =
            serde_json::from_slice(&std::fs::read(base.join("float-alpha-composition-evidence/observations.json")).unwrap()).unwrap();
        let mut count = 0;
        for row in packet["records"].as_array().unwrap().iter().filter(|row| row["backdrop"] == "transparent" && row["original"]["folder"].is_string()) {
            let original = &row["original"];
            let bytes = std::fs::read(base.join(original["folder"].as_str().unwrap()).join(original["file"].as_str().unwrap())).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), original["sha256"]);
            let file = photocraft_xcf::decode(&bytes, &photocraft_xcf::DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() }).unwrap();
            assert!(super::qualified_finite_normal_source(&file));
            for change in 0..8 {
                let mut invalid = file.clone();
                match change {
                    0 => invalid.version = 13,
                    1 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone(),
                    2 => invalid.encoding = photocraft_xcf::Encoding::Linear,
                    3 => invalid.layers[0].opacity = 0.5,
                    4 => invalid.layers[0].offset = (0, 0),
                    5 => invalid.layers[0].composite_space = 2,
                    6 => {
                        let extra = invalid.layers[0].clone();
                        invalid.layers.push(extra);
                    }
                    _ => invalid.layers[0].visible = false,
                }
                assert!(super::map("unqualified alpha", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20).is_err(), "{} variant {change}", row["id"]);
            }
            count += 1;
        }
        assert_eq!(count, 6);
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished integer Normal mask originals"]
    fn integer_normal_masks_reject_mismatched_versions_depths_and_assigned_profiles() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("normal-mask-integer-evidence");
        let hashes: serde_json::Value = serde_json::from_str(include_str!("../../compose/tests/gimp_normal_mask_integer_hashes.json")).unwrap();
        let raw = std::fs::read(root.join("observations.json")).unwrap();
        assert_eq!(Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect::<String>(), hashes["observations.json"]);
        let observations: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let mut count = 0;
        for row in observations["records"].as_array().unwrap() {
            if !row["id"].as_str().unwrap().ends_with("op4-source") {
                continue;
            }
            let bytes = std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), row["source_sha256"]);
            let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(super::map("integer", original.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
            for change in 0..7 {
                let mut invalid = original.clone();
                match change {
                    0 => invalid.version = if invalid.version == 11 { 12 } else { 11 },
                    1 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone(),
                    2 => invalid.precision += 1,
                    3 => invalid.layers[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample = photocraft_xcf::Sample::F32,
                    4 => invalid.layers[0].mask.as_mut().unwrap().plane.as_mut().unwrap().channels = 2,
                    5 => {
                        invalid.layers[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes.pop().unwrap();
                    }
                    _ => invalid.layers[0].mask.as_mut().unwrap().offset = (1, 0),
                }
                assert!(super::map("unqualified integer", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20).is_err(), "{} variant {change}", row["id"]);
            }
            count += 1;
        }
        assert_eq!(count, 12);
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished Normal mask matrix"]
    fn normal_masks_reject_unobserved_geometry_encoding_and_sample_domains() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("normal-mask-evidence");
        let raw = std::fs::read(root.join("observations.json")).unwrap();
        assert_eq!(
            Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(),
            "e544e1d5b22191bfee241b21c3e569e709862ac7d4ffd7b76d8c5939c5170b79"
        );
        let observations: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        for id in ["rgb-f32-op4-source", "rgb-f64-op4-source", "gray-f32-op4-source", "gray-f64-op4-source"] {
            let row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == id).unwrap();
            let bytes = std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(), row["source_sha256"]);
            let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(super::map("qualified", original.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
            for change in 0..15 {
                let mut invalid = original.clone();
                match change {
                    0 => invalid.version = 13,
                    1 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone(),
                    2 => invalid.encoding = photocraft_xcf::Encoding::Linear,
                    3 => invalid.layers[0].offset = (0, 0),
                    4 => invalid.layers[0].opacity = 0.5,
                    5 => invalid.layers[0].composite_space = 2,
                    6 => invalid.layers[0].blend_space = 2,
                    7 => invalid.layers[0].mask_enabled = false,
                    8 => invalid.layers[0].mask_shown = true,
                    9 => invalid.layers[0].mask_editing = true,
                    10 => invalid.layers[0].mask.as_mut().unwrap().offset = (1, 0),
                    11 => invalid.layers[0].mask.as_mut().unwrap().width = 9,
                    12 => {
                        assert!(invalid.layers[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes.pop().is_some());
                    }
                    13 => invalid.layers[0].composite_mode = 5,
                    _ => {
                        let mask = invalid.layers[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
                        if mask.sample == photocraft_xcf::Sample::F64 {
                            mask.bytes[..8].copy_from_slice(&f64::NAN.to_ne_bytes());
                        } else {
                            mask.bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes());
                        }
                    }
                }
                assert!(
                    matches!(super::map("unobserved", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20), Err(crate::IoError::Unsupported(_))),
                    "{id} variant {change}"
                );
            }
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished Posterize originals"]
    fn posterize_import_rejects_unobserved_context_envelopes_and_types() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations/gimp-posterize");
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("probe.json")).unwrap()).unwrap();
        let bytes = std::fs::read(root.join(proof["cases"][0]["input"].as_str().unwrap())).unwrap();
        assert_eq!(
            Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(),
            "22dd717b5ca0863521f20791705746d104ee6a35fa90806759bb47d96c16b9c3"
        );
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(super::map("qualified", original.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
        for change in 0..18 {
            let mut invalid = original.clone();
            match change {
                0 => invalid.version = 23,
                1 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone(),
                2 => invalid.encoding = photocraft_xcf::Encoding::Linear,
                3 => invalid.layers[0].offset = (1, 0),
                4 => invalid.layers[0].visible = false,
                5 => invalid.layers[0].opacity = 0.5,
                6 => invalid.layers[0].blend_space = 2,
                7 => invalid.layers[0].effects[0].state.blend_space = Some(-1178762671),
                8 => invalid.layers[0].effects[0].state.opacity_bits = Some(0.5_f32.to_bits()),
                9 => invalid.layers[0].effects[0].state.region = Some(1),
                10 => invalid.layers[0].effects[0].version = Some("9:0".into()),
                11 => {
                    let extra = invalid.layers[0].effects[0].clone();
                    invalid.layers[0].effects.push(extra);
                }
                12 => invalid.layers[0].plane.as_mut().unwrap().bytes[3] = 254,
                13 => invalid.layers[0].effects[0].arguments[0].value = photocraft_xcf::ArgumentValue::Float32Bits(3.0_f32.to_bits()),
                14 => invalid.layers[0].effects[0].arguments[0].value = photocraft_xcf::ArgumentValue::Integer(1),
                15 => invalid.layers[0].effects[0].arguments[0].value = photocraft_xcf::ArgumentValue::Integer(257),
                16 => {
                    let extra = invalid.layers[0].effects[0].arguments[0].clone();
                    invalid.layers[0].effects[0].arguments.push(extra);
                }
                _ => invalid.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 255,
            }
            assert!(
                matches!(super::map("unobserved", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20), Err(crate::IoError::Unsupported(_))),
                "variant {change}"
            );
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished point stack source"]
    fn point_stack_import_rejects_unobserved_context_and_envelopes() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("filter-stack-edits-evidence");
        let bytes = std::fs::read(root.join("stack.source.xcf")).unwrap();
        assert_eq!(
            Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(),
            "414a7c66f0cbb78d772b2766592472353acb2e520bec71c9f46daee3070c12f8"
        );
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(super::map("stack", original.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
        for change in 0..14 {
            let mut invalid = original.clone();
            match change {
                0 => invalid.version = 23,
                1 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone(),
                2 => invalid.encoding = photocraft_xcf::Encoding::Linear,
                3 => invalid.layers[0].offset = (1, 0),
                4 => invalid.layers[0].visible = false,
                5 => invalid.layers[0].opacity = 0.5,
                6 => invalid.layers[0].blend_space = 2,
                7 => invalid.layers[0].effects[0].state.blend_space = Some(-261651887),
                8 => invalid.layers[0].effects[0].state.opacity_bits = Some(0.25_f32.to_bits()),
                9 => invalid.layers[0].effects[0].state.region = Some(1),
                10 => invalid.layers[0].effects[0].version = Some("9:0".into()),
                11 => {
                    let extra = invalid.layers[0].effects[0].clone();
                    invalid.layers[0].effects.push(extra);
                }
                12 => invalid.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 255,
                _ => invalid.layers[0].effects[0].operation = "gegl:brightness-contrast".into(),
            }
            assert!(
                matches!(super::map("unobserved stack", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20), Err(crate::IoError::Unsupported(_))),
                "{change}"
            );
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished component blend originals"]
    fn component_blends_reject_unobserved_source_domains() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("composition-matrix");
        for (case, manifest) in [
            ("hsv-hue", include_str!("../../compose/tests/gimp_hsv_hashes.json")),
            ("lch-chroma", include_str!("../../compose/tests/gimp_lch_hashes.json")),
            ("luminance", include_str!("../../compose/tests/gimp_luma_hashes.json")),
            ("color-erase", include_str!("../../compose/tests/gimp_color_erase_hashes.json")),
            ("erase", include_str!("../../compose/tests/gimp_alpha_hashes.json")),
            ("merge", include_str!("../../compose/tests/gimp_alpha_hashes.json")),
            ("split", include_str!("../../compose/tests/gimp_alpha_hashes.json")),
            ("grain-extract-legacy", include_str!("../../compose/tests/gimp_legacy_grain_hashes.json")),
            ("grain-merge-legacy", include_str!("../../compose/tests/gimp_legacy_grain_hashes.json")),
            ("divide-legacy", include_str!("../../compose/tests/gimp_legacy_contrast_hashes.json")),
            ("dodge-legacy", include_str!("../../compose/tests/gimp_legacy_contrast_hashes.json")),
            ("burn-legacy", include_str!("../../compose/tests/gimp_legacy_contrast_hashes.json")),
            ("hardlight-legacy", include_str!("../../compose/tests/gimp_legacy_contrast_hashes.json")),
            ("grain-extract", include_str!("../../compose/tests/gimp_basic_blends_hashes.json")),
            ("grain-merge", include_str!("../../compose/tests/gimp_basic_blends_hashes.json")),
        ] {
            let path = format!("mode-{case}.xcf");
            let bytes = std::fs::read(root.join(&path)).unwrap();
            let hashes: serde_json::Value = serde_json::from_str(manifest).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|value| format!("{value:02x}")).collect::<String>(), hashes[&path]);
            let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(super::map(&path, original.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
            for change in 0..11 {
                let mut invalid = original.clone();
                match change {
                    0 => invalid.version = if original.version == 23 { 12 } else { 23 },
                    1 => invalid.icc = photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone(),
                    2 => invalid.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                    3 => invalid.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&2.0_f32.to_ne_bytes()),
                    4 => invalid.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.1_f32.to_ne_bytes()),
                    5 => invalid.layers[0].blend_space = if original.layers[0].blend_space == 0 { 1 } else { 0 },
                    6 => invalid.layers[0].composite_space = if original.layers[0].composite_space == -2 { -1 } else { -2 },
                    7 => invalid.layers[0].composite_mode = if original.layers[0].mode == 59 { -2 } else { 1 },
                    8 => invalid.layers[0].offset = (1, 1),
                    9 => invalid.layers[0].visible = false,
                    _ => invalid.encoding = photocraft_xcf::Encoding::Linear,
                }
                assert!(
                    matches!(super::map("unobserved component", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20), Err(crate::IoError::Unsupported(_))),
                    "{case}/{change}"
                );
            }
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished Median and Motion source files"]
    fn neighborhood_import_rejects_unobserved_state() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        for (path, hash) in [
            ("median-raster-color-evidence/implicit-hdr-f64.source.xcf", "8f8001f3f31d6b9936f659ab18965a8df57f9e403297497f1d339e3539fe4442"),
            ("median-gray-color-evidence/implicit-gray-hdr-f64.source.xcf", "1149bcbe3cadc2075f4c7546a7eeaa0197f07677ce882a6dc75835483cd57cc7"),
            ("operations/gegl-motion-blur-linear/operation-gegl-motion-blur-linear.xcf", "7fd61309e9ad01f397b5127f0b51f2e2fae27f9d81c53c56305f02b6d24413e8"),
            ("operations/gegl-pixelize/operation-gegl-pixelize.xcf", "a4b746de40767cbd07467cad702cda4611c95e614c718164696ef475b2fe1f34"),
        ] {
            let bytes = std::fs::read(root.join(path)).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(super::map(path, file.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok(), "{path}");
            for change in 0..14 {
                let mut invalid = file.clone();
                let owner = &mut invalid.layers[0];
                let effect = &mut owner.effects[0];
                match change {
                    0 => effect.version = Some("future".into()),
                    1 => effect.state.region = Some(1),
                    2 => effect.state.enabled = Some(false),
                    3 => effect.mask.as_mut().unwrap().mask_shown = true,
                    4 => effect.mask.as_mut().unwrap().offset = (1, 0),
                    5 => {
                        let plane = effect.mask.as_mut().unwrap().plane.as_mut().unwrap();
                        if plane.sample == photocraft_xcf::Sample::F64 {
                            plane.bytes[..8].copy_from_slice(&0.5_f64.to_ne_bytes())
                        } else {
                            plane.bytes[0] = 127
                        }
                    }
                    6 => {
                        let name = match effect.operation.as_str() {
                            "gegl:median-blur" => "radius",
                            "gegl:pixelize" => "size-x",
                            _ => "length",
                        };
                        effect.arguments.iter_mut().find(|a| a.name == name).unwrap().value = photocraft_xcf::ArgumentValue::Float64Bits(3.0_f64.to_bits());
                    }
                    7 => effect.arguments.push(effect.arguments[0].clone()),
                    8 => {
                        effect.arguments.pop();
                    }
                    9 => effect.state.composite_space = Some(2),
                    10 => {
                        let plane = owner.plane.as_mut().unwrap();
                        if plane.sample == photocraft_xcf::Sample::F64 {
                            let alpha = (plane.channels - 1) * 8;
                            plane.bytes[alpha..alpha + 8].copy_from_slice(&2.0_f64.to_ne_bytes());
                        } else {
                            plane.bytes[3] = 127
                        }
                    }
                    11 => owner.offset = (1, 0),
                    12 => invalid.encoding = photocraft_xcf::Encoding::Linear,
                    _ => {
                        let (name, value) = match effect.operation.as_str() {
                            "gegl:median-blur" => ("high-precision", photocraft_xcf::ArgumentValue::Boolean(true)),
                            "gegl:pixelize" => ("ratio-x", photocraft_xcf::ArgumentValue::Float32Bits(0.5_f32.to_bits())),
                            _ => ("angle", photocraft_xcf::ArgumentValue::Float32Bits(45.0_f32.to_bits())),
                        };
                        effect.arguments.iter_mut().find(|a| a.name == name).unwrap().value = value;
                    }
                }
                assert!(super::map(path, invalid, &photocraft_raster::Interrupt::NONE, 128 << 20).is_err(), "{path} change {change}");
            }
            assert_eq!(std::fs::read(root.join(path)).unwrap(), bytes);
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires unpublished finite Gaussian source envelope"]
    fn finite_gaussian_rejects_unknown_state_and_hostile_native_samples() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let bytes = std::fs::read(root.join("gaussian-raster-color-evidence/implicit-hdr-f64.source.xcf")).unwrap();
        let observations: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("gaussian-raster-color-evidence/observations.json")).unwrap()).unwrap();
        let descriptor = observations["records"].as_array().unwrap().iter().find(|r| r["source"]["file"] == "implicit-hdr-f64.source.xcf").unwrap();
        assert_eq!(Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(), descriptor["source"]["sha256"]);
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(super::map("finite Gaussian", file.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
        for change in 0..11 {
            let mut invalid = file.clone();
            let owner = &mut invalid.layers[0];
            match change {
                0 => owner.effects[0].version = Some("future".into()),
                1 => owner.effects[0].state.blend_space = Some(-1474592174),
                2 => owner.effects[0].state.region = Some(1),
                3 => owner.effects[0].state.enabled = Some(false),
                4 => owner.effects[0].arguments.iter_mut().find(|a| a.name == "filter").unwrap().value = photocraft_xcf::ArgumentValue::Enum(1),
                5 => owner.effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..8].copy_from_slice(&0.5_f64.to_ne_bytes()),
                6 => owner.effects[0].mask.as_mut().unwrap().mask_shown = true,
                7 => owner.plane.as_mut().unwrap().bytes[..8].copy_from_slice(&f64::NAN.to_ne_bytes()),
                8 => owner.plane.as_mut().unwrap().bytes[24..32].copy_from_slice(&2.0_f64.to_ne_bytes()),
                9 => owner.offset = (1, 0),
                _ => invalid.encoding = photocraft_xcf::Encoding::Linear,
            }
            assert!(
                matches!(super::map("invalid Gaussian", invalid, &photocraft_raster::Interrupt::NONE, 128 << 20), Err(crate::IoError::Unsupported(_))),
                "change {change}"
            );
        }
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed unpublished full XCF corpus"]
    fn isolated_gaussian_group_rejects_unqualified_envelopes_and_masks() {
        use sha2::{Digest, Sha256};
        let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").map(std::path::PathBuf::from).expect("full corpus path").join("composition-matrix");
        let hashes: serde_json::Value = serde_json::from_str(include_str!("../../compose/tests/gimp_gaussian_group_hashes.json")).unwrap();
        let name = "group-28-group-mask.xcf";
        let bytes = std::fs::read(root.join(name)).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hash, hashes[name].as_str().unwrap());
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(super::map(name, file.clone(), &photocraft_raster::Interrupt::NONE, 128 << 20).is_ok());
        for change in 0..6 {
            let mut invalid = file.clone();
            let group = &mut invalid.layers[0];
            match change {
                0 => group.effects[0].state.region = Some(2),
                1 => group.effects[0].state.blend_space = Some(-643996975),
                2 => group.effects[0].arguments[0].value = photocraft_xcf::ArgumentValue::Float32Bits(0.9_f32.to_bits()),
                3 => group.effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.5_f32.to_ne_bytes()),
                4 => group.mode = 61,
                _ => group.effects[0].mask.as_mut().unwrap().offset = (1, 0),
            }
            assert!(
                matches!(super::map(name, invalid, &photocraft_raster::Interrupt::NONE, 128 << 20), Err(crate::IoError::Unsupported(_))),
                "change {change}"
            );
        }
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
    }
    #[test]
    fn floating_qualification_requires_exact_unique_record_binding_and_observed_policy() {
        use photocraft_xcf::{Compression, Drawable, Encoding, Model, Plane, Sample, XcfFile};
        let mut floating = Drawable {
            record_offset: 64,
            floating_attachment: Some(128),
            width: 3,
            height: 2,
            offset: (2, 1),
            model: Model::Rgb,
            alpha: true,
            visible: true,
            opacity: 1.0,
            mode: 28,
            blend_space: 0,
            composite_space: 2,
            composite_mode: 1,
            ..Default::default()
        };
        floating.plane = Some(Plane { width: 3, height: 2, channels: 4, sample: Sample::F32, bytes: vec![0; 96] });
        let mut target = floating.clone();
        target.record_offset = 128;
        target.floating_attachment = None;
        target.width = 8;
        target.height = 6;
        target.offset = (0, 0);
        target.plane = Some(Plane { width: 8, height: 6, channels: 4, sample: Sample::F32, bytes: vec![0; 768] });
        let file = XcfFile {
            version: 12,
            width: 8,
            height: 6,
            model: Model::Rgb,
            precision: 650,
            sample: Sample::F32,
            encoding: Encoding::NonlinearProfile,
            compression: Compression::None,
            icc: vec![],
            dpi: (72.0, 72.0),
            unit: photocraft_xcf::Unit::Inch,
            palette: vec![],
            layers: vec![floating, target],
            channels: vec![],
            guides: vec![],
            diagnostics: vec![],
            charged_bytes: 0,
            raw_properties: vec![],
            paths: vec![],
            tattoo: None,
            item_sets: vec![],
        };
        assert_eq!(super::qualified_floating(&file), Some((64, 128)));
        for change in 0..11 {
            let mut invalid = file.clone();
            match change {
                0 => invalid.layers[0].floating_attachment = Some(64),
                1 => invalid.layers[0].floating_attachment = Some(256),
                2 => invalid.layers[1].record_offset = 64,
                3 => invalid.layers[1].floating_attachment = Some(64),
                4 => invalid.layers.swap(0, 1),
                5 => invalid.layers[0].composite_mode = 2,
                6 => invalid.layers[0].offset = (-1, 1),
                7 => invalid.layers[0].opacity = 0.5,
                8 => invalid.icc.push(1),
                9 => invalid.encoding = Encoding::Linear,
                _ => invalid.layers[0].plane.as_mut().unwrap().channels = 3,
            }
            assert_eq!(super::qualified_floating(&invalid), None, "change {change}");
        }
        let mut channels = file.clone();
        channels.layers[0].floating_attachment = None;
        let mut channel = channels.layers[1].clone();
        channel.model = Model::Gray;
        channel.alpha = false;
        channel.plane = Some(Plane { width: 8, height: 6, channels: 1, sample: Sample::F32, bytes: vec![0; 192] });
        channel.visible = false;
        channels.channels.push(channel);
        let imported = super::map("hidden channel", channels.clone(), &photocraft_raster::Interrupt::NONE, 64 << 20).unwrap().0;
        assert_eq!(imported.document.channels.len(), 1);
        channels.channels[0].visible = true;
        assert!(matches!(super::map("visible channel", channels, &photocraft_raster::Interrupt::NONE, 64 << 20),
            Err(crate::IoError::Unsupported(message)) if message.contains("visible or spot channel")));
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "diagnostic for unpublished modified-vector source authority"]
    fn modified_vector_serialized_raster_mapping_diagnostic() {
        let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").map(std::path::PathBuf::from).expect("full corpus path");
        let bytes = std::fs::read(root.join("vector-modified.xcf")).unwrap();
        let mut file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let raw = std::fs::read(root.join("vector-modified.reopened.0.native.bin")).unwrap();
        assert!(file.layers[0].plane.as_ref().unwrap().bytes == raw, "serialized plane differs from independently reopened native pixels");
        let rect = super::bounds(&file.layers[0]).unwrap();
        let name = file.layers[0].name.clone();
        for layer in &file.layers {
            println!(
                "parsed layer {:?} bounds {:?} path {:?} visible {} opacity {} mode/spaces {:?}",
                layer.name,
                super::bounds(layer),
                layer.path,
                layer.visible,
                layer.opacity,
                (layer.mode, layer.composite_mode, layer.composite_space, layer.blend_space)
            );
        }
        // Isolate raster mapping from source regeneration. This diagnostic
        // grants no production admission and preserves the original input.
        for layer in &mut file.layers {
            layer.vector = None;
            layer.unsupported.retain(|reason| reason != "editable GIMP vector layer");
        }
        let (mapped, _) = super::map("diagnostic", file, &photocraft_raster::Interrupt::NONE, photocraft_xcf::Limits::default().resident_bytes).unwrap();
        let doc = mapped.document;
        for layer in &doc.layers {
            let mut pixel = [0.0; 4];
            layer.surface().unwrap().try_read_pixel_f64(12, 14, &mut pixel).unwrap();
            println!("mapped layer {:?} {:?} pixel(12,14) {:?}", layer.name, layer.id, pixel);
        }
        let layer = doc.layers.iter().find(|layer| layer.name == name).unwrap();
        assert!(layer.surface().unwrap().try_read_interleaved(rect).unwrap() == raw, "mapped Surface differs from serialized plane");
        let expected = photocraft_codecs::decode(&std::fs::read(root.join("vector-modified.ref.png")).unwrap()).unwrap();
        let actual = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let mut worst = 0.0_f64;
        let mut witness = (0, 0, 0, 0.0, 0.0);
        for (index, pixel) in actual.px.iter().enumerate() {
            let x = index as u32 % doc.size.width;
            let y = index as u32 / doc.size.width;
            for (channel, value) in pixel.iter().enumerate() {
                let reference = f64::from(expected.get(x, y, channel));
                let difference = (value - reference).abs();
                if difference > worst {
                    worst = difference;
                    witness = (x, y, channel, *value, reference);
                }
            }
        }
        assert!(worst <= 2.0 / 255.0, "raster-only mapping error {}/255 at {witness:?}", worst * 255.0);
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "requires reviewed full XCF originals not publicly pinned"]
    fn division_blends_reject_unqualified_source_endpoints_before_publication() {
        let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").map(std::path::PathBuf::from).expect("full corpus path");
        for (case, endpoints) in [("divide", vec![0.0_f32]), ("dodge", vec![1.0]), ("burn", vec![0.0]), ("vivid-light", vec![0.0, 1.0])] {
            let bytes = std::fs::read(root.join(format!("composition-matrix/mode-{case}.xcf"))).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            super::map("qualified", file.clone(), &photocraft_raster::Interrupt::NONE, photocraft_xcf::Limits::default().resident_bytes).unwrap();
            for endpoint in endpoints {
                let mut hostile = file.clone();
                hostile.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&endpoint.to_ne_bytes());
                let error = super::map("unqualified endpoint", hostile, &photocraft_raster::Interrupt::NONE, photocraft_xcf::Limits::default().resident_bytes)
                    .unwrap_err();
                assert!(matches!(error, crate::IoError::Unsupported(_)), "{case}/{endpoint}: {error}");
            }
        }
    }
    #[test]
    fn custom_unit_display_name_uses_legacy_plural_and_modern_name() {
        let mut source = photocraft_xcf::CustomUnit {
            factor: 2.5,
            digits: 3,
            name: "internal-unit-id".into(),
            symbol: "u".into(),
            abbreviation: "un".into(),
            singular: Some("unit".into()),
            plural: Some("Units".into()),
        };
        let photocraft_doc::PhysicalUnit::Custom(legacy) = super::physical_unit(&photocraft_xcf::Unit::Custom(source.clone())) else { panic!() };
        assert_eq!(legacy.name, "Units");
        assert_eq!(source.name, "internal-unit-id");
        assert_eq!(legacy.factor, 2.5);
        source.plural = None;
        source.singular = None;
        source.name = "Modern units".into();
        let photocraft_doc::PhysicalUnit::Custom(modern) = super::physical_unit(&photocraft_xcf::Unit::Custom(source)) else { panic!() };
        assert_eq!(modern.name, "Modern units");
    }
}
