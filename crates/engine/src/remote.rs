//! Remote AI services (Preferences › Integrations): where the service is, whether each feature
//! may use it, and the API key.
//!
//! The URL and the switches are ordinary preferences ([`crate::prefs::Integrations`]). The **key
//! is not**: preferences are written to a plain JSON file and are readable over the agent
//! interface, so the desktop shell keeps the key in the operating system's credential store and
//! hands it to [`set_key`]. The engine holds it in memory only, never writes it anywhere, and
//! never includes it in a message.
//!
//! Without a URL in the preferences, the `PHOTOCRAFT_REMOTE_URL` / `PHOTOCRAFT_REMOTE_KEY`
//! environment variables are used (headless use, scripts). Nothing is configured by default, so
//! nothing leaves the machine until the user sets a service up.

use std::sync::{PoisonError, RwLock};

use photocraft_ml::RemoteConfig;

use crate::Session;

/// The preferences the remote features read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub url: String,
    pub use_for_selection: bool,
    pub use_for_layers: bool,
    pub use_for_fill: bool,
    /// How Object to Layer fills the hole: `"fast"` or a generative model id.
    pub fill_method: String,
    pub fill_improve_detail: bool,
    pub fill_context: u32,
    pub enhancer_url: String,
    pub enhancer_model: String,
}

static SETTINGS: RwLock<Settings> = RwLock::new(Settings { url: String::new(), use_for_selection: true, use_for_layers: true, use_for_fill: true, fill_method: String::new(), fill_improve_detail: false, fill_context: 62, enhancer_url: String::new(), enhancer_model: String::new() });
static ENHANCER_KEY: RwLock<Option<String>> = RwLock::new(None);
static KEY: RwLock<Option<String>> = RwLock::new(None);
/// The fill methods the service offered at the last check (empty until then).
static FILL_METHODS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// What the reference service is known to offer, shown until the service has been asked.
const DEFAULT_FILL_METHODS: [&str; 3] = ["fast", "sdxl", "juggernaut"];

/// The ways to fill a hole that the Preferences picker offers: what the service listed, else the
/// reference service's. The saved choice is always included, so it never silently disappears.
pub fn fill_methods() -> Vec<String> {
    let mut v = FILL_METHODS.read().unwrap_or_else(PoisonError::into_inner).clone();
    if v.is_empty() {
        v = DEFAULT_FILL_METHODS.iter().map(|m| m.to_string()).collect();
    }
    let chosen = settings().fill_method;
    if !chosen.is_empty() && !v.contains(&chosen) {
        v.push(chosen);
    }
    v
}

/// Whether the service said it can improve prompts (at the last check).
static CAN_ENHANCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Remembers what the service offers (from [`photocraft_ml::ServerInfo`]).
fn remember(info: &photocraft_ml::ServerInfo) {
    if !info.fill_methods.is_empty() {
        *FILL_METHODS.write().unwrap_or_else(PoisonError::into_inner) = info.fill_methods.clone();
    }
    CAN_ENHANCE.store(info.prompt_enhance, std::sync::atomic::Ordering::Relaxed);
}

/// Hands the language model's API key to the engine (credential store, or the Preferences field).
/// `None` or blank removes it. Held in memory only.
pub fn set_enhancer_key(key: Option<&str>) {
    let key = key.map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    *ENHANCER_KEY.write().unwrap_or_else(PoisonError::into_inner) = key;
}

/// Is a key for the language model set?
pub fn has_enhancer_key() -> bool {
    ENHANCER_KEY.read().unwrap_or_else(PoisonError::into_inner).is_some()
}

/// The language model that improves prompts, if one is set up: the URL, model and key from
/// Preferences, else `PHOTOCRAFT_ENHANCER_URL` / `_KEY` / `_MODEL` (headless use, scripts).
pub fn enhancer_config() -> Option<photocraft_ml::EnhancerConfig> {
    #[cfg(test)]
    {
        // Unit tests never read the environment unless they say so.
        if std::env::var("PHOTOCRAFT_REMOTE_TEST").as_deref() != Ok("1") {
            return None;
        }
    }
    let st = settings();
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let stored = ENHANCER_KEY.read().unwrap_or_else(PoisonError::into_inner).clone();
    let key = pick_key(stored, env("PHOTOCRAFT_ENHANCER_KEY")).unwrap_or_default();
    let url = if st.enhancer_url.is_empty() { env("PHOTOCRAFT_ENHANCER_URL")? } else { st.enhancer_url };
    let model = if st.enhancer_model.is_empty() { env("PHOTOCRAFT_ENHANCER_MODEL").unwrap_or_else(|| "gpt-4.1-nano".into()) } else { st.enhancer_model };
    photocraft_ml::EnhancerConfig::new(&url, &key, &model)
}

