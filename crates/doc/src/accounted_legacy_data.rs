//! Genuine Legacy pure-data edit admission. No Source or renderer authority.
use super::*;
pub(super) fn validate(owner: &AccountedDocument, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    if owner.owner._storage_class != StorageClass::Legacy || owner.document().selection_channel_id.is_some() {
        return Err(BackingError::Unsupported);
    }
    let doc = owner.document();
    match edit {
        AccountedEdit::LayerName { name, .. } => imported_edits::validate_name(name, c)?,
        AccountedEdit::LayerOpacity { opacity, .. } if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) => return Err(BackingError::Bounds),
        AccountedEdit::LayerMetadata { .. } => return Err(BackingError::Unsupported),
        _ => {}
    }
    // Complete structural/physical census precedes target, raw reads and no-op;
    // the true installed selection guard is never removed from this proof.
    selection_census(doc, false, owner.owner._selection.as_ref(), owner.quota(), c)?;
    let target = accounted_layer_access(doc, edit.layer_id(), c)?.ok_or(BackingError::Bounds)?;
    if let AccountedEdit::NativePixels { rect, bytes, .. } = edit {
        if let Some(state) = &doc.xcf_state {
            for source in &state.source_items {
                c.check(1)?;
                if source.owner == crate::item_identity::ItemRef::Layer(edit.layer_id()) {
                    return Err(BackingError::Unsupported);
                }
            }
        }
        let LayerContent::BackedRaster(backed) = &target.layer.content else { return Err(BackingError::Unsupported) };
        imported_edits::validate_native(backed, rect, bytes, c)?;
    }
    if matches!(edit, AccountedEdit::MaskEnabled { .. } | AccountedEdit::MaskLinked { .. }) {
        validate_mask_control(doc, target.layer, c)?;
    }
    c.check(1)
}
use crate::{xcf_editing::*, xcf_filters::*, xcf_items::*, xcf_provenance::*, xcf_resources::*, xcf_state::*};
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
        source_items,
        editable_indexed,
        filter_stacks,
        current_filter_output_precision,
        filter_masks,
        floating,
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
    if !editable_indexed.is_empty() || !filter_masks.is_empty() {
        return Err(BackingError::Unsupported);
    }
    let mut new_sources = vector(source_items.len())?;
    for item in source_items {
        c.check(1)?;
        let source = match &item.source {
            EditableSource::Text { content, markup, font, direction, source_modified, configuration, current_origin } => EditableSource::Text {
                content: string(content, c)?,
                markup: optional(markup, c)?,
                font: *font,
                direction: *direction,
                source_modified: *source_modified,
                configuration: arguments(configuration, 0, c)?,
                current_origin: *current_origin,
            },
            EditableSource::Vector { path, source_modified, configuration } => {
                EditableSource::Vector { path: *path, source_modified: *source_modified, configuration: arguments(configuration, 0, c)? }
            }
            EditableSource::Linked { resource, last_verified_hash, configuration, current } => EditableSource::Linked {
                resource: *resource,
                last_verified_hash: optional(last_verified_hash, c)?,
                configuration: arguments(configuration, 0, c)?,
                current: current.clone(),
            },
        };
        let cache = item
            .cache
            .as_ref()
            .map(|v| {
                Ok::<_, BackingError>(SourceCache { bytes: v.bytes.clone(), blake3: string(&v.blake3, c)?, width: v.width, height: v.height, format: v.format })
            })
            .transpose()?;
        new_sources.push(SourceItem { owner: item.owner, source, transform: item.transform.clone(), cache });
    }
    let mut new_stacks = vector(filter_stacks.len())?;
    for stack in filter_stacks {
        c.check(1)?;
        let mut nodes = vector(stack.nodes.len())?;
        for node in &stack.nodes {
            c.check(1)?;
            let mut auxiliary = vector(node.auxiliary.len())?;
            for (name, input) in &node.auxiliary {
                c.check(1)?;
                auxiliary.push((string(name, c)?, *input));
            }
            nodes.push(FilterNode {
                id: node.id,
                operation: operation(&node.operation, c)?,
                source_display_name: optional(&node.source_display_name, c)?,
                source_icon: optional(&node.source_icon, c)?,
                arguments: arguments(&node.arguments, 0, c)?,
                enabled: node.enabled,
                opacity_f64_bits: node.opacity_f64_bits,
                source_clipping: node.source_clipping,
                source_region: node.source_region,
                mask: node.mask,
                auxiliary,
                composition: node.composition.as_ref().map(|v| composition(v, c)).transpose()?,
            });
        }
        new_stacks.push(FilterStack { owner: stack.owner, nodes, revision: stack.revision });
    }
    Ok(XcfState {
        provenance,
        items: new_items,
        resources: new_resources,
        source_items: new_sources,
        editable_indexed: Vec::new(),
        filter_stacks: new_stacks,
        current_filter_output_precision: *current_filter_output_precision,
        filter_masks: Vec::new(),
        floating: floating.clone(),
        selection: EditingSelection {
            selected: copied(&selection.selected, c)?,
            active: selection.active,
            selected_paths: copied(&selection.selected_paths, c)?,
            mask_target: selection.mask_target,
        },
        item_sets: new_sets,
        opaque: new_opaque,
    })
}

