//! Native platform services: file dialogs (rfd), filesystem, codecs.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest, Recovered, Services};
use std::cell::RefCell;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

/// Everything File › Open reads: PhotoCraft and Photoshop documents, flat images, and Photoshop
/// brushes (.abr), gradients (.grd) and swatches (.aco, .ase), which go to the preset libraries.
const OPEN_EXTS: &[&str] = &[
    "xcf", "xcf.gz", "xcf.bz2", "xcf.xz", "pcraft", "psd", "psb", "psdt", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "tga", "ico", "qoi",
    "exr", "hdr", "pbm", "pgm", "ppm", "pam", "pfm", "heic", "heif", "hif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd",
    "svg", "svgz", "aco", "ase",
];

/// File › Save As formats: (filter name, extensions). The filter matching the suggested name's
/// extension comes first, so a .pcraft document saves as .pcraft by default and everything else
/// keeps defaulting to Photoshop.
const SAVE_FILTERS: &[(&str, &[&str])] = &[
    ("Photoshop", &["psd", "psb"]),
    ("PhotoCraft", &["pcraft"]),
    ("PNG", &["png"]),
    ("JPEG", &["jpg"]),
    ("WebP", &["webp"]),
    ("TIFF", &["tif"]),
    ("Targa", &["tga"]),
    ("OpenEXR", &["exr"]),
];

/// Non-document files the shell saves (Swatches panel exports): offered alone, so the dialog
/// never swaps their extension for a document format's.
const OTHER_SAVE_FILTERS: &[(&str, &[&str])] = &[("Color Swatches", &["aco"]), ("Swatch Exchange", &["ase"])];

/// Lists the save dialog's file types with the `suggested` type first (added if unlisted), so the dialog keeps that extension instead of .psd.
fn save_filters(suggested: &str) -> Vec<(String, Vec<String>)> {
    let ext = Path::new(suggested).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if let Some((name, exts)) = OTHER_SAVE_FILTERS.iter().find(|(_, exts)| exts.contains(&ext.as_str())) {
        return vec![(name.to_string(), exts.iter().map(|e| e.to_string()).collect())];
    }
    let mut v: Vec<(String, Vec<String>)> = SAVE_FILTERS.iter().map(|(name, exts)| (name.to_string(), exts.iter().map(|e| e.to_string()).collect())).collect();
    match v.iter().position(|(_, exts)| exts.contains(&ext)) {
        Some(i) => {
            let f = v.remove(i);
            v.insert(0, f);
        }
        None if !ext.is_empty() => v.insert(0, (ext.to_ascii_uppercase(), vec![ext])),
        None => {}
    }
    v
}

/// Shows an Open or Save dialog without blocking the event loop (#673, #574): rfd's async dialog
/// is created here, on the main thread, with the window as its parent (a sheet on macOS, an owned
/// modal window elsewhere), and a helper thread waits for it and hands the answer to the app.
fn show_file_dialog(request: FileDialogRequest, parent: Option<&eframe::Frame>, reply: FileDialogReply) {
    let mut dialog = rfd::AsyncFileDialog::new();
    if let Some(parent) = parent {
        dialog = dialog.set_parent(parent);
    }
    let answer: Pin<Box<dyn Future<Output = Option<FileDialogAnswer>> + Send>> = match request {
        FileDialogRequest::XcfLinkedImage => {
            let picked = dialog.add_filter("PNG", &["png"]).pick_file();
            Box::pin(async move {
                let file = picked.await?;
                match crate::bounded_resource_read::read_selection(Some(file.path())) {
                    Ok(Some(selected)) => Some(FileDialogAnswer::Contents(selected.name, selected.bytes)),
                    Ok(None) => None,
                    Err(error) => Some(FileDialogAnswer::Failed(error.to_string())),
                }
            })
        }
        FileDialogRequest::Open { multiple } => {
            let dialog = dialog.add_filter("All Formats", OPEN_EXTS).add_filter("PhotoCraft", &["pcraft"]);
            if multiple {
                let picked = dialog.pick_files();
                Box::pin(async move { picked.await.map(|files| FileDialogAnswer::Paths(files.iter().map(path_of).collect())) })
            } else {
                let picked = dialog.pick_file();
                Box::pin(async move { picked.await.map(|file| FileDialogAnswer::Paths(vec![path_of(&file)])) })
            }
        }
        FileDialogRequest::Save { suggested } => {
            for (name, exts) in save_filters(&suggested) {
                dialog = dialog.add_filter(name, &exts);
            }
            if let Some(name) = Path::new(&suggested).file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
            let picked = dialog.save_file();
            Box::pin(async move { picked.await.map(|file| FileDialogAnswer::SaveTo(path_of(&file))) })
        }
    };
    // Without the thread the reply is dropped unanswered, which the app takes as Cancel.
    if let Err(e) = std::thread::Builder::new().name("file dialog".into()).spawn(move || reply.send(block_on(answer))) {
        log::error!("couldn't wait for the file dialog: {e}");
    }
}

