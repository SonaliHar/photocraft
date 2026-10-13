//! Direct Rust selected-resource picker; no rfd read/accessor or writable handle.
//! `show_picker` runs synchronously. A queued frame may lack user activation and fails visibly.
use std::cell::Cell;
use std::rc::{Rc, Weak};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Event, FileReader, HtmlInputElement};

pub(crate) const MAX_BYTES: usize = 8 << 20;
const MAX_NAME_BYTES: usize = 2048;
type Handler = Closure<dyn FnMut(Event)>;
type Callback = Box<dyn FnOnce(Outcome)>;
pub(crate) type Outcome = Result<Option<SelectedBytes>, Error>;

#[derive(Debug)]
pub(crate) struct SelectedBytes {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

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
    UnexpectedResult,
    Changed,
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
            Self::UnexpectedResult => "xcf.resource.invalid-read-result",
            Self::Changed => "xcf.resource.changed",
        })
    }
}

impl std::error::Error for Error {}

struct FileInfo {
    name: String,
    length: usize,
}

#[derive(Default)]
struct Resources {
    input: Option<HtmlInputElement>,
    reader: Option<FileReader>,
    info: Option<FileInfo>,
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
}

/// Retain until completion; dropping an unfinished handle reports Aborted once.
pub(crate) struct PickerHandle {
    state: Rc<State>,
}

impl PickerHandle {
    pub(crate) fn finished(&self) -> bool {
        self.state.terminal.get()
    }

    pub(crate) fn cancel(&self) {
        schedule(&self.state, Err(Error::Aborted));
    }
}

impl Drop for PickerHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Setup/read errors and user cancellation are delivered through the same typed callback.
/// No await, queued task or click fallback precedes the caught show_picker call.
pub(crate) fn pick_resource(then: impl FnOnce(Outcome) + 'static) -> PickerHandle {
    let state = Rc::new(State {
        terminal: Cell::new(false),
        reading: Cell::new(false),
        resources: Cell::new(Resources::default()),
        callback: Cell::new(Some(Box::new(then))),
    });
    if let Err(error) = setup(&state) {
        schedule(&state, Err(error));
    }
    PickerHandle { state }
}

fn setup(state: &Rc<State>) -> Result<(), Error> {
    let document = web_sys::window().and_then(|window| window.document()).ok_or(Error::UnsupportedPicker)?;
    let input = document.create_element("input").map_err(|_| Error::PickerStart)?.dyn_into::<HtmlInputElement>().map_err(|_| Error::UnsupportedPicker)?;
    input.set_type("file");
    input.set_multiple(false);
    input.set_hidden(true);
    let change = handler(state, |state| {
        if state.reading.replace(true) {
            return;
        }
        if let Err(error) = begin_read(state) {
            schedule(state, Err(error));
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
    // Dedicated substitution discards the transport label. Do not materialize a JS filename.
    let name = bounded_name("selected-resource.png")?;
    let reader = FileReader::new().map_err(|_| Error::ReadStart)?;
    let load = handler(state, |state| schedule(state, loaded(state).map(Some)));
    let error = handler(state, |state| schedule(state, Err(Error::ReadFailed)));
    let abort = handler(state, |state| schedule(state, Err(Error::Aborted)));
    reader.set_onload(Some(load.as_ref().unchecked_ref()));
    reader.set_onerror(Some(error.as_ref().unchecked_ref()));
    reader.set_onabort(Some(abort.as_ref().unchecked_ref()));
    let mut resources = state.resources.take();
    resources.reader = Some(reader.clone());
    resources.info = Some(FileInfo { name, length });
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
    // At <=8 MiB every integer is exactly represented and fits the wasm32 address space.
    Ok(value as usize)
}

fn bounded_name(value: &str) -> Result<String, Error> {
    let mut name = String::new();
    name.try_reserve_exact(value.len().min(MAX_NAME_BYTES)).map_err(|_| Error::Allocation)?;
    for character in value.chars() {
        if name.len().checked_add(character.len_utf8()).ok_or(Error::Allocation)? > MAX_NAME_BYTES {
            break;
        }
        name.push(character);
    }
    Ok(name)
}

fn loaded(state: &Rc<State>) -> Result<SelectedBytes, Error> {
    let mut resources = state.resources.take();
    let reader = resources.reader.clone();
    let info = resources.info.take();
    state.resources.set(resources);
    let info = info.ok_or(Error::UnexpectedResult)?;
    let reader = reader.ok_or(Error::UnexpectedResult)?;
    let buffer = reader.result().map_err(|_| Error::ReadFailed)?.dyn_into::<js_sys::ArrayBuffer>().map_err(|_| Error::UnexpectedResult)?;
    let length = usize::try_from(buffer.byte_length()).map_err(|_| Error::TooLarge)?;
    if length != info.length || length > MAX_BYTES {
        return Err(Error::Changed);
    }
    // Reflect.construct catches synchronous typed-view construction exceptions without product JS.
    let constructor = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Uint8Array"))
        .map_err(|_| Error::UnexpectedResult)?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| Error::UnexpectedResult)?;
    let view = js_sys::Reflect::construct(&constructor, &js_sys::Array::of1(buffer.as_ref()))
        .map_err(|_| Error::UnexpectedResult)?
        .dyn_into::<js_sys::Uint8Array>()
        .map_err(|_| Error::UnexpectedResult)?;
    if usize::try_from(view.length()).map_err(|_| Error::TooLarge)? != length {
        return Err(Error::Changed);
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| Error::Allocation)?;
    bytes.resize(length, 0);
    // Private fixed ArrayBuffer, exact checked length, no intervening callback before copy_to.
    if usize::try_from(view.length()).map_err(|_| Error::TooLarge)? != bytes.len() {
        return Err(Error::Changed);
    }
    view.copy_to(&mut bytes);
    Ok(SelectedBytes { name: info.name, bytes })
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
    fn size_domain_is_checked_before_any_browser_read() {
        assert_eq!(checked_size(0.), Ok(0));
        assert_eq!(checked_size(MAX_BYTES as f64), Ok(MAX_BYTES));
        assert_eq!(checked_size(MAX_BYTES as f64 + 1.), Err(Error::TooLarge));
        for value in [-1., 0.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(checked_size(value), Err(Error::SizeInvalid));
        }
        assert_eq!(checked_size(u64::MAX as f64), Err(Error::TooLarge));
    }
    #[test]
    fn names_and_exposed_errors_are_bounded_plain_rust() {
        let name = bounded_name(&"🖌".repeat(1000)).unwrap();
        assert_eq!(name.len(), MAX_NAME_BYTES);
        assert_eq!(Error::PickerStart.to_string(), "xcf.resource.picker-start-failed");
        assert_eq!(Error::ReadFailed.to_string(), "xcf.resource.read-failed");
    }
    #[test]
    fn competing_terminal_events_claim_exactly_one_completion() {
        let terminal = Cell::new(false);
        assert!(claim_completion(&terminal));
        assert!(!claim_completion(&terminal));
        assert!(!claim_completion(&terminal));
        assert!(terminal.get());
    }
}
