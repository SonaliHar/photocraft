//! Reject image decoders without enforceable budgets before Hayro sees a page.
//! This deliberately supports a conservative subset: unfiltered, Flate and JPEG
//! streams. JPX/JBIG2/CCITT/LZW and inline images fail explicitly, not as missing
//! artwork. PDF dictionaries alone cannot bound codec-internal allocations.
//! The same pass records CMYK and spot colours, which rasterizing turns into sRGB.
use std::collections::BTreeSet;
use std::io::Read;

use super::error;
use crate::IoError;
use hayro::hayro_syntax::{
    Pdf,
    object::{Array, Dict, MaybeRef, Name, Object, Stream},
};
use photocraft_raster::Interrupt;

const STREAM_BYTES: usize = 64 * 1024 * 1024;
const TOTAL_BYTES: usize = 256 * 1024 * 1024;
const IMAGE_PIXELS: u64 = 16_000_000;
const TOTAL_IMAGE_PIXELS: u64 = 64_000_000;
const OBJECTS: usize = 100_000;

fn dimensions(w: u32, h: u32) -> Result<u64, IoError> {
    let pixels = u64::from(w) * u64::from(h);
    if w == 0 || h == 0 || w > 16_384 || h > 16_384 || pixels > IMAGE_PIXELS {
        return Err(error("embedded PDF image exceeds the 16,384-pixel side or 16-megapixel decode limit"));
    }
    Ok(pixels)
}

fn cancelled(ctl: &Interrupt) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)
}

/// Check the actual JPEG frame, not only the PDF's Width/Height declaration.
fn jpeg_dimensions(bytes: &[u8]) -> Result<u64, IoError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(error("invalid embedded JPEG"));
    }
    let mut pos = 2usize;
    while let Some(&byte) = bytes.get(pos) {
        if byte != 0xff {
            return Err(error("invalid embedded JPEG marker"));
        }
        while bytes.get(pos) == Some(&0xff) {
            pos += 1;
        }
        let marker = *bytes.get(pos).ok_or_else(|| error("truncated embedded JPEG"))?;
        pos += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 1 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let len = bytes.get(pos..pos + 2).ok_or_else(|| error("truncated embedded JPEG segment"))?;
        let [a, b] = *len else {
            return Err(error("invalid JPEG segment length"));
        };
        let len = usize::from(u16::from_be_bytes([a, b]));
        if len < 2 {
            return Err(error("invalid embedded JPEG segment"));
        }
        let segment = bytes.get(pos..pos + len).ok_or_else(|| error("truncated embedded JPEG segment"))?;
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            let frame = segment.get(2..8).ok_or_else(|| error("truncated embedded JPEG frame"))?;
            let [depth, h0, h1, w0, w1, components] = *frame else {
                return Err(error("invalid JPEG frame"));
            };
            if depth != 8 || ![1, 3, 4].contains(&components) {
                return Err(error("unsupported embedded JPEG color depth"));
            }
            return dimensions(u32::from(u16::from_be_bytes([w0, w1])), u32::from(u16::from_be_bytes([h0, h1])));
        }
        pos += len;
    }
    Err(error("embedded JPEG has no supported frame"))
}

fn inflate(bytes: &[u8], ctl: &Interrupt) -> Result<Vec<u8>, IoError> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        cancelled(ctl)?;
        let n = decoder.read(&mut chunk).map_err(|_| error("invalid PDF Flate stream"))?;
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > STREAM_BYTES {
            return Err(error("PDF decoded stream exceeds 64 MB"));
        }
        out.extend_from_slice(chunk.get(..n).ok_or_else(|| error("invalid decoded stream length"))?);
    }
    Ok(out)
}