fn operation(v: &OperationIdentity, c: &ReadControl<'_>) -> Result<OperationIdentity, BackingError> {
    Ok(OperationIdentity { name: string(&v.name, c)?, stored_version: string(&v.stored_version, c)?, schema_epoch: string(&v.schema_epoch, c)? })
}
fn arguments(values: &[FilterArgument], depth: usize, c: &ReadControl<'_>) -> Result<Vec<FilterArgument>, BackingError> {
    if depth > 32 {
        return Err(BackingError::Length);
    }
    let mut out = vector(values.len())?;
    for argument in values {
        c.check(1)?;
        let value = match &argument.value {
            FilterValue::Number(v) => FilterValue::Number(v.clone()),
            FilterValue::Bool(v) => FilterValue::Bool(*v),
            FilterValue::Resource(v) => FilterValue::Resource(*v),
            FilterValue::String(v) => FilterValue::String(string(v, c)?),
            FilterValue::Enum { domain, value } => FilterValue::Enum { domain: string(domain, c)?, value: *value },
            FilterValue::Color { components, encoding, profile } => {
                FilterValue::Color { components: copied(components, c)?, encoding: *encoding, profile: *profile }
            }
            FilterValue::StoredColor(ExactColorStorage { layout, samples, opaque_tail }) => {
                FilterValue::StoredColor(ExactColorStorage { layout: string(layout, c)?, samples: copied(samples, c)?, opaque_tail: copied(opaque_tail, c)? })
            }
            FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } => FilterValue::Curve {
                free: *free,
                points_f64_bits: copied(points_f64_bits, c)?,
                point_types: copied(point_types, c)?,
                samples_f64_bits: copied(samples_f64_bits, c)?,
            },
            FilterValue::Config(v) => FilterValue::Config(arguments(v, depth + 1, c)?),
            FilterValue::Graph(v) => {
                let mut nodes = vector(v.nodes.len())?;
                for node in &v.nodes {
                    c.check(1)?;
                    nodes.push(GraphNode {
                        id: node.id,
                        operation: operation(&node.operation, c)?,
                        version_origin: node.version_origin,
                        arguments: arguments(&node.arguments, depth + 2, c)?,
                    });
                }
                let mut edges = vector(v.edges.len())?;
                for edge in &v.edges {
                    c.check(1)?;
                    edges.push(GraphEdge {
                        from: edge.from,
                        output_port: string(&edge.output_port, c)?,
                        to: edge.to,
                        input_port: string(&edge.input_port, c)?,
                    });
                }
                FilterValue::Graph(FilterGraph { input: v.input, nodes, edges, output: v.output, original_pipeline: optional(&v.original_pipeline, c)? })
            }
        };
        out.push(FilterArgument { name: string(&argument.name, c)?, value });
    }
    Ok(out)
}
fn layer(v: &crate::Layer, c: &ReadControl<'_>, depth: usize) -> Result<crate::Layer, BackingError> {
    use crate::{Effects, Layer};
    if depth > crate::MAX_GROUP_DEPTH {
        return Err(BackingError::Length);
    }
    c.check(1)?;
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
    let content = match content {
        LayerContent::BackedRaster(backed) => LayerContent::BackedRaster(backed.clone()),
        LayerContent::Group(group) if depth < crate::MAX_GROUP_DEPTH && group.artboard.is_none() => {
            let mut children = vector(group.children.len())?;
            for child in &group.children {
                c.check(1)?;
                children.push(layer(child, c, depth + 1)?);
            }
            LayerContent::Group(crate::Group { children, expanded: group.expanded, artboard: None })
        }
        _ => return Err(BackingError::Unsupported),
    };
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
        mask: v.mask.clone(),
        vector_mask: None,
        effects: Effects { enabled: effects.enabled, items: Vec::new(), psd_raw: effects.psd_raw.clone(), reference: effects.reference },
        label: *label,
        content,
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
pub(super) fn copy(doc: &Document, c: &ReadControl<'_>) -> Result<Document, BackingError> {
    // Called only after the complete selection-specific census and reservations.
    // Every owned string/vector copied below is fallible and polls per record or
    // <=32KiB text chunk. Immutable blobs/backings keep their original guard.
    let mut new_layers = vector(doc.layers.len())?;
    for value in &doc.layers {
        c.check(1)?;
        new_layers.push(layer(value, c, 0)?);
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
        sample_encoding: *sample_encoding,
        source_color_context: source_color_context.clone(),
        working_profile_interpretation: new_working_profile_interpretation,
        id: *id,
        name: string(name, c)?,
        size: *size,
        resolution_dpi: *resolution_dpi,
        mode: *mode,
        depth: *depth,
        icc_profile: icc_profile.clone(),
        layers: new_layers,
        channels: Vec::new(),
        guides: crate::Guides { horizontal: copied(&guides.horizontal, c)?, vertical: copied(&guides.vertical, c)?, colors: copied(&guides.colors, c)? },
        selection: None,
        selection_channel_id: *selection_channel_id,
        metadata: new_metadata,
        global_light: *global_light,
        paths: Vec::new(),
        work_path: None,
        clipping_path: None,
        quick_mask: None,
        quick_mask_unselected: *quick_mask_unselected,
        patterns: Vec::new(),
        color_table: None,
        duotone: None,
        layer_comps: Vec::new(),
        last_applied_comp: *last_applied_comp,
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
        file_encoding: *file_encoding,
        ruler_origin: *ruler_origin,
    };
    c.check(1)?;
    Ok(result)
}

pub(super) fn synchronize(doc: &mut Document, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    let Some(state) = &mut doc.xcf_state else { return Ok(()) };
    match edit {
        AccountedEdit::LayerName { layer, name } => {
            for item in &mut state.items {
                c.check(1)?;
                if item.item == crate::item_identity::ItemRef::Layer(layer) {
                    item.name = string(name, c)?;
                    item.custom_name = true;
                    return Ok(());
                }
            }
            Err(BackingError::Integrity)
        }
        AccountedEdit::LayerVisibility { layer, visible } => {
            for item in &mut state.items {
                c.check(1)?;
                if item.item == crate::item_identity::ItemRef::Layer(layer) {
                    item.visible = visible;
                    return Ok(());
                }
            }
            Err(BackingError::Integrity)
        }
        AccountedEdit::MaskEnabled { layer, enabled } => {
            for item in &mut state.items {
                c.check(1)?;
                if let Some(mask) = &mut item.mask
                    && mask.owner == crate::item_identity::ItemRef::Layer(layer)
                {
                    mask.enabled = enabled;
                    return Ok(());
                }
            }
            Err(BackingError::Integrity)
        }
        AccountedEdit::MaskLinked { layer, linked } => {
            for item in &mut state.items {
                c.check(1)?;
                if let Some(mask) = &mut item.mask
                    && mask.owner == crate::item_identity::ItemRef::Layer(layer)
                {
                    mask.linked = linked;
                    return Ok(());
                }
            }
            Err(BackingError::Integrity)
        }
        AccountedEdit::LayerOpacity { .. } | AccountedEdit::NativePixels { .. } => Ok(()),
        AccountedEdit::LayerMetadata { .. } => Err(BackingError::Unsupported),
    }
}

// These historical assigned/isolated Source envelopes share Legacy storage.
// The coarse intent only preserves their restrictive endpoint; it grants no
// Source qualification and never retries generic data edits after a refusal.
pub(super) fn closed_geometry_intent(doc: &Document, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    c.check(1)?;
    if doc.layers.len() != 2 || !doc.layers.get(1).is_some_and(|v| v.mask.is_some() || matches!(v.content, LayerContent::Group(_))) {
        return Ok(false);
    }
    let Some(state) = &doc.xcf_state else { return Ok(false) };
    if matches!(state.provenance.format_version, 12 | 13) {
        return Ok(true);
    }
    if state.items.len() > 4096 {
        return Err(BackingError::Length);
    }
    for item in &state.items {
        c.check(1)?;
        if let Some(policy) = &item.composition {
            c.check(policy.compatibility_epoch.len() as u64)?;
            if matches!(
                policy.compatibility_epoch.as_str(),
                "own-accounted-isolated-normal-group-v1"
                    | crate::import_compositing::XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
                    | crate::import_compositing::XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH
                    | crate::import_compositing::XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
                    | crate::import_compositing::XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

// This is a routing discriminator for already retained ordinary-scene data,
// never a Source/profile/evaluator qualification. The full Legacy data census
// and actual-owner/guard proof remain mandatory after this bounded check.
pub(super) fn ordinary_metadata_intent(doc: &Document, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    use crate::import_compositing::{GimpOperator, GimpSpace, XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH};
    use crate::item_identity::ItemRef;
    c.check(64)?;
    let Some(state) = &doc.xcf_state else { return Ok(false) };
    let Some(source) = &doc.source_color_context else { return Ok(false) };
    if doc.layers.len() != 2
        || state.items.len() != 3
        || state.provenance.format_version != 12
        || state.provenance.format != "xcf"
        || !matches!(doc.mode, crate::ColorMode::Rgb | crate::ColorMode::Grayscale)
        || doc.sample_encoding != Some(crate::SampleEncoding::NonlinearProfile)
        || doc.working_profile_interpretation != crate::WorkingProfileInterpretation::Icc
        || !matches!(
            (doc.depth, source.source_precision),
            (crate::SampleType::U16, 250) | (crate::SampleType::U32, 350) | (crate::SampleType::F32, 650) | (crate::SampleType::F64, 750)
        )
        || !doc.channels.is_empty()
        || doc.selection_channel_id.is_some()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
    {
        return Ok(false);
    }
    let (Some(current), Some(original)) = (doc.icc_profile.as_deref(), source.original_icc.as_deref()) else { return Ok(false) };
    if !(132..=65536).contains(&current.len()) || current.len() != original.len() {
        return Ok(false);
    }
    for (a, b) in current.chunks(4096).zip(original.chunks(4096)) {
        c.check(a.len() as u64)?;
        if a != b {
            return Ok(false);
        }
    }
    let (Some(first), Some(second)) = (doc.layers.first(), doc.layers.get(1)) else { return Ok(false) };
    if first.id.0 == 0 || second.id.0 == 0 || first.id == second.id {
        return Ok(false);
    }
    let mut mask_records = 0usize;
    for (index, layer) in doc.layers.iter().enumerate() {
        c.check(64)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Ok(false) };
        let mut canonical = None;
        for item in &state.items {
            c.check(64)?;
            if item.item == ItemRef::Layer(layer.id) && canonical.replace(item).is_some() {
                return Ok(false);
            }
        }
        let Some(item) = canonical else { return Ok(false) };
        let Some(policy) = &item.composition else { return Ok(false) };
        c.check(
            (policy.compatibility_epoch.len() as u64)
                .checked_add(item.name.len() as u64)
                .and_then(|n| n.checked_add(layer.name.len() as u64))
                .ok_or(BackingError::Length)?,
        )?;
        let Some(resolved) = &policy.resolved else { return Ok(false) };
        let extent = backed.operative_bounds();
        if item.name != layer.name
            || item.visible != layer.visible
            || item.parent.is_some()
            || item.mask.is_some()
            || item.raster_bounds != Some(extent)
            || extent.is_empty()
            || policy.compatibility_epoch != XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH
            || policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || policy.stored_blend_space != 0
            || policy.stored_composite_space != if index == 0 { -1 } else { 1 }
            || policy.stored_operator != if index == 0 { -1 } else { 1 }
            || resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || resolved.operator != GimpOperator::Union
            || layer.blend != photocraft_color::BlendMode::Normal
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer
                .import_compositing
                .as_ref()
                .is_none_or(|p| p.pass_through || p.blend_encoding != crate::SampleEncoding::Linear || p.composite_encoding != crate::SampleEncoding::Linear)
        {
            return Ok(false);
        }
        if index == 0 {
            if extent != doc.bounds() || layer.mask.is_some() {
                return Ok(false);
            }
        } else {
            let Some(mask) = &layer.mask else { return Ok(false) };
            for record in &state.items {
                c.check(64)?;
                let Some(metadata) = &record.mask else { continue };
                if metadata.owner != ItemRef::Layer(layer.id) {
                    continue;
                }
                mask_records += 1;
                if !matches!(record.item, ItemRef::Mask(id) if id.0 != 0)
                    || record.parent.is_some()
                    || record.composition.is_some()
                    || record.raster_bounds != Some(extent)
                    || metadata.bounds != extent
                    || metadata.offset != [extent.x0, extent.y0]
                    || metadata.outside_f64_bits != 0f64.to_bits()
                    || metadata.enabled != mask.enabled
                    || metadata.linked != mask.linked
                    || metadata.shown
                    || metadata.editing
                    || mask.density.to_bits() != 1f32.to_bits()
                    || mask.feather.to_bits() != 0f32.to_bits()
                    || mask.surface.format() != crate::PixelFormat::new(crate::ColorMode::Grayscale, doc.depth, false)
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(mask_records == 1)
}
