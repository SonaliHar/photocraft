//! Inert editable-source DTO mapping. This module never resolves resources or admits rendering.
use crate::IoError;
use photocraft_doc::{
    ColorMode, PixelFormat, SampleType,
    item_identity::{ItemRef, PathId, ResourceId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
    xcf_items::{EditableSource, SourceCache, SourceItem, TransformState},
    xcf_provenance::OpaqueRecord,
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{ConfigNode, ConfigValue, Drawable, Plane, Sample, StoredColor, StoredResource, VectorFill};
pub struct SourceContext {
    pub owner: ItemRef,
    pub vector_path: Option<PathId>,
    pub format: PixelFormat,
    pub source_version: u32,
}
pub struct MappedSource {
    pub item: SourceItem,
    pub resources: Vec<ResourceReference>,
    pub opaque: Vec<OpaqueRecord>,
}
struct Builder<'a, 'b> {
    ctl: &'a Interrupt<'b>,
    charge: &'a mut dyn FnMut(usize) -> Result<(), IoError>,
    fresh: &'a mut dyn FnMut() -> ResourceId,
    resources: Vec<ResourceReference>,
    nodes: usize,
    resident: usize,
}
fn unsupported(message: &str) -> IoError {
    IoError::Unsupported(format!("XCF editable source: {message}"))
}
impl Builder<'_, '_> {
    fn account(&mut self, bytes: usize) -> Result<(), IoError> {
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        let total = self.resident.checked_add(bytes).ok_or_else(|| unsupported("source budget overflow"))?;
        if total > 256 << 20 {
            return Err(unsupported("source resident limit"));
        }
        (self.charge)(bytes)?;
        self.resident = total;
        Ok(())
    }
    fn reserve<T>(&mut self, out: &mut Vec<T>, count: usize) -> Result<(), IoError> {
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        self.account(count.checked_mul(std::mem::size_of::<T>()).ok_or_else(|| unsupported("allocation overflow"))?)?;
        out.try_reserve_exact(count).map_err(|_| unsupported("allocation failed"))
    }
    fn string(&mut self, value: &str) -> Result<String, IoError> {
        if value.len() > 1 << 20 || value.contains('\0') {
            return Err(unsupported("string limit"));
        }
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        self.account(value.len())?;
        let mut out = String::new();
        out.try_reserve_exact(value.len()).map_err(|_| unsupported("string allocation"))?;
        out.push_str(value);
        Ok(out)
    }
    fn bytes(&mut self, value: &[u8]) -> Result<Vec<u8>, IoError> {
        if value.len() > 64 << 20 {
            return Err(unsupported("source byte limit"));
        }
        let mut out = Vec::new();
        self.reserve(&mut out, value.len())?;
        for chunk in value.chunks(128 << 10) {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }
    fn hex(&mut self, value: &[u8]) -> Result<String, IoError> {
        let size = value.len().checked_mul(2).ok_or_else(|| unsupported("hex overflow"))?;
        if size > 1 << 20 {
            return Err(unsupported("configuration byte limit"));
        }
        self.account(size)?;
        let mut out = String::new();
        out.try_reserve_exact(size).map_err(|_| unsupported("hex allocation"))?;
        const HEX: &[u8] = b"0123456789abcdef";
        for (i, byte) in value.iter().enumerate() {
            if i % 4096 == 0 {
                self.ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            let hi = HEX.get(usize::from(*byte >> 4)).copied().ok_or_else(|| unsupported("hex digit"))?;
            let lo = HEX.get(usize::from(*byte & 15)).copied().ok_or_else(|| unsupported("hex digit"))?;
            out.push(char::from(hi));
            out.push(char::from(lo));
        }
        Ok(out)
    }
    fn arg(&mut self, name: &str, value: FilterValue) -> Result<FilterArgument, IoError> {
        self.nodes = self.nodes.checked_add(1).ok_or_else(|| unsupported("configuration count"))?;
        if self.nodes > 100000 {
            return Err(unsupported("configuration count limit"));
        }
        Ok(FilterArgument { name: self.string(name)?, value })
    }
    fn field(&mut self, out: &mut Vec<FilterArgument>, name: &str, value: FilterValue) -> Result<(), IoError> {
        self.reserve(out, 1)?;
        out.push(self.arg(name, value)?);
        Ok(())
    }
    fn resource(&mut self, kind: ResourceKind, name: &str, collection: Option<&str>, internal: bool, locator: Option<&str>) -> Result<ResourceId, IoError> {
        let id = (self.fresh)();
        if id.0 == 0 || self.resources.iter().any(|r| r.id == id) {
            return Err(unsupported("duplicate resource identity"));
        }
        let row = ResourceReference {
            id,
            kind,
            name: self.string(name)?,
            collection: collection.map(|s| self.string(s)).transpose()?,
            internal,
            locator: locator.map(|s| self.string(s)).transpose()?,
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        };
        self.account(std::mem::size_of::<ResourceReference>())?;
        self.resources.try_reserve_exact(1).map_err(|_| unsupported("resource allocation"))?;
        self.resources.push(row);
        Ok(id)
    }
    fn config_nodes(&mut self, nodes: &[ConfigNode], depth: usize) -> Result<Vec<FilterArgument>, IoError> {
        if depth > 24 {
            return Err(unsupported("configuration depth"));
        }
        let mut out = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            let (name, value) = match &node.value {
                ConfigValue::Atom(v) => ("atom", FilterValue::String(self.string(v)?)),
                ConfigValue::Number { literal, bits } => {
                    let mut numeric = Vec::new();
                    let literal = self.string(literal)?;
                    self.field(&mut numeric, "decimal", FilterValue::String(literal))?;
                    self.field(&mut numeric, "f64-bits", FilterValue::Number(ExactNumber::Float64Bits(*bits)))?;
                    ("number", FilterValue::Config(numeric))
                }
                ConfigValue::Quoted(v) => ("quoted-bytes-hex", FilterValue::String(self.hex(v)?)),
                ConfigValue::List(v) => ("list", FilterValue::Config(self.config_nodes(v, depth + 1)?)),
            };
            self.field(&mut out, &format!("{name}-{index}"), value)?;
        }
        Ok(out)
    }
    fn color(&mut self, color: &StoredColor) -> Result<FilterValue, IoError> {
        let mut out = Vec::new();
        let encoding = self.string(&color.encoding)?;
        self.field(&mut out, "encoding", FilterValue::String(encoding))?;
        let samples = self.hex(&color.samples)?;
        self.field(&mut out, "native-samples-hex", FilterValue::String(samples))?;
        if !color.icc.is_empty() {
            if color.icc.len() > 16 << 20 {
                return Err(unsupported("embedded color profile limit"));
            }
            let id = self.resource(ResourceKind::ColorProfile, "embedded vector color profile", None, false, None)?;
            let bytes = self.bytes(&color.icc)?;
            self.account(64)?;
            let digest = hash(&bytes, self.ctl)?;
            let resource = self.resources.iter_mut().find(|r| r.id == id).ok_or_else(|| unsupported("missing embedded profile identity"))?;
            resource.owned_bytes = Some(std::sync::Arc::new(bytes));
            resource.content_hash = Some(digest);
            self.field(&mut out, "profile", FilterValue::Resource(id))?;
        }
        Ok(FilterValue::Config(out))
    }
    fn pattern(&mut self, value: &StoredResource) -> Result<ResourceId, IoError> {
        self.resource(ResourceKind::Pattern, &value.name, Some(&value.collection), value.internal, None)
    }
    fn fill(&mut self, fill: &VectorFill) -> Result<FilterValue, IoError> {
        let mut out = Vec::new();
        self.field(&mut out, "style", uint(fill.style))?;
        self.field(&mut out, "antialias", FilterValue::Bool(fill.antialias))?;
        if let Some(color) = &fill.color {
            let value = self.color(color)?;
            self.field(&mut out, "color", value)?;
        }
        if let Some(pattern) = &fill.pattern {
            let id = self.pattern(pattern)?;
            self.field(&mut out, "pattern", FilterValue::Resource(id))?;
        }
        Ok(FilterValue::Config(out))
    }
}
fn uint(value: u32) -> FilterValue {
    FilterValue::Number(ExactNumber::Unsigned { width: 32, value: u64::from(value) })
}
fn direction(value: Option<&str>) -> Result<i32, IoError> {
    match value {
        Some("ltr") => Ok(0),
        Some("rtl") => Ok(1),
        Some("ttb-rtl") => Ok(2),
        Some("ttb-rtl-upright") => Ok(3),
        Some("ttb-ltr") => Ok(4),
        Some("ttb-ltr-upright") => Ok(5),
        _ => Err(unsupported("missing or unknown text direction")),
    }
}
fn hash(bytes: &[u8], ctl: &Interrupt<'_>) -> Result<String, IoError> {
    let mut hash = blake3::Hasher::new();
    for chunk in bytes.chunks(128 << 10) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        hash.update(chunk);
    }
    let mut out = String::new();
    out.try_reserve_exact(64).map_err(|_| unsupported("hash allocation"))?;
    out.push_str(hash.finalize().to_hex().as_str());
    Ok(out)
}
/// `cache` is transferred, not converted; caller supplies its exact canonical sample/model descriptor.
/// Resource IDs come from the caller's document-wide fresh-ID allocator. No IDs are inferred from tattoos.
pub fn map_source(
    source: &Drawable,
    cache: Option<Plane>,
    context: SourceContext,
    ctl: &Interrupt<'_>,
    fresh: &mut dyn FnMut() -> ResourceId,
    charge: &mut dyn FnMut(usize) -> Result<(), IoError>,
) -> Result<Option<MappedSource>, IoError> {
    map_source_with_storage(source, cache, context, ctl, fresh, charge)
}
/// Metadata-only storage-generic adapter; original cache bytes are supplied separately.
pub fn map_source_with_storage<P>(
    source: &Drawable<P>,
    cache: Option<Plane>,
    context: SourceContext,
    ctl: &Interrupt<'_>,
    fresh: &mut dyn FnMut() -> ResourceId,
    charge: &mut dyn FnMut(usize) -> Result<(), IoError>,
) -> Result<Option<MappedSource>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let count = usize::from(source.text.is_some()) + usize::from(source.vector.is_some()) + usize::from(source.link.is_some());
    if count == 0 {
        return Ok(None);
    }
    if count != 1 {
        return Err(unsupported("conflicting source kinds"));
    }
    if !matches!(context.owner, ItemRef::Layer(_)) {
        return Err(unsupported("source owner is not a layer"));
    }
    if cache.is_none() && (source.text.is_some() && source.text_source_modified() || source.vector.as_ref().is_some_and(|v| v.source_modified)) {
        return Err(unsupported("modified source is missing its authoritative raster cache"));
    }
    let mut b = Builder { ctl, charge, fresh, resources: Vec::new(), nodes: 0, resident: 0 };
    let mut opaque = Vec::new();
    let mut configuration = Vec::new();
    let editable = if let Some(text) = &source.text {
        if !text.unknown_fields.is_empty() || text.font.as_ref().is_some_and(|f| !f.unknown_fields.is_empty()) {
            return Err(unsupported("unknown text/font configuration"));
        }
        let font = text.font.as_ref().ok_or_else(|| unsupported("missing font identity"))?;
        let name = font.name.as_deref().or(font.postscript_name.as_deref()).ok_or_else(|| unsupported("missing font name"))?;
        let font_id = b.resource(ResourceKind::Font, name, None, false, None)?;
        let tree = b.config_nodes(&text.configuration, 0)?;
        b.field(&mut configuration, "gimp-text-configuration", FilterValue::Config(tree))?;
        b.field(&mut configuration, "source-flags", uint(source.text_flags))?;
        b.field(&mut configuration, "parasite-flags", uint(text.parasite_flags))?;
        let mut layout = Vec::new();
        for (name, bits) in [
            ("font-size", text.layout.font_size_bits),
            ("indent", text.layout.indent_bits),
            ("line-spacing", text.layout.line_spacing_bits),
            ("letter-spacing", text.layout.letter_spacing_bits),
            ("box-width", text.layout.box_width_bits),
            ("box-height", text.layout.box_height_bits),
            ("offset-x", text.layout.offset_x_bits),
            ("offset-y", text.layout.offset_y_bits),
        ] {
            if let Some(bits) = bits {
                b.field(&mut layout, name, FilterValue::Number(ExactNumber::Float64Bits(bits)))?;
            }
        }
        for (name, value) in [
            ("font-size-unit", &text.layout.font_size_unit),
            ("language", &text.layout.language),
            ("base-direction", &text.layout.direction),
            ("justification", &text.layout.justification),
            ("box-mode", &text.layout.box_mode),
            ("box-unit", &text.layout.box_unit),
        ] {
            if let Some(value) = value {
                let value = b.string(value)?;
                b.field(&mut layout, name, FilterValue::String(value))?;
            }
        }
        for (name, value) in [("antialias", text.layout.antialias), ("hinting", text.layout.hinting)] {
            if let Some(value) = value {
                b.field(&mut layout, name, FilterValue::Bool(value))?;
            }
        }
        if let Some(matrix) = text.layout.transformation_bits {
            let mut fields = Vec::new();
            for (index, bits) in matrix.into_iter().enumerate() {
                b.field(&mut fields, &format!("coefficient-{index}"), FilterValue::Number(ExactNumber::Float64Bits(bits)))?;
            }
            b.field(&mut layout, "transformation", FilterValue::Config(fields))?;
        }
        b.field(&mut configuration, "layout", FilterValue::Config(layout))?;
        let mut font_fields = Vec::new();
        for (name, value) in
            [("name", &font.name), ("family", &font.family), ("style", &font.style), ("postscript-name", &font.postscript_name), ("requested-hash", &font.hash)]
        {
            if let Some(value) = value {
                let value = b.string(value)?;
                b.field(&mut font_fields, name, FilterValue::String(value))?;
            }
        }
        for (name, value) in [("index", font.index), ("version", font.version)] {
            if let Some(value) = value {
                b.field(&mut font_fields, name, uint(value))?;
            }
        }
        for (name, value) in [("weight", font.weight), ("slant", font.slant), ("width", font.width)] {
            if let Some(value) = value {
                b.field(&mut font_fields, name, FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(value) }))?;
            }
        }
        b.field(&mut configuration, "font-selectors", FilterValue::Config(font_fields))?;
        let mut runs = Vec::new();
        for (index, run) in text.runs.iter().enumerate() {
            let start = usize::try_from(run.start).map_err(|_| unsupported("run offset"))?;
            let end = usize::try_from(run.end).map_err(|_| unsupported("run offset"))?;
            if start > end || end > text.content.len() || !text.content.is_char_boundary(start) || !text.content.is_char_boundary(end) {
                return Err(unsupported("invalid text run"));
            }
            let mut fields = Vec::new();
            b.field(&mut fields, "start", FilterValue::Number(ExactNumber::Unsigned { width: 64, value: run.start }))?;
            b.field(&mut fields, "end", FilterValue::Number(ExactNumber::Unsigned { width: 64, value: run.end }))?;
            let mut styles = Vec::new();
            for (style_index, id) in run.style_ids.iter().enumerate() {
                if *id >= text.styles.len() {
                    return Err(unsupported("dangling text style"));
                }
                b.field(
                    &mut styles,
                    &format!("style-id-{style_index}"),
                    FilterValue::Number(ExactNumber::Unsigned { width: 64, value: u64::try_from(*id).map_err(|_| unsupported("style index"))? }),
                )?;
            }
            b.field(&mut fields, "styles", FilterValue::Config(styles))?;
            b.field(&mut runs, &format!("run-{index}"), FilterValue::Config(fields))?;
        }
        b.field(&mut configuration, "runs", FilterValue::Config(runs))?;
        let mut styles = Vec::new();
        for (index, style) in text.styles.iter().enumerate() {
            let mut fields = Vec::new();
            let tag = b.string(&style.tag)?;
            b.field(&mut fields, "tag", FilterValue::String(tag))?;
            for (key, value) in &style.attributes {
                let value = b.string(value)?;
                b.field(&mut fields, key, FilterValue::String(value))?;
            }
            b.field(&mut styles, &format!("style-{index}"), FilterValue::Config(fields))?;
        }
        b.field(&mut configuration, "styles", FilterValue::Config(styles))?;
        let bytes = b.bytes(&text.raw)?;
        b.account(64)?;
        let digest = hash(&bytes, ctl)?;
        b.reserve(&mut opaque, 1)?;
        opaque.push(OpaqueRecord {
            owner: Some(context.owner),
            namespace: b.string("gimp/xcf/text-parasite")?,
            identity: b.string("gimp-text-layer")?,
            source_version: context.source_version,
            order: 0,
            flags: text.parasite_flags,
            bytes: std::sync::Arc::new(bytes),
            blake3: digest,
            rendering_required: true,
        });
        EditableSource::Text {
            content: b.string(&text.content)?,
            markup: text.markup.as_deref().map(|s| b.string(s)).transpose()?,
            font: font_id,
            direction: direction(text.layout.direction.as_deref())?,
            source_modified: source.text_source_modified(),
            configuration,
            current_origin: None,
        }
    } else if let Some(vector) = &source.vector {
        let path = context.vector_path.ok_or_else(|| unsupported("missing canonical vector path"))?;
        b.field(&mut configuration, "source-path-tattoo", uint(vector.path_tattoo))?;
        b.field(&mut configuration, "filled", FilterValue::Bool(vector.filled))?;
        b.field(&mut configuration, "stroked", FilterValue::Bool(vector.stroked))?;
        let fill = b.fill(&vector.fill)?;
        b.field(&mut configuration, "fill", fill)?;
        let fill = b.fill(&vector.stroke.fill)?;
        let mut stroke = Vec::new();
        b.field(&mut stroke, "fill", fill)?;
        b.field(&mut stroke, "width", FilterValue::Number(ExactNumber::Float32Bits(vector.stroke.width_bits)))?;
        b.field(&mut stroke, "cap-style", uint(vector.stroke.cap_style))?;
        b.field(&mut stroke, "join-style", uint(vector.stroke.join_style))?;
        b.field(&mut stroke, "miter-limit", FilterValue::Number(ExactNumber::Float32Bits(vector.stroke.miter_limit_bits)))?;
        let mut dashes = Vec::new();
        for (index, bits) in vector.stroke.dashes_bits.iter().enumerate() {
            b.field(&mut dashes, &format!("dash-{index}"), FilterValue::Number(ExactNumber::Float32Bits(*bits)))?;
        }
        b.field(&mut stroke, "dashes", FilterValue::Config(dashes))?;
        b.field(&mut configuration, "stroke", FilterValue::Config(stroke))?;
        EditableSource::Vector { path, source_modified: vector.source_modified, configuration }
    } else {
        let link = source.link.as_ref().ok_or_else(|| unsupported("missing link"))?;
        if link.unknown_flags() != 0 {
            return Err(unsupported("unknown link flags"));
        }
        let resource = b.resource(ResourceKind::LinkedImage, &source.name, None, false, Some(&link.path))?;
        b.field(&mut configuration, "flags", uint(link.flags))?;
        b.field(&mut configuration, "source-width", uint(link.width))?;
        b.field(&mut configuration, "source-height", uint(link.height))?;
        EditableSource::Linked { resource, last_verified_hash: None, configuration, current: None }
    };
    for (order, property) in source.raw_properties.iter().enumerate().filter(|(_, p)| matches!(p.tag, 47..=49)) {
        let bytes = b.bytes(&property.payload)?;
        b.account(64)?;
        let digest = hash(&bytes, ctl)?;
        b.reserve(&mut opaque, 1)?;
        opaque.push(OpaqueRecord {
            owner: Some(context.owner),
            namespace: b.string("gimp/xcf/source-property")?,
            identity: b.string(&property.tag.to_string())?,
            source_version: context.source_version,
            order: u32::try_from(order).map_err(|_| unsupported("property order"))?,
            flags: 0,
            bytes: std::sync::Arc::new(bytes),
            blake3: digest,
            rendering_required: true,
        });
    }
    let transform = source
        .transform
        .as_ref()
        .map(|t| {
            if t.interpolation > 4 || !t.matrix.iter().all(|v| v.is_finite()) {
                return Err(unsupported("invalid source transform"));
            }
            Ok(TransformState {
                matrix_f64_bits: t.matrix.map(|v| f64::from(v).to_bits()),
                interpolation: i32::try_from(t.interpolation).map_err(|_| unsupported("interpolation"))?,
                pre_offset: [t.offset.0, t.offset.1],
            })
        })
        .transpose()?;
    let cache = cache
        .map(|plane| {
            let sample = match plane.sample {
                Sample::U8 => SampleType::U8,
                Sample::U16 => SampleType::U16,
                Sample::U32 => SampleType::U32,
                Sample::F32 | Sample::F16 => SampleType::F32,
                Sample::F64 => SampleType::F64,
            };
            let mode = match source.model {
                photocraft_xcf::Model::Rgb => ColorMode::Rgb,
                photocraft_xcf::Model::Gray => ColorMode::Grayscale,
                photocraft_xcf::Model::Indexed => return Err(unsupported("indexed source cache needs explicit expansion")),
            };
            if context.format != PixelFormat::new(mode, sample, source.alpha)
                || context.format.channels() != plane.channels
                || plane.width != source.width
                || plane.height != source.height
            {
                return Err(unsupported("cache descriptor mismatch"));
            }
            let len = SourceCache::checked_descriptor_len(plane.width, plane.height, context.format).map_err(|e| unsupported(&e.to_string()))?;
            if len != plane.bytes.len() {
                return Err(unsupported("cache byte count mismatch"));
            }
            b.account(len.checked_add(std::mem::size_of::<SourceCache>() + 64).ok_or_else(|| unsupported("cache budget overflow"))?)?;
            let digest = hash(&plane.bytes, ctl)?;
            Ok(SourceCache { bytes: std::sync::Arc::new(plane.bytes), blake3: digest, width: plane.width, height: plane.height, format: context.format })
        })
        .transpose()?;
    b.account(std::mem::size_of::<SourceItem>())?;
    Ok(Some(MappedSource { item: SourceItem { owner: context.owner, source: editable, transform, cache }, resources: b.resources, opaque }))
}