fn predictor(dict: &Dict<'_>) -> Result<(), IoError> {
    let params = dict.get::<Dict<'_>>(b"DecodeParms").or_else(|| dict.get::<Dict<'_>>(b"DP"));
    // Filter arrays are rejected below; don't let an array of parameters evade checks.
    if dict.get::<Array<'_>>(b"DecodeParms").is_some() || dict.get::<Array<'_>>(b"DP").is_some() {
        return Err(error("PDF stream filter parameter arrays are not supported by bounded import"));
    }
    if let Some(p) = params {
        let columns = p.get::<u32>(b"Columns").unwrap_or(1);
        let colors = p.get::<u32>(b"Colors").unwrap_or(1);
        let bits = p.get::<u32>(b"BitsPerComponent").unwrap_or(8);
        if columns == 0 || columns > 16_384 || !(1..=4).contains(&colors) || ![1, 2, 4, 8, 16].contains(&bits) {
            return Err(error("PDF predictor exceeds decode limits"));
        }
    }
    Ok(())
}

fn image_colorspace(space: Object<'_>, depth: u8) -> Result<(), IoError> {
    if depth > 2 {
        return Err(error("PDF image color space nesting exceeds decode limits"));
    }
    let supported = match space {
        Object::Name(n) => matches!(n.as_ref(), b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK"),
        Object::Array(a) => {
            let mut items = a.iter::<Object<'_>>();
            match items.next().and_then(Object::into_name).as_ref().map(|n| n.as_ref()) {
                Some(b"ICCBased") => items.next().and_then(Object::into_stream).is_some_and(|s| [1, 3, 4].contains(&s.dict().get::<u32>(b"N").unwrap_or(0))),
                Some(b"Indexed") => {
                    if let Some(base) = items.next() {
                        image_colorspace(base, depth + 1)?;
                    } else {
                        return Err(error("missing PDF indexed color space"));
                    }
                    true
                }
                _ => false,
            }
        }
        _ => false,
    };
    if !supported {
        return Err(error("PDF image color space is not supported by bounded import"));
    }
    Ok(())
}

fn has_inline_image(data: &[u8]) -> bool {
    let delimiter = |b: u8| b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b);
    data.split(|b| delimiter(*b)).any(|token| {
        let mut bytes = token.iter().copied();
        // Hayro normalizes name escapes while reading operators too. Cover those
        // spellings even though conforming PDF content uses the literal BI token.
        let mut next = || {
            let b = bytes.next()?;
            if b != b'#' {
                return Some(b);
            }
            let a = char::from(bytes.next()?).to_digit(16)? as u8;
            let b = char::from(bytes.next()?).to_digit(16)? as u8;
            Some((a << 4) | b)
        };
        next() == Some(b'B') && next() == Some(b'I') && next().is_none()
    })
}

const SPOT_NAMES: usize = 16;
const SPOT_NAME_CHARS: usize = 64;
const COLOUR_DEPTH: u8 = 32;

/// Colour that rasterizing turns into sRGB, reported to the user rather than lost silently.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Inks {
    /// DeviceCMYK, CalCMYK, four-channel ICC or a process colorant was used.
    pub(super) cmyk: bool,
    /// Spot colorant names in name order, capped at `SPOT_NAMES`.
    pub(super) spots: BTreeSet<String>,
    /// More spot colorants than `SPOT_NAMES` were found.
    pub(super) more_spots: bool,
}

impl Inks {
    fn colorant(&mut self, name: &[u8]) {
        match name {
            b"Cyan" | b"Magenta" | b"Yellow" | b"Black" => self.cmyk = true,
            // All paints every separation (registration marks); None paints nothing.
            b"All" | b"None" => {}
            _ => {
                let name: String = String::from_utf8_lossy(name).chars().take(SPOT_NAME_CHARS).collect();
                if self.spots.len() < SPOT_NAMES {
                    self.spots.insert(name);
                } else if !self.spots.contains(&name) {
                    self.more_spots = true;
                }
            }
        }
    }

    /// User-facing import warnings; empty for RGB and gray documents.
    pub(super) fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.spots.is_empty() {
            let names = self.spots.iter().map(String::as_str).collect::<Vec<_>>().join(", ");
            let more = if self.more_spots { " and others" } else { "" };
            out.push(format!(
                "Spot colours ({names}{more}) were converted to sRGB through their alternate colour space. Separations are not kept, so the inks can't be output as plates."
            ));
        }
        if self.cmyk {
            out.push(
                "This PDF uses CMYK colour, which was converted to 8-bit sRGB. The CMYK values are not kept, and colours can differ from the printed result."
                    .into(),
            );
        }
        out
    }
}

