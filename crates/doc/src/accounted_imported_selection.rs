//! Selection-only copy of a closed imported plain-raster model.
//! This preserves inert source evidence, not renderer/source qualification. The
//! enclosing codec guard continues to own all shared immutable payloads.
use super::*;
use crate::{xcf_editing::*, xcf_provenance::*, xcf_resources::*, xcf_state::*};
#[path = "accounted_imported_masked_normal.rs"]
mod masked_normal;
pub(super) fn validate_assigned_geometry(doc: &Document, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    masked_normal::validate_assigned(doc, quota, c)
}
pub(super) fn copy_masked_normal(doc: &Document, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<Document, BackingError> {
    masked_normal::validate_assigned(doc, quota, c)?;
    let mut copied = copy_document(doc, c)?;
    for (source, destination) in doc.layers.iter().zip(&mut copied.layers) {
        c.check(std::mem::size_of::<crate::LayerMask>() as u64)?;
        // Only the immutable Arc handle is cloned. Its original physical map,
        // default and native tiles remain paid by the retained codec guard.
        destination.mask = source.mask.clone();
    }
    c.check(1)?;
    Ok(copied)
}

pub(super) fn applies(doc: &Document) -> bool {
    doc.xcf_state.is_some() || doc.source_color_context.is_some()
}
// No allocation or native sample interpretation. IDs are checked without a set;
// every comparison consumes caller work, including failed validation attempts.
pub(super) fn validate(doc: &Document, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    if doc.selection_channel_id.is_some() || doc.layers.is_empty() || doc.layers.len() > 4096 {
        return Err(BackingError::Unsupported);
    }
    for (index, layer) in doc.layers.iter().enumerate() {
        c.check(1)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(BackingError::Unsupported) };
        if !backed.quota().shares_ledger(quota) || layer.id.0 == 0 {
            return Err(BackingError::Integrity);
        }
        if layer.mask.is_some()
            || layer.indexed_source.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
        {
            return Err(BackingError::Unsupported);
        }
        for earlier in doc.layers.get(..index).ok_or(BackingError::Length)? {
            c.check(1)?;
            if earlier.id == layer.id {
                return Err(BackingError::Integrity);
            }
        }
    }
    if let Some(s) = &doc.xcf_state {
        if s.selection.selected.len() > 4096
            || !s.source_items.is_empty()
            || !s.filter_stacks.is_empty()
            || !s.editable_indexed.is_empty()
            || !s.filter_masks.is_empty()
            || s.floating.is_some()
            || s.items.len() != doc.layers.len()
            || !s.selection.selected_paths.is_empty()
            || s.selection.mask_target.is_some()
        {
            return Err(BackingError::Unsupported);
        }
        for (index, item) in s.items.iter().enumerate() {
            c.check(1)?;
            let crate::item_identity::ItemRef::Layer(id) = item.item else { return Err(BackingError::Unsupported) };
            if item.parent.is_some() || item.mask.is_some() {
                return Err(BackingError::Unsupported);
            }
            let mut found = None;
            for layer in &doc.layers {
                c.check(1)?;
                if layer.id == id {
                    found = Some(layer)
                }
            }
            let layer = found.ok_or(BackingError::Integrity)?;
            let LayerContent::BackedRaster(backed) = &layer.content else { return Err(BackingError::Unsupported) };
            if item.raster_bounds != Some(backed.operative_bounds()) {
                return Err(BackingError::Integrity);
            }
            for earlier in s.items.get(..index).ok_or(BackingError::Length)? {
                c.check(1)?;
                if earlier.item == item.item {
                    return Err(BackingError::Integrity);
                }
            }
        }
        // These saved references are preserved verbatim; none may become an
        // anonymous pixel-selection owner or refer to an absent item.
        let reference = |id: crate::item_identity::ItemRef| -> Result<(), BackingError> {
            for item in &s.items {
                c.check(1)?;
                if item.item == id {
                    return Ok(());
                }
            }
            Err(BackingError::Integrity)
        };
        for id in &s.selection.selected {
            reference(*id)?;
        }
        if let Some(id) = s.selection.active {
            reference(id)?;
        }
        if s.resources.len() > 4096 || s.item_sets.len() > 4096 || s.opaque.len() > 4096 {
            return Err(BackingError::Length);
        }
        for (i, resource) in s.resources.iter().enumerate() {
            c.check(1)?;
            if resource.id.0 == 0 {
                return Err(BackingError::Integrity);
            }
            for previous in s.resources.get(..i).ok_or(BackingError::Length)? {
                c.check(1)?;
                if previous.id == resource.id {
                    return Err(BackingError::Integrity);
                }
            }
        }
        for set in &s.item_sets {
            c.check(1)?;
            if set.family != ItemFamily::Layer || set.members.len() > 4096 {
                return Err(BackingError::Unsupported);
            }
            for id in &set.members {
                reference(*id)?;
            }
        }
        for opaque in &s.opaque {
            c.check(1)?;
            if let Some(id) = opaque.owner {
                reference(id)?;
            }
        }
    }
    Ok(())
}
// Conservatively retain the complete shared immutable owner envelope in each
// imported selection snapshot as well as the codec's original physical lease.
// No bytes are copied, interpreted, hashed or read from a locator here.
pub(super) fn shared_payloads(doc: &Document, census: &mut CanonicalCensus<'_, '_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    let mut blob = |bytes: &Arc<Vec<u8>>, already_counted: bool| -> Result<(), BackingError> {
        c.check(1)?;
        if !already_counted {
            census.blob(bytes, false)?;
        }
        census.counted.index = census.counted.index.checked_add((std::mem::size_of::<Vec<u8>>() + 128) as u64).ok_or(BackingError::Length)?;
        for chunk in bytes.chunks(32768) {
            c.check(chunk.len() as u64)?;
        }
        Ok(())
    };
    if let Some(bytes) = &doc.icc_profile {
        blob(bytes, false)?;
    }
    if let Some(bytes) = doc.source_color_context.as_ref().and_then(|v| v.original_icc.as_ref()) {
        blob(bytes, false)?;
    }
    if let Some(bytes) = &doc.metadata.exif {
        blob(bytes, false)?;
    }
    for (_, _, bytes) in &doc.metadata.psd_resources {
        blob(bytes, false)?;
    }
    for (_, _, bytes) in &doc.metadata.psd_global_blocks {
        blob(bytes, false)?;
    }
    for layer in &doc.layers {
        c.check(1)?;
        if let Some(bytes) = &layer.effects.psd_raw {
            blob(bytes, false)?;
        }
        for (_, bytes) in &layer.psd_blocks {
            blob(bytes, false)?;
        }
    }
    if let Some(state) = &doc.xcf_state {
        for resource in &state.resources {
            c.check(1)?;
            if let Some(bytes) = &resource.owned_bytes {
                blob(bytes, true)?;
            }
        }
        for opaque in &state.opaque {
            blob(&opaque.bytes, true)?;
        }
    }
    Ok(())
}
fn vector<T>(len: usize) -> Result<Vec<T>, BackingError> {
    let mut result = Vec::new();
    result.try_reserve_exact(len).map_err(|_| BackingError::Allocation)?;
    if result.capacity() > len.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    Ok(result)
}
fn string(value: &str, c: &ReadControl<'_>) -> Result<String, BackingError> {
    let mut result = String::new();
    result.try_reserve_exact(value.len()).map_err(|_| BackingError::Allocation)?;
    if result.capacity() > value.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    let mut start = 0;
    while start < value.len() {
        let mut end = start.saturating_add(32768).min(value.len());
        while !value.is_char_boundary(end) {
            end = end.checked_sub(1).ok_or(BackingError::Length)?;
        }
        let part = value.get(start..end).ok_or(BackingError::Length)?;
        c.check(part.len() as u64)?;
        result.push_str(part);
        start = end;
    }
    Ok(result)
}
fn optional(value: &Option<String>, c: &ReadControl<'_>) -> Result<Option<String>, BackingError> {
    value.as_deref().map(|v| string(v, c)).transpose()
}
// Call only for scalar records or immutable Arc handles, never owned buffers.
fn copied<T: Clone>(values: &[T], c: &ReadControl<'_>) -> Result<Vec<T>, BackingError> {
    let mut result = vector(values.len())?;
    for value in values {
        c.check(std::mem::size_of::<T>() as u64)?;
        result.push(value.clone());
    }
    Ok(result)
}
fn composition(v: &crate::import_compositing::GimpComposition, c: &ReadControl<'_>) -> Result<crate::import_compositing::GimpComposition, BackingError> {
    use crate::import_compositing::GimpComposition;
    let GimpComposition { compatibility_epoch, source_mode, legacy, stored_blend_space, stored_composite_space, stored_operator, resolved, pass_through } = v;
    Ok(GimpComposition {
        compatibility_epoch: string(compatibility_epoch, c)?,
        source_mode: *source_mode,
        legacy: *legacy,
        stored_blend_space: *stored_blend_space,
        stored_composite_space: *stored_composite_space,
        stored_operator: *stored_operator,
        resolved: resolved.clone(),
        pass_through: *pass_through,
    })
}
fn state(s: &XcfState, c: &ReadControl<'_>) -> Result<XcfState, BackingError> {
    let XcfState {
        provenance: p,
        items,
        resources,
        source_items: _,
        editable_indexed: _,
        filter_stacks: _,
        current_filter_output_precision,
        filter_masks: _,
        floating: _,
        selection,
        item_sets,
        opaque,
    } = s;
    let mut fingerprints = vector(p.protected_source_fingerprints.len())?;
    for value in &p.protected_source_fingerprints {
        c.check(1)?;
        fingerprints.push(string(value, c)?);
    }
    let ProjectProvenance {
        format,
        format_version,
        container,
        producer,
        release_id,
        source_label,
        source_hash,
        original_locator,
        protected_source_fingerprints: _,
    } = p;
    let provenance = ProjectProvenance {
        format: string(format, c)?,
        format_version: *format_version,
        container: string(container, c)?,
        producer: optional(producer, c)?,
        release_id: optional(release_id, c)?,
        source_label: string(source_label, c)?,
        source_hash: optional(source_hash, c)?,
        original_locator: optional(original_locator, c)?,
        protected_source_fingerprints: fingerprints,
    };
    let mut new_items = vector(items.len())?;
    for value in items {
        c.check(1)?;
        let ItemState { item, name, custom_name, visible, locks, parent, source_tattoo, source_flags, raster_bounds, mask, composition: comp } = value;
        new_items.push(ItemState {
            item: *item,
            name: string(name, c)?,
            custom_name: *custom_name,
            visible: *visible,
            locks: *locks,
            parent: *parent,
            source_tattoo: *source_tattoo,
            source_flags: *source_flags,
            raster_bounds: *raster_bounds,
            mask: mask.clone(),
            composition: comp.as_ref().map(|v| composition(v, c)).transpose()?,
        });
    }
    let mut new_resources = vector(resources.len())?;
    for value in resources {
        c.check(1)?;
        let ResourceReference { id, kind, name, collection, internal, locator, content_hash, owned_bytes, resolution } = value;
        new_resources.push(ResourceReference {
            id: *id,
            kind: kind.clone(),
            name: string(name, c)?,
            collection: optional(collection, c)?,
            internal: *internal,
            locator: optional(locator, c)?,
            content_hash: optional(content_hash, c)?,
            owned_bytes: owned_bytes.clone(),
            resolution: resolution
                .as_ref()
                .map(|r| Ok::<_, BackingError>(ResourceResolution { fidelity: r.fidelity, payload_hash: string(&r.payload_hash, c)? }))
                .transpose()?,
        });
    }
    let mut new_sets = vector(item_sets.len())?;
    for value in item_sets {
        c.check(1)?;
        let ItemSet { id, name, family, rule, members } = value;
        let rule = match rule {
            ItemSetRule::Named => ItemSetRule::Named,
            ItemSetRule::Text(v) => ItemSetRule::Text(string(v, c)?),
            ItemSetRule::Regex(v) => ItemSetRule::Regex(string(v, c)?),
            ItemSetRule::Glob(v) => ItemSetRule::Glob(string(v, c)?),
        };
        new_sets.push(ItemSet { id: *id, name: string(name, c)?, family: *family, rule, members: copied(members, c)? });
    }
    let mut new_opaque = vector(opaque.len())?;
    for value in opaque {
        c.check(1)?;
        let OpaqueRecord { owner, namespace, identity, source_version, order, flags, bytes, blake3, rendering_required } = value;
        new_opaque.push(OpaqueRecord {
            owner: *owner,
            namespace: string(namespace, c)?,
            identity: string(identity, c)?,
            source_version: *source_version,
            order: *order,
            flags: *flags,
            bytes: bytes.clone(),
            blake3: string(blake3, c)?,
            rendering_required: *rendering_required,
        });
    }
    Ok(XcfState {
        provenance,
        items: new_items,
        resources: new_resources,
        source_items: Vec::new(),
        editable_indexed: Vec::new(),
        filter_stacks: Vec::new(),
        current_filter_output_precision: *current_filter_output_precision,
        filter_masks: Vec::new(),
        floating: None,
        selection: EditingSelection { selected: copied(&selection.selected, c)?, active: selection.active, selected_paths: Vec::new(), mask_target: None },
        item_sets: new_sets,
        opaque: new_opaque,
    })
}

