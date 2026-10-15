//! # photocraft-io
//!
//! Import and export between [`photocraft_doc::Document`] and files:
//!
//! * PSD / PSB (detected by the `8BPS` signature) via `photocraft-psd`, with
//!   fidelity levels 1–3 (raster layers, masks, blend/opacity/fill,
//!   visibility, names, groups, clipping, adjustment and fill layers).
//!   Every layer keeps its unmodelled tagged blocks in `Layer::psd_blocks`
//!   (vector masks, blending options, original adjustment/fill blocks, text
//!   and smart-object data) and re-export writes them back; text, shape and
//!   smart-object layers also keep their pixels as the cached raster, and fill
//!   layers keep Photoshop's rendering in `Layer::fill_cache`.
//! * Camera raws (DNG, CR2, uncompressed / lossless TIFF-EP raws) via
//!   `photocraft-raw`, developed into a 16-bit ProPhoto RGB "Background"
//!   layer; unsupported raw variants fall back to the embedded JPEG preview.
//! * Layered TIFFs (Photoshop layer data in tags 37724 and 34377) open with their
//!   layers through the PSD path and are written back the same way; see `tiff_layers`.
//! * Paint.NET PDN3 documents open as editable bitmap layers (import only).
//! * OpenRaster (`.ora`) documents open and save with their layers, groups, blend modes,
//!   opacity and visibility; see `ora`.
//! * Krita (`.kra`, `.krz`) documents open with their paint layers, groups and transparency
//!   masks (import only); see `kra`.
//! * Affinity documents (`.af`, `.afdesign`, `.afphoto`, `.afpub`) open natively
//!   with no source save path, what isn't imported listed in the warnings; a file
//!   whose native data can't be read opens as its embedded preview; see `affinity`.
//! * Every other format goes through `photocraft-codecs` as a single layer
//!   (depth and Gray/RGB/CMYK model preserved): a locked "Background" when the
//!   image is opaque, a normal "Layer 0" when it has transparency.
//!
//! Exports to PSD render the merged composite with
//! `photocraft_compose::flatten`; flat exports report what is lost.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod abr_map;
#[cfg(not(target_arch = "wasm32"))]
pub mod accounted_native;
pub mod accounted_open_quota;
mod adjust_map;
pub mod affinity;
pub mod annotations_map;
pub mod blocks;
pub mod camera_raw_xmp;
mod channel_map;
pub mod comps_map;
pub mod effects_map;
pub mod exr_parts;
mod flat;
mod gradient_bake;
pub use gradient_bake::{Method as GradientMethod, bake as bake_gradient};
mod kra;
pub mod linked;
mod multichannel_map;
pub mod native_plane_store;
mod native_save;
mod native_open;
pub use native_open::import_native_stream_candidate;
mod native_save_model;
mod native_scene_preview;
#[cfg(not(target_arch = "wasm32"))]
pub use native_save::{save_native_path, save_native_path_guarded, save_native_path_guarded_with_resident_guard};
pub use native_save::{NativeSaveLimits, NativeSaveResult, PreparedNativeStream, prepare_native_stream, retained_native_input_bytes, retained_native_model_bytes, retained_native_payload_bytes, save_native_bytes, save_native_stream};
pub use native_save::{
    NativeCensusVisits, NativeCurrentCensus, NativeModelCensus, NativePayloadCensus, retained_native_model_census, retained_native_payload_census,
};
mod origin;
mod ora;
pub use ora::losses as ora_losses;
pub mod pattern_map;
mod pdn;
pub mod pixels;
mod psd_export;
mod psd_import;
pub mod psd_thumbnail;
pub mod raw;
pub mod slices_map;
pub mod smart_map;
pub mod source_protection;
pub mod svg;
mod text_import;
pub mod text_styles_map;
pub mod tiff_layers;
pub mod vector_map;
mod wide_export;
pub mod xcf_assessment;
pub mod xcf_channel_candidate;
pub mod xcf_color_to_alpha_candidate;
mod xcf_current_assessment;
mod xcf_export;
mod xcf_export_loss;
mod xcf_export_precision;
mod xcf_filters;
pub use xcf_filters::{CurvesMappingLimits, CurvesMappingRequirements, MappedCurvesSource, map_retained_curves_source};
pub use xcf_filters::{LevelsMappingLimits, LevelsMappingRequirements, MappedLevelsSource, map_retained_levels_source};
pub use xcf_filters::{
    ExceptionalGraphProvenance, GraphOuterMetadata, GraphSourceAuthority, GraphSourceLimits, GraphSourceRequirements,
    MappedGraphSource, map_exceptional_graph_source,
};
pub use xcf_filters::{
    BRIGHTNESS_CONTRAST_SCHEMA, BRIGHTNESS_CONTRAST_SCHEMA_308, BrightnessContrastMappingLimits, BrightnessContrastMappingRequirements,
    BrightnessContrastSourceIdentity, MappedBrightnessContrastSource, map_retained_brightness_contrast_source,
    HUE_SATURATION_SCHEMA, HueSaturationMappingLimits, HueSaturationMappingRequirements,
    HueSaturationSourceIdentity, MappedHueSaturationSource, map_retained_hue_saturation_source,
    SceneChannelBinding, SceneChannelLimits, SceneChannelRequirements, SceneChannelStorage,
    StoredSceneChannel, StoredSceneEffect, assemble_scene_channel_storage,
    ConfiguredEffectBinding, ConfiguredEffectIdentity, ConfiguredSceneStacks,
    SceneConfiguredLimits, SceneConfiguredRequirements, map_scene_channel_configured,
};
#[cfg(test)]
#[path = "../tests/common/xcf.rs"]
pub(crate) mod xcf_fixture;
pub mod xcf_graphs;
pub mod xcf_graph_candidate;
pub use xcf_filters::graph_argument_candidate;
pub use xcf_filters::scene_channel_preparation_candidate;
pub use xcf_filters::scene_channel_input_candidate;
pub use xcf_filters::colorize_source;
pub use xcf_filters::stretch_contrast_source;
pub use xcf_filters::checkerboard_source;
pub use xcf_filters::{
    STRETCH_HSV_SCHEMA, MappedStretchHsvSource, StretchHsvMappingLimits,
    StretchHsvMappingRequirements, StretchHsvSourceIdentity, map_retained_stretch_hsv_source,
};
pub mod xcf_import;
pub use xcf_import::{OwnedPlainXcfImport, import_owned_staged_plain_bytes, import_owned_staged_plain_bytes_in_quota, import_owned_staged_plain_from_reader_in_quota};
pub use xcf_import::{OwnedThresholdAlphaXcfImport, import_owned_staged_threshold_alpha_bytes_in_quota};
pub use xcf_import::{OwnedHistogramThresholdXcfImport, OwnedPointXcfImport, OwnedAnalyticGeometryXcfImport, OwnedIsolatedNormalGroupXcfImport, OwnedIsolatedGaussianGroupXcfImport};
pub use xcf_import::OwnedSeparableBlendXcfImport;
pub use xcf_import::{OwnedXcfImport, import_owned_xcf_bytes_in_quota, import_owned_xcf_from_reader_in_quota};
pub use xcf_import::{
    AssembledStoredChannel, CorrespondenceRequirements, NamespaceBacking, StoredChannelCorrespondence, assemble_owned_stored_channel,
};
pub use xcf_import::{
    CurrentProfilePolicy, SelectedCurrentChannel, SelectedLimits, SelectedProfileRequirements, SelectedRequirements, select_current,
    select_current_with_profile_policy, selected_profile_requirements,
};
pub use xcf_import::{
    LayerOuterPolicy, LayerSelectionCandidate, LayerSelectionIds, LayerSelectionLimits, LayerSelectionNamespace, LayerSelectionRequirements,
    LAYER_SELECTION_EPOCH, map_layer_selection, map_layer_selection_with_outer_policy,
};
pub use xcf_import::{
    AssembledLayerSelection, LayerCorrespondenceRequirements, LayerCurrentLimits, LayerCurrentRequirements, LayerOriginalPlanesProof,
    LayerProfilePolicy, LayerProfileRequirements, SelectedCurrentLayer, assemble_owned_layer_selection,
    assemble_owned_layer_selection_with_outer_policy, layer_profile_requirements, select_layer_current, select_layer_current_with_profile_policy,
};
pub use xcf_import::{LayerStorageRequirements, PrivateLayerCurrent, assemble_private_layer_current, layer_storage_requirements};
mod xcf_link_sources;
pub mod xcf_modified_sources;
mod xcf_ordinary_source;
mod xcf_paths;
mod xcf_point_replace_source;
pub mod xcf_policy;
pub mod xcf_source;
pub mod xcf_sources;

