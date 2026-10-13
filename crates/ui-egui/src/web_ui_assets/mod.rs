//! Web bootstrap errors remain portable; asset-bearing code is wasm-only.
#[derive(Debug, Clone)]
pub enum UiAssetError {
    NotReady,
    AlreadyPublished,
    Length,
    PackHash,
    Range(&'static str),
    AssetHash(&'static str),
    Metadata(&'static str),
    CatalogUtf8(&'static str),
    CatalogSyntax(&'static str),
    CatalogLimit(&'static str),
    Allocation,
    Text(photocraft_text::FontBootstrapError),
}

impl std::fmt::Display for UiAssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady => f.write_str("web-assets.not-ready"),
            Self::AlreadyPublished => f.write_str("web-assets.already-published"),
            Self::Length => f.write_str("web-assets.length"),
            Self::PackHash => f.write_str("web-assets.pack-hash"),
            Self::Range(id) => write!(f, "web-assets.range: {id}"),
            Self::AssetHash(id) => write!(f, "web-assets.asset-hash: {id}"),
            Self::Metadata(id) => write!(f, "web-assets.metadata: {id}"),
            Self::CatalogUtf8(id) => write!(f, "web-assets.catalog-utf8: {id}"),
            Self::CatalogSyntax(id) => write!(f, "web-assets.catalog-syntax: {id}"),
            Self::CatalogLimit(id) => write!(f, "web-assets.catalog-limit: {id}"),
            Self::Allocation => f.write_str("web-assets.allocation"),
            Self::Text(error) => write!(f, "web-assets.text: {error}"),
        }
    }
}
impl std::error::Error for UiAssetError {}

#[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))]
mod bank;
#[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))]
pub use bank::{ReadyUiAssets, WebUiAssets, ready, text_provider};
#[cfg(any(test, all(target_arch = "wasm32", feature = "web_external_assets")))]
pub mod descriptors {
    include!("trusted_descriptors.rs");
}

#[cfg(any(test, all(target_arch = "wasm32", feature = "web_external_assets")))]
mod font_definitions;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

#[cfg(test)]
mod text_canonical;

#[cfg(any(test, all(target_arch = "wasm32", feature = "web_external_assets")))]
mod validation;
