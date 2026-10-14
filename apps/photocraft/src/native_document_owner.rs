//! Native ownership and initial placement for document viewports; unrelated windows keep their role.
use egui::ViewportId;
use raw_window_handle::HasWindowHandle;
#[cfg(target_os = "linux")]
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, MutexGuard, Weak},
};
use winit::dpi::{PhysicalPosition, PhysicalSize, Size};
use winit::raw_window_handle;
use winit::window::{Window, WindowAttributes};
#[cfg(target_os = "linux")]
const CREATION_BUDGET: Duration = Duration::from_millis(50);
#[derive(Clone, Debug)]
pub enum OwnerEvent {
    Attributes { id: ViewportId, owned: bool, visible: bool },
    RootCreated,
    ChildCreated { id: ViewportId, hidden: bool, transient: bool, restored_visible: bool, detail: String },
}
type Observer = Arc<dyn Fn(OwnerEvent) + Send + Sync>;
#[derive(Clone)]
pub struct NativeOwners {
    shared: Arc<Mutex<State>>,
    observer: Option<Observer>,
    #[cfg(target_os = "linux")]
    logged: Arc<AtomicBool>,
}
struct State {
    requested: HashSet<ViewportId>,
    root: Weak<Window>,
    #[cfg(target_os = "linux")]
    generation: u64,
    #[cfg(target_os = "linux")]
    pending: HashMap<ViewportId, Pending>,
    #[cfg(target_os = "linux")]
    worker: Option<std::sync::mpsc::SyncSender<Request>>,
}
#[cfg(target_os = "linux")]
struct Pending {
    visible: bool,
    root: Weak<Window>,
    generation: u64,
}
impl Default for NativeOwners {
    fn default() -> Self {
        Self::new()
    }
}
impl NativeOwners {
    pub fn new() -> Self {
        Self::with_observer(None)
    }
    pub fn with_observer(observer: Option<Observer>) -> Self {
        #[cfg(target_os = "linux")]
        let worker = start_worker();
        Self {
            shared: Arc::new(Mutex::new(State {
                requested: HashSet::new(),
                root: Weak::new(),
                #[cfg(target_os = "linux")]
                generation: 0,
                #[cfg(target_os = "linux")]
                pending: HashMap::new(),
                #[cfg(target_os = "linux")]
                worker,
            })),
            observer,
            #[cfg(target_os = "linux")]
            logged: Arc::new(AtomicBool::new(false)),
        }
    }
    // Poison recovery preserves pending hidden-window visibility data.
    fn state(&self) -> MutexGuard<'_, State> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Exact IDs supplied by the app before document callbacks. No native I/O.
    pub fn set_document_viewports(&self, ids: &[ViewportId]) {
        self.state().requested = ids.iter().copied().filter(|id| *id != ViewportId::ROOT).collect();
    }
    pub fn hooks(&self) -> eframe::NativeViewportHooks {
        let attrs = self.clone();
        let created = self.clone();
        eframe::NativeViewportHooks {
            window_attributes: Some(Arc::new(move |ids, _builder, attributes| {
                let (attributes, owned) = attrs.owner_attributes(ids.this, attributes);
                attrs.emit(OwnerEvent::Attributes { id: ids.this, owned, visible: attributes.visible });
                attributes
            })),
            window_created: Some(Arc::new(move |ids, _builder, window| {
                if ids.this == ViewportId::ROOT {
                    {
                        let mut state = created.state();
                        state.root = Arc::downgrade(window);
                        #[cfg(target_os = "linux")]
                        {
                            state.generation = state.generation.wrapping_add(1);
                        }
                    }
                    created.emit(OwnerEvent::RootCreated);
                    return;
                }
                #[cfg(target_os = "linux")]
                created.x11_created(ids.this, window);
                #[cfg(target_os = "windows")]
                created.emit(OwnerEvent::ChildCreated {
                    id: ids.this,
                    hidden: !window.is_visible().unwrap_or(true),
                    transient: false,
                    restored_visible: window.is_visible().unwrap_or(true),
                    detail: "Windows owner supplied at creation; readback required".into(),
                });
            })),
        }
    }
    fn owner_attributes(&self, id: ViewportId, attributes: WindowAttributes) -> (WindowAttributes, bool) {
        if id == ViewportId::ROOT {
            return (attributes, false);
        }
        #[cfg(target_os = "linux")]
        {
            let mut state = self.state();
            // Failed creates have no Window callback. Keep restoration records until callback,
            // but bound that exceptional pending set and fail open visibly beyond its limit.
            if state.requested.contains(&id)
                && state.worker.is_some()
                && (state.pending.len() < 256 || state.pending.contains_key(&id))
                && let Some(root) = state.root.upgrade().filter(|r| x11_key(r).is_some())
            {
                let pending = Pending { visible: attributes.visible, root: Arc::downgrade(&root), generation: state.generation };
                state.pending.insert(id, pending);
                return (at_parent(attributes, &root).with_visible(false), true);
            }
        }
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            let state = self.state();
            if state.requested.contains(&id)
                && let Some(root) = state.root.upgrade()
                && let Ok(handle) = root.window_handle()
                && let raw_window_handle::RawWindowHandle::Win32(handle) = handle.as_raw()
            {
                return (at_parent(attributes, &root).with_owner_window(handle.hwnd.get()), true);
            }
        }
        (attributes, false)
    }
    fn emit(&self, event: OwnerEvent) {
        if let Some(observer) = &self.observer {
            observer(event);
        }
    }
    #[cfg(target_os = "linux")]
    fn x11_created(&self, id: ViewportId, window: &Arc<Window>) {
        let Some((pending, sender, valid)) = ({
            let mut state = self.state();
            state.pending.remove(&id).map(|pending| {
                let valid = state.requested.contains(&id) && pending.generation == state.generation;
                (pending, state.worker.clone(), valid)
            })
        }) else {
            return;
        };
        // Guard exists before every fallible handle/property/worker path, including poison recovery.
        let visibility = RestoreVisibility { window: window.clone(), visible: pending.visible };
        let hidden = !window.is_visible().unwrap_or(true);
        // Start the budget at this actual creation, never at role registration/earlier canvas work.
        let until = Instant::now() + CREATION_BUDGET;
        let result = (|| {
            if !valid {
                return Err("document role/root generation changed".to_owned());
            }
            let root = pending.root.upgrade().ok_or_else(|| "root expired".to_owned())?;
            let root_key = x11_key(&root).ok_or_else(|| "root is not X11".to_owned())?;
            let child_key = x11_key(window).ok_or_else(|| "child is not X11".to_owned())?;
            if root_key.display != child_key.display || root_key.kind != child_key.kind || root_key.xid == child_key.xid {
                return Err("root/child native display mismatch".into());
            }
            let (answer, receive) = std::sync::mpsc::sync_channel(1);
            let cancel = Arc::new(AtomicBool::new(false));
            let request = Request {
                root: Arc::downgrade(&root),
                child: Arc::downgrade(window),
                root_xid: root_key.xid,
                child_xid: child_key.xid,
                until,
                cancel: cancel.clone(),
                answer,
            };
            sender.ok_or_else(|| "worker unavailable".to_owned())?.try_send(request).map_err(|_| "worker busy/disconnected".to_owned())?;
            let reply = receive.recv_timeout(until.saturating_duration_since(Instant::now()));
            cancel.store(true, Ordering::Release);
            reply.map_err(|_| "X11 owner deadline".to_owned())?
        })();
        drop(visibility);
        let transient = result.is_ok();
        let detail = result.unwrap_or_else(|e| {
            if !self.logged.swap(true, Ordering::AcqRel) {
                eprintln!("Native document ownership unavailable: {e}");
            }
            e
        });
        self.emit(OwnerEvent::ChildCreated { id, hidden, transient, restored_visible: window.is_visible().unwrap_or(true), detail });
    }
}