/// Walk the object's direct children. Indirect children are visited as top-level objects,
/// so references are not followed here and reference cycles can't recurse.
fn colours(object: &Object<'_>, inks: &mut Inks, depth: u8) {
    if depth > COLOUR_DEPTH {
        return;
    }
    match object {
        Object::Name(n) => {
            if matches!(n.as_ref(), b"DeviceCMYK" | b"CalCMYK") {
                inks.cmyk = true;
            }
        }
        Object::Array(a) => array_colours(a, inks, depth),
        Object::Dict(d) => dict_colours(d, inks, depth),
        Object::Stream(s) => dict_colours(s.dict(), inks, depth),
        _ => {}
    }
}

fn dict_colours(dict: &Dict<'_>, inks: &mut Inks, depth: u8) {
    for (_, value) in dict.entries() {
        if let MaybeRef::NotRef(value) = value {
            colours(&value, inks, depth + 1);
        }
    }
}

fn array_colours(array: &Array<'_>, inks: &mut Inks, depth: u8) {
    let mut items = array.iter::<Object<'_>>();
    let family = items.next().and_then(Object::into_name);
    match family.as_ref().map(|n| n.as_ref()) {
        // [/Separation name alternate tint]: the alternate is how the ink is approximated, not
        // a sign the artwork itself is CMYK, so it is not walked.
        Some(b"Separation") => {
            if let Some(name) = items.next().and_then(Object::into_name) {
                inks.colorant(name.as_ref());
            }
        }
        // [/DeviceN [names] alternate tint attributes]
        Some(b"DeviceN") => {
            if let Some(names) = items.next().and_then(Object::into_array) {
                for name in names.iter::<Name<'_>>().take(SPOT_NAMES * 4) {
                    inks.colorant(name.as_ref());
                }
            }
        }
        Some(b"ICCBased") => {
            if items.next().and_then(Object::into_stream).is_some_and(|s| s.dict().get::<u32>(b"N") == Some(4)) {
                inks.cmyk = true;
            }
        }
        _ => {
            for item in array.raw_iter() {
                if let MaybeRef::NotRef(item) = item {
                    colours(&item, inks, depth + 1);
                }
            }
        }
    }
}

/// Content streams carry operators: page contents, forms, patterns and Type3 glyphs. Fonts,
/// ICC profiles and metadata are binary or prose, where a lone `k` means nothing.
fn has_operators(dict: &Dict<'_>) -> bool {
    const DATA_KEYS: [&[u8]; 4] = [b"N", b"Length1", b"Length2", b"Length3"];
    let form = dict.get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Form");
    let plain = !dict.contains_key(b"Type") && !dict.contains_key(b"Subtype");
    (form || plain || dict.contains_key(b"PatternType")) && !DATA_KEYS.iter().any(|k| dict.contains_key(k))
}

/// True when content sets a DeviceCMYK colour with `k` or `K`. Strings, names, comments and
/// inline dictionaries are skipped, so `(K) Tj` or `/K BMC` don't count.
fn sets_cmyk(data: &[u8]) -> bool {
    enum Mode {
        Code,
        Literal(usize),
        Hex,
        Comment,
    }
    let mut mode = Mode::Code;
    let mut escaped = false;
    let (mut len, mut first, mut name) = (0usize, 0u8, false);
    // A trailing space ends the last token.
    for &b in data.iter().chain(b" ") {
        match mode {
            Mode::Literal(depth) => {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'(' {
                    mode = Mode::Literal(depth.saturating_add(1));
                } else if b == b')' {
                    mode = if depth <= 1 { Mode::Code } else { Mode::Literal(depth - 1) };
                }
                continue;
            }
            // `<<` dictionaries are skipped with hex strings; they hold operands, not operators.
            Mode::Hex => {
                if b == b'>' {
                    mode = Mode::Code;
                }
                continue;
            }
            Mode::Comment => {
                if b == b'\n' || b == b'\r' {
                    mode = Mode::Code;
                }
                continue;
            }
            Mode::Code => {}
        }
        if !(b == 0 || b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b)) {
            if len == 0 {
                first = b;
            }
            len = len.saturating_add(1);
            continue;
        }
        if len == 1 && !name && (first == b'k' || first == b'K') {
            return true;
        }
        len = 0;
        name = b == b'/';
        match b {
            b'(' => mode = Mode::Literal(1),
            b'<' => mode = Mode::Hex,
            b'%' => mode = Mode::Comment,
            _ => {}
        }
    }
    false
}

