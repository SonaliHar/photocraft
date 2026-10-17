//! Camera Raw Filter › Save Settings… / Load Settings… (#4274): Adobe Camera Raw settings files
//! (`.xmp`), the `crs:` namespace (`http://ns.adobe.com/camera-raw-settings/1.0/`) on an
//! `x:xmpmeta/rdf:RDF/rdf:Description`, as attributes or as child elements.
//!
//! Loading applies only the settings the file holds, over the current ones, as Camera Raw's
//! Load Settings does; settings PhotoCraft has no control for (lens profiles, masks, looks, the
//! B&W Mix, …) are ignored and named in the report. Saving writes every supported control, so
//! loading the file in Photoshop reproduces the dialog, plus the full settings in PhotoCraft's
//! own namespace ([`PHOTOCRAFT_NS`]) so PhotoCraft reads its own files back exactly.
//!
//! The same files serve as a raw file's sidecar (`<raw stem>.xmp`, [`sidecar_path`]), as
//! Camera Raw and Lightroom write them; [`save_sidecar`] updates an existing one in place.
//!
//! Deliberately not mapped: the B&W Mix (`GrayMixer*`: Camera Raw starts from an automatic
//! mix, PhotoCraft's bands from zero, so the numbers don't mean the same), Guided Upright's
//! guides and Camera Raw's precomputed `UprightTransform_*` (PhotoCraft's own record keeps the
//! guides), and the process version (always written as Camera Raw's current `15.4`).
//!
//! White balance: a Camera Raw Filter on a rendered image uses the relative
//! `IncrementalTemperature` / `IncrementalTint` (−100..=100), which are PhotoCraft's Temperature
//! and Tint. The absolute `Temperature` (kelvin) / `Tint` of raw-file settings have no
//! counterpart and are ignored.

use photocraft_algo::camera_raw::{CameraRaw, UPRIGHT_MODES, validate_curve};

/// The Camera Raw settings namespace.
pub const CRS_NS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// Largest settings file read; real ones are a few kilobytes.
pub const MAX_XMP_BYTES: usize = 4 * 1024 * 1024;

type Field = fn(&mut CameraRaw) -> &mut f32;