fn path_of(file: &rfd::FileHandle) -> String {
    file.path().to_string_lossy().into_owned()
}

/// Runs `future` to completion on this thread, sleeping until it is woken.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        // Spurious wake-ups just poll again.
        std::thread::park();
    }
}

/// Per-user settings directory: `PHOTOCRAFT_CONFIG_DIR`, else `<exe dir>/PhotoCraftData` in
/// portable mode, else the platform convention. Everything the app persists lives under it; see
/// [`crate::app_dirs`].
pub fn config_dir() -> Option<PathBuf> {
    crate::app_dirs::config_dir()
}

pub fn prefs_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("preferences.json"))
}

/// The brush preset store (one file per preset group plus tip bitmaps; see
/// `photocraft_engine::preset_store`).
pub fn presets_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("Presets"))
}

fn recovery_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("Recovery"))
}

/// Write `bytes` crash-safely (temp file beside the target, fsync, rename, directory fsync; see
/// [`photocraft_format::atomic`]). Every document write (Save, Save As, Export, Save for Web) and
/// the preferences go through here, so a failed or interrupted save never destroys the old file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    photocraft_automation::files::validate_destination(path, None, None).map_err(|e| e.to_string())?;
    photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())
}

/// Interactive native output streams the prepared archive directly to its
/// atomic staging file. All-open source exclusions remain live at every final
/// target check; a snapshot does not grant any retained source-path reads.
fn save_native(
    snapshot: &photocraft_engine::document_snapshot::DocumentSnapshot,
    path: &str,
    protection: &photocraft_engine::WriteProtection,
) -> Result<photocraft_ui_egui::services::NativeSaveResult, String> {
    crate::crash_guard::guard("Save", || {
        snapshot.validate_resident().map_err(|error| error.to_string())?;
        let target = Path::new(path);
        let control = photocraft_raster::Interrupt::NONE;
        let guard = |candidate: &Path| -> Result<(), photocraft_io::IoError> {
            photocraft_automation::files::validate_destination_protected_with_interrupt(candidate, protection, Some("pcraft"), &control)
                .map_err(|error| photocraft_io::IoError::Unsupported(error.to_string()))
        };
        // Preserve the existing native atomic writer's parent-directory policy.
        // Admission and target guards precede preparation; the writer repeats
        // guards on its resolved target and each atomic rename attempt.
        let result = photocraft_io::save_native_path_guarded(snapshot, target, Default::default(), &control, &guard).map_err(|error| error.to_string())?;
        Ok(photocraft_ui_egui::services::NativeSaveResult { warnings: result.warnings })
    })
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

/// Explicit read-root material only; never the generic document reader or a stored locator.
fn rooted_linked_replacement(workspace: &photocraft_automation::AuthorizedWorkspace, path: &str, reply: FileDialogReply) {
    let path = match crate::bounded_resource_read::rooted_request_path(path) {
        Ok(path) => path,
        Err(error) => {
            reply.send(Some(FileDialogAnswer::Failed(error.to_string())));
            return;
        }
    };
    let workspace = workspace.clone();
    // Keep a second owner until spawn succeeds, so spawn failure is a visible error, not Cancel.
    let reply = Arc::new(std::sync::Mutex::new(Some(reply)));
    let worker_reply = Arc::clone(&reply);
    let worker = std::thread::Builder::new().name("linked resource read".into()).spawn(move || {
        let answer = match crate::bounded_resource_read::read_rooted(&workspace, &path) {
            Ok(selected) => FileDialogAnswer::Contents(selected.name, selected.bytes),
            Err(error) => FileDialogAnswer::Failed(error.to_string()),
        };
        if let Some(reply) = worker_reply.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            reply.send(Some(answer));
        }
    });
    if worker.is_err()
        && let Some(reply) = reply.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    {
        reply.send(Some(FileDialogAnswer::Failed("xcf.resource.read-start-failed".into())));
    }
}

/// Run `f` on the recovery store (`Err` without a config directory). The service closures never
/// call each other, so the store is never borrowed twice.
fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no config directory")?))
}

