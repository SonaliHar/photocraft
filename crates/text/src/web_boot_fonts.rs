//! Validated web font injection and the native compatibility adapter.
//! Common errors/native try_shared are available on all targets. Asset-bearing
//! types and provider exist only on the external-delivery WASM branch.
use crate::TextEngine;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebCustomFace {
    InterRegular,
    InterMedium,
    InterSemiBold,
    JetBrainsMono,
}
impl WebCustomFace {
    pub const fn id(self) -> &'static str {
        match self {
            Self::InterRegular => "font/Inter-Regular",
            Self::InterMedium => "font/Inter-Medium",
            Self::InterSemiBold => "font/Inter-SemiBold",
            Self::JetBrainsMono => "font/JetBrainsMono-Regular",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontBootstrapError {
    ProviderMissing,
    NotReady,
    AlreadyRegistered,
    FaceIdentity(WebCustomFace),
    FaceRegistration(WebCustomFace),
    Allocation,
    Budget,
    Cancelled,
    Timeout,
}
impl FontBootstrapError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ProviderMissing => "font-assets.provider-missing",
            Self::NotReady => "font-assets.not-ready",
            Self::AlreadyRegistered => "font-assets.provider-already-registered",
            Self::FaceIdentity(_) => "font-assets.face-identity",
            Self::FaceRegistration(_) => "font-assets.face-registration",
            Self::Allocation => "font-assets.allocation",
            Self::Budget => "font-assets.budget",
            Self::Cancelled => "font-assets.cancelled",
            Self::Timeout => "font-assets.timeout",
        }
    }
}
impl std::fmt::Display for FontBootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())?;
        if let Self::FaceIdentity(role) | Self::FaceRegistration(role) = self {
            write!(f, ": {}", role.id())?;
        }
        Ok(())
    }
}
impl std::error::Error for FontBootstrapError {}

#[cfg(not(all(target_arch = "wasm32", feature = "web_external_bundled_fonts")))]
pub fn try_shared() -> Result<&'static Mutex<TextEngine>, FontBootstrapError> {
    Ok(crate::shared())
}

#[cfg(any(test, all(target_arch = "wasm32", feature = "web_external_bundled_fonts")))]
mod external {
    use super::{FontBootstrapError, Mutex, TextEngine, WebCustomFace};
    use parley::fontique::Blob;
    use sha2::{Digest, Sha256};
    use skrifa::{MetadataProvider, string::StringId};
    use std::sync::{Arc, OnceLock};

