//! Explicit owned-XCF selected bytes; no path or filesystem capabilities.
//! `show_picker` runs synchronously. A queued frame may lack user activation and fails visibly.
use std::cell::Cell;
use std::rc::{Rc, Weak};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Event, FileReader, HtmlInputElement};

pub(crate) const MAX_BYTES: usize = 16 << 20;
use photocraft_raster::backing::quota::QuotaLease;
use photocraft_ui_egui::owned_xcf_bytes::{Capture, CaptureBuilder, Request};
type Handler = Closure<dyn FnMut(Event)>;
type Callback = Box<dyn FnOnce(Outcome)>;
pub(crate) type Outcome = Result<Option<Capture>, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    UnsupportedPicker,
    PickerStart,
    InvalidSelection,
    SizeInvalid,
    TooLarge,
    Allocation,
    ReadStart,
    ReadFailed,
    Aborted,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsupportedPicker => "xcf.resource.picker-unavailable",
            Self::PickerStart => "xcf.resource.picker-start-failed",
            Self::InvalidSelection => "xcf.resource.invalid-selection",
            Self::SizeInvalid => "xcf.resource.invalid-size",
            Self::TooLarge => "xcf.resource.too-large",
            Self::Allocation => "xcf.resource.allocation",
            Self::ReadStart => "xcf.resource.read-start-failed",
            Self::ReadFailed => "xcf.resource.read-failed",
            Self::Aborted => "xcf.resource.read-aborted",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Default)]
struct Resources {
    input: Option<HtmlInputElement>,
    reader: Option<FileReader>,
    info: Option<CaptureBuilder>,
    change: Option<Handler>,
    cancel: Option<Handler>,
    load: Option<Handler>,
    error: Option<Handler>,
    abort: Option<Handler>,
}

struct State {
    terminal: Cell<bool>,
    reading: Cell<bool>,
    resources: Cell<Resources>,
    callback: Cell<Option<Callback>>,
    request: Cell<Option<Request>>,
}

/// Retain until completion; dropping an unfinished handle reports Aborted once.
pub(crate) struct PickerHandle {
    state: Rc<State>,
}

impl PickerHandle {
    pub(crate) fn cancel(&self) {
        schedule(&self.state, Err(Error::Aborted.to_string()));
    }
}

impl Drop for PickerHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Setup/read errors and user cancellation are delivered through the same typed callback.
/// No await, queued task or click fallback precedes the caught show_picker call.
pub(crate) fn pick_resource(request: Request, then: impl FnOnce(Outcome) + 'static) -> PickerHandle {
    let state = Rc::new(State {
        terminal: Cell::new(false),
        reading: Cell::new(false),
        resources: Cell::new(Resources::default()),
        callback: Cell::new(Some(Box::new(then))),
        request: Cell::new(Some(request)),
    });
    if let Err(error) = setup(&state) {
        schedule(&state, Err(error.to_string()));
    }
    PickerHandle { state }
}

fn setup(state: &Rc<State>) -> Result<(), Error> {
    let document = web_sys::window().and_then(|window| window.document()).ok_or(Error::UnsupportedPicker)?;
    let input = document.create_element("input").map_err(|_| Error::PickerStart)?.dyn_into::<HtmlInputElement>().map_err(|_| Error::UnsupportedPicker)?;
    input.set_type("file");
    input.set_multiple(false);
    if let Some(request) = state.request.take() {
        input.set_accept(request.extension());
        state.request.set(Some(request));
    }
    input.set_hidden(true);
    let change = handler(state, |state| {
        if state.reading.replace(true) {
            return;
        }
        if let Err(error) = begin_read(state) {
            schedule(state, Err(error.to_string()));
        }
    });
    let cancel = handler(state, |state| schedule(state, Ok(None)));
    input.set_onchange(Some(change.as_ref().unchecked_ref()));
    input.set_oncancel(Some(cancel.as_ref().unchecked_ref()));
    state.resources.set(Resources { input: Some(input.clone()), change: Some(change), cancel: Some(cancel), ..Resources::default() });
    let body = document.body().ok_or(Error::UnsupportedPicker)?;
    body.append_child(&input).map_err(|_| Error::PickerStart)?;
    input.show_picker().map_err(|_| Error::PickerStart)
}

fn handler(state: &Rc<State>, mut run: impl FnMut(&Rc<State>) + 'static) -> Handler {
    let weak: Weak<State> = Rc::downgrade(state);
    Closure::wrap(Box::new(move |_event: Event| {
        if let Some(state) = weak.upgrade()
            && !state.terminal.get()
        {
            run(&state);
        }
    }) as Box<dyn FnMut(Event)>)
}