fn stream(stream: &Stream<'_>, ctl: &Interrupt, images: &mut u64, inks: &mut Inks) -> Result<usize, IoError> {
    cancelled(ctl)?;
    let dict = stream.dict();
    let is_image = dict.get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Image");
    if is_image {
        if let Some(space) = dict.get::<Object<'_>>(b"ColorSpace") {
            image_colorspace(space, 0)?;
        }
        *images = images.saturating_add(dimensions(dict.get::<u32>(b"Width").unwrap_or(0), dict.get::<u32>(b"Height").unwrap_or(0))?);
        if *images > TOTAL_IMAGE_PIXELS {
            return Err(error("PDF embedded images exceed 64 megapixels in total"));
        }
        if ![1, 2, 4, 8, 16].contains(&dict.get::<u32>(b"BitsPerComponent").unwrap_or(1)) {
            return Err(error("unsupported PDF image bit depth"));
        }
    }
    // Inspect names via the parser so escaped names and indirect filters cannot bypass this.
    if dict.get::<Array<'_>>(b"Filter").is_some() || dict.get::<Array<'_>>(b"F").is_some() {
        return Err(error("PDF filter chains are not supported by bounded import; resave with Flate or JPEG compression"));
    }
    let name = dict.get::<Name<'_>>(b"Filter").or_else(|| dict.get::<Name<'_>>(b"F"));
    let filter = name.as_ref().map(|n| n.as_ref());
    if filter.is_none() && (dict.contains_key(b"Filter") || dict.contains_key(b"F")) {
        return Err(error("invalid PDF stream filter"));
    }
    if !matches!(filter, None | Some(b"FlateDecode" | b"Fl" | b"DCTDecode" | b"DCT")) {
        return Err(error(
            "PDF image/stream codec has no bounded decoder (including JPEG2000, JBIG2, CCITT and LZW). Resave the PDF using JPEG or Flate compression.",
        ));
    }
    predictor(dict)?;
    if !is_image && dict.get::<Dict<'_>>(b"DecodeParms").or_else(|| dict.get::<Dict<'_>>(b"DP")).is_some_and(|p| p.get::<u32>(b"Predictor").unwrap_or(1) != 1) {
        return Err(error("PDF predictors are only supported on image streams"));
    }
    let raw = stream.raw_data();
    if raw.len() > STREAM_BYTES {
        return Err(error("PDF stream exceeds 64 MB"));
    }
    let decoded = match filter {
        Some(b"FlateDecode" | b"Fl") => std::borrow::Cow::Owned(inflate(&raw, ctl)?),
        Some(b"DCTDecode" | b"DCT") => {
            if !is_image {
                return Err(error("JPEG filter on a non-image PDF stream"));
            }
            // Both dimensions are bounded independently; count the larger codec frame too.
            *images = images.saturating_add(jpeg_dimensions(&raw)?);
            if *images > TOTAL_IMAGE_PIXELS {
                return Err(error("PDF embedded images exceed the decode budget"));
            }
            return Ok(raw.len());
        }
        _ => raw,
    };
    // Inline images can occur in page, form, pattern, appearance and Type3 streams.
    // Reject the operator before interpretation, including when nested in resources.
    // This conservative lexical check can also reject a literal string containing BI.
    if !is_image && has_inline_image(&decoded) {
        return Err(error("Inline PDF images are not supported by bounded import; resave them as image XObjects"));
    }
    if !inks.cmyk && has_operators(dict) && sets_cmyk(&decoded) {
        inks.cmyk = true;
    }
    Ok(decoded.len())
}

