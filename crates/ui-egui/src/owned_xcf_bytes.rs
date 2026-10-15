//! Explicit browser byte intake; names are labels and never filesystem grants.
use crate::PhotocraftApp;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender, TryRecvError},
};
const CAP: usize = 16 << 20;
const WORK: u64 = 8_000_000_000;
struct Work {
    used: AtomicU64,
    cancelled: AtomicBool,
    _lease: QuotaLease,
}
#[derive(Clone)]
pub struct Request {
    quota: BackingQuota,
    work: Arc<Work>,
}
impl Request {
    fn new(quota: BackingQuota) -> Result<Self, String> {
        let lease = quota.reserve(QuotaKind::Index, 4096).map_err(|e| e.to_string())?;
        if std::mem::size_of::<Work>() + std::mem::size_of::<Pending>() + 128 > 4096 {
            return Err("owned byte picker header exceeds admission".into());
        }
        Ok(Self { quota, work: Arc::new(Work { used: AtomicU64::new(0), cancelled: AtomicBool::new(false), _lease: lease }) })
    }
    pub fn with_control<R>(&self, f: impl FnOnce(&ReadControl<'_>) -> Result<R, String>) -> Result<R, String> {
        let cancelled = || self.work.cancelled.load(Ordering::Relaxed);
        let interrupt = Interrupt::cancel_only(&cancelled);
        let charge = |n: u64| {
            self.work.used.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(n).filter(|v| *v <= WORK)).map_err(|_| BackingError::Work)?;
            Ok(())
        };
        let ctl = ReadControl { interrupt: &interrupt, work: &charge };
        ctl.check(0).map_err(|e| e.to_string())?;
        f(&ctl)
    }
    /// Temporary bounded browser-label conversion, before materializing Rust text.
    pub fn reserve_label(&self, bytes: usize) -> Result<QuotaLease, String> {
        if bytes > 4096 {
            return Err("browser XCF label too long".into());
        }
        self.with_control(|c| c.check(bytes as u64).map_err(|e| e.to_string()))?;
        self.quota.reserve(QuotaKind::Resident, bytes as u64 + 128).map_err(|e| e.to_string())
    }
    pub fn begin_capture(self, name: &str, length: usize) -> Result<CaptureBuilder, String> {
        if length == 0 || length > CAP || name.is_empty() || name.len() > 4096 {
            return Err("owned XCF bytes exceed name/input limits".into());
        }
        self.with_control(|c| {
            c.check(name.len() as u64).map_err(|e| e.to_string())?;
            if name.contains('\0') {
                return Err("owned XCF label contains NUL".into());
            }
            Ok(())
        })?;
        let capacity = length.checked_add(64).ok_or("byte capacity overflow")?;
        let fee = capacity
            .checked_add(name.len().checked_mul(2).ok_or("name capacity overflow")?)
            .and_then(|n| n.checked_add(4096))
            .ok_or("capture capacity overflow")?;
        let lease = self.quota.reserve(QuotaKind::Resident, fee as u64).map_err(|e| e.to_string())?;
        // Provider ArrayBuffer and the bounded host copy block coexist with Rust bytes.
        let provider = self
            .quota
            .reserve(QuotaKind::Resident, (length as u64).checked_add(65_536 + 4096).ok_or("provider capacity overflow")?)
            .map_err(|e| e.to_string())?;
        if std::mem::size_of::<Capture>() + std::mem::size_of::<CaptureBuilder>() + std::mem::size_of::<Vec<u8>>() + 128 > 4096 {
            return Err("capture header exceeds admission".into());
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| "byte allocation failed")?;
        let mut label = String::new();
        label.try_reserve_exact(name.len()).map_err(|_| "name allocation failed")?;
        if bytes.capacity() > capacity || label.capacity() > name.len() * 2 {
            return Err("capture capacity exceeds admission".into());
        }
        label.push_str(name);
        Ok(CaptureBuilder { bytes, name: label, length, request: self, _lease: Some(lease), _provider: Some(provider) })
    }
}
// Payload always precedes the reservations that pay for its full overlap.
pub struct CaptureBuilder {
    bytes: Vec<u8>,
    name: String,
    length: usize,
    request: Request,
    _lease: Option<QuotaLease>,
    _provider: Option<QuotaLease>,
}
impl CaptureBuilder {
    pub fn with_control<R>(&self, f: impl FnOnce(&ReadControl<'_>) -> Result<R, String>) -> Result<R, String> {
        self.request.with_control(f)
    }
    pub fn append(&mut self, part: &[u8]) -> Result<(), String> {
        if self._lease.is_none() || self._provider.is_none() || part.len() > 65_536 || self.bytes.len().checked_add(part.len()).is_none_or(|n| n > self.length)
        {
            return Err("invalid captured byte chunk".into());
        }
        self.request.with_control(|c| c.check(part.len() as u64).map_err(|e| e.to_string()))?;
        self.bytes.extend_from_slice(part);
        Ok(())
    }
    pub fn expected_length(&self) -> usize {
        self.length
    }
    pub fn finish(&mut self) -> Result<Capture, String> {
        if self.bytes.len() != self.length || self._lease.is_none() || self._provider.is_none() {
            return Err("incomplete owned XCF capture".into());
        }
        self.request.with_control(|c| c.check(1).map_err(|e| e.to_string()))?;
        let lease = self._lease.take().ok_or("capture already consumed")?;
        let provider = self._provider.take().ok_or("capture already consumed")?;
        Ok(Capture {
            bytes: Arc::new(std::mem::take(&mut self.bytes)),
            name: std::mem::take(&mut self.name),
            request: self.request.clone(),
            _lease: lease,
            _provider: provider,
        })
    }
}
pub struct Capture {
    bytes: Arc<Vec<u8>>,
    name: String,
    request: Request,
    _lease: QuotaLease,
    _provider: QuotaLease,
}
pub struct Reply {
    sender: Sender<Result<Option<Capture>, String>>,
}
impl Reply {
    pub fn send(self, value: Result<Option<Capture>, String>) {
        let _ = self.sender.send(value);
    }
}
/// Host-local cancellation; no Send or filesystem authority is implied.
pub type Cancel = Box<dyn FnOnce()>;
pub type PickerFn = Box<dyn FnMut(Request, Reply) -> Option<Cancel>>;
pub(crate) struct Pending {
    receiver: Receiver<Result<Option<Capture>, String>>,
    request: Request,
    cancel: Option<Cancel>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.request.work.cancelled.store(true, Ordering::Relaxed);
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}
pub(crate) fn start(app: &mut PhotocraftApp) -> Result<Value, String> {
    if app.automation_input {
        return Err("browser picker cannot be opened by scoped automation".into());
    }
    if app.file_dialog_open() {
        return Err("a file dialog is already open".into());
    }
    if app.services.owned_xcf_bytes_picker.is_none() {
        return Err("owned XCF byte picker is unavailable".into());
    }
    let free = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let quota = match app.session.accounted_open_quota() {
        Some(q) => q.clone(),
        None => photocraft_io::accounted_open_quota::new_accounted_open_quota(&free).map_err(|e| e.to_string())?,
    };
    let request = Request::new(quota)?;
    let response = json!({"fileDialog":"open"});
    let (sender, receiver) = mpsc::channel();
    app.owned_xcf_bytes_pending = Some(Pending { receiver, request: request.clone(), cancel: None });
    let picker = app.services.owned_xcf_bytes_picker.as_mut().ok_or("owned XCF byte picker is unavailable")?;
    let cancel = picker(request, Reply { sender });
    if let Some(pending) = app.owned_xcf_bytes_pending.as_mut() {
        pending.cancel = cancel;
    }
    Ok(response)
}
fn apply(app: &mut PhotocraftApp, capture: &Capture) -> Result<Value, String> {
    capture.request.with_control(|control| {
        if app.session.accounted_open_quota().is_some_and(|q| !q.shares_ledger(&capture.request.quota)) {
            return Err("catalog quota changed during browser selection".into());
        }
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .map_err(|e| e.to_string())?
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("encodedBytes", CAP as u64), ("logicalBytes", CAP as u64), ("scratchBytes", 0)])
            .map_err(|e| e.to_string())?;
        let mut options = photocraft_xcf::DecodeOptions::default();
        options.limits.resident_bytes = 128 << 20;
        options.limits.input_bytes = CAP;
        let imported = photocraft_io::import_owned_xcf_bytes_in_quota(
            &capture.name,
            Arc::clone(&capture.bytes),
            &options,
            &limits,
            capture.request.quota.clone(),
            control,
        )
        .map_err(|e| e.to_string())?;
        crate::owned_xcf_open::adopt_bytes(app, &capture.name, &imported, control)
    })
}
pub(crate) fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(pending) = app.owned_xcf_bytes_pending.as_ref() else { return };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.owned_xcf_bytes_pending = None;
        return;
    }
    let result = match pending.receiver.try_recv() {
        Err(TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        Err(TryRecvError::Disconnected) => Err("owned XCF byte picker disconnected".into()),
        Ok(v) => v,
    };
    let held = app.owned_xcf_bytes_pending.take();
    let outcome = match result {
        Ok(Some(capture)) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(app, &capture)))
            .unwrap_or_else(|_| Err("owned XCF byte import failed internally; existing documents retained".into())),
        Ok(None) => Ok(Value::Null),
        Err(e) => Err(e),
    };
    drop(held);
    if let Err(error) = outcome {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    ctx.request_repaint();
}

#[cfg(test)]
#[path = "owned_xcf_bytes_tests.rs"]
mod tests;
