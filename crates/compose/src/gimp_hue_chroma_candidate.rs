//! Explicit typed LCh Float ROI stage, not a RGB/Lab Surface or source renderer.
//! RGB/ICC ingress and egress are caller-owned conversion boundaries. Whole-RGB
//! and separated routes differ in retained capsules; neither is inferred here.
use photocraft_algo::gimp::hue_chroma::{self, LchFloatContext, Limits, Parameters};
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
pub const SCHEMA_326_320: &str = "0c253517c76eee044fd1eac873ef011d3c62251e5f13bb7d1cf5c00e94e592f1";
pub const SCHEMA_308: &str = "4b07eb7a89f9770c342afef3906f4ee4c77e72e71ca957e33a4c3971280d40ac";
#[derive(Clone, Copy, Debug)]
pub struct SourceBinding<'a> {
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub context: LchFloatContext,
}
/// Typed native Float words, explicitly L,C,H,A. No Surface format alias exists.
#[derive(Clone, Copy)]
pub struct LchFloatView<'a> {
    pub extent: Rect,
    pub pixels: &'a [[f32; 4]],
    pub context: LchFloatContext,
}
pub struct LchFloatWindow {
    pub roi: Rect,
    pub pixels: Vec<[f32; 4]>,
    pub context: LchFloatContext,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub borrowed_source_bytes: u64,
    pub private_output_bytes: u64,
    pub peak_bytes: u64,
    pub work_units: u64,
}
pub struct BoundSource<'a> {
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
    parameters: Parameters,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
