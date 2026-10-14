//! Actual owned backing of a decoded file, independent of historical decoder charges.
//!
//! This allocation-free successful traversal counts the root inline header, every
//! Vec allocation (including spare capacity), String capacity, and boxed pointee
//! header. Inline enum/Option/element headers are already included in their parent.
//! Allocator bookkeeping/alignment overhead, source bytes held outside this file,
//! and decoder reservations are excluded; this is not process RSS or source proof.
//! The opaque receipt borrows the exact immutable file, preventing safe mutation
//! while it is used. Callers reserve any additional held backing themselves.
//!
//! These owned Box/Vec trees cannot contain cycles in safe Rust. A bounded depth
//! and visit budget therefore suffice without allocating a seen-set. No pixels,
//! text, configuration semantics, locator, or profile is interpreted.
use crate::*;
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedBackingLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
    pub visits: u64,
    pub depth: usize,
}
impl Default for OwnedBackingLimits {
    fn default() -> Self {
        Self {
            resident_bytes: if cfg!(target_arch = "wasm32") { 512 << 20 } else { 2 << 30 },
            work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 },
            visits: 20_000_000,
            depth: 100,
        }
    }
}
pub struct OwnedBackingRequirements<'f> {
    file: &'f XcfFile<Plane>,
    retained_bytes: u64,
    work_units: u64,
    visits: u64,
}
impl std::fmt::Debug for OwnedBackingRequirements<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedBackingRequirements")
            .field("retained_bytes", &self.retained_bytes)
            .field("work_units", &self.work_units)
            .field("visits", &self.visits)
            .finish_non_exhaustive()
    }
}
impl<'f> OwnedBackingRequirements<'f> {
    pub fn file(&self) -> &'f XcfFile<Plane> {
        self.file
    }
    pub fn belongs_to(&self, file: &XcfFile<Plane>) -> bool {
        std::ptr::eq(self.file, file)
    }
    pub fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }
    /// No auxiliary index is allocated by this census.
    pub fn index_bytes(&self) -> u64 {
        0
    }
    /// Actual structural accounting operations, not sample work or measured CPU.
    pub fn work_units(&self) -> u64 {
        self.work_units
    }
    pub fn visits(&self) -> u64 {
        self.visits
    }
}
const PHASE: &str = "owned XCF backing census";
fn limit(message: &str) -> XcfError {
    XcfError::new(ErrorKind::Limit, 0, message)
}
/// Performs work/visits against the supplied shared Budget, with no refund/reset.
/// `poll` is the current caller control, independent of an earlier decoder token.
/// The receipt itself does not reserve Resident again: callers reconcile it with
/// their existing decoder lease before copying or retaining other prepared data.
pub fn census_owned_file<'f>(
    file: &'f XcfFile<Plane>,
    limits: OwnedBackingLimits,
    budget: &Budget<'_>,
    poll: &dyn Fn() -> Result<(), XcfError>,
) -> Result<OwnedBackingRequirements<'f>, XcfError> {
    poll()?;
    budget.poll(PHASE)?;
    for (kind, n) in [
        (BudgetKind::Resident, limits.resident_bytes),
        (BudgetKind::Work, limits.work_units),
        (BudgetKind::Visits, limits.visits),
        (BudgetKind::GroupDepth, u64::try_from(limits.depth).map_err(|_| limit("census depth overflow"))?),
    ] {
        budget.check(kind, n, PHASE)?;
    }
    let mut meter = Meter { budget, poll, limits, bytes: 0, work: 0, visits: 0 };
    meter.add(size_of::<XcfFile<Plane>>())?;
    meter.xcf_file(file, 0)?;
    poll()?;
    budget.poll(PHASE)?;
    Ok(OwnedBackingRequirements { file, retained_bytes: meter.bytes, work_units: meter.work, visits: meter.visits })
}
struct Meter<'b, 'c> {
    budget: &'b Budget<'c>,
    poll: &'b dyn Fn() -> Result<(), XcfError>,
    limits: OwnedBackingLimits,
    bytes: u64,
    work: u64,
    visits: u64,
}
impl Meter<'_, '_> {
    fn add(&mut self, n: usize) -> Result<(), XcfError> {
        let n = u64::try_from(n).map_err(|_| limit("census byte conversion overflow"))?;
        let total = self.bytes.checked_add(n).ok_or_else(|| limit("census backing overflow"))?;
        if total > self.limits.resident_bytes {
            return Err(limit("owned XCF backing exceeds resident limit"));
        }
        self.budget.check(BudgetKind::Resident, total, PHASE)?;
        self.bytes = total;
        Ok(())
    }
    fn touch(&mut self, d: usize) -> Result<(), XcfError> {
        (self.poll)()?;
        if d > self.limits.depth {
            return Err(limit("owned XCF backing exceeds depth limit"));
        }
        let w = self.work.checked_add(1).ok_or_else(|| limit("census work overflow"))?;
        let v = self.visits.checked_add(1).ok_or_else(|| limit("census visit overflow"))?;
        if w > self.limits.work_units || v > self.limits.visits {
            return Err(limit("owned XCF backing exceeds work/visit limit"));
        }
        self.budget.consume_work(1, PHASE)?;
        self.budget.consume_visits(1, PHASE)?;
        self.work = w;
        self.visits = v;
        Ok(())
    }
    fn child(d: usize) -> Result<usize, XcfError> {
        d.checked_add(1).ok_or_else(|| limit("census depth overflow"))
    }
    fn vec<T>(&mut self, v: &Vec<T>, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        self.add(v.capacity().checked_mul(size_of::<T>()).ok_or_else(|| limit("census vector capacity overflow"))?)?;
        Ok(())
    }
    /// Admit at least the immediate child visits before starting a container walk.
    /// Nested backing is admitted incrementally before its own child walk.
    fn walk<T>(&mut self, v: &Vec<T>, d: usize, visit: fn(&mut Self, &T, usize) -> Result<(), XcfError>) -> Result<(), XcfError> {
        self.vec(v, d)?;
        let n = u64::try_from(v.len()).map_err(|_| limit("census length overflow"))?;
        let work = self.work.checked_add(n).ok_or_else(|| limit("census work overflow"))?;
        let visits = self.visits.checked_add(n).ok_or_else(|| limit("census visit overflow"))?;
        if work > self.limits.work_units || visits > self.limits.visits {
            return Err(limit("owned XCF container exceeds work/visit limit"));
        }
        self.budget.check(BudgetKind::Work, self.budget.used(BudgetKind::Work).checked_add(n).ok_or_else(|| limit("census work overflow"))?, PHASE)?;
        self.budget.check(BudgetKind::Visits, self.budget.used(BudgetKind::Visits).checked_add(n).ok_or_else(|| limit("census visit overflow"))?, PHASE)?;
        let child = Self::child(d)?;
        for value in v {
            visit(self, value, child)?;
        }
        Ok(())
    }
    fn string(&mut self, s: &String, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        self.add(s.capacity())
    }
    fn string_pair(&mut self, p: &(String, String), d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let (a, b) = p;
        self.string(a, Self::child(d)?)?;
        self.string(b, Self::child(d)?)
    }
    fn plane(&mut self, v: &Plane, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let Plane { width: _, height: _, channels: _, sample, bytes } = v;
        let n = Self::child(d)?;
        self.sample(sample, n)?;
        self.vec(bytes, n)?;
        Ok(())
    }
    fn raw_property(&mut self, v: &RawProperty, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let RawProperty { tag: _, offset: _, declared_bytes: _, payload } = v;
        let n = Self::child(d)?;
        self.vec(payload, n)?;
        Ok(())
    }
    fn effect(&mut self, v: &Effect<Plane>, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let Effect { name, icon, operation, version, properties, arguments, mask, state } = v;
        let n = Self::child(d)?;
        self.string(name, n)?;
        self.string(icon, n)?;
        self.string(operation, n)?;
        if let Some(v) = version {
            self.string(v, n)?;
        }
        self.walk(properties, n, Self::raw_property)?;
        self.walk(arguments, n, Self::argument)?;
        if let Some(v) = mask {
            self.add(size_of::<Drawable<Plane>>())?;
            self.drawable(v, n)?;
        }
        self.effect_state(state, n)?;
        Ok(())
    }
    fn effect_state(&mut self, v: &EffectState, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let EffectState { enabled: _, opacity_u8: _, opacity_bits: _, mode: _, composite_mode: _, composite_space: _, blend_space: _, clipping: _, region: _ } =
            v;

        Ok(())
    }
    fn argument(&mut self, v: &Argument, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let Argument { name, property_index: _, value, configuration, graph } = v;
        let n = Self::child(d)?;
        self.string(name, n)?;
        self.argument_value(value, n)?;
        if let Some(v) = configuration {
            self.effect_configuration(v, n)?;
        }
        if let Some(v) = graph {
            self.stored_graph(v, n)?;
        }
        Ok(())
    }
    fn stored_graph(&mut self, v: &StoredGraph, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let StoredGraph { entries, nodes: _, edges: _ } = v;
        let n = Self::child(d)?;
        self.walk(entries, n, Self::graph_entry)?;
        Ok(())
    }
    fn effect_configuration(&mut self, v: &EffectConfiguration, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let EffectConfiguration { nodes, typed } = v;
        let n = Self::child(d)?;
        self.walk(nodes, n, Self::config_node)?;
        if let Some(v) = typed {
            self.typed_configuration(v, n)?;
        }
        Ok(())
    }
    fn hue_saturation_configuration(&mut self, v: &HueSaturationConfiguration, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let HueSaturationConfiguration { time_bits: _, ranges: _, overlap_bits: _ } = v;

        Ok(())
    }
    fn brightness_contrast_configuration(&mut self, v: &BrightnessContrastConfiguration, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let BrightnessContrastConfiguration { time_bits: _, brightness_bits: _, contrast_bits: _ } = v;

        Ok(())
    }
    fn color_balance_configuration(&mut self, v: &ColorBalanceConfiguration, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let ColorBalanceConfiguration { time_bits: _, ranges, preserve_luminosity: _ } = v;
        let n = Self::child(d)?;
        self.walk(ranges, n, Self::color_balance_range)?;
        Ok(())
    }
    fn color_balance_range(&mut self, v: &ColorBalanceRange, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let ColorBalanceRange { range, cyan_red_bits: _, magenta_green_bits: _, yellow_blue_bits: _ } = v;
        let n = Self::child(d)?;
        self.string(range, n)?;
        Ok(())
    }
    fn levels_configuration(&mut self, v: &LevelsConfiguration, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let LevelsConfiguration { time_bits: _, trc, clamp_input: _, clamp_output: _, channels } = v;
        let n = Self::child(d)?;
        if let Some(v) = trc {
            self.string(v, n)?;
        }
        self.walk(channels, n, Self::levels_channel)?;
        Ok(())
    }
    fn levels_channel(&mut self, v: &LevelsChannel, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let LevelsChannel { channel, low_input_bits: _, high_input_bits: _, gamma_bits: _, low_output_bits: _, high_output_bits: _ } = v;
        let n = Self::child(d)?;
        self.string(channel, n)?;
        Ok(())
    }
    fn curves_configuration(&mut self, v: &CurvesConfiguration, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let CurvesConfiguration { time_bits: _, trc, channels } = v;
        let n = Self::child(d)?;
        if let Some(v) = trc {
            self.string(v, n)?;
        }
        self.walk(channels, n, Self::curve_channel)?;
        Ok(())
    }
    fn curve_channel(&mut self, v: &CurveChannel, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let CurveChannel { channel, curve } = v;
        let n = Self::child(d)?;
        self.string(channel, n)?;
        self.stored_curve(curve, n)?;
        Ok(())
    }
    fn stored_curve(&mut self, v: &StoredCurve, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let StoredCurve { curve_type, points_bits, point_types, n_samples: _, samples_bits } = v;
        let n = Self::child(d)?;
        self.string(curve_type, n)?;
        self.vec(points_bits, n)?;
        self.walk(point_types, n, Self::string)?;
        self.vec(samples_bits, n)?;
        Ok(())
    }
    fn drawable(&mut self, v: &Drawable<Plane>, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let Drawable {
            record_offset: _,
            tattoo: _,
            active: _,
            legacy_linked: _,
            group_flags: _,
            color_tag: _,
            mask_editing: _,
            mask_shown: _,
            show_masked: _,
            channel_color: _,
            floating_attachment: _,
            item_sets,
            name,
            width: _,
            height: _,
            model,
            alpha: _,
            offset: _,
            opacity: _,
            visible: _,
            lock_alpha: _,
            lock_content: _,
            lock_position: _,
            lock_visibility: _,
            group: _,
            path,
            mode: _,
            composite_mode: _,
            composite_space: _,
            blend_space: _,
            mask_enabled: _,
            mask,
            plane,
            selection: _,
            unsupported,
            diagnostics,
            raw_properties,
            effects,
            link,
            transform,
            text,
            text_flags: _,
            vector,
        } = v;
        let n = Self::child(d)?;
        self.vec(item_sets, n)?;
        self.string(name, n)?;
        self.model(model, n)?;
        self.vec(path, n)?;
        if let Some(v) = mask {
            self.add(size_of::<Drawable<Plane>>())?;
            self.drawable(v, n)?;
        }
        if let Some(v) = plane {
            self.plane(v, n)?;
        }
        self.walk(unsupported, n, Self::string)?;
        self.walk(diagnostics, n, Self::string)?;
        self.walk(raw_properties, n, Self::raw_property)?;
        self.walk(effects, n, Self::effect)?;
        if let Some(v) = link {
            self.linked_layer(v, n)?;
        }
        if let Some(v) = transform {
            self.layer_transform(v, n)?;
        }
        if let Some(v) = text {
            self.text_layer(v, n)?;
        }
        if let Some(v) = vector {
            self.vector_layer(v, n)?;
        }
        Ok(())
    }
    fn stored_color(&mut self, v: &StoredColor, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let StoredColor { encoding, samples, icc } = v;
        let n = Self::child(d)?;
        self.string(encoding, n)?;
        self.vec(samples, n)?;
        self.vec(icc, n)?;
        Ok(())
    }
    fn stored_resource(&mut self, v: &StoredResource, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let StoredResource { name, collection, internal: _ } = v;
        let n = Self::child(d)?;
        self.string(name, n)?;
        self.string(collection, n)?;
        Ok(())
    }
    fn vector_layer(&mut self, v: &VectorLayer, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let VectorLayer { source_modified: _, path_tattoo: _, filled: _, stroked: _, fill, stroke } = v;
        let n = Self::child(d)?;
        self.vector_fill(fill, n)?;
        self.vector_stroke(stroke, n)?;
        Ok(())
    }
    fn vector_fill(&mut self, v: &VectorFill, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let VectorFill { style: _, antialias: _, color, pattern } = v;
        let n = Self::child(d)?;
        if let Some(v) = color {
            self.stored_color(v, n)?;
        }
        if let Some(v) = pattern {
            self.stored_resource(v, n)?;
        }
        Ok(())
    }
    fn vector_stroke(&mut self, v: &VectorStroke, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let VectorStroke { fill, width_bits: _, cap_style: _, join_style: _, miter_limit_bits: _, dashes_bits } = v;
        let n = Self::child(d)?;
        self.vector_fill(fill, n)?;
        self.vec(dashes_bits, n)?;
        Ok(())
    }
    fn text_layer(&mut self, v: &TextLayer, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let TextLayer { parasite_flags: _, raw, text, markup, font, configuration, unknown_fields, content, runs, styles, layout } = v;
        let n = Self::child(d)?;
        self.vec(raw, n)?;
        if let Some(v) = text {
            self.string(v, n)?;
        }
        if let Some(v) = markup {
            self.string(v, n)?;
        }
        if let Some(v) = font {
            self.font_reference(v, n)?;
        }
        self.walk(configuration, n, Self::config_node)?;
        self.walk(unknown_fields, n, Self::string)?;
        self.string(content, n)?;
        self.walk(runs, n, Self::text_run)?;
        self.walk(styles, n, Self::markup_style)?;
        self.text_layout(layout, n)?;
        Ok(())
    }
    fn text_layout(&mut self, v: &TextLayout, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let TextLayout {
            font_size_bits: _,
            font_size_unit,
            antialias: _,
            hinting: _,
            language,
            direction,
            justification,
            indent_bits: _,
            line_spacing_bits: _,
            letter_spacing_bits: _,
            box_mode,
            box_unit,
            box_width_bits: _,
            box_height_bits: _,
            offset_x_bits: _,
            offset_y_bits: _,
            transformation_bits: _,
        } = v;
        let n = Self::child(d)?;
        if let Some(v) = font_size_unit {
            self.string(v, n)?;
        }
        if let Some(v) = language {
            self.string(v, n)?;
        }
        if let Some(v) = direction {
            self.string(v, n)?;
        }
        if let Some(v) = justification {
            self.string(v, n)?;
        }
        if let Some(v) = box_mode {
            self.string(v, n)?;
        }
        if let Some(v) = box_unit {
            self.string(v, n)?;
        }
        Ok(())
    }
    fn text_run(&mut self, v: &TextRun, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let TextRun { start: _, end: _, style_ids } = v;
        let n = Self::child(d)?;
        self.vec(style_ids, n)?;
        Ok(())
    }
    fn markup_style(&mut self, v: &MarkupStyle, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let MarkupStyle { tag, attributes } = v;
        let n = Self::child(d)?;
        self.string(tag, n)?;
        self.walk(attributes, n, Self::string_pair)?;
        Ok(())
    }
    fn font_reference(&mut self, v: &FontReference, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let FontReference { name, family, style, postscript_name, hash, index: _, weight: _, slant: _, width: _, version: _, unknown_fields } = v;
        let n = Self::child(d)?;
        if let Some(v) = name {
            self.string(v, n)?;
        }
        if let Some(v) = family {
            self.string(v, n)?;
        }
        if let Some(v) = style {
            self.string(v, n)?;
        }
        if let Some(v) = postscript_name {
            self.string(v, n)?;
        }
        if let Some(v) = hash {
            self.string(v, n)?;
        }
        self.walk(unknown_fields, n, Self::string)?;
        Ok(())
    }
    fn config_node(&mut self, v: &ConfigNode, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let ConfigNode { start: _, end: _, value } = v;
        let n = Self::child(d)?;
        self.config_value(value, n)?;
        Ok(())
    }
    fn linked_layer(&mut self, v: &LinkedLayer, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let LinkedLayer { flags: _, path, width: _, height: _ } = v;
        let n = Self::child(d)?;
        self.string(path, n)?;
        Ok(())
    }
    fn layer_transform(&mut self, v: &LayerTransform, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let LayerTransform { offset: _, interpolation: _, matrix: _ } = v;

        Ok(())
    }
    fn guide(&mut self, v: &Guide, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let Guide { position: _, horizontal: _ } = v;

        Ok(())
    }
    fn xcf_file(&mut self, v: &XcfFile<Plane>, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let XcfFile {
            paths,
            tattoo: _,
            item_sets,
            version: _,
            width: _,
            height: _,
            model,
            precision: _,
            sample,
            encoding,
            compression,
            icc,
            dpi: _,
            unit,
            palette,
            layers,
            channels,
            guides,
            diagnostics,
            charged_bytes: _,
            raw_properties,
        } = v;
        let n = Self::child(d)?;
        self.walk(paths, n, Self::vector_path)?;
        self.walk(item_sets, n, Self::item_set)?;
        self.model(model, n)?;
        self.sample(sample, n)?;
        self.encoding(encoding, n)?;
        self.compression(compression, n)?;
        self.vec(icc, n)?;
        self.unit(unit, n)?;
        self.vec(palette, n)?;
        self.walk(layers, n, Self::drawable)?;
        self.walk(channels, n, Self::drawable)?;
        self.walk(guides, n, Self::guide)?;
        self.walk(diagnostics, n, Self::string)?;
        self.walk(raw_properties, n, Self::raw_property)?;
        Ok(())
    }
    fn vector_path(&mut self, v: &VectorPath, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let VectorPath {
            raw_record,
            legacy_format: _,
            legacy_state: _,
            record_offset: _,
            name,
            tattoo: _,
            selected: _,
            visible: _,
            legacy_linked: _,
            lock_content: _,
            lock_position: _,
            lock_visibility: _,
            color_tag: _,
            item_sets,
            properties,
            parasites,
            strokes,
            unsupported,
        } = v;
        let n = Self::child(d)?;
        self.vec(raw_record, n)?;
        self.string(name, n)?;
        self.vec(item_sets, n)?;
        self.walk(properties, n, Self::raw_property)?;
        self.walk(parasites, n, Self::path_parasite)?;
        self.walk(strokes, n, Self::path_stroke)?;
        self.walk(unsupported, n, Self::string)?;
        Ok(())
    }
    fn path_parasite(&mut self, v: &PathParasite, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let PathParasite { name, flags: _, bytes } = v;
        let n = Self::child(d)?;
        self.string(name, n)?;
        self.vec(bytes, n)?;
        Ok(())
    }
    fn path_stroke(&mut self, v: &PathStroke, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let PathStroke { closed: _, points } = v;
        let n = Self::child(d)?;
        self.walk(points, n, Self::path_point)?;
        Ok(())
    }
    fn path_point(&mut self, v: &PathPoint, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let PathPoint { kind: _, legacy_integer_coordinates: _, components } = v;
        let n = Self::child(d)?;
        self.vec(components, n)?;
        Ok(())
    }
    fn item_set(&mut self, v: &ItemSet, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let ItemSet { item_type: _, method: _, name } = v;
        let n = Self::child(d)?;
        self.string(name, n)?;
        Ok(())
    }
    fn custom_unit(&mut self, v: &CustomUnit, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let CustomUnit { factor: _, digits: _, name, symbol, abbreviation, singular, plural } = v;
        let n = Self::child(d)?;
        self.string(name, n)?;
        self.string(symbol, n)?;
        self.string(abbreviation, n)?;
        if let Some(v) = singular {
            self.string(v, n)?;
        }
        if let Some(v) = plural {
            self.string(v, n)?;
        }
        Ok(())
    }