fn layer(v: &crate::Layer, c: &ReadControl<'_>) -> Result<crate::Layer, BackingError> {
    use crate::{Effects, Layer};
    let Layer {
        indexed_source: _,
        import_compositing,
        id,
        name,
        visible,
        locks,
        blend,
        opacity,
        fill_opacity,
        clipped,
        mask: _,
        vector_mask: _,
        effects,
        label,
        content,
        psd_blocks,
        psd_id,
        fill_cache: _,
        link_group,
        excluded_channels,
        blend_if,
        advanced,
        video: _,
    } = v;
    let LayerContent::BackedRaster(backed) = content else { return Err(BackingError::Unsupported) };
    Ok(Layer {
        indexed_source: None,
        import_compositing: *import_compositing,
        id: *id,
        name: string(name, c)?,
        visible: *visible,
        locks: *locks,
        blend: *blend,
        opacity: *opacity,
        fill_opacity: *fill_opacity,
        clipped: *clipped,
        mask: None,
        vector_mask: None,
        effects: Effects { enabled: effects.enabled, items: Vec::new(), psd_raw: effects.psd_raw.clone(), reference: effects.reference },
        label: *label,
        content: LayerContent::BackedRaster(backed.clone()),
        psd_blocks: copied(psd_blocks, c)?,
        psd_id: *psd_id,
        fill_cache: None,
        link_group: *link_group,
        excluded_channels: *excluded_channels,
        blend_if: crate::blend_if::BlendIf { ranges: copied(&blend_if.ranges, c)? },
        advanced: *advanced,
        video: None,
    })
}
fn physical(v: &crate::physical_resolution::PhysicalResolution, c: &ReadControl<'_>) -> Result<crate::physical_resolution::PhysicalResolution, BackingError> {
    use crate::physical_resolution::*;
    let unit = match &v.unit {
        PhysicalUnit::Pixel => PhysicalUnit::Pixel,
        PhysicalUnit::Inch => PhysicalUnit::Inch,
        PhysicalUnit::Millimeter => PhysicalUnit::Millimeter,
        PhysicalUnit::Point => PhysicalUnit::Point,
        PhysicalUnit::Pica => PhysicalUnit::Pica,
        PhysicalUnit::Custom(u) => PhysicalUnit::Custom(CustomUnit {
            name: string(&u.name, c)?,
            factor: u.factor,
            digits: u.digits,
            symbol: string(&u.symbol, c)?,
            abbreviation: string(&u.abbreviation, c)?,
        }),
    };
    Ok(PhysicalResolution { x_dpi: v.x_dpi, y_dpi: v.y_dpi, unit })
}
pub(super) fn copy_document(doc: &Document, c: &ReadControl<'_>) -> Result<Document, BackingError> {
    // Called only after the complete selection-specific census and reservations.
    // Every owned string/vector copied below is fallible and polls per record or
    // <=32KiB text chunk. Immutable blobs/backings keep their original guard.
    let mut new_layers = vector(doc.layers.len())?;
    for value in &doc.layers {
        c.check(1)?;
        new_layers.push(layer(value, c)?);
    }
    let mut text = vector(doc.metadata.text.len())?;
    for (key, value) in &doc.metadata.text {
        c.check(1)?;
        text.push((string(key, c)?, string(value, c)?));
    }
    let mut resources = vector(doc.metadata.psd_resources.len())?;
    for (id, name, bytes) in &doc.metadata.psd_resources {
        c.check(1)?;
        resources.push((*id, string(name, c)?, bytes.clone()));
    }
    let new_metadata = crate::Metadata {
        xmp: optional(&doc.metadata.xmp, c)?,
        exif: doc.metadata.exif.clone(),
        text,
        psd_resources: resources,
        psd_global_blocks: copied(&doc.metadata.psd_global_blocks, c)?,
    };
    let new_working_profile_interpretation = match &doc.working_profile_interpretation {
        WorkingProfileInterpretation::Icc => WorkingProfileInterpretation::Icc,
        WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } => WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: string(icc_blake3, c)? },
        WorkingProfileInterpretation::ImplicitSGray { icc_blake3 } => WorkingProfileInterpretation::ImplicitSGray { icc_blake3: string(icc_blake3, c)? },
    };
    #[allow(unused_variables)]
    let Document {
        physical_resolution,
        xcf_state,
        sample_encoding,
        source_color_context,
        working_profile_interpretation,
        id,
        name,
        size,
        resolution_dpi,
        mode,
        depth,
        icc_profile,
        layers,
        channels,
        guides,
        selection: _,
        selection_channel_id,
        metadata,
        global_light,
        paths,
        work_path,
        clipping_path,
        quick_mask,
        quick_mask_unselected,
        patterns,
        color_table,
        duotone,
        layer_comps,
        last_applied_comp,
        last_document_state,
        measurement,
        notes,
        text_styles,
        slices,
        variables,
        timeline,
        file_encoding,
        ruler_origin,
    } = doc;
    let result = Document {
        physical_resolution: doc.physical_resolution.as_ref().map(|v| physical(v, c)).transpose()?,
        xcf_state: doc.xcf_state.as_ref().map(|v| state(v, c)).transpose()?,
        sample_encoding: sample_encoding.clone(),
        source_color_context: source_color_context.clone(),
        working_profile_interpretation: new_working_profile_interpretation,
        id: id.clone(),
        name: string(name, c)?,
        size: size.clone(),
        resolution_dpi: resolution_dpi.clone(),
        mode: mode.clone(),
        depth: depth.clone(),
        icc_profile: icc_profile.clone(),
        layers: new_layers,
        channels: Vec::new(),
        guides: crate::Guides { horizontal: copied(&guides.horizontal, c)?, vertical: copied(&guides.vertical, c)?, colors: copied(&guides.colors, c)? },
        selection: None,
        selection_channel_id: selection_channel_id.clone(),
        metadata: new_metadata,
        global_light: global_light.clone(),
        paths: Vec::new(),
        work_path: None,
        clipping_path: None,
        quick_mask: None,
        quick_mask_unselected: quick_mask_unselected.clone(),
        patterns: Vec::new(),
        color_table: None,
        duotone: None,
        layer_comps: Vec::new(),
        last_applied_comp: last_applied_comp.clone(),
        last_document_state: None,
        measurement: crate::analysis::Measurement {
            scale: crate::analysis::MeasurementScale {
                pixel_length: measurement.scale.pixel_length,
                logical_length: measurement.scale.logical_length,
                units: string(&measurement.scale.units, c)?,
            },
            count_groups: Vec::new(),
            active_count_group: measurement.active_count_group,
            ruler: measurement.ruler,
        },
        notes: Vec::new(),
        text_styles: crate::text_styles::TextStyles {
            character: Vec::new(),
            paragraph: Vec::new(),
            basic: crate::text_styles::ParagraphStyleDef {
                id: text_styles.basic.id,
                name: string(&text_styles.basic.name, c)?,
                para_attrs: Default::default(),
                char_attrs: Default::default(),
            },
            psd_raw: None,
        },
        slices: crate::slices::Slices { group_name: string(&slices.group_name, c)?, list: Vec::new() },
        variables: crate::variables::Variables { defs: Vec::new(), data_sets: Vec::new(), active: variables.active },
        timeline: None,
        file_encoding: file_encoding.clone(),
        ruler_origin: ruler_origin.clone(),
    };
    c.check(1)?;
    Ok(result)
}

#[cfg(test)]
#[path = "accounted_imported_selection_tests.rs"]
mod tests;
