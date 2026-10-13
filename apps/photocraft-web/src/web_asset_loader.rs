//! Bounded external asset loading before the web application is constructed.
//! The 32 MiB limit counts known Rust byte ownership only; it is NOT an RSS proof.
use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use photocraft_text::{FontBootstrapError, install_web_font_provider};
use photocraft_ui_egui::web_ui_assets::descriptors::{PACK_BYTES, PACK_FILE, PACK_SHA256, SCHEMA_VERSION};
use photocraft_ui_egui::web_ui_assets::{ReadyUiAssets, UiAssetError, WebUiAssets, text_provider};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, AbortSignal, Cache, ReadableStreamDefaultReader, Request, RequestInit, Response, Window};

const DECODED_CAP: u64 = 8 * 1024 * 1024;
const KNOWN_RUST_CAP: u64 = 32 * 1024 * 1024;
const BOOT_TIMEOUT_MS: i32 = 30_000;
const CACHE_NAMESPACE: &str = "photocraft-web-assets-schema-1";

#[derive(Debug)]
pub enum BootError {
    Environment,
    Schema,
    Url,
    Network,
    Response,
    Stream,
    Length,
    Hash,
    Allocation,
    Timeout,
    Cancelled,
    Text(FontBootstrapError),
    Ui(UiAssetError),
    #[cfg(feature = "web_boot_contract_tests")]
    Contract(&'static str),
}
impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment => f.write_str("web-assets.environment"),
            Self::Schema => f.write_str("web-assets.schema"),
            Self::Url => f.write_str("web-assets.url"),
            Self::Network => f.write_str("web-assets.network"),
            Self::Response => f.write_str("web-assets.response"),
            Self::Stream => f.write_str("web-assets.stream"),
            Self::Length => f.write_str("web-assets.length"),
            Self::Hash => f.write_str("web-assets.pack-hash"),
            Self::Allocation => f.write_str("web-assets.allocation"),
            Self::Timeout => f.write_str("web-assets.timeout"),
            Self::Cancelled => f.write_str("web-assets.cancelled"),
            Self::Text(error) => write!(f, "web-assets.text: {error}"),
            Self::Ui(error) => write!(f, "{error}"),
            #[cfg(feature = "web_boot_contract_tests")]
            Self::Contract(phase) => write!(f, "web-assets.contract-{phase}"),
        }
    }
}
impl std::error::Error for BootError {}

