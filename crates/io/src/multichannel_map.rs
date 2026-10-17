//! Multichannel documents <-> PSD colour mode 7.
//!
//! A Multichannel document has no layers: its image is a set of ink (spot) channels, stored in
//! [`Document::channels`] with ink density as the value (1 = solid ink, like every spot channel).
//! A PSD stores the same planes the way Photoshop shows them in the Channels panel: 0 (black) =
//! solid ink, max = paper. Channel names come from resources 1006 / 1045 and ink colours from
//! DisplayInfo (1077), exactly as for spot channels in the other modes.

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document};
use photocraft_geom::Rect;
use photocraft_psd::file::{GlobalLayerMask, LayerInfoPlacement};
use photocraft_psd::resources::{ImageResource, ResolutionInfo, ids, version_info_resource};
use photocraft_psd::{ColorMode as PsdMode, Compression, Header, ImageData, PsdFile, TaggedBlock};
use photocraft_raster::Surface;

use crate::pixels::psd_depth;

/// Grid and guides.
const GRID_AND_GUIDES: u16 = 1032;
/// Alpha identifiers: one u32 per extra channel.
const ALPHA_IDENTIFIERS: u16 = 1053;
/// Resources that only describe layers: target layer (1024), layer groups (1026), layer
/// selection ids (1069) and layer groups enabled (1072). A Multichannel document has no layers.
const LAYER_ONLY_RESOURCES: [u16; 4] = [ids::LAYER_STATE, ids::LAYER_GROUP_INFO, 1069, 1072];

/// Value of sample `i` of a big-endian plane.
fn decode(plane: &[u8], i: usize, s: SampleType) -> f32 {
    match s {
        SampleType::U8 => f32::from(plane[i]) / 255.0,
        SampleType::U16 => f32::from(u16::from_be_bytes([plane[2 * i], plane[2 * i + 1]])) / 65535.0,
        SampleType::F32 => f32::from_be_bytes([plane[4 * i], plane[4 * i + 1], plane[4 * i + 2], plane[4 * i + 3]]),
        SampleType::U32 => plane
            .get(i.saturating_mul(4)..i.saturating_mul(4).saturating_add(4))
            .and_then(|p| <[u8; 4]>::try_from(p).ok())
            .map_or(0.0, |p| u32::from_be_bytes(p) as f32 / u32::MAX as f32),
        SampleType::F64 => plane
            .get(i.saturating_mul(8)..i.saturating_mul(8).saturating_add(8))
            .and_then(|p| <[u8; 8]>::try_from(p).ok())
            .map_or(0.0, |p| f64::from_be_bytes(p) as f32),
    }
}

/// Fills `doc` (already in Multichannel mode at the file depth) from the merged planes `all`.
pub(crate) fn import(file: &PsdFile, all: &[u8], doc: &mut Document, names: &[String], warnings: &mut Vec<String>) {
    let h = &file.header;
    let depth = doc.depth;
    let (w, hh) = (h.width as usize, h.height as usize);
    let n = w * hh;
    let plane = h.row_bytes() * hh;
    let canvas = Rect::new(0, 0, h.width as i32, h.height as i32);
    let fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    for k in 0..usize::from(h.channels) {
        let Some(p) = all.get(k * plane..(k + 1) * plane) else {
            warnings.push(format!("multichannel: channel {} is missing", k + 1));
            break;
        };
        let vals: Vec<f32> = (0..n).map(|i| 1.0 - decode(p, i, depth)).collect();
        let mut s = Surface::new(fmt);
        s.write_region(canvas, &vals);
        s.prune();
        let name = names.get(k).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Channel {}", k + 1));
        // Without DisplayInfo the channels print in black, as Photoshop shows them. DisplayInfo
        // turns alpha-type channels (selected / masked areas) back into alpha channels.
        doc.channels.push(AlphaChannel { spot: Some((Color::BLACK, 0.0)), ..AlphaChannel::new(name, s) });
    }
    if let Some(r) = file.resource(crate::channel_map::DISPLAY_INFO) {
        crate::channel_map::apply_display_info(&r.data, true, 0, &mut doc.channels);
    } else if let Some(r) = file.resource(crate::channel_map::DISPLAY_INFO_OLD) {
        crate::channel_map::apply_display_info(&r.data, false, 0, &mut doc.channels);
    }
}