    fn argument_value(&mut self, v: &ArgumentValue, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let n = Self::child(d)?;
        match v {
            ArgumentValue::Integer(_)
            | ArgumentValue::Boolean(_)
            | ArgumentValue::Unsigned(_)
            | ArgumentValue::Float32Bits(_)
            | ArgumentValue::Float64Bits(_)
            | ArgumentValue::Enum(_)
            | ArgumentValue::NullColor
            | ArgumentValue::Unknown(_) => {}
            ArgumentValue::String(s) | ArgumentValue::Config(s) => self.string(s, n)?,
            ArgumentValue::Color { encoding, samples, icc } => {
                self.string(encoding, n)?;
                self.vec(samples, n)?;
                self.vec(icc, n)?;
            }
        }
        Ok(())
    }
    fn graph_entry(&mut self, v: &GraphEntry, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let n = Self::child(d)?;
        match v {
            GraphEntry::Operation { identity, start: _, end: _ } => self.string(identity, n)?,
            GraphEntry::Property { name, value, start: _, end: _ } => {
                self.string(name, n)?;
                self.graph_value(value, n)?;
            }
        }
        Ok(())
    }
    fn graph_value(&mut self, v: &GraphValue, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let n = Self::child(d)?;
        match v {
            GraphValue::Literal { source, numeric_bits: _ } => self.string(source, n)?,
            GraphValue::Chain(v) => self.walk(v, n, Self::graph_entry)?,
        }
        Ok(())
    }
    fn config_value(&mut self, v: &ConfigValue, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let n = Self::child(d)?;
        match v {
            ConfigValue::Atom(s) => self.string(s, n)?,
            ConfigValue::Number { literal, bits: _ } => self.string(literal, n)?,
            ConfigValue::Quoted(v) => self.vec(v, n)?,
            ConfigValue::List(v) => self.walk(v, n, Self::config_node)?,
        }
        Ok(())
    }
    fn typed_configuration(&mut self, v: &TypedConfiguration, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        let n = Self::child(d)?;
        match v {
            TypedConfiguration::Levels(v) => self.levels_configuration(v, n)?,
            TypedConfiguration::Curves(v) => self.curves_configuration(v, n)?,
            TypedConfiguration::ColorBalance(v) => self.color_balance_configuration(v, n)?,
            TypedConfiguration::BrightnessContrast(v) => self.brightness_contrast_configuration(v, n)?,
            TypedConfiguration::HueSaturation(v) => self.hue_saturation_configuration(v, n)?,
        }
        Ok(())
    }
    fn unit(&mut self, v: &Unit, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        match v {
            Unit::Inch | Unit::Millimeter | Unit::Point | Unit::Pica => {}
            Unit::Custom(v) => self.custom_unit(v, Self::child(d)?)?,
        }
        Ok(())
    }
    fn model(&mut self, v: &Model, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        match v {
            Model::Rgb | Model::Gray | Model::Indexed => {}
        }
        Ok(())
    }
    fn compression(&mut self, v: &Compression, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        match v {
            Compression::None | Compression::Rle | Compression::Zlib => {}
        }
        Ok(())
    }
    fn sample(&mut self, v: &Sample, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        match v {
            Sample::U8 | Sample::U16 | Sample::U32 | Sample::F16 | Sample::F32 | Sample::F64 => {}
        }
        Ok(())
    }
    fn encoding(&mut self, v: &Encoding, d: usize) -> Result<(), XcfError> {
        self.touch(d)?;
        match v {
            Encoding::Linear | Encoding::NonlinearProfile | Encoding::Perceptual => {}
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "owned_backing_census_tests.rs"]
mod tests;