/// Own both callbacks. Drop unregisters cancellation and clears the timer;
/// neither Closure::forget nor leaked Rust storage is used by this loader.
struct Deadline {
    window: Window,
    performance: web_sys::Performance,
    end_ms: f64,
    controller: AbortController,
    stopped: Rc<Cell<u8>>, // 0 running, 1 timeout, 2 cancellation
    stop: Promise,
    timer: i32,
    timer_callback: Closure<dyn FnMut()>,
    cancel: Option<(AbortSignal, Closure<dyn FnMut()>)>,
}
impl Deadline {
    fn new(window: Window, external: Option<AbortSignal>) -> Result<Self, BootError> {
        let performance = window.performance().ok_or(BootError::Environment)?;
        let start = performance.now();
        if !start.is_finite() {
            return Err(BootError::Environment);
        }
        let controller = AbortController::new().map_err(|_| BootError::Environment)?;
        let stopped = Rc::new(Cell::new(0));
        let mut reject_slot: Option<Function> = None;
        let stop = Promise::new(&mut |_, reject| {
            reject_slot = Some(reject);
        });
        let reject = reject_slot.ok_or(BootError::Environment)?;
        let state = stopped.clone();
        let abort = controller.clone();
        let timeout_reject = reject.clone();
        let timer_callback = Closure::<dyn FnMut()>::new(move || {
            if state.get() == 0 {
                state.set(1);
                abort.abort();
                let _ = timeout_reject.call1(&JsValue::UNDEFINED, &JsValue::from_str("web-assets.timeout"));
            }
        });
        let timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(timer_callback.as_ref().unchecked_ref(), BOOT_TIMEOUT_MS)
            .map_err(|_| BootError::Environment)?;
        let mut result =
            Self { window, performance, end_ms: start + f64::from(BOOT_TIMEOUT_MS), controller, stopped, stop, timer, timer_callback, cancel: None };
        if let Some(signal) = external {
            if signal.aborted() {
                return Err(BootError::Cancelled);
            }
            let state = result.stopped.clone();
            let abort = result.controller.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                if state.get() == 0 {
                    state.set(2);
                    abort.abort();
                    let _ = reject.call1(&JsValue::UNDEFINED, &JsValue::from_str("web-assets.cancelled"));
                }
            });
            signal.add_event_listener_with_callback("abort", callback.as_ref().unchecked_ref()).map_err(|_| BootError::Environment)?;
            result.cancel = Some((signal, callback));
        }
        Ok(result)
    }
    fn check(&self) -> Result<(), BootError> {
        match self.stopped.get() {
            1 => return Err(BootError::Timeout),
            2 => return Err(BootError::Cancelled),
            _ => {}
        }
        if self.cancel.as_ref().is_some_and(|(signal, _)| signal.aborted()) {
            return Err(BootError::Cancelled);
        }
        let now = self.performance.now();
        if !now.is_finite() {
            return Err(BootError::Environment);
        }
        if now >= self.end_ms {
            return Err(BootError::Timeout);
        }
        Ok(())
    }
    async fn wait(&self, operation: Promise) -> Result<JsValue, BootError> {
        // The caller has already created operation. Observe both contenders
        // before an early cancellation/deadline return can abandon its rejection.
        let contenders = Array::new();
        contenders.push(&operation);
        contenders.push(&self.stop);
        let observed = JsFuture::from(Promise::race(&contenders));
        self.check()?;
        let result = observed.await;
        self.check()?;
        result.map_err(|_| BootError::Network)
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        self.controller.abort();
        self.window.clear_timeout_with_handle(self.timer);
        if let Some((signal, callback)) = &self.cancel {
            let _ = signal.remove_event_listener_with_callback("abort", callback.as_ref().unchecked_ref());
        }
        // Keep the timer Closure field alive through clear_timeout.
        let _ = &self.timer_callback;
    }
}

struct Reader(ReadableStreamDefaultReader);
thread_local! {
    // One callback per WASM instance, retained while any cancellation promise
    // may settle. No per-read callback is leaked or waited on during teardown.
    static CANCEL_REJECTION_SINK: Closure<dyn FnMut(JsValue)> = Closure::new(|_: JsValue| {});
    #[cfg(feature = "web_boot_contract_tests")]
    static CONTRACT_READ_PENDING: Cell<bool> = const { Cell::new(false) };
}
impl Drop for Reader {
    fn drop(&mut self) {
        // Initiate bounded teardown without awaiting an untrusted cancel hook.
        // Releasing the lock rejects any pending read; the race already owns
        // its rejection handler. Cancellation itself can reject an errored body.
        CANCEL_REJECTION_SINK.with(|sink| {
            let _ = self.0.cancel().catch(sink);
        });
        self.0.release_lock();
    }
}

