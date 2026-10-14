# Native document windows

Document windows on X11/XWayland and Windows belong to PhotoCraft's main window.
Raising the main window keeps its document windows above it. Focusing another
application lets that application cover the complete group.

The UI supplies the exact open document viewport IDs before creating any of
those viewports. This includes primary floating documents and additional views.
Other viewports retain their existing role. Window titles and process searches
do not determine ownership.

The desktop pins a small eframe 0.36.2 patch that exposes repeatable native window
attributes and creation callbacks. The remaining egui packages and winit keep
their existing versions. Windows receives its owner in the creation attributes.
On X11, each document window starts hidden, receives `WM_TRANSIENT_FOR`, and then
regains its requested visibility. The existing `WM_CLIENT_LEADER` is preserved.
Native Wayland and macOS retain their existing window behavior.

X11 work runs through one bounded worker queue. Each actual window creation gets
a fresh 50 ms UI wait; changing a title, resizing, or drawing another frame does
not trigger ownership traffic. Requests check cancellation before writes, weak
window references guard native handle lifetimes, and every failure restores requested
visibility. Authentication file reads occur on the worker and cannot be forcibly
canceled. Transport failures permit a later creation to reconnect.

Unsaved-changes confirmations for a primary floating document appear in that
document's viewport. Confirmation remains tied to the document ID when tab
indices change. The root UI blocks editing while confirmation is pending;
embedded viewports use the root UI.

TIFF and Targa options reached by Save or Save As use that same document's
viewport. Editing stays blocked until the options close. Enter accepts only in
the visible prompt; Escape in the blocked root can cancel safely. Canceling
format options returns to a pending unsaved-changes confirmation without a write.

Validation includes native KWin/XWayland controls with WGPU and Glow, actual
PhotoCraft transient-property and stacking checks, native child Escape
cancellation, and UI prompt captures in Pro/Studio at 1x and 2x. The safe Windows
ownership paths cross-compile with strict warnings; Windows desktop runtime
validation remains separate.
