//! Validated web fonts and catalogs staged before the first app/context frame.
//! No ready asset reference exists until the single BANK.set below succeeds.
use super::{
    UiAssetError,
    descriptors::{self as assets, AssetKind, FontRole},
};
use egui::FontDefinitions;
use photocraft_text::fonts::WebCustomFonts;
use photocraft_text::{FontBootstrapError, TextEngine, WebTextRefs};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Mutex, OnceLock};

static BANK: OnceLock<WebUiAssets> = OnceLock::new();

pub struct WebUiAssets {
    // Catalog bytes remain in this exact verified pack. Custom fonts are not
    // duplicated here: their opaque Arc<Vec> carriers also back the UI faces.
    pack: Vec<u8>,
    custom: WebCustomFonts,
    defaults: BTreeMap<FontRole, Vec<u8>>,
    catalogs: BTreeMap<&'static str, CatalogSlot>,
    engine: Mutex<TextEngine>,
}
struct CatalogSlot {
    range: Range<usize>,
    source: OnceLock<Result<&'static str, UiAssetError>>,
    parsed: OnceLock<crate::i18n::catalog::Catalog>,
}

// Errors have static identifiers, bounded trusted asset IDs only, and stable
// Display codes; Sol6 loader presents these as textContent before WebRunner.
#[derive(Clone, Copy)]
pub struct ReadyUiAssets {
    bank: &'static WebUiAssets,
}
impl ReadyUiAssets {
    pub fn font_definitions(self) -> Result<FontDefinitions, UiAssetError> {
        self.bank.font_definitions()
    }
    pub(crate) fn catalog(self, code: &str) -> Result<crate::i18n::ReadyCatalog, UiAssetError> {
        self.bank.catalog(code)
    }
}
pub fn ready() -> Result<ReadyUiAssets, UiAssetError> {
    BANK.get().map(|bank| ReadyUiAssets { bank }).ok_or(UiAssetError::NotReady)
}

pub fn text_provider() -> Result<WebTextRefs<'static>, FontBootstrapError> {
    let bank = BANK.get().ok_or(FontBootstrapError::NotReady)?;
    Ok(WebTextRefs { faces: &bank.custom, shared: &bank.engine })
}

impl WebUiAssets {
    pub fn prepare(pack: Vec<u8>, charge: &mut dyn FnMut(u64) -> Result<(), FontBootstrapError>) -> Result<Self, UiAssetError> {
        // Loader charges decoded pack allocation before reserve; do not charge
        // it a second time on ownership transfer into this staging bank.
        super::validation::validate_pack(&pack, charge)?;
        let mut defaults = BTreeMap::new();
        let mut catalogs = BTreeMap::new();
        let mut custom_ranges = BTreeMap::new();
        let mut next = 0_u64;
        for d in &assets::DESCRIPTORS {
            charge(0).map_err(UiAssetError::Text)?;
            if d.offset != next {
                return Err(UiAssetError::Range(d.id));
            }
            next = d.offset.checked_add(d.length).ok_or(UiAssetError::Range(d.id))?;
            let start = usize::try_from(d.offset).map_err(|_| UiAssetError::Range(d.id))?;
            let end = usize::try_from(next).map_err(|_| UiAssetError::Range(d.id))?;
            let bytes = pack.get(start..end).ok_or(UiAssetError::Range(d.id))?;
            match d.kind {
                AssetKind::Font => {
                    let meta = d.font.as_ref().ok_or(UiAssetError::Metadata(d.id))?;
                    if d.catalog.is_some() || meta.index != 0 {
                        return Err(UiAssetError::Metadata(d.id));
                    }
                    match meta.role {
                        FontRole::InterMedium | FontRole::InterSemiBold | FontRole::JetBrainsMono => {
                            if custom_ranges.insert(meta.role, start..end).is_some() {
                                return Err(UiAssetError::Metadata(d.id));
                            }
                        }
                        _ => {
                            charge(d.length).map_err(UiAssetError::Text)?;
                            let mut owned = Vec::new();
                            owned.try_reserve_exact(bytes.len()).map_err(|_| UiAssetError::Allocation)?;
                            owned.extend_from_slice(bytes);
                            if defaults.insert(meta.role, owned).is_some() {
                                return Err(UiAssetError::Metadata(d.id));
                            }
                        }
                    }
                }
                AssetKind::Catalog => {
                    let meta = d.catalog.as_ref().ok_or(UiAssetError::Metadata(d.id))?;
                    if d.font.is_some() {
                        return Err(UiAssetError::Metadata(d.id));
                    }
                    if catalogs.insert(meta.code, CatalogSlot { range: start..end, source: OnceLock::new(), parsed: OnceLock::new() }).is_some() {
                        return Err(UiAssetError::Metadata(d.id));
                    }
                }
            }
        }
        if next != assets::PACK_BYTES || defaults.len() != 4 || custom_ranges.len() != 3 || catalogs.len() != 14 {
            return Err(UiAssetError::Length);
        }
        let face = |role| custom_ranges.get(&role).and_then(|r| pack.get(r.clone())).ok_or(UiAssetError::Metadata("custom-font"));
        let custom =
            WebCustomFonts::prepare_from_boot_faces(face(FontRole::InterMedium)?, face(FontRole::InterSemiBold)?, face(FontRole::JetBrainsMono)?, charge)
                .map_err(UiAssetError::Text)?;
        let engine = TextEngine::prepare_from_boot_faces(&custom, charge).map_err(UiAssetError::Text)?;
        Ok(Self { pack, custom, defaults, catalogs, engine: Mutex::new(engine) })
    }

    pub fn publish(self) -> Result<ReadyUiAssets, UiAssetError> {
        BANK.set(self).map_err(|_| UiAssetError::AlreadyPublished)?;
        ready()
    }

    // Borrowing is only possible from the successfully published static bank.
    // FontData::from_static uses Cow::Borrowed, so Arc::make_mut used by font
    // sizing clones small metadata, not the megabytes of face data each install.
    fn font_definitions(&'static self) -> Result<FontDefinitions, UiAssetError> {
        super::font_definitions::definitions(|role| match role {
            FontRole::InterMedium => Ok(self.custom.medium_bytes()),
            FontRole::InterSemiBold => Ok(self.custom.semibold_bytes()),
            FontRole::JetBrainsMono => Ok(self.custom.mono_bytes()),
            _ => self.default_bytes(role),
        })
    }

    fn default_bytes(&'static self, role: FontRole) -> Result<&'static [u8], UiAssetError> {
        self.defaults.get(&role).map(Vec::as_slice).ok_or(UiAssetError::Metadata("default-font"))
    }

    fn catalog(&'static self, code: &str) -> Result<crate::i18n::ReadyCatalog, UiAssetError> {
        let slot = self.catalogs.get(code).ok_or(UiAssetError::Metadata("catalog-code"))?;
        let source = slot
            .source
            .get_or_init(|| {
                let bytes = self.pack.get(slot.range.clone()).ok_or(UiAssetError::Range("catalog"))?;
                std::str::from_utf8(bytes).map_err(|_| UiAssetError::CatalogUtf8("catalog"))
            })
            .as_ref()
            .map_err(Clone::clone)?;
        Ok(crate::i18n::ReadyCatalog { source, parsed: &slot.parsed })
    }
}