    struct ValidatedFace {
        bytes: Arc<Vec<u8>>,
        blob: Blob<u8>,
    }
    pub struct WebCustomFonts {
        medium: ValidatedFace,
        semibold: ValidatedFace,
        mono: ValidatedFace,
    }
    impl WebCustomFonts {
        /// Input slices are the three exact checked pack ranges. This factory
        /// owns/charges each custom copy once; Blob clones are not byte charges.
        pub fn prepare_from_boot_faces(
            medium: &[u8],
            semibold: &[u8],
            mono: &[u8],
            charge: &mut dyn FnMut(u64) -> Result<(), FontBootstrapError>,
        ) -> Result<Self, FontBootstrapError> {
            Ok(Self {
                medium: prepare_face(WebCustomFace::InterMedium, medium, charge)?,
                semibold: prepare_face(WebCustomFace::InterSemiBold, semibold, charge)?,
                mono: prepare_face(WebCustomFace::JetBrainsMono, mono, charge)?,
            })
        }
        pub fn medium_bytes(&self) -> &[u8] {
            self.medium.bytes.as_slice()
        }
        pub fn semibold_bytes(&self) -> &[u8] {
            self.semibold.bytes.as_slice()
        }
        pub fn mono_bytes(&self) -> &[u8] {
            self.mono.bytes.as_slice()
        }
        pub(crate) fn ordered_blobs(&self) -> [(WebCustomFace, Blob<u8>); 3] {
            [
                (WebCustomFace::InterMedium, self.medium.blob.clone()),
                (WebCustomFace::InterSemiBold, self.semibold.blob.clone()),
                (WebCustomFace::JetBrainsMono, self.mono.blob.clone()),
            ]
        }
    }
    fn prepare_face(
        role: WebCustomFace,
        bytes: &[u8],
        charge: &mut dyn FnMut(u64) -> Result<(), FontBootstrapError>,
    ) -> Result<ValidatedFace, FontBootstrapError> {
        let (length, postscript) = match role {
            WebCustomFace::InterMedium => (417300, "Inter-Medium"),
            WebCustomFace::InterSemiBold => (419744, "Inter-SemiBold"),
            WebCustomFace::JetBrainsMono => (270224, "JetBrainsMono-Regular"),
            WebCustomFace::InterRegular => return Err(FontBootstrapError::FaceIdentity(role)),
        };
        charge(0)?;
        if bytes.len() != length {
            return Err(FontBootstrapError::FaceIdentity(role));
        }
        let mut hash = Sha256::new();
        for chunk in bytes.chunks(4096) {
            charge(0)?;
            hash.update(chunk);
        }
        let actual: [u8; 32] = hash.finalize().into();
        if actual != expected_hash(role) {
            return Err(FontBootstrapError::FaceIdentity(role));
        }
        let face = skrifa::FontRef::new(bytes).map_err(|_| FontBootstrapError::FaceIdentity(role))?;
        let name = face.localized_strings(StringId::POSTSCRIPT_NAME).english_or_first().ok_or(FontBootstrapError::FaceIdentity(role))?;
        if !name.chars().eq(postscript.chars()) || !face.axes().is_empty() {
            return Err(FontBootstrapError::FaceIdentity(role));
        }
        // Fixed authenticated assets only. Charge explicit owned byte buffers;
        // Arc/parser/index allocator overhead remains separately measured.
        charge(u64::try_from(length).map_err(|_| FontBootstrapError::Budget)?)?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(length).map_err(|_| FontBootstrapError::Allocation)?;
        copy.extend_from_slice(bytes);
        charge(0)?;
        let bytes = Arc::new(copy);
        let blob = Blob::new(bytes.clone());
        Ok(ValidatedFace { bytes, blob })
    }
    fn expected_hash(role: WebCustomFace) -> [u8; 32] {
        match role {
            WebCustomFace::InterRegular => [
                0x40, 0xd6, 0x92, 0xfc, 0xe1, 0x88, 0xe4, 0x47, 0x1e, 0x2b, 0x3c, 0xba, 0x93, 0x7b, 0xe9, 0x67, 0x87, 0x8f, 0x63, 0x1a, 0xd3, 0xeb, 0xbb, 0xdc,
                0xd5, 0x87, 0x68, 0x7c, 0x7e, 0xbe, 0x0c, 0x82,
            ],
            WebCustomFace::InterMedium => [
                0x97, 0xad, 0x80, 0x6f, 0x52, 0x6e, 0x41, 0x54, 0x6d, 0x46, 0x36, 0x5b, 0xb3, 0xa3, 0x93, 0x14, 0x5f, 0x75, 0xb7, 0xb1, 0x56, 0x89, 0x13, 0xdb,
                0x74, 0x54, 0x9a, 0xd8, 0xb8, 0xdb, 0xa8, 0x72,
            ],
            WebCustomFace::InterSemiBold => [
                0x78, 0xa8, 0x43, 0xfa, 0xde, 0x9d, 0x46, 0x12, 0xa5, 0x56, 0x73, 0x02, 0xfb, 0x59, 0x5b, 0x56, 0x97, 0x6e, 0xb5, 0xfc, 0xeb, 0xf4, 0xfe, 0xa5,
                0xa5, 0x91, 0x2d, 0x63, 0x8b, 0xaf, 0xcd, 0xe3,
            ],
            WebCustomFace::JetBrainsMono => [
                0xe6, 0xfd, 0x0d, 0x7e, 0x91, 0x55, 0x0b, 0x3e, 0xd2, 0xb7, 0x35, 0xd4, 0x31, 0x24, 0x74, 0x36, 0x2c, 0x47, 0x16, 0xed, 0xc4, 0xfc, 0x05, 0x77,
                0xa0, 0xf6, 0x1e, 0xd7, 0x82, 0xd5, 0xae, 0xd1,
            ],
        }
    }
    pub struct WebTextRefs<'a> {
        pub faces: &'a WebCustomFonts,
        pub shared: &'a Mutex<TextEngine>,
    }
    pub type WebFontProvider = fn() -> Result<WebTextRefs<'static>, FontBootstrapError>;
    static PROVIDER: OnceLock<WebFontProvider> = OnceLock::new();
    pub fn install_web_font_provider(provider: WebFontProvider) -> Result<(), FontBootstrapError> {
        PROVIDER.set(provider).map_err(|_| FontBootstrapError::AlreadyRegistered)
    }
    pub(crate) fn ready_refs() -> Result<WebTextRefs<'static>, FontBootstrapError> {
        let provider = PROVIDER.get().ok_or(FontBootstrapError::ProviderMissing)?;
        provider()
    }
    pub fn try_shared() -> Result<&'static Mutex<TextEngine>, FontBootstrapError> {
        Ok(ready_refs()?.shared)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn exact_faces_are_owned_once_and_fail_before_readiness() {
            let _borrow_faces = |refs: WebTextRefs<'_>| refs.faces.medium_bytes().len();
            let medium = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
            let semibold = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
            let mono = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
            assert!(matches!(try_shared(), Err(FontBootstrapError::ProviderMissing)));
            install_web_font_provider(|| Err(FontBootstrapError::NotReady)).expect("single test provider");
            assert!(matches!(try_shared(), Err(FontBootstrapError::NotReady)));
            assert!(matches!(TextEngine::try_new(&mut |_| Ok(())), Err(FontBootstrapError::NotReady)));
            assert!(matches!(crate::FontDb::try_new(&mut |_| Ok(())), Err(FontBootstrapError::NotReady)));
            assert_eq!(install_web_font_provider(|| Err(FontBootstrapError::NotReady)), Err(FontBootstrapError::AlreadyRegistered));
            let mut charged = 0_u64;
            let faces = WebCustomFonts::prepare_from_boot_faces(medium, semibold, mono, &mut |bytes| {
                charged += bytes;
                Ok(())
            })
            .expect("exact fixed bundled faces");
            assert_eq!(charged, 1_107_268);
            assert_eq!(faces.medium_bytes(), medium);
            assert_eq!(faces.semibold_bytes(), semibold);
            assert_eq!(faces.mono_bytes(), mono);
            let blobs_a = faces.ordered_blobs();
            let blobs_b = faces.ordered_blobs();
            assert_eq!(blobs_a[0].1.id(), blobs_b[0].1.id());
            let mut prepared = TextEngine::prepare_from_boot_faces(&faces, &mut |bytes| {
                charged += bytes;
                Ok(())
            })
            .expect("local constructor works even though provider is NotReady");
            assert_eq!(charged, 1_518_908);
            #[cfg(not(all(target_arch = "wasm32", feature = "web_external_bundled_fonts")))]
            {
                let mut original = TextEngine::new();
                assert_eq!(prepared.fonts.faces(crate::fonts::DEFAULT_FAMILY), original.fonts.faces(crate::fonts::DEFAULT_FAMILY));
                assert_eq!(prepared.fonts.faces(crate::fonts::MONO_FAMILY), original.fonts.faces(crate::fonts::MONO_FAMILY));
            }
            assert_eq!(prepared.fonts.faces(crate::fonts::MONO_FAMILY).len(), 1);
            assert!(matches!(
                WebCustomFonts::prepare_from_boot_faces(&medium[..20], semibold, mono, &mut |_| Ok(())),
                Err(FontBootstrapError::FaceIdentity(WebCustomFace::InterMedium))
            ));
            let mut damaged = medium.to_vec();
            damaged[100] ^= 1;
            assert!(matches!(
                WebCustomFonts::prepare_from_boot_faces(&damaged, semibold, mono, &mut |_| Ok(())),
                Err(FontBootstrapError::FaceIdentity(WebCustomFace::InterMedium))
            ));
            assert!(matches!(
                WebCustomFonts::prepare_from_boot_faces(medium, semibold, mono, &mut |bytes| if bytes == 0 { Ok(()) } else { Err(FontBootstrapError::Budget) }),
                Err(FontBootstrapError::Budget)
            ));
            assert!(matches!(
                WebCustomFonts::prepare_from_boot_faces(medium, semibold, mono, &mut |_| Err(FontBootstrapError::Cancelled)),
                Err(FontBootstrapError::Cancelled)
            ));
            assert!(matches!(
                WebCustomFonts::prepare_from_boot_faces(medium, semibold, mono, &mut |_| Err(FontBootstrapError::Timeout)),
                Err(FontBootstrapError::Timeout)
            ));
        }
    }
}
#[cfg(all(test, not(all(target_arch = "wasm32", feature = "web_external_bundled_fonts"))))]
pub use external::WebCustomFonts;
#[cfg(any(test, all(target_arch = "wasm32", feature = "web_external_bundled_fonts")))]
pub(crate) use external::ready_refs;
#[cfg(all(target_arch = "wasm32", feature = "web_external_bundled_fonts"))]
pub use external::{WebCustomFonts, WebFontProvider, WebTextRefs, install_web_font_provider, try_shared};

#[cfg(all(test, not(all(target_arch = "wasm32", feature = "web_external_bundled_fonts"))))]
mod native_tests {
    #[test]
    fn fallible_adapter_preserves_native_shared_engine() {
        assert!(std::ptr::eq(super::try_shared().expect("native bundled engine"), crate::shared()));
    }

    #[test]
    fn readiness_errors_have_stable_named_codes() {
        use super::{FontBootstrapError as Error, WebCustomFace};
        assert_eq!(Error::NotReady.to_string(), "font-assets.not-ready");
        assert_eq!(Error::FaceIdentity(WebCustomFace::InterMedium).to_string(), "font-assets.face-identity: font/Inter-Medium");
    }
}
