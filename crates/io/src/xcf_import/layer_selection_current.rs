//! Opt-in current selected-storage projection of an independently held original.
//! Not a normal import, whole-scene reconstruction, render credential or source
//! grade. Unprojected channels/selection/metadata remain in the held original.
use super::{layer_selection_owner::*, *};
use photocraft_doc::{XcfState, xcf_filters::*, xcf_provenance::*, xcf_state::*};
use std::mem::size_of;

#[derive(Clone, Copy, Debug)]
pub struct LayerStorageRequirements {
    pub original_owner_bytes: u64,
    pub copy_resident_bytes: u64,
    pub profile_peak_bytes: u64,
    pub complete_resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
/// The borrow keeps the independent source backing alive during staging.
/// The caller must retain that owner and its enclosing reservation after moving
/// the document into a Session; neither this value nor its document grants rendering.
pub struct PrivateLayerCurrent<'o, 's, 'b, 'a> {
    original: &'o AssembledLayerSelection<'s, 'b, 'a>,
    document: Document,
    requirements: LayerStorageRequirements,
}
impl<'o, 's, 'b, 'a> PrivateLayerCurrent<'o, 's, 'b, 'a> {
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn requirements(&self) -> LayerStorageRequirements {
        self.requirements
    }
    pub fn into_parts(self) -> (Document, &'o AssembledLayerSelection<'s, 'b, 'a>, LayerStorageRequirements) {
        (self.document, self.original, self.requirements)
    }
}
fn bad(message: &str) -> IoError {
    IoError::Unsupported(format!("private layer current storage: {message}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)
}
fn raster(error: photocraft_raster::RasterInputError) -> IoError {
    if matches!(error, photocraft_raster::RasterInputError::Cancelled) { IoError::Cancelled } else { bad(&error.to_string()) }
}
fn caps(limits: LayerCurrentLimits) -> Result<(), IoError> {
    let maximum = LayerCurrentLimits::default();
    if limits.resident_bytes > maximum.resident_bytes || limits.index_bytes > maximum.index_bytes || limits.work_units > maximum.work_units {
        return Err(bad("lower-only limits"));
    }
    Ok(())
}
fn supported(layer: &Drawable) -> Result<(), IoError> {
    // Same concrete Normal modes as the existing native mapping. Do not adopt
    // its catch-all Normal fallback, resolve Auto spaces, or interpret filter words.
    if !matches!(layer.mode, 0 | 28) || !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) {
        return Err(bad("unsupported base layer mode/opacity"));
    }
    Ok(())
}
fn cms_limits(limits: LayerCurrentLimits) -> Result<photocraft_cms::control::Limits, IoError> {
    let max = photocraft_cms::control::Limits::default();
    Ok(photocraft_cms::control::Limits {
        resident_bytes: usize::try_from(limits.resident_bytes.min(max.resident_bytes as u64)).map_err(|_| bad("CMS address space"))?,
        work_units: limits.work_units.min(max.work_units),
    })
}
/// Prospective bounds derive from the actual held decoder/map leases, not caller
/// sizes. The copy upper deliberately includes shared Arc payloads again; no refund.
pub fn layer_storage_requirements(
    owner: &AssembledLayerSelection<'_, '_, '_>,
    policy: LayerProfilePolicy,
    limits: LayerCurrentLimits,
    ctl: &Interrupt<'_>,
) -> Result<LayerStorageRequirements, IoError> {
    poll(ctl)?;
    caps(limits)?;
    supported(owner.original_layer()?)?;
    let file = owner.original_file();
    if file.model != Model::Gray
        || file.encoding != Encoding::NonlinearProfile
        || !file.icc.is_empty()
        || !matches!(file.sample, Sample::U8 | Sample::F64)
        || file.width != 32
        || file.height != 32
    {
        return Err(bad("original closed Gray storage domain"));
    }
    let original = owner.correspondence_requirements()?;
    let copy = add(mul(original.resident_bytes, 2)?, 65536)?;
    let index = add(original.index_bytes, 65536)?;
    let (profile_peak, profile_work) = match policy {
        LayerProfilePolicy::OriginalExact => (0, 0),
        LayerProfilePolicy::MaterializedImplicitSGray => {
            let definition = photocraft_cms::Builtin::SGray.profile();
            let r = photocraft_cms::xcf_color::effective_profile_requirements_with_control(
                definition,
                photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                cms_limits(limits)?,
                &|| ctl.cancelled(),
            )
            .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
            // The encoded copy and marker coexist with the derived Profile.
            let encoded = definition.encoded_bytes().ok_or_else(|| bad("implicit Gray encoded definition absent"))?;
            (add(r.peak_resident_bytes as u64, add(encoded.len() as u64, 256)?)?, add(mul(r.work_units, 3)?, mul(encoded.len() as u64, 4)?)?)
        }
    };
    let resident = add(add(original.resident_bytes, copy)?, profile_peak)?;
    let work = add(add(original.work_units, mul(copy, 8)?)?, profile_work)?;
    if resident > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(bad("complete staging envelope"));
    }
    poll(ctl)?;
    Ok(LayerStorageRequirements {
        original_owner_bytes: original.resident_bytes,
        copy_resident_bytes: copy,
        profile_peak_bytes: profile_peak,
        complete_resident_bytes: resident,
        index_bytes: index,
        work_units: work,
    })
}
struct Copy<'c, 'i> {
    bytes: u64,
    maximum: u64,
    ctl: &'c Interrupt<'i>,
}
impl Copy<'_, '_> {
    fn take(&mut self, bytes: u64) -> Result<(), IoError> {
        poll(self.ctl)?;
        self.bytes = add(self.bytes, bytes)?;
        if self.bytes > self.maximum {
            return Err(bad("actual copied capacity"));
        }
        Ok(())
    }
    fn vector<T>(&mut self, count: usize) -> Result<Vec<T>, IoError> {
        let declared = mul(count as u64, size_of::<T>() as u64)?;
        self.take(declared)?;
        let mut out = Vec::new();
        out.try_reserve_exact(count).map_err(|_| bad("vector allocation"))?;
        self.take(mul(out.capacity() as u64, size_of::<T>() as u64)?.saturating_sub(declared))?;
        Ok(out)
    }
    fn text(&mut self, value: &str) -> Result<String, IoError> {
        if value.len() > 65536 {
            return Err(bad("bounded text"));
        }
        let mut bytes = self.vector::<u8>(value.len())?;
        for chunk in value.as_bytes().chunks(4096) {
            poll(self.ctl)?;
            bytes.extend_from_slice(chunk);
        }
        String::from_utf8(bytes).map_err(|_| bad("text encoding"))
    }
    fn optional(&mut self, value: &Option<String>) -> Result<Option<String>, IoError> {
        value.as_deref().map(|v| self.text(v)).transpose()
    }
    fn surface(&mut self, source: &Surface) -> Result<Surface, IoError> {
        self.take(add(source.resident_accounted_bytes().map_err(raster)?, 512)?)?;
        source.try_convert_with(source.format(), self.ctl).map_err(raster)
    }
    fn composition(
        &mut self,
        value: &Option<photocraft_doc::import_compositing::GimpComposition>,
    ) -> Result<Option<photocraft_doc::import_compositing::GimpComposition>, IoError> {
        let Some(v) = value else { return Ok(None) };
        // Every remaining field is fixed-sized; avoid cloning its input String.
        Ok(Some(photocraft_doc::import_compositing::GimpComposition {
            compatibility_epoch: self.text(&v.compatibility_epoch)?,
            source_mode: v.source_mode,
            legacy: v.legacy,
            stored_blend_space: v.stored_blend_space,
            stored_composite_space: v.stored_composite_space,
            stored_operator: v.stored_operator,
            resolved: v.resolved.clone(),
            pass_through: v.pass_through,
        }))
    }
    fn state(&mut self, source: &XcfState) -> Result<XcfState, IoError> {
        if source.items.len() != 2
            || source.filter_stacks.len() != 1
            || source.filter_masks.len() != 1
            || source.opaque.len() > 128
            || !source.resources.is_empty()
            || !source.source_items.is_empty()
            || !source.editable_indexed.is_empty()
            || source.floating.is_some()
            || !source.item_sets.is_empty()
            || source.selection != Default::default()
            || source.provenance.protected_source_fingerprints.len() > 32
        {
            return Err(bad("closed projected state shape"));
        }
        let p = &source.provenance;
        let mut fingerprints = self.vector(p.protected_source_fingerprints.len())?;
        for v in &p.protected_source_fingerprints {
            fingerprints.push(self.text(v)?);
        }
        let provenance = ProjectProvenance {
            format: self.text(&p.format)?,
            format_version: p.format_version,
            container: self.text(&p.container)?,
            producer: self.optional(&p.producer)?,
            release_id: self.optional(&p.release_id)?,
            source_label: self.text(&p.source_label)?,
            source_hash: self.optional(&p.source_hash)?,
            original_locator: self.optional(&p.original_locator)?,
            protected_source_fingerprints: fingerprints,
        };
        let mut items = self.vector(2)?;
        for v in &source.items {
            items.push(ItemState {
                item: v.item,
                name: self.text(&v.name)?,
                custom_name: v.custom_name,
                visible: v.visible,
                locks: v.locks,
                parent: v.parent,
                source_tattoo: v.source_tattoo,
                source_flags: v.source_flags,
                raster_bounds: v.raster_bounds,
                mask: v.mask.clone(),
                composition: self.composition(&v.composition)?,
            });
        }
        let stack = source.filter_stacks.first().ok_or_else(|| bad("stack absent"))?;
        if stack.nodes.len() != 1 {
            return Err(bad("stack shape"));
        }
        let n = stack.nodes.first().ok_or_else(|| bad("node absent"))?;
        if n.arguments.len() != 5 || !n.auxiliary.is_empty() {
            return Err(bad("node shape"));
        }
        let mut arguments = self.vector(5)?;
        for a in &n.arguments {
            poll(self.ctl)?;
            let value = match &a.value {
                FilterValue::Number(v) => FilterValue::Number(v.clone()),
                FilterValue::Bool(v) => FilterValue::Bool(*v),
                FilterValue::Enum { domain, value } => FilterValue::Enum { domain: self.text(domain)?, value: *value },
                _ => return Err(bad("unexpected argument storage")),
            };
            arguments.push(FilterArgument { name: self.text(&a.name)?, value });
        }
        let mut nodes = self.vector(1)?;
        nodes.push(FilterNode {
            id: n.id,
            operation: OperationIdentity {
                name: self.text(&n.operation.name)?,
                stored_version: self.text(&n.operation.stored_version)?,
                schema_epoch: self.text(&n.operation.schema_epoch)?,
            },
            source_display_name: self.optional(&n.source_display_name)?,
            source_icon: self.optional(&n.source_icon)?,
            arguments,
            enabled: n.enabled,
            opacity_f64_bits: n.opacity_f64_bits,
            source_clipping: n.source_clipping,
            source_region: n.source_region,
            mask: n.mask,
            auxiliary: Vec::new(),
            composition: self.composition(&n.composition)?,
        });
        let mut stacks = self.vector(1)?;
        stacks.push(FilterStack { owner: stack.owner, nodes, revision: stack.revision });
        let mask = source.filter_masks.first().ok_or_else(|| bad("mask absent"))?;
        let mut masks = self.vector(1)?;
        masks.push(photocraft_doc::FilterMask { id: mask.id, owner: mask.owner, surface: self.surface(&mask.surface)?, bounds: mask.bounds });
        let mut opaque = self.vector(source.opaque.len())?;
        for r in &source.opaque {
            poll(self.ctl)?;
            opaque.push(OpaqueRecord {
                owner: r.owner,
                namespace: self.text(&r.namespace)?,
                identity: self.text(&r.identity)?,
                source_version: r.source_version,
                order: r.order,
                flags: r.flags,
                bytes: Arc::clone(&r.bytes),
                blake3: self.text(&r.blake3)?,
                rendering_required: r.rendering_required,
            });
        }
        Ok(XcfState {
            provenance,
            items,
            filter_stacks: stacks,
            filter_masks: masks,
            opaque,
            current_filter_output_precision: source.current_filter_output_precision,
            ..Default::default()
        })
    }
    fn unit(&mut self, unit: &photocraft_xcf::Unit) -> Result<photocraft_doc::PhysicalUnit, IoError> {
        use photocraft_doc::{CustomUnit, PhysicalUnit};
        Ok(match unit {
            photocraft_xcf::Unit::Inch => PhysicalUnit::Inch,
            photocraft_xcf::Unit::Millimeter => PhysicalUnit::Millimeter,
            photocraft_xcf::Unit::Point => PhysicalUnit::Point,
            photocraft_xcf::Unit::Pica => PhysicalUnit::Pica,
            photocraft_xcf::Unit::Custom(u) => PhysicalUnit::Custom(CustomUnit {
                name: self.text(u.plural.as_ref().unwrap_or(&u.name))?,
                factor: f64::from(u.factor),
                digits: u.digits,
                symbol: self.text(&u.symbol)?,
                abbreviation: self.text(&u.abbreviation)?,
            }),
        })
    }
}
/// Both callbacks debit the same enclosing held ledger before ANY private copy.
/// The returned scene is a selected storage projection only; full original
/// channels, selection, guides and unrelated metadata remain with `owner`.
#[allow(clippy::too_many_arguments)]
pub fn assemble_private_layer_current<'o, 's, 'b, 'a>(
    owner: &'o AssembledLayerSelection<'s, 'b, 'a>,
    policy: LayerProfilePolicy,
    limits: LayerCurrentLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<PrivateLayerCurrent<'o, 's, 'b, 'a>, IoError> {
    let requirements = layer_storage_requirements(owner, policy, limits, ctl)?;
    reserve(requirements.complete_resident_bytes, requirements.index_bytes)?;
    charge(requirements.work_units)?;
    poll(ctl)?;
    let mut copy = Copy { bytes: 0, maximum: requirements.copy_resident_bytes, ctl };
    copy.take((size_of::<Document>() + size_of::<PrivateLayerCurrent<'o, 's, 'b, 'a>>()) as u64)?;
    let file = owner.original_file();
    let original = owner.original_layer()?;
    let mut document = Document::new(copy.text(&original.name)?, Size::new(file.width, file.height), ColorMode::Grayscale, depth(file.sample));
    document.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    document.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: file.precision });
    // A closed unit token is required; no guessed/custom-unit conversion.
    document
        .set_physical_resolution(photocraft_doc::PhysicalResolution {
            x_dpi: f64::from(file.dpi.0),
            y_dpi: f64::from(file.dpi.1),
            unit: copy.unit(&file.unit)?,
        })
        .map_err(|e| bad(&e.to_string()))?;
    let mut layer = Layer::new(copy.text(&original.name)?, LayerContent::Raster(copy.surface(owner.source_surface())?));
    layer.id = owner.layer_id();
    layer.opacity = original.opacity;
    layer.visible = original.visible;
    layer.blend = BlendMode::Normal;
    layer.locks.transparency = original.lock_alpha;
    layer.locks.pixels = original.lock_content;
    layer.locks.position = original.lock_position;
    layer.label = layer_color_tag(original.color_tag).map_err(bad)?;
    // No resolved import_compositing is inferred. Native raw spaces/mode and
    // visibility lock remain exactly retained in canonical items/raw records.
    let mut layers = copy.vector(1)?;
    layers.push(layer);
    document.layers = layers;
    document.xcf_state = Some(copy.state(owner.state())?);
    if policy == LayerProfilePolicy::MaterializedImplicitSGray {
        let profile = photocraft_cms::xcf_color::effective_profile_from_definition_with_control(
            photocraft_cms::Builtin::SGray.profile(),
            photocraft_cms::xcf_color::Encoding::NonlinearProfile,
            cms_limits(limits)?,
            &|| ctl.cancelled(),
        )
        .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
        let bytes = profile.encoded_bytes().ok_or_else(|| bad("generated Gray ICC absent"))?;
        let mut owned = copy.vector(bytes.len())?;
        for chunk in bytes.chunks(4096) {
            poll(ctl)?;
            owned.extend_from_slice(chunk);
        }
        copy.take(64)?;
        document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(&owned);
        if let photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { icc_blake3 } = &document.working_profile_interpretation {
            copy.take(icc_blake3.capacity().saturating_sub(64) as u64)?;
        }
        // Move the admitted Vec without a second encoded payload. Arc's private
        // header/allocation is covered by this conservative fixed reservation,
        // separate from the already reconciled Vec capacity (not an RSS claim).
        copy.take(128)?;
        document.icc_profile = Some(Arc::new(owned));
        drop(profile);
    }
    poll(ctl)?;
    let preparation = photocraft_format::PreparationLimits {
        resident_bytes: usize::try_from(requirements.copy_resident_bytes).map_err(|_| bad("census resident address"))?,
        index_bytes: usize::try_from(requirements.index_bytes).map_err(|_| bad("census index address"))?,
        ..Default::default()
    };
    let census = crate::NativeCurrentCensus::join(
        crate::retained_native_model_census(&document, preparation, ctl)?,
        crate::retained_native_payload_census(&document, preparation, ctl)?,
    )?;
    if add(census.retained_bytes() as u64, census.known_map_reservation_bytes() as u64)? > requirements.copy_resident_bytes
        || census.known_index_reservation_bytes() as u64 > requirements.index_bytes
    {
        return Err(bad("actual staging census"));
    }
    poll(ctl)?;
    Ok(PrivateLayerCurrent { original: owner, document, requirements })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_identity_copy_keeps_nonunit_words_signed_zero_and_shared_tile_identity() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let words = [1.0000000361224046f64.to_bits(), (-0f64).to_bits(), 1u64];
        let bytes = words.into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_from_interleaved_with(format, Rect::new(0, 0, 3, 1), &bytes, &Interrupt::NONE).unwrap();
        let mut c = Copy { bytes: 0, maximum: 1 << 20, ctl: &Interrupt::NONE };
        let copied = c.surface(&source).unwrap();
        assert_eq!(copied.default_bytes(), source.default_bytes());
        for ((coord, a), (other, b)) in source.tiles().zip(copied.tiles()) {
            assert_eq!(coord, other);
            assert!(Arc::ptr_eq(a, b));
        }
        for (x, word) in words.into_iter().enumerate() {
            let mut raw = [0; 8];
            copied.try_read_pixel_bytes(x as i32, 0, &mut raw).unwrap();
            assert_eq!(u64::from_ne_bytes(raw), word);
        }
    }
    #[test]
    fn unsupported_base_modes_and_nonfinite_opacity_are_not_normalized() {
        let mut layer = Drawable { mode: 28, ..Default::default() };
        assert!(supported(&layer).is_ok());
        layer.mode = 99;
        assert!(supported(&layer).is_err());
        layer.mode = 0;
        layer.opacity = f32::NAN;
        assert!(supported(&layer).is_err());
    }
    #[test]
    fn capacity_and_cancellation_refuse_before_copy() {
        let mut c = Copy { bytes: 0, maximum: 3, ctl: &Interrupt::NONE };
        assert!(c.text("four").is_err());
        assert!(add(u64::MAX, 1).is_err());
        let stop = || true;
        let ctl = Interrupt::cancel_only(&stop);
        let mut c = Copy { bytes: 0, maximum: 65536, ctl: &ctl };
        assert!(matches!(c.vector::<u8>(1), Err(IoError::Cancelled)));
    }
}