fn begin_read(state: &Rc<State>) -> Result<(), Error> {
    // Cell moves are synchronous and restored before every browser call; no RefCell borrow panic.
    let resources = state.resources.take();
    let input = resources.input.clone();
    state.resources.set(resources);
    let files = input.ok_or(Error::InvalidSelection)?.files().ok_or(Error::InvalidSelection)?;
    if files.length() == 0 {
        schedule(state, Ok(None));
        return Ok(());
    }
    if files.length() != 1 {
        return Err(Error::InvalidSelection);
    }
    let file = files.get(0).ok_or(Error::InvalidSelection)?;
    let length = checked_size(file.size())?;
    let request = state.request.take().ok_or(Error::InvalidSelection)?;
    struct Label {
        value: String,
        _lease: QuotaLease,
    }
    let value = js_sys::Reflect::get(file.as_ref(), &JsValue::from_str("name")).map_err(|_| Error::InvalidSelection)?;
    if !value.is_string() {
        return Err(Error::InvalidSelection);
    }
    let js_name = value.dyn_into::<js_sys::JsString>().map_err(|_| Error::InvalidSelection)?;
    let upper = usize::try_from(js_name.length()).map_err(|_| Error::TooLarge)?.checked_mul(3).ok_or(Error::TooLarge)?;
    let lease = request.reserve_label(upper).map_err(|_| Error::Allocation)?;
    let label = Label { value: js_name.as_string().ok_or(Error::InvalidSelection)?, _lease: lease };
    let builder = request.begin_capture(&label.value, length).map_err(|_| Error::Allocation)?;
    let reader = FileReader::new().map_err(|_| Error::ReadStart)?;
    let load = handler(state, |state| schedule(state, loaded(state)));
    let error = handler(state, |state| schedule(state, Err(Error::ReadFailed.to_string())));
    let abort = handler(state, |state| schedule(state, Err(Error::Aborted.to_string())));
    reader.set_onload(Some(load.as_ref().unchecked_ref()));
    reader.set_onerror(Some(error.as_ref().unchecked_ref()));
    reader.set_onabort(Some(abort.as_ref().unchecked_ref()));
    let mut resources = state.resources.take();
    resources.reader = Some(reader.clone());
    resources.info = Some(builder);
    resources.load = Some(load);
    resources.error = Some(error);
    resources.abort = Some(abort);
    state.resources.set(resources);
    reader.read_as_array_buffer(&file).map_err(|_| Error::ReadStart)
}

fn checked_size(value: f64) -> Result<usize, Error> {
    if !value.is_finite() || value < 0. || value.fract() != 0. {
        return Err(Error::SizeInvalid);
    }
    if value > MAX_BYTES as f64 {
        return Err(Error::TooLarge);
    }
    // At <=16 MiB every integer is exactly represented and fits the wasm32 address space.
    Ok(value as usize)
}

fn loaded(state: &Rc<State>) -> Outcome {
    let mut resources = state.resources.take();
    let result = (|| -> Result<Capture, String> {
        let reader = resources.reader.as_ref().ok_or("missing browser reader")?;
        let buffer =
            reader.result().map_err(|_| "browser XCF read failed")?.dyn_into::<js_sys::ArrayBuffer>().map_err(|_| "invalid browser XCF read result")?;
        let length = usize::try_from(buffer.byte_length()).map_err(|_| "browser XCF read length overflow")?;
        let builder = resources.info.as_mut().ok_or("missing paid browser capture")?;
        if length != builder.expected_length() {
            return Err("browser XCF read length changed".into());
        }
        let constructor = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Uint8Array"))
            .map_err(|_| "missing byte view")?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| "invalid byte view")?;
        let view = js_sys::Reflect::construct(&constructor, &js_sys::Array::of1(buffer.as_ref()))
            .map_err(|_| "byte view failed")?
            .dyn_into::<js_sys::Uint8Array>()
            .map_err(|_| "invalid byte view result")?;
        if view.length() as usize != length || length > MAX_BYTES {
            return Err("browser XCF read length changed".into());
        }
        // Fixed copied chunks; provider and output reservations remain held together.
        builder.with_control(|c| c.check(65_536).map_err(|e| e.to_string()))?;
        let mut block = [0u8; 65_536];
        for begin in (0..length).step_by(block.len()) {
            let end = begin.saturating_add(block.len()).min(length);
            builder.with_control(|c| c.check((end - begin) as u64).map_err(|e| e.to_string()))?;
            let part = block.get_mut(..end - begin).ok_or("browser copy span")?;
            view.subarray(begin as u32, end as u32).copy_to(part);
            builder.append(part)?;
        }
        resources.info.as_mut().ok_or("missing paid browser capture")?.finish()
    })();
    // On error the builder remains in resources until the JS reader is cleared.
    state.resources.set(resources);
    result.map(Some)
}

fn schedule(state: &Rc<State>, outcome: Outcome) {
    if !claim_completion(&state.terminal) {
        return;
    }
    let state = Rc::clone(state);
    // spawn_local always starts on the next microtask tick. Do not destroy the executing Closure.
    wasm_bindgen_futures::spawn_local(async move {
        let mut resources = state.resources.take();
        if let Some(input) = resources.input.as_ref() {
            input.set_onchange(None);
            input.set_oncancel(None);
        }
        if let Some(reader) = resources.reader.as_ref() {
            reader.set_onload(None);
            reader.set_onerror(None);
            reader.set_onabort(None);
            if reader.ready_state() == FileReader::LOADING {
                reader.abort();
            }
        }
        if let Some(input) = resources.input.as_ref() {
            input.remove();
        }
        // Every owned Closure is released only after its JS property has been cleared.
        drop(resources.change.take());
        drop(resources.cancel.take());
        drop(resources.load.take());
        drop(resources.error.take());
        drop(resources.abort.take());
        drop(resources);
        if let Some(callback) = state.callback.take() {
            callback(outcome);
        }
    });
}

fn claim_completion(terminal: &Cell<bool>) -> bool {
    !terminal.replace(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_metadata_size_refuses_before_any_provider_read() {
        assert_eq!(checked_size(MAX_BYTES as f64), Ok(MAX_BYTES));
        for value in [-1.0, 0.5, f64::NAN, f64::INFINITY, MAX_BYTES as f64 + 1.0] {
            assert!(checked_size(value).is_err());
        }
    }
    #[test]
    fn terminal_browser_events_complete_exactly_once() {
        let terminal = Cell::new(false);
        assert!(claim_completion(&terminal));
        assert!(!claim_completion(&terminal));
        assert!(!claim_completion(&terminal));
    }
}