/// Schema identities describe observed typed parameters, not release-wide render
/// equivalence. No composition, masks, opacity, region or space tuple is applied.
pub fn bind_source_node<'a>(node: &'a FilterNode, binding: SourceBinding<'a>, c: &Interrupt<'_>) -> Result<BoundSource<'a>, E> {
    poll(c)?;
    let epoch = match binding.schema_sha256 {
        SCHEMA_326_320 => matches!(binding.expected_epoch, "gimp-3.2.6-windows-0c253517c76eee04" | "gimp-3.2.0-linux-x86_64-appimage-schema-0c253517c76eee04"),
        SCHEMA_308 => binding.expected_epoch == "gimp-3.0.8-linux-x86_64-appimage1-schema-4b07eb7a89f9770c",
        _ => false,
    };
    if !epoch
        || node.operation.name != "gegl:hue-chroma"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != binding.expected_epoch
        || node.arguments.len() != 3
    {
        return Err(E::UnsupportedConversion);
    }
    let mut values = [None; 3];
    for argument in &node.arguments {
        poll(c)?;
        let index = match argument.name.as_str() {
            "hue" => 0,
            "chroma" => 1,
            "lightness" => 2,
            _ => return Err(E::UnsupportedConversion),
        };
        let slot = values.get_mut(index).ok_or(E::LengthMismatch)?;
        if slot.is_some() {
            return Err(E::UnsupportedConversion);
        }
        let value = match &argument.value {
            FilterValue::Number(ExactNumber::Float32Bits(v)) => f64::from(f32::from_bits(*v)),
            FilterValue::Number(ExactNumber::Float64Bits(v)) => f64::from_bits(*v),
            _ => return Err(E::UnsupportedConversion),
        };
        *slot = Some(value);
    }
    let parameters = Parameters {
        hue: values.first().copied().flatten().ok_or(E::UnsupportedConversion)?,
        chroma: values.get(1).copied().flatten().ok_or(E::UnsupportedConversion)?,
        lightness: values.get(2).copied().flatten().ok_or(E::UnsupportedConversion)?,
    };
    hue_chroma::validate(parameters)?;
    poll(c)?;
    Ok(BoundSource { node, binding, parameters })
}
fn count(rect: Rect) -> Result<usize, E> {
    if rect.x1 < rect.x0 || rect.y1 < rect.y0 {
        return Err(E::InvalidBounds);
    }
    let w = usize::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| E::LimitExceeded)?;
    let h = usize::try_from(i64::from(rect.y1) - i64::from(rect.y0)).map_err(|_| E::LimitExceeded)?;
    w.checked_mul(h).ok_or(E::LimitExceeded)
}
impl<'a> BoundSource<'a> {
    pub fn original_node(&self) -> &'a FilterNode {
        self.node
    }
    pub fn parameters(&self) -> Parameters {
        self.parameters
    }
    pub fn requirements(&self, source: LchFloatView<'_>, roi: Rect, limits: Limits) -> Result<Requirements, E> {
        if source.context != self.binding.context {
            return Err(E::UnsupportedConversion);
        }
        if !roi.is_empty() && roi.intersect(&source.extent) != roi {
            return Err(E::InvalidBounds);
        }
        let input = count(source.extent)?;
        let output = count(roi)?;
        if input != source.pixels.len() {
            return Err(E::LengthMismatch);
        }
        let borrowed_source_bytes = (input as u64).checked_mul(16).ok_or(E::LimitExceeded)?;
        let lower = hue_chroma::requirements(output, limits)?;
        let peak_bytes = borrowed_source_bytes.checked_add(lower.output_bytes).and_then(|n| n.checked_add(1024)).ok_or(E::LimitExceeded)?;
        if peak_bytes > limits.resident_bytes || peak_bytes > MAX_CHECKED_RASTER_BYTES {
            return Err(E::LimitExceeded);
        }
        Ok(Requirements { borrowed_source_bytes, private_output_bytes: lower.output_bytes, peak_bytes, work_units: lower.work_units })
    }
    /// Private output only; cancellation never publishes an incomplete window.
    pub fn render_roi(&self, source: LchFloatView<'_>, roi: Rect, limits: Limits, c: &Interrupt<'_>) -> Result<LchFloatWindow, E> {
        poll(c)?;
        let req = self.requirements(source, roi, limits)?;
        let length = usize::try_from(req.private_output_bytes / 16).map_err(|_| E::LimitExceeded)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(length).map_err(|_| E::AllocationFailed)?;
        if pixels.capacity() != length {
            return Err(E::LimitExceeded);
        }
        if length == 0 {
            poll(c)?;
            return Ok(LchFloatWindow { roi, pixels, context: self.binding.context });
        }
        let width = usize::try_from(i64::from(source.extent.x1) - i64::from(source.extent.x0)).map_err(|_| E::InvalidBounds)?;
        for y in roi.y0..roi.y1 {
            poll(c)?;
            for x in roi.x0..roi.x1 {
                poll(c)?;
                let row = usize::try_from(i64::from(y) - i64::from(source.extent.y0)).map_err(|_| E::InvalidBounds)?;
                let col = usize::try_from(i64::from(x) - i64::from(source.extent.x0)).map_err(|_| E::InvalidBounds)?;
                let index = row.checked_mul(width).and_then(|n| n.checked_add(col)).ok_or(E::LimitExceeded)?;
                let input = *source.pixels.get(index).ok_or(E::LengthMismatch)?;
                pixels.push(hue_chroma::evaluate_lch_float(input, self.parameters)?);
            }
        }
        if pixels.len() != length {
            return Err(E::LengthMismatch);
        }
        poll(c)?;
        Ok(LchFloatWindow { roi, pixels, context: self.binding.context })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::xcf_filters::{FilterArgument, OperationIdentity};
    fn context() -> LchFloatContext {
        LchFloatContext { profile_identity: [7; 32], space_identity: [9; 32] }
    }
    fn binding() -> SourceBinding<'static> {
        SourceBinding { expected_epoch: "gimp-3.2.0-linux-x86_64-appimage-schema-0c253517c76eee04", schema_sha256: SCHEMA_326_320, context: context() }
    }
    fn node() -> FilterNode {
        FilterNode {
            id: photocraft_doc::item_identity::FilterId(1),
            operation: OperationIdentity { name: "gegl:hue-chroma".into(), stored_version: "0:0".into(), schema_epoch: binding().expected_epoch.into() },
            source_display_name: None,
            source_icon: None,
            arguments: [("hue", 37.5f64), ("chroma", -12.5), ("lightness", 8.25)]
                .into_iter()
                .map(|(name, v)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(v.to_bits())) })
                .collect(),
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }
    }
    #[test]
    fn roi_binding_preserves_alpha_and_original_source_configuration() {
        let n = node();
        let before = n.clone();
        let bound = bind_source_node(&n, binding(), &Interrupt::NONE).unwrap();
        let pixels = [[50.0, 40.0, 320.0, -0.0]; 6];
        let src = LchFloatView { extent: Rect::new(-2, 7, 1, 9), pixels: &pixels, context: context() };
        let result = bound.render_roi(src, Rect::new(-1, 7, 1, 9), Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(result.pixels.len(), 4);
        for p in result.pixels {
            assert_eq!(p[..3], [58.25, 27.5, 357.5]);
            assert_eq!(p[3].to_bits(), (-0.0f32).to_bits());
        }
        assert_eq!(n, before);
        assert!(std::ptr::eq(bound.original_node(), &n));
        let mut wrong = src;
        wrong.context.space_identity = [1; 32];
        assert!(bound.render_roi(wrong, src.extent, Limits::default(), &Interrupt::NONE).is_err());
        assert!(matches!(bound.render_roi(src, src.extent, Limits::default(), &Interrupt::cancel_only(&|| true)), Err(E::Cancelled)));
        assert!(bound.render_roi(src, src.extent, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE).is_err());
    }
    #[test]
    fn schema_version_argument_shape_and_duplicate_refuse() {
        let mut n = node();
        n.operation.stored_version = "other".into();
        assert!(bind_source_node(&n, binding(), &Interrupt::NONE).is_err());
        n = node();
        n.arguments[1].name = "hue".into();
        assert!(bind_source_node(&n, binding(), &Interrupt::NONE).is_err());
        n = node();
        n.arguments[0].value = FilterValue::Number(ExactNumber::Unsigned { width: 64, value: 0 });
        assert!(bind_source_node(&n, binding(), &Interrupt::NONE).is_err());
    }
}