/// Crash recovery: background incremental .pcraft autosaves into `dir` (`None`: no config
/// directory, so autosaves fail and nothing is recovered). Recovered documents keep their entries
/// until a newer autosave replaces them or they're saved or closed (see [`RecoveryStore`]).
fn recovery_services(dir: Option<PathBuf>) -> Services {
    let store: SharedRecovery = Rc::new(RefCell::new(dir.map(RecoveryStore::new)));
    let (s1, s2, s3, s4) = (store.clone(), store.clone(), store.clone(), store.clone());
    Services {
        write_protected: Some(Box::new(|path, bytes, protection| {
            photocraft_automation::files::write_protected_native(Path::new(path), bytes, protection).map_err(|error| error.to_string())
        })),
        validate_write_protected: Some(Box::new(|path, protection| {
            photocraft_automation::files::validate_destination_protected(Path::new(path), protection, None).map_err(|error| error.to_string())
        })),
        autosave: Some(Box::new(move |doc: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, path: Option<&str>| {
            let resident = doc.resident_arc().ok_or("Automatic recovery saving is not supported for this accounted document")?;
            with_store(&s1, |s| s.autosave_checked(resident, revision, path.map(str::to_string)))?
        })),
        autosave_results: Some(Box::new(move || with_store(&s4, |s| s.take_completed()).unwrap_or_default())),
        discard_autosave: Some(Box::new(move |id: u64| {
            let _ = with_store(&s2, |s| s.discard(id));
        })),
        recover: Some(Box::new(move || {
            let found = with_store(&s3, |s| s.recover()).unwrap_or_default();
            found.into_iter().map(|(e, doc)| Recovered { key: e.info.key, path: e.info.original_path, doc }).collect()
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
            let _ = with_store(&store, |s| s.adopt(id, key));
        })),
        ..Default::default()
    }
}

/// The pixels to paste when the clipboard holds copied files rather than an image (Copy in
/// Files, Finder or Explorer puts paths on the clipboard, #338): the first file that decodes,
/// upright, as RGBA8. Files that aren't images are skipped after reading only their header.
fn image_from_files(paths: &[PathBuf]) -> Option<(u32, u32, Vec<u8>)> {
    use std::io::Read;
    paths.iter().find_map(|path| {
        // text/uri-list lines end in CRLF (RFC 2483, and GTK writes them so), but arboard splits
        // on LF only, so a path copied in GNOME Files arrives with a trailing '\r'.
        let path = path.to_str().map_or_else(|| path.clone(), |s| PathBuf::from(s.trim_end_matches('\r')));
        let mut head = Vec::with_capacity(256);
        std::fs::File::open(&path).ok()?.take(256).read_to_end(&mut head).ok()?;
        // TGA has no magic number and is only guessed from the header, so trust the extension
        // too before reading what may be a large non-image file.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match photocraft_codecs::detect(&head)? {
            photocraft_codecs::Format::Tga if photocraft_codecs::from_extension(ext) != Some(photocraft_codecs::Format::Tga) => return None,
            _ => {}
        }
        let img = photocraft_codecs::decode(&std::fs::read(&path).ok()?).ok()?;
        Some((img.width(), img.height(), img.to_rgba8()))
    })
}

/// The shell command that starts this install under XWayland, where winit delivers file drops
/// (winit 0.30 has none on Wayland, #386). A Flatpak gets the X11 socket only without the Wayland
/// one, an AppImage relaunches by its own path, and anything else is `photocraft` on the PATH.
/// `None` outside Flatpak when there is no X server (`DISPLAY` unset) to run on.
#[cfg(any(target_os = "linux", test))]
pub fn xwayland_command(flatpak_id: Option<&str>, appimage: Option<&str>, x_display: bool) -> Option<String> {
    if let Some(id) = flatpak_id.filter(|id| !id.is_empty()) {
        return Some(format!("flatpak run --nosocket=wayland --socket=x11 {}", shell_word(id)));
    }
    let exe = appimage.filter(|path| !path.is_empty()).map_or_else(|| "photocraft".to_string(), shell_word);
    x_display.then(|| format!("WAYLAND_DISPLAY= {exe}"))
}