/// `crs:` scalar name, PhotoCraft field, and the slider range values are clamped to.
const SCALARS: &[(&str, Field, f32, f32)] = &[
    ("IncrementalTemperature", |p| &mut p.temperature, -100.0, 100.0),
    ("IncrementalTint", |p| &mut p.tint, -100.0, 100.0),
    ("Exposure2012", |p| &mut p.exposure, -5.0, 5.0),
    ("Contrast2012", |p| &mut p.contrast, -100.0, 100.0),
    ("Highlights2012", |p| &mut p.highlights, -100.0, 100.0),
    ("Shadows2012", |p| &mut p.shadows, -100.0, 100.0),
    ("Whites2012", |p| &mut p.whites, -100.0, 100.0),
    ("Blacks2012", |p| &mut p.blacks, -100.0, 100.0),
    ("Texture", |p| &mut p.texture, -100.0, 100.0),
    ("Clarity2012", |p| &mut p.clarity, -100.0, 100.0),
    ("Dehaze", |p| &mut p.dehaze, -100.0, 100.0),
    ("Vibrance", |p| &mut p.vibrance, -100.0, 100.0),
    ("Saturation", |p| &mut p.saturation, -100.0, 100.0),
    ("ParametricShadows", |p| &mut p.curve_shadows, -100.0, 100.0),
    ("ParametricDarks", |p| &mut p.curve_darks, -100.0, 100.0),
    ("ParametricLights", |p| &mut p.curve_lights, -100.0, 100.0),
    ("ParametricHighlights", |p| &mut p.curve_highlights, -100.0, 100.0),
    ("ParametricShadowSplit", |p| &mut p.curve_splits[0], 0.0, 100.0),
    ("ParametricMidtoneSplit", |p| &mut p.curve_splits[1], 0.0, 100.0),
    ("ParametricHighlightSplit", |p| &mut p.curve_splits[2], 0.0, 100.0),
    ("HueAdjustmentRed", |p| &mut p.hsl_hue[0], -100.0, 100.0),
    ("HueAdjustmentOrange", |p| &mut p.hsl_hue[1], -100.0, 100.0),
    ("HueAdjustmentYellow", |p| &mut p.hsl_hue[2], -100.0, 100.0),
    ("HueAdjustmentGreen", |p| &mut p.hsl_hue[3], -100.0, 100.0),
    ("HueAdjustmentAqua", |p| &mut p.hsl_hue[4], -100.0, 100.0),
    ("HueAdjustmentBlue", |p| &mut p.hsl_hue[5], -100.0, 100.0),
    ("HueAdjustmentPurple", |p| &mut p.hsl_hue[6], -100.0, 100.0),
    ("HueAdjustmentMagenta", |p| &mut p.hsl_hue[7], -100.0, 100.0),
    ("SaturationAdjustmentRed", |p| &mut p.hsl_sat[0], -100.0, 100.0),
    ("SaturationAdjustmentOrange", |p| &mut p.hsl_sat[1], -100.0, 100.0),
    ("SaturationAdjustmentYellow", |p| &mut p.hsl_sat[2], -100.0, 100.0),
    ("SaturationAdjustmentGreen", |p| &mut p.hsl_sat[3], -100.0, 100.0),
    ("SaturationAdjustmentAqua", |p| &mut p.hsl_sat[4], -100.0, 100.0),
    ("SaturationAdjustmentBlue", |p| &mut p.hsl_sat[5], -100.0, 100.0),
    ("SaturationAdjustmentPurple", |p| &mut p.hsl_sat[6], -100.0, 100.0),
    ("SaturationAdjustmentMagenta", |p| &mut p.hsl_sat[7], -100.0, 100.0),
    ("LuminanceAdjustmentRed", |p| &mut p.hsl_lum[0], -100.0, 100.0),
    ("LuminanceAdjustmentOrange", |p| &mut p.hsl_lum[1], -100.0, 100.0),
    ("LuminanceAdjustmentYellow", |p| &mut p.hsl_lum[2], -100.0, 100.0),
    ("LuminanceAdjustmentGreen", |p| &mut p.hsl_lum[3], -100.0, 100.0),
    ("LuminanceAdjustmentAqua", |p| &mut p.hsl_lum[4], -100.0, 100.0),
    ("LuminanceAdjustmentBlue", |p| &mut p.hsl_lum[5], -100.0, 100.0),
    ("LuminanceAdjustmentPurple", |p| &mut p.hsl_lum[6], -100.0, 100.0),
    ("LuminanceAdjustmentMagenta", |p| &mut p.hsl_lum[7], -100.0, 100.0),
    // Color Grading keeps the split-toning names for the shadow and highlight wheels' hue and
    // saturation, and the balance.
    ("SplitToningShadowHue", |p| &mut p.grade_shadows.hue, 0.0, 360.0),
    ("SplitToningShadowSaturation", |p| &mut p.grade_shadows.sat, 0.0, 100.0),
    ("ColorGradeShadowLum", |p| &mut p.grade_shadows.lum, -100.0, 100.0),
    ("ColorGradeMidtoneHue", |p| &mut p.grade_midtones.hue, 0.0, 360.0),
    ("ColorGradeMidtoneSat", |p| &mut p.grade_midtones.sat, 0.0, 100.0),
    ("ColorGradeMidtoneLum", |p| &mut p.grade_midtones.lum, -100.0, 100.0),
    ("SplitToningHighlightHue", |p| &mut p.grade_highlights.hue, 0.0, 360.0),
    ("SplitToningHighlightSaturation", |p| &mut p.grade_highlights.sat, 0.0, 100.0),
    ("ColorGradeHighlightLum", |p| &mut p.grade_highlights.lum, -100.0, 100.0),
    ("ColorGradeGlobalHue", |p| &mut p.grade_global.hue, 0.0, 360.0),
    ("ColorGradeGlobalSat", |p| &mut p.grade_global.sat, 0.0, 100.0),
    ("ColorGradeGlobalLum", |p| &mut p.grade_global.lum, -100.0, 100.0),
    ("ColorGradeBlending", |p| &mut p.grade_blending, 0.0, 100.0),
    ("SplitToningBalance", |p| &mut p.grade_balance, -100.0, 100.0),
    ("Sharpness", |p| &mut p.sharpen_amount, 0.0, 150.0),
    ("SharpenRadius", |p| &mut p.sharpen_radius, 0.5, 3.0),
    ("SharpenDetail", |p| &mut p.sharpen_detail, 0.0, 100.0),
    ("SharpenEdgeMasking", |p| &mut p.sharpen_masking, 0.0, 100.0),
    ("LuminanceSmoothing", |p| &mut p.noise_luminance, 0.0, 100.0),
    ("LuminanceNoiseReductionDetail", |p| &mut p.noise_luminance_detail, 0.0, 100.0),
    ("ColorNoiseReduction", |p| &mut p.noise_color, 0.0, 100.0),
    ("ColorNoiseReductionDetail", |p| &mut p.noise_color_detail, 0.0, 100.0),
    ("GrainAmount", |p| &mut p.grain_amount, 0.0, 100.0),
    ("GrainSize", |p| &mut p.grain_size, 0.0, 100.0),
    ("GrainFrequency", |p| &mut p.grain_roughness, 0.0, 100.0),
    ("PostCropVignetteAmount", |p| &mut p.vignette_amount, -100.0, 100.0),
    ("PostCropVignetteMidpoint", |p| &mut p.vignette_midpoint, 0.0, 100.0),
    ("PostCropVignetteRoundness", |p| &mut p.vignette_roundness, -100.0, 100.0),
    ("PostCropVignetteFeather", |p| &mut p.vignette_feather, 0.0, 100.0),
    ("PostCropVignetteHighlightContrast", |p| &mut p.vignette_highlights, 0.0, 100.0),
    ("LuminanceNoiseReductionContrast", |p| &mut p.noise_luminance_contrast, 0.0, 100.0),
    ("ColorNoiseReductionSmoothness", |p| &mut p.noise_color_smoothness, 0.0, 100.0),
    ("CurveRefineSaturation", |p| &mut p.curve_refine_saturation, 0.0, 100.0),
    // Optics: Camera Raw's ranges, defaults and signs (positive Distortion corrects barrel,
    // positive Vignette brightens the corners).
    ("LensManualDistortionAmount", |p| &mut p.lens_distortion, -100.0, 100.0),
    ("VignetteAmount", |p| &mut p.lens_vignette, -100.0, 100.0),
    ("VignetteMidpoint", |p| &mut p.lens_vignette_midpoint, 0.0, 100.0),
    ("DefringePurpleAmount", |p| &mut p.defringe_purple_amount, 0.0, 20.0),
    ("DefringePurpleHueLo", |p| &mut p.defringe_purple_hue[0], 0.0, 100.0),
    ("DefringePurpleHueHi", |p| &mut p.defringe_purple_hue[1], 0.0, 100.0),
    ("DefringeGreenAmount", |p| &mut p.defringe_green_amount, 0.0, 20.0),
    ("DefringeGreenHueLo", |p| &mut p.defringe_green_hue[0], 0.0, 100.0),
    ("DefringeGreenHueHi", |p| &mut p.defringe_green_hue[1], 0.0, 100.0),
    // Geometry › Manual Transformations.
    ("PerspectiveVertical", |p| &mut p.transform_vertical, -100.0, 100.0),
    ("PerspectiveHorizontal", |p| &mut p.transform_horizontal, -100.0, 100.0),
    ("PerspectiveRotate", |p| &mut p.transform_rotate, -10.0, 10.0),
    ("PerspectiveScale", |p| &mut p.transform_scale, 50.0, 150.0),
    ("PerspectiveAspect", |p| &mut p.transform_aspect, -100.0, 100.0),
    ("PerspectiveX", |p| &mut p.transform_offset_x, -100.0, 100.0),
    ("PerspectiveY", |p| &mut p.transform_offset_y, -100.0, 100.0),
    // Calibration.
    ("ShadowTint", |p| &mut p.calibration_shadows_tint, -100.0, 100.0),
    ("RedHue", |p| &mut p.calibration_red_hue, -100.0, 100.0),
    ("RedSaturation", |p| &mut p.calibration_red_sat, -100.0, 100.0),
    ("GreenHue", |p| &mut p.calibration_green_hue, -100.0, 100.0),
    ("GreenSaturation", |p| &mut p.calibration_green_sat, -100.0, 100.0),
    ("BlueHue", |p| &mut p.calibration_blue_hue, -100.0, 100.0),
    ("BlueSaturation", |p| &mut p.calibration_blue_sat, -100.0, 100.0),
];

type Curve = fn(&mut CameraRaw) -> &mut Vec<[f32; 2]>;
const CURVES: &[(&str, Curve)] = &[
    ("ToneCurvePV2012", |p| &mut p.point_curve),
    ("ToneCurvePV2012Red", |p| &mut p.point_curve_red),
    ("ToneCurvePV2012Green", |p| &mut p.point_curve_green),
    ("ToneCurvePV2012Blue", |p| &mut p.point_curve_blue),
];

const STYLES: &[(&str, &str)] = &[("1", "highlightPriority"), ("2", "colorPriority"), ("3", "paintOverlay")];