pub use photocraft_codecs::DpxBits;
use photocraft_codecs::{CodecError, EncodeOptions};
use photocraft_doc::Document;
use photocraft_psd::{PsdError, PsdFile};

pub use adjust_map::ADJUSTMENT_KEYS;
pub use flat::{
    DEFAULT_LOSSY_WEBP_QUALITY, apply_file_encoding, document_to_image, file_encoding, import_gif_frames, import_tiff_page, tga_alpha_channel_to_transparency,
    tga_default_bits,
};
pub use origin::{ImportFormat, ImportOrigin, SourcePolicy};
pub use psd_export::{PsdExportOptions, document_to_psd, document_to_psd_with, try_document_to_psd_with};
pub use psd_import::{psd_to_document, psd_to_document_with, try_psd_to_document_with};

/// Errors from import/export.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    /// Required text assets have not completed their checked bootstrap.
    #[error("{0}")]
    FontBootstrap(#[from] photocraft_text::FontBootstrapError),
    /// XCF parse or semantic admission failure.
    #[error("{0}")]
    Xcf(#[from] photocraft_xcf::XcfError),
    /// Paint.NET document decode failure.
    #[error("PDN: {0}")]
    Pdn(String),
    /// Krita document read failure.
    #[error("Krita: {0}")]
    Kra(String),
    /// OpenRaster read failure.
    #[error("OpenRaster: {0}")]
    Ora(String),
    /// PSD parse/write failure.
    #[error("PSD: {0}")]
    Psd(#[from] PsdError),
    /// Codec failure.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
    /// The file type could not be determined.
    #[error("unknown file format: {0}")]
    UnknownFormat(String),
    /// The requested conversion is not possible.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Native `.pcraft` bundle failure.
    #[error("pcraft: {0}")]
    Pcraft(#[from] photocraft_format::FormatError),
    /// Camera raw decode failure.
    #[error("{0}")]
    Raw(#[from] photocraft_raw::RawError),
    /// An SVG that does not parse (or is too large to rasterise).
    #[error("SVG: {0}")]
    Svg(String),
    /// A background import or save was cancelled ([`import_with`], [`export_with`]).
    #[error("cancelled")]
    Cancelled,
}

impl From<photocraft_raster::RasterInputError> for IoError {
    fn from(error: photocraft_raster::RasterInputError) -> Self {
        if error == photocraft_raster::RasterInputError::Cancelled { Self::Cancelled } else { Self::Unsupported(error.to_string()) }
    }
}

impl From<photocraft_compose::RenderBandError<()>> for IoError {
    fn from(error: photocraft_compose::RenderBandError<()>) -> Self {
        match error {
            photocraft_compose::RenderBandError::Render(error) => error.into(),
            photocraft_compose::RenderBandError::Sink(()) => Self::Unsupported("composite output sink failed".into()),
        }
    }
}

/// Result of [`import`].
#[derive(Debug, Clone)]
pub struct ImportResult {
    /// The imported document.
    pub document: Document,
    /// Human-readable notes about anything approximated or dropped.
    pub warnings: Vec<String>,
    /// Actual detected format and runtime source write policy.
    pub origin: ImportOrigin,
    /// Current XCF fidelity after semantic admission. Reevaluate after edits;
    /// this report grants no resource access and is not persisted as authority.
    pub assessment: Option<xcf_assessment::ImportAssessment>,
    /// Save must not write back to an import-only source, such as Affinity or Paint.NET.
    pub source_read_only: bool,
    /// Only a stand-in picture of the file (an Affinity document whose native data couldn't be
    /// read): Open shows it with its warning; Place and other auxiliary imports refuse it.
    pub preview_only: bool,
}

/// Result of [`export`].
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Encoded file.
    pub bytes: Vec<u8>,
    /// Human-readable notes about anything approximated or dropped.
    pub warnings: Vec<String>,
}

/// Which part of the document's XMP packet and free-form text a flat export embeds.
/// PSD/PSB always keep XMP; `.pcraft` keeps both. Save As and conversions keep the whole packet, as
/// Photoshop's Save As does; Export As starts at `None`, because the packet lists the text of
/// every type layer and one id per placed document (#647).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XmpEmbed {
    /// Embed the document's whole XMP packet and free-form text.
    #[default]
    All,
    /// Embed no XMP or free-form text.
    None,
}

/// Export options.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Codec options for flat formats.
    pub encode: EncodeOptions,
    /// Write PSB even for `.psd` names when the document is small.
    pub force_psb: bool,
    /// TIFF: keep the layers (Photoshop layer data in tag 37724). Off by default, so scripted
    /// and agent saves (CLI, batch, MCP) write a flat TIFF unless they ask for layers; the app's
    /// Save As sets it from its Layers option, which keeps them as Photoshop does. `false` is
    /// Photoshop's "Discard Layers and Save a Copy".
    pub tiff_layers: bool,
    /// Which part of the document's XMP packet and free-form text a flat export embeds.
    /// PSD/PSB always keep XMP; `.pcraft` keeps both. Everything by default, as Save As does;
    /// Export As offers None.
    pub xmp: XmpEmbed,
    /// Targa: bits per pixel of an RGB file. `None` writes 32 when the document has an alpha
    /// channel or transparency, else 24; Save As asks, starting at [`tga_default_bits`].
    pub tga_bits: Option<TgaBits>,
    /// PSD/PSB: embed the thumbnail image resource (1036), Preferences › File Handling › Image
    /// Previews. On by default, as Photoshop's Always Save.
    pub image_previews: bool,
    /// PSD/PSB: write the real merged image, Preferences › File Handling › Maximize PSD and PSB
    /// File Compatibility (see [`PsdExportOptions::maximize_compatibility`]). On by default, as
    /// Photoshop's Always.
    pub psd_maximize_compatibility: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            encode: EncodeOptions::default(),
            force_psb: false,
            tiff_layers: false,
            xmp: XmpEmbed::All,
            tga_bits: None,
            image_previews: true,
            psd_maximize_compatibility: true,
        }
    }
}

