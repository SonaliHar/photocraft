//! Fixed packaged asset locations. No downloaded URL or manifest is authoritative.
pub const ASSET_DIRECTORY: &str = "ui-assets/";
pub const CACHE_KEY: &str = "photocraft-assets-current-schema-1";

pub fn valid_pack_basename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 160
        && name != "."
        && name != ".."
        && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.'))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn compiled_pack_name_cannot_supply_a_path_query_or_origin() {
        assert!(valid_pack_basename("assetpack-v1-1234.bin"));
        assert_eq!(ASSET_DIRECTORY, "ui-assets/");
        assert_eq!(CACHE_KEY, "photocraft-assets-current-schema-1");
        for invalid in ["", ".", "..", "../pack.bin", "/pack.bin", "https://evil.test/x", "pack.bin?q=1", "pack.bin#x", "pack\\x.bin"] {
            assert!(!valid_pack_basename(invalid), "{invalid}");
        }
    }
}
