//! The AI service API key, kept in the operating system's credential store.
//!
//! Preferences are plain JSON on disk and readable over the agent interface, so the key never goes
//! there. It lives in Windows Credential Manager, the macOS keychain or the Linux kernel keyring,
//! under this app's name; the shell loads it at launch and hands it to the engine, which holds it in
//! memory only ([`photocraft_engine::remote::set_key`]). On the web there is no store, so the key is
//! unavailable there (the remote features need sockets the browser build does not have).

/// Why a key could not be read or written, in words for the status line.
pub type SecretError = String;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::SecretError;

    const SERVICE: &str = "ai.storyteller.photocraft";
    const ACCOUNT: &str = "ai-service-api-key";
    const ENHANCER_ACCOUNT: &str = "prompt-enhancer-api-key";

    fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, account).map_err(|e| format!("the system credential store is not available: {e}"))
    }

    /// The saved key; `Ok(None)` when none is saved.
    fn load_from(account: &str) -> Result<Option<String>, SecretError> {
        match entry(account)?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("could not read the saved key: {e}")),
        }
    }

    fn save_to(account: &str, key: &str) -> Result<(), SecretError> {
        entry(account)?.set_password(key).map_err(|e| format!("could not save the key: {e}"))
    }

    /// Removes the saved key (no error when there is none).
    fn delete_from(account: &str) -> Result<(), SecretError> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("could not remove the key: {e}")),
        }
    }

    pub fn load() -> Result<Option<String>, SecretError> {
        load_from(ACCOUNT)
    }
    pub fn save(key: &str) -> Result<(), SecretError> {
        save_to(ACCOUNT, key)
    }
    pub fn delete() -> Result<(), SecretError> {
        delete_from(ACCOUNT)
    }
    pub fn load_enhancer() -> Result<Option<String>, SecretError> {
        load_from(ENHANCER_ACCOUNT)
    }
    pub fn save_enhancer(key: &str) -> Result<(), SecretError> {
        save_to(ENHANCER_ACCOUNT, key)
    }
    pub fn delete_enhancer() -> Result<(), SecretError> {
        delete_from(ENHANCER_ACCOUNT)
    }
}

#[cfg(target_arch = "wasm32")]
mod native {
    use super::SecretError;

    const NONE: &str = "the web build has no credential store";
    pub fn load() -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    pub fn save(_: &str) -> Result<(), SecretError> {
        Err(NONE.into())
    }
    pub fn delete() -> Result<(), SecretError> {
        Ok(())
    }
    pub fn load_enhancer() -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    pub fn save_enhancer(_: &str) -> Result<(), SecretError> {
        Err(NONE.into())
    }
    pub fn delete_enhancer() -> Result<(), SecretError> {
        Ok(())
    }
}

pub use native::{delete, delete_enhancer, load, load_enhancer, save, save_enhancer};

/// Loads the saved key into the engine, once, at launch. A key already set (the environment
/// variable path) is left alone when nothing is saved.
pub fn load_into_engine() -> Result<(), SecretError> {
    if let Some(key) = load()? {
        photocraft_engine::remote::set_key(Some(&key));
    }
    if let Some(key) = load_enhancer()? {
        photocraft_engine::remote::set_enhancer_key(Some(&key));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_wasm_free_api_compiles_and_blank_keys_are_not_loaded_as_keys() {
        // The real store is not touched in unit tests; this only pins the engine-side contract.
        photocraft_engine::remote::set_key(Some("  "));
        assert!(!photocraft_engine::remote::has_key());
    }
}