/// Exactly one Rust pack allocation, no array_buffer()/to_vec() body copy.
/// Content-Length is deliberately not authoritative for decoded gzip/Brotli.
async fn read_pack(response: Response, deadline: &Deadline) -> Result<Vec<u8>, BootError> {
    if !response.ok() {
        return Err(BootError::Response);
    }
    if PACK_BYTES == 0 || PACK_BYTES > DECODED_CAP {
        return Err(BootError::Schema);
    }
    let length = usize::try_from(PACK_BYTES).map_err(|_| BootError::Length)?;
    let stream = response.body().ok_or(BootError::Stream)?;
    let reader = Reader(ReadableStreamDefaultReader::new(&stream).map_err(|_| BootError::Stream)?);
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| BootError::Allocation)?;
    bytes.resize(length, 0);
    let mut used = 0_usize;
    let mut hash = Sha256::new();
    loop {
        #[cfg(feature = "web_boot_contract_tests")]
        CONTRACT_READ_PENDING.with(|pending| pending.set(true));
        let record = deadline.wait(reader.0.read()).await;
        #[cfg(feature = "web_boot_contract_tests")]
        CONTRACT_READ_PENDING.with(|pending| pending.set(false));
        let record = record?;
        let done = Reflect::get(&record, &JsValue::from_str("done")).map_err(|_| BootError::Stream)?.as_bool().ok_or(BootError::Stream)?;
        if done {
            break;
        }
        let chunk =
            Reflect::get(&record, &JsValue::from_str("value")).map_err(|_| BootError::Stream)?.dyn_into::<Uint8Array>().map_err(|_| BootError::Stream)?;
        let chunk_len = usize::try_from(chunk.length()).map_err(|_| BootError::Length)?;
        let end = used.checked_add(chunk_len).ok_or(BootError::Length)?;
        let destination = bytes.get_mut(used..end).ok_or(BootError::Length)?;
        chunk.copy_to(destination);
        for block in destination.chunks(4096) {
            deadline.check()?;
            hash.update(block);
        }
        used = end;
    }
    deadline.check()?;
    if used != length {
        return Err(BootError::Length);
    }
    let actual: [u8; 32] = hash.finalize().into();
    if actual != PACK_SHA256 {
        return Err(BootError::Hash);
    }
    Ok(bytes)
}

async fn open_cache(deadline: &Deadline) -> Result<Option<Cache>, BootError> {
    let Ok(storage) = deadline.window.caches() else {
        return Ok(None);
    };
    match deadline.wait(storage.open(CACHE_NAMESPACE)).await {
        Ok(value) => Ok(value.dyn_into::<Cache>().ok()),
        Err(BootError::Network) => Ok(None), // denied cache is optional persistence
        Err(error) => Err(error),
    }
}