/// Check every object against the decode budgets, and report the colour rasterizing converts.
pub(super) fn preflight(pdf: &Pdf, ctl: &Interrupt) -> Result<Inks, IoError> {
    if pdf.len() > OBJECTS {
        return Err(error("PDF exceeds 100,000 objects"));
    }
    let (mut total, mut images, mut inks) = (0usize, 0u64, Inks::default());
    for object in pdf.objects() {
        cancelled(ctl)?;
        colours(&object, &mut inks, 0);
        if let Some(s) = object.into_stream() {
            total = total.saturating_add(stream(&s, ctl, &mut images, &mut inks)?);
            if total > TOTAL_BYTES {
                return Err(error("PDF decoded streams exceed 256 MB in total"));
            }
        }
    }
    Ok(inks)
}

#[cfg(test)]
mod tests {
    #[test]
    fn inline_image_check_covers_operator_delimiters() {
        for data in [b"BI /W 1".as_slice(), b"q BI/W 1", b"q\nBI\n/W 1", b"BI", b"(x)BI/W 1", b"q B#49/W 1", b"q #42I/W 1"] {
            assert!(super::has_inline_image(data));
        }
        assert!(!super::has_inline_image(b"BT (BINDER) Tj ET"));
    }

    use super::{Inks, preflight, sets_cmyk};
    use hayro::hayro_syntax::Pdf;
    use photocraft_raster::Interrupt;
    use std::sync::Arc;