fn at_parent(attributes: WindowAttributes, root: &Window) -> WindowAttributes {
    if attributes.position.is_some() || root.is_minimized() == Some(true) {
        return attributes;
    }
    // Wayland can withhold global coordinates; leave placement to its compositor.
    let Ok(position) = root.inner_position() else { return attributes };
    default_position(attributes, position, root.inner_size(), root.scale_factor())
}

fn default_position(mut attributes: WindowAttributes, parent: PhysicalPosition<i32>, parent_size: PhysicalSize<u32>, scale: f64) -> WindowAttributes {
    if attributes.position.is_some() || !scale.is_finite() || scale <= 0.0 || parent_size.width == 0 || parent_size.height == 0 {
        return attributes;
    }
    let Some(size) = attributes.inner_size else { return attributes };
    if let Size::Logical(size) = size
        && (!size.width.is_finite() || !size.height.is_finite() || size.width <= 0.0 || size.height <= 0.0)
    {
        return attributes;
    }
    let size = size.to_physical::<u32>(scale);
    if size.width == 0 || size.height == 0 {
        return attributes;
    }
    let x = i64::from(parent.x) + i64::from(parent_size.width.saturating_sub(size.width)) / 2;
    let y = i64::from(parent.y) + i64::from(parent_size.height.saturating_sub(size.height)) / 2;
    if let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y)) {
        // Owned Windows windows use WS_POPUP: CW_USEDEFAULT places those at (0, 0).
        // Physical coordinates also avoid applying the primary monitor's DPI to this offset.
        attributes = attributes.with_position(PhysicalPosition::new(x, y));
    }
    attributes
}
#[cfg(target_os = "linux")]
struct RestoreVisibility {
    window: Arc<Window>,
    visible: bool,
}
#[cfg(target_os = "linux")]
impl Drop for RestoreVisibility {
    fn drop(&mut self) {
        self.window.set_visible(self.visible);
    }
}
#[cfg(target_os = "linux")]
struct X11Key {
    xid: u32,
    display: usize,
    kind: u8,
}
#[cfg(target_os = "linux")]
fn x11_key(window: &Window) -> Option<X11Key> {
    use raw_window_handle::{HasDisplayHandle, RawDisplayHandle, RawWindowHandle};
    match (window.window_handle().ok()?.as_raw(), window.display_handle().ok()?.as_raw()) {
        (RawWindowHandle::Xlib(w), RawDisplayHandle::Xlib(d)) => {
            Some(X11Key { xid: w.window.try_into().ok().filter(|x| *x != 0)?, display: d.display?.as_ptr() as usize, kind: 1 })
        }
        (RawWindowHandle::Xcb(w), RawDisplayHandle::Xcb(d)) => Some(X11Key { xid: w.window.get(), display: d.connection?.as_ptr() as usize, kind: 2 }),
        _ => None,
    }
}
#[cfg(target_os = "linux")]
struct Request {
    root: Weak<Window>,
    child: Weak<Window>,
    root_xid: u32,
    child_xid: u32,
    until: Instant,
    cancel: Arc<AtomicBool>,
    answer: std::sync::mpsc::SyncSender<Result<String, String>>,
}
#[cfg(target_os = "linux")]
fn start_worker() -> Option<std::sync::mpsc::SyncSender<Request>> {
    let (send, receive) = std::sync::mpsc::sync_channel::<Request>(1);
    std::thread::Builder::new()
        .name("document-native-owner".into())
        .spawn(move || {
            let mut connection = connect_local(Instant::now() + Duration::from_secs(1));
            while let Ok(request) = receive.recv() {
                // Skips before protocol IO never poison an otherwise usable connection.
                if request.cancel.load(Ordering::Acquire) || Instant::now() >= request.until {
                    let _ = request.answer.try_send(Err("request canceled/expired".into()));
                    continue;
                }
                // Same single worker; only a later actual creation can reconnect after transport loss.
                // No native strong holds while authentication filesystem IO is pending.
                if connection.is_err() {
                    connection = connect_local(request.until);
                }
                let result = (|| {
                    let (conn, deadline) = connection.as_ref().map_err(|e| AttachFailure::Rejected(e.clone()))?;
                    let _root = request.root.upgrade().ok_or_else(|| AttachFailure::Rejected("root expired".into()))?;
                    let _child = request.child.upgrade().ok_or_else(|| AttachFailure::Rejected("child expired".into()))?;
                    if request.cancel.load(Ordering::Acquire) || Instant::now() >= request.until {
                        return Err(AttachFailure::Rejected("request canceled/expired".into()));
                    }
                    deadline.set(request.until, request.cancel.clone()).map_err(|e| AttachFailure::Broken(e.to_string()))?;
                    attach(conn, request.root_xid, request.child_xid)
                })();
                // Timeout/transport/parse failures may leave unread replies; drained semantic rejection does not.
                if matches!(result, Err(AttachFailure::Broken(_))) {
                    connection = Err("X11 transport no longer usable".into());
                }
                let _ = request.answer.try_send(result.map_err(|e| match e {
                    AttachFailure::Rejected(s) | AttachFailure::Broken(s) => s,
                }));
            }
        })
        .ok()
        .map(|_| send)
}
#[cfg(target_os = "linux")]
type XConnection = x11rb::rust_connection::RustConnection<crate::x11_deadline::DeadlineStream>;
#[cfg(target_os = "linux")]
fn connect_local(until: Instant) -> Result<(XConnection, crate::x11_deadline::Deadline), String> {
    use x11rb::reexports::x11rb_protocol::parse_display::{ConnectAddress, parse_display};
    let parsed = parse_display(None).map_err(|e| e.to_string())?;
    if !parsed.host.is_empty() && parsed.host != "unix" {
        return Err("only local Unix X11 displays supported".into());
    }
    let path = parsed
        .connect_instruction()
        .find_map(|a| if let ConnectAddress::Socket(path) = a { Some(path) } else { None })
        .ok_or_else(|| "no local X11 socket".to_owned())?;
    let deadline = crate::x11_deadline::Deadline::new(until);
    let (stream, (family, address)) = crate::x11_deadline::DeadlineStream::connect(&path, deadline.clone()).map_err(|e| e.to_string())?;
    // Authentication lookup is on this one worker only. Its filesystem calls cannot
    // be force-canceled; the UI deadline/visibility guard never waits for them indefinitely.
    let (name, data) = bounded_auth(family, &address, parsed.display, until)?.unwrap_or_default();
    if Instant::now() >= until {
        return Err("X11 auth deadline".into());
    }
    let connection = XConnection::connect_to_stream_with_auth_info(stream, parsed.screen.into(), name, data).map_err(|e| e.to_string())?;
    Ok((connection, deadline))
}
#[cfg(target_os = "linux")]
#[derive(Debug)]
enum AttachFailure {
    Broken(String),
    Rejected(String),
}
#[cfg(target_os = "linux")]
impl From<x11rb::errors::ConnectionError> for AttachFailure {
    fn from(error: x11rb::errors::ConnectionError) -> Self {
        Self::Broken(error.to_string())
    }
}
#[cfg(target_os = "linux")]
impl From<x11rb::errors::ReplyError> for AttachFailure {
    fn from(error: x11rb::errors::ReplyError) -> Self {
        match error {
            x11rb::errors::ReplyError::ConnectionError(e) => Self::Broken(e.to_string()),
            x11rb::errors::ReplyError::X11Error(e) => Self::Rejected(format!("{e:?}")),
        }
    }
}
#[cfg(target_os = "linux")]
fn attach(connection: &XConnection, root: u32, child: u32) -> Result<String, AttachFailure> {
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt, PropMode},
        wrapper::ConnectionExt as _,
    };
    let operation = || -> Result<String, AttachFailure> {
        let transient = connection.intern_atom(false, b"WM_TRANSIENT_FOR")?.reply()?.atom;
        let leader = connection.intern_atom(false, b"WM_CLIENT_LEADER")?.reply()?.atom;
        // Validate both trusted native handles on this very connection/screen before writing.
        let root_tree = connection.query_tree(root)?.reply()?;
        let child_tree = connection.query_tree(child)?.reply()?;
        if root_tree.root != child_tree.root {
            return Err(AttachFailure::Rejected("root/child X11 screens differ".into()));
        }
        let attributes = connection.get_window_attributes(child)?.reply()?;
        if attributes.map_state != x11rb::protocol::xproto::MapState::UNMAPPED {
            return Err(AttachFailure::Rejected("child already mapped before ownership; preserved".into()));
        }
        let old = connection.get_property(false, child, transient, AtomEnum::ANY, 0, 2)?.reply()?;
        let old_leader = connection.get_property(false, child, leader, AtomEnum::ANY, 0, 2)?.reply()?;
        if old.type_ != u32::from(AtomEnum::NONE) {
            if old.type_ != u32::from(AtomEnum::WINDOW)
                || old.format != 32
                || old.bytes_after != 0
                || old.value32().map(|v| v.collect::<Vec<_>>()) != Some(vec![root])
            {
                return Err(AttachFailure::Rejected("existing WM_TRANSIENT_FOR conflicts; preserved".into()));
            }
        } else {
            connection.change_property32(PropMode::REPLACE, child, transient, AtomEnum::WINDOW, &[root])?.check()?;
        }
        connection.flush()?;
        let now = connection.get_property(false, child, transient, AtomEnum::WINDOW, 0, 2)?.reply()?;
        let new_leader = connection.get_property(false, child, leader, AtomEnum::ANY, 0, 2)?.reply()?;
        if now.format != 32 || now.bytes_after != 0 || now.value32().map(|v| v.collect::<Vec<_>>()) != Some(vec![root]) {
            return Err(AttachFailure::Rejected("transient readback mismatch".into()));
        }
        if old_leader.type_ != new_leader.type_
            || old_leader.format != new_leader.format
            || old_leader.bytes_after != new_leader.bytes_after
            || old_leader.value != new_leader.value
        {
            return Err(AttachFailure::Rejected("WM_CLIENT_LEADER changed unexpectedly".into()));
        }
        Ok(format!("WM_TRANSIENT_FOR={root}; leader preserved"))
    };
    operation()
}