/// `s` as one POSIX shell word: bare when every character is safe there, else single-quoted.
#[cfg(any(target_os = "linux", test))]
fn shell_word(s: &str) -> String {
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+:,@".contains(&b)) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

pub fn native(automation: Option<photocraft_automation::AuthorizedWorkspace>) -> Services {
    let clip: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::default();
    let automation_read = automation.clone().map(|workspace| {
        Box::new(move |path: &str| {
            let bytes = workspace.read(path).map_err(|error| error.to_string())?;
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_string();
            Ok((name, bytes))
        }) as photocraft_ui_egui::AutomationReadFn
    });
    let automation_read_xcf_linked_replacement = automation.clone().map(|workspace| {
        Box::new(move |path: &str, reply: FileDialogReply| rooted_linked_replacement(&workspace, path, reply))
            as photocraft_ui_egui::file_dialog::AutomationReadXcfLinkedReplacementFn
    });
    let automation_write = automation.clone().map(|workspace| {
        Box::new(move |path: &str, bytes: &[u8], protection: &photocraft_engine::WriteProtection| {
            workspace.write_protected(path, bytes, &protection.sources, &protection.fingerprints).map_err(|error| error.to_string())
        }) as photocraft_ui_egui::AutomationWriteFn
    });
    let automation_validate_write = automation.clone().map(|workspace| {
        Box::new(move |path: &str, protection: &photocraft_engine::WriteProtection| {
            workspace.guard_write_protected(path, &protection.sources, &protection.fingerprints).map_err(|e| e.to_string())
        }) as photocraft_ui_egui::AutomationValidateWriteFn
    });
    let automation_native_save = automation.clone().map(|workspace| {
        Box::new(move |snapshot: &photocraft_engine::DocumentSnapshot, path: &str, protection: &photocraft_engine::WriteProtection| {
            crate::crash_guard::guard("Save", || {
                snapshot.validate_resident().map_err(|error| error.to_string())?;
                let result = workspace
                    .save_native_with_interrupt(snapshot, path, protection, photocraft_io::NativeSaveLimits::default(), &photocraft_raster::Interrupt::NONE)
                    .map_err(|error| error.to_string())?;
                Ok(photocraft_ui_egui::services::NativeSaveResult { warnings: result.warnings })
            })
        }) as photocraft_ui_egui::services::NativeSaveFn
    });
    let step: fn(&str, &serde_json::Value) -> photocraft_engine::Result<()> = photocraft_automation::workspace::authorize_desktop_engine_step;
    let automation_authorize = automation.is_some().then_some(step);
    let automation_command = automation.map(|_| {
        Box::new(|id: &str, params: &serde_json::Value| {
            photocraft_automation::workspace::authorize_desktop_engine_command(id, params).map_err(|error| error.to_string())
        }) as photocraft_ui_egui::AutomationCommandFn
    });
    Services {
        native_save: Some(Box::new(save_native)),
        validate_write: Some(Box::new(|path, source| {
            photocraft_automation::files::validate_destination(Path::new(path), source, None).map_err(|e| e.to_string())
        })),
        automation_validate_write,
        automation_native_save,
        import: Some(Box::new(|name: &str, bytes: &[u8]| crate::crash_guard::guard("Open", || photocraft_io::import(name, bytes).map_err(|e| e.to_string())))),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let mut opts = photocraft_io::ExportOptions::default();
            if let Some(q) = settings.jpeg_quality {
                opts.encode.jpeg_quality = q;
            }
            opts.encode.webp_lossless = settings.webp_lossless;
            if let Some(q) = settings.webp_quality {
                opts.encode.webp_quality = q;
            }
            opts.tiff_layers = settings.tiff_layers;
            opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
            crate::crash_guard::guard("Export", || photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))
        })),
        file_dialog: Some(Box::new(show_file_dialog)),
        write: Some(Box::new(|path: &str, bytes: &[u8]| write_atomic(Path::new(path), bytes))),
        automation_read,
        automation_read_xcf_linked_replacement,
        automation_write,
        automation_command,
        automation_authorize,
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        inbox: None,
        open_url: Some(Box::new(|url: &str| open::that(url).map_err(|e| e.to_string()))),
        clipboard_set_image: Some({
            let clip = clip.clone();
            Box::new(move |w: u32, h: u32, px: &[u8]| {
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy".to_string())?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                };
                cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(px) }).map_err(|e| e.to_string())
            })
        }),
        clipboard_get_image: Some({
            let clip = clip.clone();
            Box::new(move || {
                let mut slot = clip.try_borrow_mut().ok()?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().ok()?),
                };
                // Copied files first: Finder also puts the file's icon on the clipboard as an
                // image, which would otherwise paste instead of the file.
                if let Some(img) = cb.get().file_list().ok().and_then(|paths| image_from_files(&paths)) {
                    return Some(img);
                }
                let img = cb.get_image().ok()?;
                Some((img.width as u32, img.height as u32, img.bytes.into_owned()))
            })
        }),
        load_prefs: Some(Box::new(|| std::fs::read_to_string(prefs_file()?).ok())),
        save_prefs: Some(Box::new(|text: &str| write_atomic(&prefs_file().ok_or("no config directory")?, text.as_bytes()))),
        append_text: Some(Box::new(|path: &str, text: &str| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
            f.write_all(text.as_bytes()).map_err(|e| e.to_string())
        })),
        // Set by main once the Apple-event handlers are connected (macOS).
        os_events: None,
        // Set by main, which starts loading the store before the window opens.
        preset_store: None,
        is_wayland: false,
        ..recovery_services(recovery_dir())
    }
}