/// Bits per pixel of an RGB Targa (Photoshop's Targa Options › Resolution).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TgaBits {
    /// The colours alone, no alpha.
    Bits24,
    /// The colours and an 8-bit alpha: the document's alpha channel, or its transparency.
    Bits32,
}

impl TgaBits {
    /// `24` or `32` as [`TgaBits`]; any other number is `None`.
    pub fn from_bits(bits: u64) -> Option<Self> {
        match bits {
            24 => Some(TgaBits::Bits24),
            32 => Some(TgaBits::Bits32),
            _ => None,
        }
    }

    /// The bits per pixel.
    pub fn bits(self) -> u8 {
        match self {
            TgaBits::Bits24 => 24,
            TgaBits::Bits32 => 32,
        }
    }
}

/// `true` if `bytes` start with the PSD/PSB signature.
pub fn is_psd(bytes: &[u8]) -> bool {
    bytes.starts_with(b"8BPS")
}

/// Imports a file. PSD/PSB, Affinity containers and camera raws are detected by magic; everything
/// else is decoded with `photocraft-codecs`.
pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth(name, bytes, photocraft_doc::MAX_GROUP_DEPTH)
}

/// [`import`] for a background open: checks `ctl` between stages (and per layer for PSD/PSB) and
/// reports progress. A cancelled import fails with [`IoError::Cancelled`].
pub fn import_with(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth_and_interrupt(name, bytes, photocraft_doc::MAX_GROUP_DEPTH, ctl)
}

/// Imports a file with a configurable SVG editability threshold. SVG groups deeper than this are
/// rasterised during conversion; the XML parser safety cap and document nesting limit stay fixed.
pub fn import_with_svg_group_depth(name: &str, bytes: &[u8], max_group_depth: usize) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth_and_interrupt(name, bytes, max_group_depth, &photocraft_raster::Interrupt::NONE)
}

/// [`import_with`] with the SVG editable-group threshold selected by the user.
pub fn import_with_svg_group_depth_and_interrupt(
    name: &str,
    bytes: &[u8],
    max_group_depth: usize,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut r = import_stages(name, bytes, ctl, max_group_depth)?;
    // A successful wrapper loader identifies the contained format. Outer compression
    // signatures alone cannot distinguish XCF from other wrapped formats such as SVGZ.
    if r.origin.format == ImportFormat::Unknown {
        r.origin = ImportOrigin::detected(bytes);
    }
    r.source_read_only |= r.origin.is_read_only();
    if r.source_read_only {r.origin.source_policy=SourcePolicy::ReadOnlySource;}
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(r)
}

