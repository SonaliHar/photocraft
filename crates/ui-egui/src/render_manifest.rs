//! Which pictures of a photo's edits are current, for a host that shows them (Image Triage).
//!
//! Every hosted save and every Camera Raw recipe render writes its pictures under names of their own
//! (`<photo>.photocraft.g<N>.png`, `<photo>.photocraft.display.g<N>.jpg`), never over an earlier picture,
//! and then switches the photo's manifest (`<photo>.photocraft.renders.json`, one atomic write) to name
//! them and what they were made from: the original photo (its size and modified time as the host saw
//! them), the project file or recipe (its SHA-256), and the render algorithm. A reader that follows the
//! manifest therefore gets exactly the pictures it describes, and one whose sources no longer match the
//! files on disk knows the pictures are out of date, whatever the files' timestamps say.
//!
//! Within one editor, a manifest is only ever replaced by a newer generation: a render that finishes after
//! a later save of the same photo publishes nothing. (Its pictures are left for the host to clear away:
//! the editor's write authority cannot delete.)

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Version of the manifest file.
pub const VERSION: u64 = 1;
/// Bumped whenever the pictures made from the same edits change (render maths, colour handling, sizes), so
/// a host knows pictures made before are out of date. Image Triage keeps the same number.
pub const ALGORITHM: u64 = 2;

/// What a host asked for: where the manifest goes, and the original photo's signature (`"<bytes>:<mtime_ns>"`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub manifest: String,
    pub source: Option<String>,
}

/// What the pictures were made from.
#[derive(Clone, Debug, PartialEq)]
pub enum Made {
    /// A saved project: the file's size and SHA-256.
    Project { size: u64, sha256: String, depth: u8 },
    /// A Camera Raw recipe: the SHA-256 of the recipe file's bytes.
    Recipe { sha256: String },
}

/// A new generation number: increasing within the editor, and from the clock across editors.
pub fn generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(seen) => last = seen,
        }
    }
}

/// `target` with the generation before its extension: `a.photocraft.png` -> `a.photocraft.g7.png`.
pub fn versioned(target: &str, generation: u64) -> String {
    let name_start = target.rfind(['/', '\\']).map_or(0, |i| i + 1);
    match target[name_start..].rfind('.') {
        Some(dot) => format!("{}.g{generation}{}", &target[..name_start + dot], &target[name_start + dot..]),
        None => format!("{target}.g{generation}"),
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The file name of `target` (the manifest names its pictures relative to its own folder).
fn file_name(target: &str) -> &str {
    target.rsplit(['/', '\\']).next().unwrap_or(target)
}

/// What an encoded picture actually holds, read from its bytes: format, size, bits per channel and
/// whether it carries an ICC profile. `None` for bytes that are neither PNG nor JPEG.
pub fn describe(bytes: &[u8]) -> Option<Value> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let be32 = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        if bytes.get(12..16)? != b"IHDR" {
            return None;
        }
        let (width, height, depth) = (be32(16)?, be32(20)?, *bytes.get(24)?);
        // iCCP comes before the image data, if at all.
        let mut at = 8;
        let mut icc = false;
        while let (Some(len), Some(kind)) = (be32(at), bytes.get(at + 4..at + 8)) {
            if kind == b"iCCP" {
                icc = true;
            }
            if kind == b"IDAT" || kind == b"IEND" {
                break;
            }
            at += 12 + len as usize;
        }
        return Some(json!({"format": "png", "width": width, "height": height, "depth": depth, "icc": icc}));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut at = 2;
        let mut icc = false;
        while at + 4 <= bytes.len() && bytes[at] == 0xFF {
            let marker = bytes[at + 1];
            let len = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
            if marker == 0xE2 && bytes.get(at + 4..at + 16) == Some(b"ICC_PROFILE\0") {
                icc = true;
            }
            if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                let depth = *bytes.get(at + 4)?;
                let height = u16::from_be_bytes([*bytes.get(at + 5)?, *bytes.get(at + 6)?]);
                let width = u16::from_be_bytes([*bytes.get(at + 7)?, *bytes.get(at + 8)?]);
                return Some(json!({"format": "jpeg", "width": width, "height": height, "depth": depth, "icc": icc}));
            }
            if marker == 0xDA {
                break;
            }
            at += 2 + len;
        }
        return None;
    }
    None
}