/// Bookkeeping fields Camera Raw writes that carry no setting.
const METADATA: &[&str] = &[
    "Version",
    "ProcessVersion",
    "HasSettings",
    "HasCrop",
    "WhiteBalance",
    "ToneCurveName2012",
    "CameraProfile",
    "PresetType",
    "UUID",
    "SupportsAmount",
    "SupportsColor",
    "SupportsMonochrome",
    "SupportsHighDynamicRange",
    "SupportsNormalDynamicRange",
    "SupportsSceneReferred",
    "SupportsOutputReferred",
    "CameraModelRestriction",
    "Copyright",
    "ContactInfo",
    "Name",
    "ShortName",
    "SortName",
    "Group",
    "Description",
];

/// What [`load`] applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadReport {
    /// How many settings were applied (PhotoCraft's own record, when used, counts as one).
    pub applied: usize,
    /// `crs:` settings without a PhotoCraft control (set to something other than their neutral
    /// value), or with an unusable value, in file order.
    pub ignored: Vec<String>,
}

fn number(s: &str) -> Option<f32> {
    let s = s.trim();
    let v: f32 = s.strip_prefix('+').unwrap_or(s).parse().ok()?;
    v.is_finite().then_some(v)
}

/// `"0, 0"` items of an `rdf:Seq` → curve points.
fn curve(node: roxmltree::Node<'_, '_>) -> Option<Vec<[f32; 2]>> {
    let seq = node.children().find(|c| c.is_element() && c.tag_name().namespace() == Some(RDF_NS) && c.tag_name().name() == "Seq")?;
    let mut points = Vec::new();
    for li in seq.children().filter(|c| c.is_element() && c.tag_name().name() == "li") {
        let text = li.text().unwrap_or("");
        let (x, y) = text.split_once(',')?;
        points.push([number(x)?, number(y)?]);
    }
    validate_curve(&points).ok()?;
    // Camera Raw's linear curve is the identity's two end points; PhotoCraft's is empty.
    if points == [[0.0, 0.0], [255.0, 255.0]] {
        points.clear();
    }
    Some(points)
}

/// An unsupported setting at its neutral value changes nothing, so it isn't reported: zero,
/// `False`, and the few neutral values that aren't zero.
fn neutral(name: &str, value: &str) -> bool {
    let v = value.trim();
    if v.eq_ignore_ascii_case("false") || number(v) == Some(0.0) {
        return true;
    }
    match name {
        "GrainSeed" => true,
        "VignetteMidpoint" => number(v) == Some(50.0),
        "PerspectiveScale" => number(v) == Some(100.0),
        _ => name.starts_with("Defringe") && name.contains("Hue"),
    }
}

fn apply(params: &mut CameraRaw, report: &mut LoadReport, name: &str, value: &str) {
    let mut applied = |ok: bool| if ok { report.applied += 1 } else { report.ignored.push(name.to_string()) };
    if let Some((_, field, lo, hi)) = SCALARS.iter().find(|(n, ..)| *n == name) {
        let v = number(value);
        if let Some(v) = v {
            *field(params) = v.clamp(*lo, *hi);
        }
        applied(v.is_some());
    } else if name == "PostCropVignetteStyle" {
        let style = STYLES.iter().find(|(n, _)| *n == value.trim());
        if let Some((_, style)) = style {
            params.vignette_style = (*style).into();
        }
        applied(style.is_some());
    } else if name == "PerspectiveUpright" {
        // 0 Off, 1 Auto, 2 Level, 3 Vertical, 4 Full, 5 Guided: the order of `UPRIGHT_MODES`.
        let mode = value.trim().parse::<usize>().ok().and_then(|i| UPRIGHT_MODES.get(i));
        if let Some(mode) = mode {
            params.upright = (*mode).into();
        }
        applied(mode.is_some());
    } else if name == "ConvertToGrayscale" {
        let v = value.trim();
        let gray = if v.eq_ignore_ascii_case("true") { Some(true) } else { v.eq_ignore_ascii_case("false").then_some(false) };
        if let Some(gray) = gray {
            params.monochrome = gray;
        }
        applied(gray.is_some());
    } else if !METADATA.contains(&name) && !neutral(name, value) {
        report.ignored.push(name.to_string());
    }
}

fn is_rdf(n: &roxmltree::Node<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(RDF_NS) && n.tag_name().name() == name
}

/// The settings `rdf:Description`s: the direct children of the (outermost) `rdf:RDF`. Nested
/// descriptions (a Look's or a mask's parameters) hold settings of their own, not the image's.
fn top_descriptions<'a, 'i>(doc: &'a roxmltree::Document<'i>) -> Vec<roxmltree::Node<'a, 'i>> {
    let Some(rdf) = doc.descendants().find(|n| is_rdf(n, "RDF")) else {
        return Vec::new();
    };
    rdf.children().filter(|n| is_rdf(n, "Description")).collect()
}

/// One setting of a description: an attribute's value or a child element.
enum Setting<'a, 'i> {
    Attr(&'a str),
    Elem(roxmltree::Node<'a, 'i>),
}

/// The `crs:` settings of the top-level descriptions, in file order.
fn crs_settings<'a, 'i>(descs: &[roxmltree::Node<'a, 'i>]) -> Vec<(&'a str, Setting<'a, 'i>)> {
    let mut out = Vec::new();
    for desc in descs {
        for a in desc.attributes().filter(|a| a.namespace() == Some(CRS_NS)) {
            out.push((a.name(), Setting::Attr(a.value())));
        }
        for c in desc.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(CRS_NS)) {
            out.push((c.tag_name().name(), Setting::Elem(c)));
        }
    }
    out
}

/// Whether two values of one setting mean the same (`+0.50` and `0.5` do).
fn same(a: &Setting<'_, '_>, b: &Setting<'_, '_>) -> bool {
    let text = |s: &Setting<'_, '_>| match s {
        Setting::Attr(v) => Some(v.to_string()),
        Setting::Elem(n) if !n.children().any(|c| c.is_element()) => Some(n.text().unwrap_or("").to_string()),
        Setting::Elem(_) => None,
    };
    match (text(a), text(b), a, b) {
        (Some(x), Some(y), ..) => match (number(&x), number(&y)) {
            (Some(x), Some(y)) => x == y,
            _ => x.trim() == y.trim(),
        },
        (None, None, Setting::Elem(x), Setting::Elem(y)) => curve(*x).is_some() && curve(*x) == curve(*y),
        _ => false,
    }
}