fn import_stages(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt, max_group_depth: usize) -> Result<ImportResult, IoError> {
    if photocraft_xcf::classify_container(bytes).is_some()
        && let Some(result) = xcf_import::import_candidate(name, bytes, ctl)?
    {
        return Ok(result);
    }
    // A declared native extension must reach its loader so malformed bundles retain format errors.
    if has_extension(name, photocraft_format::EXTENSION) || photocraft_format::is_pcraft(bytes) {
        let document = photocraft_format::load_from_bytes_with_interrupt(bytes, &Default::default(), ctl)?;
        xcf_export::active_implicit_profile(&document).map_err(|e| IoError::Unsupported(format!("native working profile: {e}")))?;
        return Ok(ImportResult { document, warnings: Vec::new(), origin: ImportOrigin::writable(ImportFormat::Pcraft), assessment: None , source_read_only: (ImportOrigin::writable(ImportFormat::Pcraft)).is_read_only(), preview_only: false});
    }
    if is_psd(bytes) {
        let file = PsdFile::from_bytes(bytes)?;
        // Nesting past the document model's cap could never be saved; reject the file with the
        // actionable limit.
        psd_import::check_group_depth(&file)?;
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(0.05);
        let (mut document, warnings) = psd_import::try_psd_to_document_with(&file, ctl)?.ok_or(IoError::Cancelled)?;
        document.name = name.to_string();
        text_import::prepare(&mut document)?;
        return Ok(ImportResult { document, warnings, origin: ImportOrigin::detected(bytes), assessment: None , source_read_only: (ImportOrigin::detected(bytes)).is_read_only(), preview_only: false});
    }
    if affinity::is_affinity(bytes) || affinity::has_extension(name) {
        return affinity::import(name, bytes);
    }
    if ora::is_ora(bytes) || has_extension(name, "ora") {
        return ora::import(name, bytes, ctl);
    }
    // Krita files are ZIP containers. A recognized raster signature still wins over a
    // misleading .kra suffix (as with SVG imports).
    if kra::is_kra(bytes) || (kra::EXTENSIONS.iter().any(|e| has_extension(name, e)) && photocraft_codecs::detect(bytes).is_none()) {
        return kra::import(name, bytes, ctl);
    }
    if has_extension(name, "pdn") || bytes.starts_with(b"PDN3") {
        return pdn::import(name, bytes, ctl);
    }
    if raw::is_raw(bytes) {
        return raw::import_raw(name, bytes);
    }
    if is_svg_input(name, bytes) {
        return svg::import_svg_with_group_depth(name, bytes, max_group_depth);
    }
    if is_xcf_filename(name) {
        return xcf_import::import(name, bytes, ctl);
    }
    flat::import_flat(name, bytes)
}

/// Extension names can be wrong (for example, an image downloaded as WebP and renamed `.svg`).
/// Valid plain SVG is recognized by its XML content; an otherwise unknown `.svg` is passed to the
/// SVG parser for a useful malformed-file error, and `.svgz` disambiguates gzip streams.
fn is_svg_input(name: &str, bytes: &[u8]) -> bool {
    // Keep the SVG extension as a fallback for malformed files so the SVG parser can return an
    // actionable SVG error. A recognized raster signature still wins over a misleading `.svg`
    // suffix (for example, a WebP downloaded as Firefox_logo.svg).
    svg::is_svg(bytes)
        || (has_extension(name, "svg") && photocraft_codecs::detect(bytes).is_none())
        || (has_extension(name, "svgz") && bytes.starts_with(&[0x1f, 0x8b]))
}

fn extension(name_or_ext: &str) -> String {
    name_or_ext.rsplit(['.', '/', '\\']).next().unwrap_or(name_or_ext).to_ascii_lowercase()
}

fn has_extension(name: &str, expected: &str) -> bool {
    name.rsplit(['/', '\\']).next().and_then(|name| name.rsplit_once('.')).is_some_and(|(base, ext)| !base.is_empty() && ext.eq_ignore_ascii_case(expected))
}

/// Plain and supported compound XCF suffixes, including a bare format name.
/// This is a destination/name hint; only decoded content establishes origin.
pub fn is_xcf_filename(name: &str) -> bool {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    leaf == "xcf"
        || leaf.ends_with(".xcf")
        || ["gz", "bz2", "xz"].iter().any(|suffix| leaf == format!("xcf.{suffix}") || leaf.ends_with(&format!(".xcf.{suffix}")))
}

/// The document as it is written to `name` (a file name, path or bare extension): a copy whose
/// name is that file's own, so a `.pcraft` manifest reports the file it lives in — the name the
/// session adopts on the save that wrote it, and the name every other format gives the document
/// when it is opened (#3312). A bare extension ("pcraft") names no file and changes nothing.
pub fn named_for_file<'a>(doc: &'a Document, name: &str) -> std::borrow::Cow<'a, Document> {
    let file = name.contains(['/', '\\', '.']).then(|| name.rsplit(['/', '\\']).next().unwrap_or(name));
    match file {
        Some(file) if !file.is_empty() && doc.name != file => {
            let mut owned = (*doc).clone();
            owned.name = file.to_string();
            std::borrow::Cow::Owned(owned)
        }
        _ => std::borrow::Cow::Borrowed(doc),
    }
}

/// Exports `doc` to the format named by `name_or_ext` (a file name, path or
/// bare extension).
pub fn export(doc: &Document, name_or_ext: &str, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    export_with(doc, name_or_ext, opts, &photocraft_raster::Interrupt::NONE)
}

/// [`export`] for a background save (#3389): reports how far encoding has got through `ctl`
/// (monotonic, `1.0` once the bytes are ready; per layer and per composite band for PSD/PSB and
/// layered TIFF, in coarse steps for the other formats) and checks it between stages. A cancelled
/// export fails with [`IoError::Cancelled`]; otherwise the bytes are exactly [`export`]'s.
pub fn export_with(doc: &Document, name_or_ext: &str, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ExportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let r = export_stages(doc, name_or_ext, opts, ctl)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(r)
}

/// Run `f` with an interrupt whose progress `0..=1` fills `lo..=hi` of `ctl`'s (same cancellation).
pub(crate) fn with_span<R>(ctl: &photocraft_raster::Interrupt, lo: f32, hi: f32, f: impl FnOnce(&photocraft_raster::Interrupt) -> R) -> R {
    let cancelled = || ctl.cancelled();
    let progress = |p: f32| ctl.progress(lo + (hi - lo) * p.clamp(0.0, 1.0));
    f(&photocraft_raster::Interrupt::new(&cancelled, &progress))
}

/// Where a .pcraft save's progress stands once its thumbnail, then its 1024 px composite preview,
/// are rendered (the bundle itself is written after them).


/// The share of a PSD/PSB export spent converting the document (layers, then the composite)
/// before its bytes are assembled: about 2.7 s of 3.0 s for a 24 MP, 5-layer RGB/8 document.
const PSD_MODEL_DONE: f32 = 0.9;