/// Authoritative ink-density export. Invert the original f64 value before the
/// single destination quantization; preconverting the channel changes half-code ties.
pub(crate) fn try_document_to_psd_with_interrupt(
    doc: &Document,
    force_psb: bool,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<(PsdFile, Vec<String>), crate::IoError> {
    let invalid = |message: &str| crate::IoError::Unsupported(format!("Multichannel PSD export: {message}"));
    if interrupt.cancelled() {
        return Err(crate::IoError::Cancelled);
    }
    if doc.mode != ColorMode::Multichannel || !doc.layers.is_empty() || doc.channels.len() > 56 {
        return Err(invalid("requires a layerless Multichannel document with at most 56 channels"));
    }
    if doc.sample_encoding.is_some() || !matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc) {
        return Err(invalid("color-profile interpretation of numeric ink channels is unsupported"));
    }
    let sample = match doc.depth {
        SampleType::U8 => SampleType::U8,
        _ => SampleType::U16,
    };
    let header =
        Header::new(photocraft_psd::Version::Psb, doc.size.width, doc.size.height, doc.channels.len().max(1) as u16, psd_depth(sample), PsdMode::Multichannel);
    header.validate()?;
    let count = u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(|| invalid("size overflow"))?;
    let len =
        count.checked_mul(doc.channels.len().max(1) as u64).and_then(|n| n.checked_mul(sample.bytes() as u64)).ok_or_else(|| invalid("plane size overflow"))?;
    // Simultaneous source, raw planes, encoder output/workspace and resource metadata.
    let mut peak = len.checked_mul(4).ok_or_else(|| invalid("allocation budget overflow"))?;
    let mut charge = |bytes: u64| -> Result<(), crate::IoError> {
        peak = peak.checked_add(bytes).ok_or_else(|| invalid("allocation budget overflow"))?;
        if peak > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
            return Err(invalid("allocation budget exceeded"));
        }
        Ok(())
    };
    charge(0)?;
    if let Some(bytes) = &doc.icc_profile {
        charge((bytes.len() as u64).checked_mul(4).and_then(|n| n.checked_add(512)).ok_or_else(|| invalid("inert ICC allocation budget overflow"))?)?;
    }
    if let Some(xmp) = &doc.metadata.xmp {
        charge(xmp.len() as u64)?;
    }
    let mut channels = Vec::new();
    channels.try_reserve_exact(doc.channels.len()).map_err(|_| invalid("channel allocation failed"))?;
    for ch in &doc.channels {
        if ch.surface.format().mode != ColorMode::Grayscale || ch.surface.channels() != 1 {
            return Err(invalid("each ink channel must be one-component Gray without alpha"));
        }
        ch.surface.try_validate_tile_origins_with(interrupt).map_err(|e| invalid(&e.to_string()))?;
        charge(ch.surface.resident_payload_bytes().map_err(|e| invalid(&e.to_string()))?)?;
        charge((ch.name.len() as u64).checked_mul(8).ok_or_else(|| invalid("name size overflow"))?)?;
        charge((ch.surface.tile_count() as u64).checked_mul(256).ok_or_else(|| invalid("tile accounting overflow"))?)?;
        channels.push(ch);
    }
    let mut planes = Vec::new();
    planes.try_reserve_exact(usize::try_from(len).map_err(|_| invalid("plane size exceeds address space"))?).map_err(|_| invalid("plane allocation failed"))?;
    let mut pixel = [0.0];
    let mut encoded = [0u8; 2];
    for plane in 0..channels.len().max(1) {
        for y in 0..doc.size.height {
            for x in 0..doc.size.width {
                if x.is_multiple_of(256) && interrupt.cancelled() {
                    return Err(crate::IoError::Cancelled);
                }
                let density = if let Some(ch) = channels.get(plane) {
                    ch.surface.try_read_pixel_f64(x as i32, y as i32, &mut pixel).map_err(|e| invalid(&e.to_string()))?;
                    *pixel.first().ok_or_else(|| invalid("missing sample"))?
                } else {
                    0.0
                };
                // Ink density is a bounded normalized numeric channel, not an HDR color.
                if !density.is_finite() || !(0.0..=1.0).contains(&density) {
                    return Err(invalid("ink density must be finite and in 0..=1"));
                }
                photocraft_color::try_write_sample_f64(&mut encoded, sample, 0, 1.0 - density).map_err(|e| invalid(&e.to_string()))?;
                if sample == SampleType::U8 {
                    planes.push(*encoded.first().ok_or_else(|| invalid("missing encoded sample"))?);
                } else {
                    planes.extend_from_slice(&u16::from_ne_bytes(encoded).to_be_bytes());
                }
            }
        }
    }
    if interrupt.cancelled() {
        return Err(crate::IoError::Cancelled);
    }
    let mut warnings = Vec::new();
    if channels.is_empty() {
        warnings.push("PSD requires an image channel; the empty Multichannel document is written with one paper channel".into());
    }
    if !matches!(doc.depth, SampleType::U8 | SampleType::U16) || channels.iter().any(|ch| ch.surface.format().sample != sample) {
        warnings.push(format!(
            "Multichannel ink densities are quantized once to {} bits/channel after inversion; save as .pcraft to retain exact native channel samples",
            psd_depth(sample)
        ));
    }
    let inert_icc = if let Some(bytes) = &doc.icc_profile {
        let mut owned = Vec::new();
        owned.try_reserve_exact(bytes.len()).map_err(|_| invalid("inert ICC allocation failed"))?;
        let capacity_limit = bytes.len().checked_mul(2).and_then(|n| n.checked_add(128)).ok_or_else(|| invalid("inert ICC capacity overflow"))?;
        if owned.capacity() > capacity_limit {
            return Err(invalid("inert ICC allocation exceeds priced capacity"));
        }
        for chunk in bytes.chunks(32768) {
            if interrupt.cancelled() {
                return Err(crate::IoError::Cancelled);
            }
            owned.extend_from_slice(chunk);
        }
        Some(ImageResource::new(ids::ICC_PROFILE, owned))
    } else {
        None
    };
    Ok(finish(doc, force_psb, sample, &channels, planes, warnings, true, inert_icc))
}