async fn fetch_pack(deadline: &Deadline, url: &str, cache_key: &str, cache: Option<&Cache>) -> Result<Vec<u8>, BootError> {
    if let Some(cache) = cache {
        match deadline.wait(cache.match_with_str(cache_key)).await {
            Ok(hit) if !hit.is_undefined() => {
                if let Ok(response) = hit.dyn_into::<Response>() {
                    match read_pack(response, deadline).await {
                        Ok(bytes) => return Ok(bytes),
                        Err(BootError::Timeout) => return Err(BootError::Timeout),
                        Err(BootError::Cancelled) => return Err(BootError::Cancelled),
                        Err(BootError::Allocation) => return Err(BootError::Allocation),
                        Err(_) => {} // discarded corrupt/stale body cannot become ready
                    }
                }
                match deadline.wait(cache.delete_with_str(cache_key)).await {
                    Ok(_) | Err(BootError::Network) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(_) | Err(BootError::Network) => {}
            Err(error) => return Err(error),
        }
    }
    let init = RequestInit::new();
    init.set_mode(web_sys::RequestMode::SameOrigin);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    init.set_redirect(web_sys::RequestRedirect::Error);
    init.set_cache(web_sys::RequestCache::NoStore);
    // Rust checks decoded length and whole/range SHA before publication. Fetch
    // SRI can buffer a complete body before these bounded reads; the compiled
    // SRI remains in packaging/audit descriptors, not this network acceptance.
    init.set_signal(Some(&deadline.controller.signal()));
    let request = Request::new_with_str_and_init(url, &init).map_err(|_| BootError::Url)?;
    let response = deadline.wait(deadline.window.fetch_with_request(&request)).await?.dyn_into::<Response>().map_err(|_| BootError::Response)?;
    if response.url() != url {
        return Err(BootError::Response);
    }
    let bytes = read_pack(response, deadline).await?;
    if let Some(cache) = cache {
        // One additional JS body copy, bounded PACK_BYTES. Browser Cache internals
        // remain unqualified overhead; no Cache.keys()/hash-key accumulation.
        let body = Uint8Array::from(bytes.as_slice());
        if let Ok(response) = Response::new_with_opt_js_u8_array(Some(&body)) {
            match deadline.wait(cache.put_with_str(cache_key, &response)).await {
                Ok(_) | Err(BootError::Network) => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(bytes)
}

/// Caller must pass the returned opaque ready capability into every app startup
/// constructor. Before this returns there is no WebRunner, egui Context, Session,
/// PhotocraftApp, services, language selection, or font-family cache access.
pub async fn prepare_boot(external_cancel: Option<AbortSignal>) -> Result<ReadyUiAssets, BootError> {
    install_web_font_provider(text_provider).map_err(BootError::Text)?;
    #[cfg(feature = "web_boot_contract_tests")]
    {
        let url_proof = verify_packaged_url_contract()?;
        log::info!("PhotoCraft boot contract urls: {url_proof}");
        let proof = crate::boot_contract_probe::before_publication().map_err(|_| BootError::Contract("before"))?;
        log::info!("PhotoCraft boot contract before: {proof}");
    }
    let window = web_sys::window().ok_or(BootError::Environment)?;
    let deadline = Deadline::new(window, external_cancel)?;
    if SCHEMA_VERSION != 1 {
        return Err(BootError::Schema);
    }
    let document = deadline.window.document().ok_or(BootError::Environment)?;
    let base = document.base_uri().map_err(|_| BootError::Url)?.ok_or(BootError::Url)?;
    let origin = deadline.window.location().origin().map_err(|_| BootError::Url)?;
    let (url, key) = resolve_asset_urls(&base, &origin)?;
    #[cfg(feature = "web_boot_contract_tests")]
    {
        let proof = verify_loader_runtime_contract(&base, &url).await?;
        log::info!("PhotoCraft boot contract loader: {proof}");
    }
    let cache = open_cache(&deadline).await?;
    let pack = fetch_pack(&deadline, &url, &key, cache.as_ref()).await?;
    let mut owned = PACK_BYTES;
    let mut charge = |bytes: u64| -> Result<(), FontBootstrapError> {
        deadline.check().map_err(|error| match error {
            BootError::Timeout => FontBootstrapError::Timeout,
            BootError::Cancelled => FontBootstrapError::Cancelled,
            _ => FontBootstrapError::Budget,
        })?;
        owned = owned.checked_add(bytes).filter(|total| *total <= KNOWN_RUST_CAP).ok_or(FontBootstrapError::Budget)?;
        Ok(())
    };
    let bank = WebUiAssets::prepare(pack, &mut charge).map_err(BootError::Ui)?;
    deadline.check()?;
    let ready = bank.publish().map_err(BootError::Ui)?;
    #[cfg(feature = "web_boot_contract_tests")]
    {
        let proof = crate::boot_contract_probe::after_publication().map_err(|_| BootError::Contract("after"))?;
        log::info!("PhotoCraft boot contract after: {proof}");
    }
    Ok(ready)
}

#[cfg(feature = "web_boot_contract_tests")]
struct ContractAbort {
    window: Window,
    timer: i32,
    callback: Closure<dyn FnMut()>,
}
#[cfg(feature = "web_boot_contract_tests")]
impl Drop for ContractAbort {
    fn drop(&mut self) {
        self.window.clear_timeout_with_handle(self.timer);
        let _ = &self.callback;
    }
}

#[cfg(feature = "web_boot_contract_tests")]
struct ContractRejections {
    window: Window,
    callback: Closure<dyn FnMut(web_sys::Event)>,
}
#[cfg(feature = "web_boot_contract_tests")]
impl Drop for ContractRejections {
    fn drop(&mut self) {
        let _ = self.window.remove_event_listener_with_callback("unhandledrejection", self.callback.as_ref().unchecked_ref());
    }
}

/// Actual CacheStorage, fetch and pending body-read paths, isolated from the
/// ordinary cache. The fixed delayed route is served only by diagnostic tooling.
#[cfg(feature = "web_boot_contract_tests")]
async fn contract_old_wait(deadline: &Deadline, operation: Promise) -> Result<JsValue, BootError> {
    // Faithful former implementation, diagnostic control only. Its expected
    // rejected read is observed by the scoped browser event listener below.
    deadline.check()?;
    let contenders = Array::new();
    contenders.push(&operation);
    contenders.push(&deadline.stop);
    let result = JsFuture::from(Promise::race(&contenders)).await;
    deadline.check()?;
    result.map_err(|_| BootError::Network)
}

#[cfg(feature = "web_boot_contract_tests")]
async fn contract_prewait(window: &Window, delayed: &str, old_control: bool) -> Result<serde_json::Value, BootError> {
    use std::cell::RefCell;
    let external = AbortController::new().map_err(|_| BootError::Environment)?;
    let deadline = Deadline::new(window.clone(), Some(external.signal()))?;
    let count = Rc::new(Cell::new(0_u32));
    let read_count = Rc::new(Cell::new(0_u32));
    let expected_read = Rc::new(RefCell::new(None::<Promise>));
    let observed = count.clone();
    let observed_read = read_count.clone();
    let expected = expected_read.clone();
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        observed.set(observed.get().saturating_add(1));
        if let (Some(read), Ok(promise)) = (expected.borrow().as_ref(), Reflect::get(event.as_ref(), &JsValue::from_str("promise")))
            && js_sys::Object::is(read.as_ref(), &promise)
        {
            observed_read.set(observed_read.get().saturating_add(1));
            if old_control {
                event.prevent_default();
            } // exact expected RED promise only
        }
    });
    window.add_event_listener_with_callback("unhandledrejection", callback.as_ref().unchecked_ref()).map_err(|_| BootError::Environment)?;
    let listener = ContractRejections { window: window.clone(), callback };
    let init = RequestInit::new();
    init.set_signal(Some(&deadline.controller.signal()));
    let request = Request::new_with_str_and_init(delayed, &init).map_err(|_| BootError::Url)?;
    let start = deadline.performance.now();
    let response = deadline.wait(window.fetch_with_request(&request)).await?.dyn_into::<Response>().map_err(|_| BootError::Response)?;
    if !response.ok() || response.url() != delayed {
        return Err(BootError::Contract("loader-prewait-response"));
    }
    let stream = response.body().ok_or(BootError::Stream)?;
    let reader = Reader(ReadableStreamDefaultReader::new(&stream).map_err(|_| BootError::Stream)?);
    // Fetch may split a server write into arbitrary stream chunks. Consume the
    // complete flushed block before the next read can genuinely be pending.
    let mut first_bytes = 0_u32;
    let mut first_reads = 0_u32;
    while first_bytes < 4096 {
        first_reads = first_reads.checked_add(1).filter(|count| *count <= 4096).ok_or(BootError::Contract("loader-prewait-first-read"))?;
        let first = deadline.wait(reader.0.read()).await?;
        if Reflect::get(&first, &JsValue::from_str("done")).map_err(|_| BootError::Stream)?.as_bool() != Some(false) {
            return Err(BootError::Contract("loader-prewait-first-read"));
        }
        let chunk =
            Reflect::get(&first, &JsValue::from_str("value")).map_err(|_| BootError::Stream)?.dyn_into::<Uint8Array>().map_err(|_| BootError::Stream)?;
        first_bytes = first_bytes.checked_add(chunk.length()).filter(|count| *count <= 4096).ok_or(BootError::Contract("loader-prewait-first-read"))?;
        let elapsed = deadline.performance.now() - start;
        if !elapsed.is_finite() || !(0.0..1000.0).contains(&elapsed) {
            return Err(BootError::Contract("loader-prewait-first-read"));
        }
    }
    let first_ms = deadline.performance.now() - start;
    if !first_ms.is_finite() || !(0.0..1000.0).contains(&first_ms) {
        return Err(BootError::Contract("loader-prewait-first-read"));
    }
    let pending = reader.0.read();
    *expected_read.borrow_mut() = Some(pending.clone());
    external.abort();
    let stopped = if old_control { contract_old_wait(&deadline, pending).await } else { deadline.wait(pending).await };
    drop(reader);
    if !matches!(stopped, Err(BootError::Cancelled)) || photocraft_ui_egui::web_ui_assets::ready().is_ok() {
        return Err(BootError::Contract("loader-prewait-cancel"));
    }
    let mut resolve_slot = None;
    let settle = Promise::new(&mut |resolve, _| resolve_slot = Some(resolve));
    let resolve = resolve_slot.ok_or(BootError::Environment)?;
    let callback = Closure::<dyn FnMut()>::new(move || {
        let _ = resolve.call0(&JsValue::UNDEFINED);
    });
    let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.as_ref().unchecked_ref(), 50).map_err(|_| BootError::Environment)?;
    let scheduled = ContractAbort { window: window.clone(), timer, callback };
    // A fresh uncancelled deadline observes the settle promise, not the stopped one.
    let settling = Deadline::new(window.clone(), None)?;
    settling.wait(settle).await?;
    drop(scheduled);
    drop(listener);
    if (old_control && (read_count.get() == 0 || count.get() != read_count.get())) || (!old_control && count.get() != 0) {
        return Err(BootError::Contract("loader-prewait-rejection"));
    }
    Ok(serde_json::json!({"case":if old_control {"old-wait-expected-red"} else {"already-cancelled-before-wait"},
        "passed":true,"expectedControlRed":old_control,"error":"web-assets.cancelled","firstServerBlockBytes":first_bytes,
        "firstBlockReadCount":first_reads,"firstBlockElapsedMs":first_ms,"pendingReadCreatedBeforeSignal":true,"readerReleased":true,"noReadyPublication":true,
        "unhandledRejections":count.get(),"exactReadPromiseUnhandledRejections":read_count.get()}))
}

#[cfg(feature = "web_boot_contract_tests")]
async fn verify_loader_runtime_contract(base: &str, pack_url: &str) -> Result<String, BootError> {
    use serde_json::json;
    const NAMESPACE: &str = "photocraft-assets-contract-20261012-v1";
    let window = web_sys::window().ok_or(BootError::Environment)?;
    let deadline = Deadline::new(window.clone(), None)?;
    let storage = window.caches().map_err(|_| BootError::Contract("loader-cache"))?;
    let cache = deadline.wait(storage.open(NAMESPACE)).await?.dyn_into::<Cache>().map_err(|_| BootError::Contract("loader-cache"))?;
    let key = web_sys::Url::new_with_base("contract-assets/corrupt-pack", base).map_err(|_| BootError::Url)?.href();
    let result = async {
        let length = u32::try_from(PACK_BYTES).map_err(|_| BootError::Length)?;
        let corrupt = Uint8Array::new_with_length(length);
        let response = Response::new_with_opt_js_u8_array(Some(&corrupt)).map_err(|_| BootError::Response)?;
        deadline.wait(cache.put_with_str(&key, &response)).await?;
        // The planted body has the exact length but cannot have the compiled SHA.
        let planted = deadline.wait(cache.match_with_str(&key)).await?.dyn_into::<Response>().map_err(|_| BootError::Response)?;
        if !matches!(read_pack(planted, &deadline).await, Err(BootError::Hash)) {
            return Err(BootError::Contract("loader-corrupt-fixture"));
        }
        let repaired = fetch_pack(&deadline, pack_url, &key, Some(&cache)).await?;
        let warm = fetch_pack(&deadline, pack_url, &key, Some(&cache)).await?;
        if repaired != warm || u64::try_from(warm.len()).map_err(|_| BootError::Length)? != PACK_BYTES {
            return Err(BootError::Contract("loader-cache-repair"));
        }
        // No banking/publication occurs in fetch_pack; neither successful cache
        // reads nor the cancelled request may grant the ready capability.
        if photocraft_ui_egui::web_ui_assets::ready().is_ok() {
            return Err(BootError::Contract("loader-publication"));
        }
        let delayed = web_sys::Url::new_with_base("contract-assets/delayed-body.bin", base).map_err(|_| BootError::Url)?.href();
        let old_wait_control = contract_prewait(&window, &delayed, true).await?;
        let fixed_prewait = contract_prewait(&window, &delayed, false).await?;
        let rejection_count = Rc::new(Cell::new(0_u32));
        let observed_rejections = rejection_count.clone();
        let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            observed_rejections.set(observed_rejections.get().saturating_add(1));
        });
        window.add_event_listener_with_callback("unhandledrejection", callback.as_ref().unchecked_ref()).map_err(|_| BootError::Environment)?;
        let rejections = ContractRejections { window: window.clone(), callback };
        let external = AbortController::new().map_err(|_| BootError::Environment)?;
        let cancelled = Deadline::new(window.clone(), Some(external.signal()))?;
        let signal_at = Rc::new(Cell::new(None::<f64>));
        let pending_at_signal = Rc::new(Cell::new(false));
        let observed_signal = signal_at.clone();
        let observed_pending = pending_at_signal.clone();
        let performance = cancelled.performance.clone();
        let callback = Closure::<dyn FnMut()>::new(move || {
            observed_signal.set(Some(performance.now()));
            observed_pending.set(CONTRACT_READ_PENDING.with(Cell::get));
            external.abort();
        });
        let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.as_ref().unchecked_ref(), 300).map_err(|_| BootError::Environment)?;
        let scheduled = ContractAbort { window: window.clone(), timer, callback };
        let stopped = fetch_pack(&cancelled, &delayed, &key, None).await;
        let elapsed = cancelled.performance.now() - signal_at.get().ok_or(BootError::Contract("loader-cancel-signal"))?;
        drop(scheduled);
        if !matches!(stopped, Err(BootError::Cancelled)) || !pending_at_signal.get() || !elapsed.is_finite() || !(0.0..=2000.0).contains(&elapsed) {
            return Err(BootError::Contract("loader-cancel"));
        }
        if photocraft_ui_egui::web_ui_assets::ready().is_ok() {
            return Err(BootError::Contract("loader-publication"));
        }
        // Allow rejection dispatch a fresh event-loop turn; the cancellation
        // promise itself is not awaited and may remain browser-owned afterward.
        let mut resolve_slot = None;
        let settle = Promise::new(&mut |resolve, _| resolve_slot = Some(resolve));
        let resolve = resolve_slot.ok_or(BootError::Environment)?;
        let callback = Closure::<dyn FnMut()>::new(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.as_ref().unchecked_ref(), 50).map_err(|_| BootError::Environment)?;
        let scheduled = ContractAbort { window: window.clone(), timer, callback };
        deadline.wait(settle).await?;
        drop(scheduled);
        drop(rejections);
        if rejection_count.get() != 0 {
            return Err(BootError::Contract("loader-cancel-rejection"));
        }
        Ok(json!({
            "contract":"web-loader-runtime-v1", "cacheNamespace":NAMESPACE,
            "packBytes":PACK_BYTES, "packSha256":PACK_SHA256.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "checks":[
                {"case":"corrupt-cache-repair", "passed":true, "fixtureError":"web-assets.pack-hash"},
                {"case":"warm-cache-reuse", "passed":true},
                {"case":"external-cancel-pending-read", "passed":true, "error":"web-assets.cancelled", "pendingReadAtSignal":true,
                 "signalToReturnMs":elapsed, "noReadyPublication":true, "cancelTeardownUnhandledRejections":rejection_count.get()},
                old_wait_control,
                fixed_prewait
            ],
            "limitations":["server request receipt proves repair/warm request counts", "no physical browser memory or cancellation-promise settlement claim"]
        })
        .to_string())
    }
    .await;
    // Remove only the namespace owned by this diagnostic. Never enumerate or
    // delete the ordinary application's cache, even when a check fails.
    let cleanup = deadline.wait(storage.delete(NAMESPACE)).await;
    match (result, cleanup) {
        (Ok(report), Ok(_)) if report.len() <= 16 * 1024 => Ok(report),
        (Ok(_), Ok(_)) => Err(BootError::Contract("loader-report")),
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
}

