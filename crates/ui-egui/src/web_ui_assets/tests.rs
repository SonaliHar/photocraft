use super::{UiAssetError, descriptors::*, validation::*};
use photocraft_text::FontBootstrapError;
use std::collections::BTreeSet;

// Reconstruct trusted inputs already compiled by an ordinary native test.
// No web generation, ignored output, registry path, or optional font checkout is needed.
static PACK: std::sync::OnceLock<Result<Vec<u8>, UiAssetError>> = std::sync::OnceLock::new();

fn native_font_bytes(role: FontRole) -> Result<&'static [u8], UiAssetError> {
    let name = match role {
        FontRole::InterMedium => "Inter-Medium",
        FontRole::InterSemiBold => "Inter-SemiBold",
        FontRole::JetBrainsMono => "JetBrainsMono",
        FontRole::Hack => "Hack",
        FontRole::UbuntuLight => "Ubuntu-Light",
        FontRole::NotoEmoji => "NotoEmoji-Regular",
        FontRole::EmojiIcon => "emoji-icon-font",
    };
    let definitions = crate::theme::native_font_definitions();
    let face = definitions.font_data.get(name).ok_or(UiAssetError::Metadata("native-font"))?;
    match &face.font {
        std::borrow::Cow::Borrowed(bytes) => Ok(*bytes),
        std::borrow::Cow::Owned(_) => Err(UiAssetError::Metadata("native-font-borrow")),
    }
}

// Match the generator's shared canonical LF policy without rewriting checkout files.
fn canonical_catalog_bytes(text: &str) -> Result<Vec<u8>, UiAssetError> {
    use super::text_canonical::{TextCanonicalError, canonical_text};
    canonical_text(text.as_bytes(), 512 << 10).map_err(|error| match error {
        TextCanonicalError::Length => UiAssetError::CatalogLimit("native-catalog"),
        TextCanonicalError::Utf8 => UiAssetError::CatalogUtf8("native-catalog"),
        TextCanonicalError::Allocation => UiAssetError::Allocation,
    })
}
fn reconstruct_native_pack() -> Result<Vec<u8>, UiAssetError> {
    let length = usize::try_from(PACK_BYTES).map_err(|_| UiAssetError::Length)?;
    let mut pack = Vec::new();
    pack.try_reserve_exact(length).map_err(|_| UiAssetError::Allocation)?;
    for descriptor in &DESCRIPTORS {
        let bytes = match descriptor.kind {
            AssetKind::Font => std::borrow::Cow::Borrowed(native_font_bytes(descriptor.font.ok_or(UiAssetError::Metadata(descriptor.id))?.role)?),
            AssetKind::Catalog => {
                let meta = descriptor.catalog.ok_or(UiAssetError::Metadata(descriptor.id))?;
                std::borrow::Cow::Owned(canonical_catalog_bytes(
                    crate::i18n::LANGUAGES.iter().find(|language| language.code == meta.code).ok_or(UiAssetError::Metadata(descriptor.id))?.text().as_str(),
                )?)
            }
        };
        if u64::try_from(pack.len()).map_err(|_| UiAssetError::Length)? != descriptor.offset
            || u64::try_from(bytes.len()).map_err(|_| UiAssetError::Length)? != descriptor.length
            || pack.len().checked_add(bytes.len()).is_none_or(|end| end > length)
        {
            return Err(UiAssetError::Range(descriptor.id));
        }
        pack.extend_from_slice(&bytes);
    }
    if pack.len() != length {
        return Err(UiAssetError::Length);
    }
    Ok(pack)
}

fn trusted_pack() -> Result<&'static [u8], UiAssetError> {
    PACK.get_or_init(reconstruct_native_pack).as_ref().map(Vec::as_slice).map_err(Clone::clone)
}

#[test]
fn exact_payload_and_catalog_registry_are_complete() {
    let pack = trusted_pack().expect("ordinary native inputs reconstruct the exact pinned pack");
    let mut checks = 0;
    validate_pack(pack, &mut |bytes| {
        assert_eq!(bytes, 0);
        checks += 1;
        Ok(())
    })
    .expect("exact trusted pack");
    assert!(checks > 100, "validation must remain interruptible during hashing/parsing");
    let actual: BTreeSet<_> = DESCRIPTORS.iter().filter_map(|d| d.catalog.map(|c| c.code)).collect();
    let expected: BTreeSet<_> = crate::i18n::LANGUAGES.iter().filter(|l| l.code != "en").map(|l| l.code).collect();
    assert_eq!(actual, expected);
    assert_eq!(
        crate::i18n::LANGUAGES.iter().map(|l| l.code).collect::<Vec<_>>(),
        ["en", "ja", "zh-hans", "zh-hant", "es", "ru", "uk", "sr", "cs", "fr", "id", "ko", "pl", "de", "pt-br", "el", "nl", "it", "hu", "tr", "vi"]
    );
}