#[cfg(target_os = "linux")]
type AuthInfo = (Vec<u8>, Vec<u8>);
#[cfg(target_os = "linux")]
fn bounded_auth(family: x11rb::reexports::x11rb_protocol::xauth::Family, address: &[u8], display: u16, until: Instant) -> Result<Option<AuthInfo>, String> {
    use std::{fs::File, io::Read, path::PathBuf};
    let path = std::env::var_os("XAUTHORITY").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".Xauthority")));
    let Some(path) = path else {
        return Ok(None);
    };
    if Instant::now() >= until {
        return Err("auth deadline".into());
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    const MAX_AUTH_BYTES: usize = 1024 * 1024;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_AUTH_BYTES as u64 {
        return Err("unsupported/oversized Xauthority file".into());
    }
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= until {
            return Err("auth deadline".into());
        }
        let mut chunk = [0; 4096];
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_AUTH_BYTES {
            return Err("Xauthority byte limit".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    parse_auth(&bytes, family, address, display)
}
#[cfg(target_os = "linux")]
fn parse_auth(mut bytes: &[u8], family: x11rb::reexports::x11rb_protocol::xauth::Family, address: &[u8], display: u16) -> Result<Option<AuthInfo>, String> {
    use x11rb::reexports::x11rb_protocol::xauth::Family;
    fn u16_field(bytes: &mut &[u8]) -> Result<u16, String> {
        let field = bytes.get(..2).ok_or_else(|| "truncated Xauthority".to_owned())?;
        let value = u16::from_be_bytes([field[0], field[1]]);
        *bytes = &bytes[2..];
        Ok(value)
    }
    fn field<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8], String> {
        let len = usize::from(u16_field(bytes)?);
        let value = bytes.get(..len).ok_or_else(|| "truncated Xauthority".to_owned())?;
        *bytes = &bytes[len..];
        Ok(value)
    }
    let number = display.to_string();
    while !bytes.is_empty() {
        let entry_family = Family::from(u16_field(&mut bytes)?);
        let entry_address = field(&mut bytes)?;
        let entry_number = field(&mut bytes)?;
        let name = field(&mut bytes)?;
        let data = field(&mut bytes)?;
        if (entry_family == Family::WILD || family == Family::WILD || (entry_family == family && entry_address == address))
            && (entry_number.is_empty() || entry_number == number.as_bytes())
            && name == b"MIT-MAGIC-COOKIE-1"
        {
            return Ok(Some((name.to_vec(), data.to_vec())));
        }
    }
    Ok(None)
}
#[cfg(test)]
mod position_tests {
    use super::*;
    #[test]
    fn default_position_uses_parent_scale_and_preserves_negative_screen_coordinates() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for parent in [PhysicalPosition::new(1920, 100), PhysicalPosition::new(-2400, -1600)] {
                let attributes = WindowAttributes::default().with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
                let requested_size = attributes.inner_size;
                let placed = default_position(attributes, parent, PhysicalSize::new(2400, 1600), scale);
                let child = requested_size.unwrap().to_physical::<u32>(scale);
                assert_eq!(
                    placed.position,
                    Some(PhysicalPosition::new(parent.x + (2400 - child.width as i32) / 2, parent.y + (1600 - child.height as i32) / 2).into())
                );
                assert_eq!(placed.inner_size, requested_size);
            }
        }
    }
    #[test]
    fn default_position_keeps_explicit_requests_and_oversized_titlebars_at_parent() {
        let parent = PhysicalPosition::new(-900, 80);
        let size = PhysicalSize::new(400, 300);
        let attributes = WindowAttributes::default().with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
        assert_eq!(default_position(attributes.clone(), parent, size, 2.0).position, Some(parent.into()));
        for position in [winit::dpi::Position::from(PhysicalPosition::new(50, 60)), winit::dpi::LogicalPosition::new(-500.5, 120.0).into()] {
            assert_eq!(default_position(attributes.clone().with_position(position), parent, size, 2.0).position, Some(position));
        }
    }
    #[test]
    fn default_position_leaves_unavailable_or_invalid_geometry_to_the_window_system() {
        let parent = PhysicalPosition::new(1920, 100);
        let size = PhysicalSize::new(2400, 1600);
        let attributes = WindowAttributes::default().with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(default_position(attributes.clone(), parent, size, scale).position, None);
        }
        for inner_size in [None, Some(Size::Physical(PhysicalSize::new(0, 300))), Some(winit::dpi::LogicalSize::new(f64::NAN, 600.0).into())] {
            let mut invalid = attributes.clone();
            invalid.inner_size = inner_size;
            assert_eq!(default_position(invalid, parent, size, 1.0).position, None);
        }
        assert_eq!(default_position(attributes.clone(), parent, PhysicalSize::new(0, 1600), 1.0).position, None);
        assert_eq!(default_position(attributes, PhysicalPosition::new(i32::MAX, i32::MAX), size, 1.0).position, None);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use x11rb::reexports::x11rb_protocol::xauth::Family;
    #[test]
    #[ignore = "requires an X11 display with room for a window at (2000, 80)"]
    fn default_document_position_uses_actual_parent_geometry() {
        use winit::{
            application::ApplicationHandler,
            event::WindowEvent,
            event_loop::{ActiveEventLoop, EventLoop},
            platform::x11::EventLoopBuilderExtX11,
        };
        struct Probe;
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                let owners = NativeOwners::new();
                let hooks = owners.hooks();
                let created = hooks.window_created.as_ref().unwrap();
                let attributes = hooks.window_attributes.as_ref().unwrap();
                let root = Arc::new(
                    event_loop
                        .create_window(
                            WindowAttributes::default()
                                .with_decorations(false)
                                .with_position(winit::dpi::PhysicalPosition::new(2000, 80))
                                .with_inner_size(winit::dpi::PhysicalSize::new(1000, 800)),
                        )
                        .unwrap(),
                );
                let root_ids = egui::ViewportIdPair { this: ViewportId::ROOT, parent: ViewportId::ROOT };
                created(root_ids, &Default::default(), &root);
                let id = ViewportId::from_hash_of("placement-probe");
                owners.set_document_viewports(&[id]);
                let ids = egui::ViewportIdPair { this: id, parent: ViewportId::ROOT };
                let requested = || WindowAttributes::default().with_decorations(false).with_inner_size(winit::dpi::PhysicalSize::new(400, 300));
                let placed = attributes(ids, &Default::default(), requested());
                let at = placed.position.expect("default document float needs a parent-relative initial position").to_physical::<i32>(root.scale_factor());
                let parent_at = root.inner_position().unwrap();
                let parent_size = root.inner_size();
                assert_eq!(
                    at,
                    winit::dpi::PhysicalPosition::new(parent_at.x + (parent_size.width as i32 - 400) / 2, parent_at.y + (parent_size.height as i32 - 300) / 2)
                );
                let child = Arc::new(event_loop.create_window(placed).unwrap());
                created(ids, &Default::default(), &child);
                assert_eq!(child.outer_position().unwrap(), at, "native creation applies the requested physical position");
                let explicit = winit::dpi::PhysicalPosition::new(50, 60);
                assert_eq!(attributes(ids, &Default::default(), requested().with_position(explicit)).position, Some(explicit.into()));
                let unrelated = egui::ViewportIdPair { this: ViewportId::from_hash_of("unrelated-probe"), parent: ViewportId::ROOT };
                assert_eq!(attributes(unrelated, &Default::default(), requested()).position, None);
                event_loop.exit();
            }
            fn window_event(&mut self, _: &ActiveEventLoop, _: winit::window::WindowId, _: WindowEvent) {}
        }
        let event_loop = EventLoop::builder().with_x11().with_any_thread(true).build().unwrap();
        event_loop.run_app(&mut Probe).unwrap();
    }
    #[test]
    fn poisoned_state_recovers_pending_visibility_data() {
        let owners = NativeOwners::new();
        let id = ViewportId::from_hash_of("poison-preservation");
        owners.state().pending.insert(id, Pending { visible: true, root: Weak::new(), generation: 7 });
        let shared = owners.shared.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = shared.lock().unwrap();
            panic!("synthetic state poison");
        });
        owners.set_document_viewports(&[id]);
        let mut state = owners.state();
        assert!(state.requested.contains(&id));
        assert!(state.pending.remove(&id).unwrap().visible);
    }
    #[test]
    fn auth_matches_only_the_local_display_or_explicit_wildcard() {
        let mut data = 256u16.to_be_bytes().to_vec();
        for part in [b"host".as_slice(), b"0", b"MIT-MAGIC-COOKIE-1", b"synthetic-cookie"] {
            data.extend_from_slice(&(part.len() as u16).to_be_bytes());
            data.extend_from_slice(part);
        }
        assert!(parse_auth(&data, Family::LOCAL, b"other", 0).unwrap().is_none());
        assert!(parse_auth(&data, Family::LOCAL, b"host", 1).unwrap().is_none());
        assert_eq!(parse_auth(&data, Family::LOCAL, b"host", 0).unwrap().unwrap().1, b"synthetic-cookie");
        data[..2].copy_from_slice(&65535u16.to_be_bytes());
        assert!(parse_auth(&data, Family::LOCAL, b"other", 0).unwrap().is_some());
        data.pop();
        assert!(parse_auth(&data, Family::LOCAL, b"host", 0).is_err());
    }
}