fn export_stages(doc: &Document, name_or_ext: &str, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ExportResult, IoError> {
    xcf_export::active_implicit_profile(doc).map_err(|e| IoError::Unsupported(format!("working profile cannot be exported: {e}")))?;
    let ext = extension(name_or_ext);
    if is_xcf_filename(name_or_ext) {
        return Err(IoError::Unsupported("XCF is an import-only format; choose a writable destination such as .pcraft".into()));
    }
    if ext == "pdn" {
        return Err(IoError::Unsupported("PDN is import-only; save as .pcraft to preserve all layers and blend modes".into()));
    }
    if affinity::EXTENSIONS.contains(&ext.as_str()) {
        return Err(IoError::Unsupported("Affinity export is not implemented; save a new PSD, PNG or .pcraft copy".into()));
    }
    if kra::EXTENSIONS.contains(&ext.as_str()) {
        return Err(IoError::Unsupported("Krita files are import-only; save as .ora to open the layers in Krita, or .pcraft to keep everything".into()));
    }
    if ext == "ora" {
        return ora::export(doc);
    }
    if ext == photocraft_format::EXTENSION {
        let mut limits=NativeSaveLimits::default();
        let leaf=name_or_ext.contains(['/', '\\', '.']).then(|| name_or_ext.rsplit(['/', '\\']).next().unwrap_or(name_or_ext));
        if leaf.is_some_and(|name| !name.is_empty() && name!=doc.name) {
            // The upstream filename-name behavior needs a private snapshot. Admit
            // its known model/map copies before cloning, retaining shared payloads.
            let retained=retained_native_input_bytes(doc,limits.preparation,ctl)?;
            let model=retained_native_model_bytes(doc,limits.preparation,ctl)?;
            let maps=native_save::retained_native_map_bytes(doc,limits.preparation,ctl)?;
            let copy=model.checked_add(maps)
                .and_then(|n|n.checked_add(name_or_ext.len())).ok_or_else(||IoError::Unsupported("native name snapshot size overflow".into()))?;
            if retained.checked_add(copy).is_none_or(|n|n>limits.preparation.resident_bytes) {
                return Err(IoError::Unsupported("native name snapshot exceeds standard preparation budget".into()));
            }
            limits.preparation.resident_bytes=limits.preparation.resident_bytes.checked_sub(copy)
                .ok_or_else(||IoError::Unsupported("native name snapshot exceeds standard preparation budget".into()))?;
            ctl.check().map_err(|_|IoError::Cancelled)?;
        }
        let named = named_for_file(doc, name_or_ext);
        let (bytes, result) = save_native_bytes(named.as_ref(), limits, ctl)?;
        let mut warnings=result.warnings;
        warnings.extend(xcf_export_loss::warnings_for(doc,xcf_export_loss::Destination::Native));
        return Ok(ExportResult { bytes, warnings });
    }
    if ext == "psd" || ext == "psb" {
        let precision = xcf_export_precision::preflight(doc, if opts.force_psb || ext == "psb" { xcf_export_precision::Destination::Psb } else { xcf_export_precision::Destination::Psd })?;
        require_layered_filter_export(doc)?;
        xcf_export::effective_icc(doc).map_err(|error| IoError::Unsupported(format!("cannot represent imported sample interpretation in PSD: {error}")))?;
        let o = PsdExportOptions { force_psb: opts.force_psb || ext == "psb", maximize_compatibility: opts.psd_maximize_compatibility, ..Default::default() };
        // The checked ink serializer converts from exact original samples once,
        // including PSD's inverted ink encoding. Pre-staging here would round first.
        let converted = if doc.mode == photocraft_doc::ColorMode::Multichannel { None } else { wide_export::psd_document(doc)? };
        let (mut file, mut warnings) = with_span(ctl, 0.0, PSD_MODEL_DONE, |ctl| psd_export::document_to_psd_interruptible(converted.as_ref().unwrap_or(doc), &o, ctl))?;
        if converted.is_some() {
            warnings.push("PSD export converts U32/F64 samples to F32; save as .pcraft to retain exact source samples".into());
        }
        if !doc.metadata.text.is_empty() { warnings.push("free-form text metadata is not supported by PSD/PSB; it will be dropped".into()); }
        if opts.image_previews {
            ctl.check().map_err(|_|IoError::Cancelled)?;
            match psd_thumbnail::resource(doc) {
                Ok(resource) => {let at=file.resources.iter().position(|r|r.id==photocraft_psd::resources::ids::VERSION_INFO).unwrap_or(file.resources.len());file.resources.insert(at,resource);},
                Err(error) => warnings.push(format!("image preview not saved: {error}")),
            }
        }
        warnings.extend(tiff_layers::strip_foreign_order_blocks(&mut file));
        warnings.extend(import_state_export_warnings(doc));
        warnings.extend(precision);
        // Never write a header the reader would refuse (e.g. a zero-sized canvas).
        file.header.validate()?;
        let bytes = file.to_bytes()?;
        return Ok(ExportResult { bytes, warnings });
    }
    let format = photocraft_codecs::from_extension(&ext).ok_or_else(|| IoError::UnknownFormat(name_or_ext.to_string()))?;
    let layered = format == photocraft_codecs::Format::Tiff && opts.tiff_layers && tiff_layers::would_write_layers(doc);
    let precision = xcf_export_precision::preflight(doc, if layered { xcf_export_precision::Destination::LayeredTiff } else { xcf_export_precision::Destination::Flat(format) })?;
    if layered {
        require_layered_filter_export(doc)?;
        let mut result = tiff_layers::export_layered(doc, opts, ctl)?;
        result.warnings.extend(xcf_export_loss::warnings_for(doc,if tiff_layers::layered_mode(doc){xcf_export_loss::Destination::Layered}else{xcf_export_loss::Destination::Flat}));
        result.warnings.extend(precision);
        return Ok(result);
    }
    let mut result = flat::export_flat_with(doc, format, opts, ctl)?;
    result.warnings.extend(xcf_export_loss::warnings_for(doc, xcf_export_loss::Destination::Flat));
    result.warnings.extend(precision);
    Ok(result)
}

fn has_enabled_xcf_filters(doc: &Document) -> bool {
    doc.xcf_state.as_ref().is_some_and(|state| state.filter_stacks.iter().any(|stack| stack.nodes.iter().any(|node| node.enabled)))
}

fn require_layered_filter_export(doc: &Document) -> Result<(), IoError> {
    if has_enabled_xcf_filters(doc) {
        return Err(IoError::Unsupported("Layered export cannot yet preserve live GIMP filters; save as .pcraft or export a flat image".into()));
    }
    Ok(())
}

#[cfg(test)]
fn native_previews(doc: &Document) -> (photocraft_format::SaveOptions, Vec<String>) {
    if (doc.sample_encoding.is_some() || matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64) || has_enabled_xcf_filters(doc))
        && matches!(doc.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale)
    {
        let result = (|| {
            let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(doc).map_err(|e| e.to_string())?;
            let profile = xcf_export::effective_icc(doc).map_err(|e| e.to_string())?;
            let source = match xcf_export::active_implicit_profile(doc).map_err(|e| e.to_string())? {
                Some(profile) => profile,
                None => match profile {
                    Some(bytes) => photocraft_cms::Profile::parse(&bytes).map_err(|e| e.to_string())?,
                    None if doc.mode == photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray.profile().clone(),
                    None => photocraft_cms::Builtin::Srgb.profile().clone(),
                },
            };
            let display = photocraft_cms::Transform::new(&source, photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::Intent::RelativeColorimetric, true)
                .map_err(|e| e.to_string())?;
            let preview = |side: u32| {
                let (fw, fh) = (doc.size.width, doc.size.height);
                let longest = fw.max(fh).max(1);
                let w = (u64::from(fw) * u64::from(side.min(longest)) / u64::from(longest)).max(1) as u32;
                let h = (u64::from(fh) * u64::from(side.min(longest)) / u64::from(longest)).max(1) as u32;
                let mut buffer = renderer.render_reduced(w, h, None, &mut |_| Ok(())).map_err(|e| e.to_string())?;
                for pixel in &mut buffer.px {
                    let input = [f64::from(pixel[0]), f64::from(pixel[1]), f64::from(pixel[2])];
                    let mut output = [0.0; 3];
                    display
                        .try_eval_f64(if doc.mode == photocraft_doc::ColorMode::Grayscale { &input[..1] } else { &input }, &mut output)
                        .map_err(|e| e.to_string())?;
                    for (target, value) in pixel.iter_mut().take(3).zip(output) {
                        *target = value.clamp(0.0, 1.0) as f32;
                    }
                }
                Ok::<_, String>(buffer.to_rgba8())
            };
            Ok::<_, String>(photocraft_format::SaveOptions { thumbnail: Some(preview(256)?), composite: Some(preview(1024)?) })
        })();
        return match result {
            Ok(previews) => (previews, Vec::new()),
            Err(error) => (Default::default(), vec![format!("Native source state was saved without optional previews: {error}")]),
        };
    }
    (
        photocraft_format::SaveOptions {
            thumbnail: Some(photocraft_compose::thumbnail(doc, 256).unwrap()),
            composite: Some(photocraft_compose::thumbnail(doc, 1024).unwrap()),
        },
        Vec::new(),
    )
}