/// Reference/test entry point uses the same typed import as the application.
#[allow(dead_code)]
pub fn import_flat(name: &str, bytes: &[u8]) -> Result<Document, String> {
    photocraft_io::import(name, bytes).map(|result| result.document).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use photocraft_format::list_recovery;
    use photocraft_ui_egui::{PhotocraftApp, prefs_ui};
    use serde_json::{Value, json};

    #[test]
    fn xwayland_command_matches_how_photocraft_was_installed() {
        // Flatpak: WAYLAND_DISPLAY doesn't reach the sandbox's socket choice; flatpak's flags do.
        let flatpak = xwayland_command(Some("ai.storyteller.photocraft"), None, false);
        assert_eq!(flatpak.as_deref(), Some("flatpak run --nosocket=wayland --socket=x11 ai.storyteller.photocraft"));
        // AppImage: its own path, quoted for the shell (spaces, quotes).
        let appimage = xwayland_command(None, Some("/home/me/My Apps/it's.AppImage"), true);
        assert_eq!(appimage.as_deref(), Some(r"WAYLAND_DISPLAY= '/home/me/My Apps/it'\''s.AppImage'"));
        let bare = xwayland_command(None, Some("/opt/photocraft-0.3.0-linux-x86_64.AppImage"), true);
        assert_eq!(bare.as_deref(), Some("WAYLAND_DISPLAY= /opt/photocraft-0.3.0-linux-x86_64.AppImage"));
        // deb, rpm, AUR, tarball: the binary on the PATH.
        assert_eq!(xwayland_command(None, None, true).as_deref(), Some("WAYLAND_DISPLAY= photocraft"));
        assert_eq!(xwayland_command(Some(""), Some(""), true).as_deref(), Some("WAYLAND_DISPLAY= photocraft"));
        // No X server (XWayland disabled): nothing to suggest.
        assert_eq!(xwayland_command(None, Some("/opt/p.AppImage"), false), None);
    }

    /// Tests every native save dialog and records the suggested file name. Each name's file type must come first
    /// in the save panel, or the panel appends the first type's extension (`photo.webp.psd`, `photo.gif.psd`).
    #[test]
    fn every_save_dialog_leads_with_its_own_extension() {
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        let log = asked.clone();
        // Record each save dialog's suggested name and cancel it, as the user would.
        let services = Services {
            file_dialog: Some(Box::new(move |request, _parent, reply| {
                if let FileDialogRequest::Save { suggested } = request {
                    log.borrow_mut().push(suggested);
                }
                reply.send(None);
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        let ctx = egui::Context::default();
        // Dialogs are shown on the next frame: poll once so each is answered before the next asks.
        let invoke = |app: &mut PhotocraftApp, id: &str| {
            let r = photocraft_ui_egui::menus::invoke(app, &ctx, id, json!({}));
            app.poll_file_dialog(&ctx, None);
            r
        };
        let dialog = |app: &mut PhotocraftApp, id: &str, fields: Value| {
            let d = invoke(app, id).unwrap()["dialog"].as_u64().unwrap();
            for (k, v) in fields.as_object().unwrap() {
                app.ui.dialog_mut(d).unwrap().fields.insert(k.clone(), v.clone());
            }
            let _ = photocraft_ui_egui::dialogs::confirm(app, d);
            app.poll_file_dialog(&ctx, None);
        };
        new_doc(&mut app, "#ff0000");
        // Save As, Save a Copy.
        // Files in a format Save As can't write, like .dng, should suggest .psd instead.
        let _ = invoke(&mut app, "file.saveAs");
        for name in ["cat.pcraft", "cat.jpeg", "cat.gif", "cat.bmp", "cat.dng"] {
            app.session.active_mut().unwrap().path = Some(name.into());
            let _ = invoke(&mut app, "file.saveAs");
        }
        let _ = invoke(&mut app, "file.saveACopy");
        // Export As, Quick Export, Save for Web (with and without slices).
        for format in ["png", "jpg", "webp", "tif", "tga"] {
            dialog(&mut app, "file.export.exportAs", json!({"format": format}));
        }
        for format in ["png", "jpg", "gif", "webp"] {
            app.run("prefs.set", json!({"path": "export.quickExportFormat", "value": format})).unwrap();
            let _ = invoke(&mut app, "file.export.quickExportAsPng");
        }
        for format in ["gif", "png8", "png24", "jpeg", "wbmp"] {
            dialog(&mut app, "file.export.saveForWebLegacy", json!({"format": format}));
        }
        app.run("slice.new", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
        dialog(&mut app, "file.export.saveForWebLegacy", json!({"html": true}));
        // Measurement Log, Export/Import Presets.
        app.run("image.analysis.recordMeasurements", json!({})).unwrap();
        let _ = invoke(&mut app, "measurementLog.export");
        dialog(&mut app, "edit.presets.exportImportPresets", json!({"action": "export"}));
        let asked = asked.borrow();
        let exts: Vec<&str> = asked.iter().filter_map(|s| s.rsplit_once('.').map(|(_, e)| e)).collect();
        // The extension each save above should suggest, in order.
        let want = [
            "psd",                       // Save As untitled
            "pcraft jpeg gif bmp psd",   // Save As opened .pcraft .jpeg .gif .bmp .dng
            "psd",                       // Save a Copy
            "png jpg webp tif tga",      // Export As
            "png jpg gif webp",          // Quick Export
            "gif png png jpg wbmp html", // Save for Web: gif png8 png24 jpeg wbmp, then with slices
            "csv pcpresets",             // Measurement Log, Export Presets
        ]
        .join(" ");
        assert_eq!(exts, want.split(' ').collect::<Vec<_>>());
        for (name, ext) in asked.iter().zip(exts) {
            assert!(save_filters(name)[0].1.iter().any(|e| e == ext), "{name}: {:?}", save_filters(name)[0]);
        }
        assert_eq!(save_filters("Untitled")[0].0, "Photoshop", "no extension keeps the default");
    }

    #[test]
    fn block_on_waits_for_a_wake_from_another_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<std::task::Waker>();
        let mut woken = false;
        let ready = std::future::poll_fn(move |cx| {
            if woken {
                return std::task::Poll::Ready(7);
            }
            woken = true;
            let _ = tx.send(cx.waker().clone());
            std::task::Poll::Pending
        });
        let waker = std::thread::spawn(move || rx.recv().unwrap().wake());
        assert_eq!(block_on(ready), 7);
        waker.join().unwrap();
    }

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

    /// One app launch with crash recovery in `dir` (dropping it is the crash: background writes
    /// already queued finish, nothing else runs).
    fn launch(dir: &Path) -> PhotocraftApp {
        PhotocraftApp::new(Session::new(), recovery_services(Some(dir.to_path_buf())))
    }

    /// The first pixel of each open document, and whether it's unsaved.
    fn open_docs(app: &PhotocraftApp) -> Vec<([f32; 4], bool)> {
        app.session.documents().iter().map(|d| (photocraft_compose::flatten(&d.doc).unwrap().px.first().copied().unwrap_or_default(), d.is_dirty())).collect()
    }

    fn index_of(app: &PhotocraftApp, color: [f32; 4]) -> usize {
        open_docs(app).iter().position(|(c, _)| *c == color).unwrap()
    }

    fn new_doc(app: &mut PhotocraftApp, color: &str) {
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }

    fn autosave(app: &mut PhotocraftApp, ctx: &egui::Context) {
        prefs_ui::autosave_now(app);
        prefs_ui::tick(app, ctx);
    }

    #[test]
    fn recovered_documents_survive_a_second_crash_until_saved_or_closed() {
        let dir = std::env::temp_dir().join(format!("photocraft-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = egui::Context::default();
        // Launch 1: two unsaved documents are autosaved, then the app crashes.
        {
            let mut app = launch(&dir);
            new_doc(&mut app, "#ff0000");
            new_doc(&mut app, "#0000ff");
            autosave(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 2);
        // Launch 2 recovers both and crashes again before its first autosave (#444).
        {
            let mut app = launch(&dir);
            let mut docs = open_docs(&app);
            docs.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
            assert_eq!(docs, [(BLUE, true), (RED, true)], "recovered and still unsaved");
            prefs_ui::tick(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 2, "recovering keeps the entries on disk");
        // Launch 3 has both again; autosaving unchanged documents adds nothing, a new document
        // (whose id may repeat one from an earlier launch) gets an entry of its own.
        {
            let mut app = launch(&dir);
            assert_eq!(app.session.documents().len(), 2);
            autosave(&mut app, &ctx);
            new_doc(&mut app, "#00ff00");
            autosave(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 3);
        // Launch 4: closing and saving recovered documents removes their entries, no duplicates.
        let mut app = launch(&dir);
        assert_eq!(app.session.documents().len(), 3);
        let red = index_of(&app, RED);
        app.run("file.close", json!({"document": red})).unwrap();
        // Saving marks the revision saved (the file write itself is the save service's job).
        assert!(app.session.set_active(index_of(&app, BLUE)));
        let st = app.session.active_mut().unwrap();
        st.saved_revision = st.revision;
        prefs_ui::tick(&mut app, &ctx);
        let left = list_recovery(&dir);
        assert_eq!(left.len(), 1);
        assert_eq!(photocraft_compose::flatten(&photocraft_format::recover(&left[0]).unwrap()).unwrap().px.first().copied(), Some(GREEN));
        let green = index_of(&app, GREEN);
        app.run("file.close", json!({"document": green})).unwrap();
        prefs_ui::tick(&mut app, &ctx);
        assert!(list_recovery(&dir).is_empty());
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_background_autosave_is_reported_and_retried_without_another_edit() {
        let dir = temp("autosave-retry");
        let recovery = dir.join("Recovery");
        // A regular file prevents creation of the recovery bundle directory.
        std::fs::write(&recovery, b"blocked").unwrap();
        let ctx = egui::Context::default();
        let mut app = launch(&recovery);
        new_doc(&mut app, "#ff0000");
        let revision = app.session.active().unwrap().revision;
        autosave(&mut app, &ctx);

        let mut failed = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            prefs_ui::tick(&mut app, &ctx);
            if app.ui.status.starts_with("Autosave failed:") {
                failed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(failed, "background write failures must reach the UI");
        assert!(app.ui.status_error);
        assert!(list_recovery(&recovery).is_empty());

        // No edit occurs: retrying this very same revision must still work.
        std::fs::remove_file(&recovery).unwrap();
        std::fs::create_dir(&recovery).unwrap();
        autosave(&mut app, &ctx);
        let mut recovered = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            prefs_ui::tick(&mut app, &ctx);
            recovered = list_recovery(&recovery);
            if !recovered.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(recovered.len(), 1, "unchanged revision should retry after failure");
        assert_eq!(recovered[0].info.revision, revision);
        let restored = photocraft_format::recover(&recovered[0]).unwrap();
        assert_eq!(photocraft_compose::flatten(&restored).unwrap().px.first().copied(), Some(RED));
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-services-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn native_stream_capability_roundtrips_and_protects_other_open_source() {
        let directory = temp("native-stream");
        let mut document = Document::new("Own native stream", photocraft_doc::Size::new(2, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U16);
        document.layers.push(photocraft_doc::Layer::raster("Own raster", document.pixel_format()));
        let snapshot = photocraft_engine::DocumentSnapshot::from_resident(Arc::new(document)).unwrap();
        let output = directory.join("own.pcraft");
        let output_text = output.to_str().unwrap();
        let protection = photocraft_engine::WriteProtection::default();
        let result = save_native(&snapshot, output_text, &protection).unwrap();
        assert!(result.warnings.is_empty());
        let restored = photocraft_format::load_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(restored, *snapshot);
        let protected = directory.join("another-open-source.pcraft");
        let original = b"Own existing source bytes";
        std::fs::write(&protected, original).unwrap();
        let protection = photocraft_engine::WriteProtection { sources: vec![protected.to_str().unwrap().into()], fingerprints: Vec::new() };
        assert!(save_native(&snapshot, protected.to_str().unwrap(), &protection).is_err());
        assert_eq!(std::fs::read(&protected).unwrap(), original);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn flat_import_retains_unequal_dpi_axes() {
        let mut img = Image::from_u8(2, 1, ChannelLayout::Rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        img.meta.dpi = Some((144.0, 288.0));
        let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).unwrap();
        let doc = super::import_flat("axes.png", &bytes).unwrap();
        let resolution = doc.effective_resolution();
        assert!((resolution.x_dpi - 144.0).abs() < 0.1);
        assert!((resolution.y_dpi - 288.0).abs() < 0.1);
        assert!((resolution.inches(144, 288).unwrap().0 - 1.0).abs() < 0.001);
        assert!((resolution.inches(144, 288).unwrap().1 - 1.0).abs() < 0.001);
    }

    #[test]
    fn typed_import_preserves_native_wide_samples_and_rejects_corrupt_input() {
        use photocraft_doc::{ColorMode, Layer, SampleType, Size};
        let integer: Vec<u8> = [1_u32, 123456789, u32::MAX - 1, u32::MAX, 3141592653, 17, 2000000001, 1234567].into_iter().flat_map(u32::to_ne_bytes).collect();
        let floating: Vec<u8> = [f64::from_bits(0x3fe0000000000001), 0.1, 0.2, 1.0, 0.25, 0.5, 0.75, 0.125].into_iter().flat_map(f64::to_ne_bytes).collect();
        for (depth, bytes) in [(SampleType::U32, integer), (SampleType::F64, floating)] {
            let mut document = Document::new("wide", Size::new(2, 1), ColorMode::Rgb, depth);
            let mut layer = Layer::raster("Wide samples", document.pixel_format());
            layer.surface_mut().unwrap().try_write_interleaved(document.bounds(), &bytes).unwrap();
            document.layers.push(layer);
            let encoded = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let imported = super::import_flat("wide.pcraft", &encoded).unwrap();
            assert_eq!(imported, document);
            assert_eq!(imported.layers[0].surface().unwrap().try_read_interleaved(imported.bounds()).unwrap(), bytes);
        }
        assert!(super::import_flat("broken.xcf", b"gimp xcf v022\0truncated").is_err());
    }

    fn red_png(w: u32, h: u32) -> Vec<u8> {
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).unwrap()
    }

    /// A 1×1 uncompressed 32-bit TGA (TGA has no magic number).
    fn tga_1x1() -> Vec<u8> {
        let mut b = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 8];
        b.extend_from_slice(&[0, 0, 255, 255]); // BGRA red
        b
    }

    #[test]
    fn copied_image_file_pastes_its_pixels() {
        let dir = temp("png");
        let png = dir.join("red.png");
        std::fs::write(&png, red_png(3, 2)).unwrap();
        let (w, h, px) = image_from_files(&[png]).unwrap();
        assert_eq!((w, h), (3, 2));
        assert_eq!(px, [255u8, 0, 0, 255].repeat(6));
    }

    #[test]
    fn crlf_from_a_uri_list_is_ignored() {
        // arboard splits text/uri-list on LF, so GNOME Files' CRLF list leaves a '\r' behind.
        let dir = temp("crlf");
        let png = dir.join("red image.png");
        std::fs::write(&png, red_png(2, 2)).unwrap();
        let with_cr = PathBuf::from(format!("{}\r", png.display()));
        assert_eq!(image_from_files(&[with_cr]).map(|(w, h, _)| (w, h)), Some((2, 2)));
    }

    #[test]
    fn non_images_are_skipped_for_the_first_image() {
        let dir = temp("mixed");
        let (txt, png) = (dir.join("notes.txt"), dir.join("red.png"));
        std::fs::write(&txt, b"not an image").unwrap();
        std::fs::write(&png, red_png(4, 1)).unwrap();
        let missing = dir.join("gone.png");
        assert_eq!(image_from_files(&[missing.clone(), txt.clone(), dir.clone(), png]).map(|(w, h, _)| (w, h)), Some((4, 1)));
        assert!(image_from_files(&[missing, txt, dir]).is_none());
        assert!(image_from_files(&[]).is_none());
    }

    #[test]
    fn a_damaged_image_is_skipped_not_a_panic() {
        let dir = temp("damaged");
        let bad = dir.join("cut.png");
        std::fs::write(&bad, &red_png(8, 8)[..20]).unwrap();
        assert!(image_from_files(&[bad]).is_none());
    }

    #[test]
    fn tga_is_trusted_only_with_its_extension() {
        let dir = temp("tga");
        let (tga, bin) = (dir.join("red.tga"), dir.join("red.bin"));
        std::fs::write(&tga, tga_1x1()).unwrap();
        std::fs::write(&bin, tga_1x1()).unwrap();
        assert_eq!(image_from_files(&[tga]), Some((1, 1, vec![255, 0, 0, 255])));
        assert!(image_from_files(&[bin]).is_none());
    }
}