// Keep the fixed parameter contract: source identity, limits, control and original budget remain explicit.
#[allow(clippy::too_many_arguments)]
fn finish(
    doc: &Document,
    force_psb: bool,
    sample: SampleType,
    channels: &[&AlphaChannel],
    planes: Vec<u8>,
    mut warnings: Vec<String>,
    checked: bool,
    inert_icc: Option<ImageResource>,
) -> (PsdFile, Vec<String>) {
    let big = doc.size.width > 30_000 || doc.size.height > 30_000;
    let size_exceeds_limit = !force_psb && !big && crate::psd_export::psd_size_exceeds_limit(doc);
    let version = crate::psd_export::psd_version(doc, force_psb, size_exceeds_limit);
    if big && !force_psb {
        warnings.push("document exceeds 30000 px; written as PSB".into());
    } else if size_exceeds_limit && !force_psb {
        warnings.push("estimated encoded size exceeds 2 GB; written as PSB".into());
    }
    let header = Header::new(version, doc.size.width, doc.size.height, channels.len().max(1) as u16, psd_depth(sample), PsdMode::Multichannel);
    // Checked export moves the already validated planes into lossless Raw transport.
    // No additional encoder workspace, uninterruptible compression, or error fallback.
    let image_data = if checked {
        ImageData { compression: Compression::Raw, data: planes }
    } else {
        ImageData::encode(Compression::Rle, &planes, &header)
            .or_else(|_| ImageData::encode(Compression::Raw, &planes, &header))
            .unwrap_or(ImageData { compression: Compression::Raw, data: planes })
    };
    let resolution = doc.effective_resolution();
    let mut axes = ResolutionInfo::from_dpi(resolution.x_dpi);
    axes.v_res_fixed = ResolutionInfo::from_dpi(resolution.y_dpi).v_res_fixed;
    let mut resources = vec![
        ImageResource::new(ids::RESOLUTION_INFO, axes.to_bytes()),
        ImageResource::new(ids::GLOBAL_ANGLE, (doc.global_light.angle.round() as i32).to_be_bytes().to_vec()),
        ImageResource::new(ids::GLOBAL_ALTITUDE, (doc.global_light.altitude.round() as i32).to_be_bytes().to_vec()),
    ];
    // An ICC blob on numeric ink channels is retained as inert metadata. It is
    // never parsed or used to reinterpret these authoritative channel densities.
    if let Some(icc) = inert_icc {
        resources.push(icc);
    }
    // Grid and guides: Photoshop writes it in every file, guides or not.
    resources.push(ImageResource::new(GRID_AND_GUIDES, crate::psd_export::guides_resource(doc)));
    if !channels.is_empty() {
        let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
        resources.push(ImageResource::new(1006, crate::psd_export::pascal_names_resource(&names)));
        resources.push(ImageResource::new(1045, crate::psd_export::unicode_names_resource(&names)));
        resources.push(ImageResource::new(crate::channel_map::DISPLAY_INFO, crate::channel_map::display_info(channels, false)));
    }
    resources.extend(crate::psd_export::path_resources(doc, &mut warnings));
    // The XMP and EXIF resolution follow the ResolutionInfo resource, so no copy contradicts it.
    let ppi = Some((resolution.x_dpi as f32, resolution.y_dpi as f32)).filter(|d| d.0 > 0.0 && d.1 > 0.0);
    if let Some(x) = &doc.metadata.xmp {
        resources.push(ImageResource::new(ids::XMP, photocraft_codecs::export_xmp(x, ppi).as_bytes().to_vec()));
    }
    if let Some(e) = &doc.metadata.exif {
        resources.push(ImageResource::new(ids::EXIF, photocraft_codecs::export_exif(e, ppi).into_owned()));
    }
    // Every other imported resource (print settings, IPTC, the layer-id seed, ...), except the
    // ones that only describe layers and any already written above.
    let written: Vec<u16> = resources.iter().map(|r| r.id).collect();
    for (id, name, data) in &doc.metadata.psd_resources {
        if LAYER_ONLY_RESOURCES.contains(id) || written.contains(id) || !crate::psd_export::keep_imported_resource(doc, *id, data) {
            continue;
        }
        // Alpha identifiers: one u32 per channel, only while they still match the channels.
        if *id == ALPHA_IDENTIFIERS && data.len() != channels.len().saturating_mul(4) {
            continue;
        }
        let mut r = ImageResource::new(*id, data.to_vec());
        r.name = crate::psd_export::legacy_name(name);
        resources.push(r);
    }
    if let Some(data) = crate::annotations_map::fresh_scale_resource(doc) {
        resources.push(ImageResource::new(crate::annotations_map::MEASUREMENT_SCALE, data));
    }
    resources.push(version_info_resource(true));
    // Document-level tagged blocks (patterns, notes, ...) are valid without a layer section; a
    // kept layer block would describe layers the document doesn't have.
    let global_blocks: Vec<TaggedBlock> = crate::annotations_map::export_blocks(doc, crate::pattern_map::export_global_blocks(doc))
        .into_iter()
        .filter(|(_, key, _)| !matches!(key, b"Layr" | b"Lr16" | b"Lr32"))
        .map(|(sig, key, data)| crate::psd_export::global_block(sig, key, &data))
        .collect();
    let color_mode_data = photocraft_psd::hdr::color_mode_data_for_depth(header.depth);
    let file = PsdFile {
        header,
        color_mode_data,
        resources,
        layer_info: None,
        layer_info_placement: LayerInfoPlacement::Section,
        global_layer_mask: (!global_blocks.is_empty()).then(GlobalLayerMask::default),
        global_blocks,
        layer_mask_trailing: Vec::new(),
        image_data,
    };
    (file, warnings)
}