#[test]
fn reconstructed_definitions_equal_exact_native_fonts() {
    let reconstructed = super::font_definitions::definitions(native_font_bytes).expect("ready definitions");
    let original = crate::theme::native_font_definitions();
    assert_eq!(reconstructed, original, "all face bytes/index/full FontTweak/family orders");
    for face in reconstructed.font_data.values() {
        assert!(matches!(face.font, std::borrow::Cow::Borrowed(_)), "context definitions borrow published bytes");
        let clone = std::sync::Arc::new((**face).clone());
        assert_eq!(face.font.as_ptr(), clone.font.as_ptr(), "sizing metadata copies must not duplicate payload");
    }
}

#[test]
fn truncated_corrupt_and_cancelled_pack_validation_are_named() {
    let pack = trusted_pack().expect("exact native pack");
    assert!(matches!(validate_pack(&pack[..pack.len() - 1], &mut |_| Ok(())), Err(UiAssetError::Length)));
    let mut corrupt = pack.to_vec();
    corrupt[0] ^= 1;
    assert!(matches!(validate_pack(&corrupt, &mut |_| Ok(())), Err(UiAssetError::PackHash)));
    let mut count = 0;
    let result = validate_pack(pack, &mut |_| {
        count += 1;
        if count == 2 { Err(FontBootstrapError::Cancelled) } else { Ok(()) }
    });
    assert!(matches!(result, Err(UiAssetError::Text(FontBootstrapError::Cancelled))));
    assert_eq!(count, 2);
}

#[test]
fn malformed_oversized_and_interrupted_catalogs_refuse_startup() {
    assert!(matches!(validate_catalog("x\ty", "test", 2, 20, 2, &mut |_| Ok(())), Err(UiAssetError::CatalogSyntax("test"))));
    assert!(matches!(validate_catalog("\tx\ty", "test", 2, 2, 2, &mut |_| Ok(())), Err(UiAssetError::CatalogLimit("test"))));
    assert!(matches!(validate_catalog("\tx\ty\n\tx\tz", "test", 2, 20, 1, &mut |_| Ok(())), Err(UiAssetError::CatalogLimit("test"))));
    assert!(matches!(
        validate_catalog("\tx\ty", "test", 2, 20, 2, &mut |_| Err(FontBootstrapError::Timeout)),
        Err(UiAssetError::Text(FontBootstrapError::Timeout))
    ));
}

#[test]
fn native_fallible_locale_and_font_adapters_preserve_behavior() {
    for language in crate::i18n::Lang::all() {
        let ready = crate::i18n::Lang::try_from_code(language.code()).expect("native").expect("registered");
        assert_eq!(ready, language);
        assert_eq!(crate::i18n::tr(ready, "File"), crate::i18n::tr(language, "File"));
    }
    assert_eq!(crate::type_tool::try_families().expect("native"), crate::type_tool::families());
    for family in ["Inter", "JetBrains Mono"] {
        assert_eq!(crate::type_tool::try_styles(family).expect("native"), crate::type_tool::styles(family));
    }
}

#[test]
fn checkout_line_endings_have_identical_canonical_catalog_bytes() {
    let lf = canonical_catalog_bytes("# source\n\tFile\tFichier\n\tEdit\tÉdition\n").expect("LF");
    let crlf = canonical_catalog_bytes("# source\r\n\tFile\tFichier\r\n\tEdit\tÉdition\r\n").expect("CRLF");
    assert_eq!(lf, crlf);
    assert_eq!(canonical_catalog_bytes("literal\rcarriage\n").expect("bare CR"), b"literal\rcarriage\n".as_slice());
    assert_eq!(canonical_catalog_bytes("").expect("empty"), b"".as_slice());
}

#[test]
fn every_catalog_matches_pinned_lf_bytes_from_both_checkout_forms() {
    for descriptor in DESCRIPTORS.iter().filter(|descriptor| descriptor.kind == AssetKind::Catalog) {
        let meta = descriptor.catalog.expect("catalog metadata");
        let input = crate::i18n::LANGUAGES.iter().find(|language| language.code == meta.code).expect("registered catalog").text();
        let lf = canonical_catalog_bytes(&input).expect("canonical LF");
        let mut crlf = Vec::new();
        for byte in &lf {
            if *byte == b'\n' {
                crlf.push(b'\r');
            }
            crlf.push(*byte);
        }
        let crlf = std::str::from_utf8(&crlf).expect("CRLF expansion retains UTF8");
        assert_eq!(canonical_catalog_bytes(crlf).expect("canonical CRLF"), lf, "{}", descriptor.id);
        assert_eq!(lf.len() as u64, descriptor.length, "{}", descriptor.id);
        assert_eq!(checked_hash(&lf, &mut |_| Ok(())).expect("hash"), descriptor.sha256, "{}", descriptor.id);
        let mut changed = lf.clone();
        *changed.first_mut().expect("nonempty catalog") ^= 1;
        assert_ne!(
            checked_hash(&changed, &mut |_| Ok(())).expect("changed hash"),
            descriptor.sha256,
            "semantic byte drift must remain visible: {}",
            descriptor.id
        );
    }
}