/// The manifest's content for pictures of generation `generation` made from `made`. `outputs` are the pictures that were
/// written, by role (`"png"`, the full-size render; `"display"`, the screen-sized one), each with the path it was written to
/// and its bytes.
pub fn content(generation: u64, made: &Made, source: Option<&str>, outputs: &[(&str, &str, &[u8])]) -> Value {
    let mut pictures = serde_json::Map::new();
    for (role, target, bytes) in outputs {
        let mut described = describe(bytes).unwrap_or_else(|| json!({}));
        described["file"] = json!(file_name(target));
        pictures.insert(role.to_string(), described);
    }
    let mut manifest = json!({
        "version": VERSION,
        "generation": generation,
        "source": source,
        "renderer": {"algorithm": ALGORITHM, "build": photocraft_engine::build_info::git_sha(), "version": photocraft_engine::build_info::VERSION},
        "outputs": pictures,
    });
    match made {
        Made::Project { size, sha256, depth } => {
            manifest["kind"] = json!("project");
            manifest["project"] = json!({"size": size, "sha256": sha256, "depth": depth});
        }
        Made::Recipe { sha256 } => {
            manifest["kind"] = json!("recipe");
            manifest["recipe"] = json!({"sha256": sha256});
        }
    }
    manifest
}

/// Switches the photo's manifest to generation `generation`, unless this editor already published a newer one for it.
/// `Ok(false)` when it was not published for that reason.
pub fn publish(write: &crate::BackgroundAutomationWriteFn, request: &Request, generation: u64, manifest: &Value) -> Result<bool, String> {
    use std::sync::{Mutex, PoisonError};
    static PUBLISHED: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());
    // Held across the write, so two publishes of one photo cannot cross.
    let mut published = PUBLISHED.lock().unwrap_or_else(PoisonError::into_inner);
    if published.iter().any(|(path, n)| *path == request.manifest && *n >= generation) {
        return Ok(false);
    }
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    write(&request.manifest, &bytes).map_err(|error| format!("could not write {}: {error}", request.manifest))?;
    published.retain(|(path, _)| *path != request.manifest);
    published.push((request.manifest.clone(), generation));
    if published.len() > 256 {
        published.remove(0);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn generations_increase() {
        let a = generation();
        let b = generation();
        assert!(b > a);
    }

    #[test]
    fn a_generation_goes_before_the_extension() {
        assert_eq!(versioned("a.dng.photocraft.png", 7), "a.dng.photocraft.g7.png");
        assert_eq!(versioned("x/a.dng.photocraft.display.jpg", 12), "x/a.dng.photocraft.display.g12.jpg");
        assert_eq!(versioned("dir.v2/name", 3), "dir.v2/name.g3");
    }

    #[test]
    fn pictures_are_described_from_their_bytes() {
        let doc = photocraft_doc::Document::new("p", photocraft_geom::Size::new(6, 4), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U16);
        let png = photocraft_io::export(&doc, "p.png", &Default::default()).unwrap().bytes;
        let described = describe(&png).unwrap();
        assert_eq!((described["format"].as_str(), described["width"].as_u64(), described["height"].as_u64()), (Some("png"), Some(6), Some(4)));
        assert_eq!(described["depth"], 16, "{described}");
        let jpeg = photocraft_io::export_display_jpeg(&doc, 64, 90).unwrap().bytes;
        let described = describe(&jpeg).unwrap();
        assert_eq!((described["format"].as_str(), described["depth"].as_u64()), (Some("jpeg"), Some(8)), "{described}");
        assert!(describe(b"not a picture").is_none());
    }

    #[test]
    fn an_older_generation_never_replaces_a_newer_manifest() {
        let writes = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let recorded = writes.clone();
        let write: crate::BackgroundAutomationWriteFn = Arc::new(move |_, bytes| {
            recorded.lock().unwrap().push(bytes.to_vec());
            Ok(())
        });
        let request = Request { manifest: format!("older-{}.renders.json", generation()), source: None };
        let made = Made::Recipe { sha256: sha256_hex(b"{}") };
        let (older, newer) = (generation(), generation());
        assert!(publish(&write, &request, newer, &content(newer, &made, None, &[])).unwrap());
        assert!(!publish(&write, &request, older, &content(older, &made, None, &[])).unwrap(), "a late, older render publishes nothing");
        let written = writes.lock().unwrap();
        assert_eq!(written.len(), 1);
        let manifest: Value = serde_json::from_slice(&written[0]).unwrap();
        assert_eq!(manifest["generation"], newer);
    }
}