fn native_previews_with_interrupt(
    doc: &Document,
    control: &photocraft_raster::Interrupt<'_>,
    resident: usize,
) -> Result<(photocraft_format::SaveOptions, Vec<String>), IoError> {
    if control.cancelled() {
        return Err(IoError::Cancelled);
    }
    if photocraft_compose::gimp_render::PreparedRenderer::requires_preparation_budget(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_plain_normal_budgeted(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_plain_point_budgeted(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_plain_box_budgeted(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_plain_motion_budgeted(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_plain_median_budgeted(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_plain_gaussian_budgeted(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_plain_brightness_contrast_budgeted(doc)
        || photocraft_compose::gimp_render::PreparedRenderer::supports_accounted_scalar_preparation(doc)
    {
        return Ok((native_scene_preview::previews(doc, control, resident)?, Vec::new()));
    }
    // Qualified ordinary previews retain preparation and both returned images
    // in one per-call ledger. Other styles retain the legacy route below; this
    // accounts known payloads, not allocator overhead or total process RSS.
    let plain = doc.sample_encoding.is_none()
        && !matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64)
        && photocraft_compose::preview_budget::plain_preview_supported(doc);
    let styled = photocraft_compose::styled_preview::styled_preview_supported(doc);
    let proxy = doc.size.area() > photocraft_compose::PROXY_THUMBNAIL_PIXELS && photocraft_compose::proxy_preview::proxy_preview_supported(doc);
    if plain || styled || proxy {
        use photocraft_compose::preview_budget::{PreviewPreparationBudget, thumbnail_with_preparation_budget};
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(resident, &observer);
        let map_error = |e| match e {
            photocraft_raster::RasterInputError::Cancelled => IoError::Cancelled,
            photocraft_raster::RasterInputError::LimitExceeded => {
                photocraft_format::FormatError::LimitExceeded("native checked preview workspace".into()).into()
            }
            other => IoError::Unsupported(other.to_string()),
        };
        let preview = |side| {
            if plain {
                thumbnail_with_preparation_budget(doc, side, control, &budget)
            } else if proxy {
                photocraft_compose::proxy_preview::thumbnail_with_proxy_preparation_budget(doc, side, control, &budget)
            } else {
                photocraft_compose::styled_preview::thumbnail_with_styled_preparation_budget(doc, side, control, &budget)
            }
        };
        let thumbnail = preview(256).map_err(map_error)?;
        let composite = preview(1024).map_err(map_error)?;
        control.check().map_err(|_| IoError::Cancelled)?;
        // NativeSave immediately measures the returned buffers and carries that
        // charge into archive preparation. No allocating work intervenes here.
        return Ok((photocraft_format::SaveOptions { thumbnail: Some(thumbnail.into_image()), composite: Some(composite.into_image()) }, Vec::new()));
    }
    // Charge output buffers and the legacy band workspace before starting any preview allocation.
    let output = (256usize * 256 + 1024 * 1024) * 24;
    let maps = (doc.size.width as usize)
        .checked_add(doc.size.height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| IoError::Unsupported("native preview size overflow".into()))?;
    let legacy_rows = doc.size.height.min(photocraft_compose::band_rows_for(doc.size.width, 0) as u32);
    let legacy_band = (doc.size.width as usize)
        .checked_mul(legacy_rows as usize)
        .and_then(|v| v.checked_mul(32))
        .ok_or_else(|| IoError::Unsupported("native preview band overflow".into()))?;
    let checked = (doc.sample_encoding.is_some()
        || matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64)
        || has_enabled_xcf_filters(doc))
        && matches!(doc.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale);
    let reserve = if checked {
        output
    } else {
        output
            .checked_add(maps)
            .and_then(|v| v.checked_add(legacy_band))
            .and_then(|v| v.checked_add(64 << 20))
            .ok_or_else(|| IoError::Unsupported("native preview resident overflow".into()))?
    };
    if reserve > resident {
        return Err(photocraft_format::FormatError::LimitExceeded("native preview workspace".into()).into());
    }
    let denied = std::cell::Cell::new(false);
    if (doc.sample_encoding.is_some() || matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64) || has_enabled_xcf_filters(doc))
        && matches!(doc.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale)
    {
        let result = (|| {
            let renderer = photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control).map_err(|e| e.to_string())?;
            let profile = xcf_export::effective_icc(doc).map_err(|e| e.to_string())?;
            let source = match xcf_export::active_implicit_profile(doc).map_err(|e| e.to_string())? {
                Some(profile) => profile,
                None => match profile {
                    Some(bytes) => photocraft_cms::Profile::parse(&bytes).map_err(|e| e.to_string())?,
                    None if doc.mode == photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray.profile().clone(),
                    None => photocraft_cms::Builtin::Srgb.profile().clone(),
                },
            };
            let display = photocraft_cms::Transform::new(&source, photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::Intent::RelativeColorimetric, true)
                .map_err(|e| e.to_string())?;
            let preview = |side: u32| {
                if control.cancelled() {
                    return Err("cancelled".to_string());
                }
                let mut used = 0u64;
                let mut budget = |work| {
                    use photocraft_cms::WideWork;
                    match work {
                        WideWork::ReserveBytes(bytes) => {
                            used = used
                                .checked_add(bytes)
                                .ok_or(photocraft_compose::gimp_composite::CompositionError::Invalid("native preview budget overflow"))?;
                            if used > (resident - reserve) as u64 {
                                denied.set(true);
                                return Err(photocraft_compose::gimp_composite::CompositionError::Invalid("native preview budget"));
                            }
                        }
                        WideWork::ReleaseBytes(bytes) => {
                            used = used.saturating_sub(bytes);
                        }
                        WideWork::SampleUnits(_) => {}
                    }
                    if control.cancelled() {
                        return Err(photocraft_compose::gimp_composite::CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled));
                    }
                    Ok(())
                };
                let (fw, fh) = (doc.size.width, doc.size.height);
                let longest = fw.max(fh).max(1);
                let w = (u64::from(fw) * u64::from(side.min(longest)) / u64::from(longest)).max(1) as u32;
                let h = (u64::from(fh) * u64::from(side.min(longest)) / u64::from(longest)).max(1) as u32;
                let mut buffer = renderer.render_reduced_with_interrupt(w, h, None, &mut budget, control).map_err(|e| e.to_string())?;
                for pixel in &mut buffer.px {
                    if control.cancelled() {
                        return Err("cancelled".to_string());
                    }
                    let input = [f64::from(pixel[0]), f64::from(pixel[1]), f64::from(pixel[2])];
                    let mut output = [0.0; 3];
                    display
                        .try_eval_f64(if doc.mode == photocraft_doc::ColorMode::Grayscale { &input[..1] } else { &input }, &mut output)
                        .map_err(|e| e.to_string())?;
                    for (target, value) in pixel.iter_mut().take(3).zip(output) {
                        *target = value.clamp(0.0, 1.0) as f32;
                    }
                }
                Ok::<_, String>(buffer.to_rgba8())
            };
            Ok::<_, String>(photocraft_format::SaveOptions { thumbnail: Some(preview(256)?), composite: Some(preview(1024)?) })
        })();
        if control.cancelled() {
            return Err(IoError::Cancelled);
        }
        if denied.get() {
            return Err(photocraft_format::FormatError::LimitExceeded("native preview renderer resident bytes".into()).into());
        }
        return Ok(match result {
            Ok(previews) => (previews, Vec::new()),
            Err(error) => (Default::default(), vec![format!("Native source state was saved without optional previews: {error}")]),
        });
    }
    Ok((
        photocraft_format::SaveOptions {
            thumbnail: Some(
                photocraft_compose::thumbnail_with_interrupt(doc, 256, control)
                    .map_err(|e| if e == photocraft_raster::RasterInputError::Cancelled { IoError::Cancelled } else { IoError::Unsupported(e.to_string()) })?,
            ),
            composite: Some(
                photocraft_compose::thumbnail_with_interrupt(doc, 1024, control)
                    .map_err(|e| if e == photocraft_raster::RasterInputError::Cancelled { IoError::Cancelled } else { IoError::Unsupported(e.to_string()) })?,
            ),
        },
        Vec::new(),
    ))
}

fn import_state_export_warnings(doc: &Document) -> Vec<String> {
    xcf_export_loss::warnings(doc)
}

/// The PSD's merged composite as straight RGBA floats (row-major), converted
/// with the document pixel model and un-matted from white (Photoshop mattes
/// the merged image of transparent documents). Used as the compositing oracle.
pub fn merged_composite(file: &PsdFile) -> Result<Vec<[f32; 4]>, IoError> {
    let h = &file.header;
    h.validate()?;
    if h.color_mode == photocraft_psd::ColorMode::Rgb && file.merged_has_alpha() {
        // Preserve native sample precision before removing the white matte.
        let raw = file.decode_merged()?;
        let plane = raw.len() / usize::from(h.channels);
        if plane == 0 {
            return Err(PsdError::Invalid("empty RGB composite".into()).into());
        }
        let planes = raw
            .chunks_exact(plane)
            .take(4)
            .map(|bytes| match h.depth {
                32 => Ok(photocraft_psd::pixels::samples_f32(bytes)),
                16 => Ok(photocraft_psd::pixels::samples_u16(bytes).into_iter().map(|v| f32::from(v) / 65535.0).collect()),
                _ => photocraft_psd::pixels::plane_to_u8(bytes, h.depth, h.width as usize, h.height as usize)
                    .map(|values| values.into_iter().map(|v| f32::from(v) / 255.0).collect()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let [r, g, b, a] = planes.as_slice() else { return Err(PsdError::Invalid("missing RGB composite channels".into()).into()) };
        return Ok(r
            .iter()
            .zip(g)
            .zip(b)
            .zip(a)
            .map(|(((&r, &g), &b), &a)| {
                let mut pixel = [r, g, b, a];
                let alpha = pixel[3];
                for c in pixel.iter_mut().take(3) {
                    *c = pixels::unmatte(*c, alpha, 1.0);
                    if h.depth != 32 {
                        *c = c.clamp(0.0, 1.0);
                    }
                }
                pixel
            })
            .collect());
    }
    // The RGBA8 preview clips signed/HDR values. Decode float merged images at their
    // original depth through the model path below; they are the rendering oracle.
    let img = if h.depth == 32 { None } else { file.composite_rgba8().ok() };
    // CMYK goes through the colour-managed model conversion (the PSD crate's RGBA preview is a
    // naive, profile-free conversion).
    if let (Some(img), false) = (img, matches!(h.color_mode, photocraft_psd::ColorMode::Lab | photocraft_psd::ColorMode::Cmyk)) {
        let unmatte = file.merged_has_alpha();
        return Ok(img
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let mut v: [f32; 4] = std::array::from_fn(|i| f32::from(p[i]) / 255.0);
                if unmatte {
                    for c in 0..3 {
                        v[c] = pixels::unmatte(v[c], v[3], 1.0).clamp(0.0, 1.0);
                    }
                }
                v
            })
            .collect());
    }
    // Generic path (Lab, CMYK and others) via the raster model conversion.
    // Keep the merged-alpha marker while discarding layers: otherwise an extra
    // spot/alpha channel in an opaque composite is mistaken for transparency.
    let layer_info = Some(photocraft_psd::LayerInfo { merged_alpha: file.merged_has_alpha(), layers: Vec::new(), padding: None });
    let (doc, _) = psd_to_document(&PsdFile { layer_info, ..file.clone() });
    // Multichannel documents keep their channels apart (no layer): composite them.
    if doc.layers.is_empty() && doc.mode == photocraft_color::ColorMode::Multichannel {
        return Ok(photocraft_compose::flatten(&doc)?.px);
    }
    let l = doc.layers.first().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    let s = l.surface().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    // CMYK reads through the file's own profile, as the compositor reads its layers, so a tagged
    // file's merged image and its composite are compared in the same colours.
    let cmyk = photocraft_compose::cmyk_space(&doc);
    Ok(photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || photocraft_compose::surface_to_buffer(s, doc.bounds()).px))
}

#[cfg(test)]
mod xcf_export_tests {
    use super::*;

    #[test]
    fn imported_linear_native_previews_are_color_managed_without_changing_samples() {
        use photocraft_doc::{ColorMode, Layer, SampleEncoding, SampleType, Size};
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let mut doc = Document::new("linear", Size::new(1, 1), mode, SampleType::F64);
            doc.sample_encoding = Some(SampleEncoding::Linear);
            doc.icc_profile = xcf_export::effective_icc(&doc).unwrap().map(std::sync::Arc::new);
            let mut layer = Layer::raster("source", doc.pixel_format());
            let components: &[f64] = if mode == ColorMode::Rgb { &[0.25, 0.25, 0.25, 1.0] } else { &[0.25, 1.0] };
            let bytes: Vec<_> = components.iter().flat_map(|v| v.to_ne_bytes()).collect();
            *layer.surface_mut().unwrap() = photocraft_raster::Surface::try_from_interleaved(doc.pixel_format(), doc.bounds(), &bytes).unwrap();
            doc.layers.push(layer);
            let before = doc.clone();
            let (options, warnings) = native_previews(&doc);
            assert!(warnings.is_empty(), "{warnings:?}");
            let thumbnail = options.thumbnail.unwrap();
            assert_eq!(thumbnail.pixels, [137, 137, 137, 255]);
            assert_eq!(doc, before);
        }
    }

    #[test]
    fn xcf_exports_are_explicitly_unsupported() {
        let doc = Document::new("test", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        assert!(matches!(export(&doc, "out.XCF", &ExportOptions::default()), Err(IoError::Unsupported(_))));
    }

    #[test]
    fn source_interpretation_loss_is_named_and_native_retains_it() {
        let mut doc = Document::new("test", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        assert!(import_state_export_warnings(&doc).is_empty());
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear);
        doc.icc_profile = xcf_export::effective_icc(&doc).unwrap().map(std::sync::Arc::new);
        assert!(import_state_export_warnings(&doc).iter().any(|warning| warning.contains("GIMP sample interpretation")));
        assert!(export(&doc, "pcraft", &ExportOptions::default()).unwrap().warnings.is_empty());
    }

    #[test]
    fn interpretation_diagnostics_bound_unicode_layer_names() {
        let mut doc = Document::new("test", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut layer = photocraft_doc::Layer::raster("🎨".repeat(10000), doc.pixel_format());
        layer.import_compositing = Some(photocraft_doc::ImportCompositing {
            blend_encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
            composite_encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
            pass_through: false,
        });
        doc.layers.push(layer);
        let warnings = import_state_export_warnings(&doc);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains('…'));
        assert!(warnings[0].len() < 1024);
    }
}

#[cfg(test)]
mod tests {
    use super::is_svg_input;

    #[test]
    fn svg_extension_does_not_override_another_format_signature() {
        let webp_header = b"RIFF\x04\0\0\0WEBP";
        assert!(!is_svg_input("Firefox_logo,_2019.svg", webp_header));
        assert!(is_svg_input("drawing.svg", b"<?xml version=\"1.0\"?><svg/>"));
        assert!(is_svg_input("drawing.svgz", &[0x1f, 0x8b, 0, 0]));
        assert!(!is_svg_input("drawing.svgz", webp_header));
    }
}