fn resolve_asset_urls(base: &str, origin: &str) -> Result<(String, String), BootError> {
    if !crate::asset_paths::valid_pack_basename(PACK_FILE) {
        return Err(BootError::Schema);
    }
    let assets_base = web_sys::Url::new_with_base(crate::asset_paths::ASSET_DIRECTORY, base).map_err(|_| BootError::Url)?;
    let pack = web_sys::Url::new_with_base(PACK_FILE, &assets_base.href()).map_err(|_| BootError::Url)?;
    if pack.origin() != origin || !matches!(pack.protocol().as_str(), "https:" | "http:") {
        return Err(BootError::Url);
    }
    // Logical key follows app deployment base/schema, never the hashed pack path.
    let key = web_sys::Url::new_with_base(crate::asset_paths::CACHE_KEY, base).map_err(|_| BootError::Url)?;
    if key.origin() != origin {
        return Err(BootError::Url);
    }
    Ok((pack.href(), key.href()))
}

#[cfg(feature = "web_boot_contract_tests")]
fn verify_packaged_url_contract() -> Result<String, BootError> {
    use std::fmt::Write;
    let origin = "https://example.invalid";
    let mut proof = String::from("{\"cases\":[");
    for (index, (case, base, prefix)) in [
        ("root", "https://example.invalid/", "https://example.invalid/"),
        ("subpath", "https://example.invalid/apps/photocraft/", "https://example.invalid/apps/photocraft/"),
        ("index", "https://example.invalid/apps/photocraft/index.html", "https://example.invalid/apps/photocraft/"),
        ("query", "https://example.invalid/apps/photocraft/index.html?cpu&webgl", "https://example.invalid/apps/photocraft/"),
    ]
    .into_iter()
    .enumerate()
    {
        let (pack, key) = resolve_asset_urls(base, origin)?;
        if pack != format!("{prefix}ui-assets/{PACK_FILE}") || key != format!("{prefix}{}", crate::asset_paths::CACHE_KEY) {
            return Err(BootError::Contract("urls"));
        }
        if index != 0 {
            proof.push(',');
        }
        write!(proof, "{{\"case\":\"{case}\",\"pack\":\"{pack}\",\"cache\":\"{key}\"}}").map_err(|_| BootError::Contract("urls"))?;
    }
    if !matches!(resolve_asset_urls("https://other.invalid/app/", origin), Err(BootError::Url)) {
        return Err(BootError::Contract("cross-origin"));
    }
    proof.push_str("],\"crossOriginRejected\":true}");
    if proof.len() > 4096 {
        return Err(BootError::Contract("urls"));
    }
    Ok(proof)
}

/// Trusted named errors are text, never innerHTML. Reload is a plain anchor:
/// no retained callback, listener leak, new app, or implicit English catalog.
pub fn show_boot_error(document: &web_sys::Document, error: &BootError) {
    if let Some(element) = document.get_element_by_id("photocraft_loading") {
        element.set_text_content(Some(&format!("PhotoCraft could not load its assets: {error}. Reload to retry.")));
        if let Ok(link) = document.create_element("a") {
            link.set_text_content(Some("Reload"));
            if let Ok(href) = document.location().and_then(|location| location.href().ok()).ok_or(())
                && link.set_attribute("href", &href).is_ok()
            {
                let _ = element.append_child(&link);
            }
        }
    }
}
