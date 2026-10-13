use super::{
    UiAssetError,
    descriptors::{self as assets, AssetKind},
};
use photocraft_text::FontBootstrapError;
use sha2::{Digest, Sha256};

pub(super) fn checked_hash(bytes: &[u8], check: &mut dyn FnMut(u64) -> Result<(), FontBootstrapError>) -> Result<[u8; 32], UiAssetError> {
    let mut hash = Sha256::new();
    for chunk in bytes.chunks(65536) {
        check(0).map_err(UiAssetError::Text)?;
        hash.update(chunk);
    }
    Ok(hash.finalize().into())
}

pub(super) fn validate_pack(pack: &[u8], check: &mut dyn FnMut(u64) -> Result<(), FontBootstrapError>) -> Result<(), UiAssetError> {
    if u64::try_from(pack.len()).map_err(|_| UiAssetError::Length)? != assets::PACK_BYTES {
        return Err(UiAssetError::Length);
    }
    if checked_hash(pack, check)? != assets::PACK_SHA256 {
        return Err(UiAssetError::PackHash);
    }
    let mut next = 0_u64;
    let mut fonts = 0_u32;
    let mut catalogs = 0_u32;
    for d in &assets::DESCRIPTORS {
        check(0).map_err(UiAssetError::Text)?;
        if d.offset != next {
            return Err(UiAssetError::Range(d.id));
        }
        next = d.offset.checked_add(d.length).ok_or(UiAssetError::Range(d.id))?;
        let start = usize::try_from(d.offset).map_err(|_| UiAssetError::Range(d.id))?;
        let end = usize::try_from(next).map_err(|_| UiAssetError::Range(d.id))?;
        let bytes = pack.get(start..end).ok_or(UiAssetError::Range(d.id))?;
        if checked_hash(bytes, check)? != d.sha256 {
            return Err(UiAssetError::AssetHash(d.id));
        }
        match d.kind {
            AssetKind::Font => {
                fonts = fonts.checked_add(1).ok_or(UiAssetError::Metadata(d.id))?;
                let meta = d.font.as_ref().ok_or(UiAssetError::Metadata(d.id))?;
                if d.catalog.is_some() || meta.index != 0 || !f32::from_bits(meta.scale_bits).is_finite() {
                    return Err(UiAssetError::Metadata(d.id));
                }
            }
            AssetKind::Catalog => {
                catalogs = catalogs.checked_add(1).ok_or(UiAssetError::Metadata(d.id))?;
                if d.font.is_some() {
                    return Err(UiAssetError::Metadata(d.id));
                }
                let meta = d.catalog.as_ref().ok_or(UiAssetError::Metadata(d.id))?;
                let text = std::str::from_utf8(bytes).map_err(|_| UiAssetError::CatalogUtf8(d.id))?;
                validate_catalog(text, d.id, meta.max_lines, meta.max_line_bytes, meta.max_entries, check)?;
            }
        }
    }
    if next != assets::PACK_BYTES || fonts != 7 || catalogs != 14 {
        return Err(UiAssetError::Length);
    }
    Ok(())
}

pub(super) fn validate_catalog(
    text: &str,
    id: &'static str,
    max_lines: u32,
    max_line_bytes: u32,
    max_entries: u32,
    check: &mut dyn FnMut(u64) -> Result<(), FontBootstrapError>,
) -> Result<(), UiAssetError> {
    let mut lines = 0_u32;
    let mut entries = 0_u32;
    for (line_no, line) in text.lines().enumerate() {
        if line_no % 64 == 0 {
            check(0).map_err(UiAssetError::Text)?
        }
        lines = lines.checked_add(1).ok_or(UiAssetError::CatalogLimit(id))?;
        if line.len() > usize::try_from(max_line_bytes).map_err(|_| UiAssetError::CatalogLimit(id))? {
            return Err(UiAssetError::CatalogLimit(id));
        }
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        match (cols.next(), cols.next(), cols.next(), cols.next()) {
            (Some(_), Some(source), Some(translated), None) if !source.is_empty() && !translated.is_empty() => {}
            _ => return Err(UiAssetError::CatalogSyntax(id)),
        }
        entries = entries.checked_add(1).ok_or(UiAssetError::CatalogLimit(id))?;
    }
    if lines > max_lines || entries > max_entries {
        return Err(UiAssetError::CatalogLimit(id));
    }
    Ok(())
}
