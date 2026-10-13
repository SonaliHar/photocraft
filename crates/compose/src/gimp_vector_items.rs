//! Scoped saved-vector regeneration from canonical geometry and typed configuration.
//! Source-cache samples and resource locators are never used to render.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
use photocraft_doc::xcf_items::{EditableSource, SourceItem};
use photocraft_doc::xcf_resources::ResourceKind;
use photocraft_doc::{ColorMode, Document, PixelFormat, SampleEncoding, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use photocraft_vector::xcf::{self, SolidSettings, SolidStroke, VectorError};

pub struct RegeneratedVector {
    pub surface: Surface,
    pub bounds: Rect,
}
fn validation_error(e: photocraft_doc::XcfValidationError) -> CompositionError {
    if e.code == "cancelled" {
        CompositionError::Raster(RasterInputError::Cancelled)
    } else {
        CompositionError::Invalid("invalid canonical vector source state")
    }
}
fn unsupported() -> CompositionError {
    CompositionError::Unsupported("unqualified saved vector configuration")
}
fn fields<'a>(a: &'a [FilterArgument], names: &[&str]) -> Result<&'a [FilterArgument]> {
    if a.len() != names.len() || names.iter().any(|n| a.iter().filter(|v| v.name == *n).count() != 1) {
        return Err(unsupported());
    }
    Ok(a)
}
fn field<'a>(a: &'a [FilterArgument], name: &str) -> Result<&'a FilterValue> {
    a.iter().find(|v| v.name == name).map(|v| &v.value).ok_or_else(unsupported)
}
fn config(v: &FilterValue) -> Result<&[FilterArgument]> {
    if let FilterValue::Config(a) = v { Ok(a) } else { Err(unsupported()) }
}
fn uint(v: &FilterValue) -> Result<u64> {
    if let FilterValue::Number(ExactNumber::Unsigned { width: 32, value }) = v { Ok(*value) } else { Err(unsupported()) }
}
fn pattern(doc: &Document, v: &FilterValue) -> Result<()> {
    let FilterValue::Resource(id) = v else {
        return Err(unsupported());
    };
    let state = doc.xcf_state.as_ref().ok_or_else(unsupported)?;
    if state.resources.iter().filter(|r| r.id == *id && r.kind == ResourceKind::Pattern).count() != 1 {
        return Err(unsupported());
    }
    Ok(())
}
fn fill(doc: &Document, a: &[FilterArgument]) -> Result<([f64; 4], bool)> {
    fields(a, &["style", "antialias", "color", "pattern"])?;
    if uint(field(a, "style")?)? != 0 {
        return Err(unsupported());
    }
    let FilterValue::Bool(antialias) = field(a, "antialias")? else {
        return Err(unsupported());
    };
    pattern(doc, field(a, "pattern")?)?;
    let c = config(field(a, "color")?)?;
    fields(c, &["encoding", "native-samples-hex"])?;
    if field(c, "encoding")? != &FilterValue::String("R'G'B'A float".into()) {
        return Err(unsupported());
    }
    let FilterValue::String(hex) = field(c, "native-samples-hex")? else {
        return Err(unsupported());
    };
    if hex.len() != 32 || !hex.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err(unsupported());
    }
    let mut bytes = [0u8; 16];
    for (i, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| unsupported())?;
        *bytes.get_mut(i).ok_or_else(unsupported)? = u8::from_str_radix(text, 16).map_err(|_| unsupported())?;
    }
    let mut rgba = [0.; 4];
    for (out, raw) in rgba.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
        *out = f64::from(f32::from_le_bytes(*raw));
        if !out.is_finite() || !(0.0..=1.0).contains(out) {
            return Err(unsupported());
        }
    }
    if rgba.last() != Some(&1.) {
        return Err(unsupported());
    }
    Ok((rgba, *antialias))
}
/// Scoped solid rectangles, binary rectilinear contours and opaque no-AA monotone arches.
/// Caller callbacks retain resident/work charges through use of the output.
pub fn qualify_source(doc: &Document, item: &SourceItem, ctl: &Interrupt<'_>) -> Result<Rect> {
    prepare(doc, item, ctl, &mut |_| Ok(())).map(|(bounds, _, _)| bounds)
}
fn prepare<'a>(
    doc: &'a Document,
    item: &SourceItem,
    ctl: &Interrupt<'_>,
    work: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<(Rect, &'a photocraft_doc::Path, SolidSettings)> {
    ctl.check().map_err(|_| CompositionError::Raster(RasterInputError::Cancelled))?;
    let state = doc.xcf_state.as_ref().ok_or_else(unsupported)?;
    if state.provenance.format != "xcf"
        || state.provenance.format_version != 24
        || !state.source_items.iter().any(|i| i == item)
        || doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::U8
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !doc.source_color_context.as_ref().is_some_and(|c| c.source_precision == 150 && c.original_icc.is_none())
        || item.transform.is_some()
    {
        return Err(unsupported());
    }
    if let Some(icc) = doc.icc_profile.as_ref() {
        work((icc.len() as u64).checked_mul(2).ok_or_else(unsupported)?)?;
        if !matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }) {
            return Err(CompositionError::Unsupported("saved vector active working profile"));
        }
        doc.working_profile_interpretation.validate_with_interrupt(Some(icc), doc.mode, ctl).map_err(validation_error)?;
        photocraft_cms::xcf_color::implicit_source_profile(icc, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::NonlinearProfile)?;
    }
    let retained = state
        .source_items
        .iter()
        .filter_map(|i| i.cache.as_ref())
        .map(|c| c.bytes.len())
        .chain(state.resources.iter().filter_map(|r| r.owned_bytes.as_ref()).map(|bytes| bytes.len()))
        .chain(state.opaque.iter().map(|o| o.bytes.len()))
        .try_fold(0u64, |n, v| n.checked_add(v as u64))
        .ok_or_else(unsupported)?;
    work(retained)?;
    doc.validate_xcf_state_with(ctl).map_err(validation_error)?;
    let cache = item.cache.as_ref().ok_or_else(unsupported)?;
    work(u64::try_from(cache.bytes.len()).map_err(|_| unsupported())?)?;
    cache.validate_with(ctl).map_err(validation_error)?;
    if cache.format != PixelFormat::RGBA8 {
        return Err(unsupported());
    }
    let EditableSource::Vector { path, source_modified: false, configuration } = &item.source else {
        return Err(unsupported());
    };
    fields(configuration, &["source-path-tattoo", "filled", "stroked", "fill", "stroke"])?;
    uint(field(configuration, "source-path-tattoo")?)?;
    let (FilterValue::Bool(filled), FilterValue::Bool(stroked)) = (field(configuration, "filled")?, field(configuration, "stroked")?) else {
        return Err(unsupported());
    };
    let fill_state = fill(doc, config(field(configuration, "fill")?)?)?;
    let stroke = config(field(configuration, "stroke")?)?;
    fields(stroke, &["fill", "width", "cap-style", "join-style", "miter-limit", "dashes"])?;
    let stroke_fill = fill(doc, config(field(stroke, "fill")?)?)?;
    let join = uint(field(stroke, "join-style")?)?;
    let shaped_join = matches!(join, 1 | 2) && !*filled && *stroked && !stroke_fill.1;
    let cap = uint(field(stroke, "cap-style")?)?;
    let extended_cap = matches!(cap, 1 | 2) && !*filled && *stroked && !stroke_fill.1 && join == 0;
    if (cap != 0 && !extended_cap)
        || (join != 0 && !shaped_join)
        || field(stroke, "miter-limit")? != &FilterValue::Number(ExactNumber::Float32Bits(10f32.to_bits()))
        || !config(field(stroke, "dashes")?)?.is_empty()
    {
        return Err(unsupported());
    }
    let settings = if *filled && !*stroked {
        if stroke_fill != ([0., 0., 0., 1.], true) || field(stroke, "width")? != &FilterValue::Number(ExactNumber::Float32Bits(6f32.to_bits())) {
            return Err(unsupported());
        }
        SolidSettings { fill: Some(fill_state.0), stroke: None, antialias: fill_state.1 }
    } else if !*filled && *stroked {
        let FilterValue::Number(ExactNumber::Float32Bits(width)) = field(stroke, "width")? else { return Err(unsupported()) };
        let admitted_width = if stroke_fill.1 {
            [1f32.to_bits(), 2f32.to_bits(), 3f32.to_bits(), 4f32.to_bits(), 5f32.to_bits(), 7f32.to_bits()].contains(width)
        } else {
            [6f32.to_bits(), 8f32.to_bits()].contains(width)
                || (join == 0 && [7f32.to_bits(), 9f32.to_bits()].contains(width))
                || (matches!(cap, 0..=2) && join == 0 && [3f32.to_bits(), 5f32.to_bits()].contains(width))
        };
        if fill_state != ([1., 1., 1., 1.], true) || !admitted_width {
            return Err(unsupported());
        }
        SolidSettings {
            fill: None,
            stroke: Some(SolidStroke {
                color: stroke_fill.0,
                style: photocraft_vector::StrokeStyle {
                    width: f64::from(f32::from_bits(*width)),
                    cap: match cap {
                        1 => photocraft_doc::LineCap::Round,
                        2 => photocraft_doc::LineCap::Square,
                        _ => photocraft_doc::LineCap::Butt,
                    },
                    join: match join {
                        1 => photocraft_doc::LineJoin::Round,
                        2 => photocraft_doc::LineJoin::Bevel,
                        _ => photocraft_doc::LineJoin::Miter,
                    },
                    miter_limit: 10.,
                    dashes: Vec::new(),
                    dash_offset: 0.,
                },
            }),
            antialias: stroke_fill.1,
        }
    } else {
        return Err(unsupported());
    };
    let mut paths = doc.paths.iter().filter(|p| p.id == *path);
    let path = paths.next().ok_or_else(unsupported)?;
    if paths.next().is_some() {
        return Err(unsupported());
    }
    // Odd Butt widths have independent open-axis witnesses only. The lower-layer
    // ideal rectangle primitive is broader and cannot establish saved-source admission.
    if let Some(stroke) = &settings.stroke
        && !settings.antialias
        && stroke.style.cap == photocraft_doc::LineCap::Butt
        && [3., 5.].contains(&stroke.style.width)
        && path.path.subpaths.first().is_none_or(|s| s.closed)
    {
        return Err(unsupported());
    }
    let bounds = if let Some(stroke) = &settings.stroke
        && !settings.antialias
        && path.path.subpaths.first().is_some_and(|s| !s.closed)
    {
        xcf::open_axis_stroke_descriptor(&path.path, &stroke.style).map_err(vector_error)?
    } else if let Some(stroke) = &settings.stroke
        && settings.antialias
    {
        xcf::antialiased_rectangle_stroke_descriptor(&path.path, &stroke.style).map_err(vector_error)?
    } else if let Some(stroke) = &settings.stroke
        && [7., 9.].contains(&stroke.style.width)
    {
        xcf::aliased_rectangle_stroke_storage_descriptor(&path.path, &stroke.style).map_err(vector_error)?
    } else if settings.fill.is_some() && !settings.antialias && path.path.subpaths.first().is_some_and(|s| s.knots.len() == 2) {
        if settings.fill.is_none_or(|fill| fill[3] != 1.) {
            return Err(unsupported());
        }
        xcf::monotone_arch_descriptor(&path.path).map_err(vector_error)?
    } else {
        let [x0, y0, x1, y1] = if let Some(stroke) = &settings.stroke {
            xcf::aliased_rectangle_stroke_descriptor(&path.path, &stroke.style).map_err(vector_error)?
        } else if settings.antialias {
            xcf::rectangle_descriptor(&path.path).map_err(vector_error)?.original_bounds
        } else {
            xcf::polygon_descriptor_with_rule(&path.path, xcf::AliasedFillRule::EvenOdd).map_err(vector_error)?
        };
        Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32)
    };
    Ok((bounds, &path.path, settings))
}
pub fn render_source(
    doc: &Document,
    item: &SourceItem,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<()>,
    work: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<RegeneratedVector> {
    let (bounds, path, settings) = prepare(doc, item, ctl, work)?;
    let mut resident_error = None;
    let mut work_error = None;
    let output = xcf::render_solid_with_rule(
        path,
        &settings,
        bounds,
        PixelFormat::RGBA8,
        ctl,
        &mut |n| {
            charge(n).map_err(|e| {
                resident_error = Some(e);
                VectorError::Budget
            })
        },
        &mut |n| {
            work(n).map_err(|e| {
                work_error = Some(e);
                VectorError::Budget
            })
        },
        if settings.antialias { xcf::AliasedFillRule::NonZero } else { xcf::AliasedFillRule::EvenOdd },
    );
    if let Some(e) = resident_error.or(work_error) {
        return Err(e);
    }
    Ok(RegeneratedVector { surface: output.map_err(vector_error)?, bounds })
}
fn vector_error(e: VectorError) -> CompositionError {
    match e {
        VectorError::Cancelled => CompositionError::Raster(RasterInputError::Cancelled),
        VectorError::Unsupported(s) => CompositionError::Unsupported(s),
        VectorError::Budget => CompositionError::Invalid("vector regeneration budget denied"),
        VectorError::Allocation => CompositionError::Invalid("vector regeneration allocation denied"),
    }
}