/// Can a prompt be improved ("Enhance")? Through the language model if one is set up, else the
/// AI service when it said it can (at the last check).
pub fn can_enhance_prompts() -> bool {
    enhancer_config().is_some() || CAN_ENHANCE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Improves a short prompt: through the language model if one is set up, else the AI service.
/// Blocking: call it from a worker thread. Only the text is sent. The error is safe to show; it
/// never contains a key.
pub fn enhance_prompt(prompt: &str) -> std::result::Result<String, String> {
    if let Some(llm) = enhancer_config() {
        return llm.enhance(prompt).map_err(|e| e.to_string());
    }
    let cfg = config().ok_or_else(|| "no language model or AI service is set up for improving prompts (Edit › Preferences › AI Integrations)".to_string())?;
    photocraft_ml::RemoteClient::new(cfg).enhance_prompt(prompt).map_err(|e| e.to_string())
}

/// Asks the service what it offers and remembers the fill methods. Blocking: call it from a
/// worker thread. Does nothing without a usable service.
pub fn refresh_fill_methods() {
    let Some(cfg) = config() else { return };
    match photocraft_ml::RemoteClient::new(cfg).info() {
        Ok(info) => remember(&info),
        Err(e) => log::debug!("could not list the service's fill methods: {e}"),
    }
}

#[cfg(test)]
thread_local! {
    /// A configuration a test injects (thread-local, so parallel tests do not see each other's).
    static TEST_CONFIG: std::cell::RefCell<Option<RemoteConfig>> = const { std::cell::RefCell::new(None) };
}

/// Tests: use `config` on this thread (`None` clears it).
#[cfg(test)]
pub(crate) fn set_test_config(config: Option<RemoteConfig>) {
    TEST_CONFIG.with(|c| *c.borrow_mut() = config);
}

/// Hands the API key to the engine (from the credential store, or the Preferences field). `None`
/// or blank removes it. Held in memory only.
pub fn set_key(key: Option<&str>) {
    let key = key.map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    *KEY.write().unwrap_or_else(PoisonError::into_inner) = key;
}

/// Is an API key set (whatever it is)?
pub fn has_key() -> bool {
    KEY.read().unwrap_or_else(PoisonError::into_inner).is_some()
}

/// The current settings.
pub fn settings() -> Settings {
    SETTINGS.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Reads the preferences (called from [`Session::apply_prefs`], so a change in the dialog takes
/// effect at once).
pub(crate) fn sync_prefs(s: &Session) {
    let p = &s.prefs().integrations;
    let new = Settings {
        url: p.remote_url.trim().to_string(),
        use_for_selection: p.use_remote_for_selection,
        use_for_layers: p.use_remote_for_layers,
        use_for_fill: p.use_remote_for_fill,
        fill_method: p.fill_method.trim().to_string(),
        fill_improve_detail: p.fill_improve_detail,
        fill_context: p.fill_context.min(100),
        enhancer_url: p.enhancer_url.trim().to_string(),
        enhancer_model: p.enhancer_model.trim().to_string(),
    };
    *SETTINGS.write().unwrap_or_else(PoisonError::into_inner) = new;
}

/// The service to talk to, if one is set up and usable (URL allowed, key present).
pub fn config() -> Option<RemoteConfig> {
    #[cfg(test)]
    {
        // The unit tests never reach a network or read the environment unless they say so.
        if let Some(c) = TEST_CONFIG.with(|c| c.borrow().clone()) {
            return Some(c);
        }
        if std::env::var("PHOTOCRAFT_REMOTE_TEST").as_deref() != Ok("1") {
            return None;
        }
    }
    let st = settings();
    if !st.url.is_empty() {
        let stored = KEY.read().unwrap_or_else(PoisonError::into_inner).clone();
        let key = pick_key(stored, std::env::var("PHOTOCRAFT_REMOTE_KEY").ok())?;
        return RemoteConfig::new(&st.url, &key);
    }
    RemoteConfig::from_env()
}

/// The key for a URL set in Preferences: the one saved in the credential store, else the
/// launcher's `PHOTOCRAFT_REMOTE_KEY` (so a URL typed in Preferences does not hide a key that
/// the environment already provides).
fn pick_key(stored: Option<String>, env: Option<String>) -> Option<String> {
    stored.filter(|k| !k.trim().is_empty()).or_else(|| env.filter(|k| !k.trim().is_empty()))
}

/// The service for Object Selection, if configured and switched on.
pub fn selection_config() -> Option<RemoteConfig> {
    settings().use_for_selection.then(config).flatten()
}

/// The service for Decompose into Layers, if configured and switched on.
pub fn layers_config() -> Option<RemoteConfig> {
    settings().use_for_layers.then(config).flatten()
}

/// The service for filling the hole Object to Layer leaves, if configured and switched on.
pub fn fill_config() -> Option<RemoteConfig> {
    settings().use_for_fill.then(config).flatten()
}

/// Why no service is usable, in words for the user; `None` when one is.
pub fn unavailable_reason() -> Option<String> {
    if config().is_some() {
        return None;
    }
    let st = settings();
    Some(if st.url.is_empty() {
        "no AI service is set up (Edit › Preferences › AI Integrations)".to_string()
    } else if !RemoteConfig::url_is_allowed(&st.url) {
        "the AI service URL must start with https:// (Edit › Preferences › AI Integrations)".to_string()
    } else {
        "the AI service needs an API key (Edit › Preferences › AI Integrations)".to_string()
    })
}

/// Preferences › "Test connection": asks the service what it offers, with `url` and `key` as typed
/// (before they are saved). Blocking: call it from a worker thread. The text is safe to show; it
/// never contains the key.
pub fn test_connection(url: &str, key: &str) -> std::result::Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Enter the service URL first".into());
    }
    if !RemoteConfig::url_is_allowed(url) {
        return Err("The URL must start with https://".into());
    }
    let cfg = RemoteConfig::new(url, key).ok_or("Enter the API key first")?;
    let info = photocraft_ml::RemoteClient::new(cfg).info().map_err(|e| e.to_string())?;
    let models = if info.models.is_empty() { "no models listed".to_string() } else { info.models.join(", ") };
    remember(&info);
    Ok(format!("Connected (API v{}). Models: {models}. Jobs waiting: {}.", if info.api_version.is_empty() { "?" } else { &info.api_version }, info.queue_length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_in_preferences_still_finds_the_launchers_key() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(pick_key(some("saved"), some("env")), some("saved"));
        assert_eq!(pick_key(None, some("env")), some("env"));
        assert_eq!(pick_key(some("  "), some("env")), some("env"));
        assert_eq!(pick_key(None, some(" ")), None);
        assert_eq!(pick_key(None, None), None);
    }

    #[test]
    fn the_key_is_held_trimmed_and_can_be_removed() {
        // Global state: this test is the only one that touches the key.
        set_key(Some("  pc_abc  "));
        assert!(has_key());
        set_key(Some("   "));
        assert!(!has_key());
        set_key(Some("pc_abc"));
        set_key(None);
        assert!(!has_key());
    }

    #[test]
    fn unavailable_reasons_name_the_missing_piece() {
        // Nothing is configured in the test build, so a reason is always given and never the key.
        let r = unavailable_reason().unwrap();
        assert!(r.contains("AI Integrations") && !r.contains("pc_"));
    }

    #[test]
    fn the_connection_test_explains_what_is_missing() {
        assert!(test_connection("", "k").unwrap_err().contains("URL"));
        assert!(test_connection("http://example.com", "k").unwrap_err().contains("https"));
        assert!(test_connection("https://example.com", " ").unwrap_err().contains("API key"));
    }

    #[test]
    fn a_test_config_overrides_everything_on_its_thread() {
        let c = RemoteConfig::new("http://127.0.0.1:1/v1", "k").unwrap();
        set_test_config(Some(c));
        assert!(config().is_some() && selection_config().is_some() && unavailable_reason().is_none());
        set_test_config(None);
        assert!(config().is_none());
    }
}
