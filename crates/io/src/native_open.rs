//! Content-first native stream candidate without collecting the encoded archive.
//! A negative candidate restores the caller's cursor for its existing importer.
use crate::{ImportFormat, ImportOrigin, ImportResult, IoError};
use photocraft_format::{FormatError, LoadOptions, zip::stream};
use photocraft_raster::Interrupt;
use std::io::{Read, Seek, SeekFrom};

fn poll(control: &Interrupt<'_>) -> Result<(), IoError> {
    control.check().map_err(|_| IoError::Cancelled)
}
fn format_error(error: FormatError) -> IoError {
    if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() }
}
fn restore_none<R: Seek>(input: &mut R, position: u64, control: &Interrupt<'_>) -> Result<Option<ImportResult>, IoError> {
    input.seek(SeekFrom::Start(position)).map_err(FormatError::from)?;
    poll(control)?;
    Ok(None)
}

/// Examine the whole seekable input from offset zero. On `None`, restore the
/// incoming cursor; the caller can run its existing content-first flat/XCF path
/// on the same held input. No path, locator or resource authority is consulted.
///
/// Plain XCF and recognized compression wrappers take precedence over a native
/// suffix. Native ZIP manifests are detected through the bounded central index,
/// including archives whose first entry is not the manifest. A declared native
/// suffix (or first-entry native manifest) retains actionable native errors.
/// Standard archive/object limits remain unchanged; this does not enable
/// `native_large`. Successful results have the same profile admission, warnings
/// and assessment as the existing byte-native import lane.
pub fn import_native_stream_candidate<R: Read + Seek>(
    name: &str,
    input: &mut R,
    control: &Interrupt<'_>,
) -> Result<Option<ImportResult>, IoError> {
    poll(control)?;
    let original = input.stream_position().map_err(FormatError::from)?;
    input.seek(SeekFrom::Start(0)).map_err(FormatError::from)?;
    let mut prefix = [0u8; 64];
    let mut filled = 0usize;
    while filled < prefix.len() {
        poll(control)?;
        let target = prefix.get_mut(filled..).ok_or_else(|| FormatError::Corrupt("native prefix range".into()))?;
        let read = match input.read(target) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            other => other.map_err(FormatError::from)?,
        };
        if read == 0 { break; }
        if read > target.len() { return Err(FormatError::Corrupt("native prefix read count".into()).into()); }
        filled = filled.checked_add(read).ok_or_else(|| FormatError::Corrupt("native prefix overflow".into()))?;
    }
    let prefix = prefix.get(..filled).ok_or_else(|| FormatError::Corrupt("native prefix range".into()))?;
    if photocraft_xcf::classify_container(prefix).is_some() {
        return restore_none(input, original, control);
    }
    let declared = crate::has_extension(name, photocraft_format::EXTENSION);
    let zip = prefix.starts_with(b"PK\x03\x04");
    let first_manifest = zip && prefix.get(26..28) == Some(&[13, 0]) && prefix.get(30..43) == Some(b"manifest.json");
    if !declared && !zip {
        return restore_none(input, original, control);
    }
    if !declared && !first_manifest {
        // Drop this temporary index before loading; the loader owns its own
        // controlled index and expanded-object accounting.
        let native = match stream::Reader::new(&mut *input, Default::default(), control) {
            Ok(archive) => archive.contains("manifest.json"),
            Err(FormatError::Cancelled) => return Err(IoError::Cancelled),
            Err(error @ FormatError::Io(_)) => return Err(error.into()),
            Err(_) => false,
        };
        poll(control)?;
        if !native { return restore_none(input, original, control); }
    }
    poll(control)?;
    input.seek(SeekFrom::Start(0)).map_err(FormatError::from)?;
    let document = photocraft_format::load_from_stream(&mut *input, &LoadOptions::default(), Default::default(), control).map_err(format_error)?;
    poll(control)?;
    crate::xcf_export::active_implicit_profile(&document).map_err(|error| IoError::Unsupported(format!("native working profile: {error}")))?;
    poll(control)?;
    control.progress(1.0);
    Ok(Some(ImportResult { document, warnings: Vec::new(), origin: ImportOrigin::writable(ImportFormat::Pcraft), assessment: None }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn content_first_and_negative_candidates_restore_incoming_position() {
        for bytes in [b"gimp xcf v013\0".as_slice(), &[31, 139, 0], b"BZh9", &[253, 55, 122, 88, 90, 0], b"ordinary"] {
            let mut input = Cursor::new(bytes);
            input.set_position(2);
            let name = if bytes == b"ordinary" { "ordinary.png" } else { "misnamed.pcraft" };
            assert!(import_native_stream_candidate(name, &mut input, &Interrupt::NONE).unwrap().is_none());
            assert_eq!(input.position(), 2);
        }
    }

    #[test]
    fn declared_truncated_native_and_cancelled_probe_are_errors() {
        let mut input = Cursor::new(b"PK\x03\x04");
        assert!(matches!(import_native_stream_candidate("own.pcraft", &mut input, &Interrupt::NONE), Err(IoError::Pcraft(_))));
        let mut input = Cursor::new(b"ordinary");
        input.set_position(2);
        assert!(matches!(import_native_stream_candidate("own.png", &mut input, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
        assert_eq!(input.position(), 2);
    }

    #[test]
    fn native_manifest_need_not_be_first_or_have_native_suffix() {
        let doc = photocraft_doc::Document::new("own", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let source = photocraft_format::zip::ZipReader::new(&native).unwrap();
        let mut writer = photocraft_format::zip::ZipWriter::new();
        writer.add("owned-extra", b"own").unwrap();
        for entry in &source.entries {
            writer.add(&entry.name, &source.read(entry, 1 << 20).unwrap()).unwrap();
        }
        let mut input = Cursor::new(writer.finish().unwrap());
        let result = import_native_stream_candidate("misnamed.png", &mut input, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(result.document.name, doc.name);
        assert_eq!(result.origin, ImportOrigin::writable(ImportFormat::Pcraft));
        assert!(result.warnings.is_empty());
        assert!(result.assessment.is_none());
    }
}