#[cfg(test)]
mod checked_tests {
    use super::*;
    use photocraft_geom::Size;

    #[test]
    fn checked_public_multichannel_inverts_native_wide_samples_once() {
        for (depth, sample, value, expected) in [
            (SampleType::U8, SampleType::F64, (128.5_f64 + 1e-8) / 255.0, vec![126]),
            (SampleType::U16, SampleType::F64, (32768.5_f64 + 1e-7) / 65535.0, 32766u16.to_be_bytes().to_vec()),
            (SampleType::F64, SampleType::F64, (32768.5_f64 + 1e-7) / 65535.0, 32766u16.to_be_bytes().to_vec()),
            (SampleType::U16, SampleType::U32, f64::from(0x8000_0001u32) / f64::from(u32::MAX), 32767u16.to_be_bytes().to_vec()),
        ] {
            let mut doc = Document::new("inks", Size::new(1, 1), ColorMode::Multichannel, depth);
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut raw = vec![0; sample.bytes()];
            photocraft_color::try_write_sample_f64(&mut raw, sample, 0, value).unwrap();
            let s = Surface::try_from_interleaved(fmt, doc.bounds(), &raw).unwrap();
            doc.channels.push(AlphaChannel::new("Own ink", s));
            let before = doc.clone();
            let out = crate::export(&doc, "own.psd", &crate::ExportOptions::default()).unwrap();
            let psd = PsdFile::from_bytes(&out.bytes).unwrap();
            assert_eq!(psd.decode_merged().unwrap(), expected, "{depth:?}/{sample:?}");
            assert_eq!(doc, before);
            if depth == SampleType::F64 {
                assert!(out.warnings.iter().any(|w| w.contains("16 bits/channel")));
            }
        }
    }