/// PhotoCraft's own lossless record ([`PHOTOCRAFT_NS`] `CameraRaw`) over the defaults, keeping
/// the document-derived `pixelScale` / `encodingGamma` of `current`. `None` when absent,
/// unreadable or invalid.
fn own_settings(descs: &[roxmltree::Node<'_, '_>], current: &CameraRaw) -> Option<CameraRaw> {
    let json = descs.iter().find_map(|d| {
        d.attribute((PHOTOCRAFT_NS, OWN_ATTR)).map(str::to_string).or_else(|| {
            d.children()
                .find(|c| c.is_element() && c.tag_name().namespace() == Some(PHOTOCRAFT_NS) && c.tag_name().name() == OWN_ATTR)
                .map(|c| c.text().unwrap_or("").to_string())
        })
    })?;
    let mut p: CameraRaw = serde_json::from_str(&json).ok()?;
    p.pixel_scale = current.pixel_scale;
    p.encoding_gamma = current.encoding_gamma;
    p.clamp_ranges();
    p.validate().ok()?;
    Some(p)
}

/// Apply a Camera Raw settings file over `params`. Fails on input that isn't XMP or holds no
/// Camera Raw settings; never panics on malformed input. Only the top-level descriptions count
/// (see [`top_descriptions`]).
///
/// A file PhotoCraft wrote carries its full settings ([`PHOTOCRAFT_NS`]); those are used, so
/// PhotoCraft → PhotoCraft round-trips exactly (Upright guides included). A `crs:` setting
/// whose value no longer matches what PhotoCraft wrote beside them was changed since by
/// another app (Lightroom, Camera Raw), and is applied over them.
pub fn load(params: &mut CameraRaw, bytes: &[u8]) -> Result<LoadReport, String> {
    if bytes.len() > MAX_XMP_BYTES {
        return Err("the settings file is too large".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not a Camera Raw settings file (not UTF-8 text)".to_string())?;
    // Some writers put a byte order mark or an `<?xpacket?>` wrapper first; roxmltree takes both.
    let text = text.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("not a Camera Raw settings file ({e})"))?;
    let descs = top_descriptions(&doc);
    let settings = crs_settings(&descs);
    let own = own_settings(&descs, params);
    if settings.is_empty() && own.is_none() {
        return Err("no Camera Raw settings in the file".into());
    }
    let mut report = LoadReport::default();
    // What PhotoCraft writes for its own record: the baseline later edits are measured against.
    let written = own.as_ref().map(save_crs);
    let written_doc = written.as_deref().and_then(|t| roxmltree::Document::parse(t).ok());
    let written_descs = written_doc.as_ref().map(top_descriptions).unwrap_or_default();
    let baseline = crs_settings(&written_descs);
    let mut next = match own {
        Some(p) => {
            report.applied += 1;
            p
        }
        None => params.clone(),
    };
    for (name, value) in &settings {
        if baseline.iter().any(|(n, v)| n == name && same(v, value)) {
            continue;
        }
        match value {
            Setting::Attr(v) => apply(&mut next, &mut report, name, v),
            Setting::Elem(c) => {
                if let Some((_, field)) = CURVES.iter().find(|(n, _)| n == name) {
                    match curve(*c) {
                        Some(points) => {
                            *field(&mut next) = points;
                            report.applied += 1;
                        }
                        None => report.ignored.push(name.to_string()),
                    }
                } else if c.children().any(|g| g.is_element()) {
                    // Structured settings (masks, looks, lens profiles…) have no counterpart.
                    if !METADATA.contains(name) {
                        report.ignored.push(name.to_string());
                    }
                } else {
                    apply(&mut next, &mut report, name, c.text().unwrap_or(""));
                }
            }
        }
    }
    *params = next;
    Ok(report)
}

fn signed(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if v > 0.0 && !s.starts_with('+') { format!("+{s}") } else { s }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// PhotoCraft's own namespace: the full settings, losslessly, beside the `crs:` ones.
pub const PHOTOCRAFT_NS: &str = "https://photocraft.app/ns/camera-raw/1.0/";
/// Its one property: the settings that differ from the defaults, as the filter's JSON.
const OWN_ATTR: &str = "CameraRaw";

/// The JSON of the settings that differ from the defaults, without the document-derived
/// preview scale and transfer curve.
fn own_json(p: &CameraRaw) -> Option<String> {
    let (serde_json::Value::Object(mut map), serde_json::Value::Object(defaults)) =
        (serde_json::to_value(p).ok()?, serde_json::to_value(CameraRaw::default()).ok()?)
    else {
        return None;
    };
    map.retain(|k, v| k != "pixelScale" && k != "encodingGamma" && defaults.get(k) != Some(v));
    serde_json::to_string(&map).ok()
}

/// Write `params` as a Camera Raw settings file Photoshop's Load Settings… reads (and as a
/// raw file's sidecar, see [`save_sidecar`]). Besides the `crs:` settings it records the full
/// settings in [`PHOTOCRAFT_NS`], which [`load`] prefers.
pub fn save(params: &CameraRaw) -> String {
    let out = save_crs(params);
    let Some(json) = own_json(params) else {
        return out;
    };
    // `save_crs` declares `crs` on its own line and ends the start tag with `HasSettings`.
    let own = format!("crs:HasSettings=\"True\"\n   photocraft:{OWN_ATTR}=\"{}\">", escape(&json));
    out.replacen("crs:HasSettings=\"True\">", &own, 1).replacen("    xmlns:crs=", &format!("    xmlns:photocraft=\"{PHOTOCRAFT_NS}\"\n    xmlns:crs="), 1)
}

/// [`save`] without PhotoCraft's own record.
fn save_crs(params: &CameraRaw) -> String {
    let mut p = params.clone();
    let wb = if p.temperature != 0.0 || p.tint != 0.0 { "Custom" } else { "As Shot" };
    let mut out = String::from(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"PhotoCraft\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\"\n    xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"\n",
    );
    out.push_str("   crs:Version=\"18.4\"\n   crs:ProcessVersion=\"15.4\"\n");
    out.push_str(&format!("   crs:WhiteBalance=\"{wb}\"\n"));
    for (name, field, lo, _) in SCALARS {
        let v = *field(&mut p);
        // Exposure has two decimals in Camera Raw, the sharpening radius and the rotation one;
        // the rest are levels.
        let text = match *name {
            "Exposure2012" => signed(v, 2),
            "SharpenRadius" | "PerspectiveRotate" => signed(v, 1),
            _ if *lo < 0.0 => signed(v.round(), 0),
            _ => format!("{:.0}", v.round()),
        };
        out.push_str(&format!("   crs:{name}=\"{text}\"\n"));
    }
    let style = STYLES.iter().find(|(_, s)| *s == p.vignette_style).map_or("1", |(n, _)| n);
    out.push_str(&format!("   crs:PostCropVignetteStyle=\"{}\"\n", escape(style)));
    let upright = UPRIGHT_MODES.iter().position(|m| *m == p.upright).unwrap_or(0);
    out.push_str(&format!("   crs:PerspectiveUpright=\"{upright}\"\n"));
    out.push_str(&format!("   crs:ConvertToGrayscale=\"{}\"\n", if p.monochrome { "True" } else { "False" }));
    out.push_str("   crs:ToneCurveName2012=\"Custom\"\n   crs:HasSettings=\"True\">\n");
    for (name, field) in CURVES {
        let points = field(&mut p);
        let points = if points.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { points.clone() };
        out.push_str(&format!("   <crs:{name}>\n    <rdf:Seq>\n"));
        for [x, y] in points {
            out.push_str(&format!("     <rdf:li>{}, {}</rdf:li>\n", x.round(), y.round()));
        }
        out.push_str(&format!("    </rdf:Seq>\n   </crs:{name}>\n"));
    }
    out.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n");
    out
}

/// A raw file's sidecar path, as Camera Raw and Lightroom name it: the raw file's path with
/// its extension replaced by `.xmp` (`/dir/IMG_1.CR2` → `/dir/IMG_1.xmp`).
pub fn sidecar_path(raw_path: &str) -> String {
    std::path::Path::new(raw_path).with_extension("xmp").to_string_lossy().into_owned()
}

/// Byte offset just past the `>` of the start tag beginning at `start`, and whether it is
/// self-closing (`/>`). Quoted values are skipped, so a `>` inside one doesn't count.
fn start_tag_end(text: &str, start: usize) -> Option<(usize, bool)> {
    let bytes = text.as_bytes();
    let mut quote = None;
    let mut prev = 0u8;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some((i + 1, prev == b'/')),
            None => {}
        }
        prev = b;
    }
    None
}

