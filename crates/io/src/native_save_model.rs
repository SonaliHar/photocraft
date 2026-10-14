//! Measured public String/Vec capacities for canonical and structural model containers.
//! Raster tiles and immutable cache/resource/source byte payloads are excluded.
//! This is not a total allocator/RSS claim for every legacy document subtype.
use super::{IoError, native_save::Payloads};
use photocraft_doc::{xcf_editing::*, xcf_filters::*, xcf_items::*, xcf_provenance::*, xcf_resources::*, *};
use photocraft_raster::Interrupt;
use std::mem::size_of;
use std::sync::Arc;
fn limit() -> IoError {
    photocraft_format::FormatError::LimitExceeded("native canonical model capacity".into()).into()
}
fn string(counter: &mut Payloads<'_, '_>, value: &String) -> Result<(), IoError> {
    counter.add(5, value.as_ptr() as usize, value.capacity())
}
fn optional(counter: &mut Payloads<'_, '_>, value: &Option<String>) -> Result<(), IoError> {
    if let Some(value) = value {
        string(counter, value)?;
    }
    Ok(())
}
fn slots<T>(counter: &mut Payloads<'_, '_>, values: &Vec<T>) -> Result<(), IoError> {
    counter.add(6, values.as_ptr() as usize, values.capacity().checked_mul(size_of::<T>()).ok_or_else(limit)?)
}
struct Walk<'a, 'c, 'i> {
    counter: &'a mut Payloads<'c, 'i>,
    control: &'a Interrupt<'i>,
    visits: usize,
    arguments: usize,
    filters: usize,
}
impl Walk<'_, '_, '_> {
    fn visit(&mut self, depth: usize) -> Result<(), IoError> {
        if self.control.cancelled() {
            return Err(IoError::Cancelled);
        }
        self.visits = self.visits.checked_add(1).ok_or_else(limit)?;
        if depth > 32 || self.visits > 20000000 {
            return Err(limit());
        }
        Ok(())
    }
    fn identity(&mut self, identity: &OperationIdentity) -> Result<(), IoError> {
        let OperationIdentity { name, stored_version, schema_epoch } = identity;
        for value in [name, stored_version, schema_epoch] {
            string(self.counter, value)?;
        }
        Ok(())
    }
    fn composition(&mut self, value: &Option<import_compositing::GimpComposition>) -> Result<(), IoError> {
        if let Some(value) = value {
            string(self.counter, &value.compatibility_epoch)?;
        }
        Ok(())
    }
    fn args(&mut self, args: &Vec<FilterArgument>, depth: usize) -> Result<(), IoError> {
        slots(self.counter, args)?;
        if args.len() > 100000 {
            return Err(limit());
        }
        for argument in args {
            self.arguments = self.arguments.checked_add(1).ok_or_else(limit)?;
            if self.arguments > 100000 {
                return Err(limit());
            }
            self.visit(depth)?;
            let FilterArgument { name, value } = argument;
            string(self.counter, name)?;
            match value {
                FilterValue::Number(_) | FilterValue::Bool(_) | FilterValue::Resource(_) => {}
                FilterValue::String(value) => string(self.counter, value)?,
                FilterValue::Enum { domain, .. } => string(self.counter, domain)?,
                FilterValue::Color { components, .. } => slots(self.counter, components)?,
                FilterValue::StoredColor(storage) => {
                    let photocraft_doc::xcf_filters::ExactColorStorage { layout, samples, opaque_tail } = storage;
                    // These owned mutable buffers are model-clone storage, not
                    // immutable raster/cache/resource Arc payloads.
                    string(self.counter, layout)?;
                    slots(self.counter, samples)?;
                    slots(self.counter, opaque_tail)?;
                    for bytes in [layout.as_bytes(), samples.as_slice(), opaque_tail.as_slice()] {
                        for _ in bytes.chunks(4096) {
                            self.visit(depth)?;
                        }
                    }
                }
                FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. } => {
                    slots(self.counter, points_f64_bits)?;
                    slots(self.counter, point_types)?;
                    slots(self.counter, samples_f64_bits)?;
                }
                FilterValue::Config(args) => self.args(args, depth + 1)?,
                FilterValue::Graph(graph) => self.graph(graph, depth + 1)?,
            }
        }
        Ok(())
    }
    fn graph(&mut self, graph: &FilterGraph, depth: usize) -> Result<(), IoError> {
        self.visit(depth)?;
        let FilterGraph { input: _, nodes, edges, output: _, original_pipeline } = graph;
        if nodes.len() > 4096 || edges.len() > 16384 {
            return Err(limit());
        }
        slots(self.counter, nodes)?;
        slots(self.counter, edges)?;
        optional(self.counter, original_pipeline)?;
        for node in nodes {
            self.visit(depth)?;
            self.identity(&node.operation)?;
            self.args(&node.arguments, depth + 1)?;
        }
        for edge in edges {
            self.visit(depth)?;
            string(self.counter, &edge.input_port)?;
            string(self.counter, &edge.output_port)?;
        }
        Ok(())
    }
    fn state(&mut self, state: &XcfState) -> Result<(), IoError> {
        // This current output marker is inline scalar state. This visitor counts
        // owned heap capacities; the marker has no separate heap payload.
        let XcfState {
            provenance,
            current_filter_output_precision: _,
            items,
            resources,
            source_items,
            editable_indexed,
            filter_stacks,
            filter_masks,
            floating: _,
            selection,
            item_sets,
            opaque,
        } = state;
        let ProjectProvenance {
            format,
            format_version: _,
            container,
            producer,
            release_id,
            source_label,
            source_hash,
            original_locator,
            protected_source_fingerprints,
        } = provenance;
        for value in [format, container, source_label] {
            string(self.counter, value)?;
        }
        for value in [producer, release_id, source_hash, original_locator] {
            optional(self.counter, value)?;
        }
        slots(self.counter, protected_source_fingerprints)?;
        for value in protected_source_fingerprints {
            self.visit(0)?;
            string(self.counter, value)?;
        }
        slots(self.counter, items)?;
        slots(self.counter, resources)?;
        slots(self.counter, source_items)?;
        slots(self.counter, editable_indexed)?;
        slots(self.counter, filter_stacks)?;
        slots(self.counter, filter_masks)?;
        slots(self.counter, item_sets)?;
        slots(self.counter, opaque)?;
        slots(self.counter, &selection.selected)?;
        slots(self.counter, &selection.selected_paths)?;
        for item in items {
            self.visit(0)?;
            string(self.counter, &item.name)?;
            self.composition(&item.composition)?;
        }
        for resource in resources {
            self.visit(0)?;
            let ResourceReference { id: _, kind: _, name, collection, internal: _, locator, content_hash, owned_bytes: _, resolution } = resource;
            string(self.counter, name)?;
            for value in [collection, locator, content_hash] {
                optional(self.counter, value)?;
            }
            if let Some(resolution) = resolution {
                string(self.counter, &resolution.payload_hash)?;
            }
        }
        for source in source_items {
            self.visit(0)?;
            let SourceItem { owner: _, source, transform: _, cache } = source;
            if let Some(cache) = cache {
                string(self.counter, &cache.blake3)?;
            }
            match source {
                EditableSource::Text { content, markup, configuration, .. } => {
                    string(self.counter, content)?;
                    optional(self.counter, markup)?;
                    self.args(configuration, 0)?;
                }
                EditableSource::Vector { configuration, .. } => self.args(configuration, 0)?,
                EditableSource::Linked { last_verified_hash, configuration, .. } => {
                    optional(self.counter, last_verified_hash)?;
                    self.args(configuration, 0)?;
                }
            }
        }
        for indexed in editable_indexed {
            self.visit(0)?;
            self.counter.add(7, indexed.raster.palette().as_ptr() as usize, indexed.raster.palette_capacity_bytes())?;
        }
        for stack in filter_stacks {
            self.visit(0)?;
            if stack.nodes.len() > 1024 {
                return Err(limit());
            }
            slots(self.counter, &stack.nodes)?;
            for node in &stack.nodes {
                self.filters = self.filters.checked_add(1).ok_or_else(limit)?;
                if self.filters > 100000 {
                    return Err(limit());
                }
                self.visit(0)?;
                self.identity(&node.operation)?;
                optional(self.counter, &node.source_display_name)?;
                optional(self.counter, &node.source_icon)?;
                self.args(&node.arguments, 0)?;
                self.composition(&node.composition)?;
                slots(self.counter, &node.auxiliary)?;
                for (name, _) in &node.auxiliary {
                    self.visit(0)?;
                    string(self.counter, name)?;
                }
            }
        }
        for set in item_sets {
            self.visit(0)?;
            string(self.counter, &set.name)?;
            slots(self.counter, &set.members)?;
            match &set.rule {
                ItemSetRule::Named => {}
                ItemSetRule::Text(value) | ItemSetRule::Regex(value) | ItemSetRule::Glob(value) => string(self.counter, value)?,
            }
        }
        for record in opaque {
            self.visit(0)?;
            string(self.counter, &record.namespace)?;
            string(self.counter, &record.identity)?;
            string(self.counter, &record.blake3)?;
        }
        Ok(())
    }
    fn legacy_document(&mut self, doc: &Document) -> Result<(), IoError> {
        let Metadata { xmp, exif: _, psd_resources, psd_global_blocks, text } = &doc.metadata;
        slots(self.counter, text)?;
        for (key, value) in text {
            self.visit(0)?;
            string(self.counter, key)?;
            string(self.counter, value)?;
        }
        optional(self.counter, xmp)?;
        slots(self.counter, psd_resources)?;
        slots(self.counter, psd_global_blocks)?;
        for (_, name, _) in psd_resources {
            self.visit(0)?;
            string(self.counter, name)?;
        }
        slots(self.counter, &doc.channels)?;
        for channel in doc.channels.iter().chain(doc.quick_mask.iter()) {
            self.visit(0)?;
            string(self.counter, &channel.name)?;
        }
        slots(self.counter, &doc.guides.horizontal)?;
        slots(self.counter, &doc.guides.vertical)?;
        slots(self.counter, &doc.guides.colors)?;
        slots(self.counter, &doc.paths)?;
        for path in &doc.paths {
            self.visit(0)?;
            string(self.counter, &path.name)?;
            self.path(&path.path)?;
        }
        if let Some(path) = &doc.work_path {
            self.path(path)?;
        }
        slots(self.counter, &doc.patterns)?;
        for pattern in &doc.patterns {
            self.visit(0)?;
            string(self.counter, &pattern.id)?;
            string(self.counter, &pattern.name)?;
        }
        Ok(())
    }
    fn path(&mut self, path: &Path) -> Result<(), IoError> {
        slots(self.counter, &path.subpaths)?;
        for subpath in &path.subpaths {
            self.visit(0)?;
            slots(self.counter, &subpath.knots)?;
        }
        Ok(())
    }
    fn layers(&mut self, layers: &Vec<Layer>, depth: usize) -> Result<(), IoError> {
        if depth > MAX_GROUP_DEPTH {
            return Err(limit());
        }
        slots(self.counter, layers)?;
        for layer in layers {
            self.visit(0)?;
            string(self.counter, &layer.name)?;
            if let Some(mask) = &layer.mask {
                self.mask_owner(mask)?;
            }
            if let LayerContent::Smart(smart) = &layer.content
                && let Some(mask) = &smart.filter_mask
            {
                self.mask_owner(mask)?;
            }
            if let Some(indexed) = &layer.indexed_source {
                self.counter.add(7, indexed.palette().as_ptr() as usize, indexed.palette_capacity_bytes())?;
            }
            if let LayerContent::Text(text) = &layer.content
                && let photocraft_doc::text::TextShape::Path { path, .. } = &text.shape
            {
                slots(self.counter, &path.knots)?;
                for _ in &path.knots {
                    self.visit(0)?;
                }
            }
            if let Some(video) = &layer.video {
                slots(self.counter, &video.frames)?;
                if let Some(original) = &video.original_frames {
                    slots(self.counter, original)?;
                }
                if let photocraft_doc::video::VideoSource::File { path } = &video.source {
                    string(self.counter, path)?;
                }
            }
            if let LayerContent::Group(group) = &layer.content {
                self.layers(&group.children, depth + 1)?;
            }
        }
        Ok(())
    }
    fn mask_owner(&mut self, mask: &LayerMask) -> Result<(), IoError> {
        // Surface moved out of inline LayerMask storage. Its distinct Arc
        // allocation must remain charged even when all raster tiles are shared.
        // This matches the mapper/preview's conservative compiled-owner bound;
        // it does not estimate opaque map nodes or total allocator/RSS overhead.
        let bytes = size_of::<Surface>().checked_add(128).ok_or_else(limit)?;
        self.counter.add(8, Arc::as_ptr(&mask.surface) as usize, bytes)
    }
}
pub(super) fn measure<'c, 'i>(doc: &Document, counter: &mut Payloads<'c, 'i>, control: &Interrupt<'i>) -> Result<usize, IoError> {
    let mut walk = Walk { counter, control, visits: 0, arguments: 0, filters: 0 };
    walk.visit(0)?;
    string(walk.counter, &doc.name)?;
    walk.layers(&doc.layers, 0)?;
    walk.legacy_document(doc)?;
    if let Some(state) = &doc.xcf_state {
        walk.state(state)?;
    }
    Ok(walk.visits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn exact_stored_color_owned_spare_capacities_are_model_storage() {
        let mut doc = Document::new("raw color model", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let layer = Layer::raster("one", doc.pixel_format());
        let id = layer.id;
        doc.layers.push(layer);
        let storage = ExactColorStorage { layout: "RGBA float".into(), samples: vec![0; 16], opaque_tail: vec![0; 4] };
        let node = FilterNode {
            id: item_identity::FilterId(2),
            operation: OperationIdentity { name: "caller:opaque-color".into(), stored_version: "stored".into(), schema_epoch: "caller".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument { name: "literal".into(), value: FilterValue::StoredColor(storage) }],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        };
        let mut state = XcfState::default();
        state.filter_stacks.push(FilterStack { owner: item_identity::ItemRef::Layer(id), nodes: vec![node], revision: 0 });
        doc.xcf_state = Some(state);
        let before = crate::retained_native_model_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
        let storage = match &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value {
            FilterValue::StoredColor(value) => value,
            _ => panic!("raw storage fixture"),
        };
        let old = storage.layout.capacity() + storage.samples.capacity() + storage.opaque_tail.capacity();
        storage.layout.reserve(512);
        storage.samples.reserve(1024);
        storage.opaque_tail.reserve(8192);
        let extra = storage.layout.capacity() + storage.samples.capacity() + storage.opaque_tail.capacity() - old;
        let after = crate::retained_native_model_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
        assert_eq!(after - before, extra);
        assert!(
            crate::retained_native_model_bytes(
                &doc,
                photocraft_format::PreparationLimits { resident_bytes: after - 1, ..Default::default() },
                &Interrupt::NONE
            )
            .is_err()
        );
    }

    #[test]
    fn shared_and_distinct_mask_surface_owners_have_separate_model_charges() {
        let mut shared = Document::new("mask owners", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mask = LayerMask::reveal_all();
        let mut first = Layer::raster("first", shared.pixel_format());
        first.mask = Some(mask.clone());
        let mut second = Layer::raster("second", shared.pixel_format());
        second.mask = Some(mask);
        shared.layers = vec![first, second];
        let mut distinct = shared.clone();
        let second = distinct.layers[1].mask.as_mut().unwrap();
        second.surface = Arc::new((*second.surface).clone());
        let model = |doc: &Document| {
            let payload = crate::retained_native_payload_bytes(doc, Default::default(), &Interrupt::NONE).unwrap();
            crate::retained_native_input_bytes(doc, Default::default(), &Interrupt::NONE).unwrap() - payload
        };
        assert_eq!(model(&distinct) - model(&shared), size_of::<Surface>() + 128);
        let required = crate::retained_native_input_bytes(&shared, Default::default(), &Interrupt::NONE).unwrap();
        let limits = photocraft_format::PreparationLimits { resident_bytes: required - 1, ..Default::default() };
        assert!(crate::retained_native_input_bytes(&shared, limits, &Interrupt::NONE).is_err());
        assert!(matches!(crate::retained_native_input_bytes(&shared, Default::default(), &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
        assert!(Arc::ptr_eq(&shared.layers[0].mask.as_ref().unwrap().surface, &shared.layers[1].mask.as_ref().unwrap().surface));
    }
}