    #[test]
    fn checked_public_multichannel_rejects_malformed_or_unqualified_input_atomically() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut base = Document::new("inks", Size::new(1, 1), ColorMode::Multichannel, SampleType::U16);
        base.channels.push(AlphaChannel::new("Own ink Î©", Surface::try_from_interleaved(fmt, base.bounds(), &0.25_f64.to_ne_bytes()).unwrap()));
        let mut cases = Vec::new();
        for value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            let mut doc = base.clone();
            doc.channels[0].surface = Surface::try_from_interleaved(fmt, doc.bounds(), &value.to_ne_bytes()).unwrap();
            cases.push(doc);
        }
        let mut wrong = base.clone();
        wrong.channels[0].surface = Surface::new(PixelFormat::RGBA8);
        cases.push(wrong);
        let mut forged = base.clone();
        let tile = forged.channels[0].surface.solid_tile(&[0.0]);
        forged.channels[0].surface.put_tiles([(photocraft_geom::TileCoord::new(i32::MAX, 0), tile)]);
        cases.push(forged);
        // Opaque ICC bytes are metadata, not a numeric ink interpretation.
        for bytes in [std::sync::Arc::new(vec![1, 2, 3]), photocraft_cms::Builtin::Srgb.profile().to_bytes()] {
            let mut profiled = base.clone();
            profiled.icc_profile = Some(bytes.clone());
            let before = profiled.clone();
            let output = crate::export(&profiled, "inert.psd", &crate::ExportOptions::default()).unwrap();
            let psd = PsdFile::from_bytes(&output.bytes).unwrap();
            assert_eq!(psd.resource(ids::ICC_PROFILE).unwrap().data.as_slice(), bytes.as_slice());
            assert_eq!(psd.decode_merged().unwrap(), 49151u16.to_be_bytes());
            assert_eq!(profiled, before);
        }
        let mut assigned = base.clone();
        assigned.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
        cases.push(assigned);
        let mut layered = base.clone();
        layered.layers.push(photocraft_doc::Layer::raster("Layer", PixelFormat::RGBA8));
        cases.push(layered);
        let mut encoded = base.clone();
        encoded.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        cases.push(encoded);
        let mut excess = base.clone();
        excess.channels.resize(57, base.channels[0].clone());
        cases.push(excess);
        let mut oversized = base.clone();
        oversized.size = Size::new(300_000, 300_000);
        cases.push(oversized);
        let mut empty = base.clone();
        empty.size = Size::new(0, 0);
        cases.push(empty);
        for doc in cases {
            let before = doc.clone();
            assert!(crate::export(&doc, "own.psd", &crate::ExportOptions::default()).is_err());
            assert_eq!(doc, before);
        }
        let before = base.clone();
        let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(try_document_to_psd_with_interrupt(&base, false, &cancelled).is_err());
        assert_eq!(base, before);
    }

    #[test]
    fn checked_public_multichannel_preserves_order_names_and_ink_table() {
        let mut doc = Document::new("inks", Size::new(1, 1), ColorMode::Multichannel, SampleType::U8);
        for (name, sample, value, color, solidity) in
            [("Own Î©", SampleType::F64, 0.25, Color::rgb(1.0, 0.0, 0.0), 0.25), ("Own Î²", SampleType::U32, 0.75, Color::rgb(0.0, 0.0, 1.0), 0.75)]
        {
            let mut raw = vec![0; sample.bytes()];
            photocraft_color::try_write_sample_f64(&mut raw, sample, 0, value).unwrap();
            let surface = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, sample, false), doc.bounds(), &raw).unwrap();
            let mut channel = AlphaChannel::new(name, surface);
            channel.spot = Some((color, solidity));
            doc.channels.push(channel);
        }
        let before = doc.clone();
        let out = crate::export(&doc, "own.psb", &crate::ExportOptions::default()).unwrap();
        let psd = PsdFile::from_bytes(&out.bytes).unwrap();
        assert_eq!(psd.header.channels, 2);
        assert_eq!(psd.decode_merged().unwrap(), [191, 64]);
        let (reopened, _) = crate::psd_to_document(&psd);
        assert_eq!(reopened.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Own Î©", "Own Î²"]);
        assert_eq!(reopened.channels.iter().map(|c| c.spot).collect::<Vec<_>>(), doc.channels.iter().map(|c| c.spot).collect::<Vec<_>>());
        assert_eq!(doc, before);
    }
}