/// `at` moved back over the ASCII whitespace before it.
fn back_over_space(text: &str, at: usize) -> usize {
    let before = text.get(..at).unwrap_or("");
    before.trim_end_matches(|c: char| c.is_ascii_whitespace()).len()
}

/// Write `params` as the sidecar of a raw file (`<raw stem>.xmp`, see [`sidecar_path`]).
///
/// Without `existing` this is [`save`]. With the current sidecar's text, everything else in it
/// is kept byte for byte (ratings, labels, keywords, other namespaces, Camera Raw settings
/// PhotoCraft has no control for, a Look's or a mask's nested settings) and only the settings
/// [`save`] writes are replaced in, or inserted into, the top-level description. The White
/// Balance menu is kept while Temperature and Tint are untouched, since a raw file's balance
/// (`crs:Temperature` in kelvin, `crs:Tint`) has no PhotoCraft counterpart. Fails, so the
/// caller leaves the file alone, when `existing` isn't XMP this can edit safely: not
/// well-formed, a DTD, no `rdf:RDF`, or the `crs` / `rdf` / `photocraft` prefixes bound to
/// other namespaces.
pub fn save_sidecar(params: &CameraRaw, existing: Option<&str>) -> Result<String, String> {
    let fresh = save(params);
    let Some(existing) = existing else {
        return Ok(fresh);
    };
    if existing.len() > MAX_XMP_BYTES {
        return Err("the sidecar is too large".into());
    }
    let (bom, text) = match existing.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", existing),
    };
    if text.contains("<!DOCTYPE") {
        // Entity expansion would make the byte ranges below unreliable.
        return Err("the sidecar has a DTD".into());
    }
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("the sidecar isn't valid XMP ({e})"))?;
    let rdf = doc.descendants().find(|n| is_rdf(n, "RDF")).ok_or("the sidecar has no rdf:RDF")?;
    if rdf.lookup_namespace_uri(Some("rdf")) != Some(RDF_NS) {
        return Err("the sidecar uses another prefix for RDF".into());
    }
    let descs = top_descriptions(&doc);
    let ours = roxmltree::Document::parse(&fresh).map_err(|e| e.to_string())?;
    let our_desc = top_descriptions(&ours).into_iter().next().ok_or("no description written")?;
    let keep_wb = params.temperature == 0.0 && params.tint == 0.0 && crs_settings(&descs).iter().any(|(n, _)| *n == "WhiteBalance");
    let owned = |ns: Option<&str>, name: &str| match ns {
        Some(CRS_NS) => !(keep_wb && name == "WhiteBalance") && (our_desc.attribute((CRS_NS, name)).is_some() || CURVES.iter().any(|(n, _)| *n == name)),
        Some(PHOTOCRAFT_NS) => name == OWN_ATTR,
        _ => false,
    };
    // Edits as (start, end, replacement) over `text`; insertions have start == end.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for desc in &descs {
        for a in desc.attributes().filter(|a| owned(a.namespace(), a.name())) {
            edits.push((back_over_space(text, a.range().start), a.range().end, String::new()));
        }
        for c in desc.children().filter(|c| c.is_element() && owned(c.tag_name().namespace(), c.tag_name().name())) {
            edits.push((back_over_space(text, c.range().start), c.range().end, String::new()));
        }
    }
    // What goes in: our attributes (as `save` wrote them, already escaped) and curve elements.
    let mut attrs = String::new();
    for a in our_desc.attributes().filter(|a| owned(a.namespace(), a.name())) {
        let prefix = if a.namespace() == Some(PHOTOCRAFT_NS) { "photocraft" } else { "crs" };
        let value = fresh.get(a.range_value()).ok_or("bad range")?;
        attrs.push_str(&format!("\n   {prefix}:{}=\"{value}\"", a.name()));
    }
    let mut elems = String::new();
    for c in our_desc.children().filter(|c| c.is_element()) {
        elems.push_str("\n   ");
        elems.push_str(fresh.get(c.range()).ok_or("bad range")?);
    }
    // The description that gets them: the first one in scope of the crs namespace, else the
    // first; an empty rdf:RDF gets ours.
    let Some(target) = descs.iter().find(|d| d.lookup_prefix(CRS_NS).is_some()).or(descs.first()) else {
        let body = fresh.get(our_desc.range()).ok_or("bad range")?;
        let (rdf_end, self_closing) = start_tag_end(text, rdf.range().start).ok_or("the sidecar's rdf:RDF is cut off")?;
        let close = text.get(..rdf.range().end).and_then(|t| t.rfind("</")).filter(|_| !self_closing).ok_or("the sidecar's rdf:RDF is empty")?;
        edits.push((close.max(rdf_end), close.max(rdf_end), format!("  {body}\n ")));
        return apply_edits(bom, text, edits);
    };
    for (prefix, ns) in [("crs", CRS_NS), ("photocraft", PHOTOCRAFT_NS)] {
        match target.lookup_namespace_uri(Some(prefix)) {
            Some(uri) if uri == ns => {}
            Some(_) => return Err(format!("the sidecar binds the {prefix} prefix to another namespace")),
            None if !attrs.contains(&format!("{prefix}:")) => {}
            None => attrs.insert_str(0, &format!("\n    xmlns:{prefix}=\"{ns}\"")),
        }
    }
    let start = target.range().start;
    let (tag_end, self_closing) = start_tag_end(text, start).ok_or("the sidecar's description is cut off")?;
    if self_closing {
        // `<rdf:Description …/>` becomes `<rdf:Description …>…</rdf:Description>`.
        let qname = text
            .get(start + 1..tag_end)
            .and_then(|t| t.split(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>').next())
            .ok_or("the sidecar's description is cut off")?;
        edits.push((tag_end.saturating_sub(2), tag_end, format!("{attrs}>{elems}\n  </{qname}>")));
    } else {
        let gt = tag_end.saturating_sub(1);
        edits.push((gt, gt, attrs));
        let close = text.get(..target.range().end).and_then(|t| t.rfind("</")).ok_or("the sidecar's description is cut off")?;
        let at = back_over_space(text, close).max(tag_end);
        edits.push((at, at, elems));
    }
    apply_edits(bom, text, edits)
}

/// `text` with `edits` applied (sorted by position; overlapping ones are an error), checked to
/// still parse.
fn apply_edits(bom: &str, text: &str, mut edits: Vec<(usize, usize, String)>) -> Result<String, String> {
    edits.sort_by_key(|e| (e.0, e.1));
    let mut out = String::from(bom);
    let mut at = 0;
    for (start, end, with) in &edits {
        out.push_str(text.get(at..*start).ok_or("the sidecar's settings overlap")?);
        out.push_str(with);
        at = *end;
    }
    out.push_str(text.get(at..).ok_or("bad range")?);
    roxmltree::Document::parse(out.trim_start_matches('\u{feff}')).map_err(|e| format!("the updated sidecar isn't valid XMP ({e})"))?;
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A hand-written settings file laid out as Photoshop 26's Camera Raw Filter saves one (the
    /// field set and order follow Camera Raw 18; the values are synthetic), plus element-form
    /// settings, which XMP allows and some writers use.
    const FIXTURE: &[u8] = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   crs:Version="18.1.1" crs:ProcessVersion="15.4" crs:WhiteBalance="Custom"
   crs:IncrementalTemperature="0" crs:IncrementalTint="+10" crs:Exposure2012="+0.05"
   crs:Contrast2012="-5" crs:Highlights2012="-20" crs:Shadows2012="+10" crs:Whites2012="+35"
   crs:Blacks2012="0" crs:Texture="+30" crs:Clarity2012="+5" crs:Vibrance="+20" crs:Saturation="-5"
   crs:ParametricShadowSplit="25" crs:ParametricMidtoneSplit="50" crs:ParametricHighlightSplit="75"
   crs:Sharpness="15" crs:SharpenRadius="+1.0" crs:LuminanceSmoothing="5"
   crs:LuminanceNoiseReductionContrast="0" crs:HueAdjustmentGreen="+25" crs:SaturationAdjustmentGreen="+20"
   crs:ColorGradeBlending="50" crs:LensProfileEnable="1" crs:VignetteAmount="-20"
   crs:PerspectiveScale="100" crs:GrainAmount="10" crs:RedHue="-5" crs:GrainSeed="2834497962"
   crs:ToneCurveName2012="Linear" crs:CameraProfile="Default Color" crs:HasSettings="True" crs:HasCrop="False">
   <crs:Dehaze>+12</crs:Dehaze>
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
   <crs:ToneCurvePV2012Red>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>128, 140</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012Red>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    #[test]
    fn reads_a_photoshop_preset() {
        let mut p = CameraRaw::default();
        let r = load(&mut p, FIXTURE).unwrap();
        assert_eq!(p.tint, 10.0);
        assert_eq!(p.exposure, 0.05);
        assert_eq!((p.contrast, p.highlights, p.shadows, p.whites), (-5.0, -20.0, 10.0, 35.0));
        assert_eq!((p.texture, p.clarity, p.vibrance, p.saturation), (30.0, 5.0, 20.0, -5.0));
        assert_eq!((p.hsl_hue[3], p.hsl_sat[3]), (25.0, 20.0));
        assert_eq!((p.sharpen_amount, p.noise_luminance, p.grain_amount), (15.0, 5.0, 10.0));
        assert_eq!(p.grade_blending, 50.0);
        // The linear tone curve is PhotoCraft's empty one; an element-form setting is read too.
        assert!(p.point_curve.is_empty());
        assert_eq!(p.point_curve_red, vec![[0.0, 0.0], [128.0, 140.0], [255.0, 255.0]]);
        assert_eq!(p.dehaze, 12.0);
        assert_eq!((p.lens_vignette, p.calibration_red_hue, p.transform_scale), (-20.0, -5.0, 100.0));
        // Lens profiles have no counterpart.
        assert_eq!(r.ignored, vec!["LensProfileEnable".to_string()]);
    }

    #[test]
    fn save_then_load_round_trips() {
        let mut p = CameraRaw {
            temperature: -12.0,
            exposure: 1.25,
            contrast: 30.0,
            sharpen_radius: 1.4,
            vignette_amount: -40.0,
            vignette_style: "paintOverlay".into(),
            curve_splits: [20.0, 45.0, 80.0],
            point_curve: vec![[0.0, 10.0], [128.0, 150.0], [255.0, 240.0]],
            ..CameraRaw::default()
        };
        p.hsl_lum[5] = -33.0;
        p.grade_shadows.hue = 210.0;
        p.grade_shadows.sat = 40.0;
        p.grade_global.lum = 7.0;
        let text = save(&p);
        assert!(text.contains("crs:Exposure2012=\"+1.25\""));
        assert!(text.contains("crs:WhiteBalance=\"Custom\""));
        let mut back = CameraRaw::default();
        let r = load(&mut back, text.as_bytes()).unwrap();
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
        assert_eq!(back, p);
    }

    #[test]
    fn load_applies_only_the_settings_in_the_file() {
        let mut p = CameraRaw { contrast: 40.0, ..CameraRaw::default() };
        let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"><crs:Exposure2012>-9</crs:Exposure2012>
            <crs:Clarity2012>abc</crs:Clarity2012></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let r = load(&mut p, xmp).unwrap();
        assert_eq!((p.exposure, p.contrast, p.clarity), (-5.0, 40.0, 0.0));
        assert_eq!(r, LoadReport { applied: 1, ignored: vec!["Clarity2012".into()] });
    }

    #[test]
    fn malformed_input_is_an_error_and_leaves_settings_alone() {
        let before = CameraRaw { exposure: 0.5, ..CameraRaw::default() };
        for bad in [
            &b""[..],
            b"\xff\xfe\x00garbage",
            b"<x:xmpmeta><unclosed",
            b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>",
            b"<?xml version=\"1.0\"?><!DOCTYPE a [<!ENTITY e \"x\">]><a>&e;</a>",
        ] {
            let mut p = before.clone();
            assert!(load(&mut p, bad).is_err());
            assert_eq!(p, before);
        }
        // A broken curve or NaN is skipped, not fatal.
        let xmp = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
            xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Contrast2012="NaN"><crs:ToneCurvePV2012><rdf:Seq>
            <rdf:li>10</rdf:li></rdf:Seq></crs:ToneCurvePV2012></rdf:Description></rdf:RDF>"#;
        let mut p = before.clone();
        let r = load(&mut p, xmp).unwrap();
        assert_eq!(p, before);
        assert_eq!(r.ignored, vec!["Contrast2012".to_string(), "ToneCurvePV2012".to_string()]);
    }

    #[test]
    fn crs_settings_alone_round_trip() {
        // Without PhotoCraft's own record (as from another app) the crs: mapping carries them.
        let mut p = CameraRaw {
            upright: "vertical".into(),
            monochrome: true,
            lens_distortion: 12.0,
            lens_vignette: -30.0,
            transform_vertical: -15.0,
            transform_rotate: 2.5,
            transform_scale: 110.0,
            transform_offset_y: 8.0,
            defringe_purple_amount: 4.0,
            defringe_green_hue: [35.0, 65.0],
            calibration_blue_sat: 20.0,
            noise_color_smoothness: 70.0,
            curve_refine_saturation: 80.0,
            ..CameraRaw::default()
        };
        p.hsl_sat[2] = 15.0;
        let text = save_crs(&p);
        assert!(!text.contains("photocraft:"));
        assert!(text.contains("crs:PerspectiveUpright=\"3\"") && text.contains("crs:PerspectiveRotate=\"+2.5\""), "{text}");
        let mut back = CameraRaw::default();
        let r = load(&mut back, text.as_bytes()).unwrap();
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
        assert_eq!(back, p);
    }

    #[test]
    fn own_record_round_trips_exactly_and_later_crs_edits_win() {
        let p = CameraRaw {
            exposure: 0.123,
            contrast: 12.5,
            upright: "guided".into(),
            upright_guides: vec![[[0.1, 0.2], [0.15, 0.9]], [[0.8, 0.1], [0.82, 0.95]]],
            // Escaped in the attribute: quotes, ampersands and angle brackets.
            vignette_style: "a\"&<b>'".into(),
            seed: 7,
            ..CameraRaw::default()
        };
        let text = save(&p);
        assert!(text.contains("xmlns:photocraft=\"https://photocraft.app/ns/camera-raw/1.0/\""));
        assert!(text.contains("&quot;") && text.contains("&amp;") && text.contains("&lt;"), "{text}");
        let current = CameraRaw { pixel_scale: 0.5, encoding_gamma: 1.8, ..CameraRaw::default() };
        let mut back = current.clone();
        load(&mut back, text.as_bytes()).unwrap();
        assert_eq!(back, CameraRaw { pixel_scale: 0.5, encoding_gamma: 1.8, ..p.clone() });
        // Lightroom changed Contrast since: that one setting comes from crs:, the rest stays exact.
        let edited = text.replace("crs:Contrast2012=\"+13\"", "crs:Contrast2012=\"-40\"");
        assert_ne!(edited, text);
        let mut back = CameraRaw::default();
        load(&mut back, edited.as_bytes()).unwrap();
        assert_eq!(back, CameraRaw { contrast: -40.0, ..p });
    }

    /// A sidecar laid out as Lightroom Classic / Camera Raw 18 write one next to a raw file
    /// (synthetic values): an xpacket wrapper, ratings and keywords, kelvin white balance,
    /// settings PhotoCraft has no control for, a Look with nested settings, and a tone curve.
    const SIDECAR: &str = r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:tiff="http://ns.adobe.com/tiff/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   xmp:Rating="4"
   xmp:Label="Red"
   xmp:ModifyDate="2026-09-30T18:04:11+02:00"
   tiff:Make="Canon"
   crs:Version="18.1.1"
   crs:ProcessVersion="15.4"
   crs:WhiteBalance="Custom"
   crs:Temperature="5600"
   crs:Tint="+12"
   crs:Exposure2012="+0.50"
   crs:Contrast2012="+15"
   crs:Highlights2012="-48"
   crs:Shadows2012="+31"
   crs:Whites2012="+8"
   crs:Blacks2012="-6"
   crs:Texture="+10"
   crs:Clarity2012="+5"
   crs:Dehaze="+3"
   crs:Vibrance="+12"
   crs:Saturation="0"
   crs:Sharpness="40"
   crs:SharpenRadius="+1.0"
   crs:SharpenDetail="25"
   crs:SharpenEdgeMasking="0"
   crs:LuminanceSmoothing="10"
   crs:ColorNoiseReduction="25"
   crs:HueAdjustmentOrange="-4"
   crs:SaturationAdjustmentBlue="-20"
   crs:LuminanceAdjustmentAqua="+7"
   crs:ColorGradeMidtoneHue="35"
   crs:ColorGradeMidtoneSat="10"
   crs:ColorGradeBlending="50"
   crs:LensProfileEnable="1"
   crs:LensProfileName="Adobe (Canon EF 24-70mm f/2.8L II USM)"
   crs:AutoLateralCA="1"
   crs:PerspectiveUpright="0"
   crs:GrainSeed="1820193414"
   crs:ToneCurveName2012="Custom"
   crs:CameraProfile="Adobe Standard"
   crs:HasSettings="True"
   crs:HasCrop="False"
   crs:AlreadyApplied="False">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>Berlin &amp; Brandenburg</rdf:li>
     <rdf:li>street</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>64, 58</rdf:li>
     <rdf:li>192, 200</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
   <crs:Look>
    <rdf:Description
     crs:Name="Adobe Standard"
     crs:Amount="1"
     crs:UUID="B952C231111CD8E0ECCF14B86BAA7077">
    <crs:Parameters>
     <rdf:Description
      crs:Version="18.1.1"
      crs:ProcessVersion="15.4"
      crs:ConvertToGrayscale="True"
      crs:Clarity2012="+99"/>
    </crs:Parameters>
    </rdf:Description>
   </crs:Look>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn reads_an_adobe_sidecar() {
        let mut p = CameraRaw::default();
        let r = load(&mut p, SIDECAR.as_bytes()).unwrap();
        assert_eq!((p.exposure, p.contrast, p.highlights, p.shadows), (0.5, 15.0, -48.0, 31.0));
        assert_eq!((p.whites, p.blacks, p.texture, p.clarity, p.dehaze, p.vibrance), (8.0, -6.0, 10.0, 5.0, 3.0, 12.0));
        assert_eq!((p.sharpen_amount, p.noise_luminance, p.noise_color), (40.0, 10.0, 25.0));
        assert_eq!((p.hsl_hue[1], p.hsl_sat[5], p.hsl_lum[4]), (-4.0, -20.0, 7.0));
        assert_eq!((p.grade_midtones.hue, p.grade_midtones.sat), (35.0, 10.0));
        assert_eq!(p.point_curve, vec![[0.0, 0.0], [64.0, 58.0], [192.0, 200.0], [255.0, 255.0]]);
        // The Look's nested settings aren't the image's; the kelvin balance has no counterpart.
        assert!(!p.monochrome);
        assert_eq!((p.temperature, p.tint), (0.0, 0.0));
        assert!(r.ignored.contains(&"Temperature".to_string()) && r.ignored.contains(&"LensProfileEnable".to_string()), "{:?}", r.ignored);
    }

    #[test]
    fn save_sidecar_keeps_everything_it_does_not_own() {
        let mut p = CameraRaw::default();
        load(&mut p, SIDECAR.as_bytes()).unwrap();
        p.exposure = 1.25;
        p.upright = "guided".into();
        p.upright_guides = vec![[[0.1, 0.1], [0.12, 0.9]], [[0.9, 0.1], [0.88, 0.9]]];
        let out = save_sidecar(&p, Some(SIDECAR)).unwrap();
        // Everything not ours, byte for byte.
        for kept in [
            "<?xpacket begin=\"﻿\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta",
            "   xmp:Rating=\"4\"\n   xmp:Label=\"Red\"\n   xmp:ModifyDate=\"2026-09-30T18:04:11+02:00\"\n   tiff:Make=\"Canon\"",
            "crs:Temperature=\"5600\"\n   crs:Tint=\"+12\"",
            "crs:LensProfileName=\"Adobe (Canon EF 24-70mm f/2.8L II USM)\"\n   crs:AutoLateralCA=\"1\"",
            "crs:CameraProfile=\"Adobe Standard\"",
            "crs:HasCrop=\"False\"\n   crs:AlreadyApplied=\"False\"",
            "   <dc:subject>\n    <rdf:Bag>\n     <rdf:li>Berlin &amp; Brandenburg</rdf:li>\n     <rdf:li>street</rdf:li>\n    </rdf:Bag>\n   </dc:subject>",
            "<?xpacket end=\"w\"?>",
        ] {
            assert!(out.contains(kept), "lost {kept:?} in\n{out}");
        }
        let look = SIDECAR.find("   <crs:Look>").and_then(|a| SIDECAR.find("</crs:Look>").map(|b| &SIDECAR[a..b])).unwrap();
        assert!(out.contains(look));
        // Ours replaced, once each; the kelvin balance's menu kept since Temperature is untouched.
        assert!(out.contains("crs:Exposure2012=\"+1.25\"") && !out.contains("crs:Exposure2012=\"+0.50\""));
        assert_eq!(out.matches("crs:Exposure2012=").count(), 1);
        assert_eq!(out.matches("<crs:ToneCurvePV2012>").count(), 1);
        assert_eq!(out.matches("crs:WhiteBalance=").count(), 1);
        assert!(out.contains("crs:WhiteBalance=\"Custom\"") && out.contains("crs:PerspectiveUpright=\"5\""));
        // PhotoCraft reads its own settings back exactly, and saving again changes nothing.
        let mut back = CameraRaw::default();
        load(&mut back, out.as_bytes()).unwrap();
        assert_eq!(back, p);
        assert_eq!(save_sidecar(&p, Some(&out)).unwrap(), out);
    }

    #[test]
    fn save_sidecar_fills_sparse_sidecars() {
        let p = CameraRaw { clarity: 20.0, ..CameraRaw::default() };
        for existing in [
            // Ratings only, a self-closing description without the crs namespace.
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmp:Rating=\"2\"/></rdf:RDF></x:xmpmeta>",
            // Element-form settings, which are replaced.
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"><crs:Clarity2012>-5</crs:Clarity2012><crs:Grain>keep</crs:Grain></rdf:Description></rdf:RDF></x:xmpmeta>",
            // No description at all.
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"></rdf:RDF></x:xmpmeta>",
        ] {
            let out = save_sidecar(&p, Some(existing)).unwrap();
            let mut back = CameraRaw::default();
            load(&mut back, out.as_bytes()).unwrap();
            assert_eq!(back, p, "{out}");
            assert_eq!(out.matches("Clarity2012").count(), 1, "{out}");
            assert!(!existing.contains("Rating") || out.contains("xmp:Rating=\"2\""));
            assert!(!existing.contains("Grain>") || out.contains("<crs:Grain>keep</crs:Grain>"));
        }
        assert_eq!(save_sidecar(&p, None).unwrap(), save(&p));
    }

    #[test]
    fn save_sidecar_refuses_what_it_cannot_edit_safely() {
        let p = CameraRaw::default();
        for bad in [
            "",
            "garbage",
            "<nope",
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>",
            "<?xml version=\"1.0\"?><!DOCTYPE a [<!ENTITY e \"x\">]><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">&e;</rdf:RDF>",
            // The crs prefix taken by something else.
            "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"urn:other\"/></rdf:RDF>",
            // RDF under another prefix.
            "<r:RDF xmlns:r=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><r:Description/></r:RDF>",
        ] {
            assert!(save_sidecar(&p, Some(bad)).is_err(), "{bad}");
        }
        // Every truncation of a real sidecar: an error or a valid result, never a panic.
        for cut in (0..SIDECAR.len()).filter(|i| SIDECAR.is_char_boundary(*i)) {
            let part = &SIDECAR[..cut];
            if let Ok(out) = save_sidecar(&p, Some(part)) {
                assert!(roxmltree::Document::parse(&out).is_ok());
            }
            let _ = load(&mut CameraRaw::default(), part.as_bytes());
        }
        assert!(save_sidecar(&p, Some(&SIDECAR[..SIDECAR.len() / 2])).is_err());
    }

    #[test]
    fn sidecar_path_swaps_the_extension() {
        assert_eq!(sidecar_path("/dir/IMG_1.CR2"), "/dir/IMG_1.xmp");
        assert_eq!(sidecar_path("/dir/a.b/DSC 2.nef"), "/dir/a.b/DSC 2.xmp");
        assert_eq!(sidecar_path("/dir/raw"), "/dir/raw.xmp");
    }
}