    /// One-page PDF: object 4 is the page content, `extra` objects start at 5.
    fn fixture(resources: &str, content: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << {resources} >> /Contents 4 0 R >>").into_bytes(),
            [format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), content, b"\nendstream"].concat(),
        ];
        objects.extend_from_slice(extra);
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(object);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    fn inks(resources: &str, content: &[u8], extra: &[Vec<u8>]) -> Inks {
        let pdf = Pdf::new(Arc::new(fixture(resources, content, extra))).unwrap();
        preflight(&pdf, &Interrupt::default()).unwrap()
    }

    fn spots(names: &[&str]) -> std::collections::BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    const SPOT_185: &str = "/CS0 [/Separation /PANTONE#20185#20C /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.91 0.76 0] /N 1 >>]";

    #[test]
    fn cmyk_operators_are_found_outside_strings_names_and_comments() {
        for data in [b"0 0 0 1 k".as_slice(), b"1 0 0 0 K", b"q 0 0 0 1 k 0 0 1 1 re f Q", b"0 0 0 1\nk\n", b"(x)0 0 0 1 k", b"[1]0 0 0 1 k"] {
            assert!(sets_cmyk(data), "{}", String::from_utf8_lossy(data));
        }
        for data in [
            b"1 0 0 rg 0 0 1 1 re f".as_slice(),
            b"BT /F1 12 Tf (K) Tj ET",
            b"BT (a \\) k) Tj ET",
            b"BT (a (k) b) Tj ET",
            b"BT <6B> Tj ET",
            b"/K BMC EMC",
            b"/Span <</MCID 0 /K k>> BDC EMC",
            b"% 0 0 0 1 k\n0 g",
            b"kk Kk 1k",
            b"",
        ] {
            assert!(!sets_cmyk(data), "{}", String::from_utf8_lossy(data));
        }
    }

    #[test]
    fn rgb_and_gray_pdfs_report_nothing() {
        let found = inks("/ColorSpace << /CS0 /DeviceRGB /CS1 /DeviceGray >>", b"1 0 0 rg 0 0 5 5 re f 0.5 g 5 5 5 5 re f", &[]);
        assert_eq!(found, Inks::default());
        assert!(found.warnings().is_empty());
    }

    #[test]
    fn device_cmyk_content_and_resources_are_reported() {
        assert!(inks("", b"0 0 0 1 k 0 0 5 5 re f", &[]).cmyk);
        assert!(inks("/ColorSpace << /CS0 /DeviceCMYK >>", b"/CS0 cs 0 0 0 1 sc 0 0 5 5 re f", &[]).cmyk);
        assert!(inks("/ColorSpace << /CS0 [/CalCMYK << >>] >>", b"", &[]).cmyk);
        let warnings = inks("", b"0 0 0 1 k 0 0 5 5 re f", &[]).warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("CMYK") && warnings[0].contains("sRGB"), "{warnings:?}");
    }

    #[test]
    fn cmyk_operators_in_fonts_and_icc_data_are_ignored() {
        // A lone `k` byte in embedded font or profile data is not an operator.
        let font = [b"<< /Length1 9 /Length 9 >>\nstream\n0 0 0 1 k\nendstream".to_vec()];
        assert!(!inks("", b"0 g", &font).cmyk);
        let icc = [b"<< /N 3 /Length 9 >>\nstream\n0 0 0 1 k\nendstream".to_vec()];
        assert!(!inks("", b"0 g", &icc).cmyk);
    }

    #[test]
    fn four_channel_icc_is_cmyk_and_three_channel_is_not() {
        let profile = |n: u8| [format!("<< /N {n} /Length 4 >>\nstream\nicc!\nendstream").into_bytes()];
        assert!(inks("/ColorSpace << /CS0 [/ICCBased 5 0 R] >>", b"", &profile(4)).cmyk);
        assert!(!inks("/ColorSpace << /CS0 [/ICCBased 5 0 R] >>", b"", &profile(3)).cmyk);
    }

    #[test]
    fn spot_colours_are_named_without_flagging_their_cmyk_alternate() {
        let found = inks(&format!("/ColorSpace << {SPOT_185} >>"), b"/CS0 cs 1 scn 0 0 5 5 re f", &[]);
        assert_eq!(found.spots, spots(&["PANTONE 185 C"]));
        assert!(!found.cmyk, "the alternate space approximates the ink; the artwork isn't CMYK");
        let warnings = found.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("PANTONE 185 C") && warnings[0].contains("Separations are not kept"), "{warnings:?}");
    }

    #[test]
    fn device_n_separates_process_and_spot_colorants() {
        let found = inks("/ColorSpace << /CS0 [/DeviceN [/Cyan /Varnish /None /All] /DeviceCMYK 5 0 R] >>", b"", &[]);
        assert_eq!(found.spots, spots(&["Varnish"]));
        assert!(found.cmyk);
        assert_eq!(found.warnings().len(), 2);
    }

    #[test]
    fn indirect_spot_colour_spaces_are_found_once() {
        // Shared colour spaces are usually indirect objects, referenced from several pages.
        let space = [b"[/Separation /Gold /DeviceRGB 6 0 R]".to_vec(), b"<< /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0.8 0.6 0.2] /N 1 >>".to_vec()];
        let found = inks("/ColorSpace << /CS0 5 0 R /CS1 5 0 R >>", b"/CS0 cs 1 scn 0 0 5 5 re f", &space);
        assert_eq!(found.spots, spots(&["Gold"]));
        assert!(!found.cmyk);
    }

    #[test]
    fn spot_names_are_capped_and_bounded() {
        let mut found = Inks::default();
        for i in 0..40 {
            found.colorant(format!("Spot {i:02}").as_bytes());
        }
        found.colorant(&[b'x'; 500]);
        found.colorant(&[0xff, 0xfe]);
        assert_eq!(found.spots.len(), super::SPOT_NAMES);
        assert!(found.more_spots);
        assert!(found.warnings()[0].contains("and others"));
        let mut long = Inks::default();
        long.colorant(&[b'x'; 500]);
        long.colorant(&[0xff, 0xfe]);
        assert!(long.spots.iter().all(|n| n.chars().count() <= super::SPOT_NAME_CHARS));
        assert_eq!(long.spots.len(), 2);
    }

    #[test]
    fn deeply_nested_objects_stop_at_the_depth_limit() {
        let nested = format!("/Deep {}/DeviceCMYK{}", "[".repeat(200), "]".repeat(200));
        // Beyond the limit the walk stops; it must not overflow the stack or panic.
        let _ = inks(&nested, b"", &[]);
        let shallow = format!("/Deep {}/DeviceCMYK{}", "[".repeat(4), "]".repeat(4));
        assert!(inks(&shallow, b"", &[]).cmyk);
    }
}
